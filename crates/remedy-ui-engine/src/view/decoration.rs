//! Decorations: lines drawn under ranges of a field's text (spelling marks and the like).

use std::ops::Range;

use crate::paint::{Brush, DisplayList, Item, Path, Stroke};
use remedy_ui_core::{Color, Point, Rect};

/// How a decoration is drawn under its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecorationKind {
    /// A wavy line, as spell checkers mark a misspelling.
    Squiggle,
    /// A straight line.
    Underline,
    /// A dotted line.
    Dotted,
    /// A fill behind the text, under its glyphs: what a search found. Without a color it is
    /// the theme's mark color.
    Highlight,
}

/// A line under a range of a field's text.
///
/// ```
/// use remedy_ui_engine::view::{Decoration, DecorationKind};
/// let d = Decoration::squiggle(4..9);
/// assert_eq!(d.kind, DecorationKind::Squiggle);
/// assert_eq!(d.color, None);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration {
    /// The byte range in the field's text.
    pub range: Range<usize>,
    /// How it is drawn.
    pub kind: DecorationKind,
    /// The color; `None` uses the theme's danger color (the theme's mark color for a
    /// [`Highlight`](DecorationKind::Highlight)).
    pub color: Option<Color>,
}

impl Decoration {
    /// A wavy line under `range` in the theme's danger color.
    pub fn squiggle(range: Range<usize>) -> Self {
        Decoration { range, kind: DecorationKind::Squiggle, color: None }
    }

    /// A fill behind `range` in the theme's mark color: what a search found.
    pub fn highlight(range: Range<usize>) -> Self {
        Decoration { range, kind: DecorationKind::Highlight, color: None }
    }

    /// The same decoration in `color`.
    pub fn colored(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// The same range drawn as `kind`.
    pub fn of_kind(mut self, kind: DecorationKind) -> Self {
        self.kind = kind;
        self
    }
}

/// The length of the part both texts start with, in bytes, on a character boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            break;
        }
        n += x.len_utf8();
    }
    n
}

/// The length of the part both texts end with, in bytes, on a character boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        n += x.len_utf8();
    }
    n
}

/// Moves decorations made for `old` onto `new`, the same text after an edit. The edit is the
/// differing middle of the two texts: decorations before it stay, those after it move by the
/// change in length, and those it touches are dropped (they are checked again).
pub(crate) fn carry(old: &str, new: &str, decorations: &mut Vec<Decoration>) {
    if old == new {
        return;
    }
    let prefix = common_prefix(old, new);
    // The suffix may not overlap the prefix in either text.
    let room = old.len().min(new.len()) - prefix;
    let mut suffix = common_suffix(old, new).min(room);
    while suffix > 0
        && !(old.is_char_boundary(old.len() - suffix) && new.is_char_boundary(new.len() - suffix))
    {
        suffix -= 1;
    }
    let old_tail = old.len() - suffix;
    decorations.retain_mut(|d| {
        if d.range.end <= prefix {
            true
        } else if d.range.start >= old_tail {
            let start = d.range.start - old_tail + (new.len() - suffix);
            let end = d.range.end - old_tail + (new.len() - suffix);
            d.range = start..end;
            true
        } else {
            false
        }
    });
}

/// Draws `kind` along the bottom of `r` (a line box), in `color`.
pub(crate) fn draw(list: &mut DisplayList, r: Rect, kind: DecorationKind, color: Color) {
    let width = (r.max.x - r.min.x).max(0.0);
    if width <= 0.0 {
        return;
    }
    let thickness = 1.0;
    let base = r.max.y - 1.0;
    match kind {
        // Filled under the glyphs by the field, before they are drawn.
        DecorationKind::Highlight => {}
        DecorationKind::Underline => {
            list.fill_rect(Rect::new(r.min.x, base, r.max.x, base + thickness), Brush::Solid(color));
        }
        DecorationKind::Dotted => {
            let mut path = Path::new();
            path.move_to(Point::new(r.min.x, base + 0.5)).line_to(Point::new(r.max.x, base + 0.5));
            let mut stroke = Stroke::new(thickness);
            stroke.dashes = vec![thickness, thickness * 2.0];
            list.push(Item::StrokePath { path, stroke, brush: Brush::Solid(color) });
        }
        DecorationKind::Squiggle => {
            // A zigzag two pixels tall with a four-pixel period, drawn as one stroked path.
            let amplitude = 1.5;
            let half = 2.0;
            let mut path = Path::new();
            let (mut x, mut up) = (r.min.x, false);
            path.move_to(Point::new(x, base + amplitude));
            while x < r.max.x {
                x = (x + half).min(r.max.x);
                up = !up;
                path.line_to(Point::new(x, base + if up { 0.0 } else { amplitude }));
            }
            let mut stroke = Stroke::new(thickness);
            stroke.join = crate::paint::StrokeJoin::Bevel;
            list.push(Item::StrokePath { path, stroke, brush: Brush::Solid(color) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(r: Range<usize>) -> Decoration {
        Decoration::squiggle(r)
    }

    #[test]
    fn decorations_before_an_edit_stay_and_after_it_move() {
        let mut v = vec![d(0..3), d(4..7), d(8..11)];
        // "abc def ghi" -> "abc dxxxef ghi": the middle word changed.
        carry("abc def ghi", "abc dxxxef ghi", &mut v);
        assert_eq!(v, vec![d(0..3), d(11..14)]);
    }

    #[test]
    fn an_edit_inside_a_range_drops_it() {
        let mut v = vec![d(4..7)];
        carry("abc def ghi", "abc dXf ghi", &mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn typing_at_the_end_keeps_earlier_ranges_and_shifts_none() {
        let mut v = vec![d(0..3)];
        carry("abc", "abc x", &mut v);
        assert_eq!(v, vec![d(0..3)]);
    }

    #[test]
    fn multibyte_text_shifts_on_character_boundaries() {
        let mut v = vec![d(6..9)];
        // Insert "é" (two bytes) at the start.
        carry("héé abc", "éhéé abc", &mut v);
        assert_eq!(v, vec![d(8..11)]);
        assert_eq!("éhéé abc".get(8..11), Some("abc"));
    }

    #[test]
    fn deleting_text_before_a_range_moves_it_back() {
        let mut v = vec![d(8..11)];
        carry("abc def ghi", "abc ghi", &mut v);
        assert_eq!(v, vec![d(4..7)]);
    }

    #[test]
    fn a_squiggle_stays_under_its_box() {
        let mut list = DisplayList::new();
        draw(&mut list, Rect::new(10.0, 0.0, 30.0, 20.0), DecorationKind::Squiggle, Color::BLACK);
        let b = list.bounds().unwrap_or_else(|| unreachable!("a squiggle draws something"));
        assert!(b.min.x >= 7.0 && b.max.x <= 33.0 && b.min.y >= 16.0 && b.max.y <= 23.5, "{b:?}");
    }
}
