//! Editable text: single-line, multi-line and secret fields.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Point, Rect};
use remedy_ui_host::{ImeEvent, TextInputState};
use remedy_ui_input::{Key, KeyEvent, Modifiers, Platform};
use remedy_ui_paint::{Brush, DisplayList};
use remedy_ui_style::{ResolvedStyle, Theme};
use remedy_ui_text::{
    Paragraph, ParagraphOptions, RichText, SecretEditor, SecretString, TextEditor, TextEngine, TextPosition,
    TextStyle,
};
use remedy_ui_tree::Dirty;

use crate::element::{El, Ui};
use crate::{Request, frame};

/// What kind of field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// One line; Enter submits.
    Single,
    /// Several lines; Enter breaks the line.
    Multi,
    /// A password or other secret: masked, kept in a zeroizing buffer, never copied, no input
    /// method.
    Secret,
}

enum Editor {
    Plain(TextEditor),
    Secret(SecretEditor),
}

/// The mask shown for each character of a secret.
const MASK: char = '•';

pub(crate) struct Field {
    kind: FieldKind,
    editor: Editor,
    style: TextStyle,
    placeholder: String,
    scale: f32,
    /// The paragraph shown, its width and where its top-left sat in the element.
    shown: Option<(f32, Rc<Paragraph>, Point)>,
    on_change: Option<Rc<dyn Fn()>>,
    on_submit: Option<Rc<dyn Fn()>>,
    version: u64,
}

impl Field {
    pub(crate) fn invalidate(&mut self) {
        self.shown = None;
    }

    pub(crate) fn set_style(&mut self, style: TextStyle) {
        self.style = style;
        self.shown = None;
    }

    fn display(&self) -> (String, Option<Range<usize>>) {
        match &self.editor {
            Editor::Plain(e) => e.display_text(),
            Editor::Secret(e) => (MASK.to_string().repeat(e.masked_len()), None),
        }
    }

    /// The selection in display-text bytes.
    fn display_selection(&self) -> Range<usize> {
        match &self.editor {
            Editor::Plain(e) => {
                let s = e.selection();
                s.anchor.min(s.focus)..s.anchor.max(s.focus)
            }
            Editor::Secret(e) => {
                // Masked text: every character is one MASK, so map character counts.
                let s = e.selection();
                let chars = |b: usize| e.secret().expose().get(..b).map_or(0, |p| p.chars().count());
                let (a, f) = (chars(s.anchor) * MASK.len_utf8(), chars(s.focus) * MASK.len_utf8());
                a.min(f)..a.max(f)
            }
        }
    }

    fn caret_index(&self) -> usize {
        match &self.editor {
            Editor::Plain(e) => {
                let (_, comp) = e.display_text();
                match (comp, e.composition()) {
                    (Some(r), Some(c)) => r.start + c.cursor.min(r.len()),
                    _ => e.selection().focus,
                }
            }
            Editor::Secret(e) => {
                let s = e.selection();
                e.secret().expose().get(..s.focus).map_or(0, |p| p.chars().count()) * MASK.len_utf8()
            }
        }
    }

    fn paragraph(&mut self, engine: &mut TextEngine, width: f32) -> Rc<Paragraph> {
        if let Some((w, p, _)) = &self.shown
            && *w == width
        {
            return p.clone();
        }
        let (text, _) = self.display();
        let wrap = (self.kind == FieldKind::Multi).then_some(width);
        let options = ParagraphOptions { max_width: wrap, scale: self.scale, ..ParagraphOptions::default() };
        let shown = if text.is_empty() { " ".to_owned() } else { text };
        let p = Rc::new(engine.layout(&RichText::plain(shown), &self.style, options));
        self.shown = Some((width, p.clone(), Point::ZERO));
        p
    }

    pub(crate) fn paint(
        &mut self,
        list: &mut DisplayList,
        content: Rect,
        s: &ResolvedStyle,
        theme: &Theme,
        engine: &mut TextEngine,
        focused: bool,
    ) {
        let p = self.paragraph(engine, content.width().max(1.0));
        if let Some(shown) = self.shown.as_mut() {
            shown.2 = content.min;
        }
        let o = content.min;
        let tokens = theme.derive();
        let (text, comp) = self.display();
        if focused {
            for r in p.selection_rects(self.display_selection()) {
                list.fill_rect(r.translate(o.to_vec2()), Brush::Solid(tokens.selected));
            }
        }
        if text.is_empty() {
            if !self.placeholder.is_empty() {
                let style = TextStyle { color: tokens.muted, ..self.style.clone() };
                let ph = engine.layout(
                    &RichText::plain(self.placeholder.clone()),
                    &style,
                    ParagraphOptions { scale: self.scale, ..ParagraphOptions::default() },
                );
                frame::glyphs(list, &ph, o, tokens.muted);
            }
        } else {
            frame::glyphs(list, &p, o, s.foreground);
        }
        if let Some(c) = comp {
            // The composition is underlined, as every platform shows input-method text.
            for r in p.selection_rects(c) {
                let y = r.max.y - 1.0;
                list.fill_rect(
                    Rect::new(r.min.x, y, r.max.x, y + 1.0).translate(o.to_vec2()),
                    Brush::Solid(s.foreground),
                );
            }
        }
        if focused {
            let at = TextPosition { index: self.caret_index().min(text.len()), ..TextPosition::default() };
            let caret = p.caret(at, 1.5);
            list.fill_rect(caret.translate(o.to_vec2()), Brush::Solid(tokens.text));
        }
    }

    pub(crate) fn semantics(&self) -> Semantics {
        let s = Semantics::new(match self.kind {
            FieldKind::Secret => Role::PasswordInput,
            FieldKind::Multi => Role::MultilineTextInput,
            FieldKind::Single => Role::TextInput,
        });
        match &self.editor {
            Editor::Plain(e) => s.value(e.text()),
            // Secrets are never exposed, not even their length.
            Editor::Secret(_) => s,
        }
    }

    /// The input-method session for this field, with its caret in window coordinates.
    pub(crate) fn input_state(&self, bounds: Rect) -> TextInputState {
        let caret = self
            .shown
            .as_ref()
            .map(|(_, p, o)| {
                p.caret(TextPosition { index: self.caret_index(), ..TextPosition::default() }, 1.0)
                    .translate(o.to_vec2())
            })
            .unwrap_or(Rect::ZERO)
            .translate(bounds.min.to_vec2());
        match &self.editor {
            Editor::Plain(e) => {
                let s = e.selection();
                TextInputState {
                    surrounding: e.text(),
                    selection: s.anchor.min(s.focus)..s.anchor.max(s.focus),
                    caret,
                    private: false,
                    multiline: self.kind == FieldKind::Multi,
                }
            }
            Editor::Secret(_) => TextInputState {
                surrounding: String::new(),
                selection: 0..0,
                caret,
                private: true,
                multiline: false,
            },
        }
    }

    /// Places the caret at a point in element coordinates.
    pub(crate) fn place(&mut self, at: Point, extend: bool) {
        let Some((_, p, o)) = &self.shown else { return };
        let pos = p.hit_test(Point::new(at.x - o.x, at.y - o.y)).index;
        match &mut self.editor {
            Editor::Plain(e) => {
                let anchor = if extend { e.selection().anchor } else { pos };
                e.set_selection(anchor, pos);
            }
            Editor::Secret(e) => {
                // Display bytes to secret bytes: count masks, walk that many characters.
                let n = pos / MASK.len_utf8();
                let idx = e.secret().expose().char_indices().nth(n).map_or(e.secret().len(), |(i, _)| i);
                let sel = e.selection();
                let anchor = if extend { sel.anchor } else { idx };
                e.set_selection(anchor, idx);
            }
        }
    }

    fn changed(&mut self) {
        self.shown = None;
        self.version += 1;
    }
}

/// What a key did to a field.
pub(crate) enum KeyOutcome {
    /// Not a field key: let it bubble (Tab, Escape, shortcuts).
    Ignored,
    /// Handled; the field may have changed.
    Handled { changed: bool },
    /// Enter in a single-line field.
    Submit,
    /// Copy this text out.
    Copy(String),
    /// Read the clipboard.
    Paste,
}

pub(crate) fn key(f: &mut Field, e: &KeyEvent, now: u64, platform: Platform) -> KeyOutcome {
    if !e.pressed || e.composing {
        return KeyOutcome::Ignored;
    }
    let primary = Modifiers::primary(platform);
    let cmd = e.modifiers.ctrl == primary.ctrl
        && e.modifiers.meta == primary.meta
        && (e.modifiers.ctrl || e.modifiers.meta);
    let extend = e.modifiers.shift;
    let word = if platform == Platform::Apple { e.modifiers.alt } else { e.modifiers.ctrl };
    let ch = |c: &str| matches!(&e.key, Key::Character(k) if k == c);
    let handled = || KeyOutcome::Handled { changed: false };
    let edited = || KeyOutcome::Handled { changed: true };
    match &mut f.editor {
        Editor::Plain(ed) => {
            let out = match &e.key {
                _ if cmd && ch("a") => {
                    ed.select_all();
                    handled()
                }
                _ if cmd && ch("c") => KeyOutcome::Copy(ed.selected_text()),
                _ if cmd && ch("x") => KeyOutcome::Copy(ed.cut(now)),
                _ if cmd && ch("v") => KeyOutcome::Paste,
                _ if cmd && ch("z") && !extend => KeyOutcome::Handled { changed: ed.undo() },
                _ if cmd && (ch("y") || (ch("z") && extend)) => KeyOutcome::Handled { changed: ed.redo() },
                Key::Backspace if word => {
                    ed.delete_word(false, now);
                    edited()
                }
                Key::Backspace => {
                    ed.backspace(now);
                    edited()
                }
                Key::Delete if word => {
                    ed.delete_word(true, now);
                    edited()
                }
                Key::Delete => {
                    ed.delete_forward(now);
                    edited()
                }
                Key::ArrowLeft | Key::ArrowRight if word => {
                    ed.move_word(e.key == Key::ArrowRight, extend);
                    handled()
                }
                Key::ArrowLeft | Key::ArrowRight => {
                    ed.move_char(e.key == Key::ArrowRight, extend);
                    handled()
                }
                Key::Home | Key::End => {
                    ed.move_to_edge(e.key == Key::End, extend);
                    handled()
                }
                Key::Enter if f.kind == FieldKind::Multi => {
                    ed.insert("\n", now);
                    edited()
                }
                Key::Enter => KeyOutcome::Submit,
                _ if cmd || e.modifiers.alt && !e.modifiers.ctrl => KeyOutcome::Ignored,
                _ => match &e.text {
                    Some(t) if !t.is_empty() => {
                        ed.insert(t, now);
                        edited()
                    }
                    _ => KeyOutcome::Ignored,
                },
            };
            if matches!(out, KeyOutcome::Copy(_)) && f.kind == FieldKind::Secret {
                return handled();
            }
            out
        }
        Editor::Secret(ed) => match &e.key {
            _ if cmd && ch("a") => {
                ed.select_all();
                handled()
            }
            // Secrets are never copied or cut out of the field.
            _ if cmd && (ch("c") || ch("x")) => handled(),
            _ if cmd && ch("v") => KeyOutcome::Paste,
            Key::Backspace => {
                ed.backspace();
                edited()
            }
            Key::Delete => {
                ed.delete_forward();
                edited()
            }
            Key::ArrowLeft | Key::ArrowRight => {
                ed.move_char(e.key == Key::ArrowRight, extend);
                handled()
            }
            Key::Enter => KeyOutcome::Submit,
            _ if cmd || e.modifiers.alt && !e.modifiers.ctrl => KeyOutcome::Ignored,
            _ => match &e.text {
                Some(t) if !t.is_empty() => {
                    ed.insert(t);
                    edited()
                }
                _ => KeyOutcome::Ignored,
            },
        },
    }
}

pub(crate) fn ime(f: &mut Field, e: &ImeEvent, now: u64) -> bool {
    let Editor::Plain(ed) = &mut f.editor else { return false };
    match e {
        ImeEvent::Preedit { text, cursor } => {
            if text.is_empty() {
                ed.cancel_composition();
            } else {
                ed.set_composition(text, cursor.as_ref().map_or(text.len(), |c| c.end));
            }
        }
        ImeEvent::Commit(t) => ed.commit_composition(t, now),
        ImeEvent::DeleteSurrounding { before, after } => {
            let s = ed.selection().focus;
            let (a, b) = (s.saturating_sub(*before), (s + after).min(ed.len()));
            if a < b {
                ed.set_selection(a, b);
                ed.backspace(now);
            }
        }
        ImeEvent::Enabled | ImeEvent::Disabled => return false,
    }
    true
}

pub(crate) fn paste(f: &mut Field, text: &str, now: u64) {
    let text = if f.kind == FieldKind::Multi { text.to_owned() } else { text.replace(['\r', '\n'], " ") };
    match &mut f.editor {
        Editor::Plain(e) => e.paste(&text, now),
        Editor::Secret(e) => e.insert(&text),
    }
}

/// A handle to a text field element.
#[derive(Clone)]
pub struct TextField {
    el: El,
    field: Rc<RefCell<Field>>,
}

impl std::fmt::Debug for TextField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextField({:?})", self.field.borrow().kind)
    }
}

impl Ui {
    /// A text field element of `kind`, styled as `widget`.
    pub fn text_field(&self, widget: &'static str, kind: FieldKind) -> TextField {
        let el = self.el(widget).focusable(true);
        let scale = self.0.borrow().scale;
        let field = Rc::new(RefCell::new(Field {
            kind,
            editor: match kind {
                FieldKind::Secret => Editor::Secret(SecretEditor::new()),
                _ => Editor::Plain(TextEditor::new("")),
            },
            style: TextStyle::default(),
            placeholder: String::new(),
            scale,
            shown: None,
            on_change: None,
            on_submit: None,
            version: 0,
        }));
        // Measured like text: the paragraph's size plus one caret width.
        let engine = self.0.borrow().engine.clone();
        let f2 = field.clone();
        let measure = move |m: remedy_ui_layout::MeasureInput| {
            let width = m.known_width.or(match m.available_width {
                remedy_ui_layout::Available::Px(w) => Some(w),
                _ => None,
            });
            let mut f = f2.borrow_mut();
            let w = width.unwrap_or(f32::MAX / 4.0);
            let wrap = if f.kind == FieldKind::Multi { w } else { f32::MAX / 4.0 };
            let p = f.paragraph(&mut engine.borrow_mut(), wrap);
            remedy_ui_core::Size::new(width.unwrap_or(p.width().ceil() + 2.0), p.height().ceil())
        };
        {
            let mut i = self.0.borrow_mut();
            let style = i.els.get(&el.id).map(|d| d.layout.clone()).unwrap_or_default();
            if let Ok(node) = i.layout.measured(&style, measure) {
                if let Some(old) = i.els.get(&el.id).map(|d| d.node) {
                    let _ = i.layout.remove(old);
                }
                if let Some(d) = i.els.get_mut(&el.id) {
                    d.node = node;
                    d.field = Some(field.clone());
                }
            }
        }
        el.ui.mark(el.id, Dirty::STYLE | Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        TextField { el, field }
    }
}

impl TextField {
    /// The element, to style and place.
    pub fn el(&self) -> El {
        self.el.clone()
    }

    /// The kind of field.
    pub fn kind(&self) -> FieldKind {
        self.field.borrow().kind
    }

    /// The text (empty for secret fields; use [`TextField::take_secret`]).
    pub fn value(&self) -> String {
        match &self.field.borrow().editor {
            Editor::Plain(e) => e.text(),
            Editor::Secret(_) => String::new(),
        }
    }

    /// How many characters a secret field holds (the mask length), or the text length.
    pub fn len(&self) -> usize {
        match &self.field.borrow().editor {
            Editor::Plain(e) => e.text().chars().count(),
            Editor::Secret(e) => e.masked_len(),
        }
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Replaces the text (plain fields), caret at the end, starting a fresh undo history.
    pub fn set_value(&self, text: &str) {
        let mut f = self.field.borrow_mut();
        if let Editor::Plain(e) = &mut f.editor {
            *e = TextEditor::new(text);
            e.move_to_edge(true, false);
        }
        f.changed();
        drop(f);
        self.el.ui.mark(self.el.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
    }

    /// Takes a secret field's contents out, leaving it empty.
    pub fn take_secret(&self) -> SecretString {
        let mut f = self.field.borrow_mut();
        let out = match &mut f.editor {
            Editor::Secret(e) => e.take(),
            Editor::Plain(e) => {
                let mut s = SecretString::new();
                s.insert(0, &e.text());
                s
            }
        };
        f.changed();
        drop(f);
        self.el.ui.mark(self.el.id, Dirty::LAYOUT | Dirty::PAINT);
        out
    }

    /// Grey text shown while the field is empty.
    pub fn placeholder(self, text: impl Into<String>) -> Self {
        self.field.borrow_mut().placeholder = text.into();
        self.el.ui.mark(self.el.id, Dirty::PAINT);
        self
    }

    /// Called after every edit.
    pub fn on_change(self, f: impl Fn() + 'static) -> Self {
        self.field.borrow_mut().on_change = Some(Rc::new(f));
        self
    }

    /// Called when Enter is pressed in a single-line or secret field.
    pub fn on_submit(self, f: impl Fn() + 'static) -> Self {
        self.field.borrow_mut().on_submit = Some(Rc::new(f));
        self
    }
}

pub(crate) fn notify_change(field: &Rc<RefCell<Field>>) {
    let cb = field.borrow().on_change.clone();
    if let Some(cb) = cb {
        cb();
    }
}

pub(crate) fn notify_submit(field: &Rc<RefCell<Field>>) {
    let cb = field.borrow().on_submit.clone();
    if let Some(cb) = cb {
        cb();
    }
}

pub(crate) fn mark_changed(field: &Rc<RefCell<Field>>) {
    field.borrow_mut().changed();
}

pub(crate) fn input_state(field: &Rc<RefCell<Field>>, bounds: Rect) -> TextInputState {
    field.borrow().input_state(bounds)
}

pub(crate) fn request_session(ui: &Ui, field: &Rc<RefCell<Field>>, bounds: Rect) {
    ui.request(Request::TextInput(Some(input_state(field, bounds))));
}
