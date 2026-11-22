//! Tiles: how a layer's content is cut into rasterizable squares, and the grids of earlier
//! raster scales kept to stand in while a zoom re-rasters.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use remedy_ui_core::{Point, Rect};

/// A tile's column and row in its layer's grid, and the raster scale of that grid.
///
/// Tiles of one layer at two scales are different tiles: while a zoom re-rasters at the new
/// scale, tiles of the old scale stay alive and stand in for the ones not rastered yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    /// Column (may be negative for content left of the origin, such as a shadow).
    pub col: i32,
    /// Row.
    pub row: i32,
    /// The grid's raster scale in steps of 1/64 ([`scale_step`]).
    pub scale: u32,
}

/// The tile edge in device pixels for a raster scale: 256, or 512 on high-DPI screens.
pub fn tile_pixels(raster_scale: f32) -> u32 {
    if raster_scale >= 2.0 { 512 } else { 256 }
}

/// Raster scales are rounded to 1/64 so float noise does not re-raster tiles.
pub fn quantize_scale(s: f32) -> f32 {
    if s.is_finite() && s > 0.0 { ((s * 64.0).round() / 64.0).max(1.0 / 64.0) } else { 1.0 }
}

/// A quantized raster scale as a whole number of 1/64 steps (the `scale` of a [`TileKey`]).
pub fn scale_step(raster_scale: f32) -> u32 {
    let s = quantize_scale(raster_scale) * 64.0;
    // Clamp before converting: scales past u32 have no tiles that fit in memory anyway.
    s.round().clamp(1.0, u32::MAX as f32) as u32
}

fn to_index(v: f32) -> i32 {
    // Clamp before converting: content far outside the i32 range has no visible tiles anyway.
    v.clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

/// A cell of a grid: column and row.
pub(crate) type Cell = (i32, i32);

/// One layer's tile grid at one raster scale.
#[derive(Clone, Debug)]
pub(crate) struct TileGrid {
    pub(crate) raster_scale: f32,
    pub(crate) step: u32,
    /// The content version each rastered tile holds.
    pub(crate) valid: HashMap<Cell, u64>,
    /// Tiles in the latest frame's prefetch area.
    pub(crate) wanted: HashSet<Cell>,
}

impl TileGrid {
    pub(crate) fn new(raster_scale: f32) -> Self {
        TileGrid {
            raster_scale,
            step: scale_step(raster_scale),
            valid: HashMap::new(),
            wanted: HashSet::new(),
        }
    }

    /// The full key of a cell in this grid.
    pub(crate) fn key(&self, (col, row): Cell) -> TileKey {
        TileKey { col, row, scale: self.step }
    }

    /// Every rastered tile's key.
    pub(crate) fn keys(&self) -> impl Iterator<Item = TileKey> + '_ {
        self.valid.keys().map(|c| self.key(*c))
    }

    /// The tile edge in content units.
    pub(crate) fn content_edge(&self) -> f32 {
        tile_pixels(self.raster_scale) as f32 / self.raster_scale
    }

    /// Every cell touching `area` (content space).
    pub(crate) fn cells_in(&self, area: Rect) -> Vec<Cell> {
        let e = self.content_edge();
        let (c0, c1) = (to_index((area.min.x / e).floor()), to_index((area.max.x / e).ceil()));
        let (r0, r1) = (to_index((area.min.y / e).floor()), to_index((area.max.y / e).ceil()));
        let mut out = Vec::new();
        for row in r0..r1 {
            for col in c0..c1 {
                out.push((col, row));
            }
        }
        out
    }

    /// Carries tiles rastered at `from` over to `to` where their cell does not touch
    /// `changed` (content space): what they hold is drawn the same at `to`.
    pub(crate) fn carry_over(&mut self, from: u64, to: u64, changed: Rect) {
        let e = self.content_edge();
        // A pixel more, for anti-aliasing at the change's edge.
        let near = Rect::new(changed.min.x - 1.0, changed.min.y - 1.0, changed.max.x + 1.0, changed.max.y + 1.0);
        for (&(col, row), version) in &mut self.valid {
            let cell = Rect::new(col as f32 * e, row as f32 * e, (col + 1) as f32 * e, (row + 1) as f32 * e);
            if *version == from && (changed.is_empty() || cell.intersect(near).is_none_or(|r| r.is_empty())) {
                *version = to;
            }
        }
    }

    /// The content area of a cell, its far edges clamped to `extent` (snapped out to device
    /// pixels) so small layers get small tiles.
    pub(crate) fn tile_rect(&self, (col, row): Cell, extent: Rect) -> Rect {
        let e = self.content_edge();
        let min = Point::new(col as f32 * e, row as f32 * e);
        let s = self.raster_scale;
        let snap = |v: f32| (v * s).ceil() / s;
        Rect::new(min.x, min.y, (min.x + e).min(snap(extent.max.x)), (min.y + e).min(snap(extent.max.y)))
    }

    /// The rastered cells that together cover all of `area` (content space, inside
    /// `extent`), or `None` when some part of it has no tile at this scale.
    pub(crate) fn covering(&self, area: Rect, extent: Rect) -> Option<Vec<Cell>> {
        // A finer grid needs more tiles than it holds to cover a large area (zooming far out):
        // count before listing them.
        let e = self.content_edge();
        let span = |a: f32, b: f32| ((b / e).ceil() - (a / e).floor()).max(0.0);
        let needed = span(area.min.x, area.max.x) * span(area.min.y, area.max.y);
        if !needed.is_finite() || needed > self.valid.len() as f32 {
            return None;
        }
        let cells: Vec<Cell> = self
            .cells_in(area)
            .into_iter()
            .filter(|c| self.tile_rect(*c, extent).intersect(area).is_some_and(|r| !r.is_empty()))
            .collect();
        (!cells.is_empty() && cells.iter().all(|c| self.valid.contains_key(c))).then_some(cells)
    }
}

/// A layer's grids: the one at its current raster scale, and grids of earlier scales whose
/// tiles stand in while the current one fills.
#[derive(Clone, Debug)]
pub(crate) struct LayerTiles {
    pub(crate) current: TileGrid,
    pub(crate) older: Vec<TileGrid>,
    /// The content last walked, its version and extent: a new version is diffed against it,
    /// and tiles away from what changed carry over.
    pub(crate) seen: Option<(u64, Rect, std::sync::Arc<remedy_ui_paint::DisplayList>)>,
}

impl LayerTiles {
    pub(crate) fn new(raster_scale: f32) -> Self {
        LayerTiles { current: TileGrid::new(raster_scale), older: Vec::new(), seen: None }
    }

    /// Moves to `raster_scale`. The grid being left is kept to stand in; a kept grid at the
    /// new scale (zooming back) becomes current again with the tiles it still has.
    pub(crate) fn rescale(&mut self, raster_scale: f32) {
        if self.current.raster_scale == raster_scale {
            return;
        }
        let next = match self.older.iter().position(|g| g.raster_scale == raster_scale) {
            Some(i) => self.older.swap_remove(i),
            None => TileGrid::new(raster_scale),
        };
        let old = std::mem::replace(&mut self.current, next);
        if !old.valid.is_empty() {
            self.older.push(old);
        }
    }

    /// Every tile held, at every scale.
    pub(crate) fn all_keys(&self) -> Vec<TileKey> {
        let mut out: Vec<TileKey> = self.current.keys().collect();
        for g in &self.older {
            out.extend(g.keys());
        }
        out
    }
}

/// How much rastering a frame may spend on tiles at a new scale while older tiles stand in.
///
/// The allowance is a time per frame; the renderer reports what rastering really costs
/// ([`Compositor::record_raster`](crate::Compositor::record_raster)), which turns that time
/// into pixels for this machine. Tiles with nothing to stand in for them are never held
/// back, and one tile always rasters per frame, so a zoom always finishes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterBudget {
    /// Raster time a frame may spend on re-scaled tiles.
    pub per_frame: Duration,
    /// Measured cost of rastering one pixel, in nanoseconds.
    pub nanos_per_pixel: f64,
    /// True once a renderer has reported a measurement.
    pub measured: bool,
}

/// Half of a 120 Hz frame (8.3 ms): the other half is left for compositing, presenting and
/// the UI thread's own work in the frame (docs/design/09-performance-and-resources.md).
pub const DEFAULT_ZOOM_RASTER_TIME: Duration = Duration::from_millis(4);

/// Before any measurement: about what vello_cpu takes per pixel of mixed UI content on a
/// mid-range core in a release build. Replaced by the first real measurement.
const INITIAL_NANOS_PER_PIXEL: f64 = 20.0;

/// How fast measurements move the estimate: recent frames count most, one odd frame little.
const COST_SMOOTHING: f64 = 0.25;

impl Default for RasterBudget {
    fn default() -> Self {
        RasterBudget {
            per_frame: DEFAULT_ZOOM_RASTER_TIME,
            nanos_per_pixel: INITIAL_NANOS_PER_PIXEL,
            measured: false,
        }
    }
}

impl RasterBudget {
    /// Pixels a frame may raster at a new scale.
    pub fn pixels(&self) -> u64 {
        let ns = self.per_frame.as_nanos() as f64;
        let px = ns / self.nanos_per_pixel.max(f64::MIN_POSITIVE);
        if px.is_finite() { px.max(0.0) as u64 } else { u64::MAX }
    }

    /// Folds in a measurement: `pixels` rastered in `took`.
    pub fn record(&mut self, pixels: u64, took: Duration) {
        if pixels == 0 {
            return;
        }
        let sample = took.as_nanos() as f64 / pixels as f64;
        if !sample.is_finite() {
            return;
        }
        self.nanos_per_pixel = if self.measured {
            self.nanos_per_pixel + (sample - self.nanos_per_pixel) * COST_SMOOTHING
        } else {
            sample
        };
        self.measured = true;
    }
}
