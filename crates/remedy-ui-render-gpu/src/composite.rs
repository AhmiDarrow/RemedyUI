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

/// Tiles at most this many pixels on a side are rastered together in an atlas.
const ATLAS_TILE: u32 = 256;
/// The atlas's width; its height grows to fit, up to [`ATLAS_HEIGHT`] a page.
const ATLAS_WIDTH: u32 = 1024;
const ATLAS_HEIGHT: u32 = 2048;

/// Runs [`FramePlan`]s on the GPU. Keep one per window.
pub struct GpuCompositor {
    renderer: GpuRenderer,
    ladder: LadderOptions,
    tiles: TileSet,
    next_id: u64,
    generation: u64,
    /// The atlas small tiles are rastered in, kept between frames.
    atlas: Option<wgpu::Texture>,
    /// Bytes of tiles and of uploaded images held, for tools.
    gauges: (remedy_ui_trace::gauges::Gauge, remedy_ui_trace::gauges::Gauge),
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

/// A graphics adapter as people read it ([`GpuCompositor::adapter`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdapterSummary {
    /// The device's name ("NVIDIA GeForce RTX 3080").
    pub name: String,
    /// The maker's PCI vendor id (0x10DE NVIDIA, 0x1002 AMD, 0x8086 Intel), 0 when unknown.
    pub vendor: u32,
    /// The driver's name and version, where the platform says.
    pub driver: String,
    /// The graphics API in use ("Vulkan", "Dx12", "Metal", "Gl").
    pub api: String,
    /// A software rasterizer (no graphics card).
    pub software: bool,
}

impl GpuCompositor {
    /// A compositor on `renderer`. `ladder` is how a replacement device is found after a loss.
    pub fn new(renderer: GpuRenderer, ladder: LadderOptions) -> Self {
        GpuCompositor {
            renderer,
            ladder,
            tiles: TileSet::default(),
            next_id: 1,
            generation: 0,
            atlas: None,
            gauges: (
                remedy_ui_trace::gauges::Gauge::new("gpu tiles"),
                remedy_ui_trace::gauges::Gauge::new("gpu images"),
            ),
        }
    }

    /// The renderer.
    pub fn renderer(&self) -> &GpuRenderer {
        &self.renderer
    }

    /// The graphics adapter drawing: its name, maker, driver and the graphics API used, for a
    /// "drawing with" line in an app's settings or a bug report.
    pub fn adapter(&self) -> AdapterSummary {
        let i = self.renderer.context().adapter.get_info();
        AdapterSummary {
            name: i.name,
            vendor: i.vendor,
            driver: [i.driver, i.driver_info]
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            api: format!("{:?}", i.backend),
            software: i.device_type == wgpu::DeviceType::Cpu,
        }
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
        let ctx = GpuContext::shared(&self.ladder)?;
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
        // The process's shared device, replaced: content embedded from the app moves to the
        // same new device.
        let ctx = GpuContext::shared(&self.ladder)?;
        let store = self.renderer_store();
        self.renderer = GpuRenderer::new(ctx, store);
        self.tiles.0.clear();
        self.atlas = None;
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

    /// Updates the gauges of what this compositor holds on the GPU: the tiles of the set in
    /// use and the images uploaded.
    fn measure(&self, tiles: &TileSet) {
        let bytes: u64 = tiles
            .0
            .values()
            .map(|t| {
                let s = t.texture.size();
                u64::from(s.width) * u64::from(s.height) * 4
            })
            .sum();
        self.gauges.0.set(bytes);
        self.gauges.1.set(self.renderer.image_bytes());
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
        self.raster(tiles, plan)?;

        let (w, h) = size16(width, height)?;
        let mut scene = Scene::new(w, h);
        let mut bindings = vello_gpu::TextureBindings::new();
        for op in &plan.ops {
            match op {
                CompositeOp::PushClip { transform, rect, radii } => {
                    // A clip path, not an isolated clip layer: clipping needs no layer of its
                    // own, and one per clipped element made every composite that many groups.
                    scene.set_transform(kaffine(*transform));
                    if *radii == remedy_ui_core::CornerRadii::ZERO {
                        scene.push_clip_rect(&krect(*rect));
                    } else {
                        scene.push_clip_path(&remedy_ui_render_cpu::rounded_path(*rect, *radii));
                    }
                }
                CompositeOp::PopClip => scene.pop_clip(),
                CompositeOp::PopOpacity => scene.pop_layer(),
                CompositeOp::PushOpacity(o) => scene.push_opacity_layer(*o),
                CompositeOp::Tile { layer, key, transform, rect, raster_scale, area } => {
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
                    scene.fill_rect(&krect(*area));
                    scene.reset_paint_transform();
                }
            }
        }
        self.measure(tiles);
        Ok((scene, bindings))
    }

    /// Rasters the plan's tile requests. A tile drawn again at the same size keeps its texture.
    /// Small tiles (an animated sprite's, a blinking caret's) are drawn together into one
    /// atlas in a single render and copied out, since every render has a fixed cost that
    /// dwarfs a small tile's pixels; large ones render on their own.
    fn raster(&mut self, tiles: &mut TileSet, plan: &FramePlan) -> Result<(), GpuError> {
        let mut small = Vec::new();
        for req in &plan.raster {
            // A texture of the right size: the tile's own when it has one.
            let reuse = tiles
                .0
                .get(&(req.layer, req.key))
                .is_some_and(|t| (t.texture.width(), t.texture.height()) == (req.width, req.height));
            if !reuse {
                let texture = self.renderer.create_target(req.width, req.height)?;
                let id = TextureId(self.next_id);
                self.next_id += 1;
                tiles.0.insert((req.layer, req.key), Tile { texture, id });
            }
            if req.width <= ATLAS_TILE && req.height <= ATLAS_TILE {
                small.push(req);
            } else if let Some(tile) = tiles.0.get(&(req.layer, req.key)) {
                self.renderer.draw(
                    &req.content,
                    req.raster_transform(),
                    Color::TRANSPARENT,
                    &tile.texture,
                )?;
            }
        }
        if small.len() < 2 {
            for req in small {
                if let Some(tile) = tiles.0.get(&(req.layer, req.key)) {
                    self.renderer.draw(
                        &req.content,
                        req.raster_transform(),
                        Color::TRANSPARENT,
                        &tile.texture,
                    )?;
                }
            }
            return Ok(());
        }
        // Shelf packing into atlas pages ATLAS_WIDTH wide, a pixel apart.
        let mut pages: Vec<Vec<(&remedy_ui_compositor::TileRequest, [u32; 4])>> = vec![Vec::new()];
        let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
        for req in small {
            if x + req.width > ATLAS_WIDTH {
                x = 0;
                y += shelf + 1;
                shelf = 0;
            }
            if y + req.height > ATLAS_HEIGHT {
                pages.push(Vec::new());
                (x, y, shelf) = (0, 0, 0);
            }
            if let Some(page) = pages.last_mut() {
                page.push((req, [x, y, req.width, req.height]));
            }
            x += req.width + 1;
            shelf = shelf.max(req.height);
        }
        for page in pages.iter().filter(|p| !p.is_empty()) {
            let height = page.iter().map(|(_, [_, y, _, h])| y + h).max().unwrap_or(1).max(1);
            let width = page.iter().map(|(_, [x, _, w, _])| x + w).max().unwrap_or(1).max(1);
            let atlas = match self.atlas.take() {
                Some(a) if a.width() >= width && a.height() >= height => a,
                _ => {
                    self.renderer.create_target(ATLAS_WIDTH, height.next_power_of_two().min(ATLAS_HEIGHT))?
                }
            };
            let jobs: Vec<(&remedy_ui_paint::DisplayList, Affine, [u32; 4])> =
                page.iter().map(|(r, slot)| (&*r.content, r.raster_transform(), *slot)).collect();
            self.renderer.draw_many(&jobs, &atlas)?;
            let device = &self.renderer.context().device;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("RemedyUI tile atlas"),
            });
            for (req, [x, y, w, h]) in page {
                let Some(tile) = tiles.0.get(&(req.layer, req.key)) else { continue };
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &atlas,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: *x, y: *y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &tile.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
                );
            }
            self.renderer.context().queue.submit([encoder.finish()]);
            self.atlas = Some(atlas);
        }
        Ok(())
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
