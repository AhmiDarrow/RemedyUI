//! Rotary controls: a knob for a continuous value and a tuner dial of labelled stations.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Point, Rect, Vec2};
use remedy_ui_core::input::{Key, KeyEvent, PointerPhase, WheelEvent};
use remedy_ui_engine::paint::{Brush, Item, Path, Stroke, StrokeCap};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, PaintCx, TextPaint, Ui};
use remedy_ui_viz::{Clock, Moved, smooth};

use crate::common::{Motion, number_text, snap};

/// Turns a value into what assistive technology reads (and a knob's tooltip-like readout).
pub type ValueText = Rc<dyn Fn(f64) -> String>;

/// An arc around `c` of radius `r` from angle `a0` to `a1` (radians, clockwise from the top).
fn arc(c: Point, r: f32, a0: f32, a1: f32) -> Path {
    let steps = (((a1 - a0).abs() / std::f32::consts::TAU) * 64.0).ceil().max(2.0) as usize;
    let mut p = Path::new();
    for i in 0..=steps {
        let a = a0 + (a1 - a0) * i as f32 / steps as f32;
        let pt = polar(c, r, a);
        if i == 0 {
            p.move_to(pt);
        } else {
            p.line_to(pt);
        }
    }
    p
}

/// The point at angle `a` (radians, clockwise from the top) and radius `r` around `c`.
fn polar(c: Point, r: f32, a: f32) -> Point {
    Point::new(c.x + r * a.sin(), c.y - r * a.cos())
}

fn line(cx: &mut PaintCx<'_>, a: Point, b: Point, width: f32, color: remedy_ui_core::Color) {
    let mut p = Path::new();
    p.move_to(a);
    p.line_to(b);
    let mut s = Stroke::new(width);
    s.cap = StrokeCap::Round;
    cx.list.push(Item::StrokePath { path: p, stroke: s, brush: Brush::Solid(color) });
}

fn stroke_path(cx: &mut PaintCx<'_>, path: Path, width: f32, color: remedy_ui_core::Color) {
    let mut s = Stroke::new(width);
    s.cap = StrokeCap::Round;
    cx.list.push(Item::StrokePath { path, stroke: s, brush: Brush::Solid(color) });
}

/// How a [`knob`] behaves and looks.
#[derive(Clone)]
#[non_exhaustive]
pub struct KnobOptions {
    /// The lowest value.
    pub min: f64,
    /// The highest value.
    pub max: f64,
    /// Values snap to multiples of this from `min`; zero for none (the keyboard then moves a
    /// hundredth of the range).
    pub step: f64,
    /// How far the knob turns from lowest to highest, in degrees (centered on the top).
    pub sweep: f32,
    /// The knob's diameter. `None` takes twice the theme's control height.
    pub size: Option<f32>,
    /// What assistive technology reads for a value. `None` reads the number.
    pub value_text: Option<ValueText>,
}

impl std::fmt::Debug for KnobOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnobOptions")
            .field("min", &self.min)
            .field("max", &self.max)
            .field("step", &self.step)
            .field("sweep", &self.sweep)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Default for KnobOptions {
    fn default() -> Self {
        KnobOptions { min: 0.0, max: 1.0, step: 0.0, sweep: 270.0, size: None, value_text: None }
    }
}

impl KnobOptions {
    /// Sets `min` and `max`: the value's range.
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// Sets `step`: values snap to multiples of this from `min`.
    pub fn step(mut self, value: f64) -> Self {
        self.step = if value.is_finite() { value.max(0.0) } else { 0.0 };
        self
    }

    /// Sets `sweep`: how far the knob turns, in degrees.
    pub fn sweep(mut self, value: f32) -> Self {
        self.sweep = if value.is_finite() { value.clamp(10.0, 360.0) } else { 270.0 };
        self
    }

    /// Sets `size`: the knob's diameter.
    pub fn size(mut self, value: f32) -> Self {
        self.size = Some(value.max(8.0));
        self
    }

    /// Sets `value_text`: what assistive technology reads for a value ("40 percent").
    pub fn value_text(mut self, f: impl Fn(f64) -> String + 'static) -> Self {
        self.value_text = Some(Rc::new(f));
        self
    }
}

/// The arrow, page, Home and End keys as a new value, or `None` for other keys.
pub(crate) fn step_key(e: &KeyEvent, cur: f64, min: f64, max: f64, step: f64) -> Option<f64> {
    let span = max - min;
    let small = if step > 0.0 { step } else { span / 100.0 };
    let big = (span / 10.0).max(small);
    Some(match e.key {
        Key::ArrowUp | Key::ArrowRight => cur + small,
        Key::ArrowDown | Key::ArrowLeft => cur - small,
        Key::PageUp => cur + big,
        Key::PageDown => cur - big,
        Key::Home => min,
        Key::End => max,
        _ => return None,
    })
}

/// One notch of the wheel: +1 up (or right), -1 down.
fn wheel_notch(e: &WheelEvent) -> f64 {
    let d = if e.delta.y != 0.0 { -e.delta.y } else { e.delta.x };
    if d > 0.0 {
        1.0
    } else if d < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// A rotary knob for `value` between `min` and `max`: an arc shows the value, a pointer on
/// the cap shows where it is turned to.
///
/// Drag up or right to turn it up (hold Shift for fine control), use the wheel, or the arrow
/// keys (Page Up and Page Down for big steps, Home and End for the ends). It is a slider to
/// assistive technology, with increment and decrement and the value read by
/// [`KnobOptions::value_text`].
#[track_caller]
pub fn knob(ui: &Ui, label: impl Into<String>, value: Signal<f64>, options: KnobOptions) -> El {
    let label = label.into();
    let (min, max, step) = (options.min, options.max, options.step);
    let span = (max - min).abs().max(f64::MIN_POSITIVE);
    let size = options.size.unwrap_or(ui.theme().spacing.control_height * 2.0);
    let sweep = options.sweep.to_radians();
    let text = options.value_text.clone().unwrap_or_else(|| Rc::new(move |v| number_text(v, step)));
    let drag_px = f64::from(ui.theme().spacing.unit * 50.0);
    let grab: Rc<Cell<Option<(Point, f64)>>> = Rc::new(Cell::new(None));

    let el = ui.el("knob").width(size).height(size).focusable(true).paint(move |cx| {
        let v = value.get_untracked().unwrap_or(min);
        let t = (((v - min) / span).clamp(0.0, 1.0)) as f32;
        let d = cx.theme.derive();
        let s = cx.size.width.min(cx.size.height);
        let c = Point::new(cx.size.width / 2.0, cx.size.height / 2.0);
        let w = (s * 0.08).max(2.0);
        let r = s / 2.0 - w;
        let a0 = -sweep / 2.0;
        let a = a0 + sweep * t;
        stroke_path(cx, arc(c, r, a0, sweep / 2.0), w, d.line);
        if t > 0.0 {
            stroke_path(cx, arc(c, r, a0, a), w, cx.style.foreground);
        }
        let body = r - w * 1.5;
        cx.list.push(Item::Shadow {
            rect: Rect::new(c.x - body, c.y - body, c.x + body, c.y + body),
            radii: remedy_ui_core::CornerRadii::all(body),
            offset: Vec2::new(0.0, w * 0.5),
            blur: w * 1.5,
            spread: 0.0,
            color: remedy_ui_core::Color::BLACK.with_alpha(0.35),
        });
        cx.list.push(Item::FillPath {
            path: Path::circle(c, body),
            brush: Brush::Solid(cx.theme.colors.card),
            even_odd: false,
        });
        stroke_path(cx, Path::circle(c, body), cx.theme.stroke.max(1.0), d.line);
        line(cx, polar(c, body * 0.35, a), polar(c, body * 0.85, a), w, d.text);
    });

    let set = move |v: f64| value.set_if_changed(snap(v, min, max, step));
    let (g, me) = (grab.clone(), el.clone());
    let el = el
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down => {
                g.set(Some((e.position, value.get_untracked().unwrap_or(min))));
                me.focus();
                true
            }
            PointerPhase::Move => match g.get() {
                Some((at, start)) => {
                    let moved = f64::from((e.position.x - at.x) - (e.position.y - at.y));
                    let fine = if e.modifiers.shift { 10.0 } else { 1.0 };
                    set(start + moved / (drag_px * fine) * span);
                    true
                }
                None => false,
            },
            PointerPhase::Up | PointerPhase::Cancel => g.replace(None).is_some(),
        })
        .on_wheel(move |e| {
            let n = wheel_notch(e);
            if n == 0.0 {
                return false;
            }
            let small = if step > 0.0 { step } else { span / 100.0 };
            set(value.get_untracked().unwrap_or(min) + n * small);
            true
        })
        .on_key(move |e| {
            if !e.pressed {
                return false;
            }
            match step_key(e, value.get_untracked().unwrap_or(min), min, max, step) {
                Some(v) => {
                    set(v);
                    true
                }
                None => false,
            }
        });
    let me = el.clone();
    el.bind(move || {
        let v = value.get();
        let _ = me.clone().semantics(
            Semantics::new(Role::Slider)
                .label(label.clone())
                .numeric(v, min, max)
                .value(text(v))
                .action(Action::Increment)
                .action(Action::Decrement),
        );
        me.repaint();
    })
}

/// A station on a [`tuner`]: a label, an optional detail (a frequency, a count) and where on
/// the dial it sits.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    /// What it is called (read out as the tuner's value).
    pub label: String,
    /// A second line under the label.
    pub detail: Option<String>,
    /// Where on the dial, 0 to 1. `None` spaces stations evenly.
    pub at: Option<f32>,
}

impl Station {
    /// A station called `label`.
    pub fn new(label: impl Into<String>) -> Self {
        Station { label: label.into(), detail: None, at: None }
    }

    /// Sets the second line under the label ("02.4").
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Puts the station at `at` (0 to 1) along the dial.
    pub fn at(mut self, at: f32) -> Self {
        self.at = at.is_finite().then(|| at.clamp(0.0, 1.0));
        self
    }
}

/// The shape of a [`tuner`]'s dial.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TunerShape {
    /// A straight dial across the width, as on a radio's front.
    #[default]
    Line,
    /// An arc, as on a meter.
    Arc,
}

/// How a [`tuner`] looks and moves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TunerOptions {
    /// The dial's shape.
    pub shape: TunerShape,
    /// How many small ticks the scale has between its ends.
    pub ticks: u32,
    /// An arc dial's sweep, in degrees (centered on the top).
    pub sweep: f32,
    /// Where time comes from (the needle glides to a new station; it jumps under reduced
    /// motion).
    pub clock: Clock,
}

impl Default for TunerOptions {
    fn default() -> Self {
        TunerOptions { shape: TunerShape::Line, ticks: 40, sweep: 120.0, clock: Clock::system() }
    }
}

impl TunerOptions {
    /// Sets `shape`: the dial's shape.
    pub fn shape(mut self, value: TunerShape) -> Self {
        self.shape = value;
        self
    }

    /// Sets `ticks`: how many small ticks the scale has.
    pub fn ticks(mut self, value: u32) -> Self {
        self.ticks = value.min(1000);
        self
    }

    /// Sets `sweep`: an arc dial's sweep, in degrees.
    pub fn sweep(mut self, value: f32) -> Self {
        self.sweep = if value.is_finite() { value.clamp(20.0, 300.0) } else { 120.0 };
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

struct Needle {
    /// Where the needle is drawn, 0 to 1.
    shown: Cell<f32>,
    /// Where it is going.
    target: Cell<f32>,
    dragging: Cell<bool>,
}

/// The dial's geometry in a box: a position 0..1 to a point on the scale, and back.
#[derive(Clone, Copy)]
struct Geometry {
    shape: TunerShape,
    w: f32,
    h: f32,
    pad: f32,
    sweep: f32,
}

impl Geometry {
    fn line_y(&self) -> f32 {
        self.h * 0.42
    }

    fn center(&self) -> Point {
        Point::new(self.w / 2.0, self.h - self.pad)
    }

    fn radius(&self) -> f32 {
        (self.h - self.pad * 2.0).min(self.w / 2.0 - self.pad).max(1.0)
    }

    fn angle(&self, t: f32) -> f32 {
        -self.sweep / 2.0 + self.sweep * t
    }

    fn at(&self, t: f32, r_frac: f32) -> Point {
        match self.shape {
            TunerShape::Line => Point::new(self.pad + (self.w - self.pad * 2.0) * t, self.line_y()),
            TunerShape::Arc => polar(self.center(), self.radius() * r_frac, self.angle(t)),
        }
    }

    /// The dial position under a local point.
    fn position(&self, p: Point) -> f32 {
        let t = match self.shape {
            TunerShape::Line => (p.x - self.pad) / (self.w - self.pad * 2.0).max(1.0),
            TunerShape::Arc => {
                let c = self.center();
                let a = (p.x - c.x).atan2(c.y - p.y);
                (a + self.sweep / 2.0) / self.sweep.max(1e-3)
            }
        };
        if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// A tuner: a dial of labelled `stations` with a needle on the `selected` one (an index),
/// for moving between rooms the way a radio moves between stations.
///
/// Drag the needle (it snaps to the nearest station when let go), click a station, use the
/// wheel, or the arrow keys, Home and End. The needle glides to a newly selected station,
/// and jumps under reduced motion. It is a slider to assistive technology whose value is the
/// selected station's label.
#[track_caller]
pub fn tuner(
    ui: &Ui,
    label: impl Into<String>,
    stations: Vec<Station>,
    selected: Signal<usize>,
    options: TunerOptions,
) -> El {
    let label = label.into();
    let n = stations.len();
    let spots: Rc<Vec<f32>> = Rc::new(
        stations
            .iter()
            .enumerate()
            .map(|(i, s)| s.at.unwrap_or(if n > 0 { (i as f32 + 0.5) / n as f32 } else { 0.5 }))
            .collect(),
    );
    let stations = Rc::new(stations);
    let theme = ui.theme();
    let ch = theme.spacing.control_height;
    let pad = theme.spacing.unit * 4.0;
    let sweep = options.sweep.to_radians();
    let shape = options.shape;
    let ticks = options.ticks;
    let start = spots.get(selected.get_untracked().unwrap_or(0)).copied().unwrap_or(0.5);
    let needle =
        Rc::new(Needle { shown: Cell::new(start), target: Cell::new(start), dragging: Cell::new(false) });

    let el = ui.el("tuner").focusable(true);
    let el = match shape {
        TunerShape::Line => el.fill_width().height(ch * 2.5),
        TunerShape::Arc => el.width(ch * 7.0).height(ch * 4.0),
    };
    let geo = move |cx_w: f32, cx_h: f32| Geometry { shape, w: cx_w, h: cx_h, pad, sweep };

    let (st, sp, nd) = (stations.clone(), spots.clone(), needle.clone());
    let el = el.paint(move |cx| {
        let g = geo(cx.size.width, cx.size.height);
        let d = cx.theme.derive();
        let sel = selected.get_untracked().unwrap_or(0);
        let small = cx.theme.type_scale.small;
        let body = cx.theme.type_scale.body;
        let w = cx.theme.stroke.max(1.0);
        // The scale: small ticks, then a tall tick at each station.
        for i in 0..=ticks {
            let t = i as f32 / ticks.max(1) as f32;
            match shape {
                TunerShape::Line => {
                    let p = g.at(t, 1.0);
                    let len = if i % 5 == 0 { pad * 0.9 } else { pad * 0.55 };
                    line(cx, Point::new(p.x, p.y - len), p, w, d.muted);
                }
                TunerShape::Arc => {
                    let len = if i % 5 == 0 { 0.12 } else { 0.07 };
                    line(cx, g.at(t, 1.0 - len), g.at(t, 1.0), w, d.muted);
                }
            }
        }
        for (i, (s, &t)) in st.iter().zip(sp.iter()).enumerate() {
            let on = i == sel;
            let color = if on { d.text } else { d.muted };
            let weight = if on { 700.0 } else { 400.0 };
            match shape {
                TunerShape::Line => {
                    let p = g.at(t, 1.0);
                    line(cx, Point::new(p.x, p.y - pad * 1.4), p, w * 2.0, color);
                    let at = Point::new(p.x, p.y + pad * 0.6);
                    let sz = cx.text(
                        &s.label,
                        at,
                        Vec2::new(0.5, 0.0),
                        TextPaint { size: body, weight, color, mono: false },
                    );
                    if let Some(detail) = &s.detail {
                        let at = Point::new(p.x, at.y + sz.height);
                        let _ = cx.text(
                            detail,
                            at,
                            Vec2::new(0.5, 0.0),
                            TextPaint { size: small, weight, color, mono: true },
                        );
                    }
                }
                TunerShape::Arc => {
                    line(cx, g.at(t, 0.8), g.at(t, 1.0), w * 2.0, color);
                    let at = g.at(t, 0.66);
                    let sz = cx.text(
                        &s.label,
                        at,
                        Vec2::new(0.5, 0.5),
                        TextPaint { size: small, weight, color, mono: false },
                    );
                    if let Some(detail) = &s.detail {
                        let at = Point::new(at.x, at.y + sz.height);
                        let _ = cx.text(
                            detail,
                            at,
                            Vec2::new(0.5, 0.5),
                            TextPaint { size: small, weight, color, mono: true },
                        );
                    }
                }
            }
        }
        // The needle, in the theme's needle color (the accent unless the sheet says otherwise).
        let t = nd.shown.get();
        let accent = cx.style.foreground;
        match shape {
            TunerShape::Line => {
                let p = g.at(t, 1.0);
                line(cx, Point::new(p.x, pad * 0.3), Point::new(p.x, p.y + pad * 0.2), w * 3.0, accent);
            }
            TunerShape::Arc => {
                let c = g.center();
                line(cx, c, g.at(t, 1.02), w * 3.0, accent);
                cx.list.push(Item::FillPath {
                    path: Path::circle(c, pad * 0.5),
                    brush: Brush::Solid(accent),
                    even_odd: false,
                });
            }
        }
    });

    let nd = needle.clone();
    let (el, motion) = Motion::attach(el, options.clock, move |dt| {
        if nd.dragging.get() {
            return Moved::default();
        }
        let (shown, target) = (nd.shown.get(), nd.target.get());
        let tau = Duration::from_millis(70);
        let next = smooth(shown, target, dt, tau, tau);
        nd.shown.set(next);
        Moved { changed: next != shown, pending: next != target }
    });

    let nearest = {
        let sp = spots.clone();
        move |t: f32| -> usize {
            sp.iter()
                .enumerate()
                .min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()))
                .map_or(0, |(i, _)| i)
        }
    };
    let pick = move |i: usize| {
        if n > 0 {
            selected.set_if_changed(i.min(n - 1));
        }
    };
    let (nd, me, near, m2, sp2) = (needle.clone(), el.clone(), nearest, motion.clone(), spots.clone());
    let local = move |p: Point| -> Option<f32> {
        let b = me.bounds()?;
        Some(geo(b.width(), b.height()).position(Point::new(p.x - b.min.x, p.y - b.min.y)))
    };
    let me = el.clone();
    let el = el
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down => {
                me.focus();
                if let Some(t) = local(e.position) {
                    nd.dragging.set(true);
                    nd.shown.set(t);
                    me.repaint();
                }
                true
            }
            PointerPhase::Move if nd.dragging.get() => {
                if let Some(t) = local(e.position) {
                    nd.shown.set(t);
                    me.repaint();
                }
                true
            }
            PointerPhase::Up | PointerPhase::Cancel if nd.dragging.replace(false) => {
                let i = near(nd.shown.get());
                pick(i);
                // The needle snaps to the station, even one that was already selected.
                nd.target.set(spot_of(&sp2, i));
                if me.ui().reduced_motion() {
                    nd.shown.set(nd.target.get());
                    me.repaint();
                }
                m2.kick();
                true
            }
            _ => false,
        })
        .on_wheel(move |e| {
            let d = wheel_notch(e);
            if d == 0.0 || n == 0 {
                return false;
            }
            let cur = selected.get_untracked().unwrap_or(0);
            pick(if d > 0.0 { cur + 1 } else { cur.saturating_sub(1) });
            true
        })
        .on_key(move |e| {
            if !e.pressed || n == 0 {
                return false;
            }
            let cur = selected.get_untracked().unwrap_or(0);
            let next = match e.key {
                Key::ArrowRight | Key::ArrowUp => cur + 1,
                Key::ArrowLeft | Key::ArrowDown => cur.saturating_sub(1),
                Key::Home => 0,
                Key::End => n - 1,
                _ => return false,
            };
            pick(next);
            true
        });

    let (me, nd) = (el.clone(), needle);
    el.bind(move || {
        let sel = selected.get();
        let name = stations.get(sel).map(|s| s.label.clone()).unwrap_or_default();
        let _ = me.clone().semantics(
            Semantics::new(Role::Slider)
                .label(label.clone())
                .numeric(sel as f64, 0.0, n.saturating_sub(1) as f64)
                .value(name)
                .action(Action::Increment)
                .action(Action::Decrement),
        );
        let target = spot_of(&spots, sel);
        nd.target.set(target);
        if !nd.dragging.get() && remedy_ui_core::reactive::untrack(|| me.ui().reduced_motion()) {
            nd.shown.set(target);
        }
        me.repaint();
        motion.kick();
    })
}

fn spot_of(spots: &[f32], i: usize) -> f32 {
    spots.get(i).copied().unwrap_or(0.5)
}
