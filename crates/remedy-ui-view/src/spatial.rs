//! Spatial navigation: arrow keys and d-pads move focus to the nearest focusable element in
//! their direction, as the W3C CSS Spatial Navigation draft picks it.

use remedy_ui_core::{Rect, Vec2};
use remedy_ui_core::input::Key;
use remedy_ui_core::tree::{Dirty, ElementId};

use crate::El;
use crate::element::Ui;
use crate::events::focus_changed;

/// A direction arrows move focus in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub(crate) fn of(key: &Key) -> Option<Dir> {
        Some(match key {
            Key::ArrowLeft => Dir::Left,
            Key::ArrowRight => Dir::Right,
            Key::ArrowUp => Dir::Up,
            Key::ArrowDown => Dir::Down,
            _ => return None,
        })
    }

    fn back(self) -> Dir {
        match self {
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
        }
    }
}

/// How spatial navigation weighs candidates ([`Ui::set_spatial_tuning`]). The defaults are
/// the W3C CSS Spatial Navigation draft's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialTuning {
    /// How much a candidate's offset across the axis counts against it going left or right,
    /// where rows sit close together: 30.
    pub cross_weight_horizontal: f32,
    /// The same going up or down: 2.
    pub cross_weight_vertical: f32,
    /// How far, in logical pixels, a candidate may reach back past the focused element's edge
    /// and still count as that way (borders and rounding leave neighbors overlapping): 1.
    pub slack: f32,
}

impl Default for SpatialTuning {
    fn default() -> Self {
        SpatialTuning { cross_weight_horizontal: 30.0, cross_weight_vertical: 2.0, slack: 1.0 }
    }
}

/// The candidate's distance from `from` going `dir`, or `None` when it is not that way.
///
/// The draft's formula: the Euclidean distance between the closest points of the two edges,
/// plus the distance along the axis, plus the cross-axis distance weighted, minus how much
/// the two overlap across the axis.
fn distance(from: Rect, to: Rect, dir: Dir, t: &SpatialTuning) -> Option<f32> {
    let (along, ortho_weight) = match dir {
        Dir::Right => (to.min.x - from.max.x, t.cross_weight_horizontal),
        Dir::Left => (from.min.x - to.max.x, t.cross_weight_horizontal),
        Dir::Down => (to.min.y - from.max.y, t.cross_weight_vertical),
        Dir::Up => (from.min.y - to.max.y, t.cross_weight_vertical),
    };
    if along < -t.slack {
        return None;
    }
    let (a0, a1, b0, b1) = match dir {
        Dir::Left | Dir::Right => (from.min.y, from.max.y, to.min.y, to.max.y),
        Dir::Up | Dir::Down => (from.min.x, from.max.x, to.min.x, to.max.x),
    };
    let overlap = a1.min(b1) - a0.max(b0);
    let along = along.max(0.0);
    let cross = (-overlap).max(0.0);
    let euclid = (along * along + cross * cross).sqrt();
    Some(euclid + along + ortho_weight * cross - overlap.max(0.0))
}

/// The best of `candidates` going `dir` from `from`; the first in tab order wins a tie.
pub(crate) fn best(
    from: Rect,
    dir: Dir,
    candidates: impl IntoIterator<Item = (ElementId, Rect)>,
    tuning: &SpatialTuning,
) -> Option<ElementId> {
    let mut best: Option<(f32, ElementId)> = None;
    for (id, r) in candidates {
        if let Some(d) = distance(from, r, dir, tuning)
            && best.is_none_or(|(b, _)| d < b)
        {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
}

/// `from` moved to just outside `group`'s opposite edge, so a wrapping group's search starts
/// over from that side.
fn wrapped(from: Rect, group: Rect, dir: Dir) -> Rect {
    let v = match dir {
        Dir::Right => Vec2::new(group.min.x - from.max.x, 0.0),
        Dir::Left => Vec2::new(group.max.x - from.min.x, 0.0),
        Dir::Down => Vec2::new(0.0, group.min.y - from.max.y),
        Dir::Up => Vec2::new(0.0, group.max.y - from.min.y),
    };
    from.translate(v)
}

impl Ui {
    /// Turns spatial navigation on or off for this window: arrow keys the focused element
    /// does not take itself move focus to the nearest focusable element that way, inside the
    /// current focus scope (an open dialog keeps it), and scroll it into view. With it on, an
    /// Escape nothing took is offered as [`Key::Back`], as TV remotes mean it.
    ///
    /// Until the app chooses, the window follows the system: on for TVs and touch-only
    /// browsers, off on desktops ([`Ui::set_system_spatial_navigation`]).
    pub fn set_spatial_navigation(&self, on: bool) {
        self.0.borrow_mut().spatial = Some(on);
    }

    /// Whether arrow keys move focus ([`Ui::set_spatial_navigation`]).
    pub fn spatial_navigation(&self) -> bool {
        let i = self.0.borrow();
        i.spatial.unwrap_or(i.system_spatial)
    }

    /// What the system suggests for spatial navigation (the runner sets it from
    /// `SystemSettings::spatial_navigation`); an app's own choice wins.
    pub fn set_system_spatial_navigation(&self, on: bool) {
        self.0.borrow_mut().system_spatial = on;
    }

    /// How spatial navigation weighs candidates (the W3C draft's weights by default).
    pub fn set_spatial_tuning(&self, tuning: SpatialTuning) {
        self.0.borrow_mut().spatial_tuning = tuning;
    }

    /// Moves focus one step `dir` from the focused element. Returns true when focus moved.
    pub(crate) fn focus_toward(&self, dir: Dir) -> bool {
        let tree = self.tree();
        let Some(f) = tree.focused() else { return false };
        let Some(from) = tree.bounds(f) else { return false };
        let tuning = self.0.borrow().spatial_tuning;
        let scope = tree.focus_scope(f);
        let candidates: Vec<_> = tree.focus_candidates(scope).into_iter().filter(|c| *c != f).collect();
        let rects: Vec<_> = candidates
            .iter()
            .filter_map(|c| {
                let r = tree.bounds(*c)?;
                (r.max.x > r.min.x && r.max.y > r.min.y).then_some((*c, r))
            })
            .collect();
        // Inside focus groups, innermost first, arrows stay in the group while it has something
        // that way; at its edge a wrapping group starts over from its other side, others let go.
        let mut found = None;
        let mut cur = tree.parent(f);
        while let Some(g) = cur.filter(|g| found.is_none() && *g != scope) {
            let (group, wrap) =
                self.0.borrow().els.get(&g).map_or((false, false), |d| (d.focus_group, d.focus_wrap));
            if group {
                let inside = || rects.iter().copied().filter(|(c, _)| tree.is_ancestor_or_self(g, *c));
                found = best(from, dir, inside(), &tuning);
                // Only along a line the group has: nothing behind means no line to come round.
                if found.is_none()
                    && wrap
                    && best(from, dir.back(), inside(), &tuning).is_some()
                    && let Some(gb) = tree.bounds(g)
                {
                    found = best(wrapped(from, gb, dir), dir, inside(), &tuning);
                }
            }
            cur = tree.parent(g);
        }
        let Some(mut target) = found.or_else(|| best(from, dir, rects.iter().copied(), &tuning)) else {
            return false;
        };
        // Coming into a focus group from outside returns to where it was left.
        let mut cur = Some(target);
        while let Some(c) = cur.filter(|c| *c != scope) {
            let i = self.0.borrow();
            if i.els.get(&c).is_some_and(|d| d.focus_group) && !tree.is_ancestor_or_self(c, f) {
                if let Some(&last) = i.group_memory.get(&c)
                    && candidates.contains(&last)
                    && tree.is_ancestor_or_self(c, last)
                {
                    target = last;
                }
                break;
            }
            drop(i);
            cur = tree.parent(c);
        }
        if !tree.focus(target).unwrap_or(false) {
            return false;
        }
        self.0.borrow_mut().focus_visible = true;
        focus_changed(self);
        true
    }

    /// Notes `now` as the last focus of each focus group holding it.
    pub(crate) fn remember_focus(&self, now: ElementId) {
        let tree = self.tree();
        let mut cur = tree.parent(now);
        while let Some(c) = cur {
            let mut i = self.0.borrow_mut();
            if i.els.get(&c).is_some_and(|d| d.focus_group) {
                i.group_memory.insert(c, now);
            }
            drop(i);
            cur = tree.parent(c);
        }
    }
}

impl El {
    /// Makes the element a focus group for spatial navigation (a row of cards, a grid):
    /// arrows stay inside it while it has something that way and leave it at its edge, and
    /// when they bring focus back in from outside, focus returns to the child it last had
    /// rather than the nearest one ([`Ui::set_spatial_navigation`]).
    pub fn focus_group(self) -> Self {
        self.edit(Dirty::NONE, |d| d.focus_group = true);
        self
    }

    /// A focus group ([`El::focus_group`]) whose arrows wrap: past its last child they come
    /// round to its first on that line (a carousel, a menu bar) instead of leaving it.
    pub fn focus_wrap(self) -> Self {
        self.edit(Dirty::NONE, |d| {
            d.focus_group = true;
            d.focus_wrap = true;
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remedy_ui_core::Point;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { min: Point::new(x, y), max: Point::new(x + w, y + h) }
    }

    #[test]
    fn overlap_beats_a_closer_diagonal() {
        let tree = remedy_ui_core::tree::Tree::new();
        let (d, b) = (tree.create("box"), tree.create("box"));
        let from = r(100.0, 0.0, 100.0, 50.0);
        // Straight below but farther, and down-left but nearer.
        let below = r(100.0, 120.0, 100.0, 50.0);
        let diagonal = r(0.0, 60.0, 90.0, 50.0);
        let both = [(d, diagonal), (b, below)];
        let t = SpatialTuning::default();
        assert_eq!(best(from, Dir::Down, both, &t), Some(b));
        assert_eq!(best(from, Dir::Up, both, &t), None, "nothing above");
        assert_eq!(best(from, Dir::Left, both, &t), Some(d));
        // An app that weighs the cross axis lightly going down takes the nearer diagonal.
        let loose = SpatialTuning { cross_weight_vertical: 0.0, ..t };
        assert_eq!(best(from, Dir::Down, both, &loose), Some(d));
    }
}
