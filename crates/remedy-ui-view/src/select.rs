//! Selecting text that is not editable, as on a web page: what people wrote in a chat, an
//! article, an error to report. Text opts in with [`El::selectable`]. A press on it starts
//! a selection (a second press selects a word, a third all of that text), dragging extends it
//! across every selectable text in reading order, and Ctrl+C (Command+C) copies it while no
//! field has focus.

use std::ops::Range;

use remedy_ui_core::Point;
use remedy_ui_input::{Buttons, PointerEvent, PointerPhase};
use remedy_ui_tree::{Dirty, ElementId};

use crate::{El, Ui};

/// A selection over text: where it started and where it is now, each an element and a byte
/// offset in its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextSel {
    pub(crate) anchor: (ElementId, usize),
    pub(crate) focus: (ElementId, usize),
}

impl El {
    /// Lets people select (and copy) this text. Buttons and other controls keep their own
    /// presses; text inside them is never selected.
    pub fn selectable(self, on: bool) -> Self {
        self.edit(Dirty::NONE, move |d| d.selectable = on);
        self
    }

    /// Marks parts of this text (byte ranges), drawn under the glyphs in the theme's mark
    /// color: what a search found. An empty list clears the marks.
    pub fn mark_text(&self, ranges: Vec<Range<usize>>) {
        if self.ui.0.borrow().els.get(&self.id).is_some_and(|d| d.marks == ranges) {
            return;
        }
        self.edit(Dirty::PAINT, move |d| d.marks = ranges);
    }

    /// This element's text, if it shows text.
    pub fn text_content(&self) -> Option<String> {
        let i = self.ui.0.borrow();
        let d = i.els.get(&self.id)?;
        let slot = d.text.as_ref()?;
        Some(crate::element::with_alts(&slot.borrow().text, &d.inline_images, None))
    }
}

impl Ui {
    /// The selected text in reading order, the parts of separate texts on lines of their own.
    pub fn selected_text(&self) -> Option<String> {
        let ranges = self.selection_ranges()?;
        let i = self.0.borrow();
        let parts: Vec<String> = ranges
            .iter()
            .filter_map(|(id, r)| {
                let d = i.els.get(id)?;
                let slot = d.text.as_ref()?;
                let s = slot.borrow();
                s.text.as_str().get(r.clone())?;
                Some(crate::element::with_alts(&s.text, &d.inline_images, Some(r.clone())))
            })
            .filter(|p| !p.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// Selects bytes `range` of `el`'s selectable text (a search's current match).
    pub fn select_text(&self, el: &El, range: Range<usize>) {
        let ok = self.0.borrow().els.get(&el.id).is_some_and(|d| d.selectable && d.text.is_some());
        if !ok {
            return;
        }
        self.0.borrow_mut().text_sel =
            Some(TextSel { anchor: (el.id, range.start), focus: (el.id, range.end) });
        self.apply_selection();
    }

    /// Selects nothing.
    pub fn clear_text_selection(&self) {
        if self.0.borrow().text_sel.is_some() {
            self.0.borrow_mut().text_sel = None;
            self.apply_selection();
        }
    }

    /// The selectable texts in reading order (the order of the element tree).
    fn reading_order(&self) -> Vec<ElementId> {
        let tree = self.tree();
        let i = self.0.borrow();
        let mut out = Vec::new();
        let mut stack = vec![tree.root()];
        while let Some(id) = stack.pop() {
            if i.els.get(&id).is_some_and(|d| d.selectable && d.text.is_some()) {
                out.push(id);
            }
            let mut kids = tree.children(id);
            kids.reverse();
            stack.extend(kids);
        }
        out
    }

    /// Each selected text and the bytes of it selected, in reading order.
    fn selection_ranges(&self) -> Option<Vec<(ElementId, Range<usize>)>> {
        let sel = self.0.borrow().text_sel?;
        let order = self.reading_order();
        let a = order.iter().position(|id| *id == sel.anchor.0)?;
        let f = order.iter().position(|id| *id == sel.focus.0)?;
        let ((s_el, s_at), (e_el, e_at), si, ei) = if (a, sel.anchor.1) <= (f, sel.focus.1) {
            (sel.anchor, sel.focus, a, f)
        } else {
            (sel.focus, sel.anchor, f, a)
        };
        let len = |id: ElementId| {
            self.0
                .borrow()
                .els
                .get(&id)
                .and_then(|d| d.text.as_ref().map(|t| t.borrow().text.as_str().len()))
                .unwrap_or(0)
        };
        let mut out = Vec::new();
        for (k, id) in order.iter().enumerate().take(ei + 1).skip(si) {
            let start = if *id == s_el && k == si { s_at } else { 0 };
            let end = if *id == e_el && k == ei { e_at } else { len(*id) };
            if start < end {
                out.push((*id, start..end));
            }
        }
        Some(out)
    }

    /// Hands each text its highlighted range and repaints the ones that changed.
    pub(crate) fn apply_selection(&self) {
        let ranges = self.selection_ranges().unwrap_or_default();
        let mut changed = Vec::new();
        {
            let mut i = self.0.borrow_mut();
            for (id, d) in i.els.iter_mut() {
                let want = ranges.iter().find(|(r, _)| r == id).map(|(_, r)| r.clone());
                if d.sel != want {
                    d.sel = want;
                    changed.push(*id);
                }
            }
        }
        for id in changed {
            self.mark(id, Dirty::PAINT);
        }
    }

    /// The byte in text element `id` nearest the window point `p`.
    fn text_index_at(&self, id: ElementId, p: Point) -> Option<usize> {
        let b = self.tree().bounds(id)?;
        let i = self.0.borrow();
        let d = i.els.get(&id)?;
        let slot = d.text.as_ref()?.borrow();
        let (_, para) = slot.cache.as_ref()?;
        let s = &d.resolved;
        let ins = s.inner_insets();
        let left = b.min.x + ins.start;
        let top = b.min.y + ins.top;
        let width = b.width() - ins.start - ins.end;
        let x = left + para.x_offset(width.max(0.0));
        let pos = para.hit_test(Point::new(p.x - x, p.y - top));
        Some(pos.index.min(slot.text.as_str().len()))
    }

    /// The word around `p` in text element `id`.
    fn word_at(&self, id: ElementId, p: Point) -> Option<Range<usize>> {
        let b = self.tree().bounds(id)?;
        let i = self.0.borrow();
        let d = i.els.get(&id)?;
        let slot = d.text.as_ref()?.borrow();
        let (_, para) = slot.cache.as_ref()?;
        let s = &d.resolved;
        let ins = s.inner_insets();
        let left = b.min.x + ins.start;
        let top = b.min.y + ins.top;
        let width = b.width() - ins.start - ins.end;
        let x = left + para.x_offset(width.max(0.0));
        Some(para.word_at(Point::new(p.x - x, p.y - top)))
    }

    /// The selectable text a press on `chain` (hit first, then its ancestors) would select:
    /// none when a control, field or drag source is in the way.
    fn selectable_in(&self, chain: &[ElementId]) -> Option<ElementId> {
        let i = self.0.borrow();
        for id in chain {
            let d = i.els.get(id)?;
            if d.click.is_some() || d.field.is_some() || d.drag_source.is_some() {
                return None;
            }
            if d.selectable && d.text.is_some() {
                return Some(*id);
            }
        }
        None
    }

    /// Pointer input for selecting text; true when it took the event.
    pub(crate) fn select_pointer(&self, e: &PointerEvent, chain: &[ElementId], clicks: u32) -> bool {
        match e.phase {
            PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => {
                let Some(id) = self.selectable_in(chain) else {
                    self.0.borrow_mut().selecting = false;
                    self.clear_text_selection();
                    return false;
                };
                let len = self
                    .0
                    .borrow()
                    .els
                    .get(&id)
                    .and_then(|d| d.text.as_ref().map(|t| t.borrow().text.as_str().len()));
                let (start, end) = match clicks {
                    1 => {
                        let at = self.text_index_at(id, e.position).unwrap_or(0);
                        (at, at)
                    }
                    2 => self.word_at(id, e.position).map_or((0, 0), |r| (r.start, r.end)),
                    _ => (0, len.unwrap_or(0)),
                };
                let extend = e.modifiers.shift && self.0.borrow().text_sel.is_some();
                {
                    let mut i = self.0.borrow_mut();
                    i.selecting = true;
                    i.text_sel = Some(match (extend, i.text_sel) {
                        (true, Some(s)) => TextSel { anchor: s.anchor, focus: (id, end) },
                        _ => TextSel { anchor: (id, start), focus: (id, end) },
                    });
                }
                self.apply_selection();
                true
            }
            PointerPhase::Move if self.0.borrow().selecting => {
                if !e.buttons.contains(Buttons::PRIMARY) {
                    self.0.borrow_mut().selecting = false;
                    return false;
                }
                let target = {
                    let i = self.0.borrow();
                    chain
                        .iter()
                        .copied()
                        .find(|id| i.els.get(id).is_some_and(|d| d.selectable && d.text.is_some()))
                };
                if let Some(id) = target
                    && let Some(at) = self.text_index_at(id, e.position)
                {
                    if let Some(s) = self.0.borrow_mut().text_sel.as_mut() {
                        s.focus = (id, at);
                    }
                    self.apply_selection();
                }
                true
            }
            PointerPhase::Up | PointerPhase::Cancel if self.0.borrow().selecting => {
                self.0.borrow_mut().selecting = false;
                true
            }
            _ => false,
        }
    }
}
