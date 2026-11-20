//! Mipmaps: smaller copies of app images drawn scaled down. Bilinear sampling reads four
//! source pixels for each drawn one, so an image drawn at a quarter of its size skips most
//! of its pixels and shimmers; drawn from a copy at most twice the drawn size, every source
//! pixel counts. The copies are built here, in the painter both renderers share, so both
//! draw the same pixels, and only an image drawn small has them (and then only the levels it
//! is drawn at reach a GPU).

use std::sync::Arc;

use remedy_ui_paint::Image;

/// Smaller copies of one image, each half the size of the one before (rounded up).
#[derive(Default)]
pub(crate) struct Levels {
    pub(crate) levels: Vec<Image>,
}

/// Which level an image drawn at `scale` device pixels per image pixel (the larger axis) is
/// drawn from: 0 for the image itself, `k` for the copy `2^k` times smaller, the one drawn at
/// more than half its size (at exactly half, the one drawn one to one). Never more than `max`.
pub(crate) fn level_for(scale: f32, max: usize) -> usize {
    if !scale.is_finite() || scale <= 0.0 || scale > 0.5 {
        return 0;
    }
    let k = (1.0 / scale).log2().floor();
    if k.is_finite() { (k as usize).min(max) } else { 0 }
}

/// How many levels an image of `w` × `h` has below itself (down to 1 × 1).
pub(crate) fn level_count(w: u32, h: u32) -> usize {
    let mut n = 0;
    let (mut w, mut h) = (w.max(1), h.max(1));
    while w > 1 || h > 1 {
        w = w.div_ceil(2);
        h = h.div_ceil(2);
        n += 1;
    }
    n
}

/// Premultiplied RGBA8 `pixels` of `w` × `h` at half the size (rounded up): each pixel the
/// rounded average of a 2 × 2 box, the last row or column repeated where the size is odd.
pub(crate) fn halve(w: u32, h: u32, pixels: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = vec![0u8; nw * nh * 4];
    let stride = w * 4;
    for (y, dst) in out.chunks_exact_mut(nw * 4).enumerate() {
        let row = |r: usize| pixels.get(r.min(h - 1) * stride..(r.min(h - 1) + 1) * stride).unwrap_or(&[]);
        let (a, b) = (row(2 * y), row(2 * y + 1));
        for (x, d) in dst.chunks_exact_mut(4).enumerate() {
            let (x0, x1) = (2 * x * 4, (2 * x + 1).min(w - 1) * 4);
            let (Some(p), Some(q), Some(r), Some(t)) =
                (a.get(x0..x0 + 4), a.get(x1..x1 + 4), b.get(x0..x0 + 4), b.get(x1..x1 + 4))
            else {
                continue;
            };
            for (k, o) in d.iter_mut().enumerate() {
                let at = |s: &[u8]| u32::from(s.get(k).copied().unwrap_or(0));
                *o = ((at(p) + at(q) + at(r) + at(t) + 2) / 4) as u8;
            }
        }
    }
    (nw as u32, nh as u32, out)
}

impl Levels {
    /// Level `k` (1 or more) of `image`, built (with those above it) the first time it is asked
    /// for. `None` for an image whose pixels do not match its size.
    pub(crate) fn get(&mut self, image: &Image, k: usize) -> Option<Image> {
        if k == 0 || image.pixels.len() != image.width as usize * image.height as usize * 4 {
            return None;
        }
        while self.levels.len() < k {
            let from = self.levels.last().unwrap_or(image);
            let (w, h, px) = halve(from.width.max(1), from.height.max(1), &from.pixels);
            self.levels.push(Image::new(w, h, Arc::from(px)));
        }
        self.levels.get(k - 1).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_halve_and_round_up() {
        assert_eq!(level_count(1, 1), 0);
        assert_eq!(level_count(8, 8), 3);
        assert_eq!(level_count(5, 1), 3, "5, 3, 2, 1");
        let (w, h, px) = halve(3, 1, &[0, 0, 0, 0, 255, 255, 255, 255, 100, 100, 100, 100]);
        assert_eq!((w, h), (2, 1));
        assert_eq!(px, vec![128, 128, 128, 128, 100, 100, 100, 100], "the odd column repeats");
    }

    #[test]
    fn the_level_drawn_is_more_than_half_its_size() {
        assert_eq!(level_for(1.0, 10), 0);
        assert_eq!(level_for(0.6, 10), 0);
        assert_eq!(level_for(0.5, 10), 1, "exactly half: the level drawn one to one");
        assert_eq!(level_for(0.49, 10), 1);
        assert_eq!(level_for(0.25, 10), 2);
        assert_eq!(level_for(0.1, 10), 3);
        assert_eq!(level_for(1.0e-9, 4), 4, "no further than the image goes");
        assert_eq!(level_for(f32::NAN, 10), 0);
        assert_eq!(level_for(0.0, 10), 0);
    }

    #[test]
    fn a_checkerboard_averages_to_grey() {
        let mut px = Vec::new();
        for y in 0..64u32 {
            for x in 0..64u32 {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                px.extend([v, v, v, 255]);
            }
        }
        let image = Image::new(64, 64, Arc::from(px));
        let mut l = Levels::default();
        let Some(third) = l.get(&image, 3) else { unreachable!("a level") };
        assert_eq!((third.width, third.height), (8, 8));
        assert!(third.pixels.chunks(4).all(|p| p == [128, 128, 128, 255]), "{:?}", third.pixels.get(..8));
        assert_eq!(l.levels.len(), 3, "built once, kept");
    }
}
