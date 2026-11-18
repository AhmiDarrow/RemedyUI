//! The canvas an animation is drawn on: straight-alpha RGBA8 at the source's size, with the
//! operations frame disposal and blending need.

use remedy_ui_media_image::DecodeError;

/// A rectangle of the canvas, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Straight-alpha RGBA8 pixels, row by row.
pub(crate) struct Canvas {
    width: u32,
    height: u32,
    px: Vec<u8>,
}

/// Memory for `len` zeroed bytes, or the error an image too large for this machine gives.
pub(crate) fn zeroed(len: usize, width: u32, height: u32) -> Result<Vec<u8>, DecodeError> {
    let mut v: Vec<u8> = Vec::new();
    v.try_reserve_exact(len).map_err(|_| DecodeError::TooLarge { width, height })?;
    v.resize(len, 0);
    Ok(v)
}

impl Canvas {
    /// A fully transparent canvas.
    pub fn new(width: u32, height: u32) -> Result<Canvas, DecodeError> {
        let len = u64::from(width) * u64::from(height) * 4;
        let len = usize::try_from(len).map_err(|_| DecodeError::TooLarge { width, height })?;
        Ok(Canvas { width, height, px: zeroed(len, width, height)? })
    }

    /// The pixels.
    pub fn pixels(&self) -> &[u8] {
        &self.px
    }

    /// `r` cut to the canvas.
    fn clip(&self, r: Region) -> Option<Region> {
        let x1 = r.x.saturating_add(r.w).min(self.width);
        let y1 = r.y.saturating_add(r.h).min(self.height);
        (r.x < x1 && r.y < y1).then_some(Region { x: r.x, y: r.y, w: x1 - r.x, h: y1 - r.y })
    }

    /// The byte range of row `y` between `x` and `x + w`.
    fn span(&self, x: u32, y: u32, w: u32) -> std::ops::Range<usize> {
        let start = (u64::from(y) * u64::from(self.width) + u64::from(x)) * 4;
        let start = usize::try_from(start).unwrap_or(usize::MAX);
        start..start.saturating_add(w as usize * 4)
    }

    /// Makes `r` transparent.
    pub fn clear(&mut self, r: Region) {
        let Some(r) = self.clip(r) else { return };
        for y in r.y..r.y + r.h {
            let span = self.span(r.x, y, r.w);
            if let Some(row) = self.px.get_mut(span) {
                row.fill(0);
            }
        }
    }

    /// A copy of all pixels, to bring back after a frame that is to be undone.
    pub fn snapshot(&self) -> Result<Vec<u8>, DecodeError> {
        let mut v: Vec<u8> = Vec::new();
        v.try_reserve_exact(self.px.len())
            .map_err(|_| DecodeError::TooLarge { width: self.width, height: self.height })?;
        v.extend_from_slice(&self.px);
        Ok(v)
    }

    /// Brings back pixels taken with [`Canvas::snapshot`].
    pub fn restore(&mut self, saved: &[u8]) {
        if saved.len() == self.px.len() {
            self.px.copy_from_slice(saved);
        }
    }

    /// Puts the frame `rgba` (`r.w`×`r.h` straight-alpha pixels) at `r`: replacing what is
    /// there (`blend` false) or drawn over it (`blend` true). Parts outside the canvas are cut.
    pub fn draw(&mut self, r: Region, rgba: &[u8], blend: bool) {
        let Some(c) = self.clip(r) else { return };
        let stride = r.w as usize * 4;
        for y in c.y..c.y + c.h {
            let from = (y - r.y) as usize * stride + (c.x - r.x) as usize * 4;
            let Some(src) = rgba.get(from..from + c.w as usize * 4) else { return };
            let span = self.span(c.x, y, c.w);
            let Some(dst) = self.px.get_mut(span) else { return };
            if blend {
                for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                    over(d, s);
                }
            } else {
                dst.copy_from_slice(src);
            }
        }
    }
}

/// Draws straight-alpha `s` over straight-alpha `d`.
fn over(d: &mut [u8; 4], s: &[u8; 4]) {
    let [sr, sg, sb, sa] = *s;
    let [dr, dg, db, da] = *d;
    if sa == 255 {
        *d = *s;
        return;
    }
    if sa == 0 {
        return;
    }
    let (sa, da) = (u32::from(sa), u32::from(da));
    // The result's alpha, times 255.
    let out = sa * 255 + da * (255 - sa);
    let mix = |s: u8, d: u8| {
        let c = u32::from(s) * sa * 255 + u32::from(d) * da * (255 - sa);
        ((c + out / 2) / out) as u8
    };
    *d = [mix(sr, dr), mix(sg, dg), mix(sb, db), ((out + 127) / 255) as u8];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_over_mixes_by_alpha_and_replacing_does_not() {
        let mut c = Canvas::new(2, 1).unwrap_or_else(|e| unreachable!("{e}"));
        let whole = Region { x: 0, y: 0, w: 2, h: 1 };
        c.draw(whole, &[255, 0, 0, 255, 0, 0, 255, 255], false);
        c.draw(whole, &[0, 255, 0, 128, 0, 255, 0, 0], true);
        // Half green over red; fully transparent over blue leaves it.
        assert_eq!(c.pixels(), &[127, 128, 0, 255, 0, 0, 255, 255]);
        c.draw(whole, &[0, 255, 0, 128, 0, 255, 0, 0], false);
        assert_eq!(c.pixels(), &[0, 255, 0, 128, 0, 255, 0, 0]);
    }

    #[test]
    fn frames_reaching_past_the_canvas_are_cut_and_regions_clear() {
        let mut c = Canvas::new(2, 2).unwrap_or_else(|e| unreachable!("{e}"));
        c.draw(Region { x: 1, y: 1, w: 3, h: 3 }, &[9; 36], false);
        assert_eq!(c.pixels().iter().filter(|b| **b == 9).count(), 4);
        c.clear(Region { x: 1, y: 0, w: 5, h: 5 });
        assert!(c.pixels().iter().all(|b| *b == 0));
    }
}
