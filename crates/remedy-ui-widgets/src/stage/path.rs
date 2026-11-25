//! Children along a line or curve (a metro line, a river), with the line drawn.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::reactive::untrack;
use remedy_ui_core::{Point, Rect, Size};
use remedy_ui_engine::paint::Path;
use remedy_ui_engine::view::{El, IntoProp, Ui};

use crate::stage::common::{LineStyle, Place, Slot, layer};

/// Segments each curved span is flattened into.
const SPAN_STEPS: usize = 16;

/// A polyline or smooth curve through points, measured by length so things can be spaced
/// along it.
///
/// ```
/// use remedy_ui_core::Point;
/// use remedy_ui_widgets::stage::Route;
///
/// let r = Route::new([Point::new(0.0, 0.0), Point::new(3.0, 0.0), Point::new(3.0, 4.0)]);
/// assert_eq!(r.length(), 7.0);
/// assert_eq!(r.point_at(3.0 / 7.0), Point::new(3.0, 0.0));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    points: Vec<Point>,
    smooth: bool,
    closed: bool,
}

impl Route {
    /// Straight lines through `points`.
    pub fn new(points: impl IntoIterator<Item = Point>) -> Self {
        let points = points.into_iter().filter(|p| p.x.is_finite() && p.y.is_finite()).collect();
        Route { points, smooth: false, closed: false }
    }

    /// A smooth curve through the points instead (Catmull-Rom), as a river bends.
    pub fn smooth(mut self) -> Self {
        self.smooth = true;
        self
    }

    /// Joins the last point back to the first (a loop line).
    pub fn closed(mut self) -> Self {
        self.closed = true;
        self
    }

    /// The route scaled by `sx` and `sy`.
    pub fn scaled(&self, sx: f32, sy: f32) -> Route {
        Route { points: self.points.iter().map(|p| Point::new(p.x * sx, p.y * sy)).collect(), ..*self }
    }

    /// The route as straight segments: its points, or for a smooth route the curve flattened.
    pub fn flatten(&self) -> Vec<Point> {
        let mut pts = self.points.clone();
        if self.closed
            && let Some(&first) = pts.first()
        {
            pts.push(first);
        }
        if !self.smooth || pts.len() < 3 {
            return pts;
        }
        let n = pts.len();
        let at = |i: isize| -> Point {
            let i = if self.closed {
                // The loop's ends meet: wrap past them (the last point repeats the first).
                i.rem_euclid(n as isize - 1) as usize
            } else {
                i.clamp(0, n as isize - 1) as usize
            };
            pts.get(i).copied().unwrap_or(Point::ZERO)
        };
        let mut out = Vec::with_capacity((n - 1) * SPAN_STEPS + 1);
        for i in 0..n - 1 {
            let i = i as isize;
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            for s in 0..SPAN_STEPS {
                let t = s as f32 / SPAN_STEPS as f32;
                let (t2, t3) = (t * t, t * t * t);
                let f = |a: f32, b: f32, c: f32, d: f32| {
                    0.5 * (2.0 * b
                        + (-a + c) * t
                        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                        + (-a + 3.0 * b - 3.0 * c + d) * t3)
                };
                out.push(Point::new(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y)));
            }
        }
        out.push(at(n as isize - 1));
        out
    }

    /// The route's length.
    pub fn length(&self) -> f32 {
        self.flatten().windows(2).map(|w| seg(w).1).sum()
    }

    /// The point a `fraction` (0 to 1) of the way along the route, by length.
    pub fn point_at(&self, fraction: f32) -> Point {
        point_on(&self.flatten(), fraction)
    }

    /// The route as a path for drawing.
    pub fn to_path(&self) -> Path {
        let mut p = Path::new();
        for (i, pt) in self.flatten().into_iter().enumerate() {
            if i == 0 {
                p.move_to(pt);
            } else {
                p.line_to(pt);
            }
        }
        p
    }
}

fn seg(w: &[Point]) -> ((Point, Point), f32) {
    match w {
        [a, b] => ((*a, *b), ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()),
        _ => ((Point::ZERO, Point::ZERO), 0.0),
    }
}

fn point_on(pts: &[Point], fraction: f32) -> Point {
    let total: f32 = pts.windows(2).map(|w| seg(w).1).sum();
    let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
    let mut left = total * f;
    for w in pts.windows(2) {
        let ((a, b), len) = seg(w);
        if left <= len && len > 0.0 {
            return a.lerp(b, left / len);
        }
        left -= len;
    }
    pts.last().copied().unwrap_or(Point::ZERO)
}

/// How children without a fraction of their own are spaced along a [`path_layout`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Spread {
    /// The first at the start, the last at the end, the rest evenly between (stations on a
    /// line). One child sits in the middle.
    #[default]
    Ends,
    /// Each in the middle of an equal share of the route (stones along a river).
    Centered,
}

/// How a [`path_layout`] maps and draws its route.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PathOptions {
    /// The space the route's points are in, stretched over the box. The default (1 by 1)
    /// takes points as fractions of the box.
    pub design: Size,
    /// How children without a fraction are spaced.
    pub spread: Spread,
    /// How the route is drawn; `None` does not draw it.
    pub line: Option<LineStyle>,
}

impl Default for PathOptions {
    fn default() -> Self {
        PathOptions { design: Size::new(1.0, 1.0), spread: Spread::Ends, line: None }
    }
}

impl PathOptions {
    /// Sets `design`: the space the route's points are in.
    pub fn design(mut self, value: Size) -> Self {
        self.design = Size::new(value.width.max(f32::MIN_POSITIVE), value.height.max(f32::MIN_POSITIVE));
        self
    }

    /// Sets `spread`: how children without a fraction are spaced.
    pub fn spread(mut self, value: Spread) -> Self {
        self.spread = value;
        self
    }

    /// Sets `line`: how the route is drawn.
    pub fn line(mut self, value: LineStyle) -> Self {
        self.line = Some(value);
        self
    }
}

struct Stop {
    slot: Slot,
    at: Option<Rc<Cell<f32>>>,
}

struct State {
    route: Route,
    options: PathOptions,
    stops: RefCell<Vec<Stop>>,
    size: Cell<Option<Size>>,
    flat: RefCell<Vec<Point>>,
    content: El,
    el: El,
}

impl State {
    fn mapped(&self, size: Size) -> Route {
        // As `PathOptions::design` keeps it: a design of no size has no scale.
        let d = self.options.design;
        let (w, h) = (d.width.max(f32::MIN_POSITIVE), d.height.max(f32::MIN_POSITIVE));
        self.route.scaled(size.width / w, size.height / h)
    }

    fn put_all(&self) {
        let Some(size) = self.size.get() else { return };
        let rtl = self.el.is_rtl();
        let flat = self.flat.borrow();
        let stops = self.stops.borrow();
        let even = stops.iter().filter(|s| s.at.is_none()).count();
        let mut k = 0;
        for s in stops.iter() {
            let f = match &s.at {
                Some(at) => at.get(),
                None => {
                    let f = match (self.options.spread, even) {
                        (Spread::Ends, 1) => 0.5,
                        (Spread::Ends, n) => k as f32 / (n - 1) as f32,
                        (Spread::Centered, n) => (k as f32 + 0.5) / n as f32,
                    };
                    k += 1;
                    f
                }
            };
            let p = point_on(&flat, f);
            s.slot.put(Rect::new(p.x, p.y, p.x, p.y), Place::Center, Place::Center, rtl, size.width);
        }
    }
}

/// Children along a route, made by [`path_layout`].
#[derive(Clone)]
pub struct PathLayout(Rc<State>);

impl std::fmt::Debug for PathLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathLayout").field("route", &self.0.route).finish_non_exhaustive()
    }
}

/// Children placed along `route`, stretched over the box: stations on a metro line, stones
/// along a river. Children added with [`PathLayout::child`] are spread by
/// [`PathOptions::spread`]; [`PathLayout::child_at`] puts one at a fraction of the route's
/// length (a constant, signal or closure, so it can travel). With [`PathOptions::line`] the
/// route itself is drawn behind them, solid or dashed, in theme colors.
///
/// Children keep their own size and are centered on their spot; they are read and focused in
/// the order added. The layout is a group named `label`.
#[track_caller]
pub fn path_layout(ui: &Ui, label: impl Into<String>, route: Route, options: PathOptions) -> PathLayout {
    let label = label.into();
    let content = layer(ui, "path_content");
    let el = ui.el("path_layout").child(content.clone());
    let state = Rc::new(State {
        route,
        options,
        stops: RefCell::new(Vec::new()),
        size: Cell::new(None),
        flat: RefCell::new(Vec::new()),
        content,
        el: el.clone(),
    });
    let s = state.clone();
    let el = el.paint(move |cx| {
        if let Some(line) = &s.options.line {
            let path = s.mapped(cx.size).to_path();
            line.stroke(cx, path);
        }
    });
    let s = state.clone();
    let el = el.on_resize(move |size| {
        s.size.set(Some(size));
        *s.flat.borrow_mut() = s.mapped(size).flatten();
        s.put_all();
    });
    if !label.is_empty() {
        let _ = el.semantics(Semantics::new(Role::Group).label(label));
    }
    PathLayout(state)
}

impl PathLayout {
    /// Adds a child spread along the route with the others added this way.
    pub fn child(self, child: impl Into<El>) -> Self {
        self.add(child.into(), None);
        self
    }

    /// Adds a child at `fraction` (0 to 1) of the route's length.
    pub fn child_at(self, fraction: impl IntoProp<f32>, child: impl Into<El>) -> Self {
        let fraction = fraction.into_prop();
        let at = Rc::new(Cell::new(untrack(|| fraction.get())));
        let slot_el = self.add(child.into(), Some(at.clone()));
        let s = self.0.clone();
        slot_el.prop("fraction", fraction, move |_, f| {
            at.set(f);
            s.put_all();
        });
        self
    }

    fn add(&self, child: El, at: Option<Rc<Cell<f32>>>) -> El {
        let slot = Slot::new(self.0.el.ui(), child, Place::Center);
        let el = slot.el.clone();
        let _ = self.0.content.clone().child(el.clone());
        self.0.stops.borrow_mut().push(Stop { slot, at });
        self.0.put_all();
        el
    }

    /// The route.
    pub fn route(&self) -> &Route {
        &self.0.route
    }

    /// The layout's element.
    pub fn el(&self) -> &El {
        &self.0.el
    }
}

impl From<PathLayout> for El {
    fn from(p: PathLayout) -> El {
        p.0.el.clone()
    }
}
