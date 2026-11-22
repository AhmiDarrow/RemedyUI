//! Glyph rasterization: hinted outlines to coverage bitmaps.
//!
//! Outlines come from skrifa, hinted at the exact pixel size (TrueType bytecode where the
//! font has it, the autohinter where it does not). Coverage is computed by exact-area
//! accumulation (the signed-area method used by font-rs): every pixel gets the true fraction
//! of its area inside the outline, with no supersampling. LCD output renders at three times
//! the horizontal resolution and applies FreeType's default five-tap filter.

use std::collections::HashMap;

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target};
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::engine::RunFont;

/// How outlines are fitted to the pixel grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Hinting {
    /// Full hinting: stems on whole pixels in both directions; one horizontal position.
    #[default]
    Full,
    /// Vertical-only hinting; four horizontal subpixel positions.
    Light,
    /// Exact outlines; four horizontal subpixel positions.
    None,
}

/// How coverage is stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Antialias {
    /// One coverage byte per pixel.
    #[default]
    Grayscale,
    /// Three coverage bytes per pixel, one per subpixel, in RGB or BGR panel order.
    Lcd {
        /// The panel's subpixels run blue, green, red.
        bgr: bool,
    },
}

impl Antialias {
    /// Bytes per pixel of images in this mode.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Antialias::Grayscale => 1,
            Antialias::Lcd { .. } => 3,
        }
    }
}

/// A rasterized glyph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Offset from the glyph origin to the image's left edge, in pixels.
    pub left: i32,
    /// Offset from the baseline up to the image's top edge, in pixels.
    pub top: i32,
    /// Coverage, row by row, `bytes_per_pixel` bytes per pixel.
    pub data: Vec<u8>,
}

/// The number of horizontal subpixel positions used when hinting allows them.
pub const SUBPIXEL_BINS: u8 = 4;

/// Which subpixel bin an x offset falls in, for a hinting mode.
pub fn subpixel_bin(x: f32, hinting: Hinting) -> u8 {
    if hinting == Hinting::Full {
        return 0;
    }
    let frac = x - x.floor();
    ((frac * f32::from(SUBPIXEL_BINS)).floor() as u8).min(SUBPIXEL_BINS - 1)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct InstanceKey {
    font: u64,
    face: u32,
    size_64: u32,
    hinting: Hinting,
    lcd: bool,
}

/// Rasterizes glyphs, keeping one hinting instance per font, size and mode.
#[derive(Default)]
pub struct GlyphRasterizer {
    instances: HashMap<InstanceKey, HintingInstance>,
}

impl std::fmt::Debug for GlyphRasterizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GlyphRasterizer({} hinting instances)", self.instances.len())
    }
}

/// Size in 1/64 pixel, the unit glyph caches key on.
pub fn size_64(size: f32) -> u32 {
    let s = (size.max(0.0) * 64.0).round();
    if s >= u32::MAX as f32 { u32::MAX } else { s as u32 }
}

impl GlyphRasterizer {
    /// A rasterizer with no cached instances.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rasterizes `glyph` of `font` at `size` pixels, with its origin moved right by the
    /// subpixel bin's offset. Returns `None` for a glyph with no outline (a space) or a font
    /// that cannot be read.
    pub fn rasterize(
        &mut self,
        font: &RunFont,
        glyph: u32,
        size: f32,
        bin: u8,
        hinting: Hinting,
        aa: Antialias,
    ) -> Option<GlyphImage> {
        let font_ref = FontRef::from_index(font.bytes(), font.index()).ok()?;
        let outlines = font_ref.outline_glyphs();
        let outline = outlines.get(GlyphId::new(glyph))?;
        let lcd = matches!(aa, Antialias::Lcd { .. });
        let mut path = PathRecorder::default();
        let px = Size::new(size);
        let drawn = match hinting {
            Hinting::None => outline.draw(DrawSettings::unhinted(px, LocationRef::default()), &mut path),
            Hinting::Full | Hinting::Light => {
                let key =
                    InstanceKey { font: font.id(), face: font.index(), size_64: size_64(size), hinting, lcd };
                let instance = match self.instances.entry(key) {
                    std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
                    std::collections::hash_map::Entry::Vacant(v) => {
                        let mode = match (hinting, lcd) {
                            (Hinting::Light, _) => SmoothMode::Light,
                            (_, true) => SmoothMode::Lcd,
                            _ => SmoothMode::Normal,
                        };
                        let target = Target::Smooth {
                            mode,
                            symmetric_rendering: true,
                            preserve_linear_metrics: false,
                        };
                        let options = HintingOptions::from(target);
                        v.insert(HintingInstance::new(&outlines, px, LocationRef::default(), options).ok()?)
                    }
                };
                outline.draw(DrawSettings::hinted(instance, false), &mut path)
            }
        };
        drawn.ok()?;
        let dx = f32::from(bin) / f32::from(SUBPIXEL_BINS);
        let x_scale = if lcd { 3.0 } else { 1.0 };
        let image = path.rasterize(dx, x_scale)?;
        Some(match aa {
            Antialias::Grayscale => image,
            Antialias::Lcd { bgr } => lcd_filter(&image, bgr),
        })
    }

    /// Rasterizes `glyph` of `font` at `size` pixels, unhinted, with each point mapped by
    /// `linear` (`[a, b, c, d]`: x' = a·x + c·y, y' = b·x + d·y, y down, in device pixels per
    /// pixel of `size`) and the origin moved by `offset` (the fraction of a device pixel the
    /// origin falls at). For runs drawn turned, skewed, mirrored or stretched unevenly; hinting
    /// grid-fits to axes that are not the device's there, so it is off. Grayscale.
    pub fn rasterize_transformed(
        &mut self,
        font: &RunFont,
        glyph: u32,
        size: f32,
        linear: [f32; 4],
        offset: (f32, f32),
    ) -> Option<GlyphImage> {
        let font_ref = FontRef::from_index(font.bytes(), font.index()).ok()?;
        let outlines = font_ref.outline_glyphs();
        let outline = outlines.get(GlyphId::new(glyph))?;
        let mut path = PathRecorder::default();
        outline.draw(DrawSettings::unhinted(Size::new(size), LocationRef::default()), &mut path).ok()?;
        let [a, b, c, d] = linear;
        // Font y is up; the transform works in y down.
        path.rasterize_mapped(|x, y| (a * x - c * y + offset.0, b * x - d * y + offset.1))
    }

    /// Drops cached hinting instances (for example under memory pressure).
    pub fn clear(&mut self) {
        self.instances.clear();
    }
}

#[derive(Clone, Copy, Debug)]
enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// Records an outline so its bounds are known before rasterizing.
#[derive(Default)]
pub(crate) struct PathRecorder {
    segs: Vec<Seg>,
}

impl OutlinePen for PathRecorder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.segs.push(Seg::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.segs.push(Seg::Line(x, y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.segs.push(Seg::Quad(cx0, cy0, x, y));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.segs.push(Seg::Cubic(cx0, cy0, cx1, cy1, x, y));
    }
    fn close(&mut self) {
        self.segs.push(Seg::Close);
    }
}

/// How far a flattened curve may stray from the true curve, in pixels.
const FLATTEN_TOLERANCE: f32 = 0.05;

impl PathRecorder {
    #[cfg(test)]
    pub(crate) fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        let mut p = PathRecorder::default();
        p.move_to(x0, y0);
        p.line_to(x1, y0);
        p.line_to(x1, y1);
        p.line_to(x0, y1);
        p.close();
        p
    }

    fn points(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.segs.iter().flat_map(|s| {
            let v: Vec<(f32, f32)> = match *s {
                Seg::Move(x, y) | Seg::Line(x, y) => vec![(x, y)],
                Seg::Quad(a, b, x, y) => vec![(a, b), (x, y)],
                Seg::Cubic(a, b, c, d, x, y) => vec![(a, b), (c, d), (x, y)],
                Seg::Close => vec![],
            };
            v
        })
    }

    /// Coverage of the outline moved right by `dx` and stretched horizontally by `x_scale`,
    /// with font units' y-up turned into bitmap y-down.
    pub(crate) fn rasterize(&self, dx: f32, x_scale: f32) -> Option<GlyphImage> {
        self.rasterize_mapped(|x, y| ((x + dx) * x_scale, -y))
    }

    /// Coverage of the outline with each point (y up, from the glyph origin) moved by `map`
    /// to pixels (y down, from the glyph origin): any affine map, so a glyph can be drawn
    /// turned, skewed or stretched as its run is.
    pub(crate) fn rasterize_mapped(&self, map: impl Fn(f32, f32) -> (f32, f32)) -> Option<GlyphImage> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in self.points() {
            let (x, y) = map(x, y);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        if x0 > x1 || y0 > y1 || !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
            return None;
        }
        let left = x0.floor();
        // The image's top edge, in pixels down from the baseline (negative: above it).
        let top = y0.floor();
        let width = (x1.ceil() - left).max(1.0);
        let height = (y1.ceil() - top).max(1.0);
        if width > 16_384.0 || height > 16_384.0 {
            // A glyph this large belongs on the path renderer, not in a coverage bitmap.
            return None;
        }
        let (w, h) = (width as usize, height as usize);
        let mut r = Raster::new(w, h);
        let at = |x: f32, y: f32| {
            let (x, y) = map(x, y);
            (x - left, y - top)
        };
        let (mut start, mut cur) = ((0.0, 0.0), (0.0, 0.0));
        for s in &self.segs {
            match *s {
                Seg::Move(x, y) => {
                    if cur != start {
                        r.line(cur, start);
                    }
                    start = at(x, y);
                    cur = start;
                }
                Seg::Line(x, y) => {
                    let p = at(x, y);
                    r.line(cur, p);
                    cur = p;
                }
                Seg::Quad(a, b, x, y) => {
                    let (c, p) = (at(a, b), at(x, y));
                    let dd = ((cur.0 - 2.0 * c.0 + p.0).powi(2) + (cur.1 - 2.0 * c.1 + p.1).powi(2)).sqrt();
                    let n = ((dd / (8.0 * FLATTEN_TOLERANCE)).sqrt().ceil() as usize).clamp(1, 256);
                    let mut last = cur;
                    for i in 1..=n {
                        let t = i as f32 / n as f32;
                        let mt = 1.0 - t;
                        let q = (
                            mt * mt * cur.0 + 2.0 * mt * t * c.0 + t * t * p.0,
                            mt * mt * cur.1 + 2.0 * mt * t * c.1 + t * t * p.1,
                        );
                        r.line(last, q);
                        last = q;
                    }
                    cur = p;
                }
                Seg::Cubic(a, b, c, d, x, y) => {
                    let (c1, c2, p) = (at(a, b), at(c, d), at(x, y));
                    let dd1 =
                        ((cur.0 - 2.0 * c1.0 + c2.0).powi(2) + (cur.1 - 2.0 * c1.1 + c2.1).powi(2)).sqrt();
                    let dd2 = ((c1.0 - 2.0 * c2.0 + p.0).powi(2) + (c1.1 - 2.0 * c2.1 + p.1).powi(2)).sqrt();
                    let dd = dd1.max(dd2);
                    let n = ((dd * 0.75 / FLATTEN_TOLERANCE).sqrt().ceil() as usize).clamp(1, 512);
                    let mut last = cur;
                    for i in 1..=n {
                        let t = i as f32 / n as f32;
                        let mt = 1.0 - t;
                        let q = (
                            mt * mt * mt * cur.0
                                + 3.0 * mt * mt * t * c1.0
                                + 3.0 * mt * t * t * c2.0
                                + t * t * t * p.0,
                            mt * mt * mt * cur.1
                                + 3.0 * mt * mt * t * c1.1
                                + 3.0 * mt * t * t * c2.1
                                + t * t * t * p.1,
                        );
                        r.line(last, q);
                        last = q;
                    }
                    cur = p;
                }
                Seg::Close => {
                    if cur != start {
                        r.line(cur, start);
                    }
                    cur = start;
                }
            }
        }
        if cur != start {
            r.line(cur, start);
        }
        Some(GlyphImage {
            width: w as u32,
            height: h as u32,
            left: left as i32,
            top: -top as i32,
            data: r.coverage(),
        })
    }
}

/// Signed-area accumulation buffer. Each row has one spare cell at the end, so a line that
/// touches the right edge still has somewhere to put its remaining area.
struct Raster {
    w: usize,
    h: usize,
    acc: Vec<f32>,
}

impl Raster {
    fn new(w: usize, h: usize) -> Self {
        Raster { w, h, acc: vec![0.0; (w + 2) * h] }
    }

    fn add(&mut self, row: usize, x: isize, v: f32) {
        let stride = self.w + 2;
        let x = x.clamp(0, stride as isize - 1) as usize;
        if let Some(cell) = self.acc.get_mut(row * stride + x) {
            *cell += v;
        }
    }

    fn line(&mut self, p0: (f32, f32), p1: (f32, f32)) {
        if (p0.1 - p1.1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, p0, p1) = if p0.1 < p1.1 { (1.0, p0, p1) } else { (-1.0, p1, p0) };
        let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
        let mut x = p0.0;
        let y_start = p0.1.max(0.0);
        if p0.1 < 0.0 {
            x -= p0.1 * dxdy;
        }
        let y_end = p1.1.min(self.h as f32);
        let mut y = y_start.floor() as usize;
        while (y as f32) < y_end {
            let dy = ((y + 1) as f32).min(p1.1) - (y as f32).max(p0.1);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let xa_floor = xa.floor();
            let xa_i = xa_floor as isize;
            let xb_ceil = xb.ceil();
            let xb_i = xb_ceil as isize;
            if xb_i <= xa_i + 1 {
                let xmf = 0.5 * (x + xnext) - xa_floor;
                self.add(y, xa_i, d - d * xmf);
                self.add(y, xa_i + 1, d * xmf);
            } else {
                let s = (xb - xa).recip();
                let xa_f = xa - xa_floor;
                let a0 = 0.5 * s * (1.0 - xa_f) * (1.0 - xa_f);
                let xb_f = xb - xb_ceil + 1.0;
                let am = 0.5 * s * xb_f * xb_f;
                self.add(y, xa_i, d * a0);
                if xb_i == xa_i + 2 {
                    self.add(y, xa_i + 1, d * (1.0 - a0 - am));
                } else {
                    let a1 = s * (1.5 - xa_f);
                    self.add(y, xa_i + 1, d * (a1 - a0));
                    for xi in xa_i + 2..xb_i - 1 {
                        self.add(y, xi, d * s);
                    }
                    let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
                    self.add(y, xb_i - 1, d * (1.0 - a2 - am));
                }
                self.add(y, xb_i, d * am);
            }
            x = xnext;
            y += 1;
        }
    }

    fn coverage(&self) -> Vec<u8> {
        let stride = self.w + 2;
        let mut out = Vec::with_capacity(self.w * self.h);
        for row in self.acc.chunks(stride) {
            let mut sum = 0.0;
            for &a in row.iter().take(self.w) {
                sum += a;
                out.push((sum.abs().min(1.0) * 255.0 + 0.5) as u8);
            }
        }
        out
    }
}

/// FreeType's default LCD filter, in 1/256: it spreads each subpixel's coverage to its
/// neighbours so colour fringes stay faint.
const LCD_FILTER: [u16; 5] = [0x08, 0x4D, 0x56, 0x4D, 0x08];

/// Turns a 3x-wide coverage image into an RGB (or BGR) image of a third the width.
fn lcd_filter(img: &GlyphImage, bgr: bool) -> GlyphImage {
    // Pad by one output pixel (three subpixels) each side for the filter's spread.
    let sub_w = img.width as usize;
    let out_w = sub_w.div_ceil(3) + 2;
    let h = img.height as usize;
    let mut data = vec![0u8; out_w * h * 3];
    for y in 0..h {
        let row = img.data.get(y * sub_w..(y + 1) * sub_w).unwrap_or(&[]);
        let sample = |s: isize| -> u16 {
            if s < 3 || s - 3 >= sub_w as isize {
                0
            } else {
                u16::from(row.get((s - 3) as usize).copied().unwrap_or(0))
            }
        };
        for ox in 0..out_w {
            for c in 0..3usize {
                let s = (ox * 3 + c) as isize;
                let mut acc: u32 = 0;
                for (k, w) in LCD_FILTER.iter().enumerate() {
                    acc += u32::from(*w) * u32::from(sample(s + k as isize - 2));
                }
                let v = (acc / 256).min(255) as u8;
                let ch = if bgr { 2 - c } else { c };
                if let Some(px) = data.get_mut((y * out_w + ox) * 3 + ch) {
                    *px = v;
                }
            }
        }
    }
    // The 3x image's left edge was at `left` subpixels; one padding pixel moves it left.
    let left = img.left.div_euclid(3) - 1;
    GlyphImage { width: out_w as u32, height: img.height, left, top: img.top, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixel_aligned_square_is_solid() -> Result<(), &'static str> {
        let img = PathRecorder::rect(0.0, 0.0, 4.0, 3.0).rasterize(0.0, 1.0).ok_or("no image")?;
        assert_eq!((img.width, img.height, img.left, img.top), (4, 3, 0, 3));
        assert!(img.data.iter().all(|&c| c == 255), "{:?}", img.data);
        Ok(())
    }

    #[test]
    fn half_pixel_edges_have_half_coverage() -> Result<(), &'static str> {
        let img = PathRecorder::rect(0.5, 0.0, 3.5, 2.0).rasterize(0.0, 1.0).ok_or("no image")?;
        assert_eq!(img.width, 4);
        let row: Vec<u8> = img.data.iter().take(4).copied().collect();
        assert_eq!(row, [128, 255, 255, 128]);
        Ok(())
    }

    #[test]
    fn coverage_sums_to_the_area() -> Result<(), &'static str> {
        // A triangle of area 12 at an awkward offset.
        let mut p = PathRecorder::default();
        p.move_to(0.3, 0.2);
        p.line_to(6.3, 0.2);
        p.line_to(0.3, 4.2);
        p.close();
        let img = p.rasterize(0.0, 1.0).ok_or("no image")?;
        let area: f32 = img.data.iter().map(|&c| f32::from(c) / 255.0).sum();
        assert!((area - 12.0).abs() < 0.1, "{area}");
        Ok(())
    }

    #[test]
    fn subpixel_bins() {
        assert_eq!(subpixel_bin(10.6, Hinting::Full), 0);
        assert_eq!(subpixel_bin(10.6, Hinting::None), 2);
        assert_eq!(subpixel_bin(10.99, Hinting::Light), 3);
        assert_eq!(subpixel_bin(-0.25, Hinting::None), 3);
    }
}
