//! Getting a GPU: the backend ladder, and noticing when the device is lost.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::GpuError;
use crate::block::block_on;

/// Which rung of the backend ladder a context came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    /// A hardware adapter through D3D12, Vulkan or Metal.
    Primary,
    /// A hardware adapter through OpenGL 3.3 / GLES 3.0.
    Gl,
    /// A software adapter (WARP, lavapipe, llvmpipe): no GPU, still the GPU code path.
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
}

impl GpuContext {
    /// Walks the ladder and returns the first rung that gives a device.
    pub fn new(options: &LadderOptions) -> Result<GpuContext, GpuError> {
        let mut last = GpuError::NoAdapter;
        for &rung in &options.rungs {
            match Self::try_rung(rung, options.high_performance) {
                Ok(ctx) => return Ok(ctx),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn try_rung(rung: Rung, high_performance: bool) -> Result<GpuContext, GpuError> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = match rung {
            Rung::Primary | Rung::Software => wgpu::Backends::PRIMARY,
            Rung::Gl => wgpu::Backends::GL,
        };
        let instance = wgpu::Instance::new(desc);
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: if high_performance {
                wgpu::PowerPreference::HighPerformance
            } else {
                wgpu::PowerPreference::LowPower
            },
            force_fallback_adapter: rung == Rung::Software,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|_| GpuError::NoAdapter)?;
        let features = adapter.features() & wgpu::Features::PIPELINE_CACHE;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("RemedyUI"),
            required_features: features,
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |_, _| flag.store(true, Ordering::SeqCst));
        Ok(GpuContext { instance, adapter, device, queue, rung, lost })
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

    /// The adapter's name and backend, for logs and diagnostics.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?}, {:?})", info.name, info.backend, self.rung)
    }
}
