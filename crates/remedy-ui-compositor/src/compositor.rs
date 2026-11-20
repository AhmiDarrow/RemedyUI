//! The compositor: takes commits, owns scroll offsets and compositor animations, decides
//! which tiles to raster, and plans each frame with its damage.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::{CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_input::{ScrollAxis, ScrollPhysics};
use remedy_ui_motion::{Animation, Motion};
use remedy_ui_paint::{Affine, DisplayList};

use crate::damage::Damage;
use crate::layer::{Layer, LayerId, LayerTree};
use crate::tiles::{Cell, LayerTiles, RasterBudget, TileKey, quantize_scale, tile_pixels};

/// One step of drawing a frame from rastered tiles.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: the compositor's operations, which every compositor backend must apply completely, so a new kind is a major change.
pub enum CompositeOp {
    /// Clips what follows to `rect` with `radii`, both in the space `transform` maps to device
    /// pixels.
    PushClip {
        /// Clip space to device pixels.
        transform: Affine,
        /// The clip box.
        rect: Rect,
        /// Its corners.
        radii: CornerRadii,
    },
    /// Ends the latest clip.
    PopClip,
    /// Starts a group drawn at `opacity`.
    PushOpacity(f32),
    /// Ends the latest group.
    PopOpacity,
    /// Starts drawing what follows, up to the matching [`PopFilter`](CompositeOp::PopFilter),
    /// through `filter` as one picture: a layer's [`filters`](crate::Layer::filters).
    PushFilter {
        /// The layer's space to device pixels (the filter's lengths are in its units).
        transform: Affine,
        /// The filter.
        filter: remedy_ui_paint::Filter,
        /// Everything the group can draw, filter included, in device pixels. A renderer
        /// draws the group's whole source within it whatever part of the frame it draws, so
        /// the filter comes out the same.
        region: Rect,
    },
    /// Ends the latest filter group.
    PopFilter,
    /// Frosted glass: blurs what the ops since the backdrop root drew (the start of the frame,
    /// or the innermost [`PushOpacity`](CompositeOp::PushOpacity) group still open, see
    /// [`CompositeOp::backdrop_root`]) and draws it inside `rect` with `radii`, at `opacity`,
    /// before the layer's content.
    Backdrop {
        /// Glass space to device pixels (the blur is in this space's units).
        transform: Affine,
        /// The glass.
        rect: Rect,
        /// Its corners.
        radii: CornerRadii,
        /// The blur's standard deviation.
        blur: f32,
        /// The glass layer's own opacity, which fades its backdrop with it.
        opacity: f32,
    },
    /// Draws a rastered tile: its pixels cover `rect` in the space `transform` maps to device
    /// pixels, and the part of it inside `area` is drawn. A tile of the current scale draws
    /// whole (`area == rect`); a tile of an earlier scale standing in during a zoom draws
    /// only where the missing tile would be.
    Tile {
        /// The layer.
        layer: LayerId,
        /// The tile.
        key: TileKey,
        /// Content space to device pixels.
        transform: Affine,
        /// The tile's content area.
        rect: Rect,
        /// Device pixels per content unit the tile was rastered at.
        raster_scale: f32,
        /// The part of `rect` to draw (content space).
        area: Rect,
    },
}

impl CompositeOp {
    /// Where the picture a backdrop at `ops[at]` blurs starts: just after the innermost
    /// [`PushOpacity`](CompositeOp::PushOpacity) or [`PushFilter`](CompositeOp::PushFilter)
    /// still open there, or 0 (the start of the frame, background included).
    pub fn backdrop_root(ops: &[CompositeOp], at: usize) -> usize {
        let mut open: Vec<(bool, usize)> = Vec::new();
        for (i, op) in ops.iter().take(at).enumerate() {
            match op {
                CompositeOp::PushClip { .. } => open.push((false, i)),
                CompositeOp::PushOpacity(_) | CompositeOp::PushFilter { .. } => open.push((true, i)),
                CompositeOp::PopClip | CompositeOp::PopOpacity | CompositeOp::PopFilter => {
                    open.pop();
                }
                CompositeOp::Tile { .. } | CompositeOp::Backdrop { .. } => {}
            }
        }
        open.iter().rev().find(|(group, _)| *group).map_or(0, |(_, i)| i + 1)
    }

    /// How far a filter pushed under `transform` carries drawing past where it is drawn, in
    /// device pixels: three standard deviations and its offset, scaled to the device, and one
    /// pixel more.
    pub fn filter_reach(transform: &Affine, filter: &remedy_ui_paint::Filter) -> f32 {
        let filter = &filter.limited(transform.scale_factor());
        let off = match filter {
            remedy_ui_paint::Filter::DropShadow { offset, .. } => offset.x.abs().max(offset.y.abs()),
            remedy_ui_paint::Filter::Blur { .. } => 0.0,
        };
        let off = if off.is_finite() { off } else { 0.0 };
        let r = ((filter.std_dev() * 3.0 + off) * transform.scale_factor()).ceil() + 1.0;
        if r.is_finite() { r } else { 0.0 }
    }

    /// How far past its glass a backdrop reads what is under it, in device pixels: three
    /// standard deviations of its blur, scaled to the device, and one pixel more.
    pub fn backdrop_reach(transform: &Affine, blur: f32) -> f32 {
        let b = remedy_ui_paint::Filter::Blur { radius: blur }.limited(transform.scale_factor()).std_dev();
        let r = (b * 3.0 * transform.scale_factor()).ceil() + 1.0;
        if r.is_finite() { r } else { 0.0 }
    }
}

/// Rectangles redrawing glass start on this grid of device pixels: a blur shrinks its source
/// by halves on a grid that starts at the corner of what is drawn, so glass drawn from a
/// rectangle starting elsewhere would come out a level or two different.
const GLASS_GRID: f32 = 64.0;

impl FramePlan {
    /// The rectangles a renderer that keeps the last frame draws again, in whole device pixels
    /// of a `width` × `height` frame: the damage with a pixel more around each rectangle (a
    /// filtered tile may touch the pixel past its box), or the whole frame when there is no
    /// frame to keep (`kept` false: the first frame, a new size) or the damage covers most of
    /// it. Rectangles that draw glass ([`CompositeOp::Backdrop`]) or a filter group
    /// ([`CompositeOp::PushFilter`]) start on a 64-pixel grid,
    /// so the glass comes out exactly as in a full frame. Both compositors use it, so a frame
    /// drawn from damage is the same frame on either.
    pub fn redraw_rects(&self, width: u32, height: u32, kept: bool) -> Vec<Rect> {
        let full = Rect::new(0.0, 0.0, width as f32, height as f32);
        if !kept || self.damage.area() >= full.size().area() * 0.6 {
            return vec![full];
        }
        let glass: Vec<Rect> = self
            .ops
            .iter()
            .filter_map(|op| match op {
                CompositeOp::Backdrop { transform, rect, blur, .. } => {
                    let g = transform.apply_rect(*rect);
                    let r = CompositeOp::backdrop_reach(transform, *blur);
                    Some(Rect::new(g.min.x - r, g.min.y - r, g.max.x + r, g.max.y + r))
                }
                CompositeOp::PushFilter { region, .. } => Some(*region),
                _ => None,
            })
            .collect();
        let mut out = Damage::new();
        for r in self.damage.rects() {
            let mut r = Rect::new(r.min.x - 1.0, r.min.y - 1.0, r.max.x + 1.0, r.max.y + 1.0);
            if glass.iter().any(|g| g.intersect(r).is_some_and(|i| !i.is_empty())) {
                r.min.x = (r.min.x / GLASS_GRID).floor() * GLASS_GRID;
                r.min.y = (r.min.y / GLASS_GRID).floor() * GLASS_GRID;
            }
            if let Some(r) = r.intersect(full) {
                out.add(r);
            }
        }
        out.rects().to_vec()
    }
}

/// A tile to raster before compositing this frame.
#[derive(Clone, Debug)]
pub struct TileRequest {
    /// The layer.
    pub layer: LayerId,
    /// The tile.
    pub key: TileKey,
    /// The content area to draw (in content space).
    pub rect: Rect,
    /// Device pixels per content unit.
    pub raster_scale: f32,
    /// Pixel width of the tile image.
    pub width: u32,
    /// Pixel height of the tile image.
    pub height: u32,
    /// What to draw.
    pub content: Arc<DisplayList>,
    /// True when the tile is on screen now (raster these first); false for prefetch.
    pub visible: bool,
}

impl TileRequest {
    /// The tile's size in pixels.
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// The transform that maps content space into this tile's pixels.
    pub fn raster_transform(&self) -> Affine {
        Affine::translate(Vec2::new(-self.rect.min.x, -self.rect.min.y))
            .then(Affine::scale(self.raster_scale, self.raster_scale))
    }
}

/// Everything a renderer needs for one frame.
#[derive(Clone, Debug, Default)]
pub struct FramePlan {
    /// Tiles to raster first, visible ones before prefetch.
    pub raster: Vec<TileRequest>,
    /// Tiles the renderer may free.
    pub released: Vec<(LayerId, TileKey)>,
    /// The composite, back to front.
    pub ops: Vec<CompositeOp>,
    /// What changed since the previous plan, in device pixels.
    pub damage: Damage,
    /// True while scrolling or animations still move, or a zoom is still rastering its new
    /// scale: request another frame.
    pub animating: bool,
    /// Visible tiles shown by stand-ins of an earlier scale this frame, rastering in later
    /// frames within the [`RasterBudget`].
    pub pending: usize,
    /// Ambient loops are on screen and running: draw the next frame this much later (at the
    /// ambient rate, not every vsync). `None` when nothing loops or loops are paused.
    pub wake_in: Option<Duration>,
}

#[derive(Clone, Debug)]
struct Overrides {
    opacity: Animation<f32>,
    offset: Animation<Vec2>,
    scale: Animation<f32>,
}

impl Overrides {
    fn new() -> Self {
        Overrides {
            opacity: Animation::new(1.0),
            offset: Animation::new(Vec2::ZERO),
            scale: Animation::new(1.0),
        }
    }

    fn settled(&self, now: Duration) -> bool {
        self.opacity.is_settled(now) && self.offset.is_settled(now) && self.scale.is_settled(now)
    }
}

#[derive(Clone, Copy, Debug)]
struct Scroller {
    x: ScrollAxis,
    y: ScrollAxis,
    /// Where the content's start shows ([`Layer::scroll_origin`](crate::Layer)).
    origin: Vec2,
}

#[derive(Clone, Debug, PartialEq)]
struct Drawn {
    transform: [f32; 6],
    opacity: f32,
    clip: Option<Rect>,
    version: u64,
    order: usize,
    raster_scale: f32,
    bounds: Option<Rect>,
    backdrop: Option<crate::layer::Backdrop>,
    filters: Vec<remedy_ui_paint::Filter>,
}

/// The compositor for one window. Single-threaded apps and tests drive it on the UI thread;
/// threaded ones hand commits over through a [`Mailbox`](crate::Mailbox).
#[derive(Debug)]
pub struct Compositor {
    tree: LayerTree,
    width: u32,
    height: u32,
    scale: f32,
    physics: ScrollPhysics,
    scrollers: HashMap<LayerId, Scroller>,
    /// Scroll containers that follow others: (follower, link).
    follows: Vec<(LayerId, crate::layer::ScrollFollow)>,
    overrides: HashMap<LayerId, Overrides>,
    grids: HashMap<LayerId, LayerTiles>,
    budget: RasterBudget,
    drawn: HashMap<LayerId, Drawn>,
    scroll_hits: Vec<(LayerId, Rect, f32)>,
    released: Vec<(LayerId, TileKey)>,
    full_damage: bool,
    last_frame: Option<Duration>,
    reduced_motion: bool,
    /// The ambient rate loops run at; `None` holds them where they are.
    ambient: Option<Duration>,
    /// Loop time: advances only while loops run, so a pause never makes them jump.
    ambient_t: Duration,
    ambient_last: Option<Duration>,
}

struct Walk<'a> {
    now: Duration,
    viewport: Rect,
    plan: &'a mut FramePlan,
    drawn: HashMap<LayerId, Drawn>,
    hits: Vec<(LayerId, Rect, f32)>,
    order: usize,
    /// Inside a scroll container: content near the viewport is about to show, so prefetch.
    scrolling: bool,
    /// Pixels this frame may raster at new scales.
    allowance: u64,
    /// Pixels rastered at new scales this frame.
    spent: u64,
    /// Device areas where a tile replaced its stand-in.
    replaced: Damage,
    /// The time ambient loops are sampled at.
    loop_t: Duration,
    /// A layer with ambient loops showed this frame.
    looping: bool,
    /// Prefetch tiles at a new scale left for a later frame.
    deferred: usize,
    /// What reads beyond where it draws: glass (its box in device pixels and how far past it
    /// it reads) and filter groups (their region, and 0). Damage in what one reads damages all
    /// of it.
    backdrops: Vec<(Rect, f32)>,
    /// How far the filter groups being walked carry drawing (device pixels): a layer's
    /// damage inside them is grown by it.
    grow: f32,
    /// For each filter group being walked, the area its layers draw so far (device pixels).
    regions: Vec<Option<Rect>>,
}

fn snap(a: Affine) -> Affine {
    let [x0, x1, x2, x3, x4, x5] = a.0;
    if x1 == 0.0 && x2 == 0.0 { Affine([x0, 0.0, 0.0, x3, x4.round(), x5.round()]) } else { a }
}

impl Compositor {
    /// A compositor for a window `width` × `height` device pixels at `scale` device pixels per
    /// logical pixel, scrolling with `physics`.
    pub fn new(width: u32, height: u32, scale: f32, physics: ScrollPhysics) -> Self {
        Compositor {
            tree: LayerTree::default(),
            width,
            height,
            scale: quantize_scale(scale),
            physics,
            scrollers: HashMap::new(),
            follows: Vec::new(),
            overrides: HashMap::new(),
            grids: HashMap::new(),
            budget: RasterBudget::default(),
            drawn: HashMap::new(),
            scroll_hits: Vec::new(),
            released: Vec::new(),
            full_damage: true,
            last_frame: None,
            reduced_motion: false,
            ambient: Some(Duration::from_secs(1) / 30),
            ambient_t: Duration::ZERO,
            ambient_last: None,
        }
    }

    /// The window size in device pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Device pixels per logical pixel.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The committed tree.
    pub fn tree(&self) -> &LayerTree {
        &self.tree
    }

    /// Follows the system's reduced-motion setting: compositor animations jump to their end.
    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
        for o in self.overrides.values_mut() {
            o.opacity.set_reduced_motion(on);
            o.offset.set_reduced_motion(on);
            o.scale.set_reduced_motion(on);
        }
    }

    /// Sets the rate ambient loops ([`AmbientLoop`](crate::AmbientLoop)) run at: a frame
    /// every `interval` while one shows, or `None` to hold them still and ask for no frames
    /// (the window is hidden, ambient motion is off). Default: 30 frames a second.
    pub fn set_ambient(&mut self, interval: Option<Duration>) {
        self.ambient = interval.filter(|d| !d.is_zero());
    }

    /// The rate ambient loops run at.
    pub fn ambient(&self) -> Option<Duration> {
        self.ambient
    }

    /// Resizes the window. The next frame redraws everything.
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.width = width;
        self.height = height;
        self.scale = quantize_scale(scale);
        self.full_damage = true;
    }

    /// Adopts a committed tree. Scroll offsets and animations of layers that remain are kept;
    /// state and tiles of layers that are gone are dropped.
    pub fn commit(&mut self, tree: LayerTree) {
        let mut alive = HashMap::new();
        let mut follows = Vec::new();
        tree.for_each(|l| {
            alive.insert(l.id, (l.scroll.is_some(), l.max_scroll(), l.scroll_origin));
            if let Some(f) = l.scroll_follow {
                follows.push((l.id, f));
            }
        });
        self.follows = follows;
        self.scrollers.retain(|id, _| alive.get(id).is_some_and(|(s, _, _)| *s));
        for (&id, &(scrolls, max, origin)) in &alive {
            if !scrolls {
                continue;
            }
            let physics = self.physics;
            let s = self.scrollers.entry(id).or_insert_with(|| {
                // A new container opens at the start of its content.
                let mut s = Scroller {
                    x: ScrollAxis::new(max.x, physics),
                    y: ScrollAxis::new(max.y, physics),
                    origin,
                };
                s.x.offset = origin.x.clamp(0.0, s.x.max);
                s.y.offset = origin.y.clamp(0.0, s.y.max);
                s
            });
            // Content that grew past the start (to the left, right to left) keeps what was
            // shown where it was.
            if s.origin != origin {
                s.x.offset = (s.x.offset + origin.x - s.origin.x).max(0.0);
                s.y.offset = (s.y.offset + origin.y - s.origin.y).max(0.0);
                s.origin = origin;
            }
            s.x.max = max.x;
            s.y.max = max.y;
            s.x.offset = s.x.offset.min(max.x);
            s.y.offset = s.y.offset.min(max.y);
        }
        self.overrides.retain(|id, _| alive.contains_key(id));
        let released = &mut self.released;
        self.grids.retain(|id, g| {
            let keep = alive.contains_key(id);
            if !keep {
                released.extend(g.all_keys().into_iter().map(|k| (*id, k)));
            }
            keep
        });
        self.tree = tree;
    }

    /// The scroll offset of a scrolling layer.
    pub fn scroll_offset(&self, id: LayerId) -> Option<Vec2> {
        self.scrollers.get(&id).map(|s| Vec2::new(s.x.offset, s.y.offset))
    }

    /// Jumps a scrolling layer to `offset` (clamped). Returns false for a layer that does not
    /// scroll.
    pub fn set_scroll_offset(&mut self, id: LayerId, offset: Vec2) -> bool {
        let Some(s) = self.scrollers.get_mut(&id) else { return false };
        s.x.offset = offset.x.clamp(0.0, s.x.max);
        s.y.offset = offset.y.clamp(0.0, s.y.max);
        s.x.velocity = 0.0;
        s.y.velocity = 0.0;
        true
    }

    /// Scrolls a layer by `delta` content units, as a drag or wheel does.
    pub fn scroll_by(&mut self, id: LayerId, delta: Vec2) -> bool {
        let Some(s) = self.scrollers.get_mut(&id) else { return false };
        s.x.drag(delta.x);
        s.y.drag(delta.y);
        true
    }

    /// Starts a fling on a scrolling layer with a release velocity in content units per second.
    pub fn fling(&mut self, id: LayerId, velocity: Vec2) -> bool {
        let Some(s) = self.scrollers.get_mut(&id) else { return false };
        s.x.fling(velocity.x);
        s.y.fling(velocity.y);
        true
    }

    /// Routes a wheel or touchpad scroll at `point` (device pixels) by `delta` (device pixels):
    /// the innermost scroll container under the point that can still move that way takes it,
    /// and an exhausted inner one hands it outward. Returns the layer that scrolled.
    pub fn scroll_at(&mut self, point: Point, delta: Vec2) -> Option<LayerId> {
        let hits: Vec<(LayerId, f32)> = self
            .scroll_hits
            .iter()
            .rev()
            .filter(|(_, r, _)| r.contains(point))
            .map(|(id, _, s)| (*id, *s))
            .collect();
        for (id, scale) in hits {
            // A pane that follows another on an axis passes that axis to its leader, so the
            // wheel over a frozen column scrolls the whole table.
            let follow = self.follows.iter().find(|(f, _)| *f == id).map(|(_, l)| *l);
            if let Some(f) = follow {
                let d = Vec2::new(delta.x / scale, delta.y / scale);
                let led = Vec2::new(if f.x { d.x } else { 0.0 }, if f.y { d.y } else { 0.0 });
                let own = Vec2::new(if f.x { 0.0 } else { d.x }, if f.y { 0.0 } else { d.y });
                let mut moved = None;
                if (led.x != 0.0 || led.y != 0.0) && self.scroll_by(f.leader, led) {
                    moved = Some(f.leader);
                }
                if (own.x != 0.0 || own.y != 0.0) && self.scroll_by(id, own) {
                    moved = moved.or(Some(id));
                }
                if moved.is_some() {
                    return moved;
                }
                continue;
            }
            let Some(s) = self.scrollers.get_mut(&id) else { continue };
            let d = Vec2::new(delta.x / scale, delta.y / scale);
            let can = |a: &ScrollAxis, d: f32| (d > 0.0 && a.offset < a.max) || (d < 0.0 && a.offset > 0.0);
            if can(&s.x, d.x) || can(&s.y, d.y) {
                s.x.drag(d.x);
                s.y.drag(d.y);
                return Some(id);
            }
        }
        None
    }

    fn overrides(&mut self, id: LayerId) -> &mut Overrides {
        let reduced = self.reduced_motion;
        self.overrides.entry(id).or_insert_with(|| {
            let mut o = Overrides::new();
            o.opacity.set_reduced_motion(reduced);
            o.offset.set_reduced_motion(reduced);
            o.scale.set_reduced_motion(reduced);
            o
        })
    }

    /// Animates a layer's opacity multiplier (1 is the committed opacity) on the compositor.
    pub fn animate_opacity(&mut self, id: LayerId, target: f32, motion: Motion, now: Duration) {
        self.overrides(id).opacity.animate_to(target.clamp(0.0, 1.0), motion, now);
    }

    /// Animates an extra offset (logical pixels in the parent's space) on the compositor.
    pub fn animate_offset(&mut self, id: LayerId, target: Vec2, motion: Motion, now: Duration) {
        self.overrides(id).offset.animate_to(target, motion, now);
    }

    /// Animates an extra scale about the layer's center on the compositor. Tiles keep their
    /// raster scale while it runs and are shown scaled.
    pub fn animate_scale(&mut self, id: LayerId, target: f32, motion: Motion, now: Duration) {
        self.overrides(id).scale.animate_to(target, motion, now);
    }

    /// True while a fling, overscroll return or compositor animation still moves.
    pub fn is_animating(&self, now: Duration) -> bool {
        let moving = |a: &ScrollAxis| a.velocity != 0.0 || a.offset < 0.0 || a.offset > a.max;
        self.scrollers.values().any(|s| moving(&s.x) || moving(&s.y))
            || self.overrides.values().any(|o| !o.settled(now))
    }

    /// Marks the whole window changed, keeping the rastered tiles: the next frame presents all
    /// of it (the platform did not keep what the window showed, as when it comes back into
    /// sight on X11).
    pub fn damage_all(&mut self) {
        self.full_damage = true;
    }

    /// Forgets every rastered tile without releasing it (the renderer lost them, as with a
    /// lost GPU device): the next frame rasters what it shows again and redraws everything.
    pub fn invalidate_tiles(&mut self) {
        self.grids.clear();
        self.released.clear();
        self.full_damage = true;
    }

    /// Frees every tile outside the latest frame's prefetch area, and every tile of an
    /// earlier scale standing in for a zoom (for memory pressure). The next plan lists them in
    /// `released`.
    pub fn trim_memory(&mut self) {
        for (id, lt) in &mut self.grids {
            for g in lt.older.drain(..) {
                self.released.extend(g.keys().map(|k| (*id, k)));
            }
            let g = &mut lt.current;
            let wanted = &g.wanted;
            let gone: Vec<Cell> = g.valid.keys().filter(|k| !wanted.contains(k)).copied().collect();
            for c in gone {
                g.valid.remove(&c);
                self.released.push((*id, g.key(c)));
            }
        }
    }

    /// Sets the raster time a frame may spend on tiles at a new scale during a zoom, while
    /// tiles of the old scale stand in (default
    /// [`DEFAULT_ZOOM_RASTER_TIME`](crate::DEFAULT_ZOOM_RASTER_TIME)).
    pub fn set_zoom_raster_time(&mut self, per_frame: Duration) {
        self.budget.per_frame = per_frame;
    }

    /// Tells the compositor what rastering cost: `pixels` (the plan's requests) took `took`.
    /// Whoever runs the plans reports every frame that rastered, so the zoom budget fits
    /// this machine.
    pub fn record_raster(&mut self, pixels: u64, took: Duration) {
        self.budget.record(pixels, took);
    }

    /// The zoom raster budget and the cost it has measured.
    pub fn raster_budget(&self) -> RasterBudget {
        self.budget
    }

    /// Plans the frame for presentation time `now`: steps scroll physics, samples
    /// animations, lists the tiles to raster and the composite, and computes damage.
    pub fn frame(&mut self, now: Duration) -> FramePlan {
        let _composite = remedy_ui_trace::phase!("composite").entered();
        let dt = self.last_frame.map_or(0.0, |l| now.saturating_sub(l).as_secs_f32());
        self.last_frame = Some(now);
        let mut animating = false;
        for s in self.scrollers.values_mut() {
            animating |= s.x.step(dt);
            animating |= s.y.step(dt);
        }
        // Followers take their leaders' offsets now, so both move in this frame.
        for (id, f) in self.follows.clone() {
            let Some((lx, ly)) = self.scrollers.get(&f.leader).map(|l| (l.x.offset, l.y.offset)) else {
                continue;
            };
            if let Some(s) = self.scrollers.get_mut(&id) {
                if f.x {
                    s.x.offset = lx.clamp(0.0, s.x.max);
                    s.x.velocity = 0.0;
                }
                if f.y {
                    s.y.offset = ly.clamp(0.0, s.y.max);
                    s.y.velocity = 0.0;
                }
            }
        }
        animating |= self.overrides.values().any(|o| !o.settled(now));
        if self.ambient.is_some() {
            if let Some(last) = self.ambient_last {
                self.ambient_t += now.saturating_sub(last);
            }
            self.ambient_last = Some(now);
        } else {
            self.ambient_last = None;
        }
        // Under reduced motion loops rest at their start.
        let loop_t = if self.reduced_motion { Duration::ZERO } else { self.ambient_t };

        let viewport = Rect::new(0.0, 0.0, self.width as f32, self.height as f32);
        let mut plan = FramePlan { animating, ..FramePlan::default() };
        let mut walk = Walk {
            now,
            viewport,
            plan: &mut plan,
            drawn: HashMap::new(),
            hits: Vec::new(),
            order: 0,
            scrolling: false,
            allowance: self.budget.pixels(),
            spent: 0,
            replaced: Damage::new(),
            deferred: 0,
            loop_t,
            looping: false,
            backdrops: Vec::new(),
            grow: 0.0,
            regions: Vec::new(),
        };
        let base = Affine::scale(self.scale, self.scale);
        let roots = self.tree.roots.clone();
        for root in &roots {
            self.walk(root, base, self.scale, 1.0, viewport, &mut walk);
        }
        let Walk { drawn, hits, replaced, deferred, looping, backdrops, .. } = walk;
        plan.animating |= plan.pending > 0 || deferred > 0;
        plan.wake_in = if looping && !self.reduced_motion { self.ambient } else { None };

        let mut damage = Damage::new();
        if self.full_damage {
            damage.add(viewport);
            self.full_damage = false;
        } else {
            for (id, new) in &drawn {
                match self.drawn.get(id) {
                    Some(old) if old == new => {}
                    Some(old) => {
                        old.bounds.into_iter().chain(new.bounds).for_each(|b| damage.add(b));
                    }
                    None => new.bounds.into_iter().for_each(|b| damage.add(b)),
                }
            }
            for (id, old) in &self.drawn {
                if !drawn.contains_key(id) {
                    old.bounds.into_iter().for_each(|b| damage.add(b));
                }
            }
        }
        damage.merge(&replaced);
        // Glass shows what is under it, blurred: a change within its reach changes the glass.
        // Glass over glass passes it on, so repeat until nothing more is added.
        let mut added = vec![false; backdrops.len()];
        loop {
            let mut grew = false;
            for ((glass, reach), done) in backdrops.iter().zip(added.iter_mut()) {
                let reads = Rect::new(
                    glass.min.x - reach,
                    glass.min.y - reach,
                    glass.max.x + reach,
                    glass.max.y + reach,
                );
                if !*done && damage.rects().iter().any(|d| d.intersect(reads).is_some_and(|r| !r.is_empty()))
                {
                    // The glass and all it reads: a rectangle drawn again holds the blur's
                    // whole source, so the glass comes out as in a full frame.
                    damage.add(reads.intersect(viewport).unwrap_or(*glass));
                    *done = true;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        damage.clip_to(viewport);
        plan.damage = damage;
        plan.released = std::mem::take(&mut self.released);
        plan.raster.sort_by_key(|r| !r.visible);
        self.drawn = drawn;
        self.scroll_hits = hits;
        plan
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        layer: &Layer,
        parent: Affine,
        parent_static_scale: f32,
        parent_opacity: f32,
        parent_clip: Rect,
        w: &mut Walk<'_>,
    ) {
        let (o_opacity, o_offset, o_scale) = match self.overrides.get(&layer.id) {
            Some(o) => (o.opacity.sample(w.now), o.offset.sample(w.now), o.scale.sample(w.now)),
            None => (1.0, Vec2::ZERO, 1.0),
        };
        let (l_affine, l_opacity) = if layer.ambient.is_empty() {
            (Affine::IDENTITY, 1.0)
        } else {
            crate::AmbientLoop::sample(&layer.ambient, w.loop_t)
        };
        let opacity = (layer.opacity * o_opacity * l_opacity).clamp(0.0, 1.0);
        let total_opacity = parent_opacity * opacity;
        if total_opacity <= 0.0 {
            // A pulse fading through zero must still come back.
            w.looping |= l_opacity <= 0.0 && parent_opacity > 0.0;
            return;
        }
        let c = layer.local_box().center();
        let scale_about_center = Affine::translate(Vec2::new(-c.x, -c.y))
            .then(Affine::scale(o_scale, o_scale))
            .then(Affine::translate(c.to_vec2()));
        let local_to_device = layer
            .transform
            .then(l_affine)
            .then(scale_about_center)
            .then(Affine::translate(Vec2::new(
                layer.bounds.min.x + o_offset.x,
                layer.bounds.min.y + o_offset.y,
            )))
            .then(parent);
        let scroll = self.scroll_offset(layer.id).unwrap_or(Vec2::ZERO);
        let content_to_device = Affine::translate(Vec2::new(-scroll.x, -scroll.y)).then(local_to_device);
        // A drifting loop moves by fractions of a pixel: snapping it would step.
        let content_to_device =
            if layer.ambient.is_empty() { snap(content_to_device) } else { content_to_device };
        let static_scale = parent_static_scale * layer.transform.scale_factor();
        let raster_scale = quantize_scale(static_scale);

        let clip = match layer.clip {
            Some(_) => match local_to_device.apply_rect(layer.local_box()).intersect(parent_clip) {
                Some(r) if !r.is_empty() => r,
                _ => return,
            },
            None => parent_clip,
        };
        let order = w.order;
        w.order += 1;

        if layer.scroll.is_some() {
            w.hits.push((layer.id, clip, static_scale.max(f32::MIN_POSITIVE)));
        }
        if let Some(radii) = layer.clip {
            w.plan.ops.push(CompositeOp::PushClip {
                transform: local_to_device,
                rect: layer.local_box(),
                radii,
            });
        }
        // The glass goes before the layer's own group: its root is the group around the layer,
        // and the layer's opacity fades it.
        let mut glass = None;
        if let Some(b) = layer.backdrop
            && let Some(device) = local_to_device.apply_rect(layer.local_box()).intersect(clip)
            && !device.is_empty()
        {
            w.plan.ops.push(CompositeOp::Backdrop {
                transform: local_to_device,
                rect: layer.local_box(),
                radii: b.radii,
                blur: b.blur,
                opacity,
            });
            w.backdrops.push((device, CompositeOp::backdrop_reach(&local_to_device, b.blur)));
            glass = Some(device);
        }
        let grouped = opacity < 1.0;
        if grouped {
            w.plan.ops.push(CompositeOp::PushOpacity(opacity));
        }
        // Filters take the layer and its children as one picture. Their region is known once
        // the children are walked; it is filled in then.
        let filter_at = w.plan.ops.len();
        let mut reach = 0.0;
        for f in &layer.filters {
            reach += CompositeOp::filter_reach(&local_to_device, f);
            w.plan.ops.push(CompositeOp::PushFilter {
                transform: local_to_device,
                filter: *f,
                region: Rect::ZERO,
            });
        }
        let filtered = !layer.filters.is_empty();
        if filtered {
            w.grow += reach;
            w.regions.push(None);
        }

        let mut bounds = None;
        if let Some(extent) = layer.content_extent() {
            let screen = clip.intersect(w.viewport);
            let inverse = content_to_device.inverse();
            let visible = screen
                .zip(inverse)
                .and_then(|(s, inv)| inv.apply_rect(s).intersect(extent))
                .filter(|r| !r.is_empty());
            bounds = content_to_device.apply_rect(extent).intersect(clip).filter(|r| !r.is_empty());
            w.looping |= !layer.ambient.is_empty() && visible.is_some();
            if let Some(visible) = visible {
                self.tiles(layer, extent, visible, content_to_device, raster_scale, clip, w);
            }
        }
        // The glass is part of what the layer draws: it appearing, moving or going is damage.
        if let Some(g) = glass {
            bounds = Some(bounds.map_or(g, |b: Rect| b.union(g)));
        }
        if let (Some(b), Some(top)) = (bounds, w.regions.last_mut()) {
            *top = Some(top.map_or(b, |t| t.union(b)));
        }
        // Inside filter groups, what the layer draws reaches as far as their filters carry it.
        if w.grow > 0.0 {
            let g = w.grow;
            bounds = bounds.map(|b| Rect::new(b.min.x - g, b.min.y - g, b.max.x + g, b.max.y + g));
        }
        w.drawn.insert(
            layer.id,
            Drawn {
                transform: content_to_device.0,
                opacity: total_opacity,
                clip: layer.clip.map(|_| clip),
                version: layer.content_version,
                order,
                raster_scale,
                bounds,
                backdrop: layer.backdrop,
                filters: layer.filters.clone(),
            },
        );

        let was_scrolling = w.scrolling;
        w.scrolling |= layer.scroll.is_some();
        for child in &layer.children {
            self.walk(child, content_to_device, static_scale, total_opacity, clip, w);
        }
        w.scrolling = was_scrolling;
        if filtered {
            w.grow -= reach;
            let drawn = w.regions.pop().flatten();
            let region = drawn
                .map(|d| Rect::new(d.min.x - reach, d.min.y - reach, d.max.x + reach, d.max.y + reach))
                .unwrap_or(Rect::ZERO);
            for op in w.plan.ops.iter_mut().skip(filter_at).take(layer.filters.len()) {
                if let CompositeOp::PushFilter { region: r, .. } = op {
                    *r = region;
                }
            }
            for _ in &layer.filters {
                w.plan.ops.push(CompositeOp::PopFilter);
            }
            if !region.is_empty() {
                w.backdrops.push((region, 0.0));
                if let Some(top) = w.regions.last_mut() {
                    *top = Some(top.map_or(region, |t| t.union(region)));
                }
            }
        }
        if grouped {
            w.plan.ops.push(CompositeOp::PopOpacity);
        }
        if layer.clip.is_some() {
            w.plan.ops.push(CompositeOp::PopClip);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tiles(
        &mut self,
        layer: &Layer,
        extent: Rect,
        visible: Rect,
        to_device: Affine,
        raster_scale: f32,
        clip: Rect,
        w: &mut Walk<'_>,
    ) {
        let lt = self.grids.entry(layer.id).or_insert_with(|| LayerTiles::new(raster_scale));
        lt.rescale(raster_scale);
        let LayerTiles { current: grid, older } = lt;
        // Inside a scroll container, prefetch one tile beyond what is visible in every
        // direction; content under a fixed clip only shows what is visible now.
        let e = grid.content_edge();
        let around = Rect::new(visible.min.x - e, visible.min.y - e, visible.max.x + e, visible.max.y + e);
        let prefetch = if w.scrolling || layer.scroll.is_some() {
            around.intersect(extent).unwrap_or(visible)
        } else {
            visible
        };
        let px = tile_pixels(raster_scale);
        let shown: std::collections::HashSet<Cell> = grid.cells_in(visible).into_iter().collect();
        grid.wanted = grid.cells_in(prefetch).into_iter().collect();
        let mut cells: Vec<Cell> = grid.wanted.iter().copied().collect();
        // While older scales stand in, the budget goes to what is on screen, nearest the
        // middle of it first; otherwise in grid order.
        let zooming = !older.is_empty();
        if zooming {
            let mid = visible.center();
            let far = |c: &Cell| {
                let r = grid.tile_rect(*c, extent);
                let (dx, dy) = (r.center().x - mid.x, r.center().y - mid.y);
                (!shown.contains(c), (dx * dx + dy * dy).to_bits())
            };
            cells.sort_by_key(far);
        } else {
            cells.sort_by_key(|&(c, r)| (r, c));
        }
        // Older grids nearest the new scale stand in best.
        let target = raster_scale.max(f32::MIN_POSITIVE).log2();
        older.sort_by(|a, b| {
            let da = (a.raster_scale.log2() - target).abs();
            let db = (b.raster_scale.log2() - target).abs();
            da.total_cmp(&db)
        });
        let mut used = vec![false; older.len()];
        for cell in cells {
            let rect = grid.tile_rect(cell, extent);
            if rect.is_empty() {
                continue;
            }
            let is_visible = shown.contains(&cell);
            let ready = grid.valid.get(&cell) == Some(&layer.content_version);
            if !ready {
                let width = ((rect.width() * raster_scale).ceil() as u32).clamp(1, px);
                let height = ((rect.height() * raster_scale).ceil() as u32).clamp(1, px);
                let cost = u64::from(width) * u64::from(height);
                let stand_in = if zooming && is_visible {
                    older.iter().enumerate().find_map(|(i, g)| g.covering(rect, extent).map(|c| (i, c)))
                } else {
                    None
                };
                let affordable = w.spent == 0 || w.spent.saturating_add(cost) <= w.allowance;
                let raster = match (zooming, is_visible) {
                    // Nothing changes scale: raster as content asks.
                    (false, _) => true,
                    // On screen with nothing to stand in: never held back.
                    (true, true) if stand_in.is_none() => true,
                    (true, true) => affordable,
                    // Prefetch at a new scale waits until what shows is done.
                    (true, false) => w.plan.pending == 0 && affordable,
                };
                if raster {
                    if zooming {
                        w.spent = w.spent.saturating_add(cost);
                        if is_visible {
                            let device = to_device.apply_rect(rect).intersect(clip);
                            device.into_iter().for_each(|d| w.replaced.add(d));
                        }
                    }
                    w.plan.raster.push(TileRequest {
                        layer: layer.id,
                        key: grid.key(cell),
                        rect,
                        raster_scale,
                        width,
                        height,
                        content: layer.content.clone(),
                        visible: is_visible,
                    });
                    grid.valid.insert(cell, layer.content_version);
                } else if let Some((i, covering)) = stand_in {
                    if let Some(u) = used.get_mut(i) {
                        *u = true;
                    }
                    w.plan.pending += 1;
                    if let Some(g) = older.get(i) {
                        for c in covering {
                            let tile = g.tile_rect(c, extent);
                            let Some(area) = tile.intersect(rect).filter(|a| !a.is_empty()) else { continue };
                            w.plan.ops.push(CompositeOp::Tile {
                                layer: layer.id,
                                key: g.key(c),
                                transform: to_device,
                                rect: tile,
                                raster_scale: g.raster_scale,
                                area,
                            });
                        }
                    }
                    continue;
                } else {
                    w.deferred += 1;
                    continue;
                }
            }
            if is_visible {
                w.plan.ops.push(CompositeOp::Tile {
                    layer: layer.id,
                    key: grid.key(cell),
                    transform: to_device,
                    rect,
                    raster_scale,
                    area: rect,
                });
            }
        }
        // Grids that stood in for nothing this frame have done their job.
        let mut i = 0;
        let released = &mut self.released;
        older.retain(|g| {
            let keep = used.get(i).copied().unwrap_or(false);
            i += 1;
            if !keep {
                released.extend(g.keys().map(|k| (layer.id, k)));
            }
            keep
        });
    }
}

/// The logical size of a window in device pixels at `scale`.
pub fn logical_size(width: u32, height: u32, scale: f32) -> Size {
    let s = if scale > 0.0 { scale } else { 1.0 };
    Size::new(width as f32 / s, height as f32 / s)
}
