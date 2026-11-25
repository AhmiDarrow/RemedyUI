//! Choices: checks, switches, radios, segments, selects and chips.

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{CornerRadii, Point, Rect};
use remedy_ui_core::input::Key;
use remedy_ui_core::layout::Align;
use remedy_ui_paint::{Brush, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::style::States;
use remedy_ui_view::{El, IntoProp, Prop, Ui};

use crate::menus::{MenuItem, menu};

fn labeled(ui: &Ui, control: El, text: Prop<String>) -> El {
    let gap = ui.theme().spacing.unit * 2.0;
    crate::min_target(
        ui.el("choice").row().align(Align::Center).gap(gap).child(control).child(crate::text(ui, text)),
    )
}

/// A check box with a label. Pressing it (or Space) flips `value`.
#[track_caller]
pub fn check(ui: &Ui, text: impl IntoProp<String>, value: Signal<bool>) -> El {
    let text = text.into_prop();
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
    let row = labeled(ui, mark.clone(), text.clone()).on_click(move || {
        let _ = value.try_update(|v| *v = !*v);
    });
    row.bind(move || {
        let Ok(on) = value.try_get() else { return };
        mark.state(States::CHECKED, on);
    })
    .semantics(move || {
        let on = value.try_get().unwrap_or(false);
        Semantics::new(Role::Check).label(text.get()).checked(on).action(Action::Click)
    })
}

/// An on/off switch with a label.
#[track_caller]
pub fn switch(ui: &Ui, text: impl IntoProp<String>, value: Signal<bool>) -> El {
    let text = text.into_prop();
    let h = ui.theme().type_scale.body + 5.0;
    let track = ui.el("switch").width(h * 1.8).height(h).paint(move |cx| {
        let on = cx.states.contains(States::CHECKED);
        let d = cx.size.height - 4.0;
        // On is toward the end: the right, or the left right to left.
        let x = if on != cx.rtl { cx.size.width - d - 2.0 } else { 2.0 };
        cx.list.fill_rounded(
            Rect::new(x, 2.0, x + d, 2.0 + d),
            CornerRadii::all(d / 2.0),
            Brush::Solid(cx.style.foreground),
        );
    });
    let row = labeled(ui, track.clone(), text.clone()).on_click(move || {
        let _ = value.try_update(|v| *v = !*v);
    });
    row.bind(move || {
        let Ok(on) = value.try_get() else { return };
        track.state(States::CHECKED, on);
    })
    .semantics(move || {
        let on = value.try_get().unwrap_or(false);
        Semantics::new(Role::Switch).label(text.get()).checked(on).action(Action::Click)
    })
}

/// One choice of several, as radio buttons in a column. Arrow keys move the choice.
#[track_caller]
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
        let row = labeled(ui, dot.clone(), Prop::Value(text.clone())).on_click(move || {
            let _ = selected.try_set(i);
        });
        let r2 = row.clone();
        let row = row
            .bind(move || {
                let Ok(sel) = selected.try_get() else { return };
                let on = sel == i;
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

fn arrows(e: &remedy_ui_core::input::KeyEvent, selected: Signal<usize>, n: usize) -> bool {
    arrows_skipping(e, selected, n, &[])
}

/// [`arrows`], stepping over the options in `skip` (shown but not choosable).
fn arrows_skipping(e: &remedy_ui_core::input::KeyEvent, selected: Signal<usize>, n: usize, skip: &[usize]) -> bool {
    if !e.pressed || n == 0 || skip.len() >= n {
        return false;
    }
    // Nothing chosen (an index past the options, as for a stored value none of them is):
    // the arrows start at the first or the last option that can be chosen.
    let cur = selected.get_untracked().ok().filter(|c| *c < n);
    let open = |i: usize| !skip.contains(&i);
    let step = |from: usize, forward: bool| {
        let mut i = from;
        for _ in 0..n {
            i = if forward { (i + 1) % n } else { (i + n - 1) % n };
            if open(i) {
                return i;
            }
        }
        from
    };
    let next = match e.key {
        Key::ArrowDown | Key::ArrowRight => step(cur.unwrap_or(n - 1), true),
        Key::ArrowUp | Key::ArrowLeft => step(cur.unwrap_or(0), false),
        Key::Home => step(n - 1, true),
        Key::End => step(0, false),
        _ => return false,
    };
    let _ = selected.try_set(next);
    true
}

/// Mutually exclusive options side by side (a few short choices, like time presets).
#[track_caller]
pub fn segmented(ui: &Ui, label: impl Into<String>, options: &[&str], selected: Signal<usize>) -> El {
    segmented_unavailable(ui, label, options, selected, &[])
}

/// [`segmented`] with some options shown but not choosable now: each `(index, why)` in
/// `unavailable` is drawn disabled, cannot be pressed or reached with the arrow keys, and
/// says `why` on hover and to assistive technology (a share "to one person" when nobody
/// else is here).
#[track_caller]
pub fn segmented_unavailable(
    ui: &Ui,
    label: impl Into<String>,
    options: &[&str],
    selected: Signal<usize>,
    unavailable: &[(usize, String)],
) -> El {
    let label = label.into();
    let n = options.len();
    let skip: std::rc::Rc<[usize]> = unavailable.iter().map(|(i, _)| *i).collect();
    let bar = ui.el("segmented").row().semantics(Semantics::new(Role::TabList).label(label));
    options.iter().enumerate().fold(bar, |b, (i, opt)| {
        let text = (*opt).to_owned();
        let why = unavailable.iter().find(|(u, _)| *u == i).map(|(_, w)| w.clone());
        let off = why.is_some();
        let sk = skip.clone();
        let seg = ui
            .el("segment")
            .row()
            .align(Align::Center)
            .child(ui.el("segment_label").text(remedy_ui_view::RichText::plain(text.clone())).nowrap())
            .on_key(move |e| arrows_skipping(e, selected, n, &sk));
        let seg = if off {
            seg.disabled(true)
        } else {
            seg.on_click(move || {
                let _ = selected.try_set(i);
            })
        };
        if let Some(why) = &why {
            crate::tooltip(&seg, why.clone());
        }
        let s2 = seg.clone();
        b.child(seg.bind(move || {
            let Ok(sel) = selected.try_get() else { return };
            let on = sel == i;
            s2.state(States::SELECTED, on);
            let mut sem = Semantics::new(Role::Tab).label(text.clone()).selected(on).set_position(i + 1, n);
            sem = match &why {
                Some(w) => sem.disabled(true).description(w.clone()),
                None => sem.action(Action::Click),
            };
            let _ = s2.clone().semantics(sem);
        }))
    })
}

/// A drop-down choice (a [`Role::ComboBox`], as a page's `<select>`): shows the chosen option;
/// pressing opens the list, a [`Role::ListBox`] of [`Role::Option`]s with the chosen one
/// selected. Typing a letter
/// jumps to the next option starting with it; arrows step through the options.
///
/// Its value and chevron sit centered, as a button's label does; `.justify(Justify::Start)`
/// puts the value at the start, and `Justify::SpaceBetween` the chevron at the end too.
#[track_caller]
pub fn select(ui: &Ui, label: impl IntoProp<String>, options: Vec<String>, selected: Signal<usize>) -> El {
    // A label that follows state (an app switching language) is read in the binding below.
    let label = label.into_prop();
    let n = options.len();
    let ui2 = ui.clone();
    let opts = options.clone();
    let anchor: std::rc::Rc<std::cell::RefCell<Option<El>>> = std::rc::Rc::default();
    let a2 = anchor.clone();
    // Whether its list is open, for screen readers (`aria-expanded`).
    let open = Signal::new(false);
    // A button without a label of its own: the value is its first child, so `justify` places
    // it from the start edge.
    let gap = ui.theme().spacing.unit * 1.5;
    let b = crate::actions::centered(ui.el("button")).gap(gap).on_click(move || {
        let Some(a) = a2.borrow().clone() else { return };
        let now = selected.get_untracked().ok();
        let items = opts
            .iter()
            .enumerate()
            .map(|(i, o)| {
                MenuItem::option(o.clone(), now == Some(i), move || {
                    let _ = selected.try_set(i);
                })
            })
            .collect();
        open.set(true);
        let _ = menu(&ui2, &a, items).on_removed(move || open.set(false));
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
        let Ok(i) = selected.try_get() else { return };
        let v = opts2.get(i).cloned().unwrap_or_default();
        let _ = shown.clone().text(remedy_ui_view::RichText::plain(v.clone()));
        let _ = b2.clone().semantics(
            Semantics::new(Role::ComboBox)
                .label(label.get())
                .value(v)
                .has_popup(remedy_ui_core::a11y::Popup::ListBox)
                .expanded(open.try_get().unwrap_or(false))
                .action(Action::Click),
        );
    })
    .on_key(move |e| {
        if arrows(e, selected, n) && !matches!(e.key, Key::ArrowLeft | Key::ArrowRight) {
            return true;
        }
        // Type-ahead: the next option starting with the typed letter.
        if let Some(t) = e.text.as_ref().and_then(|t| t.chars().next()).filter(|c| c.is_alphanumeric()) {
            let lower = t.to_lowercase().to_string();
            // From the option after the chosen one; from the first when none is chosen.
            let start = selected.get_untracked().ok().filter(|c| *c < n).map_or(0, |c| c + 1);
            let found = (0..n)
                .map(|k| (start + k) % n)
                .find(|&i| typed.get(i).is_some_and(|o| o.to_lowercase().starts_with(&lower)));
            if let Some(i) = found {
                let _ = selected.try_set(i);
                return true;
            }
        }
        false
    })
}

/// A small selectable tag.
#[track_caller]
pub fn chip(ui: &Ui, text: impl IntoProp<String>, selected: Signal<bool>) -> El {
    let text = text.into_prop();
    let c = crate::min_target(ui.el("chip"))
        .row()
        .align(Align::Center)
        .child(ui.el("chip_label").text(text.clone().map(remedy_ui_view::RichText::plain)).nowrap())
        .on_click(move || {
            let _ = selected.try_update(|v| *v = !*v);
        });
    let c2 = c.clone();
    c.bind(move || {
        let Ok(on) = selected.try_get() else { return };
        c2.state(States::SELECTED, on);
    })
    .semantics(move || {
        let on = selected.try_get().unwrap_or(false);
        Semantics::new(Role::Check).label(text.get()).checked(on).action(Action::Click)
    })
}
