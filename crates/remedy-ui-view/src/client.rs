//! Elements that take text without being text fields: a terminal, a game
//! view, a canvas with editing of its own. They get what input methods compose and commit
//! and what the clipboard pastes, and they ask the host to copy and to paste.

use std::rc::Rc;

use remedy_ui_core::Rect;
use remedy_ui_host::{ImeEvent, TextInputState};
use remedy_ui_text::{Paragraph, ParagraphOptions, RichText, TextStyle};
use remedy_ui_tree::{Dirty, ElementId};

use crate::{El, Request, Ui};

/// Text reaching an element with [`El::on_text_input`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextInput {
    /// What an input method did: composition shown, text committed, composition ended.
    Ime(ImeEvent),
    /// The clipboard's text, after the element asked with [`Ui::request_paste`] or people
    /// pasted while it had focus.
    Paste(String),
}

/// The caret an input method places its candidate window at, in the element's own logical
/// pixels.
type CaretFn = Rc<dyn Fn() -> Rect>;

#[derive(Clone)]
pub(crate) struct TextClient {
    caret: CaretFn,
    input: Rc<dyn Fn(&TextInput)>,
}

impl El {
    /// Makes the element take text without being a field: while it has focus, input-method
    /// events and pasted text go to `f`, and the input method's candidate window sits at
    /// `caret` (element logical pixels). Keys still come through [`El::on_key`]. Makes the
    /// element focusable. Call [`El::text_input_moved`] when the caret moves.
    pub fn on_text_input(self, caret: impl Fn() -> Rect + 'static, f: impl Fn(&TextInput) + 'static) -> Self {
        let _ = self.ui.tree().set_focusable(self.id, true);
        let client = TextClient { caret: Rc::new(caret), input: Rc::new(f) };
        self.edit(Dirty::NONE, |d| d.text_client = Some(client));
        self
    }

    /// Tells the input method the caret moved, when this element takes text and has focus.
    pub fn text_input_moved(&self) {
        if self.ui.tree().focused() == Some(self.id)
            && let Some(s) = self.ui.client_state(self.id)
        {
            self.ui.request(Request::TextInput(Some(s)));
        }
    }
}

impl Ui {
    pub(crate) fn client_of(&self, id: ElementId) -> Option<TextClient> {
        self.0.borrow().els.get(&id).and_then(|d| d.text_client.clone())
    }

    /// The input-method session for element `id`, when it takes text.
    pub(crate) fn client_state(&self, id: ElementId) -> Option<TextInputState> {
        let client = self.client_of(id)?;
        let at = self.tree().bounds(id).unwrap_or(Rect::ZERO);
        let caret = (client.caret)().translate(at.min.to_vec2());
        // Nothing around the caret is shared: what a terminal shows is not the input
        // method's to read.
        Some({
            let mut v_ = TextInputState::default();
            v_.surrounding = String::new();
            v_.selection = 0..0;
            v_.caret = caret;
            v_.private = false;
            v_.multiline = true;
            v_
        })
    }

    /// Hands `input` to the focused element when it takes text; false when none does.
    pub(crate) fn to_client(&self, input: &TextInput) -> bool {
        let Some(f) = self.tree().focused() else { return false };
        let Some(client) = self.client_of(f) else { return false };
        (client.input)(input);
        true
    }

    /// Makes `text` the primary selection, where the platform has one.
    pub fn set_primary_selection(&self, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() {
            self.request(Request::SetPrimary(text));
        }
    }

    /// Asks the host for the primary selection (the middle button's paste); it arrives like
    /// a paste. Where the platform has none, `fallback` arrives instead.
    pub fn request_paste_primary(&self, fallback: Option<String>) {
        self.request(Request::PastePrimary { fallback });
    }

    /// Asks the host for the clipboard's text; it arrives at the focused field, or at the
    /// focused element that takes text ([`El::on_text_input`]).
    pub fn request_paste(&self) {
        self.request(Request::Paste);
    }

    /// Lays out `text` with the window's fonts, outside painting (measuring a cell, a label's
    /// width). `None` while a frame is being painted: painters use
    /// [`PaintCx::layout_text`](crate::PaintCx::layout_text).
    pub fn layout_text(
        &self,
        text: &RichText,
        style: &TextStyle,
        options: ParagraphOptions,
    ) -> Option<Paragraph> {
        let engine = self.0.try_borrow().ok()?.engine.clone();
        let mut engine = engine.try_borrow_mut().ok()?;
        Some(engine.layout(text, style, options))
    }
}
