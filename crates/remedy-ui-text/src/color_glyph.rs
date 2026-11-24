//! Color glyph rasterization: COLRv0/v1, CBDT/CBLC and sbix to premultiplied RGBA.
//!
//! Outline-only rasterization cannot draw these glyphs: a CBDT face such as the Noto Color
//! Emoji most Linux machines install has no `glyf` table, and a COLR face's outlines are
//! only the layer shapes, not a finished emoji. This module paints them into a
//! [`GlyphImage`](crate::GlyphImage) of format [`GlyphImageFormat::Color`].

use std::io::Cursor;

use remedy_ui_core::Color;
use skrifa::bitmap::{BitmapData, BitmapGlyph, BitmapStrikes, Origin};
use skrifa::color::{Brush, ColorGlyph, ColorPainter, ColorPalettes, ColorStop, CompositeMode};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::DrawSettings;
use skrifa::raw::types::BoundingBox;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::raster::{GlyphImage, GlyphImageFormat, PathRecorder};

const FOREGROUND_PALETTE: u16 = 0xFFFF;
const PAD: i32 = 2;

/// True when `glyph` has a COLR, CBDT/CBLC, EBDT or sbix image that should be painted in
/// color instead of as a hinted outline.
pub fn is_color_glyph(font: &FontRef<'_>, glyph: u32) -> bool {
    let id = GlyphId::new(glyph);
    if font.color_glyphs().get(id).is_some() {
        return true;
    }
    BitmapStrikes::new(font).glyph_for_size(Size::unscaled(), id).is_some()
}

/// Paints a color glyph at `size` pixels (the em square). `foreground` is the text color,
/// used when a COLR layer asks for the foreground palette entry (`0xFFFF`). Returns `None`
/// when the glyph has no color image.
pub fn rasterize(font: &FontRef<'_>, glyph: u32, size: f32, foreground: Color) -> Option<GlyphImage> {
    rasterize_transformed(font, glyph, size, [1.0, 0.0, 0.0, 1.0], (0.0, 0.0), foreground)
}

/// Like [`rasterize`], with an extra linear map (`[a, b, c, d]`: x' = a·x + c·y, y' = b·x +
/// d·y, y down, in device pixels per pixel of `size`) and a fractional origin `offset`.
pub fn rasterize_transformed(
    font: &FontRef<'_>,
    glyph: u32,
    size: f32,
    linear: [f32; 4],
    offset: (f32, f32),
    foreground: Color,
) -> Option<GlyphImage> {
    let size = size.max(0.0);
    if size == 0.0 {
        return None;
    }
    let id = GlyphId::new(glyph);
    if let Some(color) = font.color_glyphs().get(id) {
        return rasterize_colr(font, &color, id, size, linear, offset, foreground);
    }
    if let Some(bitmap) = BitmapStrikes::new(font).glyph_for_size(Size::new(size), id) {
        return rasterize_bitmap(font, &bitmap, size, linear, offset);
    }
    None
}

fn rasterize_colr(
    font: &FontRef<'_>,
    glyph: &ColorGlyph<'_>,
    _id: GlyphId,
    size: f32,
    linear: [f32; 4],
    offset: (f32, f32),
    foreground: Color,
) -> Option<GlyphImage> {
    let upem = font.metrics(Size::unscaled(), LocationRef::default()).units_per_em as f32;
    if upem <= 0.0 {
        return None;
    }
    let scale = size / upem;
    // Font units (y up) to device pixels (y down): the linear part plus the origin fraction.
    let [a, b, c, d] = linear;
    let root =
        Mat { xx: a * scale, yx: b * scale, xy: -c * scale, yy: -d * scale, dx: offset.0, dy: offset.1 };
    let palettes = ColorPalettes::new(font);
    let palette = palettes.get(0);
    let mut bounds = BoundsPainter { font: font.clone(), ctm: vec![root], bounds: None };
    if glyph.paint(LocationRef::default(), &mut bounds).is_err() {
        return None;
    }
    // COLRv1 may also publish a clip box in font units.
    if let Some(bb) = glyph.bounding_box(LocationRef::default(), Size::unscaled()) {
        bounds.include_box(root.apply_box(bb));
    }
    let (min_x, min_y, max_x, max_y) = bounds.bounds?;
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    let left = min_x.floor() as i32 - PAD;
    let top_y = min_y.floor() as i32 - PAD;
    let right = max_x.ceil() as i32 + PAD;
    let bottom = max_y.ceil() as i32 + PAD;
    let width = (right - left).clamp(1, 4096) as u32;
    let height = (bottom - top_y).clamp(1, 4096) as u32;
    let pix = Pixmap::new(width, height);
    let mut painter = LayerPainter {
        font: font.clone(),
        palette,
        foreground,
        ctm: vec![root],
        clips: vec![Clip::None],
        layers: vec![pix],
        modes: vec![CompositeMode::SrcOver],
        origin: (left as f32, top_y as f32),
    };
    glyph.paint(LocationRef::default(), &mut painter).ok()?;
    let pix = painter.layers.pop()?;
    if !pix.has_ink() {
        return None;
    }
    // GlyphImage.top is the offset from the baseline *up* to the image's top edge. Device y
    // grows down, so the top edge is at `left/top_y` in device space; baseline is y = 0.
    Some(GlyphImage { width, height, left, top: -top_y, format: GlyphImageFormat::Color, data: pix.data })
}

fn rasterize_bitmap(
    font: &FontRef<'_>,
    glyph: &BitmapGlyph<'_>,
    size: f32,
    linear: [f32; 4],
    offset: (f32, f32),
) -> Option<GlyphImage> {
    let (src_w, src_h, mut rgba) = decode_bitmap(glyph)?;
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let ppem_x = if glyph.ppem_x > 0.0 { glyph.ppem_x } else { 1.0 };
    let ppem_y = if glyph.ppem_y > 0.0 { glyph.ppem_y } else { 1.0 };
    // Uniform scale from the strike, then the run's linear map (a, d are the axis scales).
    let [a, b, c, d] = linear;
    let sx = (size / ppem_x) * (a * a + b * b).sqrt().max(0.0);
    let sy = (size / ppem_y) * (c * c + d * d).sqrt().max(0.0);
    // A nearly identity linear map (the usual upright run) uses sx/sy only.
    let dst_w = ((src_w as f32) * sx).round().clamp(1.0, 4096.0) as u32;
    let dst_h = ((src_h as f32) * sy).round().clamp(1.0, 4096.0) as u32;
    if dst_w != src_w || dst_h != src_h {
        rgba = scale_rgba(&rgba, src_w, src_h, dst_w, dst_h);
    }
    let upem = font.metrics(Size::unscaled(), LocationRef::default()).units_per_em as f32;
    let fu = if upem > 0.0 { size / upem } else { 0.0 };
    // inner bearings are in strike pixels; outer bearings are in font units.
    let bearing_x = glyph.bearing_x * fu + glyph.inner_bearing_x * (size / ppem_x);
    let bearing_y = glyph.bearing_y * fu + glyph.inner_bearing_y * (size / ppem_y);
    let left = (bearing_x + offset.0).round() as i32;
    // TopLeft: bearing_y is the distance from the baseline up to the top of the bitmap.
    // BottomLeft (sbix): the image sits on the baseline, plus the inner offset.
    let top = match glyph.placement_origin {
        Origin::TopLeft => (bearing_y - offset.1).round() as i32,
        Origin::BottomLeft => ((bearing_y + dst_h as f32) - offset.1).round() as i32,
    };
    Some(GlyphImage { width: dst_w, height: dst_h, left, top, format: GlyphImageFormat::Color, data: rgba })
}

fn decode_bitmap(glyph: &BitmapGlyph<'_>) -> Option<(u32, u32, Vec<u8>)> {
    match &glyph.data {
        BitmapData::Png(bytes) => decode_png(bytes),
        BitmapData::Bgra(bytes) => {
            let w = glyph.width;
            let h = glyph.height;
            let n = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
            // Already premultiplied BGRA; swizzle to RGBA.
            let mut out = Vec::with_capacity(n);
            for &[b, g, r, a] in bytes.get(..n)?.as_chunks::<4>().0 {
                out.extend_from_slice(&[r, g, b, a]);
            }
            Some((w, h, out))
        }
        BitmapData::Mask(mask) => {
            let w = glyph.width;
            let h = glyph.height;
            let mut cov = vec![0u8; (w as usize).checked_mul(h as usize)?];
            mask.decode_to_slice(w, h, &mut cov).ok()?;
            let mut out = Vec::with_capacity(cov.len() * 4);
            for a in cov {
                out.extend_from_slice(&[a, a, a, a]);
            }
            Some((w, h, out))
        }
    }
}

fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let w = info.width;
    let h = info.height;
    let n = (w as usize).checked_mul(h as usize)?;
    let src = buf.get(..info.buffer_size())?;
    // The first `n * channels` bytes of the frame, or None when the frame is shorter.
    let pixels = |channels: usize| n.checked_mul(channels).and_then(|len| src.get(..len));
    let premul = |c: u8, a: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
    let mut out = Vec::with_capacity(n.checked_mul(4)?);
    match info.color_type {
        png::ColorType::Rgba => {
            for &[r, g, b, a] in pixels(4)?.as_chunks::<4>().0 {
                out.extend_from_slice(&[premul(r, a), premul(g, a), premul(b, a), a]);
            }
        }
        png::ColorType::Rgb => {
            for &[r, g, b] in pixels(3)?.as_chunks::<3>().0 {
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for &[g, a] in pixels(2)?.as_chunks::<2>().0 {
                let g = premul(g, a);
                out.extend_from_slice(&[g, g, g, a]);
            }
        }
        png::ColorType::Grayscale => {
            for &g in pixels(1)? {
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        _ => return None,
    }
    Some((w, h, out))
}

fn scale_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    // Bilinear, premultiplied. Tight loops; emoji strikes are a few hundred pixels.
    if sw == 0 || sh == 0 {
        return vec![0u8; dw as usize * dh as usize * 4];
    }
    let mut out = Vec::with_capacity(dw as usize * dh as usize * 4);
    let x_ratio = (sw as f32) / (dw as f32);
    let y_ratio = (sh as f32) / (dh as f32);
    for y in 0..dh {
        let fy = (y as f32 + 0.5) * y_ratio - 0.5;
        let y0 = fy.floor().clamp(0.0, (sh - 1) as f32) as u32;
        let y1 = (y0 + 1).min(sh - 1);
        let ty = (fy - y0 as f32).clamp(0.0, 1.0);
        for x in 0..dw {
            let fx = (x as f32 + 0.5) * x_ratio - 0.5;
            let x0 = fx.floor().clamp(0.0, (sw - 1) as f32) as u32;
            let x1 = (x0 + 1).min(sw - 1);
            let tx = (fx - x0 as f32).clamp(0.0, 1.0);
            // A pixel past the end of a short source reads as transparent.
            let sample = |xx: u32, yy: u32| match src.get((yy * sw + xx) as usize * 4..) {
                Some(&[r, g, b, a, ..]) => [r, g, b, a].map(f32::from),
                _ => [0.0; 4],
            };
            let top = mix(sample(x0, y0), sample(x1, y0), tx);
            let bottom = mix(sample(x0, y1), sample(x1, y1), tx);
            let px = mix(top, bottom, ty).map(|v| v.round().clamp(0.0, 255.0) as u8);
            out.extend_from_slice(&px);
        }
    }
    out
}

fn mix([a0, a1, a2, a3]: [f32; 4], [b0, b1, b2, b3]: [f32; 4], t: f32) -> [f32; 4] {
    let s = 1.0 - t;
    [a0 * s + b0 * t, a1 * s + b1 * t, a2 * s + b2 * t, a3 * s + b3 * t]
}

#[derive(Clone, Copy)]
struct Mat {
    xx: f32,
    yx: f32,
    xy: f32,
    yy: f32,
    dx: f32,
    dy: f32,
}

impl Mat {
    const IDENTITY: Self = Self { xx: 1.0, yx: 0.0, xy: 0.0, yy: 1.0, dx: 0.0, dy: 0.0 };

    fn mul(self, t: Self) -> Self {
        // self * t: apply t first, then self.
        Self {
            xx: self.xx * t.xx + self.xy * t.yx,
            yx: self.yx * t.xx + self.yy * t.yx,
            xy: self.xx * t.xy + self.xy * t.yy,
            yy: self.yx * t.xy + self.yy * t.yy,
            dx: self.xx * t.dx + self.xy * t.dy + self.dx,
            dy: self.yx * t.dx + self.yy * t.dy + self.dy,
        }
    }

    fn from_skrifa(t: skrifa::color::Transform) -> Self {
        Self { xx: t.xx, yx: t.yx, xy: t.xy, yy: t.yy, dx: t.dx, dy: t.dy }
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (self.xx * x + self.xy * y + self.dx, self.yx * x + self.yy * y + self.dy)
    }

    fn apply_box(self, bb: BoundingBox<f32>) -> (f32, f32, f32, f32) {
        let pts = [
            self.apply(bb.x_min, bb.y_min),
            self.apply(bb.x_max, bb.y_min),
            self.apply(bb.x_min, bb.y_max),
            self.apply(bb.x_max, bb.y_max),
        ];
        let xs: Vec<f32> = pts.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p.1).collect();
        (
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            ys.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        )
    }
}

struct BoundsPainter<'a> {
    font: FontRef<'a>,
    ctm: Vec<Mat>,
    bounds: Option<(f32, f32, f32, f32)>,
}

impl BoundsPainter<'_> {
    fn ctm(&self) -> Mat {
        // The root transform is never popped, so the identity is not reached.
        self.ctm.last().copied().unwrap_or(Mat::IDENTITY)
    }

    fn include(&mut self, x: f32, y: f32) {
        match &mut self.bounds {
            None => self.bounds = Some((x, y, x, y)),
            Some((x0, y0, x1, y1)) => {
                *x0 = x0.min(x);
                *y0 = y0.min(y);
                *x1 = x1.max(x);
                *y1 = y1.max(y);
            }
        }
    }

    fn include_box(&mut self, (x0, y0, x1, y1): (f32, f32, f32, f32)) {
        self.include(x0, y0);
        self.include(x1, y1);
    }

    fn include_glyph(&mut self, glyph_id: GlyphId) {
        let Some(path) = outline_path(&self.font, glyph_id) else { return };
        let m = self.ctm();
        for (x, y) in path.points() {
            let (dx, dy) = m.apply(x, y);
            self.include(dx, dy);
        }
    }
}

impl ColorPainter for BoundsPainter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        let t = self.ctm().mul(Mat::from_skrifa(transform));
        self.ctm.push(t);
    }
    fn pop_transform(&mut self) {
        if self.ctm.len() > 1 {
            self.ctm.pop();
        }
    }
    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        self.include_glyph(glyph_id);
    }
    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        self.include_box(self.ctm().apply_box(clip_box));
    }
    fn pop_clip(&mut self) {}
    fn fill(&mut self, _brush: Brush<'_>) {}
    fn fill_glyph(
        &mut self,
        glyph_id: GlyphId,
        brush_transform: Option<skrifa::color::Transform>,
        _brush: Brush<'_>,
    ) {
        if let Some(t) = brush_transform {
            self.push_transform(t);
            self.include_glyph(glyph_id);
            self.pop_transform();
        } else {
            self.include_glyph(glyph_id);
        }
    }
    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
    fn pop_layer(&mut self) {}
}

enum Clip {
    None,
    Rect { x0: i32, y0: i32, x1: i32, y1: i32 },
    Mask(Vec<u8>, u32, u32, i32, i32),
}

struct LayerPainter<'a> {
    font: FontRef<'a>,
    palette: Option<skrifa::color::ColorPalette<'a>>,
    foreground: Color,
    ctm: Vec<Mat>,
    clips: Vec<Clip>,
    layers: Vec<Pixmap>,
    modes: Vec<CompositeMode>,
    origin: (f32, f32),
}

impl LayerPainter<'_> {
    fn ctm(&self) -> Mat {
        // The root transform is never popped, so the identity is not reached.
        self.ctm.last().copied().unwrap_or(Mat::IDENTITY)
    }

    fn resolve(&self, palette_index: u16, alpha: f32) -> [f32; 4] {
        let (r, g, b, a) = if palette_index == FOREGROUND_PALETTE {
            (self.foreground.r, self.foreground.g, self.foreground.b, self.foreground.a)
        } else if let Some(c) = self.palette.as_ref().and_then(|p| p.colors().get(palette_index as usize)) {
            (
                c.red() as f32 / 255.0,
                c.green() as f32 / 255.0,
                c.blue() as f32 / 255.0,
                c.alpha() as f32 / 255.0,
            )
        } else {
            (0.0, 0.0, 0.0, 1.0)
        };
        let a = (a * alpha).clamp(0.0, 1.0);
        [r * a, g * a, b * a, a]
    }

    /// The glyph's coverage in pixmap pixels. The map is already y-down and relative to the
    /// pixmap's origin, so `rasterize_mapped`'s image sits at (`left`, `-top`) in the pixmap.
    fn coverage_of(&self, glyph_id: GlyphId) -> Option<GlyphImage> {
        let path = outline_path(&self.font, glyph_id)?;
        let m = self.ctm();
        let (ox, oy) = self.origin;
        path.rasterize_mapped(|x, y| {
            let (dx, dy) = m.apply(x, y);
            (dx - ox, dy - oy)
        })
    }

    fn blit_coverage(&mut self, img: &GlyphImage, color: [f32; 4]) {
        let Some(dest) = self.layers.last_mut() else { return };
        let clip = self.clips.last().unwrap_or(&Clip::None);
        dest.blit_coverage(img.left, -img.top, img, color, clip);
    }

    fn fill_clip(&mut self, color: [f32; 4]) {
        let Some(dest) = self.layers.last_mut() else { return };
        let clip = self.clips.last().unwrap_or(&Clip::None);
        dest.fill_clip(color, clip);
    }
}

impl ColorPainter for LayerPainter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        let t = self.ctm().mul(Mat::from_skrifa(transform));
        self.ctm.push(t);
    }
    fn pop_transform(&mut self) {
        if self.ctm.len() > 1 {
            self.ctm.pop();
        }
    }
    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        if let Some(img) = self.coverage_of(glyph_id) {
            self.clips.push(Clip::Mask(img.data, img.width, img.height, img.left, -img.top));
        } else {
            self.clips.push(Clip::None);
        }
    }
    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let (x0, y0, x1, y1) = self.ctm().apply_box(clip_box);
        let (ox, oy) = self.origin;
        self.clips.push(Clip::Rect {
            x0: (x0 - ox).floor() as i32,
            y0: (y0 - oy).floor() as i32,
            x1: (x1 - ox).ceil() as i32,
            y1: (y1 - oy).ceil() as i32,
        });
    }
    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }
    fn fill(&mut self, brush: Brush<'_>) {
        match brush {
            Brush::Solid { palette_index, alpha } => self.fill_clip(self.resolve(palette_index, alpha)),
            Brush::LinearGradient { p0, p1, color_stops, .. } => {
                self.fill_linear(p0, p1, color_stops);
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, .. } => {
                self.fill_radial(c0, r0, c1, r1, color_stops);
            }
            Brush::SweepGradient { color_stops, .. } => {
                // A sweep without a full conic rasterizer still fills with the last stop so
                // the glyph is not a hole. COLRv0 (Twemoji) never takes this branch.
                if let Some(stop) = color_stops.last() {
                    self.fill_clip(self.resolve(stop.palette_index, stop.alpha));
                }
            }
        }
    }
    fn fill_glyph(
        &mut self,
        glyph_id: GlyphId,
        brush_transform: Option<skrifa::color::Transform>,
        brush: Brush<'_>,
    ) {
        if let Some(t) = brush_transform {
            self.push_transform(t);
        }
        if let Some(img) = self.coverage_of(glyph_id) {
            match brush {
                Brush::Solid { palette_index, alpha } => {
                    self.blit_coverage(&img, self.resolve(palette_index, alpha));
                }
                _ => {
                    self.push_clip_glyph(glyph_id);
                    self.fill(brush);
                    self.pop_clip();
                }
            }
        }
        if brush_transform.is_some() {
            self.pop_transform();
        }
    }
    fn push_layer(&mut self, composite_mode: CompositeMode) {
        // The base layer is never popped, so there is always one to size the new layer by.
        let Some(&Pixmap { width, height, .. }) = self.layers.last() else { return };
        self.layers.push(Pixmap::new(width, height));
        self.modes.push(composite_mode);
    }
    fn pop_layer(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let Some(top) = self.layers.pop() else { return };
        let mode = self.modes.pop().unwrap_or(CompositeMode::SrcOver);
        if let Some(base) = self.layers.last_mut() {
            base.composite(&top, mode);
        }
    }
}

impl LayerPainter<'_> {
    fn fill_linear(
        &mut self,
        p0: skrifa::raw::types::Point<f32>,
        p1: skrifa::raw::types::Point<f32>,
        stops: &[ColorStop],
    ) {
        let colors = resolved_stops(self, stops);
        let Some(&(_, first)) = colors.first() else { return };
        if colors.len() == 1 {
            self.fill_clip(first);
            return;
        }
        let m = self.ctm();
        let (ox, oy) = self.origin;
        let (x0, y0) = m.apply(p0.x, p0.y);
        let (x1, y1) = m.apply(p1.x, p1.y);
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-8 {
            self.fill_clip(first);
            return;
        }
        let Some(dest) = self.layers.last_mut() else { return };
        let clip = self.clips.last().unwrap_or(&Clip::None);
        dest.fill_fn(clip, |x, y| {
            let px = x as f32 + 0.5 + ox;
            let py = y as f32 + 0.5 + oy;
            let t = ((px - x0) * dx + (py - y0) * dy) / len2;
            sample_stops(&colors, t)
        });
    }

    fn fill_radial(
        &mut self,
        c0: skrifa::raw::types::Point<f32>,
        r0: f32,
        _c1: skrifa::raw::types::Point<f32>,
        r1: f32,
        stops: &[ColorStop],
    ) {
        let colors = resolved_stops(self, stops);
        let Some(&(_, first)) = colors.first() else { return };
        if colors.len() == 1 {
            self.fill_clip(first);
            return;
        }
        let m = self.ctm();
        let (ox, oy) = self.origin;
        let (x0, y0) = m.apply(c0.x, c0.y);
        // Radii are in font units; scale by the average axis length of the linear part.
        let scale = ((m.xx * m.xx + m.yx * m.yx).sqrt() + (m.xy * m.xy + m.yy * m.yy).sqrt()) / 2.0;
        let r0 = r0 * scale;
        let r1 = r1 * scale;
        let Some(dest) = self.layers.last_mut() else { return };
        let clip = self.clips.last().unwrap_or(&Clip::None);
        // shortcut: t is the distance from c0 mixed between the radii, ignoring c1 (exact for
        // concentric circles); solve the two-circle cone when a font with offset centers shows.
        dest.fill_fn(clip, |x, y| {
            let px = x as f32 + 0.5 + ox;
            let py = y as f32 + 0.5 + oy;
            let dr = r1 - r0;
            let fx = px - x0;
            let fy = py - y0;
            let d = (fx * fx + fy * fy).sqrt();
            let t = if dr.abs() > 1e-6 { (d - r0) / dr } else { d };
            sample_stops(&colors, t)
        });
    }
}

fn resolved_stops(painter: &LayerPainter<'_>, stops: &[ColorStop]) -> Vec<(f32, [f32; 4])> {
    let mut out: Vec<(f32, [f32; 4])> =
        stops.iter().map(|s| (s.offset, painter.resolve(s.palette_index, s.alpha))).collect();
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn sample_stops(stops: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    let (Some(&(first_at, first)), Some(&(_, last))) = (stops.first(), stops.last()) else {
        return [0.0; 4];
    };
    if t <= first_at {
        return first;
    }
    for pair in stops.windows(2) {
        let &[(a_at, a), (b_at, b)] = pair else { continue };
        if t <= b_at {
            let span = (b_at - a_at).max(1e-6);
            return lerp_color(a, b, (t - a_at) / span);
        }
    }
    last
}

fn lerp_color([a0, a1, a2, a3]: [f32; 4], [b0, b1, b2, b3]: [f32; 4], t: f32) -> [f32; 4] {
    [a0 + (b0 - a0) * t, a1 + (b1 - a1) * t, a2 + (b2 - a2) * t, a3 + (b3 - a3) * t]
}

fn outline_path(font: &FontRef<'_>, glyph_id: GlyphId) -> Option<PathRecorder> {
    let outlines = font.outline_glyphs();
    let outline = outlines.get(glyph_id)?;
    let mut path = PathRecorder::default();
    outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut path).ok()?;
    Some(path)
}

struct Pixmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Pixmap {
    fn new(width: u32, height: u32) -> Self {
        let n = width as usize * height as usize * 4;
        Self { width, height, data: vec![0; n] }
    }

    fn has_ink(&self) -> bool {
        self.data.as_chunks::<4>().0.iter().any(|&[_, _, _, a]| a > 0)
    }

    fn blend_pixel(&mut self, x: i32, y: i32, src: [f32; 4], clip: &Clip) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let cover = clip_coverage(clip, x, y);
        if cover <= 0.0 {
            return;
        }
        let i = (y as u32 * self.width + x as u32) as usize;
        let Some(px) = self.data.as_chunks_mut::<4>().0.get_mut(i) else { return };
        let [sr, sg, sb, sa] = src;
        let sa = (sa * cover).clamp(0.0, 1.0);
        if sa <= 0.0 {
            return;
        }
        let [dr, dg, db, da] = px.map(|v| f32::from(v) / 255.0);
        let inv = 1.0 - sa;
        *px = [sr * cover + dr * inv, sg * cover + dg * inv, sb * cover + db * inv, sa + da * inv]
            .map(|v| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
    }

    /// Blends `img` (one coverage byte per pixel, or premultiplied RGBA for a color image)
    /// with its top-left at (`x0`, `y0`). Pixels past the end of a short image are skipped.
    fn blit_coverage(&mut self, x0: i32, y0: i32, img: &GlyphImage, color: [f32; 4], clip: &Clip) {
        let w = img.width as usize;
        if w == 0 {
            return;
        }
        let n = w * img.height as usize;
        let at = |k: usize| (x0 + (k % w) as i32, y0 + (k / w) as i32);
        if img.format == GlyphImageFormat::Color {
            for (k, px) in img.data.as_chunks::<4>().0.iter().take(n).enumerate() {
                if px[3] == 0 {
                    continue;
                }
                let (x, y) = at(k);
                self.blend_pixel(x, y, px.map(|v| f32::from(v) / 255.0 * color[3]), clip);
            }
        } else {
            for (k, &cov) in img.data.iter().take(n).enumerate() {
                if cov == 0 {
                    continue;
                }
                let cov = f32::from(cov) / 255.0;
                let (x, y) = at(k);
                self.blend_pixel(x, y, color.map(|c| c * cov), clip);
            }
        }
    }

    fn fill_clip(&mut self, color: [f32; 4], clip: &Clip) {
        match *clip {
            Clip::None => {
                for y in 0..self.height as i32 {
                    for x in 0..self.width as i32 {
                        self.blend_pixel(x, y, color, &Clip::None);
                    }
                }
            }
            Clip::Rect { x0, y0, x1, y1 } => {
                for y in y0.max(0)..y1.min(self.height as i32) {
                    for x in x0.max(0)..x1.min(self.width as i32) {
                        self.blend_pixel(x, y, color, &Clip::None);
                    }
                }
            }
            Clip::Mask(ref mask, w, h, x0, y0) => {
                if w == 0 {
                    return;
                }
                for (k, &cov) in mask.iter().take(w as usize * h as usize).enumerate() {
                    if cov == 0 {
                        continue;
                    }
                    let cov = f32::from(cov) / 255.0;
                    let (x, y) = (x0 + (k % w as usize) as i32, y0 + (k / w as usize) as i32);
                    self.blend_pixel(x, y, color.map(|c| c * cov), &Clip::None);
                }
            }
        }
    }

    fn fill_fn(&mut self, clip: &Clip, mut f: impl FnMut(i32, i32) -> [f32; 4]) {
        let (x0, y0, x1, y1) = match *clip {
            Clip::None => (0, 0, self.width as i32, self.height as i32),
            Clip::Rect { x0, y0, x1, y1 } => {
                (x0.max(0), y0.max(0), x1.min(self.width as i32), y1.min(self.height as i32))
            }
            Clip::Mask(_, w, h, x0, y0) => (x0, y0, x0 + w as i32, y0 + h as i32),
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let c = f(x, y);
                self.blend_pixel(x, y, c, clip);
            }
        }
    }

    fn composite(&mut self, top: &Pixmap, mode: CompositeMode) {
        let dest = self.data.as_chunks_mut::<4>().0.iter_mut();
        for (d, s) in dest.zip(top.data.as_chunks::<4>().0) {
            if s[3] == 0 && !matches!(mode, CompositeMode::Src | CompositeMode::Clear) {
                continue;
            }
            let unit = |px: [u8; 4]| px.map(|v| f32::from(v) / 255.0);
            *d = composite_premult(unit(*s), unit(*d), mode)
                .map(|v| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
        }
    }
}

fn clip_coverage(clip: &Clip, x: i32, y: i32) -> f32 {
    match *clip {
        Clip::None => 1.0,
        Clip::Rect { x0, y0, x1, y1 } => {
            if x >= x0 && x < x1 && y >= y0 && y < y1 {
                1.0
            } else {
                0.0
            }
        }
        Clip::Mask(ref mask, w, h, x0, y0) => {
            let xx = x - x0;
            let yy = y - y0;
            if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                0.0
            } else {
                mask.get((yy as u32 * w + xx as u32) as usize).map_or(0.0, |&v| f32::from(v) / 255.0)
            }
        }
    }
}

/// Composites premultiplied `src` onto premultiplied `dst` (both RGBA in 0..=1).
fn composite_premult(src: [f32; 4], dst: [f32; 4], mode: CompositeMode) -> [f32; 4] {
    let [sr, sg, sb, sa] = src;
    let [dr, dg, db, da] = dst;
    let src_over = |[sr, sg, sb, sa]: [f32; 4], [dr, dg, db, da]: [f32; 4]| {
        let inv = 1.0 - sa;
        [sr + dr * inv, sg + dg * inv, sb + db * inv, sa + da * inv]
    };
    match mode {
        CompositeMode::Clear => [0.0; 4],
        CompositeMode::Src => src,
        CompositeMode::Dest => dst,
        // Plus is additive; clamp.
        CompositeMode::Plus => {
            [(sr + dr).min(1.0), (sg + dg).min(1.0), (sb + db).min(1.0), (sa + da).min(1.0)]
        }
        CompositeMode::SrcOver => src_over(src, dst),
        CompositeMode::DestOver => src_over(dst, src),
        CompositeMode::SrcIn => [sr * da, sg * da, sb * da, sa * da],
        CompositeMode::DestIn => [dr * sa, dg * sa, db * sa, da * sa],
        CompositeMode::SrcOut => {
            let inv = 1.0 - da;
            [sr * inv, sg * inv, sb * inv, sa * inv]
        }
        CompositeMode::DestOut => {
            let inv = 1.0 - sa;
            [dr * inv, dg * inv, db * inv, da * inv]
        }
        CompositeMode::SrcAtop => {
            let inv = 1.0 - sa;
            [sr * da + dr * inv, sg * da + dg * inv, sb * da + db * inv, da]
        }
        CompositeMode::DestAtop => {
            let inv = 1.0 - da;
            [dr * sa + sr * inv, dg * sa + sg * inv, db * sa + sb * inv, sa]
        }
        CompositeMode::Xor => {
            let s = 1.0 - da;
            let d = 1.0 - sa;
            [sr * s + dr * d, sg * s + dg * d, sb * s + db * d, sa * s + da * d]
        }
        CompositeMode::Screen => {
            // screen: 1 - (1-a)(1-b); inputs are premultiplied.
            let or = sr + dr - sr * dr;
            let og = sg + dg - sg * dg;
            let ob = sb + db - sb * db;
            let oa = sa + da - sa * da;
            [or, og, ob, oa]
        }
        CompositeMode::Multiply => {
            // Multiply in straight, then premultiply. Degenerate when alpha is 0.
            let s_r = if sa > 0.0 { sr / sa } else { 0.0 };
            let s_g = if sa > 0.0 { sg / sa } else { 0.0 };
            let s_b = if sa > 0.0 { sb / sa } else { 0.0 };
            let d_r = if da > 0.0 { dr / da } else { 0.0 };
            let d_g = if da > 0.0 { dg / da } else { 0.0 };
            let d_b = if da > 0.0 { db / da } else { 0.0 };
            let oa = sa + da * (1.0 - sa);
            let or = (s_r * d_r) * oa;
            let og = (s_g * d_g) * oa;
            let ob = (s_b * d_b) * oa;
            [or, og, ob, oa]
        }
        _ => src_over(src, dst),
    }
}
