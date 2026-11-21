//! The composite walk both compositors share: which ops a rectangle of the frame draws, how
//! glass replays what is under it, and how filter groups bound their source. One walk, so a
//! frame drawn from damage is the same frame on the CPU and the GPU.

use remedy_ui_compositor::{CompositeOp, LayerId, TileKey};
use remedy_ui_core::{Color, CornerRadii, Rect};
use vello_cpu::kurbo;

use crate::matrix::{Budget, picture_rect};
use crate::{kaffine, krect, rounded};

/// The filter a layer is drawn through, as both renderers take it.
pub type LayerFilter = vello_cpu::filter_effects::Filter;

/// What the walk draws into: a renderer's scene or render context.
pub trait CompositeTarget {
    /// Sets the transform for what is drawn next.
    fn set_transform(&mut self, transform: kurbo::Affine);
    /// Clips what follows to `rect` until [`pop_clip`](Self::pop_clip).
    fn push_clip_rect(&mut self, rect: &kurbo::Rect);
    /// Clips what follows to `path` until [`pop_clip`](Self::pop_clip).
    fn push_clip_path(&mut self, path: &kurbo::BezPath);
    /// Ends the latest clip.
    fn pop_clip(&mut self);
    /// Starts a layer clipped to `path`.
    fn push_clip_layer(&mut self, path: &kurbo::BezPath);
    /// Starts a layer drawn at `opacity`.
    fn push_opacity_layer(&mut self, opacity: f32);
    /// Starts a layer drawn through `filter`.
    fn push_filter_layer(&mut self, filter: LayerFilter);
    /// Ends the latest layer.
    fn pop_layer(&mut self);
    /// Fills `rect` with `color`.
    fn fill_color(&mut self, rect: &kurbo::Rect, color: Color);
    /// Fills `part` with the pixels of the tile `(layer, key)`, placed by `paint_transform`
    /// (sampled nearest when `exact`); nothing when the tile is not held.
    fn fill_tile(
        &mut self,
        tile: (LayerId, TileKey),
        paint_transform: kurbo::Affine,
        exact: bool,
        part: &kurbo::Rect,
    );
    /// The size of what the walk draws into, in its pixels.
    fn size(&self) -> (u16, u16);
    /// Draws a colour-matrix group, as [`Surface::color_matrix_group`](crate::Surface)
    /// does: what `draw` records (into this target, which inside it is a transparent
    /// picture `rect`'s size) run through `matrix` and placed at `rect`. False, without
    /// calling `draw`, when the target cannot.
    fn color_matrix_group(
        &mut self,
        rect: [u16; 4],
        matrix: &remedy_ui_core::ColorMatrix,
        draw: &mut dyn FnMut(&mut Self),
    ) -> bool
    where
        Self: Sized;
}

/// Records the ops of `all` from index `first` that touch `area` (device pixels) into
/// `target`, shifted by `shift` (device pixels to the target's). With `close`, layers they
/// leave open are closed (a backdrop's replay). `background` is drawn under glass that reaches
/// the frame's start.
pub fn record_ops<T: CompositeTarget>(
    target: &mut T,
    all: &[CompositeOp],
    first: usize,
    area: Rect,
    close: bool,
    shift: kurbo::Affine,
    background: Color,
) {
    let mut budget = Budget::default();
    walk(target, all, first, area, close, shift, background, &mut budget);
}

/// The index in `all` of the pop closing the filter group opened at `at`, or `all.len()`.
fn filter_end(all: &[CompositeOp], at: usize) -> usize {
    let mut depth = 0usize;
    for (k, op) in all.iter().enumerate().skip(at + 1) {
        match op {
            CompositeOp::PushClip { .. } | CompositeOp::PushOpacity(_) | CompositeOp::PushFilter { .. } => {
                depth += 1;
            }
            CompositeOp::PopFilter if depth == 0 => return k,
            CompositeOp::PopClip | CompositeOp::PopOpacity | CompositeOp::PopFilter => {
                depth = depth.saturating_sub(1);
            }
            CompositeOp::Tile { .. } | CompositeOp::Backdrop { .. } => {}
        }
    }
    all.len()
}

#[allow(clippy::too_many_arguments, reason = "record_ops' arguments and the matrix budget")]
fn walk<T: CompositeTarget>(
    target: &mut T,
    all: &[CompositeOp],
    first: usize,
    area: Rect,
    close: bool,
    shift: kurbo::Affine,
    background: Color,
    budget: &mut Budget,
) {
    let ops = all.get(first..).unwrap_or(&[]);
    // Ops before this (relative to `first`) were drawn already (a colour-matrix group, as
    // one picture).
    let mut resume = 0usize;
    // Inside a skipped subtree: how many pushes deep.
    let mut skip = 0u32;
    // What is open: true for a clip path, false for a layer.
    let mut open: Vec<bool> = Vec::new();
    // What is culled against: inside a filter group, its whole region, so the group's source
    // is the same whatever part of the frame is drawn.
    let mut areas = vec![area];
    let misses =
        |r: Rect, areas: &[Rect]| r.intersect(*areas.last().unwrap_or(&area)).is_none_or(|r| r.is_empty());
    for (k, op) in ops.iter().enumerate() {
        if k < resume {
            continue;
        }
        if skip > 0 {
            match op {
                CompositeOp::PushClip { .. }
                | CompositeOp::PushOpacity(_)
                | CompositeOp::PushFilter { .. } => skip += 1,
                CompositeOp::PopClip | CompositeOp::PopOpacity | CompositeOp::PopFilter => skip -= 1,
                CompositeOp::Tile { .. } | CompositeOp::Backdrop { .. } => {}
            }
            continue;
        }
        match op {
            CompositeOp::PushClip { transform, rect, .. } if misses(transform.apply_rect(*rect), &areas) => {
                skip = 1;
            }
            CompositeOp::PushClip { transform, rect, radii } => {
                // A clip path, not an isolated clip layer: clipping needs no layer of its own,
                // and one per clipped element made every composite that many groups.
                target.set_transform(shift * kaffine(*transform));
                if *radii == CornerRadii::ZERO {
                    target.push_clip_rect(&krect(*rect));
                } else {
                    target.push_clip_path(&rounded(*rect, *radii));
                }
                open.push(true);
            }
            CompositeOp::PopClip => {
                target.pop_clip();
                open.pop();
            }
            CompositeOp::PushOpacity(o) => {
                target.push_opacity_layer(*o);
                open.push(false);
            }
            CompositeOp::PopOpacity => {
                target.pop_layer();
                open.pop();
            }
            CompositeOp::PushFilter { region, .. } if misses(*region, &areas) => {
                skip = 1;
            }
            CompositeOp::PushFilter {
                filter: remedy_ui_paint::Filter::ColorMatrix { matrix },
                region,
                ..
            } if let Some(m) = budget.admit(matrix) => {
                let end = filter_end(all, first + k);
                // What of the group shows here, in the target's pixels.
                let shown = region.intersect(*areas.last().unwrap_or(&area)).and_then(|r| r.intersect(area));
                let to_target = |r: Rect| {
                    let k = shift.transform_rect_bbox(krect(r));
                    Rect::new(k.x0 as f32, k.y0 as f32, k.x1 as f32, k.y1 as f32)
                };
                let Some(rect) = shown.and_then(|r| picture_rect(to_target(r), target.size())) else {
                    resume = end + 1 - first;
                    continue;
                };
                let [x, y, w, h] = rect;
                // The picture's pixels, in device pixels: where the group's ops are culled.
                let picture = shift.inverse().transform_rect_bbox(kurbo::Rect::new(
                    f64::from(x),
                    f64::from(y),
                    f64::from(x) + f64::from(w),
                    f64::from(y) + f64::from(h),
                ));
                let sub_area =
                    Rect::new(picture.x0 as f32, picture.y0 as f32, picture.x1 as f32, picture.y1 as f32);
                let sub_shift = kurbo::Affine::translate((-f64::from(x), -f64::from(y))) * shift;
                let group = all.get(..end).unwrap_or(all);
                budget.depth += 1;
                budget.drawn += 1;
                let ok = target.color_matrix_group(rect, &m, &mut |sub| {
                    walk(sub, group, first + k + 1, sub_area, true, sub_shift, background, budget);
                });
                budget.depth -= 1;
                if ok {
                    resume = end + 1 - first;
                    continue;
                }
                // Drawn as a plain group, closed by its pop as any other.
                target.set_transform(shift);
                target.push_opacity_layer(1.0);
                target.push_clip_rect(&krect(*region));
                open.push(false);
                open.push(true);
                areas.push(*region);
            }
            CompositeOp::PushFilter { transform, filter, region } => {
                target.set_transform(shift * kaffine(*transform));
                // The region is what the group draws grown by its filters' reach (offsets
                // included) on every side; shrunk by this filter's own reach it still holds
                // the drawing. A shadow wholly outside the target shows nothing and is not
                // drawn (the vello renderers mishandle one; see `shadow_beyond_target`).
                let in_target = {
                    let r = CompositeOp::filter_reach(transform, filter);
                    let drawn =
                        Rect::new(region.min.x + r, region.min.y + r, region.max.x - r, region.max.y - r);
                    let k = shift.transform_rect_bbox(krect(drawn));
                    Rect::new(k.x0 as f32, k.y0 as f32, k.x1 as f32, k.y1 as f32)
                };
                // vello's buffer never reaches past the target: what is drawn there.
                let (w, h) = target.size();
                let whole = Rect::new(0.0, 0.0, f32::from(w), f32::from(h));
                let unseen = in_target
                    .intersect(whole)
                    .is_some_and(|d| crate::shadow_beyond_target(filter, *transform, d, whole));
                match crate::layer_filter(filter, transform.scale_factor()).filter(|_| !unseen) {
                    Some(f) => target.push_filter_layer(f),
                    None => target.push_opacity_layer(1.0),
                }
                // The group's source, bounded by its region: the same picture to filter in
                // every rectangle that draws it.
                target.set_transform(shift);
                target.push_clip_rect(&krect(*region));
                // A filter group is a layer and a clip path, closed in that order.
                open.push(false);
                open.push(true);
                areas.push(*region);
            }
            CompositeOp::PopFilter => {
                target.pop_clip();
                target.pop_layer();
                open.pop();
                open.pop();
                if areas.len() > 1 {
                    areas.pop();
                }
            }
            CompositeOp::Backdrop { transform, rect, radii, blur, opacity } => {
                // Glass past the replay budget is drawn without its frost (see Budget): each
                // pane draws again what is under it, glass included.
                if misses(transform.apply_rect(*rect), &areas) || !budget.admit_glass() {
                    continue;
                }
                let at = first + k;
                let root = CompositeOp::backdrop_root(all, at);
                // What the glass reads: its own box and the blur's reach around it, the same
                // whatever part of the frame is drawn, so the blur is too.
                let reach = CompositeOp::backdrop_reach(transform, *blur);
                let g = transform.apply_rect(*rect);
                let under = Rect::new(g.min.x - reach, g.min.y - reach, g.max.x + reach, g.max.y + reach);
                target.set_transform(shift * kaffine(*transform));
                target.push_clip_layer(&rounded(*rect, *radii));
                if *opacity < 1.0 {
                    target.push_opacity_layer(*opacity);
                }
                match crate::backdrop_filter(*blur, transform.scale_factor()) {
                    Some(f) => target.push_filter_layer(f),
                    None => target.push_opacity_layer(1.0),
                }
                target.set_transform(shift);
                target.push_clip_rect(&krect(under));
                if root == 0 {
                    // The frame's start: its background is under everything.
                    target.fill_color(&krect(under), background);
                }
                // shortcut: glass under glass replays its own source again, so n overlapping
                // panes in one backdrop root would draw what is under the first 2^(n-1) times;
                // the budget bounds that (glass seen through more than MAX_BACKDROP_DEPTH panes
                // goes without its frost). Drawing each root's picture once into a texture both
                // renderers sample would make it linear, if stacks of glass become common.
                if let Some(prefix) = all.get(..at) {
                    budget.glass_depth += 1;
                    walk(target, prefix, root, under, true, shift, background, budget);
                    budget.glass_depth -= 1;
                }
                target.pop_clip();
                target.pop_layer();
                if *opacity < 1.0 {
                    target.pop_layer();
                }
                target.pop_layer();
            }
            CompositeOp::Tile { layer, key, transform, rect, raster_scale, area: part } => {
                if misses(transform.apply_rect(*part), &areas) {
                    continue;
                }
                // Pixel-exact placement (the common, static case) samples nearest so tiles stay
                // crisp and seamless; scaled or rotated tiles filter.
                let [a, b, c, d, e, f] = transform.0;
                let exact = b == 0.0
                    && c == 0.0
                    && a == *raster_scale
                    && d == *raster_scale
                    && e.fract() == 0.0
                    && f.fract() == 0.0;
                target.set_transform(shift * kaffine(*transform));
                let inv = 1.0 / f64::from(*raster_scale);
                let paint = kurbo::Affine::translate((f64::from(rect.min.x), f64::from(rect.min.y)))
                    * kurbo::Affine::scale(inv);
                target.fill_tile((*layer, *key), paint, exact, &krect(*part));
            }
        }
    }
    if close {
        for clip in open.into_iter().rev() {
            if clip {
                target.pop_clip();
            } else {
                target.pop_layer();
            }
        }
    }
}
