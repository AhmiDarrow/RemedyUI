//! Editable text: single-line, multi-line and secret fields.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::{Point, Rect, Size, Vec2};
use remedy_ui_host::{ImeEvent, TextInputState};
use remedy_ui_input::{Key, KeyEvent, Modifiers, Platform};
use remedy_ui_paint::{Brush, DisplayList};
use remedy_ui_style::{ResolvedStyle, Theme};
use remedy_ui_text::{
    Block, CodeEditor, Mark, Paragraph, ParagraphOptions, RichDoc, RichEditor, RichLook, RichText, RichView,
    SecretEditor, SecretString, TextEditor, TextEngine, TextPosition, TextSelection, TextStyle,
};
use remedy_ui_tree::Dirty;

use crate::decoration::{self, Decoration};
use crate::element::{El, Ui};
use crate::{Request, frame};

/// What kind of field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldKind {
    /// One line; Enter submits.
    Single,
    /// Several lines; Enter breaks the line.
    Multi,
    /// A password or other secret: masked, kept in a zeroizing buffer, never copied, no input
    /// method.
    Secret,
    /// Rich text: several lines with formatting (bold, links, mentions, lists, quotes, code).
    Rich,
    /// Code: several lines, never wrapped, several cursors, indentation that follows the code,
    /// colors from [`TextField::set_code_colors`].
    Code,
}

/// What colors code: for a text and the field's theme, colored byte ranges. It runs while
/// the view draws, so it reads the theme it is given, not the view.
pub type CodeColors = Rc<dyn Fn(&str, &Theme) -> Vec<(Range<usize>, remedy_ui_core::Color)>>;

/// A code field's editor and what colors it.
struct CodeState {
    ed: CodeEditor,
    colors: Option<CodeColors>,
    /// The theme the field was last styled with.
    theme: Theme,
    /// The colored text for the editor's version, made when needed.
    shown: std::cell::OnceCell<RichText>,
}

enum Editor {
    Plain(TextEditor),
    Secret(SecretEditor),
    Rich(Box<RichState>),
    Code(Box<CodeState>),
}

/// A rich field's editor, the look it is drawn with, and its laid-out view (made when needed).
struct RichState {
    ed: RichEditor,
    look: RichLook,
    view: std::cell::OnceCell<RichView>,
}

impl RichState {
    fn view(&self) -> &RichView {
        self.view.get_or_init(|| self.ed.view(&self.look))
    }

    fn reset(&mut self) {
        self.view = std::cell::OnceCell::new();
    }
}

/// How a rich field draws its document under `theme`.
pub(crate) fn rich_look(theme: &Theme) -> RichLook {
    let d = theme.derive();
    let c = &theme.colors;
    let link = remedy_ui_style::ensure_contrast(c.accent, c.bg, 4.5);
    RichLook {
        link,
        mention: link,
        muted: d.muted,
        code_families: theme.mono_families.clone(),
        heading_sizes: [d.sizes[4], d.sizes[3] * 1.15, d.sizes[3]],
    }
}

/// Sees a key before a field edits with it; true takes the key.
type KeyFilter = Rc<dyn Fn(&KeyEvent) -> bool>;

/// The mask shown for each character of a secret.
const MASK: char = '•';

/// The scroll along one axis that shows `lo..hi` in a view `len` long, moving as little as
/// it can from `at`.
fn follow(at: f32, lo: f32, hi: f32, len: f32) -> f32 {
    if hi - lo >= len || lo < at {
        lo
    } else if hi > at + len {
        hi - len
    } else {
        at
    }
}

/// A scroll kept within the text: no further than the text's end, never before its start.
fn clamp_scroll(s: Vec2, text: Size, view: Size) -> Vec2 {
    Vec2::new(
        s.x.clamp(0.0, (text.width - view.width).max(0.0)),
        s.y.clamp(0.0, (text.height - view.height).max(0.0)),
    )
}

pub(crate) struct Field {
    kind: FieldKind,
    editor: Editor,
    style: TextStyle,
    placeholder: String,
    /// What the field holds, for keyboards, input methods and password managers.
    purpose: remedy_ui_host::InputPurpose,
    /// The touch keyboard's Enter key, when the app picks it.
    enter: Option<remedy_ui_host::EnterKey>,
    /// The pseudo-locale placeholders are shown in, if one is active.
    pseudo: Option<remedy_ui_text::Pseudo>,
    label: Option<String>,
    /// A secret shown in the clear (the user pressed reveal).
    reveal: bool,
    scale: f32,
    /// The field lays out right to left: its text starts at the right edge.
    pub(crate) rtl: bool,
    /// Which side of a direction change the caret is on, after the arrow keys put it at a
    /// shown position where one index has two places (the end of an English word and the
    /// start of the Arabic one after it): `(shown index, side)`, valid while the caret stays.
    caret_side: Option<(usize, remedy_ui_text::Affinity)>,
    /// The paragraph shown, its width and where its top-left sat in the element.
    shown: Option<(f32, Rc<Paragraph>, Point)>,
    /// The paragraph laid out last, kept after the text changes: a huge paragraph laid out
    /// lazily is laid out again from it, keeping what the edit did not touch.
    last: Option<Rc<Paragraph>>,
    /// How far the text has scrolled inside the field's box, when it is taller (a multi-line
    /// field given a height) or wider (a single line longer than its field) than the box.
    scroll: Vec2,
    /// The caret and text version the scroll last brought into view: the view follows the
    /// caret when either moves, and otherwise stays where the wheel left it.
    followed: (usize, u64),
    /// The person typed, moved or pasted: the next paint scrolls the caret into view even
    /// where it did not move (End at the end after the wheel moved the text away).
    show_caret: bool,
    /// The text's size and the box's at the last paint (how far the wheel can scroll).
    extent: (Size, Size),
    on_change: Option<Rc<dyn Fn()>>,
    on_submit: Option<Rc<dyn Fn(remedy_ui_input::Modifiers)>>,
    /// Called after every change to the text, by a person or by the app.
    observers: Vec<Rc<dyn Fn()>>,
    version: u64,
    /// Lines drawn under ranges of the text, as of the text in `deco_text`.
    decorations: Vec<Decoration>,
    deco_text: String,
    /// The text changed since the decorations were last carried over to it.
    deco_stale: bool,
    /// Rich fields: Enter submits (a chat's composer) and Shift+Enter starts a new line.
    submit_on_enter: bool,
    /// Told when the caret or selection moves (and after edits).
    caret_observers: Vec<Rc<dyn Fn()>>,
    /// Sees keys before the field edits with them; true takes the key (a suggestion list).
    key_filter: Option<KeyFilter>,
    /// Copies and cuts go to the clipboard as secrets, cleared after this long
    /// ([`TextField::copy_as_secret`]).
    secret_copy: Option<std::time::Duration>,
    /// Told after the person copies or cuts text out of the field.
    on_copy: Option<Rc<dyn Fn()>>,
}

impl Field {
    pub(crate) fn invalidate(&mut self) {
        self.shown = None;
        if let Editor::Rich(r) = &mut self.editor {
            r.reset();
        }
        if let Editor::Code(c) = &mut self.editor {
            c.shown = std::cell::OnceCell::new();
        }
    }

    /// The editing core under a plain, rich or code field.
    fn core(&self) -> Option<&TextEditor> {
        match &self.editor {
            Editor::Plain(e) => Some(e),
            Editor::Rich(r) => Some(r.ed.editor()),
            Editor::Code(c) => Some(c.ed.editor()),
            Editor::Secret(_) => None,
        }
    }

    pub(crate) fn set_pseudo(&mut self, pseudo: Option<remedy_ui_text::Pseudo>) {
        self.pseudo = pseudo;
        self.invalidate();
    }

    pub(crate) fn set_style(&mut self, style: TextStyle, theme: &Theme) {
        self.style = style;
        if let Editor::Rich(r) = &mut self.editor {
            r.look = rich_look(theme);
        }
        if let Editor::Code(c) = &mut self.editor {
            c.theme = theme.clone();
        }
        self.invalidate();
    }

    /// Carries the decorations over edits made since they were set.
    fn sync_decorations(&mut self) {
        if !self.deco_stale {
            return;
        }
        self.deco_stale = false;
        if self.decorations.is_empty() {
            return;
        }
        let Some(e) = self.core() else {
            self.decorations.clear();
            return;
        };
        let now = e.text();
        decoration::carry(&self.deco_text, &now, &mut self.decorations);
        self.deco_text = now;
    }

    /// Where the input-method composition sits in the shown text: the text before it ends at
    /// `head`, and the text after it starts at `tail` in the text.
    fn composition_edges(&self) -> Option<(usize, usize, usize)> {
        let e = self.core()?;
        let c = e.composition()?;
        let sel = e.selection();
        let head = c.at.min(sel.anchor.min(sel.focus));
        let tail = sel.anchor.max(sel.focus).max(c.at);
        Some((head, tail, c.text.len()))
    }

    /// A range of the text as a range of the shown text (which has the input-method
    /// composition in place of the selection); `None` when the composition covers it.
    fn to_display(&self, r: &Range<usize>) -> Option<Range<usize>> {
        if let Editor::Rich(rs) = &self.editor {
            if let Some((head, tail, _)) = self.composition_edges()
                && r.end > head
                && r.start < tail.max(head + 1)
            {
                return None;
            }
            let v = rs.view();
            return Some(v.to_display(r.start)..v.to_display(r.end));
        }
        if !matches!(self.editor, Editor::Plain(_) | Editor::Code(_)) {
            return None;
        }
        let Some((head, tail, len)) = self.composition_edges() else { return Some(r.clone()) };
        if r.end <= head {
            Some(r.clone())
        } else if r.start >= tail {
            let shift = |t: usize| t - tail + head + len;
            Some(shift(r.start)..shift(r.end))
        } else {
            None
        }
    }

    /// An index of the shown text as an index of the text; `None` inside the composition.
    fn text_index(&self, i: usize) -> Option<usize> {
        if let Editor::Rich(rs) = &self.editor {
            let v = rs.view();
            if v.composition.as_ref().is_some_and(|c| c.start < i && i < c.end) {
                return None;
            }
            return Some(v.to_model(i));
        }
        if !matches!(self.editor, Editor::Plain(_) | Editor::Code(_)) {
            return None;
        }
        let Some((head, tail, len)) = self.composition_edges() else { return Some(i) };
        if i <= head {
            Some(i)
        } else if i >= head + len {
            Some(i - head - len + tail)
        } else {
            None
        }
    }

    fn display(&self) -> (String, Option<Range<usize>>) {
        match &self.editor {
            Editor::Plain(e) => e.display_text(),
            Editor::Code(c) => c.ed.editor().display_text(),
            Editor::Rich(r) => {
                let v = r.view();
                (v.text.as_str().to_owned(), v.composition.clone())
            }
            Editor::Secret(e) if self.reveal => (e.secret().expose().to_owned(), None),
            Editor::Secret(e) => (MASK.to_string().repeat(e.masked_len()), None),
        }
    }

    /// What is laid out: the display text with its styles.
    fn display_rich(&self) -> RichText {
        match &self.editor {
            Editor::Rich(r) => r.view().text.clone(),
            Editor::Code(c) => c
                .shown
                .get_or_init(|| {
                    let (text, comp) = c.ed.editor().display_text();
                    let mut rt = RichText::plain(text.clone());
                    // While composing, the text is shown as typed; colors come back after.
                    if let (Some(colors), None) = (&c.colors, comp) {
                        for (r, color) in colors(&text, &c.theme) {
                            if r.start < r.end && r.end <= text.len() {
                                rt.add_span(r, remedy_ui_text::TextStylePatch::color(color));
                            }
                        }
                    }
                    rt
                })
                .clone(),
            _ => RichText::plain(self.display().0),
        }
    }

    /// The selection in display-text bytes.
    fn display_selection(&self) -> Range<usize> {
        match &self.editor {
            Editor::Rich(r) => {
                let s = r.ed.selection().range();
                let v = r.view();
                v.to_display(s.start)..v.to_display(s.end)
            }
            Editor::Code(c) => c.ed.editor().selection().range(),
            Editor::Plain(e) => {
                let s = e.selection();
                s.anchor.min(s.focus)..s.anchor.max(s.focus)
            }
            Editor::Secret(e) if self.reveal => {
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

    /// The caret as a position in the shown paragraph, on the side the arrow keys left it.
    fn caret_position(&self, len: usize) -> TextPosition {
        let index = self.caret_index().min(len);
        let affinity = self.caret_side.filter(|(i, _)| *i == index).map(|(_, a)| a).unwrap_or_default();
        TextPosition { index, affinity }
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
            Editor::Secret(e) if self.reveal => e.selection().focus,
            Editor::Secret(e) => {
                let s = e.selection();
                e.secret().expose().get(..s.focus).map_or(0, |p| p.chars().count()) * MASK.len_utf8()
            }
            Editor::Rich(r) => {
                let v = r.view();
                match (r.ed.editor().composition(), &v.composition) {
                    (Some(c), Some(range)) => range.start + c.cursor.min(range.len()),
                    _ => v.to_display(r.ed.selection().focus),
                }
            }
            Editor::Code(c) => {
                let e = c.ed.editor();
                match (e.display_text().1, e.composition()) {
                    (Some(r), Some(k)) => r.start + k.cursor.min(r.len()),
                    _ => e.selection().focus,
                }
            }
        }
    }

    /// A code field's other cursors (the main one is drawn like any field's), with the
    /// composition-free text they index.
    fn extra_cursors(&self) -> Vec<remedy_ui_text::TextSelection> {
        match &self.editor {
            Editor::Code(c) if c.ed.editor().composition().is_none() => {
                c.ed.cursors().into_iter().skip(1).collect()
            }
            _ => Vec::new(),
        }
    }

    fn paragraph(&mut self, engine: &mut TextEngine, width: f32) -> Rc<Paragraph> {
        if let Some((w, p, _)) = &self.shown
            && *w == width
        {
            return p.clone();
        }
        let text = self.display_rich();
        // Code is never wrapped: lines stay as written, the field scrolls sideways.
        let wrap = matches!(self.kind, FieldKind::Multi | FieldKind::Rich).then_some(width);
        let options = {
            let mut v_ = ParagraphOptions::default();
            v_.max_width = wrap;
            v_.scale = self.scale;
            v_.container = crate::element::container(self.rtl);
            v_
        };
        let shown = if text.is_empty() { RichText::plain(" ") } else { text };
        let p = match self.last.take() {
            Some(before) if before.is_lazy() => {
                engine.layout_after_edit(&before, &shown, &self.style, options)
            }
            _ => engine.layout(&shown, &self.style, options),
        };
        let p = Rc::new(p);
        self.last = Some(p.clone());
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
        let (text, comp) = self.display();
        // Text taller or wider than the box scrolls inside it: to the caret when it or the
        // text moved while focused, otherwise where it was, never past the text's ends.
        let at = self.caret_position(text.len());
        let c = p.caret(at, 1.5);
        let x0 = p.x_offset(content.width());
        // The caret's box can reach past the last line's: the text ends where either ends.
        let size = Size::new(p.width().max(c.max.x + x0), p.height().max(c.max.y));
        self.extent = (size, content.size());
        let mark = (self.caret_index(), self.version);
        if focused && (mark != self.followed || self.show_caret) {
            self.scroll = Vec2::new(
                follow(self.scroll.x, c.min.x + x0, c.max.x + x0, content.width()),
                follow(self.scroll.y, c.min.y, c.max.y, content.height()),
            );
            self.followed = mark;
            self.show_caret = false;
        }
        self.scroll = clamp_scroll(self.scroll, size, content.size());
        let o = Point::new(
            content.min.x + p.x_offset(content.width()) - self.scroll.x,
            content.min.y - self.scroll.y,
        );
        if let Some(shown) = self.shown.as_mut() {
            shown.2 = o;
        }
        let tokens = theme.derive();
        // Nothing of the text draws outside the box; the caret may sit on its side edges.
        let clip = Rect::new(content.min.x - 2.0, content.min.y, content.max.x + 2.0, content.max.y);
        list.push_clip(clip, remedy_ui_core::CornerRadii::ZERO);
        if focused {
            for r in p.selection_rects(self.display_selection()) {
                list.fill_rect(r.translate(o.to_vec2()), Brush::Solid(tokens.selected));
            }
            for c in self.extra_cursors() {
                for r in p.selection_rects(c.range()) {
                    list.fill_rect(r.translate(o.to_vec2()), Brush::Solid(tokens.selected));
                }
            }
        }
        if text.is_empty() {
            if !self.placeholder.is_empty() {
                let style = {
                    let mut v_ = self.style.clone();
                    v_.color = tokens.muted;
                    v_
                };
                let plain = RichText::plain(self.placeholder.clone());
                let ph = engine.layout(&self.pseudo.map_or(plain.clone(), |p| p.apply(&plain)), &style, {
                    let mut v_ = ParagraphOptions::default();
                    v_.scale = self.scale;
                    v_.container = crate::element::container(self.rtl);
                    v_
                });
                frame::glyphs(
                    list,
                    &ph,
                    Point::new(content.min.x + ph.x_offset(content.width()), content.min.y),
                    tokens.muted,
                );
            }
        } else if p.is_lazy() {
            // A huge paragraph draws the lines in the box and a box's height more each way.
            let margin = content.height().max(600.0);
            let band = self.scroll.y - margin..self.scroll.y + content.height() + margin;
            frame::glyphs_in(list, &p, o, s.foreground, band);
        } else {
            frame::glyphs(list, &p, o, s.foreground);
        }
        self.sync_decorations();
        if !text.is_empty() {
            for d in &self.decorations {
                let Some(r) = self.to_display(&d.range) else { continue };
                let valid = r.start < r.end
                    && r.end <= text.len()
                    && text.is_char_boundary(r.start)
                    && text.is_char_boundary(r.end);
                if !valid {
                    continue;
                }
                let color = d.color.unwrap_or(theme.colors.danger);
                for line in p.selection_rects(r) {
                    decoration::draw(list, line.translate(o.to_vec2()), d.kind, color);
                }
            }
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
            let at = self.caret_position(text.len());
            let caret = p.caret(at, 1.5);
            list.fill_rect(caret.translate(o.to_vec2()), Brush::Solid(tokens.text));
            for c in self.extra_cursors() {
                let at = TextPosition { index: c.focus.min(text.len()), ..TextPosition::default() };
                list.fill_rect(p.caret(at, 1.5).translate(o.to_vec2()), Brush::Solid(tokens.text));
            }
        }
        list.pop_clip();
    }

    /// Scrolls text that overflows the box by a wheel's `delta`. False when the text cannot
    /// move that way (it fits, or is already at that end), so the scroll goes outward.
    pub(crate) fn wheel(&mut self, delta: Vec2) -> bool {
        let (text, view) = self.extent;
        let max = Vec2::new((text.width - view.width).max(0.0), (text.height - view.height).max(0.0));
        let can = |o: f32, m: f32, d: f32| (d > 0.0 && o < m) || (d < 0.0 && o > 0.0);
        if !can(self.scroll.x, max.x, delta.x) && !can(self.scroll.y, max.y, delta.y) {
            return false;
        }
        self.scroll = clamp_scroll(self.scroll + delta, text, view);
        true
    }

    pub(crate) fn semantics(&self) -> Semantics {
        let mut s = Semantics::new(match self.kind {
            FieldKind::Secret => Role::PasswordInput,
            FieldKind::Multi | FieldKind::Rich | FieldKind::Code => Role::MultilineTextInput,
            FieldKind::Single if self.purpose == remedy_ui_host::InputPurpose::Search => Role::SearchInput,
            FieldKind::Single => Role::TextInput,
        });
        if let Some(l) = &self.label {
            s = s.label(l.clone());
        }
        // Assistive technology may set a field's text (secrets are only ever typed).
        match &self.editor {
            Editor::Plain(e) => s.value(e.text()).action(remedy_ui_a11y::Action::SetValue),
            Editor::Rich(r) => s.value(r.ed.text()).action(remedy_ui_a11y::Action::SetValue),
            Editor::Code(c) => s.value(c.ed.text()).action(remedy_ui_a11y::Action::SetValue),
            // Secrets are never exposed, not even their length.
            Editor::Secret(_) => s,
        }
    }

    /// The input-method session for this field, with its caret in window coordinates.
    pub(crate) fn input_state(&self, bounds: Rect) -> TextInputState {
        let caret = self
            .shown
            .as_ref()
            .map(|(_, p, o)| p.caret(self.caret_position(usize::MAX), 1.0).translate(o.to_vec2()))
            .unwrap_or(Rect::ZERO)
            .translate(bounds.min.to_vec2());
        match self.core() {
            Some(e) => {
                let s = e.selection();
                {
                    let mut v_ = TextInputState::default();
                    v_.surrounding = e.text();
                    v_.selection = s.anchor.min(s.focus)..s.anchor.max(s.focus);
                    v_.caret = caret;
                    v_.private = false;
                    v_.multiline = matches!(self.kind, FieldKind::Multi | FieldKind::Rich | FieldKind::Code);
                    v_.numeric = self.purpose.is_numeric();
                    v_.purpose = self.purpose;
                    v_.enter = self.enter;
                    v_
                }
            }
            None => {
                let mut v_ = TextInputState::default();
                v_.surrounding = String::new();
                v_.selection = 0..0;
                v_.caret = caret;
                v_.private = true;
                v_.multiline = false;
                v_.numeric = self.purpose.is_numeric();
                // A secret is a password unless the app says it is a code or a PIN.
                v_.enter = self.enter;
                v_.purpose = match self.purpose {
                    remedy_ui_host::InputPurpose::Text => remedy_ui_host::InputPurpose::CurrentPassword,
                    p => p,
                };
                v_
            }
        }
    }

    /// Places the caret at a point in element coordinates. Returns true when that changed the
    /// content (a press on a task's box ticks it).
    pub(crate) fn place(&mut self, at: Point, extend: bool) -> bool {
        let Some((_, p, o)) = &self.shown else { return false };
        let pos = p.hit_test(Point::new(at.x - o.x, at.y - o.y)).index;
        let f_reveal = self.reveal;
        match &mut self.editor {
            Editor::Code(c) => {
                let anchor = if extend { c.ed.editor().selection().anchor } else { pos };
                if !extend {
                    c.ed.single_cursor();
                }
                c.ed.set_selection(anchor, pos);
            }
            Editor::Rich(r) => {
                let v = r.view();
                let task = if extend { None } else { v.task_at(pos) };
                let model = v.to_model(pos);
                if let Some(line) = task {
                    r.ed.toggle_task(line);
                    r.reset();
                    return true;
                }
                let anchor = if extend { r.ed.selection().anchor } else { model };
                r.ed.set_selection(anchor, model);
                r.reset();
            }
            Editor::Plain(e) => {
                let anchor = if extend { e.selection().anchor } else { pos };
                e.set_selection(anchor, pos);
            }
            Editor::Secret(e) if f_reveal => {
                let sel = e.selection();
                let anchor = if extend { sel.anchor } else { pos };
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
        false
    }

    /// Alt+click in a code field: another cursor at the point (element coordinates).
    pub(crate) fn add_cursor_at(&mut self, at: Point) -> bool {
        let Some((_, p, o)) = &self.shown else { return false };
        let pos = p.hit_test(Point::new(at.x - o.x, at.y - o.y)).index;
        let Editor::Code(c) = &mut self.editor else { return false };
        c.ed.add_cursor(pos, pos);
        true
    }

    fn changed(&mut self) {
        self.invalidate();
        self.version += 1;
        self.deco_stale = true;
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
    /// Copy this rich text out (as plain text and HTML).
    CopyRich(RichDoc),
    /// Read the clipboard.
    Paste,
}

pub(crate) fn key(f: &mut Field, e: &KeyEvent, now: u64, platform: Platform) -> KeyOutcome {
    let side = std::cell::Cell::new(f.caret_side);
    let out = field_key(f, e, now, platform, &side);
    f.caret_side = side.get();
    if !matches!(out, KeyOutcome::Ignored) {
        f.show_caret = true;
    }
    out
}

/// One press of Left or Right through text as it is shown: the caret moves to the visual
/// left or right (in Arabic or Hebrew, Right moves toward the start; through mixed text it
/// steps across runs the way they are drawn), by a character or by a word. Without Shift a
/// selection collapses to its left or right edge as drawn. `sel` and the result are in the
/// paragraph's (display) positions.
/// `side` is where the caret was left at a direction change, and where it is left now.
fn visual_step(
    p: &Paragraph,
    sel: TextSelection,
    right: bool,
    word: bool,
    extend: bool,
    side: &std::cell::Cell<Option<(usize, remedy_ui_text::Affinity)>>,
) -> TextSelection {
    let at = |i: usize| {
        let affinity = side.get().filter(|(s, _)| *s == i).map(|(_, a)| a).unwrap_or_default();
        TextPosition { index: i, affinity }
    };
    if !sel.is_caret() && !extend {
        let x = |i: usize| p.caret(at(i), 1.0).min.x;
        let (a, f) = (x(sel.anchor), x(sel.focus));
        let edge = if (a < f) == right { sel.focus } else { sel.anchor };
        side.set(None);
        return TextSelection::caret(edge);
    }
    let n = if word { p.move_word(at(sel.focus), right) } else { p.move_visual(at(sel.focus), right) };
    side.set(Some((n.index, n.affinity)));
    if extend { TextSelection { anchor: sel.anchor, focus: n.index } } else { TextSelection::caret(n.index) }
}

fn field_key(
    f: &mut Field,
    e: &KeyEvent,
    now: u64,
    platform: Platform,
    side: &std::cell::Cell<Option<(usize, remedy_ui_text::Affinity)>>,
) -> KeyOutcome {
    if !e.pressed || e.composing {
        return KeyOutcome::Ignored;
    }
    // The paragraph as last drawn, for moving through it as it looks.
    let shown = f.shown.as_ref().map(|(_, p, _)| p.clone());
    let rtl = f.rtl;
    // Outside Apple platforms Alt (without Ctrl, which with Alt is AltGr) never edits text:
    // Alt+Left is "back", Alt+letters reach menus. On Apple platforms Option moves by word.
    if platform != Platform::Apple && e.modifiers.alt && !e.modifiers.ctrl && e.text.is_none() {
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
                Key::ArrowLeft | Key::ArrowRight => {
                    let right = e.key == Key::ArrowRight;
                    match &shown {
                        // As drawn: the shown text is the editor's while nothing is composing.
                        Some(p) if ed.composition().is_none() && !ed.is_empty() => {
                            let s = visual_step(p, ed.selection(), right, word, extend, side);
                            ed.set_selection(s.anchor, s.focus);
                        }
                        // Not drawn yet: by the field's direction.
                        _ if word => ed.move_word(right != rtl, extend),
                        _ => ed.move_char(right != rtl, extend),
                    }
                    handled()
                }
                Key::Home | Key::End => {
                    ed.move_to_edge(e.key == Key::End, extend);
                    handled()
                }
                // Enter sends where the app asked for that (a chat's composer), and Shift+Enter
                // breaks the line; otherwise Enter breaks the line.
                Key::Enter if f.kind == FieldKind::Multi && f.submit_on_enter && !extend => {
                    KeyOutcome::Submit
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
        Editor::Code(c) => {
            let ed = &mut c.ed;
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
                // Ctrl+D: the word, then each next place it appears, as more cursors.
                _ if cmd && ch("d") => {
                    ed.select_next_occurrence();
                    handled()
                }
                // Ctrl+Alt+Up/Down (Option+Command on Apple): a cursor on the line above or below.
                Key::ArrowUp | Key::ArrowDown if cmd && e.modifiers.alt => {
                    ed.add_cursor_vertically(e.key == Key::ArrowDown);
                    handled()
                }
                Key::Escape if ed.cursors().len() > 1 => {
                    ed.single_cursor();
                    handled()
                }
                Key::Tab if !cmd => {
                    ed.indent(!extend, now);
                    edited()
                }
                Key::Backspace if word => {
                    ed.move_word(false, true);
                    ed.backspace(now);
                    edited()
                }
                Key::Backspace => {
                    ed.backspace(now);
                    edited()
                }
                Key::Delete if word => {
                    ed.move_word(true, true);
                    ed.delete_forward(now);
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
                    ed.move_line_edge(e.key == Key::End, extend);
                    handled()
                }
                Key::Enter => {
                    ed.enter(now);
                    edited()
                }
                _ if cmd || e.modifiers.alt && !e.modifiers.ctrl => KeyOutcome::Ignored,
                _ => match &e.text {
                    Some(t) if !t.is_empty() => {
                        ed.insert(t, now);
                        edited()
                    }
                    _ => KeyOutcome::Ignored,
                },
            };
            c.shown = std::cell::OnceCell::new();
            out
        }
        Editor::Rich(r) => {
            let digit = |d: &str| ch(d) || e.physical == format!("Digit{d}");
            // The shown paragraph and its map to the document, while nothing is composing.
            let visual = shown
                .as_ref()
                .filter(|_| r.ed.editor().composition().is_none())
                .map(|p| (p.clone(), r.view().clone()));
            let ed = &mut r.ed;
            let out = match &e.key {
                _ if cmd && ch("a") => {
                    ed.select_all();
                    handled()
                }
                _ if cmd && ch("c") => KeyOutcome::CopyRich(ed.selected()),
                _ if cmd && ch("x") => KeyOutcome::CopyRich(ed.cut(now)),
                _ if cmd && ch("v") => KeyOutcome::Paste,
                _ if cmd && ch("z") && !extend => KeyOutcome::Handled { changed: ed.undo() },
                _ if cmd && (ch("y") || (ch("z") && extend)) => KeyOutcome::Handled { changed: ed.redo() },
                // Formatting, with the shortcuts word processors use.
                _ if cmd && !extend && ch("b") => {
                    ed.toggle(Mark::Bold);
                    edited()
                }
                _ if cmd && !extend && ch("i") => {
                    ed.toggle(Mark::Italic);
                    edited()
                }
                _ if cmd && extend && (ch("x") || ch("X")) => {
                    ed.toggle(Mark::Strike);
                    edited()
                }
                _ if cmd && !extend && ch("e") => {
                    ed.toggle(Mark::Code);
                    edited()
                }
                _ if cmd && extend && digit("7") => {
                    ed.set_block(Block::Numbered(0));
                    edited()
                }
                _ if cmd && extend && digit("8") => {
                    ed.set_block(Block::Bullet(0));
                    edited()
                }
                _ if cmd && extend && digit("9") => {
                    ed.set_block(Block::Task { done: false, level: 0 });
                    edited()
                }
                _ if cmd && e.modifiers.alt && (digit("1") || digit("2") || digit("3")) => {
                    let level = if digit("1") {
                        1
                    } else if digit("2") {
                        2
                    } else {
                        3
                    };
                    ed.set_block(Block::Heading(level));
                    edited()
                }
                _ if cmd && e.modifiers.alt && digit("0") => {
                    ed.set_block(Block::Paragraph);
                    edited()
                }
                // Tab nests list items; elsewhere it moves focus as usual.
                Key::Tab if !cmd => {
                    if ed.indent(!extend) {
                        edited()
                    } else {
                        KeyOutcome::Ignored
                    }
                }
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
                Key::ArrowLeft | Key::ArrowRight => {
                    let right = e.key == Key::ArrowRight;
                    let before = ed.selection();
                    if let Some((p, v)) = &visual {
                        let shown = TextSelection {
                            anchor: v.to_display(before.anchor),
                            focus: v.to_display(before.focus),
                        };
                        let s = visual_step(p, shown, right, word, extend, side);
                        ed.set_selection(v.to_model(s.anchor), v.to_model(s.focus));
                    }
                    // Not drawn yet, or a step that stayed put (onto a list marker, which
                    // belongs to its line's start): by the field's direction.
                    if visual.is_none() || (ed.selection() == before && (before.is_caret() || extend)) {
                        if word {
                            ed.move_word(right != rtl, extend);
                        } else {
                            ed.move_char(right != rtl, extend);
                        }
                    }
                    handled()
                }
                Key::Home | Key::End => {
                    ed.move_to_edge(e.key == Key::End, extend);
                    handled()
                }
                // Enter sends where the app asked for that (a chat), Shift+Enter breaks the
                // line; otherwise Enter starts a new block and Shift+Enter breaks the line.
                Key::Enter if f.submit_on_enter && !extend => KeyOutcome::Submit,
                Key::Enter if extend && !f.submit_on_enter => {
                    ed.soft_break(now);
                    edited()
                }
                Key::Enter => {
                    ed.enter(now);
                    edited()
                }
                _ if cmd || e.modifiers.alt && !e.modifiers.ctrl => KeyOutcome::Ignored,
                _ => match &e.text {
                    Some(t) if !t.is_empty() => {
                        ed.insert(t, now);
                        edited()
                    }
                    _ => KeyOutcome::Ignored,
                },
            };
            r.reset();
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
    f.show_caret = true;
    if let Editor::Code(c) = &mut f.editor {
        let ed = &mut c.ed;
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
                let s = ed.editor().selection().focus;
                let (a, b) = (s.saturating_sub(*before), (s + after).min(ed.text().len()));
                if a < b {
                    ed.set_selection(a, b);
                    ed.backspace(now);
                }
            }
            ImeEvent::Enabled | ImeEvent::Disabled => return false,
        }
        c.shown = std::cell::OnceCell::new();
        return true;
    }
    if let Editor::Rich(r) = &mut f.editor {
        let ed = &mut r.ed;
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
                let (a, b) = (s.saturating_sub(*before), (s + after).min(ed.text().len()));
                if a < b {
                    ed.set_selection(a, b);
                    ed.backspace(now);
                }
            }
            ImeEvent::Enabled | ImeEvent::Disabled => return false,
        }
        r.reset();
        return true;
    }
    if let (Editor::Secret(_), ImeEvent::Commit(t)) = (&f.editor, e) {
        // Text inserted without keys (pasted, autofilled by a password manager, dictated)
        // lands as a commit. Compositions stay refused: their preedit shows in the clear.
        paste(f, t, now);
        return true;
    }
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
    f.show_caret = true;
    let text = if matches!(f.kind, FieldKind::Multi | FieldKind::Rich | FieldKind::Code) {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.replace(['\r', '\n'], " ")
    };
    match &mut f.editor {
        Editor::Plain(e) => e.paste(&text, now),
        Editor::Secret(e) => e.insert(&text),
        Editor::Rich(r) => {
            r.ed.paste_text(&text, now);
            r.reset();
        }
        Editor::Code(c) => {
            c.ed.paste(&text, now);
            c.shown = std::cell::OnceCell::new();
        }
    }
}

/// Replaces the whole text as one undoable edit (assistive technology setting a field's
/// value). Secret fields refuse: their contents are typed, never set from outside.
pub(crate) fn replace_all(f: &mut Field, text: &str, now: u64) -> bool {
    match &mut f.editor {
        Editor::Plain(e) => e.select_all(),
        Editor::Rich(r) => r.ed.select_all(),
        Editor::Code(c) => c.ed.select_all(),
        Editor::Secret(_) => return false,
    }
    paste(f, text, now);
    true
}

/// Pastes a rich document: into a rich field with its formatting, elsewhere as its text.
pub(crate) fn paste_doc(f: &mut Field, doc: &RichDoc, now: u64) {
    f.show_caret = true;
    match &mut f.editor {
        Editor::Rich(r) => {
            r.ed.paste(doc, now);
            r.reset();
        }
        _ => paste(f, &doc.text.replace(remedy_ui_text::SOFT_BREAK, "\n"), now),
    }
}

/// True for rich fields (they take HTML from the clipboard).
pub(crate) fn is_rich(f: &Field) -> bool {
    matches!(f.editor, Editor::Rich(_))
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
    #[track_caller]
    pub fn text_field(&self, widget: &'static str, kind: FieldKind) -> TextField {
        let el = self.el(widget).focusable(true);
        let scale = self.0.borrow().scale;
        let look = rich_look(&self.0.borrow().theme);
        let field = Rc::new(RefCell::new(Field {
            kind,
            editor: match kind {
                FieldKind::Secret => Editor::Secret(SecretEditor::new()),
                FieldKind::Rich => Editor::Rich(Box::new(RichState {
                    ed: RichEditor::new(RichDoc::plain("")),
                    look,
                    view: std::cell::OnceCell::new(),
                })),
                FieldKind::Code => Editor::Code(Box::new(CodeState {
                    ed: CodeEditor::new(""),
                    colors: None,
                    theme: self.0.borrow().theme.clone(),
                    shown: std::cell::OnceCell::new(),
                })),
                _ => Editor::Plain(TextEditor::new("")),
            },
            style: TextStyle::default(),
            placeholder: String::new(),
            purpose: remedy_ui_host::InputPurpose::Text,
            enter: None,
            pseudo: self.0.borrow().pseudo,
            label: None,
            reveal: false,
            scale,
            rtl: false,
            caret_side: None,
            shown: None,
            last: None,
            scroll: Vec2::ZERO,
            followed: (0, 0),
            show_caret: false,
            extent: (Size::ZERO, Size::ZERO),
            on_change: None,
            on_submit: None,
            observers: Vec::new(),
            version: 0,
            decorations: Vec::new(),
            deco_text: String::new(),
            deco_stale: false,
            submit_on_enter: false,
            caret_observers: Vec::new(),
            key_filter: None,
            secret_copy: None,
            on_copy: None,
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
            let wrap = if matches!(f.kind, FieldKind::Multi | FieldKind::Rich) { w } else { f32::MAX / 4.0 };
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
        // The wheel scrolls text that overflows the box; at an end it goes on outward.
        let (weak, id, fw) = (self.downgrade(), el.id, Rc::downgrade(&field));
        let el = el.on_wheel(move |e| {
            let Some(f) = fw.upgrade() else { return false };
            if e.modifiers.ctrl {
                return false;
            }
            let delta =
                if e.modifiers.shift && e.delta.x == 0.0 { Vec2::new(e.delta.y, 0.0) } else { e.delta };
            let moved = f.borrow_mut().wheel(delta);
            if moved && let Some(ui) = weak.upgrade() {
                ui.mark(id, Dirty::PAINT);
            }
            moved
        });
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
            Editor::Rich(r) => r.ed.text(),
            Editor::Code(c) => c.ed.text(),
            Editor::Secret(_) => String::new(),
        }
    }

    /// How many characters a secret field holds (the mask length), or the text length.
    pub fn len(&self) -> usize {
        match &self.field.borrow().editor {
            Editor::Plain(e) => e.text().chars().count(),
            Editor::Rich(r) => r.ed.text().chars().count(),
            Editor::Code(c) => c.ed.text().chars().count(),
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
        match &mut f.editor {
            Editor::Plain(e) => {
                *e = TextEditor::new(text);
                e.move_to_edge(true, false);
            }
            Editor::Rich(r) => r.ed.set_doc(RichDoc::plain(text)),
            Editor::Code(c) => c.ed = CodeEditor::new(text),
            Editor::Secret(_) => {}
        }
        f.changed();
        drop(f);
        self.el.ui.mark(self.el.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        notify_observers(&self.field);
    }

    /// Selects all of the text (a rename box opening with the old name ready to replace).
    pub fn select_all(&self) {
        let mut f = self.field.borrow_mut();
        match &mut f.editor {
            Editor::Plain(e) => e.select_all(),
            Editor::Rich(r) => r.ed.select_all(),
            Editor::Code(c) => c.ed.select_all(),
            Editor::Secret(_) => {}
        }
        f.changed();
        drop(f);
        self.el.ui.mark(self.el.id, Dirty::PAINT | Dirty::SEMANTICS);
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
            Editor::Rich(r) => {
                let mut s = SecretString::new();
                s.insert(0, &r.ed.text());
                s
            }
            Editor::Code(c) => {
                let mut s = SecretString::new();
                s.insert(0, &c.ed.text());
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

    /// Marks the field as holding a number, so touch keyboards show their number pad:
    /// [`purpose`](Self::purpose) with [`InputPurpose::Decimal`](remedy_ui_host::InputPurpose::Decimal).
    pub fn numeric(self) -> Self {
        self.purpose(remedy_ui_host::InputPurpose::Decimal)
    }

    /// Says what the field holds: touch keyboards show the matching keys, input methods
    /// switch mode, and password managers and autofill know what to offer. A secret field
    /// is [`CurrentPassword`](remedy_ui_host::InputPurpose::CurrentPassword) unless set.
    pub fn purpose(self, purpose: remedy_ui_host::InputPurpose) -> Self {
        self.field.borrow_mut().purpose = purpose;
        self
    }

    /// Picks the touch keyboard's Enter key: [`Next`](remedy_ui_host::EnterKey::Next) on
    /// every field of a form but the last moves on to the next field, as Tab does. Unset,
    /// it follows the field (see [`TextInputState::enter_key`](remedy_ui_host::TextInputState::enter_key)).
    pub fn enter_key(self, key: remedy_ui_host::EnterKey) -> Self {
        self.field.borrow_mut().enter = Some(key);
        self
    }

    /// Shows a secret field's text in the clear (`true`) or masked. The text stays protected
    /// from assistive technology, the clipboard and input methods either way.
    pub fn set_reveal(&self, on: bool) {
        let mut f = self.field.borrow_mut();
        f.reveal = on && f.kind == FieldKind::Secret;
        f.changed();
        drop(f);
        self.el.ui.mark(self.el.id, Dirty::LAYOUT | Dirty::PAINT);
    }

    /// True while a secret is shown in the clear.
    pub fn is_revealed(&self) -> bool {
        self.field.borrow().reveal
    }

    /// The name assistive technology reads for the field.
    pub fn label(self, text: impl Into<String>) -> Self {
        self.field.borrow_mut().label = Some(text.into());
        self.el.ui.mark(self.el.id, Dirty::SEMANTICS);
        self
    }

    /// Called after every edit.
    pub fn on_change(self, f: impl Fn() + 'static) -> Self {
        self.field.borrow_mut().on_change = Some(Rc::new(f));
        self
    }

    /// Calls `f` after every change to the text, whether a person edited it or the app set it
    /// ([`TextField::set_value`]). Unlike [`TextField::on_change`], any number of observers can
    /// be added, and they never replace the app's own `on_change`.
    pub fn observe_text(&self, f: impl Fn() + 'static) {
        self.field.borrow_mut().observers.push(Rc::new(f));
    }

    /// The selection as a byte range of the text (empty at the caret). Secret fields report
    /// an empty range.
    pub fn selection(&self) -> Range<usize> {
        self.field.borrow().core().map_or(0..0, |e| e.selection().range())
    }

    /// The caret as a byte index of the text (the selection's moving end).
    pub fn caret(&self) -> usize {
        self.field.borrow().core().map_or(0, |e| e.selection().focus)
    }

    /// Replaces the bytes in `range` with `text` as one undoable edit, caret after it, and
    /// reports the change like a typed edit. A range off the text or inside a character is
    /// refused (`false`), and secret fields refuse every range.
    pub fn replace_range(&self, range: Range<usize>, text: &str) -> bool {
        {
            let mut f = self.field.borrow_mut();
            let Some(whole) = f.core().map(TextEditor::text) else { return false };
            if range.start > range.end
                || range.end > whole.len()
                || !whole.is_char_boundary(range.start)
                || !whole.is_char_boundary(range.end)
            {
                return false;
            }
            match &mut f.editor {
                Editor::Plain(e) => {
                    e.set_selection(range.start, range.end);
                    e.paste(text, 0);
                }
                Editor::Rich(r) => {
                    r.ed.set_selection(range.start, range.end);
                    r.ed.paste_text(text, 0);
                    r.reset();
                }
                Editor::Code(c) => {
                    c.ed.single_cursor();
                    c.ed.set_selection(range.start, range.end);
                    c.ed.paste(text, 0);
                    c.shown = std::cell::OnceCell::new();
                }
                Editor::Secret(_) => return false,
            }
        }
        self.el.ui.field_edited(self.el.id, &self.field, true);
        true
    }

    /// Draws lines under ranges of the text, replacing the earlier ones. Ranges are byte
    /// ranges of the current text. Secret fields draw none.
    pub fn set_decorations(&self, decorations: Vec<Decoration>) {
        let text = self.value();
        self.set_decorations_for(&text, decorations);
    }

    /// Like [`TextField::set_decorations`] for ranges worked out on `text`, a copy of the
    /// field's text from some time ago (a check that ran in the background). Edits made since
    /// are carried over: lines before an edit stay, lines after it move with the text, and
    /// lines the edit touched are dropped.
    pub fn set_decorations_for(&self, text: &str, decorations: Vec<Decoration>) {
        {
            let mut f = self.field.borrow_mut();
            if f.kind == FieldKind::Secret {
                return;
            }
            f.decorations = decorations;
            f.deco_text = text.to_owned();
            f.deco_stale = true;
        }
        self.el.ui.mark(self.el.id, Dirty::PAINT);
    }

    /// The lines now drawn, moved over any edits since they were set.
    pub fn decorations(&self) -> Vec<Decoration> {
        let mut f = self.field.borrow_mut();
        f.sync_decorations();
        f.decorations.clone()
    }

    /// The decoration under `point` (window coordinates), once the field has been drawn.
    pub fn decoration_at(&self, point: Point) -> Option<Decoration> {
        let bounds = self.el.bounds()?;
        let mut f = self.field.borrow_mut();
        f.sync_decorations();
        let (_, p, origin) = f.shown.clone()?;
        let at = Point::new(point.x - bounds.min.x - origin.x, point.y - bounds.min.y - origin.y);
        let hit = |r: &Rect| at.x >= r.min.x && at.x < r.max.x && at.y >= r.min.y && at.y < r.max.y;
        f.decorations
            .iter()
            .find(|d| f.to_display(&d.range).is_some_and(|r| p.selection_rects(r).iter().any(hit)))
            .cloned()
    }

    /// The byte index of the text nearest `point` (window coordinates), once the field has
    /// been drawn; `None` inside an input-method composition.
    pub fn index_at(&self, point: Point) -> Option<usize> {
        let bounds = self.el.bounds()?;
        let f = self.field.borrow();
        let (_, p, origin) = f.shown.clone()?;
        let at = Point::new(point.x - bounds.min.x - origin.x, point.y - bounds.min.y - origin.y);
        f.text_index(p.hit_test(at).index)
    }

    /// Sets what colors a code field's text (syntax colors); the text is colored again on
    /// every change.
    pub fn set_code_colors(&self, colors: CodeColors) {
        {
            let mut f = self.field.borrow_mut();
            if let Editor::Code(c) = &mut f.editor {
                c.colors = Some(colors);
            }
            f.invalidate();
        }
        self.el.ui.mark(self.el.id, Dirty::PAINT);
    }

    /// Runs `f` on a code field's editor (adding cursors, indenting from a menu), then shows
    /// and reports the change like an edit. Returns `None` for other fields.
    pub fn edit_code<T>(&self, f: impl FnOnce(&mut CodeEditor) -> T) -> Option<T> {
        let (out, before) = {
            let mut field = self.field.borrow_mut();
            let Editor::Code(c) = &mut field.editor else { return None };
            let before = c.ed.version();
            let out = f(&mut c.ed);
            c.shown = std::cell::OnceCell::new();
            (out, before)
        };
        let changed = matches!(&self.field.borrow().editor, Editor::Code(c) if c.ed.version() != before);
        self.el.ui.field_edited(self.el.id, &self.field, changed);
        Some(out)
    }

    /// Reads a code field's editor (its cursors).
    pub fn read_code<T>(&self, f: impl FnOnce(&CodeEditor) -> T) -> Option<T> {
        match &self.field.borrow().editor {
            Editor::Code(c) => Some(f(&c.ed)),
            _ => None,
        }
    }

    /// The rich document (rich fields only).
    pub fn doc(&self) -> Option<RichDoc> {
        match &self.field.borrow().editor {
            Editor::Rich(r) => Some(r.ed.doc()),
            _ => None,
        }
    }

    /// Replaces a rich field's document, caret at the end, starting a fresh undo history.
    pub fn set_doc(&self, doc: RichDoc) {
        {
            let mut f = self.field.borrow_mut();
            let Editor::Rich(r) = &mut f.editor else { return };
            r.ed.set_doc(doc);
            f.changed();
        }
        self.el.ui.mark(self.el.id, Dirty::LAYOUT | Dirty::PAINT | Dirty::SEMANTICS);
        notify_observers(&self.field);
    }

    /// Runs `f` on a rich field's editor (a toolbar's bold button, a link dialog, a mention
    /// picker), then shows and reports the change like an edit. Returns `None` for other
    /// fields.
    pub fn edit_rich<T>(&self, f: impl FnOnce(&mut RichEditor) -> T) -> Option<T> {
        let (out, before) = {
            let mut field = self.field.borrow_mut();
            let Editor::Rich(r) = &mut field.editor else { return None };
            let before = r.ed.version();
            let out = f(&mut r.ed);
            r.reset();
            (out, before)
        };
        let changed = matches!(&self.field.borrow().editor, Editor::Rich(r) if r.ed.version() != before);
        self.el.ui.field_edited(self.el.id, &self.field, changed);
        Some(out)
    }

    /// Reads a rich field's editor (its selection, the marks at the caret, the block).
    pub fn read_rich<T>(&self, f: impl FnOnce(&RichEditor) -> T) -> Option<T> {
        match &self.field.borrow().editor {
            Editor::Rich(r) => Some(f(&r.ed)),
            _ => None,
        }
    }

    /// The caret's box in window coordinates, once the field has been drawn (to place a
    /// popup at it, such as mention suggestions).
    pub fn caret_rect(&self) -> Option<Rect> {
        let bounds = self.el.bounds()?;
        // Right after an edit the text is laid out again only when drawn; lay it out now.
        if self.field.borrow().shown.is_none() {
            let (engine, style) = {
                let i = self.el.ui.0.borrow();
                (i.engine.clone(), i.els.get(&self.el.id).map(|d| d.resolved.clone())?)
            };
            let inset = style.padding.start + style.border_width;
            let width = (bounds.width() - inset - style.padding.end - style.border_width).max(1.0);
            let mut f = self.field.borrow_mut();
            let p = f.paragraph(&mut engine.borrow_mut(), width);
            let o = Point::new(
                inset + p.x_offset(width) - f.scroll.x,
                style.padding.top + style.border_width - f.scroll.y,
            );
            if let Some(shown) = f.shown.as_mut() {
                shown.2 = o;
            }
        }
        let f = self.field.borrow();
        let (_, p, o) = f.shown.clone()?;
        let at = f.caret_position(usize::MAX);
        Some(p.caret(at, 1.0).translate(o.to_vec2()).translate(bounds.min.to_vec2()))
    }

    /// Calls `f` whenever the caret or the selection moves, and after every edit (a toolbar
    /// showing the formatting where the caret is).
    pub fn observe_caret(&self, f: impl Fn() + 'static) {
        self.field.borrow_mut().caret_observers.push(Rc::new(f));
    }

    /// Lets `f` see each key before the field does while the field has focus; when it returns
    /// true the key is taken (a list of suggestions taking the arrow keys and Enter).
    pub fn intercept_keys(&self, f: impl Fn(&KeyEvent) -> bool + 'static) {
        self.field.borrow_mut().key_filter = Some(Rc::new(f));
    }

    /// Copies and cuts out of this field (Ctrl+C and Ctrl+X, Command on Apple platforms) go
    /// to the clipboard as a secret: kept out of clipboard history and cloud sync where the
    /// platform allows, and cleared after `keep` if the clipboard still holds them (anything
    /// the person copied since is left alone), as `App::copy_secret` does. For fields whose
    /// text is itself secret but must be seen and edited (a note of passwords, an API key
    /// being written); a [`FieldKind::Secret`] field never copies at all. Rich fields copy
    /// plain text only, so no formatted copy outlives the clear.
    pub fn copy_as_secret(self, keep: std::time::Duration) -> Self {
        self.field.borrow_mut().secret_copy = Some(keep);
        self
    }

    /// How long copies out of this field stay on the clipboard, when they go there as
    /// secrets ([`TextField::copy_as_secret`]).
    pub fn secret_copy(&self) -> Option<std::time::Duration> {
        self.field.borrow().secret_copy
    }

    /// Called after the person copies or cuts text out of the field (not when nothing was
    /// selected), such as to say "Copied · clears in 30s".
    pub fn on_copy(self, f: impl Fn() + 'static) -> Self {
        self.field.borrow_mut().on_copy = Some(Rc::new(f));
        self
    }

    /// Multi-line and rich fields: Enter submits ([`TextField::on_submit`]) and Shift+Enter
    /// starts a new line, as a chat's composer does. Off by default (single-line fields always
    /// submit on Enter).
    pub fn submit_on_enter(self, on: bool) -> Self {
        self.field.borrow_mut().submit_on_enter = on;
        self
    }

    /// Called when Enter is pressed in a single-line or secret field, or in a multi-line or
    /// rich field that [`TextField::submit_on_enter`].
    pub fn on_submit(self, f: impl Fn() + 'static) -> Self {
        self.field.borrow_mut().on_submit = Some(Rc::new(move |_| f()));
        self
    }

    /// Called when Enter is pressed in a single-line or secret field, or in a multi-line or
    /// rich field that [`TextField::submit_on_enter`]. with the modifiers
    /// held (Shift+Enter goes back in a find bar).
    pub fn on_submit_with(self, f: impl Fn(remedy_ui_input::Modifiers) + 'static) -> Self {
        self.field.borrow_mut().on_submit = Some(Rc::new(f));
        self
    }
}

pub(crate) fn notify_change(field: &Rc<RefCell<Field>>) {
    let cb = field.borrow().on_change.clone();
    if let Some(cb) = cb {
        cb();
    }
    notify_observers(field);
}

fn notify_observers(field: &Rc<RefCell<Field>>) {
    let observers = field.borrow().observers.clone();
    for o in observers {
        o();
    }
}

pub(crate) fn notify_caret(field: &Rc<RefCell<Field>>) {
    let observers = field.borrow().caret_observers.clone();
    for o in observers {
        o();
    }
}

pub(crate) fn secret_copy(field: &Rc<RefCell<Field>>) -> Option<std::time::Duration> {
    field.borrow().secret_copy
}

pub(crate) fn notify_copy(field: &Rc<RefCell<Field>>) {
    let cb = field.borrow().on_copy.clone();
    if let Some(cb) = cb {
        cb();
    }
}

pub(crate) fn key_filter(field: &Rc<RefCell<Field>>) -> Option<KeyFilter> {
    field.borrow().key_filter.clone()
}

pub(crate) fn notify_submit(field: &Rc<RefCell<Field>>, modifiers: remedy_ui_input::Modifiers) {
    let cb = field.borrow().on_submit.clone();
    if let Some(cb) = cb {
        cb(modifiers);
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
