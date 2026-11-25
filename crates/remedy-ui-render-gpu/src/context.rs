//! Getting a GPU: the backend ladder, and noticing when the device is lost.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use remedy_ui_core::host::WindowHandles;

use crate::GpuError;

/// Which rung of the backend ladder a context came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rung {
    /// A hardware adapter through D3D12, Vulkan or Metal; WebGPU in a browser.
    Primary,
    /// A hardware adapter through OpenGL 3.3 / GLES 3.0; WebGL2 in a browser.
    Gl,
    /// A software adapter (WARP, lavapipe, llvmpipe): no GPU, still the GPU code path.
    /// Browsers offer none (their WebGL may itself run in software).
    Software,
}

/// Which rungs to try, in order, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LadderOptions {
    /// Prefer the discrete GPU on machines with two.
    pub high_performance: bool,
    /// The rungs to try, top first.
    pub rungs: Vec<Rung>,
    /// Checks each rung's device with a self-test frame before using it, and skips rungs
    /// that failed it or hung presenting ([`health`](crate::health)). On by default; a
    /// backend is tested once per driver.
    pub self_test: bool,
}

impl LadderOptions {
    /// Sets `high_performance`: prefer the discrete GPU on machines with two.
    pub fn high_performance(mut self, value: bool) -> Self {
        self.high_performance = value;
        self
    }

    /// Sets `rungs`: the rungs to try, top first.
    pub fn rungs(mut self, value: Vec<Rung>) -> Self {
        self.rungs = value;
        self
    }

    /// Sets `self_test`: checks each rung's device before using it.
    pub fn self_test(mut self, value: bool) -> Self {
        self.self_test = value;
        self
    }
}

impl Default for LadderOptions {
    fn default() -> Self {
        LadderOptions {
            high_performance: true,
            rungs: vec![Rung::Primary, Rung::Gl, Rung::Software],
            self_test: true,
        }
    }
}

/// A device and queue, with what they are and whether they are still alive.
#[derive(Clone, Debug)]
pub struct GpuContext {
    /// The instance adapters came from.
    pub instance: wgpu::Instance,
    /// The adapter.
    pub adapter: wgpu::Adapter,
    /// The device.
    pub device: wgpu::Device,
    /// Its queue.
    ///
    /// Submit to it only from the thread that presents windows (the UI thread): on Vulkan a
    /// window's own swapchain presents on this queue outside wgpu's lock, so a submission
    /// from another thread at the same time is undefined behaviour.
    pub queue: wgpu::Queue,
    /// The rung it came from.
    pub rung: Rung,
    lost: Arc<AtomicBool>,
    errors: Arc<Mutex<Vec<String>>>,
    /// The staging buffer the latest read back used, kept for the next one of the same size
    /// (snapshots, GPU content shown by the CPU renderer) and freed by
    /// [`trim_memory`](GpuContext::trim_memory) when the system asks for memory back.
    pub(crate) readback: Arc<Mutex<Option<wgpu::Buffer>>>,
    /// Textures the app renders and display lists place (`Item::Texture`).
    pub(crate) textures: crate::external::Registry,
    /// The effect kernels, made on first use; they go with the device's last context.
    pub(crate) effect_kernels: Arc<std::sync::OnceLock<Arc<crate::effects::Kernels>>>,
    pub(crate) serial: u64,
    pub(crate) timer: crate::gputime::GpuTimer,
    /// The one window a browser's WebGL2 context draws on (its `data-raw-handle` id): a
    /// WebGL2 device belongs to the canvas it was made for. `None` everywhere else.
    pub(crate) bound_canvas: Option<u32>,
    /// The Vulkan device was opened with `VK_KHR_incremental_present`: presents can say
    /// which rectangles changed.
    #[cfg_attr(
        not(any(
            all(unix, not(any(target_os = "macos", target_os = "ios"))),
            all(windows, feature = "windows-vulkan")
        )),
        allow(dead_code)
    )]
    pub(crate) incremental_present: bool,
}

/// The default of [`GpuLimits::kept_errors`](crate::GpuLimits::kept_errors): the most GPU
/// errors kept between calls to [`GpuContext::take_errors`]. An app that never asks must not
/// grow without bound when one error repeats every frame.
pub const MAX_KEPT_ERRORS: usize = 64;

/// Keeps the first errors (the cause is usually among them), then one line saying more came.
fn keep_error(v: &mut Vec<String>, e: String) {
    keep_error_within(v, e, crate::gpu_limits().kept_errors);
}

fn keep_error_within(v: &mut Vec<String>, e: String, most: usize) {
    if v.len() < most {
        v.push(e);
    } else if v.len() == most {
        v.push(DROPPED_ERRORS.into());
    }
}

/// The last of the errors kept when more came than
/// [`GpuLimits::kept_errors`](crate::GpuLimits::kept_errors).
const DROPPED_ERRORS: &str = "more GPU errors came and were dropped";

/// The browser canvas id behind a window's handles, if it is one.
pub(crate) fn canvas_id(handles: &dyn WindowHandles) -> Option<u32> {
    use remedy_ui_core::host::raw_window_handle::RawWindowHandle;
    match handles.window_handle().ok()?.as_raw() {
        RawWindowHandle::Web(h) => Some(h.id),
        _ => None,
    }
}

impl GpuContext {
    /// Walks the ladder and returns the first rung that gives a device. A browser hands out
    /// devices only asynchronously, so there this fails with the reason and
    /// [`GpuContext::new_async`] makes the device instead.
    pub fn new(options: &LadderOptions) -> Result<GpuContext, GpuError> {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let _ = options;
            Err(GpuError::Device(
                "a browser gives a GPU device only asynchronously; use GpuContext::new_async".into(),
            ))
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            crate::block::block_on(Self::new_async(options, None))
        }
    }

    /// Walks the ladder and returns the first rung that gives a device, without blocking.
    /// `window` is the window the device will present to first: a browser's WebGL2 rung
    /// needs it (a WebGL2 device is made from, and belongs to, one canvas); elsewhere it is
    /// not needed.
    pub async fn new_async(
        options: &LadderOptions,
        window: Option<Arc<dyn WindowHandles>>,
    ) -> Result<GpuContext, GpuError> {
        let mut last = GpuError::NoAdapter;
        for &rung in &options.rungs {
            match Self::try_rung(rung, options.high_performance, window.clone()).await {
                Ok(ctx) if !options.self_test => return Ok(ctx),
                // A backend that drew the self-test wrong or hung presenting is skipped; one
                // not yet known is tested now (once per driver).
                Ok(ctx) => match crate::health::verdict(&crate::health::key(&ctx)) {
                    Some(v) if v.usable() => return Ok(ctx),
                    Some(v) => {
                        last = GpuError::Device(format!("{} is not used: {}", ctx.describe(), v.reason()))
                    }
                    None => match crate::health::self_test(&ctx) {
                        Ok(()) => return Ok(ctx),
                        Err(e) => {
                            last = GpuError::Device(format!("{} failed its self-test: {e}", ctx.describe()))
                        }
                    },
                },
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    async fn try_rung(
        rung: Rung,
        high_performance: bool,
        window: Option<Arc<dyn WindowHandles>>,
    ) -> Result<GpuContext, GpuError> {
        const WEB: bool = cfg!(all(target_arch = "wasm32", target_os = "unknown"));
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = match (rung, WEB) {
            (Rung::Primary, true) => wgpu::Backends::BROWSER_WEBGPU,
            // Browsers offer no software adapter to fall back to.
            (Rung::Software, true) => return Err(GpuError::NoAdapter),
            // `WGPU_BACKEND=dx12` (or `vulkan`) picks one backend where a machine has both.
            (Rung::Primary | Rung::Software, false) => wgpu::Backends::PRIMARY.with_env(),
            (Rung::Gl, _) => wgpu::Backends::GL,
        };
        #[cfg(windows)]
        if crate::ffi::d3d_debug::enabled() {
            // Test runs only: every D3D12 check, on the GPU too, and any message fails the run.
            desc.flags |= wgpu::InstanceFlags::advanced_debugging();
            crate::ffi::d3d_debug::install();
        }
        // `WGPU_VALIDATION`, `WGPU_GPU_BASED_VALIDATION` and the rest still override.
        desc.flags = desc.flags.with_env();
        let instance = wgpu::Instance::new(desc);
        // WebGL2 adapters come from a canvas's context; WebGPU and native adapters present
        // to any window. The WebGPU rung must not touch the canvas: a canvas that once gave
        // a WebGPU context can never give a WebGL2 one, which the next rung may need.
        let (surface, bound_canvas) = match (WEB && rung == Rung::Gl, window) {
            (true, Some(w)) => {
                // The canvas's `getContext("webgl2")` blocks the page until the browser's GL
                // is up (minutes, in headless Firefox): wait for it off the page's thread.
                #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                if crate::webgl_probe::webgl2().await == crate::webgl_probe::Probe::Unusable {
                    return Err(GpuError::NoAdapter);
                }
                let id = canvas_id(&*w);
                let s = instance.create_surface(w).map_err(|e| GpuError::Surface(e.to_string()))?;
                (Some(s), id)
            }
            (true, None) => {
                return Err(GpuError::Device(
                    "WebGL2 makes its device from a window's canvas; none was given".into(),
                ));
            }
            (false, _) => (None, None),
        };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: if high_performance {
                    wgpu::PowerPreference::HighPerformance
                } else {
                    wgpu::PowerPreference::LowPower
                },
                force_fallback_adapter: rung == Rung::Software,
                compatible_surface: surface.as_ref(),
                ..Default::default()
            })
            .await
            .map_err(|_| GpuError::NoAdapter)?;
        drop(surface);
        // Timestamps are asked for where the adapter has them: devtools traces put the GPU's
        // measured frame time on their timeline (nothing is written unless a trace records).
        // NV12 textures let decoded pictures be sampled where the decoder left them.
        let features = adapter.features()
            & (wgpu::Features::PIPELINE_CACHE
                | wgpu::Features::TIMESTAMP_QUERY
                | wgpu::Features::TEXTURE_FORMAT_NV12);
        let desc = wgpu::DeviceDescriptor {
            label: Some("RemedyUI"),
            required_features: features,
            required_limits: adapter.limits(),
            ..Default::default()
        };
        // On Vulkan the device also gets `VK_KHR_incremental_present` where it has it, so a
        // window's own swapchain can report only what changed, and what importing decoded
        // pictures needs (DMA-BUFs on Linux, AHardwareBuffers on Android).
        #[cfg(any(
            all(unix, not(any(target_os = "macos", target_os = "ios"))),
            all(windows, feature = "windows-vulkan")
        ))]
        let opened = crate::ffi::vulkan::open_device(&adapter, &desc);
        #[cfg(not(any(
            all(unix, not(any(target_os = "macos", target_os = "ios"))),
            all(windows, feature = "windows-vulkan")
        )))]
        let opened: Option<Result<(wgpu::Device, wgpu::Queue, bool), String>> = None;
        let incremental_present = matches!(opened, Some(Ok((_, _, true))));
        // A device that offers the extension but would not open with it is reported (below),
        // never passed over quietly: presents would then report the whole window.
        let mut open_failure = None;
        let (device, queue) = match opened {
            Some(Ok((d, q, _))) => (d, q),
            other => {
                if let Some(Err(e)) = other {
                    open_failure = Some(format!("opening the Vulkan device with its extensions: {e}"));
                }
                adapter.request_device(&desc).await.map_err(|e| GpuError::Device(e.to_string()))?
            }
        };
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |_, _| flag.store(true, Ordering::SeqCst));
        // wgpu panics on an uncaught validation error by default; an app must not crash for
        // a GPU mistake, so errors are recorded for diagnostics instead.
        let errors = Arc::new(Mutex::new(open_failure.into_iter().collect::<Vec<_>>()));
        let sink = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut v) = sink.lock() {
                keep_error(&mut v, e.to_string());
            }
        }));
        // Decoders hand this device's adapter pictures on the GPU from now on.
        crate::ffi::video::announce(&device);
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let textures = crate::external::Registry::default();
        Ok(GpuContext {
            instance,
            adapter,
            device,
            queue,
            rung,
            lost,
            errors,
            readback: Arc::default(),
            textures,
            effect_kernels: Arc::default(),
            serial,
            timer: Default::default(),
            bound_canvas,
            incremental_present,
        })
    }

    /// True once the device is lost (driver reset, GPU removed, or [`force_loss`]).
    ///
    /// [`force_loss`]: GpuContext::force_loss
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// Records that the device is lost, found by a call outside wgpu (a window's own
    /// swapchain): the next frame recovers, as after a loss wgpu reports.
    #[cfg_attr(
        not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios"))))),
        allow(dead_code)
    )]
    pub(crate) fn mark_lost(&self) {
        self.lost.store(true, Ordering::SeqCst);
    }

    /// Destroys the device as a driver reset would, to exercise recovery.
    pub fn force_loss(&self) {
        self.device.destroy();
        // Destroy reports the loss through the callback on the next poll.
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.lost.store(true, Ordering::SeqCst);
    }

    /// Frees what this device keeps only to go faster next time (the read back's staging
    /// buffer, 33 MB for a 4K frame), when the system asks for memory back.
    pub fn trim_memory(&self) {
        if let Ok(mut b) = self.readback.lock() {
            *b = None;
        }
    }

    /// GPU validation errors recorded since the last call (an empty list in a healthy run): the
    /// first [`GpuLimits::kept_errors`](crate::GpuLimits::kept_errors), then a line saying more
    /// were dropped.
    pub fn take_errors(&self) -> Vec<String> {
        self.errors.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
    }

    /// Records a GPU error found after the fact (a browser reports error scopes later).
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), allow(dead_code))]
    pub(crate) fn record_error(&self, e: String) {
        if let Ok(mut v) = self.errors.lock() {
            keep_error(&mut v, e);
        }
    }

    /// The adapter's name and backend, for logs and diagnostics.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?}, {:?})", info.name, info.backend, self.rung)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_repeating_gpu_error_keeps_a_bounded_list() {
        let mut v = Vec::new();
        for i in 0..1000 {
            super::keep_error(&mut v, format!("error {i}"));
        }
        assert_eq!(v.len(), super::MAX_KEPT_ERRORS + 1);
        assert_eq!(v.first().map(String::as_str), Some("error 0"), "the first errors are kept");
        assert_eq!(v.last().map(String::as_str), Some(super::DROPPED_ERRORS));
        // A raised bound keeps more.
        let mut v = Vec::new();
        for i in 0..1000 {
            super::keep_error_within(&mut v, format!("error {i}"), 500);
        }
        assert_eq!(v.len(), 501);
    }

    #[test]
    fn memory_pressure_frees_the_read_back_buffer_and_reading_still_works() {
        let Ok(ctx) = super::GpuContext::new(&crate::LadderOptions::default()) else {
            unreachable!("every machine has at least the software rung")
        };
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: 64, height: 8, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let held = |c: &super::GpuContext| c.readback.lock().map(|b| b.is_some()).unwrap_or(false);
        assert!(ctx.read_back(&texture).is_ok());
        assert!(held(&ctx), "kept for the next read back");
        ctx.trim_memory();
        assert!(!held(&ctx), "freed when the system asks for memory");
        assert!(ctx.read_back(&texture).is_ok_and(|f| f.pixels.len() == 64 * 8 * 4));
    }
}
