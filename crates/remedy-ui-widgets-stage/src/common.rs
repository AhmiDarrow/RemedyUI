//! What the stage widgets share: colors from tokens, placing a child at a point or in a box,
//! and the timer loop that animates only while it is seen and motion is allowed.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::{Color, Rect};
use remedy_ui_core::layout::{AUTO, Align, Justify, Length, Position};
use remedy_ui_core::reactive::untrack;
use remedy_ui_core::style::{ColorRole, Theme};
use remedy_ui_view::{El, TickKind, Ui};
use remedy_ui_viz::{Clock, Moved, Ticker};

/// A color for art and strokes: a theme role (which follows the theme) or a fixed color.
///
/// Fixed colors are for art with its own palette (a sprite's skin and eyes); everything that is
/// part of the interface should name a role, so a skin restyles it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Ink {
    /// A theme color role, resolved under the window's theme and again when it changes.
    Role(ColorRole),
    /// A fixed color.
    Color(Color),
    /// Nothing: transparent.
    Clear,
}

impl Ink {
    /// The color under `theme`.
    pub fn resolve(self, theme: &Theme) -> Color {
        match self {
            Ink::Role(r) => r.color(theme),
            Ink::Color(c) => c,
            Ink::Clear => Color::TRANSPARENT,
        }
    }
}

impl From<ColorRole> for Ink {
    fn from(r: ColorRole) -> Self {
        Ink::Role(r)
    }
}

impl From<Color> for Ink {
    fn from(c: Color) -> Self {
        Ink::Color(c)
    }
}

/// Where a child sits along one axis of the box it is placed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Place {
    /// Its start edge (left or top) on the box's start edge. For a point, the child hangs
    /// after the point.
    Start,
    /// Centered on the box (or on the point).
    #[default]
    Center,
    /// Its end edge on the box's end edge. For a point, the child ends at the point.
    End,
    /// Stretched across the box.
    Fill,
}

/// A slot: an absolute box inside a container that holds one child and places it with
/// [`Place`]s. It lets clicks through itself to what is below; its child still takes them.
pub(crate) struct Slot {
    pub(crate) el: El,
    last: Cell<Option<(Rect, Place, Place, bool)>>,
}

impl Slot {
    pub(crate) fn new(ui: &Ui, child: El, x: Place) -> Slot {
        let child = if x == Place::Fill {
            child.grow(1.0).layout(|l| l.min_width = Length::Px(0.0))
        } else {
            child.layout(|l| l.shrink = 0.0)
        };
        let el = ui
            .el("stage_slot")
            .row()
            .pass_through()
            .layout(|l| {
                l.position = Position::Absolute;
                l.inset.top = 0.0;
                l.inset.start = 0.0;
                l.inset.end = AUTO;
                l.inset.bottom = AUTO;
                l.width = Length::Px(0.0);
                l.height = Length::Px(0.0);
                l.align_items = Align::Center;
                l.justify = Justify::Center;
            })
            .child(child);
        Slot { el, last: Cell::new(None) }
    }

    /// Puts the slot over `r` (physical coordinates in the container, which is `box_w`
    /// wide). Only edits the layout when something changed.
    pub(crate) fn put(&self, r: Rect, x: Place, y: Place, rtl: bool, box_w: f32) {
        let key = (r, x, y, rtl);
        if self.last.get() == Some(key) {
            return;
        }
        self.last.set(Some(key));
        // The container's row mirrors in right to left, so the physical sides swap there.
        let x = match (x, rtl) {
            (Place::Start, true) => Place::End,
            (Place::End, true) => Place::Start,
            (p, _) => p,
        };
        let start = if rtl { box_w - r.max.x } else { r.min.x };
        let _ = self.el.clone().layout(|l| {
            l.inset.top = r.min.y;
            l.inset.start = start;
            l.width = Length::Px(r.width().max(0.0));
            l.height = Length::Px(r.height().max(0.0));
            l.justify = match x {
                Place::Start | Place::Fill => Justify::Start,
                Place::Center => Justify::Center,
                Place::End => Justify::End,
            };
            l.align_items = match y {
                Place::Start => Align::Start,
                Place::Center => Align::Center,
                Place::End => Align::End,
                Place::Fill => Align::Stretch,
            };
        });
    }
}

/// A container that fills its parent and holds slots, letting clicks through itself.
pub(crate) fn layer(ui: &Ui, widget: &'static str) -> El {
    ui.el(widget).pass_through().layout(|l| {
        l.position = Position::Absolute;
        l.inset = remedy_ui_core::Insets::ZERO;
    })
}

/// Runs `step` on a timer while `el` is seen, motion is not reduced and `step` asks for more.
/// It starts when the element is first seen, and again whenever [`Motion::kick`] is called
/// (the thing that moves changed) or motion is allowed again. A widget at rest, hidden or
/// under reduced motion schedules nothing. Decoration ([`Motion::attach`]) also follows the
/// window's ambient policy: its rate, and stopping while ambient motion is off or paused.
#[derive(Clone)]
pub(crate) struct Motion {
    ticker: Ticker,
    seen: Rc<Cell<bool>>,
}

impl Motion {
    /// Attaches the loop to `el` and returns the element.
    pub(crate) fn attach(
        el: El,
        clock: Clock,
        step: impl FnMut(Duration) -> Moved + 'static,
    ) -> (El, Motion) {
        Self::attach_kind(el, clock, TickKind::Ambient, step)
    }

    /// Like [`Motion::attach`], for motion of `kind` (a glide that follows data is
    /// [`TickKind::Live`]: it settles even while ambient motion is off).
    pub(crate) fn attach_kind(
        el: El,
        clock: Clock,
        kind: TickKind,
        mut step: impl FnMut(Duration) -> Moved + 'static,
    ) -> (El, Motion) {
        let seen = Rc::new(Cell::new(false));
        let ui = el.ui().clone();
        let s2 = seen.clone();
        let ticker = Ticker::of_kind(&el, clock, kind, move |dt| {
            if !s2.get() || untrack(|| ui.reduced_motion()) || ui.tick_interval(kind).is_none() {
                return Moved::default();
            }
            step(dt)
        });
        let motion = Motion { ticker, seen };
        let m2 = motion.clone();
        let m3 = motion.clone();
        let ui = el.ui().clone();
        let was_reduced = Cell::new(None);
        let me = el.clone();
        let el = el
            .on_visibility(move |v| {
                m2.seen.set(v);
                if v {
                    m2.ticker.kick();
                }
            })
            .bind_named("reduced_motion", move || {
                // Ambient motion paused counts as reduced for decoration.
                let paused = kind == TickKind::Ambient && ui.ambient().is_none();
                let reduced = ui.reduced_motion() || paused;
                let was = was_reduced.replace(Some(reduced));
                if was.is_some_and(|w| w != reduced) {
                    // Painters draw their still frame (or start moving again).
                    me.repaint();
                }
                if was == Some(true) && !reduced {
                    let m = m3.clone();
                    let _ = ui.after(Duration::ZERO, move || m.kick());
                }
            });
        (el, motion)
    }

    /// Steps at most once every `interval` (a sprite's frame time).
    pub(crate) fn set_min_interval(&self, interval: Duration) {
        self.ticker.set_min_interval(interval);
    }

    /// Starts the loop if it is not running and may run.
    pub(crate) fn kick(&self) {
        if self.seen.get() {
            self.ticker.kick();
        }
    }
}

/// The number of whole steps of `step` in `v - min`, as a value again, kept in `min..=max`.
pub(crate) fn snap(v: f64, min: f64, max: f64, step: f64) -> f64 {
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    if !v.is_finite() {
        return lo;
    }
    let v = if step > 0.0 { lo + ((v - lo) / step).round() * step } else { v };
    v.clamp(lo, hi)
}

/// A number for people: whole numbers without decimals, others with as many as `step` has.
pub(crate) fn number_text(v: f64, step: f64) -> String {
    if step >= 1.0 && step.fract() == 0.0 || step <= 0.0 && v.fract() == 0.0 {
        return format!("{v:.0}");
    }
    let digits = if step > 0.0 { (-step.log10()).ceil().clamp(0.0, 6.0) as usize } else { 2 };
    format!("{v:.digits$}")
}

/// How a line is drawn (a path, an orbit's ring): its color, width and dashes.
#[derive(Clone, Debug, PartialEq)]
pub struct LineStyle {
    /// The color.
    pub ink: Ink,
    /// The width in logical pixels; `None` takes the theme's stroke width (at least one).
    pub width: Option<f32>,
    /// Dash lengths, alternating on and off, in theme units (a unit is 4 dp by default);
    /// empty for a solid line.
    pub dashes: Vec<f32>,
}

impl LineStyle {
    /// A solid line in `ink`.
    pub fn solid(ink: impl Into<Ink>) -> Self {
        LineStyle { ink: ink.into(), width: None, dashes: Vec::new() }
    }

    /// A dashed line in `ink`: two units on, one and a half off.
    pub fn dashed(ink: impl Into<Ink>) -> Self {
        LineStyle { ink: ink.into(), width: None, dashes: vec![2.0, 1.5] }
    }

    /// Sets the width in logical pixels.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.max(0.0));
        self
    }

    /// Sets the dash pattern in theme units.
    pub fn dashes(mut self, dashes: Vec<f32>) -> Self {
        self.dashes = dashes;
        self
    }

    /// Strokes `path` with this style under the painter's theme.
    pub(crate) fn stroke(&self, cx: &mut remedy_ui_view::PaintCx<'_>, path: remedy_ui_paint::Path) {
        let t = cx.theme;
        let mut stroke = remedy_ui_paint::Stroke::new(self.width.unwrap_or(t.stroke.max(1.0)));
        stroke.cap = remedy_ui_paint::StrokeCap::Round;
        stroke.join = remedy_ui_paint::StrokeJoin::Round;
        stroke.dashes = self.dashes.iter().map(|d| d.max(0.0) * t.spacing.unit).collect();
        let brush = remedy_ui_paint::Brush::Solid(self.ink.resolve(t));
        cx.list.push(remedy_ui_paint::Item::StrokePath { path, stroke, brush });
    }
}

/// An ellipse centered on `c` with radii `rx` and `ry`, as four cubic arcs.
pub(crate) fn ellipse(c: remedy_ui_core::Point, rx: f32, ry: f32) -> remedy_ui_paint::Path {
    use remedy_ui_core::Point;
    const K: f32 = 0.552_284_8;
    let (kx, ky) = (rx * K, ry * K);
    let mut p = remedy_ui_paint::Path::new();
    p.move_to(Point::new(c.x + rx, c.y));
    p.cubic_to(Point::new(c.x + rx, c.y + ky), Point::new(c.x + kx, c.y + ry), Point::new(c.x, c.y + ry));
    p.cubic_to(Point::new(c.x - kx, c.y + ry), Point::new(c.x - rx, c.y + ky), Point::new(c.x - rx, c.y));
    p.cubic_to(Point::new(c.x - rx, c.y - ky), Point::new(c.x - kx, c.y - ry), Point::new(c.x, c.y - ry));
    p.cubic_to(Point::new(c.x + kx, c.y - ry), Point::new(c.x + rx, c.y - ky), Point::new(c.x + rx, c.y));
    p.close();
    p
}
