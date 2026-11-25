//! A mixing-desk fader, with an optional level meter beside it.

use std::cell::Cell;
use std::rc::Rc;

use crate::viz::{MeterOptions, Orientation, level_meter};
use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::input::PointerPhase;
use remedy_ui_core::layout::Align;
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::{CornerRadii, Point, Rect, Vec2};
use remedy_ui_engine::paint::Brush;
use remedy_ui_engine::view::{El, Ui};

use crate::stage::common::{number_text, snap};
use crate::stage::dial::{ValueText, step_key};

/// How a [`fader`] behaves and looks.
#[derive(Clone)]
#[non_exhaustive]
pub struct FaderOptions {
    /// Which way it slides: vertical (up is more), as on a mixing desk, or horizontal.
    pub orientation: Orientation,
    /// The lowest value.
    pub min: f64,
    /// The highest value.
    pub max: f64,
    /// Values snap to multiples of this from `min`; zero for none (the keyboard then moves a
    /// hundredth of the range).
    pub step: f64,
    /// How many tick marks the scale has, ends included (zero or one for none).
    pub ticks: u32,
    /// The travel's length. `None` takes five times the theme's control height.
    pub length: Option<f32>,
    /// A level shown on a meter beside the fader (0 to 1), such as what this channel plays.
    pub meter: Option<Signal<f32>>,
    /// How that meter looks; its orientation follows the fader's.
    pub meter_options: MeterOptions,
    /// What assistive technology reads for a value. `None` reads the number.
    pub value_text: Option<ValueText>,
}

impl std::fmt::Debug for FaderOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FaderOptions")
            .field("orientation", &self.orientation)
            .field("min", &self.min)
            .field("max", &self.max)
            .field("step", &self.step)
            .field("ticks", &self.ticks)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

impl Default for FaderOptions {
    fn default() -> Self {
        FaderOptions {
            orientation: Orientation::Vertical,
            min: 0.0,
            max: 1.0,
            step: 0.0,
            ticks: 11,
            length: None,
            meter: None,
            meter_options: MeterOptions::default(),
            value_text: None,
        }
    }
}

impl FaderOptions {
    /// Sets `orientation`: which way it slides.
    pub fn orientation(mut self, value: Orientation) -> Self {
        self.orientation = value;
        self
    }

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

    /// Sets `ticks`: how many tick marks the scale has, ends included.
    pub fn ticks(mut self, value: u32) -> Self {
        self.ticks = value.min(200);
        self
    }

    /// Sets `length`: the travel's length.
    pub fn length(mut self, value: f32) -> Self {
        self.length = Some(value.max(16.0));
        self
    }

    /// Sets `meter`: a level (0 to 1) shown on a meter beside the fader.
    pub fn meter(mut self, level: Signal<f32>) -> Self {
        self.meter = Some(level);
        self
    }

    /// Sets `meter_options`: how the meter looks.
    pub fn meter_options(mut self, value: MeterOptions) -> Self {
        self.meter_options = value;
        self
    }

    /// Sets `value_text`: what assistive technology reads for a value ("-6 dB").
    pub fn value_text(mut self, f: impl Fn(f64) -> String + 'static) -> Self {
        self.value_text = Some(Rc::new(f));
        self
    }
}

/// A fader for `value` between `min` and `max`: a cap that slides in a groove along a scale
/// of ticks, as on a mixing desk, with a level meter beside it when
/// [`FaderOptions::meter`] is set.
///
/// Drag the cap (it does not jump under the pointer; pressing the groove elsewhere moves it
/// there first), use the wheel, or the arrow keys (Page Up and Page Down for big steps, Home
/// and End for the ends). It is a slider to assistive technology, with increment and
/// decrement. With a meter, the result is a row of the fader and the meter (a meter to
/// assistive technology of its own, named "`label` level").
#[track_caller]
pub fn fader(ui: &Ui, label: impl Into<String>, value: Signal<f64>, options: FaderOptions) -> El {
    let label = label.into();
    let theme = ui.theme();
    let u = theme.spacing.unit;
    let (min, max, step) = (options.min, options.max, options.step);
    let span = (max - min).abs().max(f64::MIN_POSITIVE);
    let vertical = options.orientation == Orientation::Vertical;
    let length = options.length.unwrap_or(theme.spacing.control_height * 5.0);
    let thick = u * 10.0;
    let cap = (u * 4.0).max(12.0);
    let ticks = options.ticks;
    let text = options.value_text.clone().unwrap_or_else(|| Rc::new(move |v| number_text(v, step)));

    // Along the travel, 0 at the low end; in local coordinates of a box `size`.
    let travel = move |size: remedy_ui_core::Size| {
        let len = if vertical { size.height } else { size.width };
        (cap / 2.0, (len - cap / 2.0).max(cap / 2.0))
    };
    let pos_of = move |t: f32, size: remedy_ui_core::Size| {
        let (a, b) = travel(size);
        if vertical { b - (b - a) * t } else { a + (b - a) * t }
    };

    let el = ui.el("fader").focusable(true);
    let el = if vertical { el.width(thick).height(length) } else { el.width(length).height(thick) };
    let el = el.paint(move |cx| {
        let v = value.get_untracked().unwrap_or(min);
        let t = (((v - min) / span).clamp(0.0, 1.0)) as f32;
        let d = cx.theme.derive();
        let sz = cx.size;
        let mid = if vertical { sz.width / 2.0 } else { sz.height / 2.0 };
        let (a, b) = travel(sz);
        let groove = (u * 1.0).max(2.0);
        let w = cx.theme.stroke.max(1.0);
        let rect = |along0: f32, along1: f32, across0: f32, across1: f32| {
            if vertical {
                Rect::new(across0, along0.min(along1), across1, along0.max(along1))
            } else {
                Rect::new(along0.min(along1), across0, along0.max(along1), across1)
            }
        };
        // Ticks on both sides of the groove; the ends and the middle are longer.
        if ticks > 1 {
            for i in 0..ticks {
                let f = i as f32 / (ticks - 1) as f32;
                let at = a + (b - a) * f;
                let long = i == 0 || i == ticks - 1 || i * 2 == ticks - 1;
                let len = if long { u * 2.5 } else { u * 1.5 };
                let off = groove + u;
                for side in [-1.0f32, 1.0] {
                    let (c0, c1) = (mid + side * off, mid + side * (off + len));
                    cx.list.fill_rect(
                        rect(at - w / 2.0, at + w / 2.0, c0.min(c1), c0.max(c1)),
                        Brush::Solid(d.muted),
                    );
                }
            }
        }
        cx.list.fill_rounded(
            rect(a, b, mid - groove / 2.0, mid + groove / 2.0),
            CornerRadii::all(groove / 2.0),
            Brush::Solid(cx.theme.colors.bg),
        );
        cx.list.border(
            rect(a, b, mid - groove / 2.0, mid + groove / 2.0),
            CornerRadii::all(groove / 2.0),
            w,
            d.line,
        );
        // The filled part of the groove, in the fader's color (the accent by default).
        let at = pos_of(t, sz);
        let low = if vertical { b } else { a };
        cx.list.fill_rounded(
            rect(low, at, mid - groove / 2.0, mid + groove / 2.0),
            CornerRadii::all(groove / 2.0),
            Brush::Solid(cx.style.foreground),
        );
        // The cap: a raised block with a grip line.
        let half_across = (if vertical { sz.width } else { sz.height }) * 0.42;
        let cap_rect = rect(at - cap / 2.0, at + cap / 2.0, mid - half_across, mid + half_across);
        let radius = CornerRadii::all(cx.theme.radii.small.min(cap / 3.0));
        cx.list.push(remedy_ui_engine::paint::Item::Shadow {
            rect: cap_rect,
            radii: radius,
            offset: Vec2::new(0.0, u * 0.5),
            blur: u,
            spread: 0.0,
            color: remedy_ui_core::Color::BLACK.with_alpha(0.35),
        });
        cx.list.fill_rounded(cap_rect, radius, Brush::Solid(cx.theme.colors.card));
        cx.list.border(cap_rect, radius, w, d.line);
        cx.list.fill_rect(
            rect(at - w, at + w, mid - half_across + u, mid + half_across - u),
            Brush::Solid(d.text),
        );
    });

    let set = move |v: f64| value.set_if_changed(snap(v, min, max, step));
    // While dragging: how far from the cap's center the pointer took hold.
    let grab: Rc<Cell<Option<f32>>> = Rc::new(Cell::new(None));
    let me = el.clone();
    let value_at = move |p: Point, hold: f32| -> Option<f64> {
        let b = me.bounds()?;
        let (a, bb) = travel(b.size());
        let along = if vertical { p.y - b.min.y } else { p.x - b.min.x } - hold;
        let t = if vertical { (bb - along) / (bb - a).max(1.0) } else { (along - a) / (bb - a).max(1.0) };
        Some(min + f64::from(t.clamp(0.0, 1.0)) * span)
    };
    let me = el.clone();
    let cap_at = move || -> Option<f32> {
        let b = me.bounds()?;
        let v = value.get_untracked().unwrap_or(min);
        let t = (((v - min) / span).clamp(0.0, 1.0)) as f32;
        let p = pos_of(t, b.size());
        Some(if vertical { b.min.y + p } else { b.min.x + p })
    };
    let (g, me) = (grab.clone(), el.clone());
    let el = el
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down => {
                me.focus();
                let along = if vertical { e.position.y } else { e.position.x };
                let hold = match cap_at() {
                    Some(c) if (along - c).abs() <= cap / 2.0 => along - c,
                    _ => {
                        if let Some(v) = value_at(e.position, 0.0) {
                            set(v);
                        }
                        0.0
                    }
                };
                g.set(Some(hold));
                true
            }
            PointerPhase::Move => match g.get() {
                Some(hold) => {
                    if let Some(v) = value_at(e.position, hold) {
                        set(v);
                    }
                    true
                }
                None => false,
            },
            PointerPhase::Up | PointerPhase::Cancel => g.replace(None).is_some(),
        })
        .on_wheel(move |e| {
            let d = if e.delta.y != 0.0 { -e.delta.y } else { e.delta.x };
            if d == 0.0 {
                return false;
            }
            let small = if step > 0.0 { step } else { span / 100.0 };
            set(value.get_untracked().unwrap_or(min) + small * f64::from(d.signum()));
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
    let l2 = label.clone();
    let el = el.bind(move || {
        let v = value.get();
        let _ = me.clone().semantics(
            Semantics::new(Role::Slider)
                .label(l2.clone())
                .numeric(v, min, max)
                .value(text(v))
                .action(Action::Increment)
                .action(Action::Decrement),
        );
        me.repaint();
    });

    let Some(level) = options.meter else { return el };
    let meter = level_meter(
        ui,
        {
            let u = ui.clone();
            move || crate::stage::STRINGS.format(&u, "fader-level", &[("label", label.as_str().into())])
        },
        level,
        options.meter_options.orientation(options.orientation).length(length),
    );
    let wrap = ui.el("fader_strip").gap(u * 2.0).align(Align::Center);
    let wrap = if vertical { wrap.row() } else { wrap.column() };
    wrap.child(el).child(meter)
}
