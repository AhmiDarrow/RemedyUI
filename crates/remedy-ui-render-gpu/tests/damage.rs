//! The GPU compositor draws only the damage into the frame it keeps, and the result is the
//! same, pixel for pixel, as drawing the whole frame again; frosted glass over moving content
//! included.

use std::time::Duration;

use remedy_ui_compositor::{Compositor, FramePlan, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_paint::{Brush, DisplayList, Filter};
use remedy_ui_render_cpu::{CpuCompositor, Frame};
use remedy_ui_render_gpu::*;

fn gpu() -> GpuCompositor {
    match GpuContext::new(&LadderOptions::default()) {
        Ok(ctx) => GpuCompositor::new(GpuRenderer::new(ctx, None), LadderOptions::default()),
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    }
}

const W: u32 = 320;
const H: u32 = 200;

/// Stripes, a moving dot, a blurred card that moves too, and (with `glass`) a pane of frosted
/// glass the dot passes under.
fn scene(x: f32, glass: bool) -> LayerTree {
    let mut back = DisplayList::new();
    for i in 0..32 {
        let c = Color::rgb8((i * 7) as u8, 80 + (i * 3) as u8, 220 - (i * 6) as u8);
        back.fill_rect(Rect::new(i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 200.0), Brush::Solid(c));
    }
    let mut dot = DisplayList::new();
    dot.fill_rounded(Rect::new(0.0, 0.0, 18.0, 18.0), CornerRadii::all(9.0), Brush::Solid(Color::WHITE));
    let mut card = DisplayList::new();
    card.push_filter(Filter::DropShadow {
        offset: remedy_ui_core::Vec2::new(0.0, 4.0),
        blur: 8.0,
        color: Color::rgba8(0, 0, 0, 140),
    });
    card.fill_rounded(
        Rect::new(0.0, 0.0, 40.0, 24.0),
        CornerRadii::all(5.0),
        Brush::Solid(Color::rgb8(250, 200, 60)),
    );
    card.pop_filter();
    let mut root = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 320.0, 200.0), back)
        .with_child(Layer::new(LayerId(2), Rect::new(x, 60.0, x + 18.0, 78.0), dot))
        .with_child(Layer::new(LayerId(3), Rect::new(250.0 - x * 0.5, 140.0, 290.0 - x * 0.5, 164.0), card));
    if glass {
        let mut tint = DisplayList::new();
        tint.fill_rounded(
            Rect::new(0.0, 0.0, 120.0, 70.0),
            CornerRadii::all(12.0),
            Brush::Solid(Color::rgba8(255, 255, 255, 50)),
        );
        root = root.with_child(
            Layer::new(LayerId(4), Rect::new(100.0, 30.0, 220.0, 100.0), tint)
                .with_backdrop(5.0, CornerRadii::all(12.0)),
        );
    }
    LayerTree::new(vec![root])
}

fn worst(a: &Frame, b: &Frame) -> u8 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.pixels.iter().zip(&b.pixels).map(|(p, q)| p.abs_diff(*q)).max().unwrap_or(0)
}

/// Drives a compositor through the moves, giving each plan to `f`.
fn frames(glass: bool, mut f: impl FnMut(usize, &FramePlan)) {
    let mut comp = Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED);
    for (k, x) in [10.0f32, 30.5, 61.0, 96.0, 140.25, 141.0, 190.0, 260.0].into_iter().enumerate() {
        comp.commit(scene(x, glass));
        let plan = comp.frame(Duration::from_millis(16 * k as u64));
        f(k, &plan);
    }
}

#[test]
fn gpu_damage_frames_equal_full_redraws() {
    for glass in [false, true] {
        let mut partial = gpu();
        let mut full = gpu();
        frames(glass, |k, plan| {
            let a = partial.present_to_frame(plan, W, H, Color::BLACK);
            // The same plan composited whole into a new target.
            let b = full.renderer().create_target(W, H).and_then(|t| {
                full.present(plan, &t, Color::BLACK).and_then(|()| full.renderer().read_back(&t))
            });
            let (Ok(a), Ok(b)) = (a, b) else { unreachable!("both draw") };
            if k > 0 {
                assert!(
                    partial.last_composited() < u64::from(W * H),
                    "frame {k}: a small move composites a little ({} pixels)",
                    partial.last_composited()
                );
            }
            assert_eq!(
                worst(&a, &b),
                0,
                "frame {k} (glass {glass}): kept pixels plus redrawn damage equal a full frame"
            );
        });
        assert!(partial.renderer().context().take_errors().is_empty());
    }
}

#[test]
fn gpu_damage_frames_match_the_cpu_compositor() {
    let mut g = gpu();
    let mut cpu = CpuCompositor::new();
    frames(true, |k, plan| {
        let (Ok(a), Ok(b)) =
            (g.present_to_frame(plan, W, H, Color::BLACK), cpu.present(plan, W, H, Color::BLACK))
        else {
            unreachable!("both draw")
        };
        let w = worst(&a, &b);
        // Rasterizers round antialiased edges and blur weights a little differently.
        assert!(w <= 3, "frame {k}: the GPU's damage frames match the CPU's (worst {w})");
    });
}

#[test]
fn damage_under_glass_damages_the_glass() {
    let mut comp = Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(scene(10.0, true));
    let _ = comp.frame(Duration::ZERO);
    // The dot moves under the glass (and the blur's reach): the whole pane is damaged.
    comp.commit(scene(140.0, true));
    let plan = comp.frame(Duration::from_millis(16));
    let pane = Rect::new(100.0, 30.0, 220.0, 100.0);
    let covered: f32 =
        plan.damage.rects().iter().filter_map(|r| r.intersect(pane)).map(|r| r.size().area()).sum();
    assert_eq!(covered, pane.size().area(), "the pane is redrawn whole");
}
