//! Progressive zoom: tiles of the old scale stand in while the new scale rasters within a
//! per-frame budget.

use std::time::Duration;

use remedy_ui_compositor::*;
use remedy_ui_core::{Color, Point, Rect};
use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_paint::{Affine, Brush, DisplayList};

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// A 1000 × 1000 layer under an 800 × 600 window, zoomed by its transform.
fn zoomed(scale: f32) -> LayerTree {
    let mut list = DisplayList::new();
    list.fill_rect(Rect::new(0.0, 0.0, 1000.0, 1000.0), Brush::Solid(Color::BLACK));
    let mut l = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 1000.0, 1000.0), list);
    l.transform = Affine::scale(scale, scale);
    LayerTree::new(vec![l])
}

fn shown(plan: &FramePlan) -> Vec<(TileKey, Rect)> {
    plan.ops
        .iter()
        .filter_map(|o| match o {
            CompositeOp::Tile { key, area, .. } => Some((*key, *area)),
            _ => None,
        })
        .collect()
}

fn stand_ins(plan: &FramePlan, step: u32) -> Vec<Rect> {
    shown(plan).into_iter().filter(|(k, _)| k.scale == step).map(|(_, a)| a).collect()
}

#[test]
fn zooming_shows_old_tiles_scaled_while_new_ones_raster_within_the_budget() {
    let mut c = Compositor::new(800, 600, 1.0, ScrollPhysics::CLAMPED);
    c.commit(zoomed(1.0));
    let p = c.frame(ms(0));
    assert_eq!(p.raster.len(), 12, "4 × 3 tiles of 256 at 1x");
    assert_eq!(scale_step(1.0), 64);
    // The budget holds one 512-pixel tile a frame.
    c.set_zoom_raster_time(Duration::from_nanos(512 * 512 * 20));
    c.commit(zoomed(2.0));
    let p = c.frame(ms(16));
    assert_eq!(p.raster.len(), 1, "one new tile this frame: {:?}", p.raster);
    assert!(p.raster.iter().all(|r| r.raster_scale == 2.0 && r.key.scale == 128 && r.visible));
    assert_eq!(p.pending, 3, "the other three visible tiles wait");
    assert!(p.animating, "frames keep coming until the zoom is rastered");
    let old = stand_ins(&p, 64);
    assert!(!old.is_empty(), "tiles of the old scale stand in");
    assert!(p.released.is_empty(), "the old tiles stay while they stand in");
    // Stand-ins draw only where a new tile is missing: never over the one rastered.
    let done = p.raster.first().map(|r| r.rect).unwrap_or(Rect::ZERO);
    assert!(old.iter().all(|a| a.intersect(done).is_none_or(|i| i.is_empty())));
    let first_tile = p.raster.first().map(|r| r.rect.min);
    assert_eq!(first_tile, Some(Point::new(0.0, 0.0)), "the tile nearest the middle goes first");
    let mut t = 32;
    let mut rastered = 1;
    loop {
        let p = c.frame(ms(t));
        rastered += p.raster.iter().filter(|r| r.visible).count();
        assert!(p.raster.len() <= 1, "never more than the budget");
        assert!(!p.damage.is_empty() || p.raster.is_empty(), "a replaced stand-in is damage");
        if p.pending == 0 {
            assert_eq!(p.released.len(), 12, "the old scale is freed once nothing needs it");
            break;
        }
        t += 16;
        assert!(t < 2000);
    }
    assert_eq!(rastered, 4, "the visible 2 × 2 tiles of 512 at 2x");
    let p = c.frame(ms(t + 16));
    assert!(!p.animating && p.raster.is_empty());
}

#[test]
fn zooming_back_mid_way_reuses_the_tiles_still_held() {
    let mut c = Compositor::new(800, 600, 1.0, ScrollPhysics::CLAMPED);
    c.commit(zoomed(1.0));
    let _ = c.frame(ms(0));
    c.set_zoom_raster_time(Duration::ZERO);
    c.commit(zoomed(2.0));
    let p = c.frame(ms(16));
    assert_eq!(p.pending, 3);
    c.commit(zoomed(1.0));
    let p = c.frame(ms(32));
    assert!(p.raster.is_empty(), "the 1x tiles were kept: {:?}", p.raster);
    assert_eq!(p.pending, 0);
    assert_eq!(p.released.len(), 1, "the one 2x tile is freed");
}

#[test]
fn content_with_nothing_to_stand_in_is_never_held_back() {
    let mut c = Compositor::new(800, 600, 1.0, ScrollPhysics::CLAMPED);
    c.commit(zoomed(4.0));
    let _ = c.frame(ms(0));
    c.set_zoom_raster_time(Duration::ZERO);
    // Zooming out shows content the 4x grid never rastered.
    c.commit(zoomed(1.0));
    let p = c.frame(ms(16));
    assert_eq!(p.raster.len() + p.pending, 12);
    assert!(p.raster.len() >= 11, "only the corner the 4x tiles cover may wait: {}", p.raster.len());
    assert!(shown(&p).len() >= 12);
}

#[test]
fn measured_raster_cost_sizes_the_zoom_budget_and_trim_frees_stand_ins() {
    let mut c = Compositor::new(800, 600, 1.0, ScrollPhysics::CLAMPED);
    assert!(!c.raster_budget().measured);
    c.record_raster(1_000_000, Duration::from_millis(10));
    let b = c.raster_budget();
    assert!(b.measured && (b.nanos_per_pixel - 10.0).abs() < 1e-6);
    assert_eq!(b.pixels(), 400_000, "4 ms at 10 ns a pixel");
    c.record_raster(1_000_000, Duration::from_millis(30));
    assert!(c.raster_budget().nanos_per_pixel > 10.0, "slower frames shrink the budget");
    c.record_raster(0, Duration::from_millis(30));
    c.commit(zoomed(1.0));
    let _ = c.frame(ms(0));
    c.set_zoom_raster_time(Duration::ZERO);
    c.commit(zoomed(2.0));
    let p = c.frame(ms(16));
    assert_eq!(p.pending, 3);
    c.trim_memory();
    let p = c.frame(ms(32));
    assert!(p.released.iter().any(|(_, k)| k.scale == 64), "memory pressure drops the stand-ins");
    assert_eq!(p.pending, 0, "with nothing to stand in, what shows rasters now");
}
