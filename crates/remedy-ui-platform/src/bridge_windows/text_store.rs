//! The text store TSF edits, without any COM: the field's text as UTF-16 (TSF's "ACP"
//! positions), the selection, the composition, and the [`ImeEvent`]s those edits mean for the
//! app.
//!
//! The app owns the text. It reports the field through `TextInputState`; input methods read
//! and edit this mirror; each edit becomes an event the app applies, and the app's next report
//! confirms it. While a composition runs, the composition is the store's, and app reports
//! only move the caret rectangle.

use std::ops::Range;

use remedy_ui_core::Rect;
use remedy_ui_core::host::{ImeEvent, TextInputState};

/// Why an edit was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreError {
    /// A position beyond the text, or a reversed range.
    InvalidPosition,
    /// An edit away from both the selection and the composition, which the app's editing
    /// model cannot express.
    OutsideEditableArea,
}

/// What an edit changed, in ACP positions (TSF's `TS_TEXTCHANGE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextChange {
    pub(crate) start: usize,
    pub(crate) old_end: usize,
    pub(crate) new_end: usize,
}

/// What an app report changed, so TSF can be told.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppUpdate {
    pub(crate) text: Option<TextChange>,
    pub(crate) selection: bool,
    pub(crate) layout: bool,
}

#[derive(Debug, Default)]
pub(crate) struct TextStore {
    doc: Vec<u16>,
    selection: Range<usize>,
    /// The selection's active end is its start (the user extended it leftward).
    start_active: bool,
    composition: Option<Range<usize>>,
    caret: Rect,
    events: Vec<ImeEvent>,
}

/// UTF-16 position `pos` in `units` as a UTF-8 byte offset into the same text.
fn utf8_offset(units: &[u16], pos: usize) -> usize {
    char::decode_utf16(units.iter().take(pos).copied()).map(|c| c.map_or(3, char::len_utf8)).sum()
}

/// UTF-8 byte offset `byte` in `s` as a UTF-16 position.
fn utf16_offset(s: &str, byte: usize) -> usize {
    s.get(..byte.min(s.len())).map_or(0, |p| p.encode_utf16().count())
}

fn string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

impl TextStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The whole text.
    pub(crate) fn text(&self) -> &[u16] {
        &self.doc
    }

    /// The selection, and whether its start is the active end.
    pub(crate) fn selection(&self) -> (Range<usize>, bool) {
        (self.selection.clone(), self.start_active)
    }

    /// The running composition.
    pub(crate) fn composition(&self) -> Option<Range<usize>> {
        self.composition.clone()
    }

    /// The caret rectangle the app reported (window logical pixels).
    pub(crate) fn caret(&self) -> Rect {
        self.caret
    }

    /// Events for the app since the last call.
    pub(crate) fn take_events(&mut self) -> Vec<ImeEvent> {
        std::mem::take(&mut self.events)
    }

    fn check(&self, r: &Range<usize>) -> Result<(), StoreError> {
        if r.start > r.end || r.end > self.doc.len() {
            return Err(StoreError::InvalidPosition);
        }
        Ok(())
    }

    /// Takes the app's report of the field.
    pub(crate) fn set_from_app(&mut self, state: &TextInputState) -> AppUpdate {
        let layout = self.caret != state.caret;
        self.caret = state.caret;
        if self.composition.is_some() {
            return AppUpdate { layout, ..AppUpdate::default() };
        }
        let doc: Vec<u16> = state.surrounding.encode_utf16().collect();
        let sel = utf16_offset(&state.surrounding, state.selection.start)
            ..utf16_offset(&state.surrounding, state.selection.end.max(state.selection.start));
        let text = (doc != self.doc).then(|| {
            // Report the smallest changed span: common prefix and suffix stay.
            let prefix = doc.iter().zip(&self.doc).take_while(|(a, b)| a == b).count();
            let max_suffix = doc.len().min(self.doc.len()) - prefix;
            let suffix = doc
                .iter()
                .rev()
                .zip(self.doc.iter().rev())
                .take(max_suffix)
                .take_while(|(a, b)| a == b)
                .count();
            TextChange { start: prefix, old_end: self.doc.len() - suffix, new_end: doc.len() - suffix }
        });
        let selection = sel != self.selection;
        self.doc = doc;
        self.selection = sel;
        self.start_active = false;
        AppUpdate { text, selection, layout }
    }

    /// Moves the selection (an input method placing the caret inside its composition, or a
    /// reconversion selecting a word).
    pub(crate) fn set_selection(&mut self, r: Range<usize>, start_active: bool) -> Result<(), StoreError> {
        self.check(&r)?;
        self.selection = r;
        self.start_active = start_active;
        if self.composition.is_some() {
            self.preedit();
        }
        Ok(())
    }

    fn preedit(&mut self) {
        let Some(c) = self.composition.clone() else { return };
        let comp = self.doc.get(c.clone()).unwrap_or_default();
        let text = string(comp);
        let s = self.selection.start.clamp(c.start, c.end) - c.start;
        let e = self.selection.end.clamp(c.start, c.end) - c.start;
        let cursor = Some(utf8_offset(comp, s)..utf8_offset(comp, e));
        self.events.push(ImeEvent::Preedit { text, cursor });
    }

    /// Replaces `r` with `text`. Inside the composition it edits the preedit; at the
    /// selection it commits; around the caret it deletes surrounding text and commits.
    pub(crate) fn set_text(&mut self, r: Range<usize>, text: &[u16]) -> Result<TextChange, StoreError> {
        self.check(&r)?;
        let in_composition = self.composition.as_ref().is_some_and(|c| c.start <= r.start && r.end <= c.end);
        let caret = self.selection.end;
        let at_selection = r == self.selection;
        let around_caret = r.start <= caret && caret <= r.end;
        // While composing, input methods edit only their composition.
        if self.composition.is_some() && !in_composition || !(in_composition || at_selection || around_caret)
        {
            return Err(StoreError::OutsideEditableArea);
        }
        if !in_composition && !at_selection {
            // Text around the caret goes first; the commit then inserts at the caret.
            let before = utf8_offset(&self.doc, caret) - utf8_offset(&self.doc, r.start);
            let after = utf8_offset(&self.doc, r.end) - utf8_offset(&self.doc, caret);
            self.events.push(ImeEvent::DeleteSurrounding { before, after });
        }
        let change = self.replace(r, text);
        if in_composition {
            self.preedit();
        } else {
            self.events.push(ImeEvent::Commit(string(text)));
        }
        Ok(change)
    }

    fn replace(&mut self, r: Range<usize>, text: &[u16]) -> TextChange {
        let (start, old_end) = (r.start, r.end);
        let new_end = start + text.len();
        self.doc.splice(r, text.iter().copied());
        // Edits while composing lie inside the composition, which grows or shrinks with them.
        if let Some(c) = self.composition.as_mut() {
            c.end = c.end + new_end - old_end;
        }
        // The caret lands after the inserted text.
        self.selection = new_end..new_end;
        self.start_active = false;
        TextChange { start, old_end, new_end }
    }

    /// Inserts at the selection, replacing it (`InsertTextAtSelection`). With `query_only`,
    /// only reports where the text would go.
    pub(crate) fn insert_at_selection(
        &mut self,
        text: &[u16],
        query_only: bool,
    ) -> Result<(Range<usize>, Option<TextChange>), StoreError> {
        let r = self.selection.clone();
        if query_only {
            return Ok((r.start..r.end, None));
        }
        let change = self.set_text(r.clone(), text)?;
        Ok((r.start..r.start + text.len(), Some(change)))
    }

    /// A composition starts over `r` (empty for new typing; existing text for reconversion,
    /// which leaves the document and comes back as preedit).
    pub(crate) fn start_composition(&mut self, r: Range<usize>) -> Result<(), StoreError> {
        self.check(&r)?;
        self.events.push(ImeEvent::Enabled);
        if !r.is_empty() {
            let caret = self.selection.end.clamp(r.start, r.end);
            let before = utf8_offset(&self.doc, caret) - utf8_offset(&self.doc, r.start);
            let after = utf8_offset(&self.doc, r.end) - utf8_offset(&self.doc, caret);
            self.events.push(ImeEvent::DeleteSurrounding { before, after });
        }
        self.composition = Some(r);
        self.preedit();
        Ok(())
    }

    /// The composition now covers `r`.
    pub(crate) fn update_composition(&mut self, r: Range<usize>) -> Result<(), StoreError> {
        self.check(&r)?;
        self.composition = Some(r);
        self.preedit();
        Ok(())
    }

    /// The composition ends: its text is committed and stays in the document.
    pub(crate) fn end_composition(&mut self) {
        let Some(c) = self.composition.take() else { return };
        let text = string(self.doc.get(c).unwrap_or_default());
        self.events.push(ImeEvent::Preedit { text: String::new(), cursor: None });
        self.events.push(ImeEvent::Commit(text));
        self.events.push(ImeEvent::Disabled);
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn field(text: &str, caret: usize) -> TextInputState {
        {
            let mut v_ = TextInputState::default();
            v_.surrounding = text.into();
            v_.selection = caret..caret;
            v_.caret = Rect::new(10.0, 2.0, 11.0, 20.0);
            v_.private = false;
            v_.multiline = false;
            v_
        }
    }

    #[test]
    fn typing_through_an_input_method_previews_then_commits() {
        let mut s = TextStore::new();
        s.set_from_app(&field("Hello ", 6));
        s.start_composition(6..6).unwrap();
        s.insert_at_selection(&u("に"), false).unwrap();
        s.insert_at_selection(&u("ほ"), false).unwrap();
        s.set_text(6..8, &u("日本")).unwrap();
        s.end_composition();
        let ev = s.take_events();
        assert_eq!(ev.first(), Some(&ImeEvent::Enabled));
        assert!(ev.contains(&ImeEvent::Preedit { text: "にほ".into(), cursor: Some(6..6) }), "{ev:?}");
        assert!(ev.contains(&ImeEvent::Preedit { text: "日本".into(), cursor: Some(6..6) }));
        assert_eq!(&ev[ev.len() - 2..], &[ImeEvent::Commit("日本".into()), ImeEvent::Disabled]);
        assert_eq!(s.text(), u("Hello 日本").as_slice());
        // The app's confirming report changes nothing TSF must hear about.
        let mut confirm = field("Hello 日本", "Hello 日本".len());
        confirm.caret = s.caret();
        assert_eq!(s.set_from_app(&confirm), AppUpdate::default());
    }

    #[test]
    fn app_reports_name_the_smallest_change_and_wait_while_composing() {
        let mut s = TextStore::new();
        s.set_from_app(&field("abcdef", 6));
        let up = s.set_from_app(&field("abXYef", 2));
        assert_eq!(up.text, Some(TextChange { start: 2, old_end: 4, new_end: 4 }));
        assert!(up.selection);
        s.start_composition(2..2).unwrap();
        let mut moved = field("zzz", 0);
        moved.caret = Rect::new(50.0, 2.0, 51.0, 20.0);
        let up = s.set_from_app(&moved);
        assert_eq!(
            up,
            AppUpdate { text: None, selection: false, layout: true },
            "the composition owns the text"
        );
        assert_eq!(s.text(), u("abXYef").as_slice());
    }

    #[test]
    fn reconversion_takes_existing_text_back_into_a_composition() {
        let mut s = TextStore::new();
        let mut st = field("変換 test", 0);
        st.selection = 0.."変換".len();
        s.set_from_app(&st);
        s.take_events();
        s.start_composition(0..2).unwrap();
        assert_eq!(
            s.take_events(),
            vec![
                ImeEvent::Enabled,
                ImeEvent::DeleteSurrounding { before: 6, after: 0 },
                ImeEvent::Preedit { text: "変換".into(), cursor: Some(0..6) },
            ]
        );
    }

    #[test]
    fn direct_inserts_commit_and_replace_the_selection() {
        let mut s = TextStore::new();
        s.set_from_app(&field("", 0));
        let (r, change) = s.insert_at_selection(&u("😀"), false).unwrap();
        assert_eq!(r, 0..2, "a surrogate pair is two positions");
        assert_eq!(change, Some(TextChange { start: 0, old_end: 0, new_end: 2 }));
        assert_eq!(s.take_events(), vec![ImeEvent::Commit("😀".into())]);
        let mut st = field("abc", 0);
        st.selection = 1..2;
        s.set_from_app(&st);
        assert_eq!(s.insert_at_selection(&u("X"), true).unwrap(), (1..2, None), "a query changes nothing");
        assert_eq!(s.text(), u("abc").as_slice());
        s.insert_at_selection(&u("X"), false).unwrap();
        assert_eq!(s.take_events(), vec![ImeEvent::Commit("X".into())]);
        assert_eq!(s.selection().0, 2..2, "the caret lands after the insert");
    }

    #[test]
    fn edits_around_the_caret_delete_then_commit_and_others_are_refused() {
        let mut s = TextStore::new();
        s.set_from_app(&field("hello", 5));
        s.set_text(3..5, &u("p!")).unwrap();
        assert_eq!(
            s.take_events(),
            vec![ImeEvent::DeleteSurrounding { before: 2, after: 0 }, ImeEvent::Commit("p!".into())]
        );
        assert_eq!(s.set_text(0..1, &u("H")), Err(StoreError::OutsideEditableArea));
        assert_eq!(s.set_text(4..99, &u("x")), Err(StoreError::InvalidPosition));
        s.start_composition(5..5).unwrap();
        assert_eq!(
            s.set_text(0..1, &u("H")),
            Err(StoreError::OutsideEditableArea),
            "composing edits stay inside"
        );
    }

    #[test]
    fn preedit_cursors_are_utf8_offsets() {
        let mut s = TextStore::new();
        s.set_from_app(&field("", 0));
        s.start_composition(0..0).unwrap();
        s.insert_at_selection(&u("a😀b"), false).unwrap();
        s.set_selection(3..3, false).unwrap();
        let last = s.take_events().pop();
        assert_eq!(last, Some(ImeEvent::Preedit { text: "a😀b".into(), cursor: Some(5..5) }));
    }
}
