//! Values: sliders and steppers.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{CornerRadii, Rect};
use remedy_ui_input::{Buttons, Key, PointerPhase};
use remedy_ui_layout::Align;
use remedy_ui_paint::Brush;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};

fn snap(v: f64, min: f64, max: f64, step: f64) -> f64 {
    let v = v.clamp(min, max);
    if step > 0.0 { (min + ((v - min) / step).round() * step).clamp(min, max) } else { v }
}

/// A horizontal slider over `min..=max` in steps of `step` (0 for continuous). Drag it, or
/// use arrows (one step), Page Up/Down (a tenth) and Home/End.
#[track_caller]
pub fn slider(ui: &Ui, label: impl Into<String>, value: Signal<f64>, min: f64, max: f64, step: f64) -> El {
    let label = label.into();
    let h = ui.theme().spacing.control_height;
    let span = (max - min).max(f64::MIN_POSITIVE);
    let s = ui.el("slider").height(h).fill_width().focusable(true);
    let s2 = s.clone();
    let set_from_x = move |x: f32| {
        if let Some(b) = s2.bounds() {
            let t = f64::from(((x - b.min.x) / b.width().max(1.0)).clamp(0.0, 1.0));
            value.set(snap(min + t * span, min, max, step));
        }
    };
    let painter_value = value;
    let s3 = s.clone();
    let l2 = label.clone();
    s.paint(move |cx| {
        let t = ((painter_value.get_untracked().unwrap_or(min) - min) / span).clamp(0.0, 1.0) as f32;
        let d = cx.theme.derive();
        let mid = cx.size.height / 2.0;
        let r = (cx.size.height * 0.3).max(5.0);
        let (x0, x1) = (r, cx.size.width - r);
        let x = x0 + (x1 - x0) * t;
        cx.list.fill_rounded(
            Rect::new(x0, mid - 2.0, x1, mid + 2.0),
            CornerRadii::all(2.0),
            Brush::Solid(d.line),
        );
        cx.list.fill_rounded(
            Rect::new(x0, mid - 2.0, x, mid + 2.0),
            CornerRadii::all(2.0),
            Brush::Solid(cx.theme.colors.accent),
        );
        cx.list.fill_rounded(
            Rect::new(x - r, mid - r, x + r, mid + r),
            CornerRadii::all(r),
            Brush::Solid(d.text),
        );
    })
    .on_pointer(move |e| {
        let drag = e.phase == PointerPhase::Down
            || (e.phase == PointerPhase::Move && e.buttons.contains(Buttons::PRIMARY));
        if drag {
            set_from_x(e.position.x);
        }
        drag
    })
    .on_key(move |e| {
        if !e.pressed {
            return false;
        }
        let step_size = if step > 0.0 { step } else { span / 100.0 };
        let cur = value.get_untracked().unwrap_or(min);
        let next = match e.key {
            Key::ArrowRight | Key::ArrowUp => cur + step_size,
            Key::ArrowLeft | Key::ArrowDown => cur - step_size,
            Key::PageUp => cur + span / 10.0,
            Key::PageDown => cur - span / 10.0,
            Key::Home => min,
            Key::End => max,
            _ => return false,
        };
        value.set(snap(next, min, max, step));
        true
    })
    .bind(move || {
        let v = value.get();
        let _ = s3.clone().semantics(
            Semantics::new(Role::Slider)
                .label(l2.clone())
                .numeric(v, min, max)
                .action(Action::Increment)
                .action(Action::Decrement),
        );
        s3.repaint();
    })
}

/// A number with − and + buttons, in steps of `step` within `min..=max`.
#[track_caller]
pub fn stepper(ui: &Ui, label: impl Into<String>, value: Signal<f64>, min: f64, max: f64, step: f64) -> El {
    let label = label.into();
    let gap = ui.theme().spacing.unit;
    let shown = ui.el("label").nowrap().layout(|l| l.min_width = remedy_ui_layout::Length::Px(32.0));
    let minus = crate::icon_button(ui, crate::Icon::Minus, remedy_ui_view::words::fill(&ui.word("Decrease {label}"), &[("label", &label)]), move || {
        value.update(|v| *v = snap(*v - step, min, max, step));
    });
    let plus = crate::icon_button(ui, crate::Icon::Plus, remedy_ui_view::words::fill(&ui.word("Increase {label}"), &[("label", &label)]), move || {
        value.update(|v| *v = snap(*v + step, min, max, step));
    });
    let row = ui
        .el("stepper")
        .row()
        .align(Align::Center)
        .gap(gap)
        .child(minus.clone())
        .child(shown.clone())
        .child(plus.clone());
    let r2 = row.clone();
    row.bind(move || {
        let v = value.get();
        let text = if step.fract() == 0.0 { format!("{v:.0}") } else { format!("{v}") };
        let _ = shown.clone().text(remedy_ui_view::RichText::plain(text.clone()));
        let _ = minus.clone().disabled(v <= min);
        let _ = plus.clone().disabled(v >= max);
        let _ = r2
            .clone()
            .semantics(Semantics::new(Role::Slider).label(label.clone()).numeric(v, min, max).value(text));
    })
}
