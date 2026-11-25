//! The persistent pipeline cache: pipelines compile once per driver, not once per start.

use std::path::PathBuf;

use crate::render_gpu::GpuContext;

const MAGIC: &[u8; 8] = b"RUIPIPE1";

/// FNV-1a: detects a torn or corrupted cache file (not an adversary: the file is the user's own).
fn checksum(data: &[u8]) -> u64 {
    data.iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Where the pipeline cache lives on disk. Drivers without pipeline-cache support (D3D12 and
/// Metal cache compiled pipelines themselves) get no file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineCacheStore {
    dir: PathBuf,
}

impl PipelineCacheStore {
    /// A store in `dir` (an app's cache directory).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        PipelineCacheStore { dir: dir.into() }
    }

    fn file(&self, ctx: &GpuContext) -> Option<PathBuf> {
        if !ctx.device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            return None;
        }
        let key = wgpu::util::pipeline_cache_key(&ctx.adapter.get_info())?;
        let safe: String =
            key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
        Some(self.dir.join(format!("{safe}.pipelines")))
    }

    /// Opens the cache for this device: seeded from disk when a valid file exists, empty
    /// otherwise. `None` when the driver has no pipeline-cache support.
    pub fn open(&self, ctx: &GpuContext) -> Option<wgpu::PipelineCache> {
        let path = self.file(ctx)?;
        let bytes = std::fs::read(&path).ok();
        let data = bytes.as_deref().and_then(|b| {
            let (head, body) = (b.get(..16)?, b.get(16..)?);
            let sum = u64::from_le_bytes(head.get(8..16)?.try_into().ok()?);
            (head.get(..8)? == MAGIC && checksum(body) == sum).then_some(body)
        });
        Some(crate::render_gpu::ffi::create_pipeline_cache(&ctx.device, data))
    }

    /// Writes the cache back (after start-up compiled what it needed). Written to a temporary
    /// file and renamed, so a crash never leaves a torn cache.
    pub fn save(&self, ctx: &GpuContext, cache: &wgpu::PipelineCache) -> std::io::Result<bool> {
        let (Some(path), Some(body)) = (self.file(ctx), cache.get_data()) else { return Ok(false) };
        std::fs::create_dir_all(&self.dir)?;
        let mut out = Vec::with_capacity(body.len() + 16);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&checksum(&body).to_le_bytes());
        out.extend_from_slice(&body);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &out)?;
        std::fs::rename(&tmp, &path)?;
        Ok(true)
    }
}
