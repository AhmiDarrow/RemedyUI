//! Executing compositor frame plans on the GPU: tiles live in textures, the composite samples
//! them, and a lost device is rebuilt from the retained layer tree.

use std::collections::HashMap;

use remedy_ui_compositor::{Compositor, FramePlan, LayerId, TileKey};
use remedy_ui_core::{Color, Rect};
use remedy_ui_paint::Affine;
use remedy_ui_render_cpu::{CompositeTarget, Frame, record_ops};
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
    /// The frame [`present_to_frame`](Self::present_to_frame) keeps: only damage is drawn
    /// into it again.
    retained: Option<wgpu::Texture>,
    /// Pixels the latest damage-aware present composited.
    composited: u64,
    /// Bytes of tiles and of uploaded images held, for tools.
    gauges: (remedy_ui_trace::gauges::Gauge, remedy_ui_trace::gauges::Gauge),
}

impl std::fmt::Debug for GpuCompositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuCompositor({} tiles, {:?})", self.tiles.0.len(), self.renderer)
    }
}

/// What one more damaged rectangle costs the GPU, in pixels it composites in the same time
/// (a tuning constant): each is a scene, an encoding and a submit of its own. Measured on an
/// RTX 3080 at 4K: about 0.12 ms a rectangle, while compositing costs about 0.13 ms a million
/// pixels.
const RENDER_COST_PIXELS: f32 = 1_000_000.0;

/// The rectangles of `plan` to draw again ([`FramePlan::redraw_rects`]) as whole pixels
/// `(x0, y0, x1, y1)` within a `w` × `h` frame.
fn damage_rects(plan: &FramePlan, w: u16, h: u16, kept: bool) -> Vec<(u16, u16, u16, u16)> {
    plan.redraw_rects(u32::from(w), u32::from(h), kept, RENDER_COST_PIXELS)
        .into_iter()
        .filter_map(|r| {
            let (x0, y0) = (r.min.x.max(0.0) as u16, r.min.y.max(0.0) as u16);
            let (x1, y1) = (r.max.x.ceil().min(f32::from(w)) as u16, r.max.y.ceil().min(f32::from(h)) as u16);
            (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
        })
        .collect()
}

/// Records the composite of the part of the frame inside `area` (device pixels) as a scene of
/// `w` × `h` whose origin is `area`'s corner. `background` is drawn under a backdrop that
/// reaches the frame's start (the target is cleared to it first).
fn compose(
    tiles: &TileSet,
    plan: &FramePlan,
    area: Rect,
    (w, h): (u16, u16),
    background: Color,
    pictures: Pictures<'_>,
) -> (Scene, vello_gpu::TextureBindings) {
    let mut rec = Recorder {
        scene: Scene::new(w, h),
        bindings: vello_gpu::TextureBindings::new(),
        tiles,
        pictures,
        next_picture: u64::MAX - 1,
    };
    let shift = kurbo::Affine::translate((-f64::from(area.min.x), -f64::from(area.min.y)));
    record_ops(&mut rec, &plan.ops, 0, area, false, shift, background);
    (rec.scene, rec.bindings)
}

/// The GPU's [`CompositeTarget`]: a scene, the tiles it draws from, and the textures it binds.
struct Recorder<'a> {
    scene: Scene,
    bindings: vello_gpu::TextureBindings,
    tiles: &'a TileSet,
    pictures: Pictures<'a>,
    /// The id the next colour-matrix picture is bound under, counting down from the top
    /// (tiles count up from 1).
    next_picture: u64,
}

/// What renders a composite's colour-matrix groups to pictures: the renderer drawing the
/// composite (whose target format is `format`) and the matrix pass.
struct Pictures<'a> {
    ctx: &'a GpuContext,
    vello: &'a mut vello_gpu::Renderer,
    resources: &'a mut vello_gpu::Resources,
    matrix: &'a crate::matrix::MatrixPass,
    format: wgpu::TextureFormat,
}

impl CompositeTarget for Recorder<'_> {
    fn set_transform(&mut self, transform: kurbo::Affine) {
        self.scene.set_transform(transform);
    }
    fn push_clip_rect(&mut self, rect: &kurbo::Rect) {
        self.scene.push_clip_rect(rect);
    }
    fn push_clip_path(&mut self, path: &kurbo::BezPath) {
        self.scene.push_clip_path(path);
    }
    fn pop_clip(&mut self) {
        self.scene.pop_clip();
    }
    fn push_clip_layer(&mut self, path: &kurbo::BezPath) {
        self.scene.push_clip_layer(path);
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        self.scene.push_opacity_layer(opacity);
    }
    fn push_filter_layer(&mut self, filter: remedy_ui_render_cpu::LayerFilter) {
        self.scene.push_filter_layer(filter);
    }
    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }
    fn fill_color(&mut self, rect: &kurbo::Rect, c: Color) {
        self.scene
            .set_paint(vello_gpu::color::AlphaColor::<vello_gpu::color::Srgb>::new([c.r, c.g, c.b, c.a]));
        self.scene.fill_rect(rect);
    }
    fn fill_tile(
        &mut self,
        tile: (LayerId, TileKey),
        paint_transform: kurbo::Affine,
        exact: bool,
        part: &kurbo::Rect,
    ) {
        let Some(tile) = self.tiles.0.get(&tile) else { return };
        self.bindings.insert(tile.id, tile.texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.scene.set_paint(vello_gpu::Image {
            image: ImageSource::external_texture(tile.id, TextureRegion::Full, true),
            sampler: ImageSampler {
                quality: if exact { ImageQuality::Low } else { ImageQuality::Medium },
                ..Default::default()
            },
        });
        self.scene.set_paint_transform(paint_transform);
        self.scene.fill_rect(part);
        self.scene.reset_paint_transform();
    }
    fn size(&self) -> (u16, u16) {
        (self.scene.width(), self.scene.height())
    }
    fn color_matrix_group(
        &mut self,
        rect: [u16; 4],
        matrix: &remedy_ui_core::ColorMatrix,
        draw: &mut dyn FnMut(&mut Self),
    ) -> bool {
        let [_, _, w, h] = rect;
        if w == 0 || h == 0 {
            return true;
        }
        let outer = std::mem::replace(&mut self.scene, Scene::new(w, h));
        draw(self);
        let picture = std::mem::replace(&mut self.scene, outer);
        let p = &mut self.pictures;
        // A failure here is a lost device, which the frame reports when it is submitted.
        if let Ok(texture) =
            p.matrix.picture(p.ctx, p.vello, p.resources, &picture, p.format, &self.bindings, matrix)
        {
            let id = TextureId(self.next_picture);
            self.next_picture -= 1;
            self.bindings.insert(id, texture.create_view(&wgpu::TextureViewDescriptor::default()));
            crate::matrix::place(&mut self.scene, id, rect);
        }
        true
    }
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
            retained: None,
            composited: 0,
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

    /// Frees what this compositor and its device keep only to go faster next time (the small
    /// tiles' atlas, the read back's staging buffer), when the system asks for memory back.
    /// Tiles go through the compositor's own [`Compositor::trim_memory`].
    pub fn trim_memory(&mut self) {
        self.atlas = None;
        self.renderer.context().trim_memory();
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
        self.retained = None;
        self.generation += 1;
        Ok(())
    }

    fn renderer_store(&self) -> Option<crate::PipelineCacheStore> {
        self.renderer.store_ref().cloned()
    }

    /// Frees released tiles, rasters requested ones, and composites the whole frame into
    /// `target` (whose earlier content is not kept).
    pub fn present(
        &mut self,
        plan: &FramePlan,
        target: &wgpu::Texture,
        background: Color,
    ) -> Result<(), GpuError> {
        let mut tiles = std::mem::take(&mut self.tiles);
        let prepared = self.prepare(&mut tiles, plan, target.width(), target.height(), background);
        self.tiles = tiles;
        let (scene, bindings) = prepared?;
        self.renderer.submit(&scene, background, target, &bindings)
    }

    /// Pixels the latest [`present_to_frame`](Self::present_to_frame) or window present
    /// composited: the whole frame on a full redraw, only the damage (and a pixel around it)
    /// otherwise.
    pub fn last_composited(&self) -> u64 {
        self.composited
    }

    /// Rasters the plan's tiles and composites its damage into `frame`, the frame kept between
    /// presents (made, or made again at a new size, as needed: then all of it is drawn). Only
    /// the damaged rectangles are drawn, each as a scene of its own size placed at its corner,
    /// with the rest of the frame left as it was; damage over most of the frame draws it all.
    /// `with` is the renderer for `format` (a window's), or `None` for this compositor's own.
    #[allow(clippy::too_many_arguments, reason = "a frame's target and its renderer")]
    pub(crate) fn render_damage(
        &mut self,
        tiles: &mut TileSet,
        plan: &FramePlan,
        frame: &mut Option<wgpu::Texture>,
        format: wgpu::TextureFormat,
        (width, height): (u32, u32),
        background: Color,
        with: Option<(&mut vello_gpu::Renderer, &mut vello_gpu::Resources)>,
    ) -> Result<(), GpuError> {
        if self.is_lost() {
            return Err(GpuError::Lost);
        }
        let (w, h) = size16(width, height)?;
        for key in &plan.released {
            tiles.0.remove(key);
        }
        self.raster(tiles, plan)?;
        let reuse = frame
            .as_ref()
            .is_some_and(|f| (f.width(), f.height()) == (width, height) && f.format() == format);
        if !reuse {
            *frame = Some(self.renderer.context().device.create_texture(&wgpu::TextureDescriptor {
                label: Some("RemedyUI frame"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }
        let Some(target) = frame.as_ref() else { return Err(GpuError::BadSize) };
        let rects = damage_rects(plan, w, h, reuse);
        self.composited = 0;
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let ctx = self.renderer.context().clone();
        let (vello, resources) = match with {
            Some((v, r)) => (v, r),
            None => (&mut self.renderer.vello, &mut self.renderer.resources),
        };
        for (x0, y0, x1, y1) in rects {
            let (rw, rh) = (x1 - x0, y1 - y0);
            self.composited += u64::from(rw) * u64::from(rh);
            let area = Rect::new(f32::from(x0), f32::from(y0), f32::from(x1), f32::from(y1));
            let clear = [vello_gpu::RectU16 { x0, y0, x1, y1 }];
            let color =
                vello_gpu::color::AlphaColor::new([background.r, background.g, background.b, background.a]);
            let mut options = vello_gpu::RenderOptions::default();
            options.target_init = if (rw, rh) == (w, h) {
                vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::Viewport { color })
            } else {
                vello_gpu::TargetInit::Clear(vello_gpu::ClearSettings::Rects { color, rects: &clear })
            };
            // A scene of the rectangle's own size, placed at its corner: the GPU draws only it.
            options.offset = (i32::from(x0), i32::from(y0));
            let pictures = Pictures { ctx: &ctx, vello, resources, matrix: &self.renderer.matrix, format };
            let (scene, bindings) = compose(tiles, plan, area, (rw, rh), background, pictures);
            crate::renderer::submit_scene_with(&ctx, vello, resources, &scene, &view, &bindings, options)?;
        }
        self.measure(tiles);
        Ok(())
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
    /// `width` × `height` frame over `background` (the target's clear color, which glass at
    /// the frame's start blurs too).
    pub(crate) fn prepare(
        &mut self,
        tiles: &mut TileSet,
        plan: &FramePlan,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<(Scene, vello_gpu::TextureBindings), GpuError> {
        if self.is_lost() {
            return Err(GpuError::Lost);
        }
        for key in &plan.released {
            tiles.0.remove(key);
        }
        self.raster(tiles, plan)?;
        let (w, h) = size16(width, height)?;
        // The background is the target's clear color; a backdrop at the frame's start draws it.
        let r = &mut self.renderer;
        let pictures = Pictures {
            ctx: &r.ctx,
            vello: &mut r.vello,
            resources: &mut r.resources,
            matrix: &r.matrix,
            format: crate::FORMAT,
        };
        let area = Rect::new(0.0, 0.0, f32::from(w), f32::from(h));
        let out = compose(tiles, plan, area, (w, h), background, pictures);
        self.measure(tiles);
        Ok(out)
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
            // Filters blur on a grid that starts at the corner of what is drawn: a tile with
            // one draws on its own, at its own corner, like the CPU's.
            if req.width <= ATLAS_TILE && req.height <= ATLAS_TILE && !req.content.has_filters() {
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

    /// Presents into the frame this compositor keeps and reads it back (tests, snapshots,
    /// thumbnails). Like a window, it draws only the plan's damage into what the previous
    /// call drew (all of it the first time, or at a new size), so hand it every plan.
    pub fn present_to_frame(
        &mut self,
        plan: &FramePlan,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<Frame, GpuError> {
        self.present_to_kept(plan, width, height, background)?;
        let Some(t) = &self.retained else { return Err(GpuError::BadSize) };
        self.renderer.read_back(t)
    }

    /// [`present_to_frame`](Self::present_to_frame) without the read back: the frame stays
    /// on the GPU (measuring compositing alone, or drawing ahead of a later snapshot).
    pub fn present_to_kept(
        &mut self,
        plan: &FramePlan,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<(), GpuError> {
        let mut tiles = std::mem::take(&mut self.tiles);
        let mut retained = self.retained.take();
        let done = self.render_damage(
            &mut tiles,
            plan,
            &mut retained,
            crate::FORMAT,
            (width, height),
            background,
            None,
        );
        self.tiles = tiles;
        self.retained = retained;
        done
    }
}
