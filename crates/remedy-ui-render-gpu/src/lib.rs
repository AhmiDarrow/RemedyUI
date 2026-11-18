//! RemedyUI's GPU renderer.
//!
//! Runs on wgpu (D3D12, Vulkan, Metal, OpenGL 3.3 / GLES 3.0), found by a backend ladder that
//! ends in a software adapter. Display lists draw through `vello_gpu` with the same
//! [`Painter`](remedy_ui_render_cpu::Painter) as the CPU renderer, so both draw the same glyphs
//! and shapes; compositor tiles live in textures and composite with clips and opacity groups.
//! Pipelines are created at start-up from a persistent cache where the driver supports one. A
//! lost device is replaced and every tile rastered again from the retained layer tree with no
//! app code. See `docs/design/04-rendering.md`.

mod block;
mod cache;
mod composite;
mod context;
mod external;
#[allow(unsafe_code)]
mod ffi;
mod gputime;
mod images;
mod renderer;
mod surface;
mod video;

pub use cache::PipelineCacheStore;
pub use composite::GpuCompositor;
pub use context::{GpuContext, LadderOptions, Rung};
pub use renderer::{FORMAT, GpuRenderer};
pub use surface::{GpuSurface, Presented};
pub use wgpu;

/// Why the GPU renderer could not draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuError {
    /// No adapter on any rung of the ladder.
    NoAdapter,
    /// The adapter refused a device.
    Device(String),
    /// The device was lost; recover before drawing again.
    Lost,
    /// The size is zero or beyond 65,535 pixels on an axis.
    BadSize,
    /// Drawing failed.
    Render(String),
    /// Reading pixels back failed.
    Readback(String),
    /// A window surface could not be made or used.
    Surface(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter => f.write_str("no GPU adapter, hardware or software"),
            GpuError::Device(e) => write!(f, "the adapter refused a device: {e}"),
            GpuError::Lost => f.write_str("the GPU device was lost"),
            GpuError::BadSize => f.write_str("the target size is zero or beyond 65,535 pixels on an axis"),
            GpuError::Render(e) => write!(f, "drawing failed: {e}"),
            GpuError::Readback(e) => write!(f, "reading pixels back failed: {e}"),
            GpuError::Surface(e) => write!(f, "the window surface failed: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}
