//! The editing core shared by text inputs and rich editors.
//!
//! [`TextEditor`] holds text in a rope (so a large document edits in time proportional to the
//! change), a selection, an IME composition and an undo history. Every boundary it moves over
//! is a grapheme cluster (a user-perceived character, so an emoji with modifiers or a letter
//! with combining marks moves and deletes as one), and word motions use the Unicode word
//! rules. Visual (left/right on screen) movement through mixed-direction text lives on the
//! laid-out [`crate::Paragraph`]; the editor's motions are logical.
//!
//! [`SecretString`] and [`SecretEditor`] are for passwords: memory is zeroed whenever it is
//! released or replaced, nothing is kept for undo, and `Debug` never shows the contents.

use std::ops::Range;

use icu_segmenter::options::WordBreakInvariantOptions;
use icu_segmenter::{GraphemeClusterSegmenter, WordSegmenter};
use ropey::Rope;
use zeroize::Zeroize;

/// How much text around a position is read to find grapheme and word boundaries. A tuning
/// constant: longer than any real grapheme cluster or word, short enough to keep editing in
/// a huge document fast.
const BOUNDARY_WINDOW: usize = 1024;

/// Typing pauses longer than this (in milliseconds) start a new undo step.
pub const UNDO_PAUSE_MS: u64 = 1000;

/// A selection: the `anchor` stays put while the `focus` moves with Shift+arrows. Equal ends
/// are a caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    /// Where the selection started (byte index).
    pub anchor: usize,
    /// Where it ends and the caret is (byte index).
    pub focus: usize,
}

impl TextSelection {
    /// A caret at `at`.
    pub fn caret(at: usize) -> Self {
        TextSelection { anchor: at, focus: at }
    }

    /// The selected bytes, in order.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }

    /// True when nothing is selected.
    pub fn is_caret(&self) -> bool {
        self.anchor == self.focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Clone, Debug)]
struct UndoStep {
    at: usize,
    removed: String,
    inserted: String,
    before: TextSelection,
    after: TextSelection,
}

/// Text being composed by an input method (IME): shown inline, not yet part of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    /// Where the composition sits in the text (byte index; it replaces the selection there).
    pub at: usize,
    /// The composing text.
    pub text: String,
    /// The IME's caret within the composing text (byte index into `text`).
    pub cursor: usize,
}

/// An editable text with selection, IME composition and undo.
#[derive(Clone, Debug, Default)]
pub struct TextEditor {
    rope: Rope,
    sel: TextSelection,
    composition: Option<Composition>,
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    last: Option<(EditKind, u64)>,
    version: u64,
}

impl TextEditor {
    /// An editor holding `text`, with the caret at the end.
    pub fn new(text: &str) -> Self {
        let rope = Rope::from_str(text);
        let end = rope.len_bytes();
        TextEditor { rope, sel: TextSelection::caret(end), ..Self::default() }
    }

    /// The text.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The text's length in bytes.
    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    /// True when there is no text.
    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// Bumped on every change to the text, for change detection.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The selection.
    pub fn selection(&self) -> TextSelection {
        self.sel
    }

    /// The selected text.
    pub fn selected_text(&self) -> String {
        self.slice(self.sel.range())
    }

    /// Text in a byte range (clamped to character boundaries).
    pub fn slice(&self, r: Range<usize>) -> String {
        let s = self.char_floor(r.start);
        let e = self.char_floor(r.end.max(r.start));
        let (cs, ce) = (self.rope.byte_to_char(s), self.rope.byte_to_char(e));
        self.rope.slice(cs..ce).to_string()
    }

    /// Sets the selection, moving each end to the nearest grapheme boundary at or before it.
    pub fn set_selection(&mut self, anchor: usize, focus: usize) {
        self.sel = TextSelection { anchor: self.grapheme_floor(anchor), focus: self.grapheme_floor(focus) };
        self.last = None;
    }

    /// Selects everything.
    pub fn select_all(&mut self) {
        self.sel = TextSelection { anchor: 0, focus: self.len() };
        self.last = None;
    }

    // ---------- boundaries ----------

    fn char_floor(&self, i: usize) -> usize {
        let i = i.min(self.len());
        let c = self.rope.byte_to_char(i);
        self.rope.char_to_byte(c)
    }

    /// The text around `at`, with the window's start byte.
    fn window(&self, at: usize) -> (usize, String) {
        let start = self.char_floor(at.saturating_sub(BOUNDARY_WINDOW));
        let end = self.char_floor(at.saturating_add(BOUNDARY_WINDOW));
        (start, self.slice(start..end))
    }

    fn grapheme_breaks(&self, at: usize) -> (usize, Vec<usize>) {
        let (start, text) = self.window(at);
        let breaks: Vec<usize> =
            GraphemeClusterSegmenter::new().segment_str(&text).map(|b| b + start).collect();
        (start, breaks)
    }

    fn grapheme_floor(&self, at: usize) -> usize {
        let at = self.char_floor(at);
        let (_, breaks) = self.grapheme_breaks(at);
        breaks.iter().copied().filter(|&b| b <= at).max().unwrap_or(0)
    }

    /// The grapheme boundary before `at` (or 0).
    pub fn previous_grapheme(&self, at: usize) -> usize {
        let (_, breaks) = self.grapheme_breaks(at);
        breaks.iter().copied().filter(|&b| b < at).max().unwrap_or(0)
    }

    /// The grapheme boundary after `at` (or the end).
    pub fn next_grapheme(&self, at: usize) -> usize {
        let (_, breaks) = self.grapheme_breaks(at);
        breaks.iter().copied().filter(|&b| b > at).min().unwrap_or(self.len()).min(self.len())
    }

    /// Word segments around `at`: (start, end, is a word).
    fn word_segments(&self, at: usize) -> Vec<(usize, usize, bool)> {
        let (start, text) = self.window(at);
        let seg = WordSegmenter::new_for_non_complex_scripts(WordBreakInvariantOptions::default());
        let mut it = seg.segment_str(&text);
        let mut out = Vec::new();
        let mut prev = None;
        while let Some(b) = it.next() {
            if let Some(p) = prev {
                out.push((p + start, b + start, it.is_word_like()));
            }
            prev = Some(b);
        }
        out
    }

    /// The start of the word before `at`, skipping spaces and punctuation first (Ctrl+Left).
    pub fn previous_word(&self, at: usize) -> usize {
        self.word_segments(at)
            .into_iter()
            .filter(|&(s, _, word)| word && s < at)
            .map(|(s, _, _)| s)
            .max()
            .unwrap_or(0)
    }

    /// The end of the word after `at`, skipping spaces and punctuation first (Ctrl+Right).
    pub fn next_word(&self, at: usize) -> usize {
        self.word_segments(at)
            .into_iter()
            .filter(|&(_, e, word)| word && e > at)
            .map(|(_, e, _)| e)
            .min()
            .unwrap_or(self.len())
    }

    /// The word around `at`, for double-click.
    pub fn word_at(&self, at: usize) -> Range<usize> {
        self.word_segments(at)
            .into_iter()
            .find(|&(s, e, _)| s <= at && at < e)
            .map(|(s, e, _)| s..e)
            .unwrap_or(at..at)
    }

    // ---------- movement ----------

    fn move_to(&mut self, to: usize, extend: bool) {
        self.sel = if extend {
            TextSelection { anchor: self.sel.anchor, focus: to }
        } else {
            TextSelection::caret(to)
        };
        self.last = None;
    }

    /// Left/Right (logical): one grapheme, or collapse a selection to its edge.
    pub fn move_char(&mut self, forward: bool, extend: bool) {
        let to = if !extend && !self.sel.is_caret() {
            if forward { self.sel.range().end } else { self.sel.range().start }
        } else if forward {
            self.next_grapheme(self.sel.focus)
        } else {
            self.previous_grapheme(self.sel.focus)
        };
        self.move_to(to, extend);
    }

    /// Ctrl+Left/Right (logical): one word.
    pub fn move_word(&mut self, forward: bool, extend: bool) {
        let to = if forward { self.next_word(self.sel.focus) } else { self.previous_word(self.sel.focus) };
        self.move_to(to, extend);
    }

    /// Ctrl+Home / Ctrl+End.
    pub fn move_to_edge(&mut self, end: bool, extend: bool) {
        let to = if end { self.len() } else { 0 };
        self.move_to(to, extend);
    }

    // ---------- editing ----------

    fn apply(&mut self, at: usize, remove: usize, insert: &str, kind: EditKind, now_ms: u64) {
        let before = self.sel;
        let end = self.char_floor(at + remove);
        let removed = self.slice(at..end);
        let (cs, ce) = (self.rope.byte_to_char(at), self.rope.byte_to_char(end));
        self.rope.remove(cs..ce);
        self.rope.insert(cs, insert);
        let caret = at + insert.len();
        self.sel = TextSelection::caret(caret);
        self.version += 1;
        self.redo.clear();
        let step = UndoStep { at, removed, inserted: insert.to_string(), before, after: self.sel };
        // Typing (or deleting) in one place without a long pause joins the previous step,
        // except that typing a space after a word starts a new one.
        let joins = match (self.last, self.undo.last()) {
            (Some((k, t)), Some(prev))
                if k == kind && kind != EditKind::Other && now_ms.saturating_sub(t) < UNDO_PAUSE_MS =>
            {
                let contiguous = match kind {
                    EditKind::Typing => prev.at + prev.inserted.len() == at && step.removed.is_empty(),
                    EditKind::Deleting => {
                        step.inserted.is_empty() && (at + step.removed.len() == prev.at || at == prev.at)
                    }
                    EditKind::Other => false,
                };
                let word_break = kind == EditKind::Typing
                    && insert.chars().all(char::is_whitespace)
                    && !prev.inserted.chars().last().is_some_and(char::is_whitespace);
                contiguous && !word_break
            }
            _ => false,
        };
        if joins && let Some(prev) = self.undo.last_mut() {
            match kind {
                EditKind::Typing => prev.inserted.push_str(&step.inserted),
                EditKind::Deleting => {
                    if at + step.removed.len() == prev.at {
                        // Backspace: the removed text comes before what was removed already.
                        prev.removed.insert_str(0, &step.removed);
                        prev.at = at;
                    } else {
                        prev.removed.push_str(&step.removed);
                    }
                }
                EditKind::Other => {}
            }
            prev.after = step.after;
        } else {
            self.undo.push(step);
        }
        self.last = Some((kind, now_ms));
    }

    /// Types `s`, replacing the selection. `now_ms` is the time of the key press, used to
    /// group typing into undo steps.
    pub fn insert(&mut self, s: &str, now_ms: u64) {
        let r = self.sel.range();
        let kind = if r.is_empty() && s.chars().count() <= 1 { EditKind::Typing } else { EditKind::Other };
        self.apply(r.start, r.len(), s, kind, now_ms);
    }

    /// Pastes `s`: one undo step, replacing the selection.
    pub fn paste(&mut self, s: &str, now_ms: u64) {
        let r = self.sel.range();
        self.apply(r.start, r.len(), s, EditKind::Other, now_ms);
    }

    /// Backspace: deletes the selection, or the grapheme before the caret.
    pub fn backspace(&mut self, now_ms: u64) {
        let r = self.sel.range();
        if !r.is_empty() {
            return self.apply(r.start, r.len(), "", EditKind::Other, now_ms);
        }
        let start = self.previous_grapheme(r.start);
        if start < r.start {
            self.apply(start, r.start - start, "", EditKind::Deleting, now_ms);
        }
    }

    /// Delete: deletes the selection, or the grapheme after the caret.
    pub fn delete_forward(&mut self, now_ms: u64) {
        let r = self.sel.range();
        if !r.is_empty() {
            return self.apply(r.start, r.len(), "", EditKind::Other, now_ms);
        }
        let end = self.next_grapheme(r.start);
        if end > r.start {
            self.apply(r.start, end - r.start, "", EditKind::Deleting, now_ms);
        }
    }

    /// Ctrl+Backspace / Ctrl+Delete: deletes to the previous (or next) word boundary.
    pub fn delete_word(&mut self, forward: bool, now_ms: u64) {
        let r = self.sel.range();
        if !r.is_empty() {
            return self.apply(r.start, r.len(), "", EditKind::Other, now_ms);
        }
        let (s, e) =
            if forward { (r.start, self.next_word(r.start)) } else { (self.previous_word(r.start), r.start) };
        if s < e {
            self.apply(s, e - s, "", EditKind::Other, now_ms);
        }
    }

    /// Removes and returns the selected text (Cut).
    pub fn cut(&mut self, now_ms: u64) -> String {
        let text = self.selected_text();
        let r = self.sel.range();
        if !r.is_empty() {
            self.apply(r.start, r.len(), "", EditKind::Other, now_ms);
        }
        text
    }

    /// Undoes the last step. Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else { return false };
        let (cs, ce) = (
            self.rope.byte_to_char(step.at),
            self.rope.byte_to_char(self.char_floor(step.at + step.inserted.len())),
        );
        self.rope.remove(cs..ce);
        self.rope.insert(cs, &step.removed);
        self.sel = step.before;
        self.version += 1;
        self.last = None;
        self.redo.push(step);
        true
    }

    /// Redoes the last undone step. Returns false when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else { return false };
        let (cs, ce) = (
            self.rope.byte_to_char(step.at),
            self.rope.byte_to_char(self.char_floor(step.at + step.removed.len())),
        );
        self.rope.remove(cs..ce);
        self.rope.insert(cs, &step.inserted);
        self.sel = step.after;
        self.version += 1;
        self.last = None;
        self.undo.push(step);
        true
    }

    /// True when there is a step to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True when there is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // ---------- input methods ----------

    /// Starts or updates an IME composition at the selection. The text is not changed until
    /// it is committed.
    pub fn set_composition(&mut self, text: &str, cursor: usize) {
        let at = self.composition.as_ref().map_or(self.sel.range().start, |c| c.at);
        let mut cursor = cursor.min(text.len());
        while !text.is_char_boundary(cursor) {
            cursor -= 1;
        }
        self.composition = Some(Composition { at, text: text.to_string(), cursor });
    }

    /// The current composition, if any.
    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    /// Commits `text` from the IME, replacing the selection, as one undo step.
    pub fn commit_composition(&mut self, text: &str, now_ms: u64) {
        self.composition = None;
        self.paste(text, now_ms);
    }

    /// Cancels the composition; the text is unchanged.
    pub fn cancel_composition(&mut self) {
        self.composition = None;
    }

    /// The text as shown: the composition, if any, in place of the selection, with the range
    /// it occupies (for its underline).
    pub fn display_text(&self) -> (String, Option<Range<usize>>) {
        match &self.composition {
            None => (self.text(), None),
            Some(c) => {
                let r = self.sel.range();
                let mut s = self.slice(0..c.at.min(r.start));
                let start = s.len();
                s.push_str(&c.text);
                let end = s.len();
                s.push_str(&self.slice(r.end.max(c.at)..self.len()));
                (s, Some(start..end))
            }
        }
    }
}

/// A string for secrets. Its memory is zeroed when it is dropped and whenever it grows into
/// a new allocation; `Debug` shows only its length.
#[derive(Default)]
pub struct SecretString {
    bytes: Vec<u8>,
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretString({} bytes)", self.bytes.len())
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl SecretString {
    /// An empty secret.
    pub fn new() -> Self {
        Self::default()
    }

    /// The secret's text. Use it, then let it go; do not copy it into ordinary strings.
    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.bytes).unwrap_or_default()
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Makes room for `more` bytes. When the buffer must move, the old one is zeroed before
    /// it is freed, so no copy of the secret is left behind.
    fn reserve(&mut self, more: usize) {
        let need = self.bytes.len() + more;
        if need <= self.bytes.capacity() {
            return;
        }
        let mut grown = Vec::with_capacity(need.max(self.bytes.capacity() * 2).max(32));
        grown.extend_from_slice(&self.bytes);
        self.bytes.zeroize();
        self.bytes = grown;
    }

    /// Inserts `s` at byte `at` (clamped to a character boundary).
    pub fn insert(&mut self, at: usize, s: &str) {
        let at = self.floor(at);
        self.reserve(s.len());
        let tail: Vec<u8> = self.bytes.split_off(at);
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.extend_from_slice(&tail);
        let mut tail = tail;
        tail.zeroize();
    }

    /// Removes a byte range (clamped to character boundaries).
    pub fn remove(&mut self, r: Range<usize>) {
        let (s, e) = (self.floor(r.start), self.floor(r.end.max(r.start)));
        let mut tail = self.bytes.split_off(e);
        self.bytes.truncate(s);
        // `truncate` keeps the removed bytes in the allocation; overwrite them.
        let spare = self.bytes.spare_capacity_mut();
        for b in spare.iter_mut() {
            b.write(0);
        }
        self.bytes.extend_from_slice(&tail);
        tail.zeroize();
    }

    /// Empties the secret, zeroing its memory.
    pub fn clear(&mut self) {
        self.bytes.zeroize();
    }

    /// Takes the secret out, leaving this one empty.
    pub fn take(&mut self) -> SecretString {
        std::mem::take(self)
    }

    fn floor(&self, i: usize) -> usize {
        let s = self.expose();
        let mut i = i.min(s.len());
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// Editing for password fields: insert, delete and move by grapheme, with nothing kept for
/// undo and the text held only in a [`SecretString`].
#[derive(Debug, Default)]
pub struct SecretEditor {
    secret: SecretString,
    sel: TextSelection,
}

impl SecretEditor {
    /// An empty secret editor.
    pub fn new() -> Self {
        Self::default()
    }

    /// The secret.
    pub fn secret(&self) -> &SecretString {
        &self.secret
    }

    /// Takes the secret out (for example to unlock a vault), leaving the editor empty.
    pub fn take(&mut self) -> SecretString {
        self.sel = TextSelection::default();
        self.secret.take()
    }

    /// The selection.
    pub fn selection(&self) -> TextSelection {
        self.sel
    }

    /// How many characters to show as dots.
    pub fn masked_len(&self) -> usize {
        GraphemeClusterSegmenter::new().segment_str(self.secret.expose()).count().saturating_sub(1)
    }

    fn breaks(&self) -> Vec<usize> {
        GraphemeClusterSegmenter::new().segment_str(self.secret.expose()).collect()
    }

    /// Types (or pastes) `s`, replacing the selection.
    pub fn insert(&mut self, s: &str) {
        let r = self.sel.range();
        self.secret.remove(r.clone());
        self.secret.insert(r.start, s);
        self.sel = TextSelection::caret(r.start + s.len());
    }

    /// Backspace.
    pub fn backspace(&mut self) {
        let r = self.sel.range();
        let start = if r.is_empty() {
            self.breaks().into_iter().filter(|&b| b < r.start).max().unwrap_or(0)
        } else {
            r.start
        };
        self.secret.remove(start..r.end);
        self.sel = TextSelection::caret(start);
    }

    /// Delete.
    pub fn delete_forward(&mut self) {
        let r = self.sel.range();
        let end = if r.is_empty() {
            self.breaks().into_iter().filter(|&b| b > r.start).min().unwrap_or(self.secret.len())
        } else {
            r.end
        };
        self.secret.remove(r.start..end);
        self.sel = TextSelection::caret(r.start);
    }

    /// Left/Right by grapheme.
    pub fn move_char(&mut self, forward: bool, extend: bool) {
        let at = self.sel.focus;
        let to = if forward {
            self.breaks().into_iter().filter(|&b| b > at).min().unwrap_or(self.secret.len())
        } else {
            self.breaks().into_iter().filter(|&b| b < at).max().unwrap_or(0)
        };
        self.sel = if extend {
            TextSelection { anchor: self.sel.anchor, focus: to }
        } else {
            TextSelection::caret(to)
        };
    }

    /// Selects everything.
    pub fn select_all(&mut self) {
        self.sel = TextSelection { anchor: 0, focus: self.secret.len() };
    }

    /// Sets the selection (byte indices, snapped down to character boundaries).
    pub fn set_selection(&mut self, anchor: usize, focus: usize) {
        let s = self.secret.expose();
        let snap = |i: usize| {
            let mut i = i.min(s.len());
            while !s.is_char_boundary(i) {
                i -= 1;
            }
            i
        };
        self.sel = TextSelection { anchor: snap(anchor), focus: snap(focus) };
    }
}
