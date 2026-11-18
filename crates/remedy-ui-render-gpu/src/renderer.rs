//! Drawing display lists into GPU textures, and reading them back.

use remedy_ui_core::Color;
use remedy_ui_paint::{Affine, DisplayList};
use std::collections::HashMap;
use std::sync::Arc;

use remedy_ui_render_cpu::{Frame, ImageKey, Painter, Surface};
use vello_gpu::kurbo::{self, BezPath, Stroke};
use vello_gpu::peniko::{BlendMode, Fill};
use vello_gpu::{ImageSource, PaintType, Pixmap, Scene, TextureId, Tint};

use crate::images::GpuImages;
use crate::video::GpuVideos;

use crate::{GpuContext, GpuError, PipelineCacheStore};

/// The texture format RemedyUI renders in: premultiplied RGBA8.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A `vello_gpu` scene as a [`Surface`], so the shared [`Painter`] draws onto it. Images and
/// glyphs go to GPU textures through the renderer's image store.
pub(crate) struct GpuScene<'a> {
    pub(crate) scene: Scene,
    images: &'a mut GpuImages,
    videos: &'a mut GpuVideos,
    externals: &'a mut HashMap<u64, TextureId>,
    ctx: &'a GpuContext,
}

impl Surface for GpuScene<'_> {
    fn set_transform(&mut self, t: kurbo::Affine) {
        self.scene.set_transform(t);
    }
    fn set_paint(&mut self, paint: impl Into<PaintType>) {
        self.scene.set_paint(paint);
    }
    fn set_paint_transform(&mut self, t: kurbo::Affine) {
        self.scene.set_paint_transform(t);
    }
    fn reset_paint_transform(&mut self) {
        self.scene.reset_paint_transform();
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        self.scene.set_fill_rule(rule);
    }
    fn set_stroke(&mut self, stroke: Stroke) {
        self.scene.set_stroke(stroke);
    }
    fn set_tint(&mut self, tint: Option<Tint>) {
        self.scene.set_tint(tint);
    }
    fn fill_path(&mut self, path: &BezPath) {
        self.scene.fill_path(path);
    }
    fn fill_rect(&mut self, rect: &kurbo::Rect) {
        self.scene.fill_rect(rect);
    }
    fn stroke_path(&mut self, path: &BezPath) {
        self.scene.stroke_path(path);
    }
    fn fill_blurred_rounded_rect(&mut self, rect: &kurbo::Rect, radius: f32, std_dev: f32) {
        self.scene.fill_blurred_rounded_rect(rect, radius, std_dev);
    }
    fn set_blend_mode(&mut self, mode: BlendMode) {
        self.scene.set_blend_mode(mode);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        self.scene.push_clip_layer(path);
    }
    fn push_group(&mut self, blend: BlendMode, opacity: f32) {
        self.scene.push_layer(None, Some(blend), Some(opacity), None, None);
    }
    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }
    fn image_source(&mut self, key: ImageKey, pixmap: &Arc<Pixmap>, filtered: bool) -> Option<ImageSource> {
        self.images.source(&self.ctx.device, &self.ctx.queue, key, pixmap, filtered)
    }
    fn video_source(&mut self, frame: &remedy_ui_paint::VideoFrame) -> Option<ImageSource> {
        let images = &mut *self.images;
        let shown =
            self.videos.texture(&self.ctx.device, &self.ctx.queue, frame, &mut || images.fresh_id())?;
        if let Some(old) = shown.replaced {
            images.unbind(&[old]);
        }
        if let Some(view) = shown.new_view {
            images.bind_external(shown.id, view);
        }
        Some(ImageSource::external_texture(shown.id, vello_gpu::TextureRegion::Full, true))
    }
    fn texture_source(&mut self, texture: &remedy_ui_paint::ExternalTexture) -> Option<ImageSource> {
        let id = match self.externals.get(&texture.id) {
            Some(id) => *id,
            None => {
                let reg = self.ctx.textures.get(texture.id)?;
                if reg.width != texture.width || reg.height != texture.height {
                    return None;
                }
                let id = self.images.fresh_id();
                self.images.bind_external(id, reg.view);
                self.externals.insert(texture.id, id);
                id
            }
        };
        Some(ImageSource::external_texture(id, vello_gpu::TextureRegion::Full, true))
    }
}

pub(crate) fn size16(width: u32, height: u32) -> Result<(u16, u16), GpuError> {
    let w = u16::try_from(width).map_err(|_| GpuError::BadSize)?;
    let h = u16::try_from(height).map_err(|_| GpuError::BadSize)?;
    if w == 0 || h == 0 {
        return Err(GpuError::BadSize);
    }
    Ok((w, h))
}

/// The GPU renderer: display lists to textures through `vello_gpu`, with the same painter
/// (and therefore the same glyphs and shapes) as the CPU renderer.
pub struct GpuRenderer {
    pub(crate) ctx: GpuContext,
    pub(crate) vello: vello_gpu::Renderer,
    pub(crate) resources: vello_gpu::Resources,
    pub(crate) painter: Painter,
    images: GpuImages,
    videos: GpuVideos,
    /// Registered app textures bound for drawing: registration id to the bound id.
    externals: HashMap<u64, TextureId>,
    pipeline_cache: Option<wgpu::PipelineCache>,
    store: Option<PipelineCacheStore>,
}

impl std::fmt::Debug for GpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuRenderer({}, {:?})", self.ctx.describe(), self.painter)
    }
}

impl GpuRenderer {
    /// A renderer on `ctx`. With a store, pipelines come from (and go back to) the persistent
    /// pipeline cache; every pipeline is created here, at start-up, never mid-frame.
    pub fn new(ctx: GpuContext, store: Option<PipelineCacheStore>) -> Self {
        let pipeline_cache = store.as_ref().and_then(|s| s.open(&ctx));
        let mut settings = vello_gpu::RenderSettings::default();
        if let Some(c) = &pipeline_cache {
            settings = settings.with_pipeline_cache(c.clone());
        }
        let config = vello_gpu::RenderTargetConfig { format: FORMAT, width: 1, height: 1 };
        let (vello, resources) = vello_gpu::Renderer::new_with(&ctx.device, &ctx.queue, &config, settings);
        let images = GpuImages::new(&ctx.device);
        let videos = GpuVideos::new(&ctx.device, pipeline_cache.as_ref());
        GpuRenderer {
            ctx,
            vello,
            resources,
            painter: Painter::new(),
            images,
            videos,
            externals: HashMap::new(),
            pipeline_cache,
            store,
        }
    }

    pub(crate) fn store_ref(&self) -> Option<&PipelineCacheStore> {
        self.store.as_ref()
    }

    /// Bytes of GPU memory held by uploaded glyphs and images.
    pub fn image_bytes(&self) -> u64 {
        self.images.bytes() + self.videos.bytes()
    }

    /// The device this renderer draws with.
    pub fn context(&self) -> &GpuContext {
        &self.ctx
    }

    /// Writes the pipeline cache to disk. Returns false when the driver keeps its own.
    pub fn save_pipeline_cache(&self) -> std::io::Result<bool> {
        match (&self.store, &self.pipeline_cache) {
            (Some(s), Some(c)) => s.save(&self.ctx, c),
            _ => Ok(false),
        }
    }

    /// Drops glyph and image caches (memory pressure).
    pub fn clear_caches(&mut self) {
        self.painter.clear_caches();
        let _ = self.videos.clear();
        self.images.clear();
        self.externals.clear();
        self.vello.reset_and_shrink(&mut self.resources);
    }

    /// A texture RemedyUI can render into, sample from, and read back.
    pub fn create_target(&self, width: u32, height: u32) -> Result<wgpu::Texture, GpuError> {
        size16(width, height)?;
        Ok(self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RemedyUI target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }))
    }

    /// Draws `list` through `base` onto `target`, cleared to `background` first.
    pub fn draw(
        &mut self,
        list: &DisplayList,
        base: Affine,
        background: Color,
        target: &wgpu::Texture,
    ) -> Result<(), GpuError> {
        let (w, h) = size16(target.width(), target.height())?;
        // Registrations the app has dropped since: let go of their textures.
        let gone: Vec<TextureId> = self
            .externals
            .iter()
            .filter(|(reg, _)| !self.ctx.textures.contains(**reg))
            .map(|(_, id)| *id)
            .collect();
        if !gone.is_empty() {
            self.externals.retain(|reg, _| self.ctx.textures.contains(*reg));
            self.images.unbind(&gone);
        }
        let mut scene = GpuScene {
            scene: Scene::new(w, h),
            images: &mut self.images,
            videos: &mut self.videos,
            externals: &mut self.externals,
            ctx: &self.ctx,
        };
        let area = remedy_ui_core::Rect::new(0.0, 0.0, f32::from(w), f32::from(h));
        self.painter.draw_within(&mut scene, list, base, Some(area));
        let scene = scene.scene;
        let gone = self.painter.take_evicted();
        if !gone.is_empty() {
            self.images.evict(&gone);
        }
        let streams = self.painter.take_evicted_videos();
        if !streams.is_empty() {
            let ids = self.videos.evict(&streams);
            self.images.unbind(&ids);
        }
        let bindings = self.images.bindings.clone();
        self.submit(&scene, background, target, &bindings)
    }

    pub(crate) fn submit(
        &mut self,
        scene: &Scene,
        background: Color,
        target: &wgpu::Texture,
        bindings: &vello_gpu::TextureBindings,
    ) -> Result<(), GpuError> {
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        submit_scene(&self.ctx, &mut self.vello, &mut self.resources, scene, background, &view, bindings)
    }

    /// Renders `list` at `scale` onto `background` and reads the pixels back: snapshots,
    /// golden images, thumbnails.
    pub fn render(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
        scale: f32,
        background: Color,
    ) -> Result<Frame, GpuError> {
        let target = self.create_target(width, height)?;
        self.draw(list, Affine::scale(scale, scale), background, &target)?;
        self.read_back(&target)
    }

    /// Copies a target's pixels back to memory (premultiplied RGBA8).
    pub fn read_back(&self, texture: &wgpu::Texture) -> Result<Frame, GpuError> {
        self.ctx.read_back(texture)
    }
}

impl GpuContext {
    /// Copies the pixels of a 4-byte-per-pixel texture made with `COPY_SRC` usage back to
    /// memory, waiting for the GPU (snapshots, tests, and GPU content shown by the CPU
    /// renderer).
    pub fn read_back(&self, texture: &wgpu::Texture) -> Result<Frame, GpuError> {
        if texture.format().block_copy_size(None) != Some(4) {
            return Err(GpuError::Readback(format!("{:?} is not 4 bytes per pixel", texture.format())));
        }
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return Err(GpuError::Readback("the texture was not made with COPY_SRC usage".into()));
        }
        let (width, height) = (texture.width(), texture.height());
        let row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = row.div_ceil(align) * align;
        let device = &self.device;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RemedyUI readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI copy") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let mapped = buffer.slice(..).get_mapped_range().map_err(|e| GpuError::Readback(e.to_string()))?;
        let mut pixels = Vec::with_capacity(row as usize * height as usize);
        for y in 0..height as usize {
            let start = y * padded as usize;
            let line =
                mapped.get(start..start + row as usize).ok_or(GpuError::Readback("short buffer".into()))?;
            pixels.extend_from_slice(line);
        }
        drop(mapped);
        buffer.unmap();
        Ok(Frame { width, height, pixels })
    }
}

/// Records and submits `scene` into `view` with `vello`, cleared to `background` first.
pub(crate) fn submit_scene(
    ctx: &GpuContext,
    vello: &mut vello_gpu::Renderer,
    resources: &mut vello_gpu::Resources,
    scene: &Scene,
    background: Color,
    view: &wgpu::TextureView,
    bindings: &vello_gpu::TextureBindings,
) -> Result<(), GpuError> {
    if ctx.is_lost() {
        return Err(GpuError::Lost);
    }
    let mut encoder =
        ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("RemedyUI frame") });
    let clear = vello_gpu::ClearSettings::Viewport {
        color: vello_gpu::color::AlphaColor::new([background.r, background.g, background.b, background.a]),
    };
    let mut options = vello_gpu::RenderOptions::default();
    options.target_init = vello_gpu::TargetInit::Clear(clear);
    vello
        .render(
            scene,
            resources,
            &mut encoder,
            vello_gpu::RenderTarget { view, depth_view: None },
            bindings,
            options,
        )
        .map_err(|e| GpuError::Render(e.to_string()))?;
    ctx.queue.submit([encoder.finish()]);
    Ok(())
}
