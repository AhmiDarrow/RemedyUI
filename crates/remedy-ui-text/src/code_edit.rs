//! Code editing: the shared editing core with several cursors at once, indentation that
//! follows the code, and lines moved as blocks. Every edit made at several cursors is one
//! undo step, and undo puts every cursor back.

use std::ops::Range;

use crate::editing::{TextChange, TextEditor, TextSelection};

/// How code is indented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indent {
    /// What one level is: a tab, or some spaces.
    pub unit: String,
}

impl Default for Indent {
    fn default() -> Self {
        Indent { unit: "    ".into() }
    }
}

/// Where a position goes after a change (positions inside removed text go to its start).
fn map_pos(p: usize, c: &TextChange) -> usize {
    if p < c.at {
        p
    } else if p >= c.at + c.removed {
        p - c.removed + c.inserted
    } else {
        c.at
    }
}

/// The cursors before and after one undo step.
#[derive(Clone, Debug)]
struct CursorStep {
    depth: usize,
    before: Vec<TextSelection>,
    after: Vec<TextSelection>,
}

/// A code editor's text and cursors. The first cursor is the main one (the one the view
/// scrolls to and input methods follow).
#[derive(Clone, Debug, Default)]
pub struct CodeEditor {
    ed: TextEditor,
    /// Cursors besides the main one, in no order.
    extra: Vec<TextSelection>,
    indent: Indent,
    undo: Vec<CursorStep>,
    redo: Vec<CursorStep>,
}

impl CodeEditor {
    /// An editor holding `text`, the caret at the start.
    pub fn new(text: &str) -> Self {
        let mut ed = TextEditor::new(text);
        ed.set_selection(0, 0);
        CodeEditor { ed, ..Self::default() }
    }

    /// The text.
    pub fn text(&self) -> String {
        self.ed.text()
    }

    /// The editing core, to read.
    pub fn editor(&self) -> &TextEditor {
        &self.ed
    }

    /// Bumped on every change.
    pub fn version(&self) -> u64 {
        self.ed.version()
    }

    /// How a level of indentation is written.
    pub fn set_indent(&mut self, indent: Indent) {
        self.indent = indent;
    }

    /// Every cursor, the main one first, then the rest in text order.
    pub fn cursors(&self) -> Vec<TextSelection> {
        let mut rest = self.extra.clone();
        rest.sort_by_key(|s| s.range().start);
        std::iter::once(self.ed.selection()).chain(rest).collect()
    }

    /// Keeps only the main cursor (Escape).
    pub fn single_cursor(&mut self) {
        self.extra.clear();
    }

    /// Sets the main cursor; the others stay.
    pub fn set_selection(&mut self, anchor: usize, focus: usize) {
        self.ed.set_selection(anchor, focus);
        self.merge();
    }

    /// Adds a cursor (Alt+click).
    pub fn add_cursor(&mut self, anchor: usize, focus: usize) {
        let main = self.ed.selection();
        self.ed.set_selection(anchor, focus);
        let added = self.ed.selection();
        self.ed.set_selection(main.anchor, main.focus);
        self.extra.push(added);
        self.merge();
    }

    /// Drops cursors that overlap or sit on the same place, keeping the main one.
    fn merge(&mut self) {
        let main = self.ed.selection().range();
        let mut kept: Vec<TextSelection> = Vec::new();
        for s in std::mem::take(&mut self.extra) {
            let r = s.range();
            let overlaps = |o: &Range<usize>| {
                (r.start < o.end && o.start < r.end)
                    || (r.is_empty() && o.is_empty() && r.start == o.start)
                    || (r.is_empty() && o.start < r.start && r.start < o.end)
            };
            if !overlaps(&main) && !kept.iter().any(|k| overlaps(&k.range())) {
                kept.push(s);
            }
        }
        self.extra = kept;
    }

    /// Adds a cursor on the line above or below each cursor, at the same column (Ctrl+Alt+Up
    /// or Down).
    pub fn add_cursor_vertically(&mut self, down: bool) {
        let text = self.ed.text();
        let mut add = Vec::new();
        for s in self.cursors() {
            let at = s.focus;
            let line = self.ed.line_of(at);
            let start = self.ed.line_range(line).start;
            let column = text.get(start..at).map_or(0, |t| t.chars().count());
            let target = if down { line + 1 } else { line.wrapping_sub(1) };
            if target >= self.ed.line_count() {
                continue;
            }
            let r = self.ed.line_range(target);
            let line_text = text.get(r.clone()).unwrap_or("");
            let offset: usize = line_text.chars().take(column).map(char::len_utf8).sum();
            add.push(r.start + offset);
        }
        for at in add {
            self.extra.push(TextSelection::caret(at));
        }
        self.merge();
    }

    /// Selects the word at the main cursor, or with one selected, adds a cursor on the next
    /// place the same text appears (Ctrl+D).
    pub fn select_next_occurrence(&mut self) {
        let main = self.ed.selection();
        if main.is_caret() {
            let w = self.ed.word_at(main.focus);
            if !w.is_empty() {
                self.ed.set_selection(w.start, w.end);
            }
            return;
        }
        let text = self.ed.text();
        let needle = text.get(main.range()).unwrap_or("").to_owned();
        if needle.is_empty() {
            return;
        }
        let taken: Vec<Range<usize>> = self.cursors().iter().map(TextSelection::range).collect();
        let last = taken.iter().map(|r| r.end).max().unwrap_or(0);
        // The next one after the last cursor, round to the start.
        let found = text
            .get(last..)
            .and_then(|t| t.find(&needle).map(|i| last + i))
            .or_else(|| text.find(&needle))
            .filter(|at| !taken.iter().any(|r| r.start == *at));
        if let Some(at) = found {
            self.extra.push(TextSelection { anchor: at, focus: at + needle.len() });
        }
    }

    /// Runs `edit` at every cursor, last in the text first so earlier positions stay put,
    /// as one undo step; then puts each cursor where its edit left it.
    fn each(&mut self, mut edit: impl FnMut(&mut TextEditor)) {
        let before = self.cursors();
        let mut order: Vec<(usize, TextSelection)> = before.iter().copied().enumerate().collect();
        order.sort_by_key(|(_, s)| std::cmp::Reverse(s.range().start));
        let single = order.len() == 1;
        if !single {
            self.ed.begin_group();
        }
        let _ = self.ed.take_changes();
        // Where each cursor ends, then moved by the edits made after it (those before it in
        // the text).
        let mut results: Vec<(usize, TextSelection, usize)> = Vec::new();
        for (i, s) in order {
            self.ed.set_selection(s.anchor, s.focus);
            edit(&mut self.ed);
            let mark = self.ed.take_changes();
            for r in &mut results {
                for c in &mark {
                    r.1 = TextSelection { anchor: map_pos(r.1.anchor, c), focus: map_pos(r.1.focus, c) };
                }
            }
            results.push((i, self.ed.selection(), mark.len()));
        }
        if !single {
            self.ed.end_group();
        }
        results.sort_by_key(|(i, ..)| *i);
        let mut it = results.into_iter().map(|(_, s, _)| s);
        if let Some(main) = it.next() {
            self.ed.set_selection(main.anchor, main.focus);
        }
        self.extra = it.collect();
        self.merge();
        let after = self.cursors();
        if before != after || self.ed.undo_depth() > 0 {
            self.undo.push(CursorStep { depth: self.ed.undo_depth(), before, after });
            self.redo.clear();
        }
    }

    /// Types `s` at every cursor.
    pub fn insert(&mut self, s: &str, now_ms: u64) {
        self.each(|e| e.insert(s, now_ms));
    }

    /// Pastes `s`: one line per cursor when it has as many lines as there are cursors (a
    /// column copied from several cursors), else all of it at each.
    pub fn paste(&mut self, s: &str, now_ms: u64) {
        let lines: Vec<&str> = s.split('\n').collect();
        let n = self.cursors().len();
        if n > 1 && lines.len() == n {
            // In text order: the first line to the first cursor.
            let mut order: Vec<usize> = (0..n).collect();
            let cursors = self.cursors();
            order.sort_by_key(|i| cursors.get(*i).map_or(0, |c| c.range().start));
            let mut piece_for = vec![String::new(); n];
            for (k, i) in order.into_iter().enumerate() {
                if let (Some(slot), Some(line)) = (piece_for.get_mut(i), lines.get(k)) {
                    *slot = (*line).to_owned();
                }
            }
            let mut idx = 0;
            let pieces = piece_for;
            let mut sorted: Vec<usize> = (0..n).collect();
            sorted.sort_by_key(|i| std::cmp::Reverse(cursors.get(*i).map_or(0, |c| c.range().start)));
            self.each(|e| {
                let i = sorted.get(idx).copied().unwrap_or(0);
                idx += 1;
                e.paste(pieces.get(i).map_or("", String::as_str), now_ms);
            });
            return;
        }
        self.each(|e| e.paste(s, now_ms));
    }

    /// Backspace at every cursor. In leading whitespace it removes a level of indentation.
    pub fn backspace(&mut self, now_ms: u64) {
        let unit = self.indent.unit.clone();
        self.each(|e| {
            let s = e.selection();
            if s.is_caret() {
                let line = e.line_range(e.line_of(s.focus));
                let before = e.slice(line.start..s.focus);
                if !before.is_empty() && before.chars().all(|c| c == ' ') && unit.chars().all(|c| c == ' ') {
                    let n = before.len() % unit.len();
                    let n = if n == 0 { unit.len() } else { n };
                    e.set_selection(s.focus - n, s.focus);
                }
            }
            e.backspace(now_ms);
        });
    }

    /// Delete at every cursor.
    pub fn delete_forward(&mut self, now_ms: u64) {
        self.each(|e| e.delete_forward(now_ms));
    }

    /// Enter at every cursor: the new line keeps the line's indentation, one level more after
    /// an opening bracket or a colon, and a closing bracket right after the cursor goes to a
    /// line of its own.
    pub fn enter(&mut self, now_ms: u64) {
        let unit = self.indent.unit.clone();
        self.each(|e| {
            let s = e.selection();
            let line = e.line_range(e.line_of(s.range().start));
            let text = e.slice(line.clone());
            let lead: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let before = e.slice(line.start..s.range().start);
            let after = e.slice(s.range().end..line.end);
            let opens = before.trim_end().ends_with(['{', '[', '(', ':']);
            let closes = after.trim_start().starts_with(['}', ']', ')']);
            let inner = if opens { format!("{lead}{unit}") } else { lead.clone() };
            if opens && closes {
                // Between brackets: the cursor on an indented line, the closer below.
                e.insert(&format!("\n{inner}\n{lead}"), now_ms);
                let at = e.selection().focus - (1 + lead.len());
                e.set_selection(at, at);
            } else {
                e.insert(&format!("\n{inner}"), now_ms);
            }
        });
    }

    /// The lines the cursors touch, each once, in order.
    fn touched_lines(&self) -> Vec<usize> {
        let mut lines: Vec<usize> = Vec::new();
        for s in self.cursors() {
            let r = s.range();
            let last = if !r.is_empty() && self.ed.line_range(self.ed.line_of(r.end)).start == r.end {
                // A selection ending at a line's start leaves that line out.
                self.ed.line_of(r.end).saturating_sub(1).max(self.ed.line_of(r.start))
            } else {
                self.ed.line_of(r.end)
            };
            lines.extend(self.ed.line_of(r.start)..=last);
        }
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    /// Tab: with a selection over lines, indents them; otherwise inserts a level at each
    /// cursor. Shift+Tab takes a level off the lines.
    pub fn indent(&mut self, deeper: bool, now_ms: u64) {
        let spans_lines = self.cursors().iter().any(|s| !s.is_caret());
        if deeper && !spans_lines {
            let unit = self.indent.unit.clone();
            self.insert(&unit, now_ms);
            return;
        }
        let before = self.cursors();
        let lines = self.touched_lines();
        let unit = self.indent.unit.clone();
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for l in lines.iter().rev() {
            let r = self.ed.line_range(*l);
            if deeper {
                if !r.is_empty() {
                    edits.push((r.start, 0, unit.clone()));
                }
            } else {
                let text = self.ed.slice(r.clone());
                let n = if text.starts_with('\t') {
                    1
                } else {
                    text.chars().take(unit.len()).take_while(|c| *c == ' ').count()
                };
                if n > 0 {
                    edits.push((r.start, n, String::new()));
                }
            }
        }
        if edits.is_empty() {
            return;
        }
        self.ed.begin_group();
        let _ = self.ed.take_changes();
        for (at, remove, insert) in &edits {
            self.ed.set_selection(*at, *at + *remove);
            self.ed.paste(insert, now_ms);
        }
        let changes = self.ed.take_changes();
        self.ed.end_group();
        // Every cursor moves with the text around it.
        let moved: Vec<TextSelection> = before
            .iter()
            .map(|s| {
                let (mut a, mut f) = (s.anchor, s.focus);
                for c in &changes {
                    let grow =
                        |p: usize| if c.removed == 0 && p == c.at { p + c.inserted } else { map_pos(p, c) };
                    a = grow(a);
                    f = grow(f);
                }
                TextSelection { anchor: a, focus: f }
            })
            .collect();
        let mut it = moved.into_iter();
        if let Some(main) = it.next() {
            self.ed.set_selection(main.anchor, main.focus);
        }
        self.extra = it.collect();
        self.undo.push(CursorStep { depth: self.ed.undo_depth(), before, after: self.cursors() });
        self.redo.clear();
    }

    /// Moves every cursor by a character (or collapses its selection).
    pub fn move_char(&mut self, forward: bool, extend: bool) {
        self.move_each(|e| e.move_char(forward, extend));
    }

    /// Moves every cursor by a word.
    pub fn move_word(&mut self, forward: bool, extend: bool) {
        self.move_each(|e| e.move_word(forward, extend));
    }

    /// Home/End for every cursor: Home goes to the line's first non-blank character, then to
    /// its start.
    pub fn move_line_edge(&mut self, end: bool, extend: bool) {
        self.move_each(|e| {
            let s = e.selection();
            let r = e.line_range(e.line_of(s.focus));
            let to = if end {
                r.end
            } else {
                let text = e.slice(r.clone());
                let first = r.start + (text.len() - text.trim_start().len());
                if s.focus == first { r.start } else { first }
            };
            let anchor = if extend { s.anchor } else { to };
            e.set_selection(anchor, to);
        });
    }

    fn move_each(&mut self, mut mv: impl FnMut(&mut TextEditor)) {
        let all = self.cursors();
        let mut out = Vec::with_capacity(all.len());
        for s in all {
            self.ed.set_selection(s.anchor, s.focus);
            mv(&mut self.ed);
            out.push(self.ed.selection());
        }
        let mut it = out.into_iter();
        if let Some(main) = it.next() {
            self.ed.set_selection(main.anchor, main.focus);
        }
        self.extra = it.collect();
        self.merge();
    }

    /// Undoes the last step, cursors included.
    pub fn undo(&mut self) -> bool {
        let depth = self.ed.undo_depth();
        if !self.ed.undo() {
            return false;
        }
        let _ = self.ed.take_changes();
        if let Some(i) = self.undo.iter().rposition(|s| s.depth == depth) {
            let step = self.undo.remove(i);
            self.restore(&step.before);
            self.redo.push(step);
        } else {
            self.extra.clear();
        }
        true
    }

    /// Redoes the last undone step, cursors included.
    pub fn redo(&mut self) -> bool {
        if !self.ed.redo() {
            return false;
        }
        let _ = self.ed.take_changes();
        let depth = self.ed.undo_depth();
        if let Some(i) = self.redo.iter().rposition(|s| s.depth == depth) {
            let step = self.redo.remove(i);
            self.restore(&step.after);
            self.undo.push(step);
        } else {
            self.extra.clear();
        }
        true
    }

    fn restore(&mut self, cursors: &[TextSelection]) {
        let mut it = cursors.iter().copied();
        if let Some(main) = it.next() {
            self.ed.set_selection(main.anchor, main.focus);
        }
        self.extra = it.collect();
    }

    /// The selected text of every cursor, one per line, in text order (what Copy takes).
    pub fn selected_text(&self) -> String {
        let mut parts: Vec<TextSelection> = self.cursors().into_iter().filter(|s| !s.is_caret()).collect();
        parts.sort_by_key(|s| s.range().start);
        parts.iter().map(|s| self.ed.slice(s.range())).collect::<Vec<_>>().join("\n")
    }

    /// Shows an input method's composing text at the main cursor.
    pub fn set_composition(&mut self, text: &str, cursor: usize) {
        self.ed.set_composition(text, cursor);
    }

    /// Commits composed text: typed at every cursor, as one step.
    pub fn commit_composition(&mut self, text: &str, now_ms: u64) {
        self.ed.cancel_composition();
        self.insert(text, now_ms);
    }

    /// Drops the composition.
    pub fn cancel_composition(&mut self) {
        self.ed.cancel_composition();
    }

    /// Cuts the selections (Cut): their text, one per line, removed at every cursor.
    pub fn cut(&mut self, now_ms: u64) -> String {
        let text = self.selected_text();
        if self.cursors().iter().any(|s| !s.is_caret()) {
            self.each(|e| {
                if !e.selection().is_caret() {
                    e.backspace(now_ms);
                }
            });
        }
        text
    }

    /// Selects everything (one cursor).
    pub fn select_all(&mut self) {
        self.extra.clear();
        self.ed.select_all();
    }
}
