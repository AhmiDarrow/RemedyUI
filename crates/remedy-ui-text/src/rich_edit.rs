//! Rich editing: the same editing core as plain fields ([`TextEditor`]: grapheme-safe
//! movement, input methods, undo grouped by typing), with formatting kept beside the text.
//!
//! - **Marks** format runs of characters: bold, italic, strikethrough, code, links and
//!   mentions. They follow every edit; typing at the end of bold text continues it, typing
//!   inside a link extends it, and a mention is one unit (deleting any of it deletes it all,
//!   the caret steps over it).
//! - **Blocks** give each line a kind: paragraph, heading, bullet or numbered list item (with
//!   a nesting level), task, quote or code. Enter continues a list, Enter on an empty item
//!   ends it, Backspace at the start of an item turns it back into a paragraph.
//! - **Undo** covers both: one history, where making words bold is a step like typing them.
//!
//! [`RichEditor::view`] lays the document out for display: the marks as spans, each block's
//! marker ("•", "3.", "☐") before its line, and a map between positions in the text and in
//! what is shown.

use std::ops::Range;

use remedy_ui_core::Color;

use crate::editing::{TextChange, TextEditor, TextSelection};
use crate::rich::RichText;
use crate::style::TextStylePatch;

/// The line separator: a line break inside one block (Shift+Enter in a list item).
pub const SOFT_BREAK: char = '\u{2028}';

/// Formatting on a run of characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// Exhaustive on purpose: the rich document model, which every serializer and view must handle completely, so a new kind is a major change.
pub enum Mark {
    /// Bold.
    Bold,
    /// Italic.
    Italic,
    /// A line through the middle.
    Strike,
    /// Code, in the monospace face.
    Code,
    /// A link to this destination.
    Link(String),
    /// A mention of someone or something the app knows by this id; edited as one unit.
    Mention(String),
}

impl Mark {
    /// Typing right after a run of this mark continues it.
    pub fn continues(&self) -> bool {
        matches!(self, Mark::Bold | Mark::Italic | Mark::Strike | Mark::Code)
    }

    /// The same kind of mark (any link is a link).
    pub fn same_kind(&self, other: &Mark) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// What kind of block a line is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// Exhaustive on purpose: the rich document model, which every serializer and view must handle completely, so a new kind is a major change.
pub enum Block {
    /// Body text.
    #[default]
    Paragraph,
    /// A heading, level 1 to 6.
    Heading(u8),
    /// A bulleted list item, nested `level` deep (0 at the top).
    Bullet(u8),
    /// A numbered list item, nested `level` deep.
    Numbered(u8),
    /// A task, done or not, nested `level` deep.
    Task {
        /// Done.
        done: bool,
        /// How deep it is nested.
        level: u8,
    },
    /// A quote.
    Quote,
    /// A line of a code block.
    Code,
}

/// How deep lists nest.
pub const MAX_LEVEL: u8 = 5;

impl Block {
    /// A list item's nesting level (0 for other blocks).
    pub fn level(self) -> u8 {
        match self {
            Block::Bullet(l) | Block::Numbered(l) | Block::Task { level: l, .. } => l,
            _ => 0,
        }
    }

    /// True for list items and tasks.
    pub fn is_list(self) -> bool {
        matches!(self, Block::Bullet(_) | Block::Numbered(_) | Block::Task { .. })
    }

    fn with_level(self, level: u8) -> Block {
        let level = level.min(MAX_LEVEL);
        match self {
            Block::Bullet(_) => Block::Bullet(level),
            Block::Numbered(_) => Block::Numbered(level),
            Block::Task { done, .. } => Block::Task { done, level },
            b => b,
        }
    }

    /// What the next line is after Enter: lists, quotes and code go on; a heading is followed
    /// by body text; a new task starts not done.
    pub fn next(self) -> Block {
        match self {
            Block::Heading(_) => Block::Paragraph,
            Block::Task { level, .. } => Block::Task { done: false, level },
            b => b,
        }
    }

    /// The same kind of block, whatever its level or state.
    pub fn same_kind(self, other: Block) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// A rich document: text, marks over byte ranges of it, and one block per line (lines end at
/// `\n`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichDoc {
    /// The text.
    pub text: String,
    /// Marks over byte ranges of the text.
    pub marks: Vec<(Range<usize>, Mark)>,
    /// One block per line.
    pub blocks: Vec<Block>,
}

impl RichDoc {
    /// Plain text: paragraphs, no marks.
    pub fn plain(text: impl Into<String>) -> Self {
        RichDoc { text: text.into(), ..Self::default() }.normalized()
    }

    /// How many lines the text has.
    pub fn lines(&self) -> usize {
        self.text.bytes().filter(|b| *b == b'\n').count() + 1
    }

    /// The document made consistent: one block per line, marks inside the text on character
    /// boundaries, merged where they touch, in order.
    pub fn normalized(mut self) -> Self {
        self.blocks.resize(self.lines(), Block::Paragraph);
        let text = &self.text;
        for (r, _) in &mut self.marks {
            let floor = |mut i: usize| {
                i = i.min(text.len());
                while !text.is_char_boundary(i) {
                    i -= 1;
                }
                i
            };
            *r = floor(r.start)..floor(r.end);
        }
        normalize(&mut self.marks);
        self
    }

    /// The bytes of line `line`, without its `\n`.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let mut start = 0;
        for (i, l) in self.text.split('\n').enumerate() {
            if i == line {
                return start..start + l.len();
            }
            start += l.len() + 1;
        }
        self.text.len()..self.text.len()
    }

    /// The marks covering byte `at` (a mark covers the bytes it starts at, up to its end).
    pub fn marks_at(&self, at: usize) -> Vec<&Mark> {
        self.marks.iter().filter(|(r, _)| r.start <= at && at < r.end).map(|(_, m)| m).collect()
    }
}

/// Merges each mark's ranges where they overlap (or touch, except mentions: two mentions side
/// by side stay two), drops empty ones, and sorts by position.
fn normalize(marks: &mut Vec<(Range<usize>, Mark)>) {
    let mut kinds: Vec<Mark> = Vec::new();
    for (_, m) in marks.iter() {
        if !kinds.contains(m) {
            kinds.push(m.clone());
        }
    }
    let mut out = Vec::new();
    for k in kinds {
        let mut rs: Vec<Range<usize>> =
            marks.iter().filter(|(r, m)| *m == k && r.start < r.end).map(|(r, _)| r.clone()).collect();
        rs.sort_by_key(|r| (r.start, r.end));
        let touching = !matches!(k, Mark::Mention(_));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for r in rs {
            match merged.last_mut() {
                Some(last) if r.start < last.end || (touching && r.start == last.end) => {
                    last.end = last.end.max(r.end);
                }
                _ => merged.push(r),
            }
        }
        out.extend(merged.into_iter().map(|r| (r, k.clone())));
    }
    out.sort_by_key(|(r, _)| (r.start, r.end));
    *marks = out;
}

/// Removes `mark` (or, with `kind`, any mark of its kind) from bytes `range`, splitting runs.
fn unmark(marks: &mut Vec<(Range<usize>, Mark)>, range: Range<usize>, mark: &Mark, kind: bool) {
    let mut out = Vec::new();
    for (r, m) in marks.drain(..) {
        let hit = if kind { m.same_kind(mark) } else { m == *mark };
        if !hit || r.end <= range.start || r.start >= range.end {
            out.push((r, m));
            continue;
        }
        if r.start < range.start {
            out.push((r.start..range.start, m.clone()));
        }
        if r.end > range.end {
            out.push((range.end..r.end, m));
        }
    }
    *marks = out;
}

/// Where position `p` is after `c`: before the change it stays, after it shifts, inside what
/// was removed it goes to the change's start.
fn map_pos(p: usize, c: &TextChange) -> usize {
    if p < c.at {
        p
    } else if p == c.at && c.removed == 0 {
        // Text inserted where a run starts goes before it.
        p + c.inserted
    } else if p >= c.at + c.removed {
        p - c.removed + c.inserted
    } else {
        c.at
    }
}

/// The formatting kept beside the text.
#[derive(Clone, Debug, Default, PartialEq)]
struct Format {
    marks: Vec<(Range<usize>, Mark)>,
    blocks: Vec<Block>,
}

/// An undo step's formatting: what it was before the step, for undo depth `depth`.
#[derive(Clone, Debug)]
struct FormatStep {
    depth: usize,
    before: Format,
}

/// Colors and faces the view draws a rich document with (from the theme).
#[derive(Clone, Debug, PartialEq)]
pub struct RichLook {
    /// Links.
    pub link: Color,
    /// Mentions.
    pub mention: Color,
    /// List markers, quote bars and quotes.
    pub muted: Color,
    /// The monospace families for code.
    pub code_families: Vec<String>,
    /// Text sizes for headings 1, 2 and 3 to 6.
    pub heading_sizes: [f32; 3],
}

/// A rich document laid out for display, with the map between text and display positions.
#[derive(Clone, Debug)]
pub struct RichView {
    /// What is shown: the text (with any input-method composition), block markers, styles.
    pub text: RichText,
    /// The composition's bytes in `text`, while an input method is composing.
    pub composition: Option<Range<usize>>,
    /// Where each line starts in the shown text (before markers) and its marker's length.
    markers: Vec<(usize, usize)>,
    /// Shown text: the composition replaced model bytes `model` with `len` bytes.
    comp: Option<(Range<usize>, usize)>,
    /// Task lines, by the display range of their box.
    tasks: Vec<(Range<usize>, usize)>,
}

impl RichView {
    /// The display position of text position `at`.
    pub fn to_display(&self, at: usize) -> usize {
        let shown = match &self.comp {
            Some((m, len)) if at >= m.end => at - m.len() + len,
            Some((m, _)) if at > m.start => m.start,
            _ => at,
        };
        shown + self.markers.iter().filter(|(s, _)| *s <= shown).map(|(_, l)| l).sum::<usize>()
    }

    /// The text position of display position `at` (a marker maps to its line's start; the
    /// composition to where it sits).
    pub fn to_model(&self, at: usize) -> usize {
        let mut extra = 0;
        let mut shown = at;
        for (s, l) in &self.markers {
            if s + extra + l <= at {
                extra += l;
                shown = at - extra;
            } else if s + extra <= at {
                shown = *s;
                break;
            } else {
                break;
            }
        }
        match &self.comp {
            Some((m, len)) if shown >= m.start + len => shown - len + m.len(),
            Some((m, _)) if shown > m.start => m.start,
            _ => shown,
        }
    }

    /// The task line whose box is at display position `at`, if any.
    pub fn task_at(&self, at: usize) -> Option<usize> {
        self.tasks.iter().find(|(r, _)| r.start <= at && at < r.end).map(|(_, line)| *line)
    }
}

/// An editor for a [`RichDoc`]: plain editing plus marks, blocks and one undo history.
#[derive(Clone, Debug, Default)]
pub struct RichEditor {
    ed: TextEditor,
    fmt: Format,
    /// Marks the next typed text takes, set by toggling with nothing selected.
    pending: Option<Vec<Mark>>,
    undo: Vec<FormatStep>,
    redo: Vec<FormatStep>,
}

impl RichEditor {
    /// An editor holding `doc`, with the caret at the end.
    pub fn new(doc: RichDoc) -> Self {
        let doc = doc.normalized();
        RichEditor {
            ed: TextEditor::new(&doc.text),
            fmt: Format { marks: doc.marks, blocks: doc.blocks },
            ..Self::default()
        }
    }

    /// The document.
    pub fn doc(&self) -> RichDoc {
        RichDoc { text: self.ed.text(), marks: self.fmt.marks.clone(), blocks: self.fmt.blocks.clone() }
    }

    /// Replaces the document (not undoable: a new document, like opening a file).
    pub fn set_doc(&mut self, doc: RichDoc) {
        *self = RichEditor::new(doc);
    }

    /// The plain text.
    pub fn text(&self) -> String {
        self.ed.text()
    }

    /// The editing core underneath, to read (boundaries, selection, composition).
    pub fn editor(&self) -> &TextEditor {
        &self.ed
    }

    /// Bumped on every change to the text or its formatting.
    pub fn version(&self) -> u64 {
        self.ed.version()
    }

    /// The selection.
    pub fn selection(&self) -> TextSelection {
        self.ed.selection()
    }

    // ---------- the format following edits ----------

    /// Runs an edit on the text and brings the formatting along. `typed` text takes the marks
    /// the caret has (or the pending ones).
    fn edit(&mut self, typed: bool, f: impl FnOnce(&mut TextEditor)) {
        let before = self.fmt.clone();
        let depth = self.ed.undo_depth();
        let pending = if typed { self.pending.take() } else { None };
        f(&mut self.ed);
        let changes = self.ed.take_changes();
        if changes.is_empty() {
            self.pending = pending.or(self.pending.take());
            return;
        }
        for c in &changes {
            self.follow(c, typed, pending.as_deref());
        }
        self.pending = None;
        if self.ed.undo_depth() > depth {
            self.undo.push(FormatStep { depth: self.ed.undo_depth(), before });
        }
        self.redo.clear();
    }

    /// Moves the marks and blocks for one change.
    fn follow(&mut self, c: &TextChange, typed: bool, pending: Option<&[Mark]>) {
        let old = std::mem::take(&mut self.fmt.marks);
        let mut marks = Vec::with_capacity(old.len());
        for (r, m) in old {
            // A mention is one unit: an edit inside it leaves plain text.
            if let Mark::Mention(_) = m {
                let removed_inside = c.removed > 0 && c.at < r.end && c.at + c.removed > r.start;
                let inserted_inside = c.inserted > 0 && r.start < c.at && c.at < r.end;
                let covers = c.at <= r.start && c.at + c.removed >= r.end;
                if (removed_inside || inserted_inside) && !covers {
                    continue;
                }
            }
            let (start, end) = (map_pos(r.start, c), map_pos(r.end, c));
            let end = if r.end == c.at && c.removed == 0 { r.end } else { end };
            marks.push((start..end, m));
        }
        if typed && c.inserted > 0 {
            let new = c.at..c.at + c.inserted;
            let wanted: Vec<Mark> = match pending {
                Some(p) => p.to_vec(),
                None => marks
                    .iter()
                    .filter(|(r, m)| m.continues() && r.start < c.at && r.end >= c.at)
                    .map(|(_, m)| m.clone())
                    .collect(),
            };
            for m in [Mark::Bold, Mark::Italic, Mark::Strike, Mark::Code] {
                if !wanted.contains(&m) {
                    unmark(&mut marks, new.clone(), &m, false);
                }
            }
            for m in wanted {
                marks.push((new.clone(), m));
            }
        }
        normalize(&mut marks);
        self.fmt.marks = marks;
        // One block per line: lines removed go, lines inserted take the next block after
        // the line the change started on.
        let first = self.ed.line_of(c.at);
        let kind = self.fmt.blocks.get(first).copied().unwrap_or_default();
        let from = (first + 1).min(self.fmt.blocks.len());
        let to = (first + 1 + c.removed_lines).min(self.fmt.blocks.len());
        self.fmt.blocks.splice(from..to, std::iter::repeat_n(kind.next(), c.inserted_lines));
        self.fmt.blocks.resize(self.ed.line_count(), Block::Paragraph);
    }

    /// Changes the formatting only, as one undo step.
    fn format(&mut self, f: impl FnOnce(&mut Format, &TextEditor)) {
        let before = self.fmt.clone();
        f(&mut self.fmt, &self.ed);
        normalize(&mut self.fmt.marks);
        if self.fmt == before {
            return;
        }
        self.ed.push_step();
        self.undo.push(FormatStep { depth: self.ed.undo_depth(), before });
        self.redo.clear();
    }

    // ---------- selection and movement ----------

    /// The mention around byte `at` (strictly inside it), if any.
    fn mention_around(&self, at: usize) -> Option<Range<usize>> {
        self.fmt
            .marks
            .iter()
            .find(|(r, m)| matches!(m, Mark::Mention(_)) && r.start < at && at < r.end)
            .map(|(r, _)| r.clone())
    }

    /// Sets the selection (each end on a grapheme boundary).
    pub fn set_selection(&mut self, anchor: usize, focus: usize) {
        self.ed.set_selection(anchor, focus);
        self.pending = None;
    }

    /// Selects everything.
    pub fn select_all(&mut self) {
        self.ed.select_all();
        self.pending = None;
    }

    /// Steps the focus out of a mention it landed inside, in the direction it was going.
    fn step_over_mention(&mut self, forward: bool, extend: bool) {
        let s = self.ed.selection();
        if let Some(r) = self.mention_around(s.focus) {
            let to = if forward { r.end } else { r.start };
            let anchor = if extend { s.anchor } else { to };
            self.ed.set_selection(anchor, to);
        }
    }

    /// Left/Right: one character (a mention is one), or collapse a selection.
    pub fn move_char(&mut self, forward: bool, extend: bool) {
        self.ed.move_char(forward, extend);
        self.step_over_mention(forward, extend);
        self.pending = None;
    }

    /// Ctrl+Left/Right: by word.
    pub fn move_word(&mut self, forward: bool, extend: bool) {
        self.ed.move_word(forward, extend);
        self.step_over_mention(forward, extend);
        self.pending = None;
    }

    /// Home/End.
    pub fn move_to_edge(&mut self, end: bool, extend: bool) {
        self.ed.move_to_edge(end, extend);
        self.pending = None;
    }

    // ---------- editing ----------

    /// Types `s` at the selection, in the marks the caret has.
    pub fn insert(&mut self, s: &str, now_ms: u64) {
        self.edit(true, |e| e.insert(s, now_ms));
    }

    /// Pastes plain text, in the marks the caret has.
    pub fn paste_text(&mut self, s: &str, now_ms: u64) {
        self.edit(true, |e| e.paste(s, now_ms));
    }

    /// Pastes a rich document at the selection, keeping its marks; its first line joins the
    /// line it lands in (taking the pasted block when that line is empty), and its other
    /// lines keep their blocks.
    pub fn paste(&mut self, doc: &RichDoc, now_ms: u64) {
        let doc = doc.clone().normalized();
        let at = self.ed.selection().range().start;
        let line = self.ed.line_of(at);
        // The line is empty apart from what the paste replaces.
        let (lr, sel) = (self.ed.line_range(line), self.ed.selection().range());
        let line_empty = lr.start >= sel.start && lr.end <= sel.end;
        self.edit(false, |e| e.paste(&doc.text, now_ms));
        // The pasted marks and blocks, without a step of their own: they are part of the paste.
        for (r, m) in &doc.marks {
            self.fmt.marks.push((r.start + at..r.end + at, m.clone()));
        }
        normalize(&mut self.fmt.marks);
        for (i, b) in doc.blocks.iter().enumerate() {
            if i == 0 && !line_empty {
                continue;
            }
            if let Some(slot) = self.fmt.blocks.get_mut(line + i) {
                *slot = *b;
            }
        }
    }

    /// Enter: a new line that continues the block; on an empty list item, quote line or the
    /// last empty line of code, ends the block instead.
    pub fn enter(&mut self, now_ms: u64) {
        let sel = self.ed.selection();
        let line = self.ed.line_of(sel.range().start);
        let block = self.fmt.blocks.get(line).copied().unwrap_or_default();
        let range = self.ed.line_range(line);
        if sel.is_caret()
            && range.is_empty()
            && block != Block::Paragraph
            && !matches!(block, Block::Heading(_))
        {
            if block.level() > 0 {
                self.indent(false);
            } else {
                self.format(|f, _| {
                    if let Some(b) = f.blocks.get_mut(line) {
                        *b = Block::Paragraph;
                    }
                });
            }
            return;
        }
        let at_start = sel.is_caret() && sel.focus == range.start && !range.is_empty();
        self.edit(true, |e| e.insert("\n", now_ms));
        if at_start {
            // Enter at the start of a line opens an empty line above that keeps the line's
            // own block below: a heading stays a heading.
            if let Some(b) = self.fmt.blocks.get_mut(line + 1) {
                *b = block;
            }
            if let Some(b) = self.fmt.blocks.get_mut(line) {
                *b = if block.is_list() || block == Block::Quote || block == Block::Code {
                    block.next()
                } else {
                    Block::Paragraph
                };
            }
        }
    }

    /// Shift+Enter: a line break inside the block.
    pub fn soft_break(&mut self, now_ms: u64) {
        let mut b = [0u8; 4];
        let s = SOFT_BREAK.encode_utf8(&mut b);
        self.edit(true, |e| e.insert(s, now_ms));
    }

    /// Backspace: at the start of a list item, quote or heading, makes it body text (or one
    /// level shallower); after a mention, deletes the mention; otherwise as in any field.
    pub fn backspace(&mut self, now_ms: u64) {
        let sel = self.ed.selection();
        if sel.is_caret() {
            let line = self.ed.line_of(sel.focus);
            let block = self.fmt.blocks.get(line).copied().unwrap_or_default();
            if sel.focus == self.ed.line_range(line).start && block != Block::Paragraph {
                if block.level() > 0 {
                    self.indent(false);
                } else {
                    self.format(|f, _| {
                        if let Some(b) = f.blocks.get_mut(line) {
                            *b = Block::Paragraph;
                        }
                    });
                }
                return;
            }
            let mention = self
                .fmt
                .marks
                .iter()
                .find(|(r, m)| matches!(m, Mark::Mention(_)) && r.start < sel.focus && sel.focus <= r.end)
                .map(|(r, _)| r.clone());
            if let Some(r) = mention {
                self.ed.set_selection(r.start, r.end);
            }
        }
        self.edit(false, |e| e.backspace(now_ms));
    }

    /// Delete: as Backspace, forwards.
    pub fn delete_forward(&mut self, now_ms: u64) {
        let sel = self.ed.selection();
        if sel.is_caret()
            && let Some((r, _)) = self
                .fmt
                .marks
                .iter()
                .find(|(r, m)| matches!(m, Mark::Mention(_)) && r.start <= sel.focus && sel.focus < r.end)
        {
            let r = r.clone();
            self.ed.set_selection(r.start, r.end);
        }
        self.edit(false, |e| e.delete_forward(now_ms));
    }

    /// Ctrl+Backspace / Ctrl+Delete.
    pub fn delete_word(&mut self, forward: bool, now_ms: u64) {
        self.edit(false, |e| e.delete_word(forward, now_ms));
    }

    /// The selected part of the document.
    pub fn selected(&self) -> RichDoc {
        let r = self.ed.selection().range();
        let text = self.ed.slice(r.clone());
        let marks = self
            .fmt
            .marks
            .iter()
            .filter(|(m, _)| m.start < r.end && m.end > r.start)
            .map(|(m, k)| (m.start.max(r.start) - r.start..m.end.min(r.end) - r.start, k.clone()))
            .collect();
        let (first, last) = (self.ed.line_of(r.start), self.ed.line_of(r.end));
        let blocks = self.fmt.blocks.get(first..=last).map(<[Block]>::to_vec).unwrap_or_default();
        RichDoc { text, marks, blocks }.normalized()
    }

    /// Removes and returns the selection (Cut).
    pub fn cut(&mut self, now_ms: u64) -> RichDoc {
        let doc = self.selected();
        if !self.ed.selection().is_caret() {
            self.edit(false, |e| {
                let _ = e.cut(now_ms);
            });
        }
        doc
    }

    /// Undoes the last step, text and formatting together.
    pub fn undo(&mut self) -> bool {
        let depth = self.ed.undo_depth();
        if !self.ed.undo() {
            return false;
        }
        let _ = self.ed.take_changes();
        if let Some(i) = self.undo.iter().rposition(|s| s.depth == depth) {
            let step = self.undo.remove(i);
            let after = std::mem::replace(&mut self.fmt, step.before);
            self.redo.push(FormatStep { depth, before: after });
        }
        self.fmt.blocks.resize(self.ed.line_count(), Block::Paragraph);
        self.pending = None;
        true
    }

    /// Redoes the last undone step.
    pub fn redo(&mut self) -> bool {
        if !self.ed.redo() {
            return false;
        }
        let _ = self.ed.take_changes();
        let depth = self.ed.undo_depth();
        if let Some(i) = self.redo.iter().rposition(|s| s.depth == depth) {
            let step = self.redo.remove(i);
            let before = std::mem::replace(&mut self.fmt, step.before);
            self.undo.push(FormatStep { depth, before });
        }
        self.fmt.blocks.resize(self.ed.line_count(), Block::Paragraph);
        self.pending = None;
        true
    }

    /// True when there is a step to undo.
    pub fn can_undo(&self) -> bool {
        self.ed.can_undo()
    }

    /// True when there is a step to redo.
    pub fn can_redo(&self) -> bool {
        self.ed.can_redo()
    }

    // ---------- input methods ----------

    /// Shows composing text at the selection.
    pub fn set_composition(&mut self, text: &str, cursor: usize) {
        self.ed.set_composition(text, cursor);
    }

    /// Commits the composition as one typed step.
    pub fn commit_composition(&mut self, text: &str, now_ms: u64) {
        self.edit(true, |e| e.commit_composition(text, now_ms));
    }

    /// Drops the composition.
    pub fn cancel_composition(&mut self) {
        self.ed.cancel_composition();
    }

    // ---------- formatting ----------

    /// The marks that continue (bold, italic, strikethrough, code) active at the selection:
    /// covering all of it, or at the caret what typing would get.
    pub fn active_marks(&self) -> Vec<Mark> {
        if let Some(p) = &self.pending {
            return p.clone();
        }
        let r = self.ed.selection().range();
        if r.is_empty() {
            let at = r.start;
            return self
                .fmt
                .marks
                .iter()
                .filter(|(m, k)| k.continues() && m.start < at && m.end >= at)
                .map(|(_, k)| k.clone())
                .collect();
        }
        [Mark::Bold, Mark::Italic, Mark::Strike, Mark::Code]
            .into_iter()
            .filter(|m| self.covers(r.clone(), m))
            .collect()
    }

    /// True when every character of `range` (line breaks aside) has `mark`.
    fn covers(&self, range: Range<usize>, mark: &Mark) -> bool {
        let text = self.ed.slice(range.clone());
        text.char_indices().filter(|(_, c)| *c != '\n').all(|(i, _)| {
            let at = range.start + i;
            self.fmt.marks.iter().any(|(r, m)| m == mark && r.start <= at && at < r.end)
        })
    }

    /// Turns `mark` (bold, italic, strikethrough or code) on or off for the selection, or for
    /// what is typed next at the caret.
    pub fn toggle(&mut self, mark: Mark) {
        let r = self.ed.selection().range();
        if r.is_empty() {
            let mut now = self.active_marks();
            match now.iter().position(|m| *m == mark) {
                Some(i) => {
                    now.remove(i);
                }
                None => now.push(mark),
            }
            self.pending = Some(now);
            return;
        }
        let on = self.covers(r.clone(), &mark);
        self.format(|f, _| {
            unmark(&mut f.marks, r.clone(), &mark, false);
            if !on {
                f.marks.push((r, mark));
            }
        });
    }

    /// The link at the caret (or covering the selection's start): its range and destination.
    pub fn link_at_caret(&self) -> Option<(Range<usize>, String)> {
        let at = self.ed.selection().range().start;
        self.fmt.marks.iter().find_map(|(r, m)| match m {
            Mark::Link(url) if r.start <= at && at <= r.end && r.start < r.end => {
                Some((r.clone(), url.clone()))
            }
            _ => None,
        })
    }

    /// Links the selection to `url`, or with `None` removes links from it. With nothing
    /// selected, acts on the link at the caret; with no link there, inserts `text` (or the URL)
    /// linked.
    pub fn set_link(&mut self, url: Option<String>, text: Option<&str>, now_ms: u64) {
        let mut r = self.ed.selection().range();
        if r.is_empty()
            && let Some((lr, _)) = self.link_at_caret()
        {
            r = lr;
        }
        if r.is_empty() {
            let Some(url) = url else { return };
            let shown = text.filter(|t| !t.is_empty()).unwrap_or(&url).to_owned();
            let at = r.start;
            self.ed.break_step();
            self.edit(false, |e| e.insert(&shown, now_ms));
            self.ed.break_step();
            let end = at + shown.len();
            self.fmt.marks.push((at..end, Mark::Link(url)));
            normalize(&mut self.fmt.marks);
            return;
        }
        let any = Mark::Link(String::new());
        self.format(|f, _| {
            unmark(&mut f.marks, r.clone(), &any, true);
            if let Some(url) = url {
                f.marks.push((r, Mark::Link(url)));
            }
        });
    }

    /// Inserts a mention of `id` shown as `text` (one unit), then a space.
    pub fn insert_mention(&mut self, id: &str, text: &str, now_ms: u64) {
        let at = self.ed.selection().range().start;
        // One undo step of its own: the mention and its space.
        self.ed.break_step();
        self.edit(false, |e| e.insert(&format!("{text} "), now_ms));
        self.ed.break_step();
        self.fmt.marks.push((at..at + text.len(), Mark::Mention(id.to_owned())));
        normalize(&mut self.fmt.marks);
    }

    /// Replaces bytes `range` (what was typed after "@") with a mention.
    pub fn replace_with_mention(&mut self, range: Range<usize>, id: &str, text: &str, now_ms: u64) {
        self.ed.set_selection(range.start, range.end);
        self.insert_mention(id, text, now_ms);
    }

    /// The block of the line with the caret.
    pub fn block(&self) -> Block {
        let line = self.ed.line_of(self.ed.selection().focus);
        self.fmt.blocks.get(line).copied().unwrap_or_default()
    }

    /// The lines the selection touches.
    fn selected_lines(&self) -> Range<usize> {
        let r = self.ed.selection().range();
        self.ed.line_of(r.start)..self.ed.line_of(r.end) + 1
    }

    /// Makes the selected lines `block`; when they all are that kind already, makes them
    /// paragraphs again.
    pub fn set_block(&mut self, block: Block) {
        let lines = self.selected_lines();
        self.format(|f, _| {
            let all = f.blocks.get(lines.clone()).is_some_and(|bs| bs.iter().all(|b| b.same_kind(block)));
            for b in f.blocks.get_mut(lines).into_iter().flatten() {
                *b = if all { Block::Paragraph } else { block.with_level(b.level()) };
            }
        });
    }

    /// Tab / Shift+Tab in a list: nests the selected items one level deeper or shallower.
    /// Returns false when the lines are not list items (so Tab can move focus instead).
    pub fn indent(&mut self, deeper: bool) -> bool {
        let lines = self.selected_lines();
        let listed = self.fmt.blocks.get(lines.clone()).is_some_and(|bs| bs.iter().all(|b| b.is_list()));
        if !listed {
            return false;
        }
        self.format(|f, _| {
            for b in f.blocks.get_mut(lines).into_iter().flatten() {
                let l = b.level();
                let next = if deeper { l.saturating_add(1).min(MAX_LEVEL) } else { l.saturating_sub(1) };
                *b = b.with_level(next);
            }
        });
        true
    }

    /// Ticks or unticks the task on `line`.
    pub fn toggle_task(&mut self, line: usize) {
        self.format(|f, _| {
            if let Some(Block::Task { done, .. }) = f.blocks.get_mut(line) {
                *done = !*done;
            }
        });
    }

    // ---------- display ----------

    /// The document laid out for display with `look`.
    pub fn view(&self, look: &RichLook) -> RichView {
        let (shown, composition) = self.ed.display_text();
        // The model bytes the composition replaced, and how long it is.
        let comp = match (self.ed.composition(), &composition) {
            (Some(c), Some(range)) => {
                let r = self.ed.selection().range();
                Some((c.at.min(r.start)..r.end.max(c.at), range.len()))
            }
            _ => None,
        };
        let to_shown = |at: usize| match &comp {
            Some((m, len)) if at >= m.end => at - m.len() + len,
            Some((m, _)) if at > m.start => m.start,
            _ => at,
        };
        // Lines of the shown text and their markers.
        let mut out = RichText::new();
        let mut markers = Vec::new();
        let mut tasks = Vec::new();
        let mut numbers = [0u32; MAX_LEVEL as usize + 1];
        let mut line_starts = Vec::new();
        let mut start = 0;
        for (i, line) in shown.split('\n').enumerate() {
            line_starts.push((start, start + line.len(), i));
            start += line.len() + 1;
        }
        let mut display_of = Vec::with_capacity(line_starts.len());
        for &(s, e, i) in &line_starts {
            let block = self.fmt.blocks.get(i).copied().unwrap_or_default();
            // Numbering counts items at a level until something else breaks the list.
            match block {
                Block::Numbered(l) => {
                    let l = usize::from(l);
                    if let Some(n) = numbers.get_mut(l) {
                        *n += 1;
                    }
                    for deeper in numbers.iter_mut().skip(l + 1) {
                        *deeper = 0;
                    }
                }
                b if b.is_list() => {
                    for deeper in numbers.iter_mut().skip(usize::from(b.level())) {
                        *deeper = 0;
                    }
                }
                _ => numbers = [0; MAX_LEVEL as usize + 1],
            }
            let indent = "\u{2003}".repeat(usize::from(block.level()));
            let marker = match block {
                Block::Bullet(l) => {
                    let dot = match l % 3 {
                        0 => "•",
                        1 => "◦",
                        _ => "▪",
                    };
                    format!("{indent}{dot}\u{2002}")
                }
                Block::Numbered(l) => {
                    format!("{indent}{}.\u{2002}", numbers.get(usize::from(l)).copied().unwrap_or(1))
                }
                Block::Task { done, .. } => format!("{indent}{}\u{2002}", if done { "☑" } else { "☐" }),
                Block::Quote => "▍\u{2002}".to_owned(),
                _ => String::new(),
            };
            if i > 0 {
                out.push_text("\n");
            }
            let mstart = out.len();
            if !marker.is_empty() {
                out.push_styled(&marker, TextStylePatch::color(look.muted));
                markers.push((s, marker.len()));
                if let Block::Task { .. } = block {
                    tasks.push((mstart + indent.len()..mstart + marker.len(), i));
                }
            }
            let body = out.len();
            out.push_text(shown.get(s..e).unwrap_or(""));
            display_of.push((s, body));
            let line_range = body..out.len();
            match block {
                Block::Heading(l) => {
                    let [h1, h2, h3] = look.heading_sizes;
                    let size = match l {
                        1 => h1,
                        2 => h2,
                        _ => h3,
                    };
                    out.add_span(
                        line_range,
                        TextStylePatch { size: Some(size), weight: Some(700.0), ..TextStylePatch::default() },
                    );
                }
                Block::Quote => out.add_span(
                    line_range,
                    TextStylePatch {
                        italic: Some(true),
                        color: Some(look.muted),
                        ..TextStylePatch::default()
                    },
                ),
                Block::Code => out.add_span(
                    line_range,
                    TextStylePatch {
                        families: Some(look.code_families.clone()),
                        ..TextStylePatch::default()
                    },
                ),
                _ => {}
            }
        }
        // Marks, moved into display positions.
        let to_display = |at: usize| {
            let sh = to_shown(at);
            let line = display_of.iter().rposition(|(s, _)| *s <= sh).unwrap_or(0);
            display_of.get(line).map_or(sh, |(s, d)| d + (sh - s))
        };
        for (r, m) in &self.fmt.marks {
            let (a, b) = (to_display(r.start), to_display(r.end));
            if a >= b {
                continue;
            }
            let patch = match m {
                Mark::Bold => TextStylePatch::bold(),
                Mark::Italic => TextStylePatch::italic(),
                Mark::Strike => TextStylePatch::strikethrough(),
                Mark::Code => {
                    TextStylePatch { families: Some(look.code_families.clone()), ..TextStylePatch::default() }
                }
                Mark::Link(_) => TextStylePatch {
                    color: Some(look.link),
                    underline: Some(true),
                    ..TextStylePatch::default()
                },
                Mark::Mention(_) => TextStylePatch {
                    color: Some(look.mention),
                    weight: Some(600.0),
                    ..TextStylePatch::default()
                },
            };
            out.add_span(a..b, patch);
        }
        let composition =
            composition.map(|c| to_display_shown(&display_of, c.start)..to_display_shown(&display_of, c.end));
        RichView { text: out, composition, markers, comp, tasks }
    }
}

/// A shown-text position in display positions, given each line's shown and display starts.
fn to_display_shown(display_of: &[(usize, usize)], sh: usize) -> usize {
    let line = display_of.iter().rposition(|(s, _)| *s <= sh).unwrap_or(0);
    display_of.get(line).map_or(sh, |(s, d)| d + (sh - s))
}
