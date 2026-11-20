//! Executing compositor frame plans on the CPU: raster requested tiles, keep them, and
//! composite them with clips and opacity groups.

use std::collections::HashMap;
use std::sync::Arc;

use remedy_ui_compositor::{CompositeOp, FramePlan, LayerId, TileKey};
use remedy_ui_core::{Color, CornerRadii, Rect};
use vello_cpu::kurbo;
use vello_cpu::peniko::{ImageQuality, ImageSampler};
use vello_cpu::{ImageSource, Pixmap, RenderContext, Resources};

use crate::{CpuRenderer, Frame, RenderError, color, kaffine, rounded, size16};

/// Runs [`FramePlan`]s: the CPU half of the compositor. Keep one per window; it holds the
/// rastered tiles between frames.
pub struct CpuCompositor {
    renderer: CpuRenderer,
    tiles: HashMap<(LayerId, TileKey), Arc<Pixmap>>,
    last_raster: (u64, std::time::Duration),
    /// Bytes of tiles held, for tools.
    gauge: remedy_ui_trace::gauges::Gauge,
    /// The last frame composited: parts the damage leaves alone are kept from it.
    last: Option<Frame>,
    /// The composite's render context, kept between frames.
    ctx: crate::Reused,
    /// Pixels composited by the latest present.
    composited: u64,
}

impl std::fmt::Debug for CpuCompositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CpuCompositor({} tiles, {:?})", self.tiles.len(), self.renderer)
    }
}

impl Default for CpuCompositor {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuCompositor {
    /// A compositor with no tiles.
    pub fn new() -> Self {
        CpuCompositor {
            renderer: CpuRenderer::new(),
            tiles: HashMap::new(),
            last_raster: (0, std::time::Duration::ZERO),
            gauge: remedy_ui_trace::gauges::Gauge::new("cpu tiles"),
            last: None,
            ctx: crate::Reused::default(),
            composited: 0,
        }
    }

    /// Tiles held.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Bytes of tile pixels held.
    pub fn tile_bytes(&self) -> usize {
        self.tiles.values().map(|p| p.data_as_u8_slice().len()).sum()
    }

    /// What the latest [`present`](Self::present) rastered: pixels, and the time it took.
    /// Hand it to [`Compositor::record_raster`](remedy_ui_compositor::Compositor::record_raster)
    /// so zooms raster within the frame budget on this machine.
    pub fn last_raster(&self) -> (u64, std::time::Duration) {
        self.last_raster
    }

    /// Drops glyph and image caches; tiles are freed through the plan's `released` list.
    pub fn clear_caches(&mut self) {
        self.renderer.clear_caches();
    }

    /// Frees released tiles, rasters requested ones, and composites the frame onto
    /// `background`. The frame is complete; the plan's damage says which part a bridge needs
    /// to present.
    pub fn present(
        &mut self,
        plan: &FramePlan,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<Frame, RenderError> {
        self.present_frame(plan, width, height, background).cloned()
    }

    /// [`present`](Self::present) without copying the frame: the compositor keeps it (the
    /// next frame starts from it), and this borrows it. What a bridge presents every frame.
    pub fn present_frame(
        &mut self,
        plan: &FramePlan,
        width: u32,
        height: u32,
        background: Color,
    ) -> Result<&Frame, RenderError> {
        let _raster = remedy_ui_trace::phase!("raster").entered();
        let (w, h) = size16(width, height)?;
        for key in &plan.released {
            self.tiles.remove(key);
        }
        let started = remedy_ui_core::time::Instant::now();
        let mut pixels = 0u64;
        for req in &plan.raster {
            pixels += req.pixels();
            let pix = self.renderer.render_pixmap(
                &req.content,
                req.width,
                req.height,
                req.raster_transform(),
                Color::TRANSPARENT,
            )?;
            self.tiles.insert((req.layer, req.key), Arc::new(pix));
        }
        self.last_raster = (pixels, started.elapsed());

        // Only what changed is composited again; the rest of the last frame is kept. The
        // composite is recorded once for the whole window and only the damaged rectangles are
        // rasterized from it. A frame of a new size, the first frame, or damage over most of
        // the window draws it all.
        let full = Rect::new(0.0, 0.0, f32::from(w), f32::from(h));
        let reuse = self.last.as_ref().is_some_and(|f| (f.width, f.height) == (width, height));
        let rects: Vec<Rect> = if reuse && plan.damage.area() < full.size().area() * 0.6 {
            // One pixel more around each: a filtered tile may touch the pixel past its box.
            plan.damage
                .rects()
                .iter()
                .filter_map(|r| {
                    Rect::new(r.min.x - 1.0, r.min.y - 1.0, r.max.x + 1.0, r.max.y + 1.0).intersect(full)
                })
                .filter(|r| !r.is_empty())
                .collect()
        } else {
            vec![full]
        };
        let mut frame = match self.last.take() {
            Some(f) if (f.width, f.height) == (width, height) => f.pixels,
            _ => vec![0u8; usize::from(w) * usize::from(h) * 4],
        };
        self.composited = 0;
        if !rects.is_empty() {
            let mut resources = Resources::new();
            for r in rects {
                let (x0, y0) = (r.min.x.max(0.0) as u16, r.min.y.max(0.0) as u16);
                let (x1, y1) =
                    (r.max.x.ceil().min(f32::from(w)) as u16, r.max.y.ceil().min(f32::from(h)) as u16);
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let (rw, rh) = (x1 - x0, y1 - y0);
                self.composited += u64::from(rw) * u64::from(rh);
                let area = Rect::new(f32::from(x0), f32::from(y0), f32::from(x1), f32::from(y1));
                let ctx = self.record(plan, background, area, rw, rh);
                let mut part = Pixmap::new(rw, rh);
                ctx.render(&mut part, &mut resources);
                self.ctx.keep(ctx);
                let src = part.data_as_u8_slice();
                if (rw, rh) == (w, h) {
                    frame.copy_from_slice(src);
                    continue;
                }
                let row = usize::from(rw) * 4;
                for y in 0..usize::from(rh) {
                    let dst = ((usize::from(y0) + y) * usize::from(w) + usize::from(x0)) * 4;
                    if let (Some(d), Some(s)) =
                        (frame.get_mut(dst..dst + row), src.get(y * row..(y + 1) * row))
                    {
                        d.copy_from_slice(s);
                    }
                }
            }
        }
        self.gauge.set(self.tile_bytes() as u64);
        let last = self.last.insert(Frame { width, height, pixels: frame });
        Ok(last)
    }

    /// Pixels the latest [`present`](Self::present) composited: the whole window on a full
    /// frame, only the damage (and a pixel around it) otherwise.
    pub fn last_composited(&self) -> u64 {
        self.composited
    }

    /// Records the composite of the part of the frame inside `area` (device pixels, `w` × `h`)
    /// into a (reused) render context. What lies outside it is left out: tiles that miss it,
    /// and whole clipped subtrees whose clip misses it.
    fn record(&mut self, plan: &FramePlan, background: Color, area: Rect, w: u16, h: u16) -> RenderContext {
        let shift = kurbo::Affine::translate((-f64::from(area.min.x), -f64::from(area.min.y)));
        let mut ctx = self.ctx.take(w, h);
        ctx.set_paint(color(background));
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        // Inside a skipped subtree: how many pushes deep.
        let mut skip = 0u32;
        for op in &plan.ops {
            if skip > 0 {
                match op {
                    CompositeOp::PushClip { .. } | CompositeOp::PushOpacity(_) => skip += 1,
                    CompositeOp::PopClip | CompositeOp::PopOpacity => skip -= 1,
                    CompositeOp::Tile { .. } => {}
                }
                continue;
            }
            match op {
                CompositeOp::PushClip { transform, rect, .. }
                    if transform.apply_rect(*rect).intersect(area).is_none_or(|r| r.is_empty()) =>
                {
                    skip = 1;
                }
                CompositeOp::PushClip { transform, rect, radii } => {
                    // A clip path, not a clip layer: clipping needs no isolated buffer, and a
                    // layer per clipped element made every composite cost as many groups.
                    ctx.set_transform(shift * kaffine(*transform));
                    if *radii == CornerRadii::ZERO {
                        ctx.push_clip_rect(&crate::krect(*rect));
                    } else {
                        ctx.push_clip_path(&rounded(*rect, *radii));
                    }
                }
                CompositeOp::PopClip => ctx.pop_clip(),
                CompositeOp::PopOpacity => ctx.pop_layer(),
                CompositeOp::PushOpacity(o) => ctx.push_opacity_layer(*o),
                CompositeOp::Tile { layer, key, transform, rect, raster_scale, area: part } => {
                    if transform.apply_rect(*part).intersect(area).is_none_or(|r| r.is_empty()) {
                        continue;
                    }
                    let Some(pix) = self.tiles.get(&(*layer, *key)) else { continue };
                    // Pixel-exact placement (the common, static case) samples nearest so tiles
                    // stay crisp and seamless; scaled or rotated tiles filter.
                    let [a, b, c, d, e, f] = transform.0;
                    let exact = b == 0.0
                        && c == 0.0
                        && a == *raster_scale
                        && d == *raster_scale
                        && e.fract() == 0.0
                        && f.fract() == 0.0;
                    ctx.set_transform(shift * kaffine(*transform));
                    ctx.set_paint(vello_cpu::Image {
                        image: ImageSource::Pixmap(pix.clone()),
                        sampler: ImageSampler {
                            quality: if exact { ImageQuality::Low } else { ImageQuality::Medium },
                            ..Default::default()
                        },
                    });
                    let inv = 1.0 / f64::from(*raster_scale);
                    ctx.set_paint_transform(
                        kurbo::Affine::translate((f64::from(rect.min.x), f64::from(rect.min.y)))
                            * kurbo::Affine::scale(inv),
                    );
                    ctx.fill_rect(&crate::krect(*part));
                    ctx.reset_paint_transform();
                }
            }
        }
        ctx.flush();
        ctx
    }
}
