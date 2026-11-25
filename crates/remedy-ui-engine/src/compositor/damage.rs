//! Damage: the device-pixel area that changed since the last presented frame.

use remedy_ui_core::Rect;

/// A set of device-pixel rectangles that need redrawing and presenting.
///
/// Rectangles are snapped out to whole pixels. Overlapping or touching rectangles merge, and
/// so do two whose union wastes little area, which keeps the list short without a fixed cap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Damage {
    rects: Vec<Rect>,
}

/// Merge two rects when their union is at most this much larger than their combined area.
const MERGE_WASTE: f32 = 1.25;

fn snap_out(r: Rect) -> Rect {
    Rect::new(r.min.x.floor(), r.min.y.floor(), r.max.x.ceil(), r.max.y.ceil())
}

fn touches(a: Rect, b: Rect) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}

fn worth_merging(a: Rect, b: Rect) -> bool {
    let u = a.union(b);
    let overlap = a.intersect(b).map_or(0.0, |i| i.size().area());
    let covered = a.size().area() + b.size().area() - overlap;
    touches(a, b) || u.size().area() <= covered * MERGE_WASTE
}

impl Damage {
    /// No damage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Damage covering `r`.
    pub fn of(r: Rect) -> Self {
        let mut d = Damage::new();
        d.add(r);
        d
    }

    /// Adds a rectangle, merging it with any it overlaps or nearly fills a box with.
    pub fn add(&mut self, r: Rect) {
        // A NaN on any side compares as not empty, and would never merge or clip away.
        let nan = r.min.x.is_nan() || r.min.y.is_nan() || r.max.x.is_nan() || r.max.y.is_nan();
        if r.is_empty() || nan || !r.min.x.is_finite() || !r.max.y.is_finite() {
            return;
        }
        let mut r = snap_out(r);
        // Merge until nothing else qualifies: a merge can make the rect touch others.
        while let Some(i) = self.rects.iter().position(|&o| worth_merging(o, r)) {
            r = r.union(self.rects.swap_remove(i));
        }
        self.rects.push(r);
    }

    /// Adds every rect of `other`.
    pub fn merge(&mut self, other: &Damage) {
        for &r in &other.rects {
            self.add(r);
        }
    }

    /// Keeps only what lies inside `viewport`.
    pub fn clip_to(&mut self, viewport: Rect) {
        self.rects =
            self.rects.iter().filter_map(|r| r.intersect(viewport)).filter(|r| !r.is_empty()).collect();
    }

    /// The rectangles.
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// True when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// One box around everything.
    pub fn bounds(&self) -> Option<Rect> {
        self.rects.iter().copied().reduce(Rect::union)
    }

    /// The damaged area in pixels (rects never overlap after merging touching ones).
    pub fn area(&self) -> f32 {
        self.rects.iter().map(|r| r.size().area()).sum()
    }
}
