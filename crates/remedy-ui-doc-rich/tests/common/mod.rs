//! Helpers shared by the document tests.
#![allow(dead_code)]

use std::time::Duration;

use remedy_ui_core::{Point, Size};
use remedy_ui_core::input::{Buttons, Key, KeyEvent, Modifiers, PointerEvent, PointerPhase};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Request, Ui};

pub fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

/// A frame; scroll requests are applied the way the runner applies them.
pub fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
    for r in ui.take_requests() {
        if let Request::ScrollTo { scroller, offset } = r {
            ui.set_scroll_offset(scroller, offset);
            let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
        }
    }
    let _ = ui.take_requests();
}

pub fn key(ui: &Ui, k: Key, m: Modifiers) -> bool {
    let r = ui.key(&KeyEvent::press(k).with_modifiers(m).with_text(None));
    frame(ui);
    r
}

pub fn pointer(ui: &Ui, at: Point, phase: PointerPhase, buttons: Buttons, time: Duration) {
    ui.pointer(&PointerEvent::new(phase, at).with_buttons(buttons).with_time(time));
}

pub fn centre(el: &El) -> Point {
    let b = el.bounds().unwrap_or_default();
    Point::new((b.min.x + b.max.x) / 2.0, (b.min.y + b.max.y) / 2.0)
}

pub fn click(ui: &Ui, at: Point, time: Duration) {
    pointer(ui, at, PointerPhase::Down, Buttons::PRIMARY, time);
    pointer(ui, at, PointerPhase::Up, Buttons::default(), time);
    frame(ui);
}

/// Every element under `el` made by `widget`, in tree order.
pub fn find(el: &El, widget: &str) -> Vec<El> {
    let mut out = Vec::new();
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == widget) {
            out.push(e.clone());
        }
        let mut kids = e.child_elements();
        kids.reverse();
        stack.extend(kids);
    }
    out
}

pub fn texts(row: &El) -> Vec<String> {
    find(row, "label").iter().filter_map(|l| l.info().and_then(|i| i.text)).collect()
}

pub const NONE: Modifiers = Modifiers::NONE;
pub const SHIFT: Modifiers = Modifiers { ctrl: false, shift: true, alt: false, meta: false };
pub const CTRL: Modifiers = Modifiers { ctrl: true, shift: false, alt: false, meta: false };
pub const ALT_SHIFT: Modifiers = Modifiers { ctrl: false, shift: true, alt: true, meta: false };

/// Problems the accessibility audit finds, described.
pub fn audit(ui: &Ui) -> Vec<String> {
    frame(ui);
    remedy_ui_devtools::audit(ui)
        .iter()
        .map(|p| {
            format!(
                "{} [{}]",
                p.message(),
                ui.element(p.element).info().map(|i| remedy_ui_devtools::describe(&i)).unwrap_or_default()
            )
        })
        .collect()
}

/// Every element under `el` whose semantics have role `role`.
pub fn with_role(el: &El, role: remedy_ui_core::a11y::Role) -> Vec<El> {
    let mut out = Vec::new();
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().and_then(|i| i.semantics).is_some_and(|s| s.role() == role) {
            out.push(e.clone());
        }
        let mut kids = e.child_elements();
        kids.reverse();
        stack.extend(kids);
    }
    out
}

/// The name assistive technology reads for `el`.
pub fn name(el: &El) -> String {
    el.info().and_then(|i| i.semantics).and_then(|s| s.name().map(str::to_owned)).unwrap_or_default()
}

/// The first element under `el` made by `widget`.
pub fn first(el: &El, widget: &str) -> El {
    find(el, widget).into_iter().next().unwrap_or_else(|| unreachable!("no {widget}"))
}
