//! Choices: checks, switches, radios, segments, selects and chips.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect};
use remedy_ui_input::Key;
use remedy_ui_layout::Align;
use remedy_ui_paint::{Brush, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_reactive::Signal;
use remedy_ui_style::States;
use remedy_ui_view::{El, Ui};

use crate::overlays::{MenuItem, menu};

fn labeled(ui: &Ui, control: El, text: &str) -> El {
    let gap = ui.theme().spacing.unit * 2.0;
    ui.el("choice").row().align(Align::Center).gap(gap).child(control).child(crate::text(ui, text.to_owned()))
}

/// A check box with a label. Pressing it (or Space) flips `value`.
pub fn check(ui: &Ui, text: impl Into<String>, value: Signal<bool>) -> El {
    let text = text.into();
    let box_size = ui.theme().type_scale.body + 3.0;
    let mark = ui.el("check").width(box_size).height(box_size).paint(|cx| {
        if !cx.states.contains(States::CHECKED) {
            return;
        }
        let (w, h) = (cx.size.width, cx.size.height);
        let mut p = Path::new();
        p.move_to(Point::new(w * 0.24, h * 0.52))
            .line_to(Point::new(w * 0.43, h * 0.7))
            .line_to(Point::new(w * 0.76, h * 0.32));
        let stroke = Stroke {
            width: (w / 8.0).max(1.5),
            cap: StrokeCap::Round,
            join: StrokeJoin::Round,
            dashes: Vec::new(),
        };
        cx.list.push(remedy_ui_paint::Item::StrokePath {
            path: p,
            stroke,
            brush: Brush::Solid(cx.style.foreground),
        });
    });
    let row = labeled(ui, mark.clone(), &text).on_click(move || value.update(|v| *v = !*v));
    let r2 = row.clone();
    row.bind(move || {
        let v = value.get();
        mark.state(States::CHECKED, v);
        let _ = r2
            .clone()
            .semantics(Semantics::new(Role::Check).label(text.clone()).checked(v).action(Action::Click));
    })
}

/// An on/off switch with a label.
pub fn switch(ui: &Ui, text: impl Into<String>, value: Signal<bool>) -> El {
    let text = text.into();
    let h = ui.theme().type_scale.body + 5.0;
    let track = ui.el("switch").width(h * 1.8).height(h).paint(move |cx| {
        let on = cx.states.contains(States::CHECKED);
        let d = cx.size.height - 4.0;
        let x = if on { cx.size.width - d - 2.0 } else { 2.0 };
        cx.list.fill_rounded(
            Rect::new(x, 2.0, x + d, 2.0 + d),
            CornerRadii::all(d / 2.0),
            Brush::Solid(cx.style.foreground),
        );
    });
    let row = labeled(ui, track.clone(), &text).on_click(move || value.update(|v| *v = !*v));
    let r2 = row.clone();
    row.bind(move || {
        let v = value.get();
        track.state(States::CHECKED, v);
        let _ = r2
            .clone()
            .semantics(Semantics::new(Role::Switch).label(text.clone()).checked(v).action(Action::Click));
    })
}

/// One choice of several, as radio buttons in a column. Arrow keys move the choice.
pub fn radio_group(ui: &Ui, label: impl Into<String>, options: &[&str], selected: Signal<usize>) -> El {
    let label = label.into();
    let n = options.len();
    let gap = ui.theme().spacing.unit * 1.5;
    let group = ui.el("radio_group").column().gap(gap).semantics(Semantics::new(Role::Group).label(label));
    let size = ui.theme().type_scale.body + 3.0;
    options.iter().enumerate().fold(group, |g, (i, opt)| {
        let dot = ui.el("radio").width(size).height(size).paint(|cx| {
            if cx.states.contains(States::CHECKED) {
                let r = cx.size.width * 0.25;
                let c = Point::new(cx.size.width / 2.0, cx.size.height / 2.0);
                cx.list.fill_rounded(
                    Rect::new(c.x - r, c.y - r, c.x + r, c.y + r),
                    CornerRadii::all(r),
                    Brush::Solid(cx.style.foreground),
                );
            }
        });
        let text = (*opt).to_owned();
        let row = labeled(ui, dot.clone(), &text).on_click(move || selected.set(i));
        let r2 = row.clone();
        let row = row
            .bind(move || {
                let on = selected.get() == i;
                dot.state(States::CHECKED, on);
                let _ = r2.clone().semantics(
                    Semantics::new(Role::Radio)
                        .label(text.clone())
                        .checked(on)
                        .set_position(i + 1, n)
                        .action(Action::Click),
                );
            })
            .on_key(move |e| arrows(e, selected, n));
        g.child(row)
    })
}

fn arrows(e: &remedy_ui_input::KeyEvent, selected: Signal<usize>, n: usize) -> bool {
    if !e.pressed || n == 0 {
        return false;
    }
    let cur = selected.get_untracked().unwrap_or(0);
    let next = match e.key {
        Key::ArrowDown | Key::ArrowRight => (cur + 1) % n,
        Key::ArrowUp | Key::ArrowLeft => (cur + n - 1) % n,
        Key::Home => 0,
        Key::End => n - 1,
        _ => return false,
    };
    selected.set(next);
    true
}

/// Mutually exclusive options side by side (a few short choices, like time presets).
pub fn segmented(ui: &Ui, label: impl Into<String>, options: &[&str], selected: Signal<usize>) -> El {
    let label = label.into();
    let n = options.len();
    let bar = ui.el("segmented").row().semantics(Semantics::new(Role::TabList).label(label));
    options.iter().enumerate().fold(bar, |b, (i, opt)| {
        let text = (*opt).to_owned();
        let seg = ui
            .el("segment")
            .row()
            .align(Align::Center)
            .child(ui.el("segment_label").text(remedy_ui_view::RichText::plain(text.clone())).nowrap())
            .on_click(move || selected.set(i))
            .on_key(move |e| arrows(e, selected, n));
        let s2 = seg.clone();
        b.child(seg.bind(move || {
            let on = selected.get() == i;
            s2.state(States::SELECTED, on);
            let _ = s2.clone().semantics(
                Semantics::new(Role::Tab)
                    .label(text.clone())
                    .selected(on)
                    .set_position(i + 1, n)
                    .action(Action::Click),
            );
        }))
    })
}

/// A drop-down choice: shows the chosen option; pressing opens the list. Typing a letter
/// jumps to the next option starting with it; arrows step through the options.
pub fn select(ui: &Ui, label: impl Into<String>, options: Vec<String>, selected: Signal<usize>) -> El {
    let label = label.into();
    let n = options.len();
    let ui2 = ui.clone();
    let opts = options.clone();
    let anchor: std::rc::Rc<std::cell::RefCell<Option<El>>> = std::rc::Rc::default();
    let a2 = anchor.clone();
    let b = crate::button(ui, "", move || {
        let Some(a) = a2.borrow().clone() else { return };
        let items =
            opts.iter().enumerate().map(|(i, o)| MenuItem::new(o.clone(), move || selected.set(i))).collect();
        let _ = menu(&ui2, &a, items);
    });
    let shown = ui.el("select_value").nowrap();
    let b = b.child(shown.clone()).child(crate::icon(
        ui,
        crate::Icon::ChevronDown,
        ui.theme().type_scale.small + 2.0,
    ));
    *anchor.borrow_mut() = Some(b.clone());
    let opts2 = options.clone();
    let b2 = b.clone();
    let typed = options.clone();
    b.bind(move || {
        let i = selected.get();
        let v = opts2.get(i).cloned().unwrap_or_default();
        let _ = shown.clone().text(remedy_ui_view::RichText::plain(v.clone()));
        let _ = b2
            .clone()
            .semantics(Semantics::new(Role::Button).label(label.clone()).value(v).action(Action::Click));
    })
    .on_key(move |e| {
        if arrows(e, selected, n) && !matches!(e.key, Key::ArrowLeft | Key::ArrowRight) {
            return true;
        }
        // Type-ahead: the next option starting with the typed letter.
        if let Some(t) = e.text.as_ref().and_then(|t| t.chars().next()).filter(|c| c.is_alphanumeric()) {
            let cur = selected.get_untracked().unwrap_or(0);
            let lower = t.to_lowercase().to_string();
            let found = (1..=n)
                .map(|k| (cur + k) % n)
                .find(|&i| typed.get(i).is_some_and(|o| o.to_lowercase().starts_with(&lower)));
            if let Some(i) = found {
                selected.set(i);
                return true;
            }
        }
        false
    })
}

/// A small selectable tag.
pub fn chip(ui: &Ui, text: impl Into<String>, selected: Signal<bool>) -> El {
    let text = text.into();
    let c = ui
        .el("chip")
        .row()
        .child(ui.el("chip_label").text(remedy_ui_view::RichText::plain(text.clone())).nowrap())
        .on_click(move || selected.update(|v| *v = !*v));
    let c2 = c.clone();
    c.bind(move || {
        let on = selected.get();
        c2.state(States::SELECTED, on);
        let _ = c2
            .clone()
            .semantics(Semantics::new(Role::Check).label(text.clone()).checked(on).action(Action::Click));
    })
}
