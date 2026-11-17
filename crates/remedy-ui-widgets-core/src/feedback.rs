//! Feedback: progress, spinners, lamps, badges, toasts, empty states and skeletons.

use std::time::Duration;

use remedy_ui_a11y::{Live, Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect};
use remedy_ui_layout::{Align, Length, Position};
use remedy_ui_paint::{Brush, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};

/// Frames per second for engine-drawn indeterminate animations.
const ANIMATION_STEP: Duration = Duration::from_millis(33);

/// Advances `phase` while `el` is alive and motion is allowed, repainting it each step.
fn animate(el: &El, phase: Signal<f32>) {
    let ui = el.ui().clone();
    if ui.reduced_motion() {
        return;
    }
    let el = el.clone();
    let tick = std::rc::Rc::new(std::cell::RefCell::new(None::<Box<dyn Fn()>>));
    let t2 = tick.clone();
    let ui2 = ui.clone();
    *tick.borrow_mut() = Some(Box::new(move || {
        if !ui2.tree().contains(el.id()) || ui2.reduced_motion() {
            return;
        }
        phase.update(|p| *p = (*p + 0.04) % 1.0);
        el.repaint();
        let t3 = t2.clone();
        ui2.after(ANIMATION_STEP, move || {
            if let Some(f) = t3.borrow().as_ref() {
                f();
            }
        });
    }));
    if let Some(f) = tick.borrow().as_ref() {
        f();
    }
}

/// A progress bar: `Some(fraction)` (0..1) when the amount is known, `None` for
/// indeterminate work.
#[track_caller]
pub fn progress(ui: &Ui, label: impl Into<String>, value: Signal<Option<f32>>) -> El {
    let label = label.into();
    let h = ui.theme().spacing.unit * 1.5;
    let phase = Signal::new(0.0f32);
    let bar = ui.el("progress").height(h).fill_width().paint(move |cx| {
        let w = cx.size.width;
        let (x0, x1) = match value.get_untracked().ok().flatten() {
            Some(f) => (0.0, w * f.clamp(0.0, 1.0)),
            None => {
                let p = phase.get_untracked().unwrap_or(0.0);
                let x = (p * 1.4 - 0.4) * w;
                (x.max(0.0), (x + 0.4 * w).min(w))
            }
        };
        if x1 > x0 {
            let r = cx.style.radius.min(cx.size.height / 2.0);
            cx.list.fill_rounded(
                Rect::new(x0, 0.0, x1, cx.size.height),
                CornerRadii::all(r),
                Brush::Solid(cx.style.foreground),
            );
        }
    });
    let b2 = bar.clone();
    let animating = std::rc::Rc::new(std::cell::Cell::new(false));
    bar.bind(move || {
        let v = value.get();
        let mut s = Semantics::new(Role::Progress).label(label.clone());
        if let Some(f) = v {
            s = s.numeric(f64::from(f), 0.0, 1.0);
        } else if !animating.replace(true) {
            animate(&b2, phase);
        }
        let _ = b2.clone().semantics(s);
        b2.repaint();
    })
}

/// A spinning ring for work of unknown length (still under reduced motion).
#[track_caller]
pub fn spinner(ui: &Ui, label: impl Into<String>) -> El {
    let size = ui.theme().type_scale.body + 6.0;
    let phase = Signal::new(0.0f32);
    let el = ui
        .el("spinner")
        .width(size)
        .height(size)
        .semantics(Semantics::new(Role::Progress).label(label.into()))
        .paint(move |cx| {
            let p = phase.get_untracked().unwrap_or(0.0) * std::f32::consts::TAU;
            let c = Point::new(cx.size.width / 2.0, cx.size.height / 2.0);
            let r = cx.size.width.min(cx.size.height) / 2.0 - 2.0;
            let mut path = Path::new();
            let steps = 24;
            for i in 0..=steps {
                let a = p + (i as f32 / steps as f32) * std::f32::consts::PI * 1.5;
                let pt = Point::new(c.x + r * a.cos(), c.y + r * a.sin());
                if i == 0 {
                    path.move_to(pt);
                } else {
                    path.line_to(pt);
                }
            }
            let stroke =
                Stroke { width: 2.0, cap: StrokeCap::Round, join: StrokeJoin::Round, dashes: Vec::new() };
            cx.list.push(remedy_ui_paint::Item::StrokePath {
                path,
                stroke,
                brush: Brush::Solid(cx.style.foreground),
            });
        });
    animate(&el, phase);
    el
}

/// A status light: on (the theme's ok color) or off.
#[track_caller]
pub fn lamp(ui: &Ui, label: impl Into<String>, on: Signal<bool>) -> El {
    let label = label.into();
    let size = ui.theme().spacing.unit * 2.5;
    let l = ui.el("lamp").width(size).height(size).paint(move |cx| {
        let c = &cx.theme.colors;
        let color = if on.get_untracked().unwrap_or(false) { c.ok } else { cx.theme.derive().line };
        cx.list.fill_rounded(
            Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
            CornerRadii::all(cx.size.width / 2.0),
            Brush::Solid(color),
        );
    });
    let l2 = l.clone();
    l.bind(move || {
        let v = on.get();
        let _ = l2.clone().semantics(Semantics::new(Role::Status).label(label.clone()).value(if v {
            "on"
        } else {
            "off"
        }));
        l2.repaint();
    })
}

/// A small count or tag.
#[track_caller]
pub fn badge(ui: &Ui, text: impl Into<String>) -> El {
    ui.el("badge").text(remedy_ui_view::RichText::plain(text.into())).nowrap()
}

/// A short message at the bottom of the window that goes away by itself after `duration`
/// (or when clicked). Screen readers announce it.
#[track_caller]
pub fn toast(ui: &Ui, message: impl Into<String>, duration: Duration) -> El {
    let message = message.into();
    let u = ui.theme().spacing.unit;
    let t = ui
        .el("toast")
        .row()
        .align(Align::Center)
        .semantics(Semantics::new(Role::Status).label(message.clone()).live(Live::Polite))
        .child(crate::text(ui, message));
    // A click-through layer the size of the window holds the toast at its bottom.
    let host = ui
        .el("toast_host")
        .column()
        .justify(remedy_ui_layout::Justify::End)
        .align(Align::Center)
        .padding(remedy_ui_core::Insets::all(u * 4.0))
        .layout(|l| {
            l.position = Position::Absolute;
            l.inset = remedy_ui_core::Insets::ZERO;
            l.width = Length::Percent(1.0);
            l.height = Length::Percent(1.0);
        })
        .pass_through()
        .child(t.clone());
    let host = ui.overlay(host);
    let h2 = host.clone();
    let _ = t.clone().on_click(move || h2.remove());
    let h3 = host.clone();
    ui.after(duration, move || h3.remove());
    t
}

/// What an empty list or a first run shows: a title and a hint.
#[track_caller]
pub fn empty_state(ui: &Ui, title: impl Into<String>, hint: impl Into<String>) -> El {
    let gap = ui.theme().spacing.unit * 2.0;
    ui.el("empty_state")
        .column()
        .align(Align::Center)
        .gap(gap)
        .child(crate::heading(ui, 3, title.into()))
        .child(crate::label(ui, hint.into(), "muted"))
}

/// A placeholder block shown while content loads.
#[track_caller]
pub fn skeleton(ui: &Ui, width: f32, height: f32) -> El {
    let fill = ui.theme().derive().hover;
    let r = ui.theme().radii.small;
    ui.el("skeleton")
        .width(width)
        .height(height)
        .semantics(Semantics::new(Role::Progress).label("Loading"))
        .paint(move |cx| {
            cx.list.fill_rounded(
                Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
                CornerRadii::all(r),
                Brush::Solid(fill),
            )
        })
}
