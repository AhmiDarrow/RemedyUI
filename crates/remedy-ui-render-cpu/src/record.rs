//! The composite walk both compositors share: which ops a rectangle of the frame draws, how
//! glass replays what is under it, and how filter groups bound their source. One walk, so a
//! frame drawn from damage is the same frame on the CPU and the GPU.

use remedy_ui_compositor::{CompositeOp, LayerId, TileKey};
use remedy_ui_core::{Color, CornerRadii, Rect};
use vello_cpu::kurbo;

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
    let ops = all.get(first..).unwrap_or(&[]);
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
            CompositeOp::PushFilter { transform, filter, region } => {
                target.set_transform(shift * kaffine(*transform));
                match crate::layer_filter(filter, transform.scale_factor()) {
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
                if misses(transform.apply_rect(*rect), &areas) {
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
                // panes in one backdrop root draw what is under the first 2^(n-1) times; drawing
                // each root's picture once into a texture both renderers sample would make it
                // linear, if stacks of glass become common.
                if let Some(prefix) = all.get(..at) {
                    record_ops(target, prefix, root, under, true, shift, background);
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
