//! Executing compositor frame plans on the CPU: raster requested tiles, keep them, and
//! composite them with clips and opacity groups.

use std::collections::HashMap;
use std::sync::Arc;

use remedy_ui_compositor::{CompositeOp, FramePlan, LayerId, TileKey};
use remedy_ui_core::Color;
use vello_cpu::kurbo;
use vello_cpu::peniko::{ImageQuality, ImageSampler};
use vello_cpu::{ImageSource, Pixmap, RenderContext, Resources};

use crate::{CpuRenderer, Frame, RenderError, color, kaffine, rounded, size16};

/// Runs [`FramePlan`]s: the CPU half of the compositor. Keep one per window; it holds the
/// rastered tiles between frames.
pub struct CpuCompositor {
    renderer: CpuRenderer,
    tiles: HashMap<(LayerId, TileKey), Arc<Pixmap>>,
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
        CpuCompositor { renderer: CpuRenderer::new(), tiles: HashMap::new() }
    }

    /// Tiles held.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// Bytes of tile pixels held.
    pub fn tile_bytes(&self) -> usize {
        self.tiles.values().map(|p| p.data_as_u8_slice().len()).sum()
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
        let (w, h) = size16(width, height)?;
        for key in &plan.released {
            self.tiles.remove(key);
        }
        for req in &plan.raster {
            let pix = self.renderer.render_pixmap(
                &req.content,
                req.width,
                req.height,
                req.raster_transform(),
                Color::TRANSPARENT,
            )?;
            self.tiles.insert((req.layer, req.key), Arc::new(pix));
        }

        let mut ctx = RenderContext::new(w, h);
        ctx.set_paint(color(background));
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        for op in &plan.ops {
            match op {
                CompositeOp::PushClip { transform, rect, radii } => {
                    ctx.set_transform(kaffine(*transform));
                    ctx.push_clip_layer(&rounded(*rect, *radii));
                }
                CompositeOp::PopClip | CompositeOp::PopOpacity => ctx.pop_layer(),
                CompositeOp::PushOpacity(o) => ctx.push_opacity_layer(*o),
                CompositeOp::Tile { layer, key, transform, rect, raster_scale } => {
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
                    ctx.set_transform(kaffine(*transform));
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
                    ctx.fill_rect(&crate::krect(*rect));
                    ctx.reset_paint_transform();
                }
            }
        }
        ctx.flush();
        let mut target = Pixmap::new(w, h);
        ctx.render(&mut target, &mut Resources::new());
        Ok(Frame { width, height, pixels: target.data_as_u8_slice().to_vec() })
    }
}
