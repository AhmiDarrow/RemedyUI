//! Logical and device pixels.
//!
//! Layout works in logical pixels. A window's [`Scale`] (the OS display scale times the app's
//! zoom) turns them into device pixels. After layout, edges are snapped to whole device pixels
//! so lines stay crisp at fractional scales such as 125% and 150%.

use crate::geometry::{Point, Rect, Size};

/// Device pixels per logical pixel. Always finite and greater than zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale(f32);

impl Default for Scale {
    fn default() -> Self {
        Scale(1.0)
    }
}

impl Scale {
    /// A scale of `factor` device pixels per logical pixel. A factor that is not finite or not
    /// greater than zero is not a scale any display reports, so `None` is returned.
    pub fn new(factor: f32) -> Option<Scale> {
        if factor.is_finite() && factor > 0.0 { Some(Scale(factor)) } else { None }
    }

    /// The factor.
    pub fn get(self) -> f32 {
        self.0
    }

    /// This scale multiplied by `zoom` (the app's own zoom, such as Ctrl + / −).
    /// Returns `None` if the result would not be a valid scale.
    pub fn zoomed(self, zoom: f32) -> Option<Scale> {
        Scale::new(self.0 * zoom)
    }

    /// Logical to device pixels.
    pub fn to_device(self, v: f32) -> f32 {
        v * self.0
    }

    /// Device to logical pixels.
    pub fn to_logical(self, v: f32) -> f32 {
        v / self.0
    }

    /// `v` (logical) moved to the nearest whole device pixel, in logical pixels.
    pub fn snap(self, v: f32) -> f32 {
        (v * self.0).round() / self.0
    }

    /// A point snapped to the device grid.
    pub fn snap_point(self, p: Point) -> Point {
        Point::new(self.snap(p.x), self.snap(p.y))
    }

    /// A rectangle with each edge snapped to the device grid. Snapping edges (not origin
    /// and size separately) means neighbours that share an edge still share it after
    /// snapping: no gaps and no overlaps.
    pub fn snap_rect(self, r: Rect) -> Rect {
        Rect { min: self.snap_point(r.min), max: self.snap_point(r.max) }
    }

    /// The width in logical pixels of a line that is `device_px` whole device pixels thick,
    /// at least one. Use it for hairlines and borders: a 1 dp border at 150% becomes one
    /// crisp device pixel (or two when `logical` rounds up), never 1.5.
    pub fn crisp_line(self, logical: f32) -> f32 {
        let px = (logical * self.0).round().max(1.0);
        px / self.0
    }

    /// A size rounded up to whole device pixels, for allocating surfaces.
    pub fn device_size_ceil(self, s: Size) -> (u32, u32) {
        (to_u32_ceil(s.width * self.0), to_u32_ceil(s.height * self.0))
    }
}

fn to_u32_ceil(v: f32) -> u32 {
    let c = v.ceil();
    if c.is_nan() || c <= 0.0 {
        0
    } else if c >= u32::MAX as f32 {
        u32::MAX
    } else {
        c as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_factors() {
        assert!(Scale::new(0.0).is_none());
        assert!(Scale::new(-1.0).is_none());
        assert!(Scale::new(f32::NAN).is_none());
        assert!(Scale::new(f32::INFINITY).is_none());
        assert_eq!(Scale::new(1.5).map(Scale::get), Some(1.5));
    }

    #[test]
    fn neighbours_share_snapped_edges() {
        let s = Scale::new(1.25).unwrap_or_default();
        let a = s.snap_rect(Rect::new(0.0, 0.0, 10.3, 10.0));
        let b = s.snap_rect(Rect::new(10.3, 0.0, 20.7, 10.0));
        assert_eq!(a.max.x, b.min.x);
        assert_eq!(s.to_device(a.max.x).fract(), 0.0);
    }

    #[test]
    fn crisp_lines_are_whole_device_pixels() {
        let s = Scale::new(1.5).unwrap_or_default();
        let w = s.crisp_line(1.0);
        assert_eq!(s.to_device(w).fract(), 0.0);
        assert!(s.to_device(w) >= 1.0);
        assert_eq!(s.to_device(s.crisp_line(0.1)), 1.0);
    }

    #[test]
    fn device_size_rounds_up_and_clamps() {
        let s = Scale::new(2.0).unwrap_or_default();
        assert_eq!(s.device_size_ceil(Size::new(10.2, 0.0)), (21, 0));
        assert_eq!(s.device_size_ceil(Size::new(f32::MAX, 1.0)), (u32::MAX, 2));
    }
}
