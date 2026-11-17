//! Values that can be animated: anything that can be split into numbers and put back.

use remedy_ui_core::{Color, CornerRadii, Insets, Point, Rect, Size, Vec2};

/// A value an [`crate::Animation`] can move. Each component moves on its own; colors move
/// in Oklab so fades do not pass through grey.
pub trait Animatable: Clone {
    /// The value as numbers.
    fn to_components(&self) -> Vec<f32>;
    /// The value from numbers (as many as `to_components` gives).
    fn from_components(c: &[f32]) -> Self;
}

fn at(c: &[f32], i: usize) -> f32 {
    c.get(i).copied().unwrap_or(0.0)
}

impl Animatable for f32 {
    fn to_components(&self) -> Vec<f32> {
        vec![*self]
    }
    fn from_components(c: &[f32]) -> Self {
        at(c, 0)
    }
}

impl Animatable for Point {
    fn to_components(&self) -> Vec<f32> {
        vec![self.x, self.y]
    }
    fn from_components(c: &[f32]) -> Self {
        Point::new(at(c, 0), at(c, 1))
    }
}

impl Animatable for Vec2 {
    fn to_components(&self) -> Vec<f32> {
        vec![self.x, self.y]
    }
    fn from_components(c: &[f32]) -> Self {
        Vec2::new(at(c, 0), at(c, 1))
    }
}

impl Animatable for Size {
    fn to_components(&self) -> Vec<f32> {
        vec![self.width, self.height]
    }
    fn from_components(c: &[f32]) -> Self {
        Size::new(at(c, 0), at(c, 1))
    }
}

impl Animatable for Rect {
    fn to_components(&self) -> Vec<f32> {
        vec![self.min.x, self.min.y, self.max.x, self.max.y]
    }
    fn from_components(c: &[f32]) -> Self {
        Rect::new(at(c, 0), at(c, 1), at(c, 2), at(c, 3))
    }
}

impl Animatable for Insets {
    fn to_components(&self) -> Vec<f32> {
        vec![self.top, self.end, self.bottom, self.start]
    }
    fn from_components(c: &[f32]) -> Self {
        Insets { top: at(c, 0), end: at(c, 1), bottom: at(c, 2), start: at(c, 3) }
    }
}

impl Animatable for CornerRadii {
    fn to_components(&self) -> Vec<f32> {
        vec![self.top_start, self.top_end, self.bottom_end, self.bottom_start]
    }
    fn from_components(c: &[f32]) -> Self {
        CornerRadii { top_start: at(c, 0), top_end: at(c, 1), bottom_end: at(c, 2), bottom_start: at(c, 3) }
    }
}

impl Animatable for Color {
    /// Oklab lightness, a, b and alpha, so a tween between two colors keeps its brightness.
    fn to_components(&self) -> Vec<f32> {
        self.to_oklab().to_vec()
    }
    fn from_components(c: &[f32]) -> Self {
        Color::from_oklab(at(c, 0), at(c, 1), at(c, 2), at(c, 3))
    }
}
