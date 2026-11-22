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
/// use arrows (one step), Page Up/Down (a tenth) and Home/End. Right to left, the minimum is
/// at the right and the Left arrow increases the value, as it moves the thumb that way.
///
/// It draws with the theme's colors `slider-track`, `slider-fill` and `slider-thumb` when the
/// theme has them (else the line, the accent and the text), and its sizes `slider-thumb` (the
/// thumb's width) and `slider-track` (the track's thickness) from
/// [`Theme::metrics`](remedy_ui_style::Theme::metrics) (else a thumb 0.6 of its height, at
/// least 10, on a 4 px track).
#[track_caller]
pub fn slider(ui: &Ui, label: impl Into<String>, value: Signal<f64>, min: f64, max: f64, step: f64) -> El {
    let label = label.into();
    let h = ui.theme().spacing.control_height;
    let span = (max - min).max(f64::MIN_POSITIVE);
    let s = ui.el("slider").height(h).fill_width().focusable(true);
    let s2 = s.clone();
    let set_from_x = move |x: f32| {
        if let Some(b) = s2.bounds() {
            let mut t = f64::from(((x - b.min.x) / b.width().max(1.0)).clamp(0.0, 1.0));
            if s2.is_rtl() {
                t = 1.0 - t;
            }
            value.set(snap(min + t * span, min, max, step));
        }
    };
    let painter_value = value;
    let s3 = s.clone();
    let key_el = s.clone();
    let l2 = label.clone();
    s.paint(move |cx| {
        let t = ((painter_value.get_untracked().unwrap_or(min) - min) / span).clamp(0.0, 1.0) as f32;
        let d = cx.theme.derive();
        let token = |name: &str| cx.theme.colors.custom.get(name).copied();
        let (track, fill) = (token("slider-track").unwrap_or(d.line), token("slider-fill"));
        let thumb = token("slider-thumb").unwrap_or(d.text);
        let mid = cx.size.height / 2.0;
        let r = cx.theme.metrics.get("slider-thumb").map_or((cx.size.height * 0.3).max(5.0), |w| w / 2.0);
        let half = cx.theme.metric("slider-track", 4.0) / 2.0;
        let (x0, x1) = (r, cx.size.width - r);
        let x = if cx.rtl { x1 - (x1 - x0) * t } else { x0 + (x1 - x0) * t };
        // The filled part runs from the minimum's end to the thumb.
        let (f0, f1) = if cx.rtl { (x, x1) } else { (x0, x) };
        cx.list.fill_rounded(
            Rect::new(x0, mid - half, x1, mid + half),
            CornerRadii::all(half),
            Brush::Solid(track),
        );
        cx.list.fill_rounded(
            Rect::new(f0, mid - half, f1, mid + half),
            CornerRadii::all(half),
            Brush::Solid(fill.unwrap_or(cx.theme.colors.accent)),
        );
        cx.list.fill_rounded(
            Rect::new(x - r, mid - r, x + r, mid + r),
            CornerRadii::all(r),
            Brush::Solid(thumb),
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
        let rtl = key_el.is_rtl();
        let next = match e.key {
            Key::ArrowUp => cur + step_size,
            Key::ArrowDown => cur - step_size,
            Key::ArrowRight if rtl => cur - step_size,
            Key::ArrowLeft if rtl => cur + step_size,
            Key::ArrowRight => cur + step_size,
            Key::ArrowLeft => cur - step_size,
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

/// Shows `target` (a [`slider`], say) only while the pointer is over `group` or keyboard focus
/// is inside it, `width` wide then and folded to nothing otherwise, as a player's volume
/// beside its mute button. Folded, it stays in the tab order and the accessibility tree: Tab
/// reaches it (which unfolds it) and screen readers find it. It fades in and out with the
/// theme's quick motion (at once under reduced motion). Returns `group`.
pub fn reveal_on_hover(group: El, target: &El, width: f32) -> El {
    let (hover, focus) = (Signal::new(false), Signal::new(false));
    let t = target.clone();
    let ui = group.ui().clone();
    let quick = ui.theme().motion.quick;
    group
        .on_hover(move |on| hover.set_if_changed(on))
        .on_focus_within(move |on| focus.set_if_changed(on))
        .bind(move || {
            let shown = hover.get() || focus.get();
            let _ = t
                .clone()
                .layout(move |l| l.width = remedy_ui_layout::Length::Px(if shown { width } else { 0.0 }));
            let to = remedy_ui_view::motion::Target::Opacity(if shown { 1.0 } else { 0.0 });
            match ui.reduced_motion() {
                true => t.set_animated(to),
                false => t.animate(
                    to,
                    remedy_ui_view::motion::Motion::Tween {
                        duration: quick,
                        easing: remedy_ui_view::motion::Easing::EASE_IN_OUT,
                    },
                ),
            }
        })
}

/// A number with − and + buttons, in steps of `step` within `min..=max`.
#[track_caller]
pub fn stepper(ui: &Ui, label: impl Into<String>, value: Signal<f64>, min: f64, max: f64, step: f64) -> El {
    let label = label.into();
    let gap = ui.theme().spacing.unit;
    let shown = ui.el("label").nowrap().layout(|l| l.min_width = remedy_ui_layout::Length::Px(32.0));
    let (u1, l1, u2, l2) = (ui.clone(), label.clone(), ui.clone(), label.clone());
    let minus = crate::icon_button(
        ui,
        crate::Icon::Minus,
        move || crate::STRINGS.format(&u1, "stepper-decrease", &[("label", l1.as_str().into())]),
        move || value.update(|v| *v = snap(*v - step, min, max, step)),
    );
    let plus = crate::icon_button(
        ui,
        crate::Icon::Plus,
        move || crate::STRINGS.format(&u2, "stepper-increase", &[("label", l2.as_str().into())]),
        move || value.update(|v| *v = snap(*v + step, min, max, step)),
    );
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
