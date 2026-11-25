//! How much the GPU renderer keeps and how long it waits: every bound, with its default, set
//! by the app with [`set_gpu_limits`]. What it draws is bounded by
//! [`PaintLimits`](crate::paint::PaintLimits), which the CPU renderer shares.

use std::sync::{PoisonError, RwLock};
use std::time::Duration;

use crate::render_gpu::MAX_KEPT_ERRORS;
use crate::render_gpu::health::PRESENT_HANG;

/// The default of [`GpuLimits::health_entries`].
pub const MAX_HEALTH_ENTRIES: usize = 64;
/// The default of [`GpuLimits::health_line`].
pub const MAX_HEALTH_LINE: usize = 512;
/// The default of [`GpuLimits::swapchain_wait`].
pub const SWAPCHAIN_WAIT: Duration = Duration::from_secs(1);
/// The default of [`GpuLimits::video_fence_wait`].
pub const VIDEO_FENCE_WAIT: Duration = Duration::from_secs(1);
/// The default of [`GpuLimits::video_imports`].
pub const MAX_VIDEO_IMPORTS: usize = 64;

/// The GPU renderer's bounds, for every device in the process ([`set_gpu_limits`]). Each
/// default suits ordinary apps.
///
/// What changing each risks: a longer `swapchain_wait` or `video_fence_wait` blocks the UI
/// thread that long when the system or a decoder stalls (a shorter one skips a frame or
/// shows the picture before its writes are done sooner); a longer `present_hang` keeps a
/// hung backend longer before the ladder lets it go (Windows resets a GPU silent for two
/// seconds); `kept_errors`, `health_entries` and `health_line` bound memory and the health
/// file against an error repeating every frame or a hand-edited file.
///
/// ```
/// use std::time::Duration;
/// use remedy_ui_engine::render_gpu::{GpuLimits, set_gpu_limits};
/// set_gpu_limits(GpuLimits::default().kept_errors(256).swapchain_wait(Duration::from_millis(250)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GpuLimits {
    /// GPU errors kept between calls to
    /// [`GpuContext::take_errors`](crate::render_gpu::GpuContext::take_errors); then one line says more
    /// were dropped. Default [`MAX_KEPT_ERRORS`] (64).
    pub kept_errors: usize,
    /// Backends the [health](crate::render_gpu::health) file remembers. Default [`MAX_HEALTH_ENTRIES`]
    /// (64).
    pub health_entries: usize,
    /// The longest line read from the health file; a backend's key is kept to half of it.
    /// Changing it changes keys longer than the old half, so those backends test again.
    /// Default [`MAX_HEALTH_LINE`] (512).
    pub health_line: usize,
    /// How long a present may run before its backend counts as hung, for watchdogs made
    /// from now on. Default [`PRESENT_HANG`] (3 s).
    pub present_hang: Duration,
    /// RemedyUI's own swapchains (Vulkan on Linux and with `windows-vulkan`, DXGI on
    /// Windows): how long to wait for the next image before reporting the swapchain busy
    /// (the frame is skipped). wgpu's own swapchain, used everywhere else, has its own wait.
    /// Default [`SWAPCHAIN_WAIT`] (1 s).
    pub swapchain_wait: Duration,
    /// Linux DMA-BUF video: how long a decoded picture's writes are waited for on the CPU
    /// before it is shown anyway. Default [`VIDEO_FENCE_WAIT`] (1 s).
    pub video_fence_wait: Duration,
    /// Hardware video (Linux DMA-BUF, Windows D3D12): decoder pictures kept imported per
    /// stream before the oldest are let go (a decoder's pool is a few dozen). Default
    /// [`MAX_VIDEO_IMPORTS`] (64).
    pub video_imports: usize,
}

impl GpuLimits {
    /// The defaults, as a constant.
    pub const DEFAULT: GpuLimits = GpuLimits {
        kept_errors: MAX_KEPT_ERRORS,
        health_entries: MAX_HEALTH_ENTRIES,
        health_line: MAX_HEALTH_LINE,
        present_hang: PRESENT_HANG,
        swapchain_wait: SWAPCHAIN_WAIT,
        video_fence_wait: VIDEO_FENCE_WAIT,
        video_imports: MAX_VIDEO_IMPORTS,
    };
}

impl Default for GpuLimits {
    fn default() -> Self {
        GpuLimits::DEFAULT
    }
}

macro_rules! setters {
    ($($name:ident: $t:ty),* $(,)?) => {
        impl GpuLimits {
            $(
                #[doc = concat!("Sets [`", stringify!($name), "`](Self::", stringify!($name), ").")]
                pub fn $name(mut self, value: $t) -> Self {
                    self.$name = value;
                    self
                }
            )*
        }
    };
}

setters! {
    kept_errors: usize,
    health_entries: usize,
    health_line: usize,
    present_hang: Duration,
    swapchain_wait: Duration,
    video_fence_wait: Duration,
    video_imports: usize,
}

static LIMITS: RwLock<GpuLimits> = RwLock::new(GpuLimits::DEFAULT);

/// Sets the GPU renderer's bounds for every device in the process, from their next use.
pub fn set_gpu_limits(limits: GpuLimits) {
    *LIMITS.write().unwrap_or_else(PoisonError::into_inner) = limits;
}

/// The GPU renderer's bounds now ([`set_gpu_limits`]).
pub fn gpu_limits() -> GpuLimits {
    *LIMITS.read().unwrap_or_else(PoisonError::into_inner)
}
