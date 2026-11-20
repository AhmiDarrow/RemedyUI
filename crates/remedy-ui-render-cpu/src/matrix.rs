//! Colour-matrix groups ([`Filter::ColorMatrix`](remedy_ui_paint::Filter::ColorMatrix)).
//! vello's filter graph runs no colour matrices, so a group with one is drawn to pixels of its
//! own (in device space, at the target's resolution), run through the matrix, and placed back
//! pixel for pixel. Both renderers and both walks (display lists and composites) share what is
//! here, so they cut the same pictures.

use remedy_ui_core::{ColorMatrix, Rect};
use remedy_ui_paint::{Item, MAX_FILTER_MATRIX_DEPTH, MAX_FILTER_MATRIX_GROUPS};
use vello_cpu::kurbo;
use vello_cpu::peniko::{ImageQuality, ImageSampler};
use vello_cpu::{ImageSource, Pixmap};

use crate::Surface;

/// A matrix group's picture starts on this grid of target pixels: blurs inside it shrink their
/// source by halves on a grid that starts at the picture's corner, so a picture starting
/// elsewhere would blur a level or two differently from tile to tile.
const GRID: f32 = 64.0;

/// How many matrix groups a raster has open and has drawn, against
/// [`MAX_FILTER_MATRIX_DEPTH`] and [`MAX_FILTER_MATRIX_GROUPS`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Budget {
    pub(crate) depth: usize,
    pub(crate) drawn: usize,
}

impl Budget {
    /// The matrix a group is drawn through, or `None` when it is drawn as a plain group: the
    /// identity, or past the limits.
    pub(crate) fn admit(&self, matrix: &ColorMatrix) -> Option<ColorMatrix> {
        let m = matrix.limited();
        (!m.is_identity() && self.depth < MAX_FILTER_MATRIX_DEPTH && self.drawn < MAX_FILTER_MATRIX_GROUPS)
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
