//! Easing curves.

/// A CSS-style cubic Bézier timing curve from (0, 0) to (1, 1) with two control points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    /// First control point's x (0 to 1).
    pub x1: f32,
    /// First control point's y.
    pub y1: f32,
    /// Second control point's x (0 to 1).
    pub x2: f32,
    /// Second control point's y.
    pub y2: f32,
}

impl CubicBezier {
    fn coords(a1: f32, a2: f32, s: f32) -> f32 {
        // B(s) for one axis with endpoints 0 and 1.
        let u = 1.0 - s;
        3.0 * u * u * s * a1 + 3.0 * u * s * s * a2 + s * s * s
    }

    fn deriv(a1: f32, a2: f32, s: f32) -> f32 {
        let u = 1.0 - s;
        3.0 * u * u * a1 + 6.0 * u * s * (a2 - a1) + 3.0 * s * s * (1.0 - a2)
    }

    /// The curve parameter whose x is `x`, by Newton's method with a bisection fallback.
    fn solve(&self, x: f32) -> f32 {
        let (x1, x2) = (self.x1.clamp(0.0, 1.0), self.x2.clamp(0.0, 1.0));
        let mut s = x;
        for _ in 0..8 {
            let err = Self::coords(x1, x2, s) - x;
            if err.abs() < 1e-6 {
                return s;
            }
            let d = Self::deriv(x1, x2, s);
            if d.abs() < 1e-6 {
                break;
            }
            s -= err / d;
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        s = x;
        for _ in 0..40 {
            if Self::coords(x1, x2, s) < x {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.0;
        }
        s
    }

    /// Progress (y) at time fraction `t` (x).
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 {
            return t;
        }
        Self::coords(self.y1, self.y2, self.solve(t))
    }

    /// dy/dx at `t`.
    pub fn slope(&self, t: f32) -> f32 {
        let s = self.solve(t.clamp(0.0, 1.0));
        let dx = Self::deriv(self.x1.clamp(0.0, 1.0), self.x2.clamp(0.0, 1.0), s);
        if dx.abs() < 1e-6 { 0.0 } else { Self::deriv(self.y1, self.y2, s) / dx }
    }
}

/// An easing curve.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Easing {
    /// Constant speed.
    Linear,
    /// A cubic Bézier curve.
    Bezier(CubicBezier),
}

impl Easing {
    /// The default for most interface motion: quick start, gentle stop.
    pub const STANDARD: Easing = Easing::Bezier(CubicBezier { x1: 0.2, y1: 0.0, x2: 0.0, y2: 1.0 });
    /// For things arriving on screen: decelerates into place.
    pub const DECELERATE: Easing = Easing::Bezier(CubicBezier { x1: 0.0, y1: 0.0, x2: 0.0, y2: 1.0 });
    /// For things leaving the screen: accelerates away.
    pub const ACCELERATE: Easing = Easing::Bezier(CubicBezier { x1: 0.3, y1: 0.0, x2: 1.0, y2: 1.0 });
    /// CSS `ease-in-out`.
    pub const EASE_IN_OUT: Easing = Easing::Bezier(CubicBezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 });

    /// Progress at time fraction `t`.
    pub fn apply(&self, t: f32) -> f32 {
        match self {
            Easing::Linear => t.clamp(0.0, 1.0),
            Easing::Bezier(b) => b.apply(t),
        }
    }

    /// Rate of progress at `t`, per unit of `t`.
    pub fn slope(&self, t: f32) -> f32 {
        match self {
            Easing::Linear => 1.0,
            Easing::Bezier(b) => b.slope(t),
        }
    }
}
