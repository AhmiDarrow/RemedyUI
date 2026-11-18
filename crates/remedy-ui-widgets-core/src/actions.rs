//! Buttons.

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_layout::Align;
use remedy_ui_reactive::Signal;
use remedy_ui_style::States;
use remedy_ui_view::{El, IntoProp, RichText, Ui};

use crate::icons::{Icon, icon};
use crate::overlays::{MenuItem, menu};

fn centered(el: El) -> El {
    el.row().align(Align::Center).justify(remedy_ui_layout::Justify::Center)
}

/// A button with a text label. Style it with `.variant("primary" | "quiet" | "danger" |
/// "danger_solid")`; disable it with `.disabled(true)` or `.disabled(..)`.
#[track_caller]
pub fn button(ui: &Ui, label: impl IntoProp<String>, on_press: impl Fn() + 'static) -> El {
    let label = label.into_prop();
    let gap = ui.theme().spacing.unit * 1.5;
    centered(ui.el("button"))
        .gap(gap)
        .child(
            ui.el("button_label")
                .text(label.clone().map(RichText::plain))
                .nowrap()
                .layout(|l| l.shrink = 0.0),
        )
        .on_click(on_press)
        .semantics(label.map(|l| Semantics::new(Role::Button).label(l).action(Action::Click)))
}

/// A button whose label follows `label`.
#[deprecated(
    since = "0.1.0",
    note = "`button` takes a closure or signal as its label: `button(ui, move || …, on_press)`"
)]
#[track_caller]
pub fn button_dyn(ui: &Ui, label: impl Fn() -> String + 'static, on_press: impl Fn() + 'static) -> El {
    button(ui, label, on_press)
}

/// A button showing only an icon. The label is required: it is what assistive technology
/// reads and what the tooltip shows.
#[track_caller]
pub fn icon_button(ui: &Ui, glyph: Icon, label: impl IntoProp<String>, on_press: impl Fn() + 'static) -> El {
    let label = label.into_prop();
    let size = ui.theme().type_scale.body + 3.0;
    let b = centered(ui.el("icon_button"))
        .child(icon(ui, glyph, size))
        .on_click(on_press)
        .semantics(label.clone().map(|l| Semantics::new(Role::Button).label(l).action(Action::Click)));
    crate::overlays::tooltip(&b, label);
    b
}

/// A button that stays pressed: shows `on` and flips it when pressed.
#[track_caller]
pub fn toggle_button(ui: &Ui, label: impl IntoProp<String>, on: Signal<bool>) -> El {
    let label = label.into_prop();
    let b = button(ui, label.clone(), move || on.update(|v| *v = !*v));
    let b2 = b.clone();
    b.bind(move || {
        let v = on.get();
        b2.state(States::CHECKED, v);
        b2.state(States::SELECTED, v);
    })
    .semantics(move || {
        Semantics::new(Role::Button).label(label.get()).checked(on.get()).action(Action::Click)
    })
}

/// A button that opens a menu of `items` below itself.
#[track_caller]
pub fn menu_button(ui: &Ui, label: impl IntoProp<String>, items: Vec<MenuItem>) -> El {
    let label = label.into_prop();
    let ui2 = ui.clone();
    let slot: std::rc::Rc<std::cell::RefCell<Option<El>>> = std::rc::Rc::default();
    let anchor: std::rc::Rc<std::cell::RefCell<Option<El>>> = std::rc::Rc::default();
    let a2 = anchor.clone();
    let b = button(ui, label.clone(), move || {
        if let Some(open) = slot.borrow_mut().take() {
            open.remove();
            return;
        }
        if let Some(a) = a2.borrow().clone() {
            let m = menu(&ui2, &a, items.clone());
            *slot.borrow_mut() = Some(m);
        }
    })
    .child(icon(ui, Icon::ChevronDown, ui.theme().type_scale.small + 2.0))
    .semantics(label.map(|l| Semantics::new(Role::Button).label(l).expanded(false).action(Action::Click)));
    *anchor.borrow_mut() = Some(b.clone());
    b
}
