//! Tiles: how a layer's content is cut into rasterizable squares.

use std::collections::{HashMap, HashSet};

use remedy_ui_core::{Point, Rect};

/// A tile's column and row in its layer's grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    /// Column (may be negative for content left of the origin, such as a shadow).
    pub col: i32,
    /// Row.
    pub row: i32,
}

/// The tile edge in device pixels for a raster scale: 256, or 512 on high-DPI screens.
pub fn tile_pixels(raster_scale: f32) -> u32 {
    if raster_scale >= 2.0 { 512 } else { 256 }
}

/// Raster scales are rounded to 1/64 so float noise does not re-raster tiles.
pub fn quantize_scale(s: f32) -> f32 {
    if s.is_finite() && s > 0.0 { ((s * 64.0).round() / 64.0).max(1.0 / 64.0) } else { 1.0 }
}

fn to_index(v: f32) -> i32 {
    // Clamp before converting: content far outside the i32 range has no visible tiles anyway.
    v.clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

/// One layer's tile grid at one raster scale.
#[derive(Clone, Debug)]
pub(crate) struct TileGrid {
    pub(crate) raster_scale: f32,
    /// The content version each rastered tile holds.
    pub(crate) valid: HashMap<TileKey, u64>,
    /// Tiles in the latest frame's prefetch area.
    pub(crate) wanted: HashSet<TileKey>,
}

impl TileGrid {
    pub(crate) fn new(raster_scale: f32) -> Self {
        TileGrid { raster_scale, valid: HashMap::new(), wanted: HashSet::new() }
    }

    /// The tile edge in content units.
    pub(crate) fn content_edge(&self) -> f32 {
        tile_pixels(self.raster_scale) as f32 / self.raster_scale
    }

    /// Every tile touching `area` (content space).
    pub(crate) fn keys_in(&self, area: Rect) -> Vec<TileKey> {
        let e = self.content_edge();
        let (c0, c1) = (to_index((area.min.x / e).floor()), to_index((area.max.x / e).ceil()));
        let (r0, r1) = (to_index((area.min.y / e).floor()), to_index((area.max.y / e).ceil()));
        let mut out = Vec::new();
        for row in r0..r1 {
            for col in c0..c1 {
                out.push(TileKey { col, row });
            }
        }
        out
    }

    /// The content area of a tile, its far edges clamped to `extent` (snapped out to device
    /// pixels) so small layers get small tiles.
    pub(crate) fn tile_rect(&self, key: TileKey, extent: Rect) -> Rect {
        let e = self.content_edge();
        let min = Point::new(key.col as f32 * e, key.row as f32 * e);
        let s = self.raster_scale;
        let snap = |v: f32| (v * s).ceil() / s;
        Rect::new(min.x, min.y, (min.x + e).min(snap(extent.max.x)), (min.y + e).min(snap(extent.max.y)))
    }
}
