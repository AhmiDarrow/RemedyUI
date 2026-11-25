//! A star rating.

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Point, Rect};
use remedy_ui_core::input::{Key, PointerPhase};
use remedy_ui_engine::paint::{Brush, Item, Path, Stroke, StrokeJoin};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, Ui};
use std::cell::Cell;
use std::rc::Rc;

/// How a [`rating`] behaves.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RatingOptions {
    /// How many stars there are; the highest value.
    pub max: u32,
    /// Whether half stars can be picked (with the pointer, the arrow keys and assistive
    /// technology).
    pub half_steps: bool,
    /// Show the rating without letting people change it. A read-only rating is not a control:
    /// it is an image that says its value.
    pub read_only: bool,
    /// Whether choosing the value that is already set clears the rating to zero.
    pub clearable: bool,
    /// The width of one star. `None` takes six theme units.
    pub star_size: Option<f32>,
    /// What the value is called when read out, such as "stars" or "hearts". The default,
    /// "stars", is read in the window's language; another unit is read as given.
    pub unit: String,
}

impl RatingOptions {
    /// Sets `max`: how many stars there are; the highest value.
    pub fn max(mut self, value: u32) -> Self {
        self.max = value;
        self
    }

    /// Sets `half_steps`: whether half stars can be picked (with the pointer, the arrow keys and assistive technology).
    pub fn half_steps(mut self, value: bool) -> Self {
        self.half_steps = value;
        self
    }

    /// Sets `read_only`: show the rating without letting people change it.
    pub fn read_only(mut self, value: bool) -> Self {
        self.read_only = value;
        self
    }

    /// Sets `clearable`: whether choosing the value that is already set clears the rating to zero.
    pub fn clearable(mut self, value: bool) -> Self {
        self.clearable = value;
        self
    }

    /// Sets `star_size`: the width of one star.
    pub fn star_size(mut self, value: f32) -> Self {
        self.star_size = Some(value);
        self
    }

    /// Sets `unit`: what the value is called when read out, such as "stars" or "hearts".
    pub fn unit(mut self, value: String) -> Self {
        self.unit = value;
        self
    }
}

impl Default for RatingOptions {
    fn default() -> Self {
        RatingOptions {
            max: 5,
            half_steps: false,
            read_only: false,
            clearable: true,
            star_size: None,
            unit: DEFAULT_UNIT.into(),
        }
    }
}

/// The outline of a five-pointed star in a square of side `size` whose top-left is `at`.
pub fn star_path(at: Point, size: f32) -> Path {
    let c = Point::new(at.x + size / 2.0, at.y + size * 0.53);
    let outer = size * 0.5;
    let inner = outer * 0.4;
    let mut p = Path::new();
    for i in 0..10 {
        let r = if i % 2 == 0 { outer } else { inner };
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        let pt = Point::new(c.x + r * a.cos(), c.y + r * a.sin());
        if i == 0 {
            p.move_to(pt);
        } else {
            p.line_to(pt);
        }
    }
    p.close();
    p
}

fn snap(v: f32, max: u32, half: bool) -> f32 {
    let step = if half { 0.5 } else { 1.0 };
    ((v / step).round() * step).clamp(0.0, max as f32)
}

/// The English default [`RatingOptions::unit`], which reads in the window's language.
const DEFAULT_UNIT: &str = "stars";

/// "3 of 5 stars" in the window's language (tracked). The default unit has a message of its
/// own, so the word takes the right plural form; an app's own unit goes in as it is.
fn spoken(ui: &Ui, v: f32, max: u32, unit: &str) -> String {
    // Whole values read whole, the rest with one decimal (2.5).
    let v = f64::from((v * 10.0).round()) / 10.0;
    let args = [("value", v.into()), ("max", max.into())];
    if unit == DEFAULT_UNIT {
        crate::extra::STRINGS.format(ui, "rating-value-stars", &args)
    } else {
        let [a, b] = args;
        crate::extra::STRINGS.format(ui, "rating-value", &[a, b, ("unit", unit.into())])
    }
}

/// A row of stars showing `value` (0 to `max`), which people can change with the pointer
/// (hover previews the value under it), the arrow keys (Up and Down too), Home and End, or by
/// typing a digit. It is a slider to assistive technology, with increment and decrement.
///
/// Stars fill from the inline start, so a right-to-left window fills from the right and the
/// arrow keys follow.
#[track_caller]
pub fn rating(ui: &Ui, label: impl Into<String>, value: Signal<f32>, options: RatingOptions) -> El {
    let label = label.into();
    let max = options.max.max(1);
    let half = options.half_steps;
    let star = options.star_size.unwrap_or(ui.theme().spacing.unit * 6.0).max(8.0);
    let gap = (star * 0.15).round().max(1.0);
    let width = max as f32 * star + (max - 1) as f32 * gap;
    let hover: Signal<Option<f32>> = Signal::new(None);
    let ro = options.read_only;

    let el = ui.el("rating").width(width).height(star).paint(move |cx| {
        let shown = hover.get().unwrap_or_else(|| value.get()).clamp(0.0, max as f32);
        let line = cx.theme.derive().line;
        let fill = cx.style.foreground;
        for i in 0..max {
            // Right to left windows put the first star at the right.
            let slot = if cx.rtl { max - 1 - i } else { i };
            let x = slot as f32 * (star + gap);
            let path = star_path(Point::new(x, 0.0), star);
            cx.list.push(Item::FillPath { path: path.clone(), brush: Brush::Solid(line), even_odd: false });
            let part = (shown - i as f32).clamp(0.0, 1.0);
            if part > 0.0 {
                let clip = if cx.rtl {
                    Rect::new(x + star * (1.0 - part), 0.0, x + star, star)
                } else {
                    Rect::new(x, 0.0, x + star * part, star)
                };
                cx.list.push_clip(clip, remedy_ui_core::CornerRadii::ZERO);
                cx.list.push(Item::FillPath {
                    path: path.clone(),
                    brush: Brush::Solid(fill),
                    even_odd: false,
                });
                cx.list.pop_clip();
            }
            let mut stroke = Stroke::new(1.0);
            stroke.join = StrokeJoin::Round;
            cx.list.push(Item::StrokePath {
                path,
                stroke,
                brush: Brush::Solid(if part > 0.0 { fill } else { cx.theme.derive().muted }),
            });
        }
    });

    let sem_el = el.clone();
    let unit = options.unit.clone();
    let u = ui.clone();
    let l2 = label.clone();
    if ro {
        return el.bind(move || {
            let v = value.get().clamp(0.0, max as f32);
            let _ = sem_el.clone().semantics(
                Semantics::new(Role::Image)
                    .label(l2.clone())
                    .value(spoken(&u, v, max, &unit))
                    .read_only(true),
            );
            sem_el.repaint();
        });
    }

    let step = if half { 0.5 } else { 1.0 };
    let clearable = options.clearable;
    let point = {
        let el = el.clone();
        move |x: f32| -> Option<f32> {
            let b = el.bounds()?;
            let rtl = el.is_rtl();
            let local = if rtl { b.max.x - x } else { x - b.min.x };
            let cell = star + gap;
            let idx = (local / cell).floor().clamp(0.0, (max - 1) as f32);
            let within = ((local - idx * cell) / star).clamp(0.0, 1.0);
            let v = if half && within <= 0.5 { idx + 0.5 } else { idx + 1.0 };
            Some(v.clamp(step, max as f32))
        }
    };
    let pointer_down = Rc::new(Cell::new(false));
    let key_el = el.clone();
    let hover_el = el.clone();
    let click_point = point.clone();
    el.focusable(true)
        .on_pointer(move |e| match e.phase {
            PointerPhase::Down => {
                pointer_down.set(true);
                if let Some(v) = click_point(e.position.x) {
                    let cur = value.get_untracked().unwrap_or(0.0);
                    value.set_if_changed(if clearable && (cur - v).abs() < 1e-6 { 0.0 } else { v });
                }
                true
            }
            PointerPhase::Move if !pointer_down.get() => {
                hover.set_if_changed(click_point(e.position.x));
                false
            }
            // Dragging across the stars scrubs the value.
            PointerPhase::Move => {
                if let Some(v) = click_point(e.position.x) {
                    value.set_if_changed(v);
                }
                true
            }
            PointerPhase::Up | PointerPhase::Cancel => pointer_down.replace(false),
        })
        .on_hover(move |inside| {
            if !inside {
                hover.set_if_changed(None);
            }
        })
        .on_key(move |e| {
            if !e.pressed {
                return false;
            }
            let rtl = key_el.is_rtl();
            let cur = value.get_untracked().unwrap_or(0.0);
            let next = match &e.key {
                Key::ArrowUp => cur + step,
                Key::ArrowDown => cur - step,
                Key::ArrowRight => cur + if rtl { -step } else { step },
                Key::ArrowLeft => cur + if rtl { step } else { -step },
                Key::Home => 0.0,
                Key::End => max as f32,
                Key::Character(c) => match c.parse::<u32>() {
                    Ok(n) if n <= max && c.len() == 1 => n as f32,
                    _ => return false,
                },
                _ => return false,
            };
            value.set_if_changed(snap(next, max, half));
            true
        })
        .bind(move || {
            let v = value.get().clamp(0.0, max as f32);
            let _ = hover_el.clone().semantics(
                Semantics::new(Role::Slider)
                    .label(l2.clone())
                    .numeric(f64::from(v), 0.0, f64::from(max))
                    .value(spoken(&u, v, max, &unit))
                    .action(Action::Increment)
                    .action(Action::Decrement),
            );
            hover_el.repaint();
        })
}
