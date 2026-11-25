//! Geometry in logical pixels (dp).
//!
//! All values are `f32`. Layout works in logical pixels; [`crate::units::Scale`] converts them
//! to device pixels and snaps edges to the device grid after layout.

use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

/// A position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal position, growing to the right.
    pub x: f32,
    /// Vertical position, growing downward.
    pub y: f32,
}

/// A displacement between two points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f32,
    /// Vertical component.
    pub y: f32,
}

/// A width and a height. Neither is negative in a size produced by this crate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// An axis-aligned rectangle given by its two corners. `min` is the top-left corner and
/// `max` the bottom-right; a rectangle made by this crate always has `min <= max`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Top-left corner.
    pub min: Point,
    /// Bottom-right corner.
    pub max: Point,
}

/// Space on each side of a box, in logical directions: `start` and `end` follow the text
/// direction (start is left in left-to-right text), `top` and `bottom` do not.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    /// Space at the top.
    pub top: f32,
    /// Space at the inline end (right in left-to-right text).
    pub end: f32,
    /// Space at the bottom.
    pub bottom: f32,
    /// Space at the inline start (left in left-to-right text).
    pub start: f32,
}

/// Radii of a rounded rectangle's four corners, in logical directions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadii {
    /// Top-start corner.
    pub top_start: f32,
    /// Top-end corner.
    pub top_end: f32,
    /// Bottom-end corner.
    pub bottom_end: f32,
    /// Bottom-start corner.
    pub bottom_start: f32,
}

impl Point {
    /// The origin, (0, 0).
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    /// A point at `x`, `y`.
    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }

    /// This point as a vector from the origin.
    pub const fn to_vec2(self) -> Vec2 {
        Vec2 { x: self.x, y: self.y }
    }

    /// The point part way from `self` to `other`: `t = 0` gives `self`, `t = 1` gives `other`.
    pub fn lerp(self, other: Point, t: f32) -> Point {
        Point::new(self.x + (other.x - self.x) * t, self.y + (other.y - self.y) * t)
    }
}

impl Vec2 {
    /// The zero vector.
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    /// A vector of `x`, `y`.
    pub const fn new(x: f32, y: f32) -> Self {
        Vec2 { x, y }
    }

    /// The vector's length.
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    /// The dot product with `other`.
    pub fn dot(self, other: Vec2) -> f32 {
        self.x * other.x + self.y * other.y
    }
}

impl Size {
    /// A size with no area.
    pub const ZERO: Size = Size { width: 0.0, height: 0.0 };

    /// A size of `width` by `height`. Negative or NaN values become zero, so a size is never
    /// negative.
    pub fn new(width: f32, height: f32) -> Self {
        Size { width: non_negative(width), height: non_negative(height) }
    }

    /// The area, width times height.
    pub fn area(self) -> f32 {
        self.width * self.height
    }

    /// True when either side is zero.
    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// This size grown by `insets` on every side (or shrunk, for negative insets), never below zero.
    pub fn inflate(self, insets: Insets) -> Size {
        Size::new(self.width + insets.horizontal(), self.height + insets.vertical())
    }
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const ZERO: Rect = Rect { min: Point::ZERO, max: Point::ZERO };

    /// The rectangle spanning two corners, in either order.
    pub fn from_points(a: Point, b: Point) -> Self {
        Rect { min: Point::new(a.x.min(b.x), a.y.min(b.y)), max: Point::new(a.x.max(b.x), a.y.max(b.y)) }
    }

    /// The rectangle at `origin` with `size`.
    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Rect { min: origin, max: Point::new(origin.x + size.width, origin.y + size.height) }
    }

    /// The rectangle with left, top, right and bottom edges, in any order.
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Rect::from_points(Point::new(x0, y0), Point::new(x1, y1))
    }

    /// Width.
    pub fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    /// Height.
    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    /// Width and height.
    pub fn size(self) -> Size {
        Size::new(self.width(), self.height())
    }

    /// The center point.
    pub fn center(self) -> Point {
        self.min.lerp(self.max, 0.5)
    }

    /// True when the rectangle has no area.
    pub fn is_empty(self) -> bool {
        self.max.x <= self.min.x || self.max.y <= self.min.y
    }

    /// True when `p` is inside. The top and left edges are inside, the bottom and right
    /// edges outside, so rectangles that share an edge never both contain a point on it.
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.min.x && p.x < self.max.x && p.y >= self.min.y && p.y < self.max.y
    }

    /// The overlap of two rectangles, or `None` when they do not overlap.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let r = Rect {
            min: Point::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: Point::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        };
        if r.is_empty() { None } else { Some(r) }
    }

    /// The smallest rectangle containing both. An empty rectangle adds nothing.
    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Rect {
            min: Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    /// This rectangle moved by `v`.
    pub fn translate(self, v: Vec2) -> Rect {
        Rect { min: self.min + v, max: self.max + v }
    }

    /// This rectangle shrunk by `insets` (in left-to-right terms: `start` is the left side).
    /// It never inverts: an over-inset rectangle collapses to its center line.
    pub fn deflate(self, insets: Insets) -> Rect {
        let x0 = self.min.x + insets.start;
        let x1 = self.max.x - insets.end;
        let y0 = self.min.y + insets.top;
        let y1 = self.max.y - insets.bottom;
        let (x0, x1) = if x0 <= x1 { (x0, x1) } else { ((x0 + x1) / 2.0, (x0 + x1) / 2.0) };
        let (y0, y1) = if y0 <= y1 { (y0, y1) } else { ((y0 + y1) / 2.0, (y0 + y1) / 2.0) };
        Rect { min: Point::new(x0, y0), max: Point::new(x1, y1) }
    }
}

impl Insets {
    /// No space on any side.
    pub const ZERO: Insets = Insets { top: 0.0, end: 0.0, bottom: 0.0, start: 0.0 };

    /// The same space on every side.
    pub const fn all(v: f32) -> Self {
        Insets { top: v, end: v, bottom: v, start: v }
    }

    /// `vertical` at top and bottom, `horizontal` at start and end.
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Insets { top: vertical, end: horizontal, bottom: vertical, start: horizontal }
    }

    /// Start plus end.
    pub fn horizontal(self) -> f32 {
        self.start + self.end
    }

    /// Top plus bottom.
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }

    /// These insets with start and end swapped, for right-to-left text.
    pub const fn mirrored(self) -> Insets {
        Insets { top: self.top, end: self.start, bottom: self.bottom, start: self.end }
    }
}

impl CornerRadii {
    /// Square corners.
    pub const ZERO: CornerRadii =
        CornerRadii { top_start: 0.0, top_end: 0.0, bottom_end: 0.0, bottom_start: 0.0 };

    /// The same radius at every corner.
    pub const fn all(r: f32) -> Self {
        CornerRadii { top_start: r, top_end: r, bottom_end: r, bottom_start: r }
    }

    /// These radii scaled down, all by the same factor, so that no two adjacent corners
    /// overlap on a box of `size` (the CSS rule). Negative radii become zero, and an infinite
    /// one counts as the largest finite radius.
    pub fn fit(self, size: Size) -> CornerRadii {
        let clean = |v: f32| non_negative(v).min(f32::MAX);
        let r = CornerRadii {
            top_start: clean(self.top_start),
            top_end: clean(self.top_end),
            bottom_end: clean(self.bottom_end),
            bottom_start: clean(self.bottom_start),
        };
        // Summed in f64: two huge radii (a pill written as `f32::MAX`) do not overflow.
        let mut f: f64 = 1.0;
        for (a, b, side) in [
            (r.top_start, r.top_end, size.width),
            (r.bottom_start, r.bottom_end, size.width),
            (r.top_start, r.bottom_start, size.height),
            (r.top_end, r.bottom_end, size.height),
        ] {
            let (sum, side) = (f64::from(a) + f64::from(b), f64::from(side));
            if sum > side && sum > 0.0 {
                f = f.min(side / sum);
            }
        }
        let fitted = |v: f32| (f64::from(v) * f) as f32;
        CornerRadii {
            top_start: fitted(r.top_start),
            top_end: fitted(r.top_end),
            bottom_end: fitted(r.bottom_end),
            bottom_start: fitted(r.bottom_start),
        }
    }

    /// These radii with start and end swapped, for right-to-left text.
    pub const fn mirrored(self) -> CornerRadii {
        CornerRadii {
            top_start: self.top_end,
            top_end: self.top_start,
            bottom_end: self.bottom_start,
            bottom_start: self.bottom_end,
        }
    }
}

fn non_negative(v: f32) -> f32 {
    if v > 0.0 { v } else { 0.0 }
}

impl Add<Vec2> for Point {
    type Output = Point;
    fn add(self, v: Vec2) -> Point {
        Point::new(self.x + v.x, self.y + v.y)
    }
}

impl AddAssign<Vec2> for Point {
    fn add_assign(&mut self, v: Vec2) {
        *self = *self + v;
    }
}

impl Sub<Vec2> for Point {
    type Output = Point;
    fn sub(self, v: Vec2) -> Point {
        Point::new(self.x - v.x, self.y - v.y)
    }
}

impl SubAssign<Vec2> for Point {
    fn sub_assign(&mut self, v: Vec2) {
        *self = *self - v;
    }
}

impl Sub for Point {
    type Output = Vec2;
    fn sub(self, other: Point) -> Vec2 {
        Vec2::new(self.x - other.x, self.y - other.y)
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, v: Vec2) -> Vec2 {
        Vec2::new(self.x + v.x, self.y + v.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, v: Vec2) -> Vec2 {
        Vec2::new(self.x - v.x, self.y - v.y)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_corners_any_order() {
        assert_eq!(Rect::new(10.0, 20.0, 0.0, 5.0), Rect::new(0.0, 5.0, 10.0, 20.0));
    }

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(!r.contains(Point::new(10.0, 5.0)));
        assert!(!r.contains(Point::new(5.0, 10.0)));
    }

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 15.0, 15.0);
        assert_eq!(a.intersect(b), Some(Rect::new(5.0, 5.0, 10.0, 10.0)));
        assert_eq!(a.intersect(Rect::new(10.0, 0.0, 20.0, 10.0)), None);
        assert_eq!(a.union(b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert_eq!(Rect::ZERO.union(b), b);
    }

    #[test]
    fn size_never_negative() {
        assert_eq!(Size::new(-3.0, f32::NAN), Size::ZERO);
        assert_eq!(Size::new(4.0, 2.0).inflate(Insets::all(-5.0)), Size::ZERO);
    }

    #[test]
    fn deflate_never_inverts() {
        let r = Rect::new(0.0, 0.0, 10.0, 4.0).deflate(Insets::all(3.0));
        assert_eq!(r.width(), 4.0);
        assert_eq!(r.height(), 0.0);
        assert_eq!(r.center(), Point::new(5.0, 2.0));
    }

    #[test]
    fn radii_fit_the_css_way() {
        let r = CornerRadii::all(30.0).fit(Size::new(40.0, 100.0));
        assert_eq!(r, CornerRadii::all(20.0));
        assert_eq!(CornerRadii::all(-1.0).fit(Size::new(10.0, 10.0)), CornerRadii::ZERO);
        // A pill written with the largest radius there is, or an infinite one, is still a pill.
        for huge in [f32::MAX, f32::INFINITY] {
            assert_eq!(CornerRadii::all(huge).fit(Size::new(40.0, 100.0)), CornerRadii::all(20.0));
        }
    }

    #[test]
    fn mirroring_swaps_start_and_end() {
        let i = Insets { top: 1.0, end: 2.0, bottom: 3.0, start: 4.0 };
        assert_eq!(i.mirrored(), Insets { top: 1.0, end: 4.0, bottom: 3.0, start: 2.0 });
        assert_eq!(i.mirrored().mirrored(), i);
    }
}
