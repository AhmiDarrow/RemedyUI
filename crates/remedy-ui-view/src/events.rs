//! Input routing: hover and press states, clicks, focus, keys, text fields and input methods.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_core::Point;
use remedy_ui_host::ImeEvent;
use remedy_ui_input::{Key, KeyEvent, Platform, PointerEvent, PointerPhase};
use remedy_ui_style::States;
use remedy_ui_tree::{Dirty, ElementId};

use crate::Request;
use crate::element::Ui;
use crate::field::{self, Field, KeyOutcome};

/// A click: a primary press and release on an element, or Enter or Space while it has focus.
/// Delivered to elements with [`El::on_click`](crate::El::on_click).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    /// Where the pointer was, in window logical pixels; `None` from the keyboard.
    pub position: Option<Point>,
}

/// Pointer input as elements receive it (window logical pixels).
pub type PointerInput = PointerEvent;

impl Ui {
    fn chain(&self, id: Option<ElementId>) -> Vec<ElementId> {
        let tree = self.tree();
        let mut out = Vec::new();
        let mut cur = id;
        while let Some(c) = cur {
            out.push(c);
            cur = tree.parent(c);
        }
        out
    }

    fn set_state(&self, id: ElementId, s: States, on: bool) {
        let changed = {
            let mut i = self.0.borrow_mut();
            match i.els.get_mut(&id) {
                Some(d) if d.states.contains(s) != on => {
                    d.states = d.states.with(s, on);
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.mark(id, Dirty::STYLE | Dirty::SEMANTICS);
        }
    }

    fn field_of(&self, id: ElementId) -> Option<Rc<RefCell<Field>>> {
        self.0.borrow().els.get(&id).and_then(|d| d.field.clone())
    }

    fn clickable(&self, id: ElementId) -> bool {
        self.0
            .borrow()
            .els
            .get(&id)
            .is_some_and(|d| d.click.is_some() && !d.states.contains(States::DISABLED))
    }

    fn platform(&self) -> Platform {
        self.0.borrow().platform
    }

    /// Routes pointer input. Returns true when an element took it.
    pub fn pointer(&self, e: &PointerEvent) -> bool {
        let tree = self.tree();
        let hit = tree.hit_test(e.position);
        // Hover follows the pointer; a press keeps its element hovered until release.
        let chain = self.chain(hit);
        let old = std::mem::take(&mut self.0.borrow_mut().hovered);
        for id in old.iter().filter(|id| !chain.contains(id)) {
            self.set_state(*id, States::HOVER, false);
        }
        if e.phase != PointerPhase::Cancel {
            for id in &chain {
                self.set_state(*id, States::HOVER, true);
            }
            self.0.borrow_mut().hovered = chain.clone();
        }
        match e.phase {
            PointerPhase::Down if e.buttons.contains(remedy_ui_input::Buttons::PRIMARY) => {
                let target = chain.iter().copied().find(|id| self.clickable(*id));
                if let Some(t) = target {
                    self.set_state(t, States::PRESSED, true);
                }
                self.0.borrow_mut().pressed = target.or(hit);
                let focusable = chain.iter().copied().find(|id| tree.is_focusable(*id));
                match focusable {
                    Some(f) if tree.focused() != Some(f) => {
                        let _ = tree.focus(f);
                        self.0.borrow_mut().focus_visible = false;
                        focus_changed(self);
                    }
                    None if tree.focused().is_some() => {
                        tree.blur();
                        focus_changed(self);
                    }
                    _ => {}
                }
                if let Some(f) = focusable
                    && let Some(field) = self.field_of(f)
                {
                    self.place_caret(f, &field, e.position, false);
                }
            }
            PointerPhase::Move if e.buttons.contains(remedy_ui_input::Buttons::PRIMARY) => {
                if let Some(p) = self.0.borrow().pressed.filter(|_| true)
                    && let Some(field) = self.field_of(p)
                {
                    self.place_caret(p, &field, e.position, true);
                }
            }
            PointerPhase::Up => {
                let pressed = self.0.borrow_mut().pressed.take();
                if let Some(p) = pressed {
                    self.set_state(p, States::PRESSED, false);
                    if self.clickable(p) && hit.is_some_and(|h| tree.is_ancestor_or_self(p, h)) {
                        let _ = tree.dispatch(p, &Click { position: Some(e.position) });
                    }
                }
            }
            PointerPhase::Cancel => {
                if let Some(p) = self.0.borrow_mut().pressed.take() {
                    self.set_state(p, States::PRESSED, false);
                }
            }
            _ => {}
        }
        let mut handled = hit.is_some_and(|h| h != tree.root());
        if let Some(h) = hit
            && let Ok(d) = tree.dispatch(h, e)
        {
            handled |= d.handled;
        }
        handled
    }

    fn place_caret(&self, id: ElementId, field: &Rc<RefCell<Field>>, at: Point, extend: bool) {
        let Some(b) = self.tree().bounds(id) else { return };
        field.borrow_mut().place(Point::new(at.x - b.min.x, at.y - b.min.y), extend);
        self.mark(id, Dirty::PAINT);
        field::request_session(self, field, b);
    }

    /// Routes a key. Returns true when something took it.
    pub fn key(&self, e: &KeyEvent) -> bool {
        let tree = self.tree();
        let focused = tree.focused();
        if let Some(f) = focused
            && let Some(field) = self.field_of(f)
        {
            let now = self.0.borrow().now_ms;
            let outcome = field::key(&mut field.borrow_mut(), e, now, self.platform());
            match outcome {
                KeyOutcome::Ignored => {}
                KeyOutcome::Handled { changed } => {
                    self.field_edited(f, &field, changed);
                    return true;
                }
                KeyOutcome::Submit => {
                    field::notify_submit(&field);
                    return true;
                }
                KeyOutcome::Copy(text) => {
                    self.field_edited(f, &field, true);
                    if !text.is_empty() {
                        self.request(Request::Copy { text, sensitive: false });
                    }
                    return true;
                }
                KeyOutcome::Paste => {
                    self.request(Request::Paste);
                    return true;
                }
            }
        }
        let target = focused.unwrap_or_else(|| tree.root());
        if tree.dispatch(target, e).is_ok_and(|d| d.handled) {
            return true;
        }
        if !e.pressed {
            return false;
        }
        match &e.key {
            Key::Tab => {
                let moved = if e.modifiers.shift { tree.focus_previous() } else { tree.focus_next() };
                self.0.borrow_mut().focus_visible = true;
                focus_changed(self);
                moved.is_some()
            }
            Key::Enter | Key::Space if !e.repeat => match focused {
                Some(f) if self.clickable(f) => {
                    let _ = tree.dispatch(f, &Click { position: None });
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn field_edited(&self, id: ElementId, field: &Rc<RefCell<Field>>, changed: bool) {
        if changed {
            field::mark_changed(field);
            self.mark(id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        } else {
            self.mark(id, Dirty::PAINT);
        }
        if let Some(b) = self.tree().bounds(id) {
            field::request_session(self, field, b);
        }
        if changed {
            field::notify_change(field);
        }
    }

    /// Routes an input-method event to the focused field.
    pub fn ime(&self, e: &ImeEvent) -> bool {
        let tree = self.tree();
        let Some(f) = tree.focused() else { return false };
        let Some(field) = self.field_of(f) else { return false };
        let now = self.0.borrow().now_ms;
        let changed = field::ime(&mut field.borrow_mut(), e, now);
        if changed {
            self.field_edited(f, &field, true);
        }
        changed
    }

    /// Delivers clipboard text after a [`Request::Paste`].
    pub fn paste(&self, text: &str) {
        let tree = self.tree();
        let Some(f) = tree.focused() else { return };
        let Some(field) = self.field_of(f) else { return };
        let now = self.0.borrow().now_ms;
        field::paste(&mut field.borrow_mut(), text, now);
        self.field_edited(f, &field, true);
    }

    /// Sets which platform's shortcut conventions keys follow.
    pub fn set_platform(&self, platform: Platform) {
        self.0.borrow_mut().platform = platform;
    }
}

/// Updates focus states and the input-method session after focus moved.
pub(crate) fn focus_changed(ui: &Ui) {
    let tree = ui.tree();
    let now = tree.focused();
    let (before, visible) = {
        let i = ui.0.borrow();
        (i.focus_shown, i.focus_visible)
    };
    if before != now {
        if let Some(b) = before {
            ui.set_state(b, States::FOCUS, false);
            ui.set_state(b, States::FOCUS_VISIBLE, false);
            ui.mark(b, Dirty::PAINT);
        }
        ui.0.borrow_mut().focus_shown = now;
    }
    if let Some(n) = now {
        ui.set_state(n, States::FOCUS, true);
        ui.set_state(n, States::FOCUS_VISIBLE, visible);
        ui.mark(n, Dirty::PAINT | Dirty::SEMANTICS);
    }
    let session = now.and_then(|n| ui.field_of(n).map(|f| (n, f)));
    let had = ui.0.borrow().field_session;
    match session {
        Some((n, f)) => {
            ui.0.borrow_mut().field_session = Some(n);
            let b = tree.bounds(n).unwrap_or(remedy_ui_core::Rect::ZERO);
            field::request_session(ui, &f, b);
        }
        None if had.is_some() => {
            ui.0.borrow_mut().field_session = None;
            ui.request(Request::TextInput(None));
        }
        None => {}
    }
}
