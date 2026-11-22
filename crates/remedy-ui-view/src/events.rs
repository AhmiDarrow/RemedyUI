//! Input routing: hover and press states, clicks, focus, keys, text fields and input methods.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_core::Point;
use remedy_ui_host::ImeEvent;
use remedy_ui_input::{Key, KeyEvent, Modifiers, Platform, PointerEvent, PointerPhase};
use remedy_ui_style::States;
use remedy_ui_tree::{Dirty, ElementId};

use crate::Request;
use crate::element::Ui;
use crate::field::{self, Field, KeyOutcome};
use crate::spatial::Dir;

/// A click: a primary press and release on an element, or Enter or Space while it has focus.
/// Delivered to elements with [`El::on_click`](crate::El::on_click).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    /// Where the pointer was, in window logical pixels; `None` from the keyboard.
    pub position: Option<Point>,
}

/// Pointer input as elements receive it (window logical pixels).
pub type PointerInput = PointerEvent;

/// A key a window's menu bar answers to wherever focus is, offered to it before anything
/// focused sees the key ([`Ui::add_menu_keys`]). On Apple platforms, where Option types
/// characters, only [`MenuKey::F10`] comes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuKey {
    /// Alt pressed and released with nothing in between (no other key, no press): focus the
    /// menu bar, or leave it.
    AltTap,
    /// F10 with no modifier: focus the menu bar, or leave it.
    F10,
    /// Alt and a character key (without Ctrl or the Windows/Command key, so AltGr's characters
    /// are typed): open the menu with that access key. The character is in lower case.
    Mnemonic(String),
}

/// Hears a window's menu keys ([`Ui::add_menu_keys`]); returns true when it took the key.
pub type MenuKeyHandler = Rc<dyn Fn(&MenuKey) -> bool>;

fn is_alt(k: &Key) -> bool {
    matches!(k, Key::Named(n) if n == "Alt")
}

impl Ui {
    /// Hears the window's menu keys ([`MenuKey`]: Alt tapped alone, F10, Alt with a letter)
    /// before the focused element does, so a menu bar opens from a text field as from
    /// anywhere else. Only these keys come here; everything else goes to focus as usual.
    /// Returns an id for [`Ui::remove_menu_keys`]. The handler added last is asked first.
    pub fn add_menu_keys(&self, f: impl Fn(&MenuKey) -> bool + 'static) -> u64 {
        let mut i = self.0.borrow_mut();
        let id = i.next_observer;
        i.next_observer += 1;
        i.menu_keys.insert(0, (id, Rc::new(f)));
        id
    }

    /// Stops a handler added with [`Ui::add_menu_keys`].
    pub fn remove_menu_keys(&self, id: u64) {
        self.0.borrow_mut().menu_keys.retain(|(i, _)| *i != id);
    }

    /// True while Alt (Option) is held down in this window (tracked): menus underline their
    /// access keys then, except on Apple platforms, which have none.
    pub fn alt_held(&self) -> bool {
        let s = self.0.borrow().alt_held;
        s.get()
    }

    /// Tells the view its window gained or lost the keyboard (the runner does): keys held
    /// when it left are let go, so Alt does not stay down when the person switched windows
    /// with Alt+Tab.
    pub fn set_window_focused(&self, focused: bool) {
        self.note_window_focused(focused);
        if !focused {
            let held = {
                let mut i = self.0.borrow_mut();
                i.alt_tap = false;
                i.alt_held
            };
            if remedy_ui_reactive::untrack(|| held.get()) {
                held.set(false);
            }
        }
    }

    fn offer_menu_key(&self, k: &MenuKey) -> bool {
        let handlers: Vec<MenuKeyHandler> =
            self.0.borrow().menu_keys.iter().map(|(_, f)| f.clone()).collect();
        handlers.iter().any(|f| remedy_ui_reactive::batch(|| f(k)))
    }

    /// The menu key `e` is, keeping track of Alt on its own: `None` for every other key. On
    /// Apple platforms Option types characters and taps nothing, so only F10 is one there.
    fn menu_key(&self, e: &KeyEvent) -> Option<MenuKey> {
        let apple = self.platform() == Platform::Apple;
        let held = self.0.borrow().alt_held;
        if is_alt(&e.key) {
            let tap = {
                let mut i = self.0.borrow_mut();
                let tap = i.alt_tap;
                i.alt_tap = e.pressed
                    && (e.repeat && tap
                        || !e.repeat && !e.modifiers.ctrl && !e.modifiers.shift && !e.modifiers.meta);
                tap
            };
            if remedy_ui_reactive::untrack(|| held.get()) != e.pressed {
                held.set(e.pressed);
            }
            return (!e.pressed && tap && !apple).then_some(MenuKey::AltTap);
        }
        if !e.pressed {
            return None;
        }
        // Anything else while Alt is down makes it a chord, not a tap.
        self.0.borrow_mut().alt_tap = false;
        if e.composing {
            return None;
        }
        let m = e.modifiers;
        match &e.key {
            Key::F(10) if m == remedy_ui_input::Modifiers::NONE && !e.repeat => Some(MenuKey::F10),
            Key::Character(c) if m.alt && !m.ctrl && !m.meta && !e.repeat && !apple => {
                Some(MenuKey::Mnemonic(c.to_lowercase()))
            }
            _ => None,
        }
    }
}

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

    pub(crate) fn clickable(&self, id: ElementId) -> bool {
        self.0
            .borrow()
            .els
            .get(&id)
            .is_some_and(|d| d.click.is_some() && !d.states.contains(States::DISABLED))
    }

    /// Routes pointer input. Returns true when an element took it.
    pub fn pointer(&self, e: &PointerEvent) -> bool {
        let _events = remedy_ui_trace::phase!("events", ui = self.id()).entered();
        self.0.borrow_mut().consumed = false;
        let tap = self.0.borrow().interceptor.clone();
        if let Some(tap) = tap
            && tap(e)
        {
            self.0.borrow_mut().consumed = true;
            return true;
        }
        let tree = self.tree();
        let hit = tree.hit_test(e.position);
        // A wrapped link's further lines are that link too.
        let hit = self.link_at(hit, e.position).or(hit);
        if self.drag_pointer(e, hit) {
            self.0.borrow_mut().consumed = true;
            return true;
        }
        // Hover follows the pointer; a press keeps its element hovered until release. A
        // lifted finger hovers nothing (touch screens have no hover between touches), so a
        // tap leaves no hover look and no tooltip behind; a long press still shows them.
        let chain = self.chain(hit);
        let lifted = e.phase == PointerPhase::Cancel
            || (e.kind == remedy_ui_input::PointerKind::Touch && e.phase == PointerPhase::Up);
        let old = std::mem::take(&mut self.0.borrow_mut().hovered);
        for id in old.iter().filter(|id| lifted || !chain.contains(id)) {
            self.set_state(*id, States::HOVER, false);
        }
        if !lifted {
            for id in &chain {
                self.set_state(*id, States::HOVER, true);
            }
            self.0.borrow_mut().hovered = chain.clone();
        }
        if e.phase == PointerPhase::Down {
            // A press while Alt is down makes Alt part of it (Alt+click), not a tap.
            self.0.borrow_mut().alt_tap = false;
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
            self.0.borrow_mut().consumed = true;
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
                let keeps = chain.iter().any(|id| self.0.borrow().els.get(id).is_some_and(|d| d.keeps_focus));
                match focusable {
                    _ if keeps => {}
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
                // Taken first: the borrow must end before the state changes.
                let pressed = self.0.borrow_mut().pressed.take();
                if let Some(p) = pressed {
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
            self.0.borrow_mut().consumed = d.handled;
            if e.phase == PointerPhase::Down {
                self.0.borrow_mut().captured = d.handled_by;
            }
        }
        if matches!(e.phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.0.borrow_mut().captured = None;
        }
        handled
    }

    /// True when the last pointer event was consumed: an element's pointer handler took it (a
    /// context menu opening on a secondary press, a drawing surface), a tool intercepted it, or
    /// it moved a drag or a text selection. Landing on an element that does nothing with it is
    /// not consuming it. Hosts use it to keep or cancel what the platform would do with the
    /// press itself (a browser's own context menu).
    pub fn pointer_consumed(&self) -> bool {
        self.0.borrow().consumed
    }

    /// Routes a wheel or touchpad scroll to the elements under it that take scrolls
    /// ([`El::on_wheel`](crate::El::on_wheel)), innermost first. Returns true when one used it;
    /// otherwise the host scrolls the scroll containers under the pointer.
    pub fn wheel(&self, e: &remedy_ui_input::WheelEvent) -> bool {
        let _events = remedy_ui_trace::phase!("events", ui = self.id()).entered();
        self.positioned(e.position, e)
    }

    /// Routes a touchpad pinch to the elements under it that zoom
    /// ([`El::on_pinch`](crate::El::on_pinch)). Returns true when one used it.
    pub fn pinch(&self, e: &remedy_ui_input::PinchEvent) -> bool {
        let _events = remedy_ui_trace::phase!("events", ui = self.id()).entered();
        // A pinch stays with the element it started on, as a press does.
        let tree = self.tree();
        let held = self.0.borrow().pinching.filter(|p| tree.contains(*p));
        let target = match e.phase {
            remedy_ui_input::PinchPhase::Start => tree.hit_test(e.position),
            _ => held.or_else(|| tree.hit_test(e.position)),
        };
        let Some(t) = target else { return false };
        let d = tree.dispatch(t, e).ok();
        let handled = d.as_ref().is_some_and(|d| d.handled);
        self.0.borrow_mut().pinching = match e.phase {
            remedy_ui_input::PinchPhase::Start => d.and_then(|d| d.handled_by),
            remedy_ui_input::PinchPhase::Change => held.or(Some(t)),
            _ => None,
        };
        handled
    }

    fn positioned<E: 'static>(&self, at: Point, e: &E) -> bool {
        let tree = self.tree();
        let Some(hit) = tree.hit_test(at) else { return false };
        tree.dispatch(hit, e).is_ok_and(|d| d.handled)
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
    /// takes it, the runner leaves the window. With spatial navigation on
    /// ([`Ui::set_spatial_navigation`]), an Escape nothing took is offered as Back, as a TV
    /// remote's back key often arrives: a router or navigation stack listening for
    /// [`Key::Back`] goes back.
    pub fn key(&self, e: &KeyEvent) -> bool {
        let _events = remedy_ui_trace::phase!("events", ui = self.id()).entered();
        if let Some(k) = self.menu_key(e)
            && self.offer_menu_key(&k)
        {
            return true;
        }
        if e.key == Key::Back {
            if self.route_key(e) {
                return true;
            }
            let escape = e.clone().with_key(Key::Escape).with_text(None);
            return self.route_key(&escape);
        }
        if self.route_key(e) {
            return true;
        }
        if e.key == Key::Escape && self.spatial_navigation() {
            return self.route_key(&e.clone().with_key(Key::Back).with_text(None));
        }
        false
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
                        match field::secret_copy(&field) {
                            Some(keep) => self.request(Request::CopySecret { text, keep }),
                            None => self.request(Request::Copy { text, sensitive: false, html: None }),
                        }
                        field::notify_copy(&field);
                    }
                    return true;
                }
                KeyOutcome::CopyRich(doc) => {
                    self.field_edited(f, &field, true);
                    if !doc.text.is_empty() {
                        let text = doc.text.replace(remedy_ui_text::SOFT_BREAK, "\n");
                        match field::secret_copy(&field) {
                            // A secret leaves as plain text only and is not kept for a
                            // formatted paste: nothing of it outlives the clear.
                            Some(keep) => self.request(Request::CopySecret { text, keep }),
                            None => {
                                let html = remedy_ui_text::to_html(&doc);
                                self.0.borrow_mut().rich_clip = Some((html.clone(), doc));
                                self.request(Request::Copy { text, sensitive: false, html: Some(html) });
                            }
                        }
                        field::notify_copy(&field);
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
        let own_text = focused.is_some_and(|f| self.client_of(f).is_some());
        if e.pressed
            && cmd
            && !own_text
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
        // Undo and redo for the window's changes, when nothing above took the keys.
        let letter =
            |l: &str| matches!(&e.key, remedy_ui_input::Key::Character(c) if c.eq_ignore_ascii_case(l));
        if cmd && letter("z") && !e.modifiers.shift {
            return self.undo_change();
        }
        if cmd && (letter("y") || (letter("z") && e.modifiers.shift)) {
            return self.redo_change();
        }
        match &e.key {
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
                if e.modifiers == Modifiers::NONE && self.spatial_navigation() =>
            {
                match (focused, Dir::of(&e.key)) {
                    (None, _) => self.focus_start(false),
                    (Some(_), Some(dir)) => self.focus_toward(dir),
                    _ => false,
                }
            }
            Key::Tab => self.focus_start(e.modifiers.shift),
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

    /// With nothing focused, Tab and arrows start in the topmost modal: what is under it is
    /// out of reach. `None` when no modal is open, else whether focus went in.
    fn focus_in_top_modal(&self) -> Option<bool> {
        let tree = self.tree();
        let overlays = self.0.borrow().overlays;
        let top = tree.children(overlays).into_iter().rev().find(|o| tree.focus_scope(*o) == *o)?;
        self.0.borrow_mut().focus_visible = true;
        Some(crate::El { ui: self.clone(), id: top }.focus_first())
    }

    /// Tab's step to the next (or previous) focusable in the current scope; also where the
    /// first arrow goes with nothing focused.
    fn focus_start(&self, back: bool) -> bool {
        let tree = self.tree();
        if tree.focused().is_none()
            && let Some(moved) = self.focus_in_top_modal()
        {
            return moved;
        }
        let moved = if back { tree.focus_previous() } else { tree.focus_next() };
        self.0.borrow_mut().focus_visible = true;
        focus_changed(self);
        moved.is_some()
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
        let _events = remedy_ui_trace::phase!("events", ui = self.id()).entered();
        let tree = self.tree();
        let Some(f) = tree.focused() else { return false };
        let Some(field) = self.field_of(f) else {
            return self.to_client(&crate::TextInput::Ime(e.clone()));
        };
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
        let _events = remedy_ui_trace::phase!("events", ui = self.id()).entered();
        let tree = self.tree();
        let Some(f) = tree.focused() else { return };
        let Some(field) = self.field_of(f) else {
            self.to_client(&crate::TextInput::Paste(text.to_owned()));
            return;
        };
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
        let _events = remedy_ui_trace::phase!("events", ui = self.id()).entered();
        let tree = self.tree();
        let Some(f) = tree.focused() else { return };
        let Some(field) = self.field_of(f) else {
            self.to_client(&crate::TextInput::Paste(text.to_owned()));
            return;
        };
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
            ui.remember_focus(n);
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
    let client = now.filter(|_| session.is_none()).and_then(|n| ui.client_state(n).map(|s| (n, s)));
    let had = ui.0.borrow().field_session;
    match session {
        Some((n, f)) => {
            ui.0.borrow_mut().field_session = Some(n);
            let b = tree.bounds(n).unwrap_or(remedy_ui_core::Rect::ZERO);
            field::request_session(ui, &f, b);
        }
        None if client.is_some() => {
            if let Some((n, state)) = client {
                ui.0.borrow_mut().field_session = Some(n);
                ui.request(Request::TextInput(Some(state)));
            }
        }
        None if had.is_some() => {
            ui.0.borrow_mut().field_session = None;
            ui.request(Request::TextInput(None));
        }
        None => {}
    }
    // Focus within: the elements holding the old focus and not the new lose it, those
    // holding the new and not the old gain it (moving inside an element changes nothing).
    if before != now {
        let chain = |from: Option<ElementId>| {
            let mut out = Vec::new();
            let mut at = from;
            while let Some(id) = at {
                out.push(id);
                at = tree.parent(id);
            }
            out
        };
        // The old chain as it was when focus came (an element removed with focus inside it
        // has no parents left to walk).
        let old = std::mem::take(&mut ui.0.borrow_mut().focus_chain);
        let new = chain(now);
        ui.0.borrow_mut().focus_chain = new.clone();
        for id in old.iter().filter(|id| !new.contains(id)) {
            ui.set_state(*id, States::FOCUS_WITHIN, false);
            ui.mark(*id, Dirty::PAINT);
            told.extend(ui.0.borrow().els.get(id).and_then(|d| d.focus_within.clone()).map(|f| (f, false)));
        }
        for id in new.iter().filter(|id| !old.contains(id)) {
            ui.set_state(*id, States::FOCUS_WITHIN, true);
            ui.mark(*id, Dirty::PAINT);
            told.extend(ui.0.borrow().els.get(id).and_then(|d| d.focus_within.clone()).map(|f| (f, true)));
        }
    }
    for (f, focused) in told {
        f(focused);
    }
}
