//! Colour-matrix groups ([`Filter::ColorMatrix`](remedy_ui_paint::Filter::ColorMatrix)).
//! vello's filter graph runs no colour matrices, so a group with one is drawn to pixels of its
//! own (in device space, at the target's resolution), run through the matrix, and placed back
//! pixel for pixel. Both renderers and both walks (display lists and composites) share what is
//! here, so they cut the same pictures.

use remedy_ui_core::{ColorMatrix, Rect};
use remedy_ui_paint::{Affine, DisplayList, Item, PaintLimits};
use vello_cpu::kurbo;
use vello_cpu::peniko::{ImageQuality, ImageSampler};
use vello_cpu::{ImageSource, Pixmap};

use crate::Surface;

/// A matrix group's picture starts on this grid of target pixels: blurs inside it shrink their
/// source by halves on a grid that starts at the picture's corner, so a picture starting
/// elsewhere would blur a level or two differently from tile to tile.
const GRID: f32 = 64.0;

/// How many matrix groups a raster has open and has drawn, against the raster's
/// [`PaintLimits`] (read once when it starts, so a raster keeps one set of bounds).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budget {
    pub(crate) limits: PaintLimits,
    pub(crate) depth: usize,
    pub(crate) drawn: usize,
    /// Glass: how deep in replays of what is under glass, and how many replays so far,
    /// against [`PaintLimits::backdrop_depth`] and [`PaintLimits::backdrop_replays`].
    pub(crate) glass_depth: usize,
    pub(crate) glass_drawn: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { limits: remedy_ui_paint::paint_limits(), depth: 0, drawn: 0, glass_depth: 0, glass_drawn: 0 }
    }
}

impl Budget {
    /// True (counting it) when one more pane of glass may draw again what is under it.
    pub(crate) fn admit_glass(&mut self) -> bool {
        let ok =
            self.glass_depth < self.limits.backdrop_depth && self.glass_drawn < self.limits.backdrop_replays;
        if ok {
            self.glass_drawn += 1;
        }
        ok
    }

    /// The matrix a group is drawn through, or `None` when it is drawn as a plain group: the
    /// identity, or past the limits.
    pub(crate) fn admit(&self, matrix: &ColorMatrix) -> Option<ColorMatrix> {
        let m = matrix.limited();
        (!m.is_identity()
            && self.depth < self.limits.filter_matrix_depth
            && self.drawn < self.limits.filter_matrix_groups)
            .then_some(m)
    }
}

/// The index of the item closing the group opened at `at` (a group is closed by a layer or
/// filter pop, as the painter closes them), or `items.len()` when the list leaves it open.
pub(crate) fn group_end(items: &[Item], at: usize) -> usize {
    let mut depth = 0usize;
    for (k, item) in items.iter().enumerate().skip(at + 1) {
        match item {
            Item::PushLayer { .. } | Item::PushFilter(_) => depth += 1,
            Item::PopLayer | Item::PopFilter if depth == 0 => return k,
            Item::PopLayer | Item::PopFilter => depth -= 1,
            _ => {}
        }
    }
    items.len()
}

/// Where the run of items `inner`, drawn through `t`, can draw inside `target` (target
/// pixels): its bounds, cut to the target. A run whose clips and groups do not nest (a clip
/// closed by a group's pop, which the painter allows: any pop closes the layer on top) may
/// draw beyond what [`DisplayList::bounds_of`] says, so there it is the whole target.
/// `None` when it draws nothing there.
pub(crate) fn drawn_box(inner: &[Item], t: Affine, target: Rect) -> Option<Rect> {
    let drawn = if nests(inner) { t.apply_rect(DisplayList::bounds_of(inner)?) } else { target };
    drawn.intersect(target).filter(|r| !r.is_empty())
}

/// True when every pop in `items` closes what it names (a clip a clip, a group a group) and
/// nothing is closed that was not opened in them; groups may be left open.
fn nests(items: &[Item]) -> bool {
    let mut open: Vec<bool> = Vec::new();
    for item in items {
        match item {
            Item::PushClip { .. } => open.push(true),
            Item::PushLayer { .. } | Item::PushFilter(_) => open.push(false),
            Item::PopClip if open.pop() != Some(true) => return false,
            Item::PopLayer | Item::PopFilter if open.pop() != Some(false) => return false,
            _ => {}
        }
    }
    true
}

/// The whole pixels `[x, y, width, height]` of a `size` target a group drawing over `drawn`
/// (target pixels) is pictured in: what it draws inside the target, its corner on the grid.
/// `None` when it draws nothing there.
pub(crate) fn picture_rect(drawn: Rect, size: (u16, u16)) -> Option<[u16; 4]> {
    let (w, h) = (f32::from(size.0), f32::from(size.1));
    let r = drawn.intersect(Rect::new(0.0, 0.0, w, h))?;
    if r.is_empty()
        || !(r.min.x.is_finite() && r.min.y.is_finite() && r.max.x.is_finite() && r.max.y.is_finite())
    {
        return None;
    }
    let x0 = ((r.min.x / GRID).floor() * GRID).max(0.0);
    let y0 = ((r.min.y / GRID).floor() * GRID).max(0.0);
    let x1 = r.max.x.ceil().min(w);
    let y1 = r.max.y.ceil().min(h);
    (x1 > x0 && y1 > y0).then_some([x0 as u16, y0 as u16, (x1 - x0) as u16, (y1 - y0) as u16])
}

/// Fills `[x, y, w, h]` of `s` with `source`'s pixels, one for one.
pub(crate) fn place<S: Surface>(s: &mut S, source: ImageSource, [x, y, w, h]: [u16; 4]) {
    let (x, y) = (f64::from(x), f64::from(y));
    s.set_transform(kurbo::Affine::IDENTITY);
    s.set_paint(vello_cpu::Image {
        image: source,
        sampler: ImageSampler { quality: ImageQuality::Low, ..Default::default() },
    });
    s.set_paint_transform(kurbo::Affine::translate((x, y)));
    s.fill_rect(&kurbo::Rect::new(x, y, x + f64::from(w), y + f64::from(h)));
    s.reset_paint_transform();
}

/// Renders `ctx` (a whole picture) to pixels and runs them through `matrix`.
pub(crate) fn finish(mut ctx: vello_cpu::RenderContext, matrix: &ColorMatrix) -> Pixmap {
    ctx.flush();
    let mut pix = Pixmap::new(ctx.width(), ctx.height());
    ctx.render(&mut pix, &mut vello_cpu::Resources::new());
    matrix.apply_to_pixels(pix.data_as_u8_slice_mut());
    pix
}
