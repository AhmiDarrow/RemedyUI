//! The GPU renderer's unsafe calls.

/// Creates a pipeline cache seeded with `data`.
///
/// `data` must have been returned by `PipelineCache::get_data` on an adapter with the same
/// `wgpu::util::pipeline_cache_key`. [`PipelineCacheStore`](crate::PipelineCacheStore) only
/// passes bytes it wrote itself for that key, checked by its own header and checksum, and
/// wgpu re-validates the driver's header (falling back to an empty cache on any mismatch).
pub(crate) fn create_pipeline_cache(device: &wgpu::Device, data: Option<&[u8]>) -> wgpu::PipelineCache {
    // SAFETY: see above. `fallback: true` makes the driver discard data it does not accept
    // instead of failing.
    unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("RemedyUI pipelines"),
            data,
            fallback: true,
        })
    }
}

/// Decoded pictures on the GPU, opened on the renderer's device.
pub(crate) mod video;

/// Linux: decoded pictures in DMA-BUFs, imported on wgpu's Vulkan device.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub(crate) mod dmabuf;

/// Windows: a swapchain of RemedyUI's own on wgpu's D3D12 device, presenting only what changed.
#[cfg(windows)]
pub(crate) mod dxgi;

/// Linux and Android (and Windows tests): a swapchain of RemedyUI's own on wgpu's Vulkan
/// device, presenting only what changed.
#[cfg(any(
    all(unix, not(any(target_os = "macos", target_os = "ios"))),
    all(windows, feature = "windows-vulkan")
))]
pub(crate) mod vulkan;

/// The last thing done to a swapchain image being drawn, which the return to the present
/// state starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios")))))]
pub(crate) enum LastUse {
    /// Nothing: it is as the swapchain handed it out.
    Untouched,
    /// Copied or drawn into.
    Written,
    /// Read back after it was written (tests).
    Read,
}
