//! Virtual lists: lists of any length where only the rows near the view exist.
//!
//! Heights are learned as rows are measured; rows not measured yet use an estimate (the
//! average of the measured ones, or the caller's guess). Positions come from two Fenwick
//! trees, one over measured heights and one counting unmeasured rows, so a position is
//! `measured prefix + unmeasured count × estimate`: refining the estimate costs nothing, a
//! new row at the end costs O(log n), and a lookup costs O(log² n).
//!
//! **Anchoring** keeps what the user is looking at still. When rows above the view are
//! inserted, removed or re-measured, the first visible row stays at the same place on screen.
//! A list scrolled to its end sticks there (chat): new rows push the view along, until the
//! user scrolls up; scrolling back to the end sticks again.

use std::ops::Range;

/// A Fenwick (binary indexed) tree of sums.
#[derive(Clone, Debug, Default)]
struct Fenwick<T> {
    tree: Vec<T>,
}

impl<T: Copy + Default + std::ops::Add<Output = T> + std::ops::Sub<Output = T>> Fenwick<T> {
    fn build(values: &[T]) -> Self {
        let mut tree = values.to_vec();
        for i in 0..tree.len() {
            let j = i | (i + 1);
            if j < tree.len() {
                let v = tree.get(i).copied().unwrap_or_default();
                if let Some(t) = tree.get_mut(j) {
                    *t = *t + v;
                }
            }
        }
        Fenwick { tree }
    }

    /// Sum of the first `n` values.
    fn prefix(&self, n: usize) -> T {
        let mut sum = T::default();
        let mut i = n.min(self.tree.len());
        while i > 0 {
            sum = sum + self.tree.get(i - 1).copied().unwrap_or_default();
            i &= i - 1;
        }
        sum
    }

    fn add(&mut self, index: usize, delta: T) {
        let mut i = index;
        while i < self.tree.len() {
            if let Some(t) = self.tree.get_mut(i) {
                *t = *t + delta;
            }
            i |= i + 1;
        }
    }

    /// Appends a value in O(log n).
    fn push(&mut self, value: T) {
        let i = self.tree.len();
        // This node covers values (i & (i + 1)) ..= i, which is the prefix up to i minus the
        // prefix up to (i & (i + 1)), plus the new value.
        let low = i & (i + 1);
        let covered = self.prefix(i) - self.prefix(low);
        self.tree.push(covered + value);
    }
}

/// Where a jump puts its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JumpAlign {
    /// The row's top at the view's top.
    Start,
    /// The row centered.
    Center,
    /// The row's bottom at the view's bottom.
    End,
}

/// The state of one virtual list.
#[derive(Clone, Debug)]
pub struct VirtualList {
    heights: Vec<Option<f32>>,
    measured: Fenwick<f32>,
    unmeasured: Fenwick<i64>,
    measured_sum: f32,
    measured_count: u32,
    guess: f32,
    viewport: f32,
    scroll: f32,
    stuck: bool,
    stick_to_end: bool,
}

impl VirtualList {
    /// A list of `len` rows whose heights are guessed at `estimate` until measured.
    /// `stick_to_end` makes it behave like a chat: it starts at the end and stays there.
    pub fn new(len: usize, estimate: f32, stick_to_end: bool) -> Self {
        let mut v = VirtualList {
            heights: vec![None; len],
            measured: Fenwick::default(),
            unmeasured: Fenwick::default(),
            measured_sum: 0.0,
            measured_count: 0,
            // Not a height (NaN, infinite) would make every position NaN.
            guess: if estimate.is_finite() { estimate.max(1.0) } else { 1.0 },
            viewport: 0.0,
            scroll: 0.0,
            stuck: stick_to_end,
            stick_to_end,
        };
        v.rebuild();
        v
    }

    fn rebuild(&mut self) {
        let m: Vec<f32> = self.heights.iter().map(|h| h.unwrap_or(0.0)).collect();
        let u: Vec<i64> = self.heights.iter().map(|h| i64::from(h.is_none())).collect();
        self.measured = Fenwick::build(&m);
        self.unmeasured = Fenwick::build(&u);
    }

    /// The height used for rows not measured yet: the average of the measured ones, or the
    /// caller's guess while none is measured or they average under a pixel (rows measured
    /// before they had content say nothing about the rest, and rows estimated at no height
    /// would all be in view at once).
    pub fn estimate(&self) -> f32 {
        let average = self.measured_sum / self.measured_count as f32;
        if self.measured_count > 0 && average >= 1.0 { average } else { self.guess }
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.heights.len()
    }

    /// True when the list has no rows.
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// Top of row `i` (or the total height for `i == len`).
    pub fn offset_of(&self, i: usize) -> f32 {
        let i = i.min(self.len());
        self.measured.prefix(i) + self.unmeasured.prefix(i) as f32 * self.estimate()
    }

    /// Height of row `i`, measured or estimated.
    pub fn height_of(&self, i: usize) -> f32 {
        self.heights.get(i).copied().flatten().unwrap_or_else(|| self.estimate())
    }

    /// Height of all rows.
    pub fn total_height(&self) -> f32 {
        self.offset_of(self.len())
    }

    /// The row at vertical position `y` (clamped to the list).
    pub fn index_at(&self, y: f32) -> usize {
        if self.is_empty() {
            return 0;
        }
        let (mut lo, mut hi) = (0usize, self.len());
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if self.offset_of(mid) <= y { lo = mid } else { hi = mid }
        }
        lo
    }

    fn max_scroll(&self) -> f32 {
        (self.total_height() - self.viewport).max(0.0)
    }

    /// The scroll offset (top of the view, from the top of the list).
    pub fn scroll_offset(&self) -> f32 {
        self.scroll
    }

    /// True while the view is held at the end.
    pub fn is_stuck_to_end(&self) -> bool {
        self.stuck
    }

    /// Sets the view's height.
    pub fn set_viewport(&mut self, height: f32) {
        let anchor = self.anchor();
        self.viewport = height.max(0.0);
        self.restore(anchor);
    }

    /// Scrolls to `y`, clamped. Reaching the end of a sticky list sticks it again; leaving
    /// the end lets go.
    pub fn scroll_to(&mut self, y: f32) {
        // Not a position (0 / 0 from a thumb dragged on an empty track): the view stays.
        if y.is_nan() {
            return;
        }
        self.scroll = y.clamp(0.0, self.max_scroll());
        self.stuck = self.stick_to_end && self.scroll >= self.max_scroll() - 0.5;
    }

    /// Scrolls by `dy`.
    pub fn scroll_by(&mut self, dy: f32) {
        self.scroll_to(self.scroll + dy);
    }

    /// Rows that intersect the view widened by `prefetch` pixels each way: the rows to mount.
    pub fn visible_range(&self, prefetch: f32) -> Range<usize> {
        if self.is_empty() {
            return 0..0;
        }
        let top = (self.scroll - prefetch).max(0.0);
        let bottom = self.scroll + self.viewport + prefetch;
        let mut first = self.index_at(top);
        // Rows with no height at the view's top edge are in view too: left out, they could
        // never be measured again once they have content.
        while first > 0 && self.heights.get(first - 1).copied().flatten() == Some(0.0) {
            first -= 1;
        }
        let mut last = self.index_at(bottom);
        if self.offset_of(last) < bottom {
            last += 1;
        }
        first..last.min(self.len())
    }

    /// Jumps to row `i`, even one never measured.
    pub fn jump_to(&mut self, i: usize, align: JumpAlign) {
        let i = i.min(self.len().saturating_sub(1));
        let (top, h) = (self.offset_of(i), self.height_of(i));
        let y = match align {
            JumpAlign::Start => top,
            JumpAlign::Center => top + h / 2.0 - self.viewport / 2.0,
            JumpAlign::End => top + h - self.viewport,
        };
        self.scroll_to(y);
    }

    // ---------- changes, with anchoring ----------

    fn anchor(&self) -> Option<(usize, f32)> {
        if self.stuck || self.is_empty() {
            return None;
        }
        let i = self.index_at(self.scroll);
        Some((i, self.scroll - self.offset_of(i)))
    }

    fn restore(&mut self, anchor: Option<(usize, f32)>) {
        match anchor {
            None if self.stuck => self.scroll = self.max_scroll(),
            None => self.scroll = self.scroll.clamp(0.0, self.max_scroll()),
            Some((i, within)) => {
                let i = i.min(self.len().saturating_sub(1));
                self.scroll = (self.offset_of(i) + within).clamp(0.0, self.max_scroll());
            }
        }
    }

    /// Records row `i`'s measured height. The view stays anchored. A height that is not a
    /// finite number is no measurement: the row keeps what it had.
    pub fn set_height(&mut self, i: usize, height: f32) {
        // An infinite height would turn every later sum into NaN.
        if !height.is_finite() {
            return;
        }
        let Some(slot) = self.heights.get(i).copied() else { return };
        let anchor = self.anchor();
        let height = height.max(0.0);
        match slot {
            Some(old) => {
                self.measured.add(i, height - old);
                self.measured_sum += height - old;
            }
            None => {
                self.measured.add(i, height);
                self.unmeasured.add(i, -1);
                self.measured_sum += height;
                self.measured_count += 1;
            }
        }
        if let Some(h) = self.heights.get_mut(i) {
            *h = Some(height);
        }
        self.restore(anchor);
    }

    /// Forgets every measured height (after a width change): rows are measured again as they
    /// come into view. The view stays anchored.
    pub fn invalidate_heights(&mut self) {
        let anchor = self.anchor();
        self.heights.iter_mut().for_each(|h| *h = None);
        self.measured_sum = 0.0;
        self.measured_count = 0;
        self.rebuild();
        self.restore(anchor);
    }

    /// Adds `n` rows at the end (new messages). O(log n) per row.
    pub fn push(&mut self, n: usize) {
        let anchor = self.anchor();
        for _ in 0..n {
            self.heights.push(None);
            self.measured.push(0.0);
            self.unmeasured.push(1);
        }
        self.restore(anchor);
    }

    /// Inserts `n` rows before row `at` (older messages loaded above). The view stays on what
    /// it showed.
    pub fn insert(&mut self, at: usize, n: usize) {
        let at = at.min(self.len());
        let anchor = self.anchor().map(|(i, w)| if i >= at { (i + n, w) } else { (i, w) });
        self.heights.splice(at..at, std::iter::repeat_n(None, n));
        self.rebuild();
        self.restore(anchor);
    }

    /// Removes rows in `range`. The view stays on the first row it showed that remains.
    pub fn remove(&mut self, range: Range<usize>) {
        let (s, e) = (range.start.min(self.len()), range.end.min(self.len()));
        if s >= e {
            return;
        }
        let anchor = self.anchor().map(|(i, w)| {
            if i >= e {
                (i - (e - s), w)
            } else if i >= s {
                (s, 0.0)
            } else {
                (i, w)
            }
        });
        for h in self.heights.drain(s..e).flatten() {
            self.measured_sum -= h;
            self.measured_count -= 1;
        }
        self.rebuild();
        self.restore(anchor);
    }
}
