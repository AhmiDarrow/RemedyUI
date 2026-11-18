//! Getting a GPU: the backend ladder, and noticing when the device is lost.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use remedy_ui_host::WindowHandles;

use crate::GpuError;

/// Which rung of the backend ladder a context came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
pub struct LadderOptions {
    /// Prefer the discrete GPU on machines with two.
    pub high_performance: bool,
    /// The rungs to try, top first.
    pub rungs: Vec<Rung>,
}

impl Default for LadderOptions {
    fn default() -> Self {
        LadderOptions { high_performance: true, rungs: vec![Rung::Primary, Rung::Gl, Rung::Software] }
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
    pub queue: wgpu::Queue,
    /// The rung it came from.
    pub rung: Rung,
    lost: Arc<AtomicBool>,
    errors: Arc<Mutex<Vec<String>>>,
    /// Textures the app renders and display lists place (`Item::Texture`).
    pub(crate) textures: crate::external::Registry,
    pub(crate) serial: u64,
    pub(crate) timer: crate::gputime::GpuTimer,
    /// The one window a browser's WebGL2 context draws on (its `data-raw-handle` id): a
    /// WebGL2 device belongs to the canvas it was made for. `None` everywhere else.
    pub(crate) bound_canvas: Option<u32>,
}

/// The browser canvas id behind a window's handles, if it is one.
pub(crate) fn canvas_id(handles: &dyn WindowHandles) -> Option<u32> {
    use remedy_ui_host::raw_window_handle::RawWindowHandle;
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
                Ok(ctx) => return Ok(ctx),
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
            (Rung::Primary | Rung::Software, false) => wgpu::Backends::PRIMARY,
            (Rung::Gl, _) => wgpu::Backends::GL,
        };
        let instance = wgpu::Instance::new(desc);
        // WebGL2 adapters come from a canvas's context; WebGPU and native adapters present
        // to any window. The WebGPU rung must not touch the canvas: a canvas that once gave
        // a WebGPU context can never give a WebGL2 one, which the next rung may need.
        let (surface, bound_canvas) = match (WEB && rung == Rung::Gl, window) {
            (true, Some(w)) => {
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
        let features =
            adapter.features() & (wgpu::Features::PIPELINE_CACHE | wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("RemedyUI"),
                required_features: features,
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .map_err(|e| GpuError::Device(e.to_string()))?;
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |_, _| flag.store(true, Ordering::SeqCst));
        // wgpu panics on an uncaught validation error by default; an app must not crash for
        // a GPU mistake, so errors are recorded for diagnostics instead.
        let errors = Arc::new(Mutex::new(Vec::new()));
        let sink = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut v) = sink.lock() {
                v.push(e.to_string());
            }
        }));
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
            textures,
            serial,
            timer: Default::default(),
            bound_canvas,
        })
    }

    /// True once the device is lost (driver reset, GPU removed, or [`force_loss`]).
    ///
    /// [`force_loss`]: GpuContext::force_loss
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// Destroys the device as a driver reset would, to exercise recovery.
    pub fn force_loss(&self) {
        self.device.destroy();
        // Destroy reports the loss through the callback on the next poll.
        let _ = self.device.poll(wgpu::PollType::Poll);
        self.lost.store(true, Ordering::SeqCst);
    }

    /// GPU validation errors recorded since the last call (an empty list in a healthy run).
    pub fn take_errors(&self) -> Vec<String> {
        self.errors.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
    }

    /// Records a GPU error found after the fact (a browser reports error scopes later).
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), allow(dead_code))]
    pub(crate) fn record_error(&self, e: String) {
        if let Ok(mut v) = self.errors.lock() {
            v.push(e);
        }
    }

    /// The adapter's name and backend, for logs and diagnostics.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?}, {:?})", info.name, info.backend, self.rung)
    }
}
