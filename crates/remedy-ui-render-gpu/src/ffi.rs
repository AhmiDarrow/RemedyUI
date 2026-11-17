//! The one unsafe call in the GPU renderer.

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
