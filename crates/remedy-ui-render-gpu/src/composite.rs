//! Executing compositor frame plans on the GPU: tiles live in textures, the composite samples
//! them, and a lost device is rebuilt from the retained layer tree.

use std::collections::HashMap;

use remedy_ui_compositor::{CompositeOp, Compositor, FramePlan, LayerId, TileKey};
use remedy_ui_core::Color;
use remedy_ui_paint::Affine;
use remedy_ui_render_cpu::Frame;
use vello_gpu::kurbo;
use vello_gpu::peniko::{ImageQuality, ImageSampler};
use vello_gpu::{ImageSource, Scene, TextureId, TextureRegion};

use crate::renderer::size16;
use crate::{GpuContext, GpuError, GpuRenderer, LadderOptions};

/// A window's rastered tiles. Each window keeps its own: layer ids are per window.
#[derive(Default)]
pub(crate) struct TileSet(pub(crate) HashMap<(LayerId, TileKey), Tile>);

pub(crate) struct Tile {
    texture: wgpu::Texture,
    id: TextureId,
}

/// Runs [`FramePlan`]s on the GPU. Keep one per window.
pub struct GpuCompositor {
    renderer: GpuRenderer,
    ladder: LadderOptions,
    tiles: TileSet,
    next_id: u64,
    generation: u64,
}

impl std::fmt::Debug for GpuCompositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuCompositor({} tiles, {:?})", self.tiles.0.len(), self.renderer)
    }
}

fn kaffine(a: Affine) -> kurbo::Affine {
    kurbo::Affine::new(a.0.map(f64::from))
}

fn krect(r: remedy_ui_core::Rect) -> kurbo::Rect {
    kurbo::Rect::new(f64::from(r.min.x), f64::from(r.min.y), f64::from(r.max.x), f64::from(r.max.y))
}

impl GpuCompositor {
    /// A compositor on `renderer`. `ladder` is how a replacement device is found after a loss.
    pub fn new(renderer: GpuRenderer, ladder: LadderOptions) -> Self {
        GpuCompositor { renderer, ladder, tiles: TileSet::default(), next_id: 1, generation: 0 }
    }

    /// The renderer.
    pub fn renderer(&self) -> &GpuRenderer {
        &self.renderer
    }

    /// Bumped on every device rebuild; surfaces from an older device rebuild themselves.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Tiles held.
    pub fn tile_count(&self) -> usize {
        self.tiles.0.len()
    }

    /// True when the device is gone and [`recover`](Self::recover) must run before drawing.
    pub fn is_lost(&self) -> bool {
        self.renderer.ctx.is_lost()
    }

    /// Rebuilds after a device loss: a new device from the ladder, new pipelines, and every
    /// tile rastered again from the compositor's retained layer tree on the next frame. The
    /// app is not involved.
    pub fn recover(&mut self, compositor: &mut Compositor) -> Result<(), GpuError> {
        let ctx = GpuContext::new(&self.ladder)?;
        let store = self.renderer_store();
        self.renderer = GpuRenderer::new(ctx, store);
        self.recover_device()?;
        compositor.invalidate_tiles();
        Ok(())
    }

    /// Rebuilds the device after a loss. Every window's compositor must then forget its
    /// tiles ([`Compositor::invalidate_tiles`]); window surfaces rebuild themselves on their
    /// next present.
    pub fn recover_device(&mut self) -> Result<(), GpuError> {
        let ctx = GpuContext::new(&self.ladder)?;
        let store = self.renderer_store();
        self.renderer = GpuRenderer::new(ctx, store);
        self.tiles.0.clear();
        self.generation += 1;
        Ok(())
    }

    fn renderer_store(&self) -> Option<crate::PipelineCacheStore> {
        self.renderer.store_ref().cloned()
    }

    /// Frees released tiles, rasters requested ones, and composites into `target`.
    pub fn present(
        &mut self,
        plan: &FramePlan,
        target: &wgpu::Texture,
        background: Color,
    ) -> Result<(), GpuError> {
        let mut tiles = std::mem::take(&mut self.tiles);
        let prepared = self.prepare(&mut tiles, plan, target.width(), target.height());
        self.tiles = tiles;
        let (scene, bindings) = prepared?;
        self.renderer.submit(&scene, background, target, &bindings)
    }

    /// Frees released tiles, rasters requested ones, and records the composite of a
    /// `width` × `height` frame.
    pub(crate) fn prepare(
        &mut self,
        tiles: &mut TileSet,
        plan: &FramePlan,
        width: u32,
        height: u32,
    ) -> Result<(Scene, vello_gpu::TextureBindings), GpuError> {
        if self.is_lost() {
            return Err(GpuError::Lost);
        }
        for key in &plan.released {
            tiles.0.remove(key);
        }
        for req in &plan.raster {
            let texture = self.renderer.create_target(req.width, req.height)?;
            self.renderer.draw(&req.content, req.raster_transform(), Color::TRANSPARENT, &texture)?;
            let id = TextureId(self.next_id);
            self.next_id += 1;
            tiles.0.insert((req.layer, req.key), Tile { texture, id });
        }

        let (w, h) = size16(width, height)?;
        let mut scene = Scene::new(w, h);
        let mut bindings = vello_gpu::TextureBindings::new();
        for op in &plan.ops {
            match op {
                CompositeOp::PushClip { transform, rect, radii } => {
                    scene.set_transform(kaffine(*transform));
                    scene.push_clip_layer(&remedy_ui_render_cpu::rounded_path(*rect, *radii));
                }
                CompositeOp::PopClip | CompositeOp::PopOpacity => scene.pop_layer(),
                CompositeOp::PushOpacity(o) => scene.push_opacity_layer(*o),
                CompositeOp::Tile { layer, key, transform, rect, raster_scale } => {
                    let Some(tile) = tiles.0.get(&(*layer, *key)) else { continue };
                    bindings
                        .insert(tile.id, tile.texture.create_view(&wgpu::TextureViewDescriptor::default()));
                    let [a, b, c, d, e, f] = transform.0;
                    let exact = b == 0.0
                        && c == 0.0
                        && a == *raster_scale
                        && d == *raster_scale
                        && e.fract() == 0.0
                        && f.fract() == 0.0;
                    scene.set_transform(kaffine(*transform));
                    scene.set_paint(vello_gpu::Image {
                        image: ImageSource::external_texture(tile.id, TextureRegion::Full, true),
                        sampler: ImageSampler {
                            quality: if exact { ImageQuality::Low } else { ImageQuality::Medium },
                            ..Default::default()
                        },
                    });
                    let inv = 1.0 / f64::from(*raster_scale);
                    scene.set_paint_transform(
                        kurbo::Affine::translate((f64::from(rect.min.x), f64::from(rect.min.y)))
                            * kurbo::Affine::scale(inv),
                    );
                    scene.fill_rect(&krect(*rect));
                    scene.reset_paint_transform();
                }
            }
        }
        Ok((scene, bindings))
    }

    /// Presents into a new texture and reads it back (tests and snapshots).
    pub fn present_to_frame(
        &mut self,
        plan: &FramePlan,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<Frame, GpuError> {
        let target = self.renderer.create_target(width, height)?;
        self.present(plan, &target, background)?;
        self.renderer.read_back(&target)
    }
}
