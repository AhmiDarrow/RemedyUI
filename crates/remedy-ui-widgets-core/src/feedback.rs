//! Feedback: progress, spinners, lamps, badges, toasts, empty states and skeletons.

use std::time::Duration;

use remedy_ui_a11y::{Action, Live, Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect};
use remedy_ui_layout::{Align, Length, Position};
use remedy_ui_paint::{Brush, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, IntoProp, PaintCx, Ui};

/// How long one turn of an engine-drawn indeterminate animation takes.
const LOOP: Duration = Duration::from_millis(825);

/// Where a looping animation is (0 to 1) at the frame's time, asking for the next frame
/// while motion is allowed: it runs on the frame clock, with no timer, and stops asking
/// (a still frame) under reduced motion.
fn phase(cx: &mut PaintCx<'_>) -> f32 {
    if cx.reduced_motion {
        return 0.0;
    }
    cx.request_next_frame();
    (cx.time.as_secs_f32() / LOOP.as_secs_f32()).fract()
}

/// A progress bar: `Some(fraction)` (0..1) when the amount is known, `None` for
/// indeterminate work.
#[track_caller]
pub fn progress(ui: &Ui, label: impl IntoProp<String>, value: Signal<Option<f32>>) -> El {
    let label = label.into_prop();
    let h = ui.theme().spacing.unit * 1.5;
    let bar = ui.el("progress").height(h).fill_width().paint(move |cx| {
        let w = cx.size.width;
        let (x0, x1) = match value.get() {
            Some(f) => (0.0, w * f.clamp(0.0, 1.0)),
            None => {
                let p = phase(cx);
                let x = (p * 1.4 - 0.4) * w;
                (x.max(0.0), (x + 0.4 * w).min(w))
            }
        };
        // Right to left, progress grows (and work sweeps) from the right.
        let (x0, x1) = if cx.rtl { (w - x1, w - x0) } else { (x0, x1) };
        if x1 > x0 {
            let r = cx.style.radius.min(cx.size.height / 2.0);
            cx.list.fill_rounded(
                Rect::new(x0, 0.0, x1, cx.size.height),
                CornerRadii::all(r),
                Brush::Solid(cx.style.foreground),
            );
        }
    });
    bar.semantics(move || {
        let mut s = Semantics::new(Role::Progress).label(label.get());
        if let Some(f) = value.get() {
            s = s.numeric(f64::from(f), 0.0, 1.0);
        }
        s
    })
}

/// A spinning ring for work of unknown length (still under reduced motion).
#[track_caller]
pub fn spinner(ui: &Ui, label: impl IntoProp<String>) -> El {
    let size = ui.theme().type_scale.body + 6.0;
    ui.el("spinner")
        .width(size)
        .height(size)
        .semantics(label.into_prop().map(|l| Semantics::new(Role::Progress).label(l)))
        .paint(move |cx| {
            let p = phase(cx) * std::f32::consts::TAU;
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
        })
}

/// A status light: on or off. On, it shows its text color from the style sheet: the
/// theme's ok color, or with a variant `danger`, `warn` or `accent` that color (a problem
/// band's light is red, not "all is well" green).
#[track_caller]
pub fn lamp(ui: &Ui, label: impl IntoProp<String>, on: Signal<bool>) -> El {
    let label = label.into_prop();
    let size = ui.theme().spacing.unit * 2.5;
    let l = ui.el("lamp").width(size).height(size).paint(move |cx| {
        let color = if on.get() { cx.style.foreground } else { cx.theme.derive().line };
        cx.list.fill_rounded(
            Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
            CornerRadii::all(cx.size.width / 2.0),
            Brush::Solid(color),
        );
    });
    let u = ui.clone();
    l.semantics(move || {
        Semantics::new(Role::Status)
            .label(label.get())
            .value(crate::tr(&u, if on.get() { "lamp-on" } else { "lamp-off" }))
    })
}

/// A small count or tag.
#[track_caller]
pub fn badge(ui: &Ui, text: impl IntoProp<String>) -> El {
    ui.el("badge").text(text.into_prop().map(remedy_ui_view::RichText::plain)).nowrap()
}

/// A short message at the bottom of the window that goes away by itself after `duration`
/// (or when clicked). Screen readers announce it, and assistive technology can dismiss it
/// with the same click action a pointer uses.
#[track_caller]
pub fn toast(ui: &Ui, message: impl Into<String>, duration: Duration) -> El {
    toast_at(ui, message, duration, Align::Center)
}

/// The layer holding the toasts showing now.
fn toast_host(ui: &Ui) -> Option<El> {
    let [_, overlays, _] = ui.layers();
    overlays.child_elements().into_iter().find(|c| c.info().is_some_and(|i| i.widget == "toast_host"))
}

/// Puts the toasts showing over an overlay that was just opened (a dialog's scrim would dim
/// them).
pub(crate) fn raise_toasts(ui: &Ui) {
    if let Some(host) = toast_host(ui) {
        let [_, overlays, _] = ui.layers();
        overlays.insert_child(usize::MAX, &host);
    }
}

/// A [`toast`] at the bottom's start, center or end (`align`). Toasts showing at the same
/// time stack, the newest lowest, above whatever else is open; the stack sits where the
/// newest one asked.
#[track_caller]
pub fn toast_at(ui: &Ui, message: impl Into<String>, duration: Duration, align: Align) -> El {
    let message = message.into();
    let u = ui.theme().spacing.unit;
    let t = ui
        .el("toast")
        .row()
        .align(Align::Center)
        .semantics(
            Semantics::new(Role::Status).label(message.clone()).live(Live::Polite).action(Action::Click),
        )
        .child(crate::text(ui, message));
    // One click-through layer the size of the window holds every toast at its bottom.
    let [_, overlays, _] = ui.layers();
    let host = toast_host(ui).unwrap_or_else(|| {
        ui.el("toast_host")
            .column()
            .justify(remedy_ui_layout::Justify::End)
            .gap(u * 2.0)
            .padding(remedy_ui_core::Insets::all(u * 4.0))
            .layout(|l| {
                l.position = Position::Absolute;
                l.inset = remedy_ui_core::Insets::ZERO;
                l.width = Length::Percent(1.0);
                l.height = Length::Percent(1.0);
            })
            .pass_through()
    });
    let host = host.align(align).child(t.clone());
    // Last among the overlays: over a dialog opened since the first toast.
    overlays.insert_child(usize::MAX, &host);
    let gone = {
        let (host, t) = (host, t.clone());
        std::rc::Rc::new(move || {
            t.remove();
            if host.child_elements().is_empty() {
                host.remove();
            }
        })
    };
    let g2 = gone.clone();
    let _ = t.clone().on_click(move || g2());
    ui.after(duration, move || gone());
    t
}

/// What an empty list or a first run shows: a title and a hint.
#[track_caller]
pub fn empty_state(ui: &Ui, title: impl IntoProp<String>, hint: impl IntoProp<String>) -> El {
    let gap = ui.theme().spacing.unit * 2.0;
    ui.el("empty_state")
        .column()
        .align(Align::Center)
        .gap(gap)
        .child(crate::heading(ui, 3, title))
        .child(crate::text(ui, hint).variant("muted"))
}

/// A placeholder block shown while content loads.
#[track_caller]
pub fn skeleton(ui: &Ui, width: f32, height: f32) -> El {
    let fill = ui.theme().derive().hover;
    let r = ui.theme().radii.small;
    let u = ui.clone();
    ui.el("skeleton")
        .width(width)
        .height(height)
        .semantics(move || Semantics::new(Role::Progress).label(crate::tr(&u, "loading")))
        .paint(move |cx| {
            cx.list.fill_rounded(
                Rect::new(0.0, 0.0, cx.size.width, cx.size.height),
                CornerRadii::all(r),
                Brush::Solid(fill),
            )
        })
}
