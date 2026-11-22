//! Spatial navigation: arrow keys and d-pads move focus to the nearest focusable element in
//! their direction, as the W3C CSS Spatial Navigation draft picks it.

use remedy_ui_core::Rect;
use remedy_ui_input::Key;
use remedy_ui_tree::{Dirty, ElementId};

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
}

/// The candidate's distance from `from` going `dir`, or `None` when it is not that way.
///
/// The draft's formula: the Euclidean distance between the closest points of the two edges,
/// plus the distance along the axis, plus the cross-axis distance weighted (more for left
/// and right, where rows sit close together), minus how much the two overlap across the axis.
fn distance(from: Rect, to: Rect, dir: Dir) -> Option<f32> {
    // A pixel of slack: borders and rounding leave neighbors touching or overlapping by one.
    const SLACK: f32 = 1.0;
    let (along, ortho_weight) = match dir {
        Dir::Right => (to.min.x - from.max.x, 30.0),
        Dir::Left => (from.min.x - to.max.x, 30.0),
        Dir::Down => (to.min.y - from.max.y, 2.0),
        Dir::Up => (from.min.y - to.max.y, 2.0),
    };
    if along < -SLACK {
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
) -> Option<ElementId> {
    let mut best: Option<(f32, ElementId)> = None;
    for (id, r) in candidates {
        if let Some(d) = distance(from, r, dir)
            && best.is_none_or(|(b, _)| d < b)
        {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id)
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

    /// Moves focus one step `dir` from the focused element. Returns true when focus moved.
    pub(crate) fn focus_toward(&self, dir: Dir) -> bool {
        let tree = self.tree();
        let Some(f) = tree.focused() else { return false };
        let Some(from) = tree.bounds(f) else { return false };
        let scope = tree.focus_scope(f);
        let candidates: Vec<_> = tree.focus_candidates(scope).into_iter().filter(|c| *c != f).collect();
        let rects = candidates.iter().filter_map(|c| {
            let r = tree.bounds(*c)?;
            (r.max.x > r.min.x && r.max.y > r.min.y).then_some((*c, r))
        });
        let Some(mut target) = best(from, dir, rects) else { return false };
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
    /// Makes the element a focus group for spatial navigation (a row of cards, a grid): when
    /// arrows bring focus back into it from outside, focus returns to the child it last had
    /// rather than the nearest one ([`Ui::set_spatial_navigation`]).
    pub fn focus_group(self) -> Self {
        self.edit(Dirty::NONE, |d| d.focus_group = true);
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
        let tree = remedy_ui_tree::Tree::new();
        let (d, b) = (tree.create("box"), tree.create("box"));
        let from = r(100.0, 0.0, 100.0, 50.0);
        // Straight below but farther, and down-left but nearer.
        let below = r(100.0, 120.0, 100.0, 50.0);
        let diagonal = r(0.0, 60.0, 90.0, 50.0);
        let both = [(d, diagonal), (b, below)];
        assert_eq!(best(from, Dir::Down, both), Some(b));
        assert_eq!(best(from, Dir::Up, both), None, "nothing above");
        assert_eq!(best(from, Dir::Left, both), Some(d));
    }
}
