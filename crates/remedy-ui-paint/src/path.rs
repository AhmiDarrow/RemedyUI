//! Vector paths.

use remedy_ui_core::{CornerRadii, Point, Rect};

/// One path segment.
#[derive(Clone, Copy, Debug, PartialEq)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum PathEl {
    /// Starts a new subpath.
    MoveTo(Point),
    /// A straight line.
    LineTo(Point),
    /// A quadratic Bézier curve (control, end).
    QuadTo(Point, Point),
    /// A cubic Bézier curve (control 1, control 2, end).
    CubicTo(Point, Point, Point),
    /// Closes the subpath.
    Close,
}

/// A vector path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    els: Vec<PathEl>,
}

impl Path {
    /// An empty path.
    pub fn new() -> Self {
        Self::default()
    }

    /// The segments.
    pub fn elements(&self) -> &[PathEl] {
        &self.els
    }

    /// Starts a subpath.
    pub fn move_to(&mut self, p: Point) -> &mut Self {
        self.els.push(PathEl::MoveTo(p));
        self
    }

    /// A line.
    pub fn line_to(&mut self, p: Point) -> &mut Self {
        self.els.push(PathEl::LineTo(p));
        self
    }

    /// A quadratic curve.
    pub fn quad_to(&mut self, c: Point, p: Point) -> &mut Self {
        self.els.push(PathEl::QuadTo(c, p));
        self
    }

    /// A cubic curve.
    pub fn cubic_to(&mut self, c1: Point, c2: Point, p: Point) -> &mut Self {
        self.els.push(PathEl::CubicTo(c1, c2, p));
        self
    }

    /// Closes the subpath.
    pub fn close(&mut self) -> &mut Self {
        self.els.push(PathEl::Close);
        self
    }

    /// A circle (four cubics).
    pub fn circle(center: Point, r: f32) -> Path {
        const K: f32 = 0.552_284_8;
        let (cx, cy) = (center.x, center.y);
        let mut p = Path::new();
        p.move_to(Point::new(cx + r, cy))
            .cubic_to(Point::new(cx + r, cy + r * K), Point::new(cx + r * K, cy + r), Point::new(cx, cy + r))
            .cubic_to(Point::new(cx - r * K, cy + r), Point::new(cx - r, cy + r * K), Point::new(cx - r, cy))
            .cubic_to(Point::new(cx - r, cy - r * K), Point::new(cx - r * K, cy - r), Point::new(cx, cy - r))
            .cubic_to(Point::new(cx + r * K, cy - r), Point::new(cx + r, cy - r * K), Point::new(cx + r, cy))
            .close();
        p
    }

    /// A rectangle with rounded corners (radii fitted to it as CSS does), clockwise from
    /// the top start corner's end, as one closed subpath.
    pub fn rounded_rect(rect: Rect, radii: CornerRadii) -> Path {
        const K: f32 = 1.0 - 0.552_284_8;
        let r = radii.fit(rect.size());
        let (x0, y0, x1, y1) = (rect.min.x, rect.min.y, rect.max.x, rect.max.y);
        let pt = Point::new;
        let mut p = Path::new();
        p.move_to(pt(x0 + r.top_start, y0)).line_to(pt(x1 - r.top_end, y0));
        if r.top_end > 0.0 {
            let a = r.top_end;
            p.cubic_to(pt(x1 - a * K, y0), pt(x1, y0 + a * K), pt(x1, y0 + a));
        }
        p.line_to(pt(x1, y1 - r.bottom_end));
        if r.bottom_end > 0.0 {
            let a = r.bottom_end;
            p.cubic_to(pt(x1, y1 - a * K), pt(x1 - a * K, y1), pt(x1 - a, y1));
        }
        p.line_to(pt(x0 + r.bottom_start, y1));
        if r.bottom_start > 0.0 {
            let a = r.bottom_start;
            p.cubic_to(pt(x0 + a * K, y1), pt(x0, y1 - a * K), pt(x0, y1 - a));
        }
        p.line_to(pt(x0, y0 + r.top_start));
        if r.top_start > 0.0 {
            let a = r.top_start;
            p.cubic_to(pt(x0, y0 + a * K), pt(x0 + a * K, y0), pt(x0 + a, y0));
        }
        p.close();
        p
    }

    /// The box around every point, control points included (a conservative bound).
    pub fn bounds(&self) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut add = |p: Point| {
            let r = Rect::from_points(p, p);
            out = Some(out.map_or(r, |o| {
                Rect::new(o.min.x.min(p.x), o.min.y.min(p.y), o.max.x.max(p.x), o.max.y.max(p.y))
            }));
        };
        for el in &self.els {
            match *el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => add(p),
                PathEl::QuadTo(c, p) => {
                    add(c);
                    add(p);
                }
                PathEl::CubicTo(a, b, p) => {
                    add(a);
                    add(b);
                    add(p);
                }
                PathEl::Close => {}
            }
        }
        out
    }
}

/// How line ends look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum StrokeCap {
    /// Flat at the end point.
    #[default]
    Butt,
    /// Rounded.
    Round,
    /// Flat, half the width past the end point.
    Square,
}

/// How corners look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Exhaustive on purpose: part of the display list every renderer must draw completely, so a new kind is a major change.
pub enum StrokeJoin {
    /// Sharp (beveled past the miter limit).
    #[default]
    Miter,
    /// Rounded.
    Round,
    /// Cut off.
    Bevel,
}

/// A stroke style.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    /// Width in logical pixels.
    pub width: f32,
    /// Line ends.
    pub cap: StrokeCap,
    /// Corners.
    pub join: StrokeJoin,
    /// Dash lengths, alternating on and off; empty for a solid line.
    pub dashes: Vec<f32>,
}

impl Stroke {
    /// A solid stroke of `width`.
    pub fn new(width: f32) -> Self {
        Stroke { width, cap: StrokeCap::Butt, join: StrokeJoin::Miter, dashes: Vec::new() }
    }
}
