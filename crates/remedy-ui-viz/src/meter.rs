//! The level meter: a bar with colored zones, a peak marker and a threshold the user sets.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{CornerRadii, Insets, Rect, Size};
use remedy_ui_core::input::{Buttons, Key, PointerPhase};
use remedy_ui_core::layout::{Length, Position};
use remedy_ui_paint::Brush;
use remedy_ui_core::reactive::{Signal, untrack};
use remedy_ui_view::{El, IntoProp, Ui};

use crate::drive::{Clock, Moved, Smoothing, Ticker};
use crate::peak::PeakHold;
use crate::unit_level;

/// How the bar lies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Exhaustive on purpose: horizontal or vertical.
pub enum Orientation {
    /// Left to right. Levels read the same in every text direction.
    #[default]
    Horizontal,
    /// Bottom to top.
    Vertical,
}

/// How loud a level is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Zone {
    /// Fine.
    Normal,
    /// Close to the limit.
    Loud,
    /// At the limit: the signal is being cut.
    Clipping,
}

impl Zone {
    /// A word for screen readers, in English (the meter says it in the window's language).
    pub fn name(self) -> &'static str {
        match self {
            Zone::Normal => "normal",
            Zone::Loud => "loud",
            Zone::Clipping => "clipping",
        }
    }
}

/// Where the zones begin, as fractions of the full scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zones {
    /// Levels from here are loud.
    pub loud: f32,
    /// Levels from here are clipping.
    pub clipping: f32,
}

impl Default for Zones {
    fn default() -> Self {
        Zones { loud: 0.7, clipping: 0.95 }
    }
}

impl Zones {
    /// The zone of a level.
    pub fn zone(&self, level: f32) -> Zone {
        let (loud, clipping) = self.ordered();
        if level >= clipping {
            Zone::Clipping
        } else if level >= loud {
            Zone::Loud
        } else {
            Zone::Normal
        }
    }

    /// The two boundaries, in range and in order.
    fn ordered(&self) -> (f32, f32) {
        let loud = unit_level(self.loud);
        (loud, unit_level(self.clipping).max(loud))
    }
}

/// How the level meter looks and behaves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MeterOptions {
    /// Which way the bar lies.
    pub orientation: Orientation,
    /// Where loud and clipping begin.
    pub zones: Zones,
    /// How the bar rises and falls. Ignored under reduced motion.
    pub smoothing: Smoothing,
    /// How long the peak marker stays at the highest level; `None` shows no peak marker.
    pub peak_hold: Option<Duration>,
    /// How fast the peak marker falls after the hold, as a fraction of the scale per second.
    /// Under reduced motion it drops at once instead.
    pub peak_fall_per_second: f32,
    /// The bar's thickness. `None` takes it from the theme.
    pub thickness: Option<f32>,
    /// The bar's length. `None` fills the width of a horizontal bar; a vertical bar takes a
    /// length from the theme.
    pub length: Option<f32>,
    /// A marker the user sets by dragging it or with the arrow keys (a voice-activation
    /// threshold). `None` shows no marker.
    pub threshold: Option<Signal<f32>>,
    /// What screen readers call the threshold. `None` calls it the meter's label's threshold
    /// ("Microphone threshold"), in the window's language.
    pub threshold_label: Option<String>,
    /// How far an arrow key moves the threshold (a fraction of the scale).
    pub threshold_step: f32,
    /// Where time comes from.
    pub clock: Clock,
}

impl MeterOptions {
    /// Sets `orientation`: which way the bar lies.
    pub fn orientation(mut self, value: Orientation) -> Self {
        self.orientation = value;
        self
    }

    /// Sets `zones`: where loud and clipping begin.
    pub fn zones(mut self, value: Zones) -> Self {
        self.zones = value;
        self
    }

    /// Sets `smoothing`: how the bar rises and falls.
    pub fn smoothing(mut self, value: Smoothing) -> Self {
        self.smoothing = value;
        self
    }

    /// Sets `peak_hold`: how long the peak marker stays at the highest level; `None` shows no peak marker.
    pub fn peak_hold(mut self, value: Duration) -> Self {
        self.peak_hold = Some(value);
        self
    }

    /// Sets `peak_fall_per_second`: how fast the peak marker falls after the hold, as a fraction of the scale per second.
    pub fn peak_fall_per_second(mut self, value: f32) -> Self {
        self.peak_fall_per_second = value;
        self
    }

    /// Sets `thickness`: the bar's thickness.
    pub fn thickness(mut self, value: f32) -> Self {
        self.thickness = Some(value);
        self
    }

    /// Sets `length`: the bar's length.
    pub fn length(mut self, value: f32) -> Self {
        self.length = Some(value);
        self
    }

    /// Sets `threshold`: a marker the user sets by dragging it or with the arrow keys (a voice-activation threshold).
    pub fn threshold(mut self, value: Signal<f32>) -> Self {
        self.threshold = Some(value);
        self
    }

    /// Sets `threshold_label`: what screen readers call the threshold.
    pub fn threshold_label(mut self, value: String) -> Self {
        self.threshold_label = Some(value);
        self
    }

    /// Sets `threshold_step`: how far an arrow key moves the threshold (a fraction of the scale).
    pub fn threshold_step(mut self, value: f32) -> Self {
        self.threshold_step = value;
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

impl Default for MeterOptions {
    fn default() -> Self {
        MeterOptions {
            orientation: Orientation::Horizontal,
            zones: Zones::default(),
            smoothing: Smoothing::meter(),
            peak_hold: Some(Duration::from_millis(1200)),
            peak_fall_per_second: 0.7,
            thickness: None,
            length: None,
            threshold: None,
            threshold_label: None,
            threshold_step: 0.01,
            clock: Clock::default(),
        }
    }
}

/// The part of a bar from fraction `a` to `b` of the scale, across its whole thickness.
fn span(o: Orientation, size: Size, a: f32, b: f32) -> Rect {
    match o {
        Orientation::Horizontal => Rect::new(size.width * a, 0.0, size.width * b, size.height),
        Orientation::Vertical => Rect::new(0.0, size.height * (1.0 - b), size.width, size.height * (1.0 - a)),
    }
}

/// The part of a bar `half` pixels either side of fraction `at`, kept inside the bar.
fn mark(o: Orientation, size: Size, at: f32, half: f32) -> Rect {
    let length = match o {
        Orientation::Horizontal => size.width,
        Orientation::Vertical => size.height,
    };
    let centre = (length * at).clamp(half, (length - half).max(half));
    let (a, b) = ((centre - half) / length.max(1.0), (centre + half) / length.max(1.0));
    span(o, size, a, b)
}

/// A threshold rounded to a step.
fn snap(v: f32, step: f32) -> f32 {
    let v = unit_level(v);
    if step > 0.0 { unit_level((v / step).round() * step) } else { v }
}

struct State {
    shown: f32,
    raw: f32,
    peak: PeakHold,
}

/// A level meter for `level` (0 to 1), with a peak marker and zones colored from the theme
/// (`ok`, `warn`, `danger`).
///
/// With a threshold in the options the user can drag the marker, or use the arrow keys (one
/// step), Page Up and Down (a tenth) and Home and End. Screen readers hear the level as a
/// progress value with its zone, and the threshold as a slider.
#[track_caller]
pub fn level_meter(ui: &Ui, label: impl IntoProp<String>, level: Signal<f32>, options: MeterOptions) -> El {
    let label = label.into_prop();
    let unit = ui.theme().spacing.unit;
    let thickness = options.thickness.unwrap_or(unit * 2.5).max(2.0);
    let o = options.orientation;
    let zones = options.zones;
    let hold = options.peak_hold;
    let state = Rc::new(RefCell::new(State {
        shown: 0.0,
        raw: 0.0,
        peak: PeakHold::new(hold.unwrap_or(Duration::ZERO), options.peak_fall_per_second),
    }));

    let paint_state = state.clone();
    let mut root = ui.el("level_meter");
    root = match o {
        Orientation::Horizontal => {
            let r = root.height(thickness);
            match options.length {
                Some(w) => r.width(w),
                None => r.fill_width(),
            }
        }
        Orientation::Vertical => root.width(thickness).height(options.length.unwrap_or(unit * 30.0)),
    };
    let root = root
        .semantics(Semantics::new(Role::Progress).label(untrack(|| label.get())).numeric(0.0, 0.0, 1.0))
        .paint(move |cx| {
            let (shown, peak) = paint_state.try_borrow().map_or((0.0, 0.0), |s| (s.shown, s.peak.peak()));
            let whole = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
            let cross = match o {
                Orientation::Horizontal => cx.size.height,
                Orientation::Vertical => cx.size.width,
            };
            let c = &cx.theme.colors;
            let (loud, clipping) = zones.ordered();
            let bands = [(0.0, loud, c.ok), (loud, clipping, c.warn), (clipping, 1.0, c.danger)];
            cx.list.push_clip(whole, CornerRadii::all(cx.style.radius.min(cross / 2.0)));
            for (from, to, color) in bands {
                if to > from {
                    cx.list.fill_rect(span(o, cx.size, from, to), Brush::Solid(color.with_alpha(0.22)));
                    if shown > from {
                        cx.list.fill_rect(span(o, cx.size, from, shown.min(to)), Brush::Solid(color));
                    }
                }
            }
            cx.list.pop_clip();
            if hold.is_some() && peak > 0.0 {
                let color = match zones.zone(peak) {
                    Zone::Normal => c.ok,
                    Zone::Loud => c.warn,
                    Zone::Clipping => c.danger,
                };
                cx.list.fill_rect(mark(o, cx.size, peak, 1.0), Brush::Solid(color));
            }
        });

    let step_state = state.clone();
    let clock = options.clock.clone();
    let fall = options.peak_fall_per_second;
    let step_clock = clock.clone();
    let step_el = root.clone();
    let smoothing = options.smoothing;
    let ticker = Ticker::new(&root, clock.clone(), move |dt| {
        let reduced = step_el.ui().reduced_motion();
        let Ok(mut s) = step_state.try_borrow_mut() else { return Moved::default() };
        let raw = s.raw;
        let next = if reduced { raw } else { smoothing.step(s.shown, raw, dt) };
        let old_peak = s.peak.peak();
        s.peak.set_fall_per_second(if reduced { 0.0 } else { fall });
        let peak = if hold.is_some() { s.peak.feed(raw, step_clock.now()) } else { 0.0 };
        let changed = (next - s.shown).abs() > f32::EPSILON || (peak - old_peak).abs() > f32::EPSILON;
        s.shown = next;
        let pending = (next - raw).abs() > f32::EPSILON || (hold.is_some() && s.peak.is_moving(raw));
        Moved { changed, pending }
    });

    // Screen readers hear the level in twentieths and when the zone changes, not 50 times a
    // second.
    let heard = Rc::new(RefCell::new((-1i32, Zone::Normal, String::new())));
    let sem_el = root.clone();
    let sem_label = label.clone();
    let u = ui.clone();
    let root = root.bind(move || {
        let v = unit_level(level.get());
        if let Ok(mut s) = state.try_borrow_mut() {
            s.raw = v;
            if hold.is_some() {
                // Catch short peaks between two timer steps.
                s.peak.feed(v, clock.now());
            }
        }
        let tick = (v * 20.0).round() as i32;
        let zone = zones.zone(v);
        // Said again when the label or the window's language changes too.
        let name = sem_label.get();
        let now = (tick, zone, format!("{}\n{name}", u.locale()));
        if heard.replace(now.clone()) != now {
            let id = match zone {
                Zone::Normal => "meter-value-normal",
                Zone::Loud => "meter-value-loud",
                Zone::Clipping => "meter-value-clipping",
            };
            let said = crate::STRINGS.format(&u, id, &[("percent", (tick * 5).into())]);
            let _ = sem_el.clone().semantics(
                Semantics::new(Role::Progress)
                    .label(name)
                    .numeric(f64::from(tick) / 20.0, 0.0, 1.0)
                    .value(said),
            );
        }
        ticker.kick();
    });

    match options.threshold {
        Some(threshold) => {
            let u = ui.clone();
            let given = options.threshold_label;
            let name = move || {
                given.clone().unwrap_or_else(|| {
                    crate::STRINGS.format(&u, "meter-threshold", &[("label", label.get().into())])
                })
            };
            root.child(threshold_handle(ui, o, name, threshold, options.threshold_step))
        }
        None => root,
    }
}

/// The element over the bar that holds the threshold: it draws the marker and takes the
/// pointer and keyboard.
fn threshold_handle(
    ui: &Ui,
    o: Orientation,
    name: impl Fn() -> String + 'static,
    threshold: Signal<f32>,
    step: f32,
) -> El {
    let u = ui.clone();
    let handle = ui.el("meter_threshold").focusable(true).layout(|l| {
        l.position = Position::Absolute;
        l.inset = Insets::ZERO;
        l.width = Length::Percent(1.0);
        l.height = Length::Percent(1.0);
    });
    let pointer_el = handle.clone();
    let sem_el = handle.clone();
    let set_from = move |x: f32, y: f32| {
        let Some(b) = pointer_el.bounds() else { return };
        let t = match o {
            Orientation::Horizontal => (x - b.min.x) / b.width().max(1.0),
            Orientation::Vertical => 1.0 - (y - b.min.y) / b.height().max(1.0),
        };
        threshold.set_if_changed(snap(t, step));
    };
    handle
        .paint(move |cx| {
            let at = unit_level(threshold.get_untracked().unwrap_or(0.0));
            // A dark edge under a light line shows on every zone color.
            cx.list.fill_rect(mark(o, cx.size, at, 2.0), Brush::Solid(cx.theme.colors.bg.with_alpha(0.7)));
            cx.list.fill_rect(mark(o, cx.size, at, 1.0), Brush::Solid(cx.style.foreground));
        })
        .on_pointer(move |e| {
            let drag = e.phase == PointerPhase::Down
                || (e.phase == PointerPhase::Move && e.buttons.contains(Buttons::PRIMARY));
            if drag {
                set_from(e.position.x, e.position.y);
            }
            drag
        })
        .on_key(move |e| {
            if !e.pressed {
                return false;
            }
            let cur = unit_level(threshold.get_untracked().unwrap_or(0.0));
            let next = match e.key {
                Key::ArrowRight | Key::ArrowUp => cur + step,
                Key::ArrowLeft | Key::ArrowDown => cur - step,
                Key::PageUp => cur + 0.1,
                Key::PageDown => cur - 0.1,
                Key::Home => 0.0,
                Key::End => 1.0,
                _ => return false,
            };
            threshold.set_if_changed(snap(next, step));
            true
        })
        .bind(move || {
            let v = unit_level(threshold.get());
            let percent =
                crate::STRINGS.format(&u, "meter-percent", &[("percent", (v * 100.0).round().into())]);
            let _ = sem_el.clone().semantics(
                Semantics::new(Role::Slider)
                    .label(name())
                    .numeric(f64::from(v), 0.0, 1.0)
                    .value(percent)
                    .action(Action::Increment)
                    .action(Action::Decrement),
            );
            sem_el.repaint();
        })
}
