//! Where pages go: a continuous column of rows (one page, or two side by side), at any zoom,
//! without visiting every page. Row positions are a prefix sum, so finding what a viewport
//! shows is a binary search however long the document is.

use std::ops::Range;

use remedy_ui_core::{Point, Rect, Size, Vec2};
use remedy_ui_paint::Affine;

/// How pages sit side by side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Spread {
    /// One page per row.
    #[default]
    Single,
    /// Two pages per row: 1-2, 3-4, …
    Two,
    /// Two pages per row after a cover alone: 1, 2-3, 4-5, … (books).
    TwoCover,
}

/// How pages are turned, clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// Exhaustive on purpose: pages turn in quarter turns.
pub enum Rotation {
    /// Upright.
    #[default]
    None,
    /// A quarter turn clockwise.
    Clockwise,
    /// Upside down.
    Half,
    /// A quarter turn counter-clockwise.
    CounterClockwise,
}

impl Rotation {
    /// The angle in degrees, clockwise.
    pub fn degrees(self) -> u32 {
        match self {
            Rotation::None => 0,
            Rotation::Clockwise => 90,
            Rotation::Half => 180,
            Rotation::CounterClockwise => 270,
        }
    }

    /// A quarter turn further clockwise.
    pub fn clockwise(self) -> Rotation {
        match self {
            Rotation::None => Rotation::Clockwise,
            Rotation::Clockwise => Rotation::Half,
            Rotation::Half => Rotation::CounterClockwise,
            Rotation::CounterClockwise => Rotation::None,
        }
    }

    /// A quarter turn back.
    pub fn counter_clockwise(self) -> Rotation {
        self.clockwise().clockwise().clockwise()
    }

    /// The size a page of `size` shows at.
    pub fn apply(self, size: Size) -> Size {
        match self {
            Rotation::None | Rotation::Half => size,
            Rotation::Clockwise | Rotation::CounterClockwise => Size::new(size.height, size.width),
        }
    }

    /// Page points (unrotated, `size`) to the turned page's points.
    pub fn transform(self, size: Size) -> Affine {
        let (w, h) = (size.width, size.height);
        match self {
            Rotation::None => Affine::IDENTITY,
            Rotation::Clockwise => Affine([0.0, 1.0, -1.0, 0.0, h, 0.0]),
            Rotation::Half => Affine([-1.0, 0.0, 0.0, -1.0, w, h]),
            Rotation::CounterClockwise => Affine([0.0, -1.0, 1.0, 0.0, 0.0, w]),
        }
    }
}

/// How the zoom follows the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Fit {
    /// The zoom is what the user set.
    #[default]
    Free,
    /// The widest row fills the width.
    Width,
    /// A whole row (one page, or a spread) fits in the window.
    Page,
    /// The current row is as tall as the window; it may be wider than it.
    Height,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    first: usize,
    count: usize,
    /// Sum of the row's page widths, in points.
    width: f32,
    /// The tallest page, in points.
    height: f32,
}

/// Where every page of a document sits at a zoom. Zoom is logical pixels per point; the gap
/// between pages and the margin around them stay the same on screen at every zoom.
#[derive(Clone, Debug)]
pub struct PageLayout {
    sizes: Vec<Size>,
    rotation: Rotation,
    spread: Spread,
    rows: Vec<Row>,
    row_of: Vec<usize>,
    /// Row tops in points, without gaps: `prefix[r]` is the height of rows before `r`.
    prefix: Vec<f32>,
    widest_one: f32,
    widest_two: f32,
    gap: f32,
    margin: f32,
}

impl PageLayout {
    /// Lays out pages of `sizes` (points, unrotated) with `gap` logical pixels between pages
    /// and `margin` around them.
    pub fn new(sizes: Vec<Size>, spread: Spread, rotation: Rotation, gap: f32, margin: f32) -> Self {
        let shown: Vec<Size> = sizes.iter().map(|s| rotation.apply(*s)).collect();
        let mut rows = Vec::new();
        let mut i = 0;
        while i < shown.len() {
            let pair = match spread {
                Spread::Single => false,
                Spread::Two => true,
                Spread::TwoCover => i > 0,
            };
            let count = if pair && i + 1 < shown.len() { 2 } else { 1 };
            let pages = shown.get(i..i + count).unwrap_or(&[]);
            rows.push(Row {
                first: i,
                count,
                width: pages.iter().map(|s| s.width.max(0.0)).sum(),
                height: pages.iter().map(|s| s.height.max(0.0)).fold(0.0, f32::max),
            });
            i += count;
        }
        let mut prefix = Vec::with_capacity(rows.len() + 1);
        let mut row_of = vec![0; shown.len()];
        let mut acc = 0.0f32;
        for (r, row) in rows.iter().enumerate() {
            prefix.push(acc);
            acc += row.height;
            for p in row.first..row.first + row.count {
                if let Some(slot) = row_of.get_mut(p) {
                    *slot = r;
                }
            }
        }
        prefix.push(acc);
        let widest = |n: usize| rows.iter().filter(|r| r.count == n).map(|r| r.width).fold(0.0, f32::max);
        let (widest_one, widest_two) = (widest(1), widest(2));
        PageLayout { sizes, rotation, spread, rows, row_of, prefix, widest_one, widest_two, gap, margin }
    }

    /// Number of pages.
    pub fn len(&self) -> usize {
        self.sizes.len()
    }

    /// True with no pages.
    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }

    /// The spread.
    pub fn spread(&self) -> Spread {
        self.spread
    }

    /// The rotation.
    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// A page's size in points, unrotated.
    pub fn page_size(&self, page: usize) -> Size {
        self.sizes.get(page).copied().unwrap_or(Size::ZERO)
    }

    /// The gap between pages, logical pixels.
    pub fn gap(&self) -> f32 {
        self.gap
    }

    /// The margin around the pages, logical pixels.
    pub fn margin(&self) -> f32 {
        self.margin
    }

    /// The row a page is in.
    pub fn row_of(&self, page: usize) -> usize {
        self.row_of.get(page).copied().unwrap_or(0)
    }

    /// The pages of row `r`.
    pub fn row_pages(&self, r: usize) -> Range<usize> {
        self.rows.get(r).map_or(0..0, |row| row.first..row.first + row.count)
    }

    /// Number of rows.
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    fn gaps(&self, row: &Row) -> f32 {
        self.gap * row.count.saturating_sub(1) as f32
    }

    /// The size of everything at `zoom`, logical pixels.
    pub fn content_size(&self, zoom: f32) -> Size {
        let one = self.widest_one * zoom;
        let two = if self.widest_two > 0.0 { self.widest_two * zoom + self.gap } else { 0.0 };
        let rows = self.rows.len();
        let total = self.prefix.last().copied().unwrap_or(0.0);
        Size::new(
            one.max(two) + 2.0 * self.margin,
            total * zoom + self.gap * rows.saturating_sub(1) as f32 + 2.0 * self.margin,
        )
    }

    /// The top of row `r` at `zoom`.
    pub fn row_top(&self, r: usize, zoom: f32) -> f32 {
        self.margin + self.prefix.get(r).copied().unwrap_or(0.0) * zoom + self.gap * r as f32
    }

    /// The bottom of row `r` at `zoom`.
    pub fn row_bottom(&self, r: usize, zoom: f32) -> f32 {
        self.row_top(r, zoom) + self.rows.get(r).map_or(0.0, |row| row.height) * zoom
    }

    /// The rows that cross `y0..y1` (content pixels) at `zoom`.
    pub fn rows_between(&self, y0: f32, y1: f32, zoom: f32) -> Range<usize> {
        let n = self.rows.len();
        let first = partition(n, |r| self.row_bottom(r, zoom) <= y0);
        let end = partition(n, |r| self.row_top(r, zoom) < y1);
        first..end.max(first)
    }

    /// The pages that cross `y0..y1` (content pixels) at `zoom`.
    pub fn pages_between(&self, y0: f32, y1: f32, zoom: f32) -> Range<usize> {
        let rows = self.rows_between(y0, y1, zoom);
        let start = self.rows.get(rows.start).map_or(self.len(), |r| r.first);
        let end = rows.end.checked_sub(1).and_then(|r| self.rows.get(r)).map_or(start, |r| r.first + r.count);
        start..end.max(start)
    }

    /// Where a page shows, in content pixels, at `zoom` in a viewport `viewport_width` wide
    /// (narrower content is centered in it).
    pub fn page_rect(&self, page: usize, zoom: f32, viewport_width: f32) -> Rect {
        let r = self.row_of(page);
        let Some(row) = self.rows.get(r) else { return Rect::ZERO };
        let width = self.content_size(zoom).width.max(viewport_width);
        let row_w = row.width * zoom + self.gaps(row);
        let mut x = (width - row_w) / 2.0;
        let top = self.row_top(r, zoom);
        for p in row.first..row.first + row.count {
            let s = self.rotation.apply(self.page_size(p));
            let (w, h) = (s.width * zoom, s.height * zoom);
            if p == page {
                let y = top + (row.height * zoom - h) / 2.0;
                return Rect::new(x, y, x + w, y + h);
            }
            x += w + self.gap;
        }
        Rect::ZERO
    }

    /// The transform from a page's points (unrotated) to content pixels.
    pub fn page_transform(&self, page: usize, zoom: f32, viewport_width: f32) -> Affine {
        let at = self.page_rect(page, zoom, viewport_width).min;
        self.rotation
            .transform(self.page_size(page))
            .then(Affine::scale(zoom, zoom))
            .then(Affine::translate(Vec2::new(at.x, at.y)))
    }

    /// The page under content point `p` (or the nearest one in its row), and the point on it
    /// in the page's points (unrotated, possibly outside the page).
    pub fn page_at(&self, p: Point, zoom: f32, viewport_width: f32) -> Option<(usize, Point)> {
        if self.rows.is_empty() {
            return None;
        }
        let n = self.rows.len();
        // The row containing `p.y`, or the nearest: gaps go to the row above.
        let r = partition(n, |r| self.row_bottom(r, zoom) + self.gap <= p.y).min(n - 1);
        let pages = self.row_pages(r);
        let page = pages
            .clone()
            .min_by(|a, b| {
                let d = |pg: usize| {
                    let rect = self.page_rect(pg, zoom, viewport_width);
                    (rect.min.x - p.x).max(p.x - rect.max.x).max(0.0)
                };
                d(*a).total_cmp(&d(*b))
            })
            .unwrap_or(pages.start);
        let inv = self.page_transform(page, zoom, viewport_width).inverse()?;
        Some((page, inv.apply(p)))
    }

    /// The zoom `fit` asks for in a `viewport` (logical pixels), around `page`'s row; `None`
    /// for [`Fit::Free`] or nothing to fit.
    pub fn fit_zoom(&self, fit: Fit, viewport: Size, page: usize) -> Option<f32> {
        let room_w = viewport.width - 2.0 * self.margin;
        let width = {
            let one = (self.widest_one > 0.0).then(|| room_w / self.widest_one);
            let two = (self.widest_two > 0.0).then(|| (room_w - self.gap) / self.widest_two);
            match (one, two) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        };
        let z = match fit {
            Fit::Free => return None,
            Fit::Width => width,
            Fit::Page => {
                let row = self.rows.get(self.row_of(page))?;
                let tall = (row.height > 0.0).then(|| (viewport.height - 2.0 * self.margin) / row.height);
                match (width, tall) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                }
            }
            Fit::Height => {
                let row = self.rows.get(self.row_of(page))?;
                (row.height > 0.0).then(|| (viewport.height - 2.0 * self.margin) / row.height)
            }
        };
        z.filter(|z| z.is_finite() && *z > 0.0)
    }

    /// As [`fit_zoom`](Self::fit_zoom), but measured on `page`'s own row (that page, or its
    /// spread) instead of the document's widest row: what "fit this page" means in a
    /// document whose pages differ in size.
    pub fn fit_row_zoom(&self, fit: Fit, viewport: Size, page: usize) -> Option<f32> {
        let row = self.rows.get(self.row_of(page))?;
        let gaps = self.gap * row.count.saturating_sub(1) as f32;
        let wide = (row.width > 0.0).then(|| (viewport.width - 2.0 * self.margin - gaps) / row.width);
        let tall = (row.height > 0.0).then(|| (viewport.height - 2.0 * self.margin) / row.height);
        let z = match fit {
            Fit::Free => return None,
            Fit::Width => wide,
            Fit::Height => tall,
            Fit::Page => match (wide, tall) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        };
        z.filter(|z| z.is_finite() && *z > 0.0)
    }
}

/// The first index in `0..n` where `pred` is false (`pred` is true then false).
fn partition(n: usize, pred: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if pred(mid) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(n: usize) -> Vec<Size> {
        vec![Size::new(612.0, 792.0); n]
    }

    #[test]
    fn rows_are_found_by_search_at_any_zoom() {
        let l = PageLayout::new(letter(300), Spread::Single, Rotation::None, 8.0, 16.0);
        for zoom in [0.01, 0.5, 1.0, 64.0] {
            let total = l.content_size(zoom).height;
            assert!((total - (300.0 * 792.0 * zoom + 299.0 * 8.0 + 32.0)).abs() < total * 1e-5);
            let top = l.row_top(150, zoom);
            let pages = l.pages_between(top + 1.0, top + 2.0, zoom);
            assert_eq!(pages, 150..151, "zoom {zoom}");
        }
        assert_eq!(l.pages_between(-100.0, 0.0, 1.0), 0..0);
        assert_eq!(l.pages_between(0.0, 1e9, 1.0), 0..300);
    }

    #[test]
    fn spreads_pair_pages_and_the_cover_stands_alone() {
        let l = PageLayout::new(letter(5), Spread::TwoCover, Rotation::None, 8.0, 0.0);
        assert_eq!(l.rows(), 3);
        assert_eq!(l.row_pages(0), 0..1);
        assert_eq!(l.row_pages(1), 1..3);
        assert_eq!(l.row_pages(2), 3..5);
        let a = l.page_rect(1, 1.0, 0.0);
        let b = l.page_rect(2, 1.0, 0.0);
        assert_eq!(b.min.x - a.max.x, 8.0, "the gap between facing pages");
        let cover = l.page_rect(0, 1.0, 0.0);
        assert!((cover.center().x - (a.min.x + b.max.x) / 2.0).abs() < 1e-3, "rows are centered");
    }

    #[test]
    fn rotation_turns_pages_and_maps_points_back() {
        let l = PageLayout::new(letter(2), Spread::Single, Rotation::Clockwise, 0.0, 0.0);
        let r = l.page_rect(0, 1.0, 0.0);
        assert_eq!(r.size(), Size::new(792.0, 612.0));
        let t = l.page_transform(0, 1.0, 0.0);
        // The page's top-left corner goes to the top-right after a clockwise turn.
        assert_eq!(t.apply(Point::ZERO), Point::new(792.0, 0.0));
        let hit = l.page_at(Point::new(700.0, 10.0), 1.0, 0.0);
        let (page, at) = hit.unwrap_or((9, Point::ZERO));
        assert_eq!(page, 0);
        assert!((at.x - 10.0).abs() < 1e-3 && (at.y - 92.0).abs() < 1e-3, "{at:?}");
    }

    #[test]
    fn fit_width_and_fit_page() {
        let l = PageLayout::new(letter(3), Spread::Single, Rotation::None, 8.0, 10.0);
        let w = l.fit_zoom(Fit::Width, Size::new(632.0, 400.0), 0);
        assert_eq!(w, Some(1.0));
        let p = l.fit_zoom(Fit::Page, Size::new(632.0, 416.0), 0).unwrap_or(0.0);
        assert!((p - 0.5).abs() < 1e-6);
        assert_eq!(l.fit_zoom(Fit::Free, Size::new(1.0, 1.0), 0), None);
    }

    #[test]
    fn fit_height_and_fitting_one_page_of_a_mixed_document() {
        let l = PageLayout::new(letter(3), Spread::Single, Rotation::None, 8.0, 10.0);
        assert_eq!(l.fit_zoom(Fit::Height, Size::new(100.0, 812.0), 0), Some(1.0), "as tall as the window");
        // A wide page later in the document halves "fit width" for all of it, but not for
        // a fit made to the first page alone.
        let sizes = vec![Size::new(612.0, 792.0), Size::new(1224.0, 792.0)];
        let l = PageLayout::new(sizes, Spread::Single, Rotation::None, 8.0, 10.0);
        let view = Size::new(632.0, 416.0);
        assert_eq!(l.fit_zoom(Fit::Width, view, 0), Some(0.5));
        assert_eq!(l.fit_row_zoom(Fit::Width, view, 0), Some(1.0));
        assert_eq!(l.fit_row_zoom(Fit::Width, view, 1), Some(0.5));
        assert_eq!(l.fit_row_zoom(Fit::Page, view, 0), Some(0.5), "the height is the tighter side");
        assert_eq!(l.fit_row_zoom(Fit::Free, view, 0), None);
    }
}
