//! Series of values: a ring buffer, ranges, and the math that turns values into points.

use std::collections::VecDeque;

use remedy_ui_core::Point;

/// A source of values, oldest first.
pub trait Samples {
    /// How many values there are.
    fn len(&self) -> usize;

    /// The value at `index` (0 is the oldest).
    fn get(&self, index: usize) -> Option<f32>;

    /// How many values the source holds when full, if it has a size. A sparkline of a source
    /// that is still filling keeps its newest value at the right edge and leaves the left
    /// empty until the history fills.
    fn capacity(&self) -> Option<usize> {
        None
    }

    /// True when there are no values.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Samples for Vec<f32> {
    fn len(&self) -> usize {
        <[f32]>::len(self)
    }

    fn get(&self, index: usize) -> Option<f32> {
        <[f32]>::get(self, index).copied()
    }
}

/// A ring buffer of the most recent values: push as they arrive, the oldest fall off.
///
/// Keep it in a `Signal<History>` and call `update(|h| h.push(v))`.
#[derive(Clone, Debug, PartialEq)]
pub struct History {
    values: VecDeque<f32>,
    capacity: usize,
}

impl History {
    /// An empty history that keeps the latest `capacity` values (at least one).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        // Room is made as values arrive: a huge capacity is a limit, not memory to take now.
        History { values: VecDeque::with_capacity(capacity.min(1024)), capacity }
    }

    /// Adds a value, dropping the oldest when full.
    pub fn push(&mut self, value: f32) {
        if self.values.len() >= self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    /// How many values fit.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Changes how many values fit, dropping the oldest if there are too many.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        while self.values.len() > self.capacity {
            self.values.pop_front();
        }
    }

    /// The newest value.
    pub fn latest(&self) -> Option<f32> {
        self.values.back().copied()
    }

    /// Forgets every value.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// The values, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        self.values.iter().copied()
    }
}

impl Samples for History {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn get(&self, index: usize) -> Option<f32> {
        self.values.get(index).copied()
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.capacity)
    }
}

/// The span of values a chart shows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Range {
    /// Fit the values currently shown.
    Auto,
    /// Always this span: `(min, max)`.
    Fixed(f32, f32),
}

impl Range {
    /// The `(low, high)` to draw for values with this lowest and highest, always with
    /// `high > low`. A flat series is centered rather than stuck on an edge.
    pub fn resolve(self, lowest: f32, highest: f32) -> (f32, f32) {
        let (lo, hi) = match self {
            Range::Fixed(a, b) if a.is_finite() && b.is_finite() => (a.min(b), a.max(b)),
            _ if lowest.is_finite() && highest.is_finite() => (lowest, highest),
            _ => (0.0, 1.0),
        };
        if hi - lo > f32::EPSILON * hi.abs().max(lo.abs()).max(1.0) { (lo, hi) } else { (lo - 0.5, hi + 0.5) }
    }
}

/// The lowest and highest finite values of a source, if it has any.
pub fn extent<S: Samples + ?Sized>(s: &S) -> Option<(f32, f32)> {
    let mut out: Option<(f32, f32)> = None;
    for i in 0..s.len() {
        if let Some(v) = s.get(i).filter(|v| v.is_finite()) {
            out = Some(match out {
                Some((lo, hi)) => (lo.min(v), hi.max(v)),
                None => (v, v),
            });
        }
    }
    out
}

/// Where a value falls in `lo..hi`, 0 to 1.
pub fn fraction(v: f32, lo: f32, hi: f32) -> f32 {
    let f = (v - lo) / (hi - lo);
    // A span of nothing (or a value that is not a number) has no place in it.
    if f.is_nan() { 0.0 } else { f.clamp(0.0, 1.0) }
}

/// How a line is laid out in a box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlotBox {
    /// Width of the area for the line.
    pub width: f32,
    /// Height of the area for the line.
    pub height: f32,
    /// Space kept free at the top and bottom, so a stroke is not cut at the edges.
    pub margin: f32,
}

/// Turns a source into runs of points (a gap in the data, a value that is not a number,
/// ends a run). The newest value sits on the right edge; `window` is how many values the
/// width stands for (at least the source's length). Many values per pixel are reduced to
/// each pixel column's lowest and highest, in the order they happened, so the line keeps
/// its spikes and the work follows the width, not the data.
pub fn plot<S: Samples + ?Sized>(s: &S, window: usize, range: (f32, f32), area: PlotBox) -> Vec<Vec<Point>> {
    let n = s.len();
    let window = window.max(n).max(1);
    let span = (window.saturating_sub(1)).max(1) as f32;
    let x_of = |i: usize| -> f32 {
        let back = (n.saturating_sub(1)).saturating_sub(i) as f32;
        if window == 1 { area.width } else { area.width - area.width * back / span }
    };
    let y_of = |v: f32| -> f32 {
        let inner = (area.height - 2.0 * area.margin).max(0.0);
        area.margin + inner * (1.0 - fraction(v, range.0, range.1))
    };
    let columns = (area.width.max(1.0)).ceil() as usize;
    let per_column = n as f32 / columns.min(window) as f32;
    let mut runs: Vec<Vec<Point>> = Vec::new();
    let mut run: Vec<Point> = Vec::new();
    if per_column <= 2.0 {
        for i in 0..n {
            match s.get(i).filter(|v| v.is_finite()) {
                Some(v) => run.push(Point::new(x_of(i), y_of(v))),
                None => {
                    if !run.is_empty() {
                        runs.push(std::mem::take(&mut run));
                    }
                }
            }
        }
    } else {
        // Reduce: walk the values in buckets of one pixel column.
        let mut i = 0;
        while i < n {
            let x = x_of(i).floor();
            let mut lo: Option<(usize, f32)> = None;
            let mut hi: Option<(usize, f32)> = None;
            let mut gap = false;
            let mut j = i;
            while j < n && x_of(j).floor() == x {
                match s.get(j).filter(|v| v.is_finite()) {
                    Some(v) => {
                        if lo.is_none_or(|(_, l)| v < l) {
                            lo = Some((j, v));
                        }
                        if hi.is_none_or(|(_, h)| v > h) {
                            hi = Some((j, v));
                        }
                    }
                    None => gap = true,
                }
                j += 1;
            }
            if let (Some((a, av)), Some((b, bv))) = (lo, hi) {
                let (first, second) = if a <= b { ((a, av), (b, bv)) } else { ((b, bv), (a, av)) };
                run.push(Point::new(x_of(first.0), y_of(first.1)));
                if second.0 != first.0 {
                    run.push(Point::new(x_of(second.0), y_of(second.1)));
                }
            }
            if gap && !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
            // Always advance (a bucket holds at least one value).
            i = j.max(i + 1);
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    runs
}

/// Reduces `values` to at most `columns` by taking the highest of each group, in order
/// (so a spectrum of 512 bins in 100 pixels still shows every peak).
pub fn group_max<S: Samples + ?Sized>(s: &S, columns: usize) -> Vec<f32> {
    let n = s.len();
    let columns = columns.max(1);
    if n <= columns {
        return (0..n).map(|i| s.get(i).filter(|v| v.is_finite()).unwrap_or(0.0)).collect();
    }
    (0..columns)
        .map(|c| {
            let from = c * n / columns;
            let to = ((c + 1) * n / columns).max(from + 1).min(n);
            (from..to).filter_map(|i| s.get(i)).filter(|v| v.is_finite()).fold(f32::NEG_INFINITY, f32::max)
        })
        .map(|v| if v.is_finite() { v } else { 0.0 })
        .collect()
}
