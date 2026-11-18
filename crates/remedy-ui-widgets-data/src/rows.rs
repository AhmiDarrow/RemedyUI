//! Fixed-height virtual rows: only the rows near the viewport exist, rows still in view are
//! kept as the view moves, and only rows entering or leaving are built or dropped.

use std::collections::BTreeMap;
use std::rc::Rc;

use remedy_ui_layout::Length;
use remedy_ui_view::{El, Ui};

/// Builds the element for row `i`.
pub(crate) type RowBuilder = Rc<dyn Fn(usize) -> El>;

pub(crate) struct Rows {
    /// The column of rows inside a scroll container.
    content: El,
    top: El,
    bottom: El,
    row_height: f32,
    count: usize,
    build: RowBuilder,
    mounted: BTreeMap<usize, El>,
    /// Rows beyond the viewport kept built on each side, so a flick does not show gaps.
    overscan: usize,
}

impl Rows {
    pub(crate) fn new(ui: &Ui, row_height: f32, build: RowBuilder) -> (Rows, El) {
        let top = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
        let bottom = ui.el("virtual_space").layout(|l| l.shrink = 0.0);
        let content = ui.el("virtual_rows").column().layout(|l| l.shrink = 0.0);
        let column = ui
            .el("virtual_body")
            .column()
            .layout(|l| l.shrink = 0.0)
            .child(top.clone())
            .child(content.clone())
            .child(bottom.clone());
        (
            Rows { content, top, bottom, row_height, count: 0, build, mounted: BTreeMap::new(), overscan: 8 },
            column,
        )
    }

    /// The number of rows; every built row is dropped when `rebuild` (the data changed).
    pub(crate) fn set_count(&mut self, n: usize, rebuild: bool) {
        self.count = n;
        if rebuild {
            for (_, el) in std::mem::take(&mut self.mounted) {
                el.remove();
            }
        }
    }

    /// The row index at a content y.
    pub(crate) fn index_at(&self, y: f32) -> usize {
        (y.max(0.0) / self.row_height) as usize
    }

    /// Builds the rows a viewport at `offset_y` of `viewport_h` shows, keeping the ones
    /// already built.
    pub(crate) fn show(&mut self, offset_y: f32, viewport_h: f32) {
        let first = self.index_at(offset_y).saturating_sub(self.overscan);
        let last =
            (self.index_at(offset_y + viewport_h.max(self.row_height)) + 1 + self.overscan).min(self.count);
        let first = first.min(last);
        let gone: Vec<usize> = self.mounted.keys().copied().filter(|i| *i < first || *i >= last).collect();
        for i in gone {
            if let Some(el) = self.mounted.remove(&i) {
                el.remove();
            }
        }
        // New rows go in at their place among the kept ones.
        for i in first..last {
            if self.mounted.contains_key(&i) {
                continue;
            }
            let at = self.mounted.range(..i).count();
            let el = (self.build)(i).layout(|l| {
                l.shrink = 0.0;
            });
            self.content.insert_child(at, &el);
            self.mounted.insert(i, el);
        }
        let h = self.row_height;
        let _ = self.top.clone().layout(move |l| l.height = Length::Px(first as f32 * h));
        let below = self.count.saturating_sub(last) as f32 * h;
        let _ = self.bottom.clone().layout(move |l| l.height = Length::Px(below));
    }
}
