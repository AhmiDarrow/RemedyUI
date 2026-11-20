//! Commits, tiles, damage, compositor-side scrolling and animation.

use std::sync::Arc;
use std::time::Duration;

use remedy_ui_compositor::*;
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_motion::{Easing, Motion};
use remedy_ui_paint::{Brush, DisplayList};

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

fn solid(w: f32, h: f32) -> DisplayList {
    let mut l = DisplayList::new();
    l.fill_rect(Rect::new(0.0, 0.0, w, h), Brush::Solid(Color::BLACK));
    l
}

fn page(content_h: f32) -> LayerTree {
    let content = Layer::new(LayerId(2), Rect::new(0.0, 0.0, 400.0, content_h), solid(400.0, content_h));
    let scroller = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 400.0, 300.0), DisplayList::new())
        .scrolling(Size::new(400.0, content_h))
        .with_child(content);
    LayerTree::new(vec![scroller])
}

fn tiles(plan: &FramePlan) -> Vec<(LayerId, TileKey)> {
    plan.ops
        .iter()
        .filter_map(|o| match o {
            CompositeOp::Tile { layer, key, .. } => Some((*layer, *key)),
            _ => None,
        })
        .collect()
}

#[test]
fn mailbox_keeps_only_the_newest_commit() {
    let m = Mailbox::new();
    assert!(!m.post(1));
    assert!(m.post(2), "the untaken commit is replaced");
    assert!(m.is_pending());
    assert_eq!(m.take(), Some(2));
    assert_eq!(m.take(), None);
    let shared = Arc::new(Mailbox::new());
    let s2 = shared.clone();
    let t = std::thread::spawn(move || s2.post(LayerTree::default()));
    assert!(t.join().is_ok());
    assert!(shared.take().is_some(), "commits cross threads");
}

#[test]
fn damage_merges_overlaps_and_keeps_far_rects_apart() {
    let mut d = Damage::new();
    d.add(Rect::new(0.0, 0.0, 10.0, 10.0));
    d.add(Rect::new(5.0, 5.0, 15.0, 15.0));
    assert_eq!(d.rects(), &[Rect::new(0.0, 0.0, 15.0, 15.0)]);
    d.add(Rect::new(500.0, 500.0, 510.0, 510.0));
    assert_eq!(d.rects().len(), 2, "far apart rects stay separate");
    d.add(Rect::new(0.2, 100.5, 3.7, 101.0));
    assert!(d.rects().contains(&Rect::new(0.0, 100.0, 4.0, 101.0)), "snapped out to whole pixels");
    d.clip_to(Rect::new(0.0, 0.0, 100.0, 120.0));
    assert_eq!(d.rects().len(), 2, "the far rect is clipped away");
}

#[test]
fn first_frame_is_fully_damaged_and_an_idle_frame_has_none() {
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(page(1000.0));
    let p = c.frame(ms(0));
    assert_eq!(p.damage.rects(), &[Rect::new(0.0, 0.0, 400.0, 300.0)]);
    assert!(!p.raster.is_empty());
    assert!(!p.animating);
    let p = c.frame(ms(16));
    assert!(p.damage.is_empty(), "nothing changed");
    assert!(p.raster.is_empty(), "nothing re-rasters");
}

#[test]
fn only_visible_and_prefetch_tiles_raster() {
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(page(10_000.0));
    let p = c.frame(ms(0));
    let rows: Vec<i32> = p.raster.iter().filter(|r| r.layer == LayerId(2)).map(|r| r.key.row).collect();
    assert!(rows.contains(&0) && rows.contains(&1), "visible rows: 300px tall over 256px tiles");
    assert!(rows.contains(&2), "one prefetch row");
    assert!(!rows.contains(&3), "nothing far away");
    let first_prefetch = p.raster.iter().position(|r| !r.visible);
    let last_visible = p.raster.iter().rposition(|r| r.visible);
    assert!(first_prefetch > last_visible, "visible tiles come first");
    let shown = tiles(&p);
    assert!(shown.iter().all(|(_, k)| k.row <= 1), "only visible tiles composite");
}

#[test]
fn scrolling_moves_tiles_without_rerastering_and_damages_the_viewport() {
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(page(10_000.0));
    let _ = c.frame(ms(0));
    assert!(c.scroll_by(LayerId(1), Vec2::new(0.0, 100.0)));
    let p = c.frame(ms(16));
    assert!(p.raster.iter().all(|r| r.key.row >= 3), "rows 0-2 were rastered already: {:?}", p.raster);
    assert!(!p.damage.is_empty());
    let moved = p.ops.iter().find_map(|o| match o {
        CompositeOp::Tile { layer: LayerId(2), key: TileKey { row: 1, .. }, transform, .. } => {
            Some(transform.0[5])
        }
        _ => None,
    });
    assert_eq!(moved, Some(-100.0), "content moved up by the scroll");
    assert_eq!(c.scroll_offset(LayerId(1)), Some(Vec2::new(0.0, 100.0)));
}

#[test]
fn scroll_offsets_clamp_and_survive_commits() {
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(page(1000.0));
    c.scroll_by(LayerId(1), Vec2::new(0.0, 5000.0));
    assert_eq!(c.scroll_offset(LayerId(1)), Some(Vec2::new(0.0, 700.0)), "clamped to content minus viewport");
    c.commit(page(1000.0));
    assert_eq!(c.scroll_offset(LayerId(1)), Some(Vec2::new(0.0, 700.0)), "kept across commits");
    c.commit(page(500.0));
    assert_eq!(c.scroll_offset(LayerId(1)), Some(Vec2::new(0.0, 200.0)), "clamped when content shrinks");
    c.commit(LayerTree::default());
    assert_eq!(c.scroll_offset(LayerId(1)), None, "dropped with the layer");
}

#[test]
fn flings_animate_then_settle() {
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(page(100_000.0));
    let _ = c.frame(ms(0));
    c.fling(LayerId(1), Vec2::new(0.0, 3000.0));
    assert!(c.is_animating(ms(0)));
    let mut t = 0;
    let mut last = 0.0;
    while c.frame(ms(t)).animating {
        let y = c.scroll_offset(LayerId(1)).map_or(0.0, |v| v.y);
        assert!(y >= last, "a fling moves one way");
        last = y;
        t += 16;
        assert!(t < 20_000, "the fling must stop");
    }
    assert!(last > 500.0, "it travelled: {last}");
    assert!(!c.is_animating(ms(t)));
}

#[test]
fn wheel_routes_to_the_innermost_scroller_then_outward() {
    let inner_content = Layer::new(LayerId(4), Rect::new(0.0, 0.0, 200.0, 400.0), solid(200.0, 400.0));
    let inner = Layer::new(LayerId(3), Rect::new(0.0, 50.0, 200.0, 150.0), DisplayList::new())
        .scrolling(Size::new(200.0, 400.0))
        .with_child(inner_content);
    let content =
        Layer::new(LayerId(2), Rect::new(0.0, 0.0, 400.0, 1000.0), solid(400.0, 1000.0)).with_child(inner);
    let outer = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 400.0, 300.0), DisplayList::new())
        .scrolling(Size::new(400.0, 1000.0))
        .with_child(content);
    let mut c = Compositor::new(1000, 700, 2.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![outer]));
    let _ = c.frame(ms(0));
    let at = Point::new(100.0, 200.0); // device pixels inside the inner scroller (50..150 logical)
    assert_eq!(c.scroll_at(at, Vec2::new(0.0, 200.0)), Some(LayerId(3)));
    assert_eq!(c.scroll_offset(LayerId(3)), Some(Vec2::new(0.0, 100.0)), "device delta divided by scale");
    assert_eq!(c.scroll_at(at, Vec2::new(0.0, 1000.0)), Some(LayerId(3)));
    assert_eq!(c.scroll_offset(LayerId(3)), Some(Vec2::new(0.0, 300.0)), "the inner one reaches its end");
    assert_eq!(c.scroll_at(at, Vec2::new(0.0, 100.0)), Some(LayerId(1)), "an exhausted inner hands outward");
    assert_eq!(c.scroll_at(Point::new(900.0, 650.0), Vec2::new(0.0, 10.0)), None, "outside every scroller");
}

#[test]
fn content_changes_damage_only_their_layer() {
    let a = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 50.0, 50.0), solid(50.0, 50.0));
    let b = Layer::new(LayerId(2), Rect::new(200.0, 200.0, 250.0, 250.0), solid(50.0, 50.0));
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![a.clone(), b.clone()]));
    let _ = c.frame(ms(0));
    let mut b2 = b;
    b2.content_version = 1;
    c.commit(LayerTree::new(vec![a, b2]));
    let p = c.frame(ms(16));
    assert_eq!(p.damage.rects(), &[Rect::new(200.0, 200.0, 250.0, 250.0)]);
    assert_eq!(p.raster.len(), 1, "only the changed layer re-rasters");
    assert_eq!(p.raster.first().map(|r| r.layer), Some(LayerId(2)));
}

#[test]
fn removed_layers_damage_and_release_their_tiles() {
    let a = Layer::new(LayerId(1), Rect::new(10.0, 10.0, 60.0, 60.0), solid(50.0, 50.0));
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![a]));
    let _ = c.frame(ms(0));
    c.commit(LayerTree::default());
    let p = c.frame(ms(16));
    assert_eq!(p.damage.rects(), &[Rect::new(10.0, 10.0, 60.0, 60.0)]);
    assert_eq!(p.released, vec![(LayerId(1), TileKey { col: 0, row: 0, scale: 64 })]);
}

#[test]
fn compositor_animations_run_without_commits() {
    let a = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 100.0, 100.0), solid(100.0, 100.0));
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![a]));
    let _ = c.frame(ms(0));
    let tween = Motion::Tween { duration: ms(100), easing: Easing::Linear };
    c.animate_offset(LayerId(1), Vec2::new(100.0, 0.0), tween, ms(0));
    c.animate_opacity(LayerId(1), 0.5, tween, ms(0));
    let mid = c.frame(ms(50));
    assert!(mid.animating);
    assert!(mid.raster.is_empty(), "moving and fading re-rasters nothing");
    let x = mid.ops.iter().find_map(|o| match o {
        CompositeOp::Tile { transform, .. } => Some(transform.0[4]),
        _ => None,
    });
    assert_eq!(x, Some(50.0));
    assert!(mid.ops.iter().any(|o| matches!(o, CompositeOp::PushOpacity(v) if (*v - 0.75).abs() < 1e-4)));
    assert_eq!(mid.damage.bounds(), Some(Rect::new(0.0, 0.0, 150.0, 100.0)), "old and new positions");
    let end = c.frame(ms(200));
    assert!(!end.animating);
    assert!(!c.is_animating(ms(200)));
}

#[test]
fn reduced_motion_jumps_animations_to_their_end() {
    let a = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 100.0, 100.0), solid(100.0, 100.0));
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.set_reduced_motion(true);
    c.commit(LayerTree::new(vec![a]));
    c.animate_offset(
        LayerId(1),
        Vec2::new(100.0, 0.0),
        Motion::Tween { duration: ms(500), easing: Easing::Linear },
        ms(0),
    );
    let p = c.frame(ms(1));
    assert!(!p.animating);
}

#[test]
fn clips_wrap_children_and_hide_offscreen_layers() {
    let child = Layer::new(LayerId(2), Rect::new(0.0, 0.0, 300.0, 300.0), solid(300.0, 300.0));
    let clipper = Layer::new(LayerId(1), Rect::new(10.0, 10.0, 110.0, 110.0), DisplayList::new())
        .clipped(CornerRadii::all(8.0))
        .with_child(child);
    let away = Layer::new(LayerId(3), Rect::new(5000.0, 0.0, 5100.0, 100.0), solid(100.0, 100.0));
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![clipper, away]));
    let p = c.frame(ms(0));
    assert!(
        matches!(p.ops.first(), Some(CompositeOp::PushClip { radii, .. }) if *radii == CornerRadii::all(8.0))
    );
    assert!(matches!(p.ops.last(), Some(CompositeOp::PopClip)));
    assert!(p.raster.iter().all(|r| r.layer != LayerId(3)), "off-screen layers raster nothing");
    let child_tiles: Vec<_> = p.raster.iter().filter(|r| r.layer == LayerId(2)).collect();
    assert_eq!(child_tiles.len(), 1, "only the visible tile: a fixed clip does not prefetch");
}

#[test]
fn high_dpi_uses_larger_tiles_and_scale_changes_reraster() {
    let a = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 100.0, 100.0), solid(100.0, 100.0));
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![a]));
    let p = c.frame(ms(0));
    assert_eq!(p.raster.first().map(|r| (r.width, r.height, r.raster_scale)), Some((100, 100, 1.0)));
    c.resize(800, 600, 2.0);
    let p = c.frame(ms(16));
    assert_eq!(p.raster.first().map(|r| (r.width, r.raster_scale)), Some((200, 2.0)));
    assert_eq!(
        p.released,
        vec![(LayerId(1), TileKey { col: 0, row: 0, scale: 64 })],
        "old-scale tiles are freed"
    );
    assert_eq!(p.damage.rects(), &[Rect::new(0.0, 0.0, 800.0, 600.0)]);
    assert_eq!(tile_pixels(2.0), 512);
}

#[test]
fn trim_memory_frees_tiles_outside_the_prefetch_area() {
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(page(10_000.0));
    let _ = c.frame(ms(0));
    c.set_scroll_offset(LayerId(1), Vec2::new(0.0, 5000.0));
    let _ = c.frame(ms(16));
    c.trim_memory();
    let p = c.frame(ms(32));
    let freed: Vec<i32> = p.released.iter().map(|(_, k)| k.row).collect();
    assert!(freed.contains(&0) && freed.contains(&1), "the top rows are released: {freed:?}");
    assert!(freed.iter().all(|r| *r < 18), "rows near the new offset stay");
}

struct Recorder(std::sync::mpsc::Sender<f32>);

impl Presenter for Recorder {
    fn present(&mut self, plan: &FramePlan) {
        let y = plan.ops.iter().find_map(|o| match o {
            CompositeOp::Tile { layer: LayerId(2), transform, .. } => Some(transform.0[5]),
            _ => None,
        });
        let _ = self.0.send(y.unwrap_or(f32::NAN));
    }
}

#[test]
fn scrolling_keeps_the_display_rate_while_the_ui_thread_is_stalled() {
    let (tx, rx) = std::sync::mpsc::channel();
    let comp = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    let thread = match CompositorThread::spawn(comp, Recorder(tx)) {
        Ok(t) => t,
        Err(e) => unreachable!("{e}"),
    };
    thread.commit(page(100_000.0));
    thread.vsync(ms(0));
    assert!(thread.wait_report(Duration::from_secs(5)).is_some_and(|r| r.presented));

    // The UI thread stalls (a slow handler) until the test releases it, after every frame.
    let (release, stalled) = std::sync::mpsc::channel::<()>();
    let ui = std::thread::spawn(move || stalled.recv().is_ok());
    thread.fling(LayerId(1), Vec2::new(0.0, 4000.0));
    let mut presented = 0;
    for i in 1..=30u64 {
        thread.vsync(ms(i * 16));
        let r = thread.wait_report(Duration::from_secs(5));
        assert!(r.is_some(), "every vsync is answered");
        presented += usize::from(r.is_some_and(|r| r.presented));
    }
    assert!(!ui.is_finished(), "the UI thread was stalled the whole time");
    let _ = release.send(());
    assert_eq!(presented, 30, "a new frame on every refresh");
    let offsets: Vec<f32> = rx.try_iter().collect();
    assert!(
        offsets.windows(2).all(|w| matches!(w, [a, b] if b < a)),
        "content moves on every frame: {offsets:?}"
    );
    assert!(ui.join().is_ok_and(|released| released));
}

#[test]
fn an_unchanged_frame_presents_nothing() {
    let (tx, rx) = std::sync::mpsc::channel();
    let thread =
        match CompositorThread::spawn(Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED), Recorder(tx)) {
            Ok(t) => t,
            Err(e) => unreachable!("{e}"),
        };
    thread.commit(page(1000.0));
    thread.vsync(ms(0));
    thread.vsync(ms(16));
    thread.vsync(ms(32));
    let reports: Vec<FrameReport> =
        (0..3).filter_map(|_| thread.wait_report(Duration::from_secs(5))).collect();
    assert_eq!(reports.iter().map(|r| r.presented).collect::<Vec<_>>(), vec![true, false, false]);
    assert_eq!(rx.try_iter().count(), 1, "the presenter ran once");
    // A commit of the same unchanged tree still presents nothing.
    thread.commit(page(1000.0));
    thread.vsync(ms(48));
    assert!(thread.wait_report(Duration::from_secs(5)).is_some_and(|r| !r.presented));
    assert!(thread.stop().is_some());
}

#[test]
fn followers_move_with_their_leader_in_the_same_frame() {
    let body = |id: u64, w: f32| {
        Layer::new(LayerId(id), Rect::new(0.0, 0.0, 200.0, 300.0), DisplayList::new())
            .scrolling(Size::new(w, 3000.0))
            .with_child(Layer::new(LayerId(id + 100), Rect::new(0.0, 0.0, w, 3000.0), solid(w, 3000.0)))
    };
    let leader = body(1, 800.0);
    let mut frozen = body(2, 200.0);
    frozen.scroll_follow = Some(ScrollFollow { leader: LayerId(1), x: false, y: true });
    let mut header = Layer::new(LayerId(3), Rect::new(0.0, 0.0, 200.0, 30.0), DisplayList::new())
        .scrolling(Size::new(800.0, 30.0))
        .with_child(Layer::new(LayerId(103), Rect::new(0.0, 0.0, 800.0, 30.0), solid(800.0, 30.0)));
    header.scroll_follow = Some(ScrollFollow { leader: LayerId(1), x: true, y: false });
    let mut c = Compositor::new(600, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![leader, frozen, header]));
    let _ = c.frame(ms(0));
    assert!(c.scroll_by(LayerId(1), Vec2::new(120.0, 500.0)));
    let _ = c.frame(ms(16));
    assert_eq!(c.scroll_offset(LayerId(1)), Some(Vec2::new(120.0, 500.0)));
    assert_eq!(
        c.scroll_offset(LayerId(2)),
        Some(Vec2::new(0.0, 500.0)),
        "the frozen pane follows vertically only"
    );
    assert_eq!(
        c.scroll_offset(LayerId(3)),
        Some(Vec2::new(120.0, 0.0)),
        "the header follows horizontally only"
    );
}

fn beam_scene() -> (Compositor, Layer) {
    // A big static backdrop and a small beam layer that spins about its own center.
    let backdrop = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 400.0, 300.0), solid(400.0, 300.0));
    let mut beam = Layer::new(LayerId(2), Rect::new(300.0, 20.0, 360.0, 80.0), solid(60.0, 60.0));
    beam.ambient = vec![AmbientLoop::spin(Point::new(30.0, 30.0), ms(4000))];
    let c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    (c, backdrop.with_child(beam.clone()))
}

#[test]
fn ambient_loops_run_at_the_ambient_rate_and_damage_only_their_layer() {
    let (mut c, root) = beam_scene();
    c.commit(LayerTree::new(vec![root]));
    let first = c.frame(ms(0));
    assert_eq!(first.wake_in, Some(Duration::from_secs(1) / 30), "the default ambient rate");
    assert!(!first.animating, "not every vsync");
    c.set_ambient(Some(ms(100)));
    let p = c.frame(ms(100));
    assert_eq!(p.wake_in, Some(ms(100)));
    assert!(p.raster.is_empty(), "a turn re-rasters nothing");
    let d = p.damage.bounds().unwrap_or(Rect::ZERO);
    assert!(d.min.x >= 280.0 && d.max.x <= 380.0 && d.max.y <= 100.0, "only around the beam: {d:?}");
}

#[test]
fn paused_loops_hold_still_ask_nothing_and_resume_without_a_jump() {
    let (mut c, root) = beam_scene();
    c.commit(LayerTree::new(vec![root]));
    let _ = c.frame(ms(0));
    let _ = c.frame(ms(1000));
    c.set_ambient(None);
    let held = c.frame(ms(2000));
    assert_eq!(held.wake_in, None);
    assert!(held.damage.is_empty(), "nothing moved while paused");
    assert!(c.frame(ms(60_000)).damage.is_empty());
    c.set_ambient(Some(ms(33)));
    let back = c.frame(ms(61_000));
    assert!(back.damage.is_empty(), "resumes where it stopped");
    assert!(!c.frame(ms(61_033)).damage.is_empty());
}

#[test]
fn loops_rest_under_reduced_motion_and_offscreen_loops_ask_nothing() {
    let (mut c, root) = beam_scene();
    c.set_reduced_motion(true);
    c.commit(LayerTree::new(vec![root]));
    let _ = c.frame(ms(0));
    let p = c.frame(ms(500));
    assert_eq!(p.wake_in, None);
    assert!(p.damage.is_empty());

    let mut away = Layer::new(LayerId(3), Rect::new(5000.0, 0.0, 5100.0, 100.0), solid(100.0, 100.0));
    away.ambient = vec![AmbientLoop::pulse(0.5, 1.0, ms(1000))];
    let mut c = Compositor::new(400, 300, 1.0, ScrollPhysics::CLAMPED);
    c.commit(LayerTree::new(vec![away]));
    assert_eq!(c.frame(ms(0)).wake_in, None, "an unseen loop keeps no frames coming");
}
