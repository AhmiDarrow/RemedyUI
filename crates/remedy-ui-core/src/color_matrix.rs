//! Colour matrices: the per-pixel colour changes behind CSS `saturate()`, `hue-rotate()`,
//! `grayscale()`, `sepia()`, `brightness()`, `contrast()` and `invert()`, and SVG
//! `feColorMatrix`.

/// The largest magnitude an entry of a [`ColorMatrix`] keeps ([`ColorMatrix::limited`]). Any
/// entry past it already saturates every channel it touches, and keeping entries this small
/// keeps every sum finite.
pub const MAX_COLOR_MATRIX_ENTRY: f32 = 256.0;

/// A 4 × 5 colour matrix on straight-alpha sRGB channels, each 0.0 to 1.0: rows give the new
/// red, green, blue and alpha; columns weigh the old red, green, blue and alpha, and the fifth
/// adds a constant. The result is clamped to 0.0..=1.0. Pixels nothing was drawn on stay
/// transparent, so a matrix only recolours what was drawn.
///
/// Presets follow the CSS filter functions; [`then`](Self::then) chains them into one matrix
/// (clamped once at the end, where CSS clamps between steps: the same unless a step leaves
/// the 0..1 range, as `saturate(3)` can).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorMatrix(pub [f32; 20]);

impl Default for ColorMatrix {
    fn default() -> Self {
        ColorMatrix::IDENTITY
    }
}

/// `x`, or `fallback` when `x` is NaN or infinite.
fn finite(x: f32, fallback: f32) -> f32 {
    if x.is_finite() { x } else { fallback }
}

impl ColorMatrix {
    /// Changes nothing.
    pub const IDENTITY: ColorMatrix = ColorMatrix([
        1.0, 0.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, 0.0,
    ]);

    /// A matrix that scales red, green and blue by `rgb` and adds `offset` to each, leaving
    /// alpha alone.
    fn linear(rgb: f32, offset: f32) -> ColorMatrix {
        ColorMatrix([
            rgb, 0.0, 0.0, 0.0, offset, //
            0.0, rgb, 0.0, 0.0, offset, //
            0.0, 0.0, rgb, 0.0, offset, //
            0.0, 0.0, 0.0, 1.0, 0.0,
        ])
    }

    /// A matrix whose colour rows are `m` (alpha left alone).
    fn rgb(m: [[f32; 3]; 3]) -> ColorMatrix {
        let [r, g, b] = m;
        ColorMatrix([
            r[0], r[1], r[2], 0.0, 0.0, //
            g[0], g[1], g[2], 0.0, 0.0, //
            b[0], b[1], b[2], 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, 0.0,
        ])
    }

    /// CSS `saturate(amount)`: 0 is grey, 1 unchanged, more than 1 more vivid.
    pub fn saturate(amount: f32) -> ColorMatrix {
        let s = finite(amount, 1.0).max(0.0);
        ColorMatrix::rgb([
            [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
            [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
            [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
        ])
    }

    /// CSS `grayscale(amount)`: 0 unchanged to 1 fully grey.
    pub fn grayscale(amount: f32) -> ColorMatrix {
        let g = 1.0 - finite(amount, 0.0).clamp(0.0, 1.0);
        ColorMatrix::rgb([
            [0.2126 + 0.7874 * g, 0.7152 - 0.7152 * g, 0.0722 - 0.0722 * g],
            [0.2126 - 0.2126 * g, 0.7152 + 0.2848 * g, 0.0722 - 0.0722 * g],
            [0.2126 - 0.2126 * g, 0.7152 - 0.7152 * g, 0.0722 + 0.9278 * g],
        ])
    }

    /// CSS `sepia(amount)`: 0 unchanged to 1 fully sepia.
    pub fn sepia(amount: f32) -> ColorMatrix {
        let g = 1.0 - finite(amount, 0.0).clamp(0.0, 1.0);
        ColorMatrix::rgb([
            [0.393 + 0.607 * g, 0.769 - 0.769 * g, 0.189 - 0.189 * g],
            [0.349 - 0.349 * g, 0.686 + 0.314 * g, 0.168 - 0.168 * g],
            [0.272 - 0.272 * g, 0.534 - 0.534 * g, 0.131 + 0.869 * g],
        ])
    }

    /// CSS `hue-rotate(degrees)`: turns every hue around the colour wheel.
    pub fn hue_rotate(degrees: f32) -> ColorMatrix {
        let (sin, cos) = finite(degrees, 0.0).rem_euclid(360.0).to_radians().sin_cos();
        ColorMatrix::rgb([
            [
                0.213 + cos * 0.787 - sin * 0.213,
                0.715 - cos * 0.715 - sin * 0.715,
                0.072 - cos * 0.072 + sin * 0.928,
            ],
            [
                0.213 - cos * 0.213 + sin * 0.143,
                0.715 + cos * 0.285 + sin * 0.140,
                0.072 - cos * 0.072 - sin * 0.283,
            ],
            [
                0.213 - cos * 0.213 - sin * 0.787,
                0.715 - cos * 0.715 + sin * 0.715,
                0.072 + cos * 0.928 + sin * 0.072,
            ],
        ])
    }

    /// CSS `brightness(amount)`: 0 black, 1 unchanged, more than 1 brighter.
    pub fn brightness(amount: f32) -> ColorMatrix {
        ColorMatrix::linear(finite(amount, 1.0).max(0.0), 0.0)
    }

    /// CSS `contrast(amount)`: 0 mid grey, 1 unchanged, more than 1 harsher.
    pub fn contrast(amount: f32) -> ColorMatrix {
        let c = finite(amount, 1.0).max(0.0);
        ColorMatrix::linear(c, 0.5 - 0.5 * c)
    }

    /// CSS `invert(amount)`: 0 unchanged to 1 a negative.
    pub fn invert(amount: f32) -> ColorMatrix {
        let a = finite(amount, 0.0).clamp(0.0, 1.0);
        ColorMatrix::linear(1.0 - 2.0 * a, a)
    }

    /// CSS `opacity(amount)` as a matrix: alpha scaled by `amount`, 0 to 1.
    pub fn opacity(amount: f32) -> ColorMatrix {
        let mut m = ColorMatrix::IDENTITY;
        m.0[18] = finite(amount, 1.0).clamp(0.0, 1.0);
        m
    }

    /// The matrix's four rows (red, green, blue, alpha), each five weights.
    fn rows(&self) -> [[f32; 5]; 4] {
        let [r, g, b, a] = [0, 1, 2, 3].map(|i| {
            let mut row = [0.0f32; 5];
            for (w, v) in row.iter_mut().zip(self.0.iter().skip(i * 5)) {
                *w = *v;
            }
            row
        });
        [r, g, b, a]
    }

    /// `self`, then `next`, as one matrix.
    pub fn then(&self, next: &ColorMatrix) -> ColorMatrix {
        let (a, n) = (self.rows(), next.rows());
        let mut out = [0.0f32; 20];
        for (o, nrow) in out.as_chunks_mut::<5>().0.iter_mut().zip(&n) {
            for (j, v) in o.iter_mut().enumerate() {
                // The new row weighs the old rows (and, in the offset column, adds its own).
                let weighed: f32 =
                    nrow.iter().zip(&a).map(|(w, arow)| w * arow.get(j).copied().unwrap_or(0.0)).sum();
                *v = if j == 4 { weighed + nrow.get(4).copied().unwrap_or(0.0) } else { weighed };
            }
        }
        ColorMatrix(out).limited()
    }

    /// The matrix as renderers apply it: entries that are NaN or infinite take the identity's
    /// value, and the rest are kept within ±[`MAX_COLOR_MATRIX_ENTRY`], so every sum stays
    /// finite whatever a theme asked for.
    pub fn limited(&self) -> ColorMatrix {
        let mut out = self.0;
        for (v, id) in out.iter_mut().zip(ColorMatrix::IDENTITY.0) {
            *v = finite(*v, id).clamp(-MAX_COLOR_MATRIX_ENTRY, MAX_COLOR_MATRIX_ENTRY);
        }
        ColorMatrix(out)
    }

    /// True when the matrix changes nothing.
    pub fn is_identity(&self) -> bool {
        self.limited() == ColorMatrix::IDENTITY
    }

    /// The matrix applied to a straight-alpha colour (each channel 0..1), clamped.
    pub fn apply(&self, [r, g, b, a]: [f32; 4]) -> [f32; 4] {
        self.rows().map(|[wr, wg, wb, wa, offset]| {
            let v = wr * r + wg * g + wb * b + wa * a + offset;
            if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
        })
    }

    /// The matrix applied to one premultiplied RGBA8 pixel: unpremultiplied, run through the
    /// matrix, clamped and premultiplied again, rounding to the nearest value. A transparent
    /// pixel stays transparent. Renderers compute exactly this, to within a step of rounding.
    pub fn apply_premultiplied(&self, px: [u8; 4]) -> [u8; 4] {
        if px[3] == 0 {
            return [0; 4];
        }
        self.apply_with_inverse(px, 1.0 / f32::from(px[3]))
    }

    /// [`apply_premultiplied`](Self::apply_premultiplied) of a pixel with alpha above zero,
    /// given `1 / alpha` (in 8-bit steps).
    fn apply_with_inverse(&self, [r, g, b, a]: [u8; 4], inv: f32) -> [u8; 4] {
        let u = |c: u8| (f32::from(c) * inv).min(1.0);
        let [nr, ng, nb, na] = self.apply([u(r), u(g), u(b), f32::from(a) / 255.0]);
        let q = |v: f32| (v * 255.0 + 0.5) as u8;
        [q(nr * na), q(ng * na), q(nb * na), q(na)]
    }

    /// The matrix applied in place to premultiplied RGBA8 pixels (any trailing partial pixel
    /// is left alone), with the same results as [`apply_premultiplied`](Self::apply_premultiplied)
    /// on each. Runs of one colour, the most of what an interface draws, cost one pixel.
    pub fn apply_to_pixels(&self, pixels: &mut [u8]) {
        let m = self.limited();
        // Transparent stays transparent: the first run needs no arithmetic.
        let (mut last_in, mut last_out) = ([0u8; 4], [0u8; 4]);
        for px in pixels.as_chunks_mut::<4>().0 {
            if *px != last_in {
                last_in = *px;
                last_out = match px[3] {
                    0 => [0; 4],
                    a => m.apply_with_inverse(*px, 1.0 / f32::from(a)),
                };
            }
            *px = last_out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn neutral_amounts_change_nothing() {
        for m in [
            ColorMatrix::saturate(1.0),
            ColorMatrix::grayscale(0.0),
            ColorMatrix::sepia(0.0),
            ColorMatrix::hue_rotate(0.0),
            ColorMatrix::hue_rotate(360.0),
            ColorMatrix::brightness(1.0),
            ColorMatrix::contrast(1.0),
            ColorMatrix::invert(0.0),
            ColorMatrix::opacity(1.0),
        ] {
            assert!(m.0.iter().zip(ColorMatrix::IDENTITY.0).all(|(a, b)| (a - b).abs() < 1e-3), "{m:?}");
            let c = [0.2, 0.6, 0.9, 0.7];
            assert!(close(m.apply(c), c), "{m:?}");
        }
        assert!(ColorMatrix::IDENTITY.is_identity());
        assert!(!ColorMatrix::grayscale(1.0).is_identity());
    }

    #[test]
    fn presets_do_what_css_says() {
        // Grey: every channel the same, alpha kept.
        let [r, g, b, a] = ColorMatrix::grayscale(1.0).apply([1.0, 0.0, 0.0, 0.5]);
        assert!((r - 0.2126).abs() < 1e-4 && (g - r).abs() < 1e-4 && (b - r).abs() < 1e-4 && a == 0.5);
        assert!(close(ColorMatrix::saturate(0.0).apply([0.0, 1.0, 0.0, 1.0]), [0.715, 0.715, 0.715, 1.0]));
        assert!(close(ColorMatrix::invert(1.0).apply([0.25, 0.5, 1.0, 1.0]), [0.75, 0.5, 0.0, 1.0]));
        assert!(close(ColorMatrix::contrast(0.0).apply([0.1, 0.9, 0.3, 1.0]), [0.5, 0.5, 0.5, 1.0]));
        assert!(close(ColorMatrix::brightness(2.0).apply([0.25, 0.75, 0.0, 1.0]), [0.5, 1.0, 0.0, 1.0]));
        assert!(close(ColorMatrix::opacity(0.5).apply([1.0, 1.0, 1.0, 0.8]), [1.0, 1.0, 1.0, 0.4]));
        // A half turn sends red toward cyan.
        let [r, g, b, _] = ColorMatrix::hue_rotate(180.0).apply([1.0, 0.0, 0.0, 1.0]);
        assert!(r < 0.01 && g > 0.2 && b > 0.2, "{r} {g} {b}");
        // Grey stays grey under any hue rotation.
        let grey = ColorMatrix::hue_rotate(73.0).apply([0.4, 0.4, 0.4, 1.0]);
        assert!(close(grey, [0.4, 0.4, 0.4, 1.0]), "{grey:?}");
    }

    #[test]
    fn chaining_is_applying_in_order() {
        let a = ColorMatrix::sepia(0.6);
        let b = ColorMatrix::hue_rotate(40.0);
        let c = [0.3, 0.5, 0.2, 0.9];
        assert!(close(a.then(&b).apply(c), b.apply(a.apply(c))));
        // Offsets chain too.
        let i = ColorMatrix::invert(1.0);
        assert!(close(i.then(&i).apply(c), c));
        assert!(close(ColorMatrix::IDENTITY.then(&a).apply(c), a.apply(c)));
    }

    #[test]
    fn hostile_values_are_limited() {
        let mut m = ColorMatrix([f32::NAN; 20]);
        assert_eq!(m.limited(), ColorMatrix::IDENTITY);
        m.0 = [f32::MAX; 20];
        let l = m.limited();
        assert!(l.0.iter().all(|v| *v == MAX_COLOR_MATRIX_ENTRY));
        assert!(l.apply([1.0; 4]).iter().all(|v| v.is_finite()));
        // Chaining huge matrices stays finite.
        let big = ColorMatrix([MAX_COLOR_MATRIX_ENTRY; 20]);
        assert!(big.then(&big).then(&big).0.iter().all(|v| v.is_finite()));
        assert!(ColorMatrix::saturate(f32::INFINITY).is_identity());
        assert!(ColorMatrix::hue_rotate(f32::NAN).is_identity());
        assert!(ColorMatrix::grayscale(-3.0).is_identity());
    }

    #[test]
    fn pixels_in_bulk_match_one_by_one() {
        let m = ColorMatrix::hue_rotate(77.0).then(&ColorMatrix::saturate(1.7));
        let mut px = Vec::new();
        let mut x = 0x1234_5678u32;
        for i in 0..4000u32 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let a = if i % 7 == 0 { 0 } else { (x >> 24) as u8 };
            let c = |s: u32| (((x >> s) & 0xff) as u16 * u16::from(a) / 255) as u8;
            // Runs of the same pixel, as interfaces draw.
            for _ in 0..(i % 3) + 1 {
                px.extend([c(0), c(8), c(16), a]);
            }
        }
        let want: Vec<u8> = px.as_chunks::<4>().0.iter().flat_map(|p| m.apply_premultiplied(*p)).collect();
        m.apply_to_pixels(&mut px);
        assert_eq!(px, want);
    }

    #[test]
    fn premultiplied_pixels_round_trip() {
        let id = ColorMatrix::IDENTITY;
        for px in [[0, 0, 0, 0], [10, 20, 30, 40], [128, 64, 0, 128], [255, 255, 255, 255], [1, 0, 1, 1]] {
            assert_eq!(id.apply_premultiplied(px), px);
        }
        // Transparent stays transparent, even under a matrix that adds alpha.
        let mut add = ColorMatrix::IDENTITY;
        add.0[19] = 1.0;
        assert_eq!(add.apply_premultiplied([0, 0, 0, 0]), [0, 0, 0, 0]);
        assert_eq!(add.apply_premultiplied([50, 0, 0, 100]), [128, 0, 0, 255]);
        // Half-transparent white, inverted: half-transparent black.
        assert_eq!(ColorMatrix::invert(1.0).apply_premultiplied([128, 128, 128, 128]), [0, 0, 0, 128]);
        let mut px = vec![128, 128, 128, 128, 0, 0, 0, 0, 7];
        ColorMatrix::invert(1.0).apply_to_pixels(&mut px);
        assert_eq!(px, vec![0, 0, 0, 128, 0, 0, 0, 0, 7]);
    }
}
