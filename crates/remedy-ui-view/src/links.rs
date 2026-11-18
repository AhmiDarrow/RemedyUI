//! Links inside text, as on a web page: words in a paragraph that take you somewhere. A text
//! element's spans marked with [`TextPatch::link`](crate::TextPatch) become links once the
//! element has [`El::on_link`]. Each link is a real element placed over its words: Tab reaches
//! it, Enter or a click follows it, a screen reader reads it as a link inside the text, and
//! the focus ring shows where it is. A link that wraps onto further lines follows from a press
//! on any of its words.

use std::ops::Range;
use std::rc::Rc;

use remedy_ui_a11y::{Action, Role, Semantics};
use remedy_ui_core::{Point, Rect};
use remedy_ui_tree::{Dirty, ElementId};

use crate::element::Inner;
use crate::{El, Ui};

/// What follows a text's links.
type LinkHandler = Rc<dyn Fn(u64)>;

/// A text element's links: who follows them, and the element standing for each.
pub(crate) struct TextLinks {
    handler: LinkHandler,
    /// Each link's id, its words, and its element.
    shown: Vec<(u64, String, ElementId)>,
}

impl El {
    /// Lets people follow the links in this text: `f` gets the id of the link followed.
    /// Mark the linked words with [`TextPatch::link`](crate::TextPatch) when building the
    /// text.
    pub fn on_link(self, f: impl Fn(u64) + 'static) -> Self {
        let handler: LinkHandler = Rc::new(f);
        {
            let mut i = self.ui.0.borrow_mut();
            if let Some(d) = i.els.get_mut(&self.id) {
                match &mut d.links {
                    Some(l) => l.handler = handler,
                    None => d.links = Some(TextLinks { handler, shown: Vec::new() }),
                }
            }
        }
        sync(&self);
        self
    }

    /// The elements standing for the links in this text, in text order.
    pub fn link_elements(&self) -> Vec<El> {
        let i = self.ui.0.borrow();
        i.els
            .get(&self.id)
            .and_then(|d| d.links.as_ref())
            .map(|l| l.shown.iter().map(|(_, _, id)| El { ui: self.ui.clone(), id: *id }).collect())
            .unwrap_or_default()
    }
}

/// Each link in `text`: its id and its words (spans sharing an id are one link), in order.
fn links_of(text: &remedy_ui_text::RichText) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, Vec<Range<usize>>)> = Vec::new();
    for s in text.spans() {
        let Some(id) = s.patch.link else { continue };
        match out.iter_mut().find(|(k, _)| *k == id) {
            Some((_, r)) => r.push(s.range.clone()),
            None => out.push((id, vec![s.range.clone()])),
        }
    }
    out.into_iter()
        .map(|(id, ranges)| {
            let words: Vec<&str> = ranges.iter().filter_map(|r| text.as_str().get(r.clone())).collect();
            (id, words.join(" "))
        })
        .collect()
}

/// Makes the element's link elements match its text's links.
pub(crate) fn sync(el: &El) {
    let (want, handler, old) = {
        let i = el.ui.0.borrow();
        let Some(d) = i.els.get(&el.id) else { return };
        let Some(links) = &d.links else { return };
        let Some(slot) = &d.text else { return };
        let want = links_of(&slot.borrow().text);
        let old: Vec<(u64, String)> = links.shown.iter().map(|(k, w, _)| (*k, w.clone())).collect();
        if want == old {
            return;
        }
        (want, links.handler.clone(), links.shown.iter().map(|(_, _, id)| *id).collect::<Vec<_>>())
    };
    for id in old {
        El { ui: el.ui.clone(), id }.remove();
    }
    let mut shown = Vec::new();
    for (link, words) in want {
        let h = handler.clone();
        let child = el
            .ui
            .el("text_link")
            .semantics(Semantics::new(Role::Link).label(words.clone()).action(Action::Click))
            .on_click(move || h(link));
        child.edit(Dirty::NONE, |d| d.inline_link = Some(link));
        let _ = el.clone().child(child.clone());
        shown.push((link, words, child.id));
    }
    let mut i = el.ui.0.borrow_mut();
    if let Some(l) = i.els.get_mut(&el.id).and_then(|d| d.links.as_mut()) {
        l.shown = shown;
    }
}

/// Where the text element `parent` (a `size` box) shows link `link`, in the element's local
/// pixels: each line's piece.
fn pieces(i: &Inner, parent: ElementId, link: u64, width: f32) -> Vec<Rect> {
    let Some(d) = i.els.get(&parent) else { return Vec::new() };
    let Some(slot) = &d.text else { return Vec::new() };
    let slot = slot.borrow();
    let Some((_, para)) = slot.cache.as_ref() else { return Vec::new() };
    let s = &d.resolved;
    let left = s.padding.start + s.border_width;
    let top = s.padding.top + s.border_width;
    let inner = (width - s.padding.start - s.padding.end - s.border_width * 2.0).max(0.0);
    let x = left + para.x_offset(inner);
    para.link_rects(link).into_iter().map(|r| r.translate(remedy_ui_core::Vec2::new(x, top))).collect()
}

/// The box of link element `id` in its parent text (`parent_width` wide): its first piece.
pub(crate) fn placed_rect(i: &Inner, id: ElementId, parent: ElementId, parent_width: f32) -> Option<Rect> {
    let link = i.els.get(&id)?.inline_link?;
    pieces(i, parent, link, parent_width).first().copied()
}

impl Ui {
    /// The link element under window point `p` when `hit` is a text with links and `p` is on
    /// a piece of a link other than the one its element covers.
    pub(crate) fn link_at(&self, hit: Option<ElementId>, p: Point) -> Option<ElementId> {
        let hit = hit?;
        let b = self.tree().bounds(hit)?;
        let i = self.0.borrow();
        let d = i.els.get(&hit)?;
        let links = d.links.as_ref()?;
        let local = Point::new(p.x - b.min.x, p.y - b.min.y);
        links
            .shown
            .iter()
            .find(|(link, _, _)| pieces(&i, hit, *link, b.width()).iter().any(|r| r.contains(local)))
            .map(|(_, _, id)| *id)
    }
}
