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

    pub(crate) fn set_state(&self, id: ElementId, s: States, on: bool) {
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
            self.mark(id, Dirty::STYLE | Dirty::PAINT | Dirty::SEMANTICS);
            if s == States::HOVER {
                let cb = self.0.borrow().els.get(&id).and_then(|d| d.hover.clone());
                if let Some(cb) = cb {
                    cb(on);
                }
            }
        }
    }

    pub(crate) fn field_of(&self, id: ElementId) -> Option<Rc<RefCell<Field>>> {
        self.0.borrow().els.get(&id).and_then(|d| d.field.clone())
    }

    fn clickable(&self, id: ElementId) -> bool {
        self.0
            .borrow()
            .els
            .get(&id)
            .is_some_and(|d| d.click.is_some() && !d.states.contains(States::DISABLED))
    }

    /// Routes pointer input. Returns true when an element took it.
    pub fn pointer(&self, e: &PointerEvent) -> bool {
        let tap = self.0.borrow().interceptor.clone();
        if let Some(tap) = tap
            && tap(e)
        {
            return true;
        }
        let tree = self.tree();
        let hit = tree.hit_test(e.position);
        // A wrapped link's further lines are that link too.
        let hit = self.link_at(hit, e.position).or(hit);
        if self.drag_pointer(e, hit) {
            return true;
        }
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
        if e.phase == PointerPhase::Down && e.buttons.contains(remedy_ui_input::Buttons::PRIMARY) {
            // Count presses in a row: same element, within the double-click time and a few
            // pixels (a hand moves a little between clicks).
            let mut i = self.0.borrow_mut();
            let again = i.last_press.is_some_and(|(id, p, t)| {
                let (dx, dy) = (e.position.x - p.x, e.position.y - p.y);
                Some(id) == hit && dx * dx + dy * dy <= 16.0 && e.time.saturating_sub(t) <= i.double_click
            });
            i.click_count = if again { i.click_count + 1 } else { 1 };
            i.last_press = hit.map(|h| (h, e.position, e.time));
        }
        // Selecting text: a press starts (or clears) it; dragging extends it.
        let clicks = self.0.borrow().click_count;
        let selecting = self.select_pointer(e, &chain, clicks);
        if selecting && e.phase != PointerPhase::Down {
            return true;
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
                    // A click on empty space blurs, except inside the scope holding focus (a
                    // modal keeps focus while you click its background).
                    None if tree.focused().is_some_and(|f| {
                        let scope = tree.focus_scope(f);
                        scope == tree.root() || !chain.contains(&scope)
                    }) =>
                    {
                        tree.blur();
                        focus_changed(self);
                    }
                    _ => {}
                }
                if let Some(f) = focusable
                    && let Some(field) = self.field_of(f)
                {
                    let b = tree.bounds(f).unwrap_or_default();
                    let local = Point::new(e.position.x - b.min.x, e.position.y - b.min.y);
                    // Alt+click adds a cursor in a code field.
                    if e.modifiers.alt && field.borrow_mut().add_cursor_at(local) {
                        self.mark(f, Dirty::PAINT);
                    } else {
                        self.place_caret(f, &field, e.position, false);
                    }
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
        // Pointer capture: the element that took a press keeps the drag.
        let captured = self.0.borrow().captured.filter(|c| tree.contains(*c));
        let target = match e.phase {
            PointerPhase::Move | PointerPhase::Up | PointerPhase::Cancel if captured.is_some() => captured,
            _ => hit,
        };
        let mut handled = target.is_some_and(|h| h != tree.root());
        if let Some(h) = target
            && let Ok(d) = tree.dispatch(h, e)
        {
            handled |= d.handled;
            if e.phase == PointerPhase::Down {
                self.0.borrow_mut().captured = d.handled_by;
            }
        }
        if matches!(e.phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.0.borrow_mut().captured = None;
        }
        handled
    }

    fn place_caret(&self, id: ElementId, field: &Rc<RefCell<Field>>, at: Point, extend: bool) {
        let Some(b) = self.tree().bounds(id) else { return };
        let changed = field.borrow_mut().place(Point::new(at.x - b.min.x, at.y - b.min.y), extend);
        if changed {
            self.field_edited(id, field, true);
            return;
        }
        self.mark(id, Dirty::PAINT);
        field::request_session(self, field, b);
        field::notify_caret(field);
    }

    /// Routes a key. Returns true when something took it.
    ///
    /// The system's back action ([`Key::Back`]) is offered as itself first (a navigation
    /// stack goes back), then as Escape (a menu, dialog or drag is dismissed). When neither
    /// takes it, the runner leaves the window.
    pub fn key(&self, e: &KeyEvent) -> bool {
        if e.key == Key::Back {
            if self.route_key(e) {
                return true;
            }
            let escape = KeyEvent { key: Key::Escape, text: None, ..e.clone() };
            return self.route_key(&escape);
        }
        self.route_key(e)
    }

    fn route_key(&self, e: &KeyEvent) -> bool {
        if e.pressed && e.key == remedy_ui_input::Key::Escape && self.cancel_drag() {
            return true;
        }
        let tree = self.tree();
        let focused = tree.focused();
        if let Some(f) = focused
            && let Some(field) = self.field_of(f)
        {
            if let Some(filter) = field::key_filter(&field)
                && filter(e)
            {
                return true;
            }
            let now = self.0.borrow().now_ms;
            let outcome = field::key(&mut field.borrow_mut(), e, now, self.platform());
            match outcome {
                KeyOutcome::Ignored => {}
                KeyOutcome::Handled { changed } => {
                    self.field_edited(f, &field, changed);
                    return true;
                }
                KeyOutcome::Submit => {
                    field::notify_submit(&field, e.modifiers);
                    return true;
                }
                KeyOutcome::Copy(text) => {
                    self.field_edited(f, &field, true);
                    if !text.is_empty() {
                        self.request(Request::Copy { text, sensitive: false, html: None });
                    }
                    return true;
                }
                KeyOutcome::CopyRich(doc) => {
                    self.field_edited(f, &field, true);
                    if !doc.text.is_empty() {
                        let html = remedy_ui_text::to_html(&doc);
                        let text = doc.text.replace(remedy_ui_text::SOFT_BREAK, "\n");
                        self.0.borrow_mut().rich_clip = Some((html.clone(), doc));
                        self.request(Request::Copy { text, sensitive: false, html: Some(html) });
                    }
                    return true;
                }
                KeyOutcome::Paste => {
                    self.request(Request::Paste);
                    return true;
                }
            }
        }
        // Copy selected text when no field holds focus (a field copies its own).
        let cmd = if self.platform() == remedy_ui_input::Platform::Apple {
            e.modifiers.meta
        } else {
            e.modifiers.ctrl
        };
        if e.pressed
            && cmd
            && matches!(&e.key, remedy_ui_input::Key::Character(c) if c.eq_ignore_ascii_case("c"))
            && let Some(text) = self.selected_text()
        {
            self.request(Request::Copy { text, sensitive: false, html: None });
            return true;
        }
        // With nothing focused, keys go to the window's content root (where app-wide
        // shortcuts listen).
        let content = self.0.borrow().content;
        let target = focused.unwrap_or(content);
        if tree.dispatch(target, e).is_ok_and(|d| d.handled) {
            return true;
        }
        if !e.pressed {
            return false;
        }
        match &e.key {
            Key::Tab => {
                // With nothing focused, Tab starts in the topmost modal: what is under it is
                // out of reach.
                if focused.is_none() {
                    let overlays = self.0.borrow().overlays;
                    let top = tree.children(overlays).into_iter().rev().find(|o| tree.focus_scope(*o) == *o);
                    if let Some(top) = top {
                        self.0.borrow_mut().focus_visible = true;
                        return crate::El { ui: self.clone(), id: top }.focus_first();
                    }
                }
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

    pub(crate) fn field_edited(&self, id: ElementId, field: &Rc<RefCell<Field>>, changed: bool) {
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
        field::notify_caret(field);
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

    /// Delivers the clipboard after a [`Request::Paste`]: its text, and its HTML when it has
    /// some. A rich field takes the HTML's allowed formatting (and a copy of its own whole);
    /// other fields take the text.
    pub fn paste_html(&self, text: &str, html: Option<&str>) {
        let tree = self.tree();
        let Some(f) = tree.focused() else { return };
        let Some(field) = self.field_of(f) else { return };
        let rich = field::is_rich(&field.borrow());
        let Some(html) = html.filter(|_| rich) else {
            return self.paste(text);
        };
        let own = self.0.borrow().rich_clip.as_ref().filter(|(h, _)| h == html).map(|(_, d)| d.clone());
        let doc = own.unwrap_or_else(|| remedy_ui_text::from_html(html));
        let now = self.0.borrow().now_ms;
        field::paste_doc(&mut field.borrow_mut(), &doc, now);
        self.field_edited(f, &field, true);
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
/// An element's focus-change callback.
type FocusCallback = Rc<dyn Fn(bool)>;

pub(crate) fn focus_changed(ui: &Ui) {
    let tree = ui.tree();
    let now = tree.focused();
    let (before, visible) = {
        let i = ui.0.borrow();
        (i.focus_shown, i.focus_visible)
    };
    let mut told: Vec<(FocusCallback, bool)> = Vec::new();
    if before != now {
        if let Some(b) = before {
            ui.set_state(b, States::FOCUS, false);
            ui.set_state(b, States::FOCUS_VISIBLE, false);
            ui.mark(b, Dirty::PAINT);
            told.extend(ui.0.borrow().els.get(&b).and_then(|d| d.focus.clone()).map(|f| (f, false)));
        }
        ui.0.borrow_mut().focus_shown = now;
        if let Some(n) = now {
            told.extend(ui.0.borrow().els.get(&n).and_then(|d| d.focus.clone()).map(|f| (f, true)));
        }
    }
    if let Some(n) = now {
        ui.set_state(n, States::FOCUS, true);
        ui.set_state(n, States::FOCUS_VISIBLE, visible);
        ui.mark(n, Dirty::PAINT | Dirty::SEMANTICS);
        // Moving focus by keyboard shows what got it.
        if visible && before != now {
            crate::El { ui: ui.clone(), id: n }.scroll_into_view();
        }
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
    for (f, focused) in told {
        f(focused);
    }
}
