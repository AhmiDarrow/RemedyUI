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
use crate::tiles::{TileGrid, TileKey, quantize_scale, tile_pixels};

/// One step of drawing a frame from rastered tiles.
#[derive(Clone, Debug, PartialEq)]
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
    /// Draws a rastered tile: its pixels cover `rect` in the space `transform` maps to device
    /// pixels.
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
    },
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
    /// True while scrolling or animations still move: request another frame.
    pub animating: bool,
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drawn {
    transform: [f32; 6],
    opacity: f32,
    clip: Option<Rect>,
    version: u64,
    order: usize,
    raster_scale: f32,
    bounds: Option<Rect>,
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
    overrides: HashMap<LayerId, Overrides>,
    grids: HashMap<LayerId, TileGrid>,
    drawn: HashMap<LayerId, Drawn>,
    scroll_hits: Vec<(LayerId, Rect, f32)>,
    released: Vec<(LayerId, TileKey)>,
    full_damage: bool,
    last_frame: Option<Duration>,
    reduced_motion: bool,
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
            overrides: HashMap::new(),
            grids: HashMap::new(),
            drawn: HashMap::new(),
            scroll_hits: Vec::new(),
            released: Vec::new(),
            full_damage: true,
            last_frame: None,
            reduced_motion: false,
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
        tree.for_each(|l| {
            alive.insert(l.id, (l.scroll.is_some(), l.max_scroll()));
        });
        self.scrollers.retain(|id, _| alive.get(id).is_some_and(|(s, _)| *s));
        for (&id, &(scrolls, max)) in &alive {
            if !scrolls {
                continue;
            }
            let physics = self.physics;
            let s = self.scrollers.entry(id).or_insert(Scroller {
                x: ScrollAxis::new(max.x, physics),
                y: ScrollAxis::new(max.y, physics),
            });
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
                released.extend(g.valid.keys().map(|k| (*id, *k)));
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

    /// Forgets every rastered tile without releasing it (the renderer lost them, as with a
    /// lost GPU device): the next frame rasters what it shows again and redraws everything.
    pub fn invalidate_tiles(&mut self) {
        self.grids.clear();
        self.released.clear();
        self.full_damage = true;
    }

    /// Frees every tile outside the latest frame's prefetch area (for memory pressure). The
    /// next plan lists them in `released`.
    pub fn trim_memory(&mut self) {
        for (id, g) in &mut self.grids {
            let wanted = &g.wanted;
            let gone: Vec<TileKey> = g.valid.keys().filter(|k| !wanted.contains(k)).copied().collect();
            for k in gone {
                g.valid.remove(&k);
                self.released.push((*id, k));
            }
        }
    }

    /// Plans the frame for presentation time `now`: steps scroll physics, samples
    /// animations, lists the tiles to raster and the composite, and computes damage.
    pub fn frame(&mut self, now: Duration) -> FramePlan {
        let dt = self.last_frame.map_or(0.0, |l| now.saturating_sub(l).as_secs_f32());
        self.last_frame = Some(now);
        let mut animating = false;
        for s in self.scrollers.values_mut() {
            animating |= s.x.step(dt);
            animating |= s.y.step(dt);
        }
        animating |= self.overrides.values().any(|o| !o.settled(now));

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
        };
        let base = Affine::scale(self.scale, self.scale);
        let roots = self.tree.roots.clone();
        for root in &roots {
            self.walk(root, base, self.scale, 1.0, viewport, &mut walk);
        }
        let Walk { drawn, hits, .. } = walk;

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
        let opacity = (layer.opacity * o_opacity).clamp(0.0, 1.0);
        let total_opacity = parent_opacity * opacity;
        if total_opacity <= 0.0 {
            return;
        }
        let c = layer.local_box().center();
        let scale_about_center = Affine::translate(Vec2::new(-c.x, -c.y))
            .then(Affine::scale(o_scale, o_scale))
            .then(Affine::translate(c.to_vec2()));
        let local_to_device = layer
            .transform
            .then(scale_about_center)
            .then(Affine::translate(Vec2::new(
                layer.bounds.min.x + o_offset.x,
                layer.bounds.min.y + o_offset.y,
            )))
            .then(parent);
        let scroll = self.scroll_offset(layer.id).unwrap_or(Vec2::ZERO);
        let content_to_device =
            snap(Affine::translate(Vec2::new(-scroll.x, -scroll.y)).then(local_to_device));
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
        let grouped = opacity < 1.0;
        if grouped {
            w.plan.ops.push(CompositeOp::PushOpacity(opacity));
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
            if let Some(visible) = visible {
                self.tiles(layer, extent, visible, content_to_device, raster_scale, w);
            }
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
            },
        );

        let was_scrolling = w.scrolling;
        w.scrolling |= layer.scroll.is_some();
        for child in &layer.children {
            self.walk(child, content_to_device, static_scale, total_opacity, clip, w);
        }
        w.scrolling = was_scrolling;
        if grouped {
            w.plan.ops.push(CompositeOp::PopOpacity);
        }
        if layer.clip.is_some() {
            w.plan.ops.push(CompositeOp::PopClip);
        }
    }

    fn tiles(
        &mut self,
        layer: &Layer,
        extent: Rect,
        visible: Rect,
        to_device: Affine,
        raster_scale: f32,
        w: &mut Walk<'_>,
    ) {
        let grid = self.grids.entry(layer.id).or_insert_with(|| TileGrid::new(raster_scale));
        if grid.raster_scale != raster_scale {
            self.released.extend(grid.valid.keys().map(|k| (layer.id, *k)));
            *grid = TileGrid::new(raster_scale);
        }
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
        let shown: std::collections::HashSet<TileKey> = grid.keys_in(visible).into_iter().collect();
        grid.wanted = grid.keys_in(prefetch).into_iter().collect();
        let mut keys: Vec<TileKey> = grid.wanted.iter().copied().collect();
        keys.sort();
        for key in keys {
            let rect = grid.tile_rect(key, extent);
            if rect.is_empty() {
                continue;
            }
            let is_visible = shown.contains(&key);
            if grid.valid.get(&key) != Some(&layer.content_version) {
                let width = ((rect.width() * raster_scale).ceil() as u32).clamp(1, px);
                let height = ((rect.height() * raster_scale).ceil() as u32).clamp(1, px);
                w.plan.raster.push(TileRequest {
                    layer: layer.id,
                    key,
                    rect,
                    raster_scale,
                    width,
                    height,
                    content: layer.content.clone(),
                    visible: is_visible,
                });
                grid.valid.insert(key, layer.content_version);
            }
            if is_visible {
                w.plan.ops.push(CompositeOp::Tile {
                    layer: layer.id,
                    key,
                    transform: to_device,
                    rect,
                    raster_scale,
                });
            }
        }
    }
}

/// The logical size of a window in device pixels at `scale`.
pub fn logical_size(width: u32, height: u32, scale: f32) -> Size {
    let s = if scale > 0.0 { scale } else { 1.0 };
    Size::new(width as f32 / s, height as f32 / s)
}
