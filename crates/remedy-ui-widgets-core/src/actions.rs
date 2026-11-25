//! Buttons.

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::layout::Align;
use remedy_ui_core::reactive::Signal;
use remedy_ui_core::style::States;
use remedy_ui_engine::view::{El, IntoProp, RichText, Ui};

use crate::icons::{Icon, icon};
use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_core::input::Key;

use crate::menus::{MenuItem, OpenMenu, open_menu};

pub(crate) fn centered(el: El) -> El {
    el.row().align(Align::Center).justify(remedy_ui_core::layout::Justify::Center)
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

/// A button that opens a menu of `items` below itself: on a click, Enter or Space, and with
/// Down (focus on its first entry) or Up (its last). A second click closes it; it reports
/// whether its menu is open to assistive technology.
#[track_caller]
pub fn menu_button(ui: &Ui, label: impl IntoProp<String>, items: Vec<MenuItem>) -> El {
    let label = label.into_prop();
    let open: Signal<bool> = Signal::new(false);
    let slot: Rc<RefCell<Option<OpenMenu>>> = Rc::default();
    let anchor: Rc<RefCell<Option<El>>> = Rc::default();
    let items = Rc::new(items);
    let show = {
        let (slot, anchor, items) = (slot.clone(), anchor.clone(), items.clone());
        Rc::new(move |keyboard: bool, last: bool| {
            if let Some(m) = slot.borrow_mut().take()
                && m.is_open()
            {
                m.close();
                return;
            }
            let Some(a) = anchor.borrow().clone() else { return };
            let b = a.bounds().unwrap_or(remedy_ui_core::Rect::ZERO);
            let m = open_menu(&a, b, (*items).clone(), keyboard, last);
            open.set(true);
            m.on_closed(move || open.set(false));
            *slot.borrow_mut() = Some(m);
        })
    };
    let (s1, s2) = (show.clone(), show);
    let b = button(ui, label.clone(), move || s1(false, false))
        .child(icon(ui, Icon::ChevronDown, ui.theme().type_scale.small + 2.0))
        .on_key(move |e| match e.key {
            Key::ArrowDown | Key::ArrowUp if e.pressed => {
                s2(true, e.key == Key::ArrowUp);
                true
            }
            _ => false,
        })
        .semantics(move || {
            Semantics::new(Role::Button).label(label.get()).expanded(open.get()).action(Action::Click)
        });
    *anchor.borrow_mut() = Some(b.clone());
    b
}
