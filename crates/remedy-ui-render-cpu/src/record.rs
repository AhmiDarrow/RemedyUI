//! The composite walk both compositors share: which ops a rectangle of the frame draws, how
//! glass replays what is under it, and how filter groups bound their source. One walk, so a
//! frame drawn from damage is the same frame on the CPU and the GPU.

use remedy_ui_compositor::{CompositeOp, LayerId, TileKey};
use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_paint::{FilterBudget, MAX_GROUP_DEPTH};
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
/// the frame's start. At most [`MAX_GROUP_DEPTH`] clips and layers are open at once: a push
/// past it opens nothing (what it holds is drawn as if it were not there), and its pop closes
/// nothing.
pub fn record_ops<T: CompositeTarget>(
    target: &mut T,
    all: &[CompositeOp],
    first: usize,
    area: Rect,
    close: bool,
    shift: kurbo::Affine,
    background: Color,
) {
    walk(target, all, first, area, FilterBudget::FULL, close, shift, background, 0);
}

/// The most clips and layers glass opens around its replay.
const GLASS_DEPTH: usize = 4;

/// What a push left open, closed by its pop.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    /// A clip path.
    Clip,
    /// A layer.
    Layer,
    /// Nothing: the push was past [`MAX_GROUP_DEPTH`].
    Skipped,
}

/// The clips and layers a walk has open, bounded by [`MAX_GROUP_DEPTH`].
struct Stack {
    /// What each push left open, innermost last.
    open: Vec<Open>,
    /// Clips and layers open, around the walk and in it.
    depth: usize,
}

impl Stack {
    /// Whether `n` more clips and layers fit under the bound.
    fn fits(&self, n: usize) -> bool {
        self.depth + n <= MAX_GROUP_DEPTH
    }

    /// Records a push of `kind`: true if it may open it, false (and nothing to close at its
    /// pop) past the bound.
    fn push(&mut self, kind: Open) -> bool {
        let fits = self.fits(1);
        self.open.push(if fits { kind } else { Open::Skipped });
        if fits {
            self.depth += 1;
        }
        fits
    }

    /// Closes the latest push, if it opened anything.
    fn pop<T: CompositeTarget>(&mut self, target: &mut T) {
        match self.open.pop() {
            Some(Open::Clip) => target.pop_clip(),
            Some(Open::Layer) => target.pop_layer(),
            Some(Open::Skipped) | None => return,
        }
        self.depth -= 1;
    }
}

/// [`record_ops`] with `depth` clips and layers already open around the ops, inside filters
/// that leave `budget` to the filters within (a backdrop's replay is drawn inside its blur and
/// the filter groups around it).
#[allow(clippy::too_many_arguments)]
fn walk<T: CompositeTarget>(
    target: &mut T,
    all: &[CompositeOp],
    first: usize,
    area: Rect,
    budget: FilterBudget,
    close: bool,
    shift: kurbo::Affine,
    background: Color,
    depth: usize,
) {
    let ops = all.get(first..).unwrap_or(&[]);
    // Inside a skipped subtree: how many pushes deep.
    let mut skip = 0u32;
    // What is open.
    let mut open = Stack { open: Vec::new(), depth };
    // What is culled against: inside a filter group, its whole region, so the group's source
    // is the same whatever part of the frame is drawn. With each, what the filter groups open
    // around it leave the filters inside them (see `FilterBudget`).
    let mut areas = vec![(area, budget)];
    let misses = |r: Rect, areas: &[(Rect, FilterBudget)]| {
        r.intersect(areas.last().map_or(area, |a| a.0)).is_none_or(|r| r.is_empty())
    };
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
                if !open.push(Open::Clip) {
                    continue;
                }
                target.set_transform(shift * kaffine(*transform));
                if *radii == CornerRadii::ZERO {
                    target.push_clip_rect(&krect(*rect));
                } else {
                    target.push_clip_path(&rounded(*rect, *radii));
                }
            }
            CompositeOp::PopClip | CompositeOp::PopOpacity => open.pop(target),
            CompositeOp::PushOpacity(o) => {
                if open.push(Open::Layer) {
                    target.push_opacity_layer(*o);
                }
            }
            CompositeOp::PushFilter { region, .. } if misses(*region, &areas) => {
                skip = 1;
            }
            CompositeOp::PushFilter { transform, filter, region } => {
                // A filter group is a layer and a clip path, closed in that order. Past the
                // depth bound it is neither, and its content is drawn unfiltered.
                let left = areas.last().map_or(budget, |a| a.1);
                if !open.fits(2) {
                    open.push(Open::Skipped);
                    open.push(Open::Skipped);
                    areas.push((*region, left));
                    continue;
                }
                open.push(Open::Layer);
                open.push(Open::Clip);
                target.set_transform(shift * kaffine(*transform));
                let (filter, inside) = left.limit(filter, transform.scale_factor());
                areas.push((*region, inside));
                match crate::vello_filter(&filter) {
                    Some(f) => target.push_filter_layer(f),
                    None => target.push_opacity_layer(1.0),
                }
                // The group's source, bounded by its region: the same picture to filter in
                // every rectangle that draws it.
                target.set_transform(shift);
                target.push_clip_rect(&krect(*region));
            }
            CompositeOp::PopFilter => {
                open.pop(target);
                open.pop(target);
                if areas.len() > 1 {
                    areas.pop();
                }
            }
            CompositeOp::Backdrop { transform, rect, radii, blur, opacity } => {
                // Glass opens up to three layers and a clip, and replays what is under it
                // inside them: past the depth bound it is not drawn.
                if misses(transform.apply_rect(*rect), &areas) || !open.fits(GLASS_DEPTH) {
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
                // What is under the glass is drawn inside its blur, and inside the filter
                // groups around it.
                let left = areas.last().map_or(budget, |a| a.1);
                let glass = remedy_ui_paint::Filter::Blur { radius: *blur };
                let (glass, inside) = left.limit(&glass, transform.scale_factor());
                match crate::vello_filter(&glass) {
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
                    walk(
                        target,
                        prefix,
                        root,
                        under,
                        inside,
                        true,
                        shift,
                        background,
                        open.depth + GLASS_DEPTH,
                    );
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
        while !open.open.is_empty() {
            open.pop(target);
        }
    }
}
