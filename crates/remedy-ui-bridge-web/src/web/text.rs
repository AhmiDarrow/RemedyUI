//! Text input through a hidden field at the caret.
//!
//! While an engine text field edits, a transparent `<textarea>` (multi-line fields) or
//! `<input>` (one-line fields; `type=password` for secrets, so input methods and soft
//! keyboards learn nothing) holds focus at the caret. The input carries the field's
//! [`InputPurpose`] as `type`, `inputmode`, `autocomplete` and `enterkeyhint`, so soft
//! keyboards show the right keys and password managers fill usernames and passwords:
//! input methods compose in it and place their candidate windows by it, and soft keyboards
//! open for it. It mirrors the field's text around the caret, so the composition, the
//! soft keyboard's autocorrect and its deletions have context; what they change is read back
//! by comparing the text, and reaches the field as input-method events.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use remedy_ui_host::{EnterKey, HostEvent, ImeEvent, InputPurpose, TextInputState, WindowId};
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use super::dom::{self, Listener};
use super::post;
use crate::convert;

/// The hidden fields of one window and the session they serve.
pub(crate) struct TextInput {
    /// Multi-line fields.
    area: web_sys::HtmlTextAreaElement,
    /// One-line fields and secrets.
    line: web_sys::HtmlInputElement,
    /// The field's state as last told by the engine, with the text the hidden field holds.
    state: RefCell<Option<TextInputState>>,
    /// An input method is composing.
    pub(crate) composing: Cell<bool>,
    /// The bridge itself is moving focus (its focus events are not the user's).
    pub(crate) moving_focus: Cell<bool>,
    listeners: RefCell<Vec<Listener>>,
}

fn hidden(el: &HtmlElement) {
    dom::style(
        el,
        &[
            ("position", "absolute"),
            ("left", "0"),
            ("top", "0"),
            ("width", "1px"),
            ("height", "1em"),
            ("padding", "0"),
            ("margin", "0"),
            ("border", "none"),
            ("outline", "none"),
            ("resize", "none"),
            ("overflow", "hidden"),
            ("opacity", "0"),
            ("color", "transparent"),
            ("background", "transparent"),
            ("caret-color", "transparent"),
            ("white-space", "pre"),
            ("pointer-events", "none"),
            ("display", "none"),
        ],
    );
}

impl TextInput {
    pub(crate) fn new(surface: &HtmlElement) -> Option<TextInput> {
        let area: web_sys::HtmlTextAreaElement = dom::element("textarea")?;
        let line: web_sys::HtmlInputElement = dom::element("input")?;
        hidden(&area);
        hidden(&line);
        for el in [area.unchecked_ref::<web_sys::Element>(), line.unchecked_ref()] {
            let _ = el.set_attribute("autocomplete", "off");
            let _ = el.set_attribute("data-remedy-text", "");
        }
        let _ = surface.append_child(&area);
        let _ = surface.append_child(&line);
        Some(TextInput {
            area,
            line,
            state: RefCell::new(None),
            composing: Cell::new(false),
            moving_focus: Cell::new(false),
            listeners: RefCell::new(Vec::new()),
        })
    }

    /// Listens to both hidden fields for `window`.
    pub(crate) fn attach(&self, window: WindowId) {
        let mut ls = Vec::new();
        for el in [self.area.unchecked_ref::<web_sys::EventTarget>(), self.line.unchecked_ref()] {
            ls.push(Listener::new(el, "compositionstart", move |_| {
                with(window, |t| t.composition_start(window))
            }));
            ls.push(Listener::new(el, "compositionupdate", move |e| {
                let data = e.dyn_ref::<web_sys::CompositionEvent>().and_then(web_sys::CompositionEvent::data);
                with(window, |t| t.composition_update(window, data.unwrap_or_default()));
            }));
            ls.push(Listener::new(el, "compositionend", move |e| {
                let data = e.dyn_ref::<web_sys::CompositionEvent>().and_then(web_sys::CompositionEvent::data);
                with(window, |t| t.composition_end(window, data.unwrap_or_default()));
            }));
            ls.push(Listener::new(el, "input", move |e| {
                let composing =
                    e.dyn_ref::<web_sys::InputEvent>().is_some_and(web_sys::InputEvent::is_composing);
                with(window, |t| t.input(window, composing));
            }));
        }
        *self.listeners.borrow_mut() = ls;
    }

    fn element(&self, area: bool) -> &HtmlElement {
        if area { self.area.unchecked_ref() } else { self.line.unchecked_ref() }
    }

    fn value(&self, area: bool) -> String {
        if area { self.area.value() } else { self.line.value() }
    }

    fn set_value(&self, area: bool, text: &str, selection: (u32, u32)) {
        if !area {
            self.line.set_value(text);
            let _ = self.line.set_selection_range(selection.0, selection.1);
        } else {
            self.area.set_value(text);
            let _ = self.area.set_selection_range(selection.0, selection.1);
        }
    }

    /// Names the hidden field after the engine field it stands in for.
    pub(crate) fn set_label(&self, label: Option<&str>) {
        let area = self.state.borrow().as_ref().is_some_and(in_area);
        dom::attr(self.element(area), "aria-label", label);
    }

    /// True while a field edits.
    pub(crate) fn active(&self) -> bool {
        self.state.borrow().is_some()
    }

    /// When the field's Enter key moves focus ("next" or "previous"), whether it goes back.
    pub(crate) fn enter_moves(&self) -> Option<bool> {
        match self.state.borrow().as_ref().map(TextInputState::enter_key) {
            Some(EnterKey::Next) => Some(false),
            Some(EnterKey::Previous) => Some(true),
            _ => None,
        }
    }

    /// Starts or updates the session for the engine's field, `label` naming it for screen
    /// readers.
    pub(crate) fn update(&self, state: TextInputState, label: Option<String>) {
        let area = in_area(&state);
        let el = self.element(area).clone();
        let other = self.element(!area).clone();
        dom::style(&other, &[("display", "none")]);
        let caret = state.caret;
        dom::style(
            &el,
            &[
                ("display", "block"),
                ("left", &format!("{}px", caret.min.x)),
                ("top", &format!("{}px", caret.min.y)),
                ("height", &format!("{}px", caret.height().max(1.0))),
                ("font-size", &format!("{}px", caret.height().max(1.0))),
            ],
        );
        if !area {
            self.line.set_type(input_type(&state));
        }
        let hints = Hints::of(&state);
        let _ = el.set_attribute("enterkeyhint", hints.enter);
        dom::attr(&el, "inputmode", hints.mode);
        let _ = el.set_attribute("autocomplete", hints.autocomplete);
        let literal = (!hints.prose).then_some("off");
        dom::attr(&el, "autocapitalize", literal);
        dom::attr(&el, "autocorrect", literal);
        dom::attr(&el, "spellcheck", (!hints.prose).then_some("false"));
        dom::attr(&el, "aria-label", label.as_deref());
        dom::attr(&el, "aria-multiline", state.multiline.then_some("true"));
        // Text changed under a composition would break it; the field catches up after.
        if !self.composing.get() {
            let range = convert::clamp_range(&state.surrounding, &state.selection);
            let sel = (
                convert::byte_to_utf16(&state.surrounding, range.start),
                convert::byte_to_utf16(&state.surrounding, range.end),
            );
            if self.value(area) != state.surrounding || !self.selection_is(area, sel) {
                self.set_value(area, &state.surrounding, sel);
            }
        }
        *self.state.borrow_mut() = Some(state);
        if dom::active_element().as_ref() != Some(el.unchecked_ref()) {
            self.moving_focus.set(true);
            dom::focus(&el);
            self.moving_focus.set(false);
        }
    }

    fn selection_is(&self, area: bool, (a, b): (u32, u32)) -> bool {
        let (s, e) = if !area {
            (self.line.selection_start().ok().flatten(), self.line.selection_end().ok().flatten())
        } else {
            (self.area.selection_start().ok().flatten(), self.area.selection_end().ok().flatten())
        };
        s == Some(a) && e == Some(b)
    }

    /// Ends the session: the hidden fields let go of focus (which closes a soft keyboard) and
    /// are emptied.
    pub(crate) fn end(&self) {
        let was = self.state.borrow_mut().take();
        let Some(was) = was else { return };
        let area = in_area(&was);
        let el = self.element(area).clone();
        let focused = dom::active_element().as_ref() == Some(el.unchecked_ref());
        self.set_value(area, "", (0, 0));
        dom::style(&el, &[("display", "none")]);
        self.composing.set(false);
        if focused && let Some(parent) = el.parent_element().and_then(|p| p.dyn_into::<HtmlElement>().ok()) {
            self.moving_focus.set(true);
            dom::focus(&parent);
            self.moving_focus.set(false);
        }
    }

    fn composition_start(&self, window: WindowId) {
        self.composing.set(true);
        post(HostEvent::Ime(window, ImeEvent::Enabled));
    }

    fn composition_update(&self, window: WindowId, text: String) {
        let end = text.len();
        post(HostEvent::Ime(window, ImeEvent::Preedit { text, cursor: Some(end..end) }));
    }

    fn composition_end(&self, window: WindowId, text: String) {
        self.composing.set(false);
        // The field now holds the committed text; the engine's next state replaces it.
        if let Some(s) = self.state.borrow_mut().as_mut() {
            s.surrounding = self.value(in_area(s));
            let caret = self.caret_byte(s);
            s.selection = caret..caret;
        }
        post(HostEvent::Ime(window, ImeEvent::Preedit { text: String::new(), cursor: None }));
        if !text.is_empty() {
            post(HostEvent::Ime(window, ImeEvent::Commit(text)));
        }
        post(HostEvent::Ime(window, ImeEvent::Disabled));
    }

    fn caret_byte(&self, s: &TextInputState) -> usize {
        let at = if !in_area(s) {
            self.line.selection_end().ok().flatten()
        } else {
            self.area.selection_end().ok().flatten()
        };
        convert::utf16_to_byte(&s.surrounding, at.unwrap_or(0))
    }

    /// Text that reached the field outside a composition (soft keyboards, dictation, the
    /// emoji panel): the difference from what it held becomes deletions and a commit.
    fn input(&self, window: WindowId, composing: bool) {
        if composing || self.composing.get() {
            return;
        }
        let edit = {
            let mut st = self.state.borrow_mut();
            let Some(s) = st.as_mut() else { return };
            let now = self.value(in_area(s));
            let caret = convert::clamp_range(&s.surrounding, &s.selection).end;
            let (before, after, inserted) = convert::edit_between(&s.surrounding, &now, caret);
            if s.private {
                // The engine keeps the secret; the hidden field goes back to holding none.
                self.set_value(false, "", (0, 0));
                s.selection = 0..0;
            } else {
                s.surrounding = now;
                let c = self.caret_byte(s);
                s.selection = c..c;
            }
            (before, after, inserted)
        };
        let (before, after, inserted) = edit;
        if before > 0 || after > 0 {
            post(HostEvent::Ime(window, ImeEvent::DeleteSurrounding { before, after }));
        }
        if !inserted.is_empty() {
            post(HostEvent::Ime(window, ImeEvent::Commit(inserted)));
        }
    }
}

/// The field edits in the `<textarea>` (several lines), not the `<input>`.
fn in_area(s: &TextInputState) -> bool {
    s.multiline && !s.private
}

/// The one-line input's `type`. Kinds without a selection (`email`, `number`) are left out:
/// the hidden field mirrors the caret, so it must be selectable; `inputmode` picks their keys.
fn input_type(s: &TextInputState) -> &'static str {
    match s.purpose {
        _ if s.private || s.purpose.is_password() => "password",
        InputPurpose::Url => "url",
        InputPurpose::Search => "search",
        _ => "text",
    }
}

/// What the hidden field tells soft keyboards and password managers.
struct Hints {
    /// `inputmode`, or none for the full keyboard.
    mode: Option<&'static str>,
    /// `autocomplete`.
    autocomplete: &'static str,
    /// `enterkeyhint`.
    enter: &'static str,
    /// Words: capitalizing, correcting and spell checking help.
    prose: bool,
}

impl Hints {
    fn of(s: &TextInputState) -> Hints {
        use InputPurpose as P;
        let mode = match s.purpose {
            P::Digits | P::OneTimeCode => Some("numeric"),
            P::Decimal => Some("decimal"),
            P::Url => Some("url"),
            P::Email => Some("email"),
            P::Search => Some("search"),
            _ if s.numeric => Some("decimal"),
            _ => None,
        };
        let autocomplete = match s.purpose {
            P::Username => "username",
            P::CurrentPassword => "current-password",
            P::NewPassword => "new-password",
            P::OneTimeCode => "one-time-code",
            P::Email => "email",
            _ => "off",
        };
        let enter = match s.enter_key() {
            EnterKey::Enter => "enter",
            EnterKey::Go => "go",
            EnterKey::Next => "next",
            EnterKey::Previous => "previous",
            EnterKey::Search => "search",
            EnterKey::Send => "send",
            _ => "done",
        };
        let prose = !s.private && matches!(s.purpose, P::Text | P::Search);
        Hints { mode, autocomplete, enter, prose }
    }
}

fn with(window: WindowId, f: impl FnOnce(&TextInput)) {
    let w: Option<Rc<super::window::Win>> = super::driver().and_then(|d| d.window(window));
    if let Some(w) = w {
        f(&w.text);
    }
}
