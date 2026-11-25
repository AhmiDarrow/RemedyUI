//! The GPU compositor draws only the damage into the frame it keeps, and the result is the
//! same, pixel for pixel, as drawing the whole frame again; frosted glass over moving content
//! included.

use std::time::Duration;

use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::{Color, CornerRadii, Rect};
use remedy_ui_engine::compositor::{Compositor, FramePlan, Layer, LayerId, LayerTree};
use remedy_ui_engine::paint::{Brush, DisplayList, Filter};
use remedy_ui_engine::render_cpu::{CpuCompositor, Frame};
use remedy_ui_engine::render_gpu::*;

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
fn frames(glass: bool, f: impl FnMut(usize, &FramePlan)) {
    frames_at(glass, 1.0, f);
}

/// [`frames`] in a window of `scale` device pixels per logical pixel (W × H logical).
fn frames_at(glass: bool, scale: f32, mut f: impl FnMut(usize, &FramePlan)) {
    let (w, h) = ((W as f32 * scale) as u32, (H as f32 * scale) as u32);
    let mut comp = Compositor::new(w, h, scale, ScrollPhysics::CLAMPED);
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
fn hidpi_damage_frames_equal_full_redraws_and_match_the_cpu() {
    // 512-pixel tiles, glass on a grid twice the blur's reach, filters bounded at scale 2.
    let (w, h) = (W * 2, H * 2);
    for glass in [false, true] {
        let mut partial = gpu();
        let mut full = gpu();
        let mut cpu = CpuCompositor::new();
        frames_at(glass, 2.0, |k, plan| {
            let a = partial.present_to_frame(plan, w, h, Color::BLACK);
            let b = full.renderer().create_target(w, h).and_then(|t| {
                full.present(plan, &t, Color::BLACK).and_then(|()| full.renderer().read_back(&t))
            });
            let c = cpu.present(plan, w, h, Color::BLACK);
            let (Ok(a), Ok(b), Ok(c)) = (a, b, c) else { unreachable!("all draw") };
            if k > 0 {
                assert!(partial.last_composited() < u64::from(w * h), "frame {k} composites only its damage");
            }
            assert_eq!(worst(&a, &b), 0, "frame {k} (glass {glass}): damage-only equals a full frame at 2x");
            let wc = worst(&a, &c);
            assert!(wc <= 3, "frame {k} (glass {glass}): the GPU matches the CPU at 2x (worst {wc})");
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

/// A blurred group over the stripes whose child layer (a dot) moves, and a drop-shadowed
/// group around a second moving layer.
fn filtered_scene(x: f32) -> LayerTree {
    let LayerTree { roots } = scene(x, false);
    let Some(back) = roots.first() else { unreachable!("a root") };
    let back_list = (*back.content).clone();
    let mut dot = DisplayList::new();
    dot.fill_rounded(Rect::new(0.0, 0.0, 22.0, 22.0), CornerRadii::all(11.0), Brush::Solid(Color::WHITE));
    let mut plate = DisplayList::new();
    plate.fill_rect(Rect::new(0.0, 0.0, 120.0, 50.0), Brush::Solid(Color::rgba8(255, 255, 255, 60)));
    let blurred = Layer::new(LayerId(20), Rect::new(40.0, 20.0, 160.0, 70.0), plate)
        .with_filters(vec![Filter::Blur { radius: 3.0 }])
        .with_child(Layer::new(LayerId(21), Rect::new(x * 0.4, 14.0, x * 0.4 + 22.0, 36.0), dot.clone()));
    let shadowed = Layer::new(LayerId(30), Rect::new(180.0, 110.0, 300.0, 180.0), DisplayList::new())
        .with_filters(vec![Filter::DropShadow {
            offset: remedy_ui_core::Vec2::new(4.0, 6.0),
            blur: 10.0,
            color: Color::rgba8(0, 0, 0, 180),
        }])
        .with_child(Layer::new(
            LayerId(31),
            Rect::new(10.0 + x * 0.3, 10.0, 32.0 + x * 0.3, 32.0),
            dot.clone(),
        ));
    // A colour matrix over a layer whose child moves: only the damage is recoloured again.
    let mut tint = DisplayList::new();
    tint.fill_rect(Rect::new(0.0, 0.0, 90.0, 60.0), Brush::Solid(Color::rgba8(220, 120, 30, 200)));
    let recoloured = Layer::new(LayerId(40), Rect::new(20.0, 110.0, 110.0, 170.0), tint)
        .with_filters(vec![Filter::ColorMatrix { matrix: remedy_ui_core::ColorMatrix::hue_rotate(150.0) }])
        .with_child(Layer::new(LayerId(41), Rect::new(x * 0.5, 20.0, x * 0.5 + 22.0, 42.0), dot));
    let root = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 320.0, 200.0), back_list)
        .with_child(blurred)
        .with_child(shadowed)
        .with_child(recoloured);
    LayerTree::new(vec![root])
}

#[test]
fn filtered_groups_of_layers_draw_from_damage_like_a_full_frame() {
    let mut comp = Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED);
    let mut partial = gpu();
    let mut full = gpu();
    let mut cpu = CpuCompositor::new();
    for (k, x) in [10.0f32, 30.5, 61.0, 96.0, 140.25, 190.0].into_iter().enumerate() {
        comp.commit(filtered_scene(x));
        let plan = comp.frame(Duration::from_millis(16 * k as u64));
        assert!(
            plan.ops
                .iter()
                .any(|o| matches!(o, remedy_ui_engine::compositor::CompositeOp::PushFilter { .. }))
        );
        let a = partial.present_to_frame(&plan, W, H, Color::BLACK);
        let b = full
            .renderer()
            .create_target(W, H)
            .and_then(|t| full.present(&plan, &t, Color::BLACK).and_then(|()| full.renderer().read_back(&t)));
        let c = cpu.present(&plan, W, H, Color::BLACK);
        let (Ok(a), Ok(b), Ok(c)) = (a, b, c) else { unreachable!("all draw") };
        if k > 0 {
            assert!(partial.last_composited() < u64::from(W * H), "frame {k} composites only its damage");
        }
        assert_eq!(worst(&a, &b), 0, "frame {k}: damage-only equals a full frame on the GPU");
        assert!(worst(&a, &c) <= 3, "frame {k}: the GPU matches the CPU (worst {})", worst(&a, &c));
    }
    // The CPU's damage frames equal a full CPU frame too.
    let mut fresh = Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED);
    fresh.commit(filtered_scene(190.0));
    let full_cpu = CpuCompositor::new().present(&fresh.frame(Duration::ZERO), W, H, Color::BLACK);
    let last = cpu.present(&comp.frame(Duration::from_millis(200)), W, H, Color::BLACK);
    let (Ok(full_cpu), Ok(last)) = (full_cpu, last) else { unreachable!("both draw") };
    assert_eq!(worst(&full_cpu, &last), 0);
}

#[test]
#[ignore = "a measurement: cargo test --release -p remedy-ui-render-gpu --test damage -- --ignored --nocapture"]
fn damage_only_compositing_costs_less_than_full_frames() {
    // A 4K frame: compositing alone is timed (present, then wait for the GPU), with no read
    // back, which at this size costs more than either composite and would hide the gain.
    let (w, h) = (3840u32, 2160u32);
    let (fw, fh) = (w as f32, h as f32);
    let tree = |x: f32| {
        let mut back = DisplayList::new();
        for i in 0..384 {
            let c = Color::rgb8((i / 2) as u8, 90, (200 - i / 2) as u8);
            back.fill_rect(Rect::new(i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, fh), Brush::Solid(c));
        }
        let mut dot = DisplayList::new();
        dot.fill_rounded(Rect::new(0.0, 0.0, 24.0, 24.0), CornerRadii::all(12.0), Brush::Solid(Color::WHITE));
        LayerTree::new(vec![
            Layer::new(LayerId(1), Rect::new(0.0, 0.0, fw, fh), back).with_child(Layer::new(
                LayerId(2),
                Rect::new(x, 300.0, x + 24.0, 324.0),
                dot,
            )),
        ])
    };
    let wait = |c: &GpuCompositor| {
        let _ = c
            .renderer()
            .context()
            .device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    };
    let mut comp = Compositor::new(w, h, 1.0, ScrollPhysics::CLAMPED);
    let mut partial = gpu();
    let mut full = gpu();
    let target = full.renderer().create_target(w, h).unwrap_or_else(|e| unreachable!("{e}"));
    let (mut t_partial, mut t_full) = (Duration::ZERO, Duration::ZERO);
    for k in 0..65u64 {
        comp.commit(tree(100.0 + k as f32 * 7.0));
        let plan = comp.frame(Duration::from_millis(16 * k));
        let s = std::time::Instant::now();
        assert!(partial.present_to_kept(&plan, w, h, Color::BLACK).is_ok());
        wait(&partial);
        if k >= 5 {
            t_partial += s.elapsed();
        }
        let s = std::time::Instant::now();
        assert!(full.present(&plan, &target, Color::BLACK).is_ok());
        wait(&full);
        if k >= 5 {
            t_full += s.elapsed();
        }
    }
    let (p, f) = (t_partial.as_secs_f64() * 1000.0 / 60.0, t_full.as_secs_f64() * 1000.0 / 60.0);
    eprintln!(
        "{}: {w}x{h}, a 24-pixel dot moving: damage-only {p:.3} ms a frame, full {f:.3} ms ({:.1}x), {} of {} pixels composited",
        full.renderer().context().describe(),
        f / p,
        partial.last_composited(),
        w * h
    );
    assert!(p * 2.0 < f, "damage-only is at least twice as cheap at 4K ({p:.3} vs {f:.3} ms)");
    // A 4K snapshot: the first read back makes the staging buffer, the rest reuse it.
    let _ = full.renderer().read_back(&target);
    let s = std::time::Instant::now();
    for _ in 0..20 {
        assert!(full.renderer().read_back(&target).is_ok());
    }
    eprintln!("4K read back: {:.2} ms", s.elapsed().as_secs_f64() * 1000.0 / 20.0);
}

#[test]
#[ignore = "a measurement: cargo test --release -p remedy-ui-render-gpu --test damage -- --ignored --nocapture"]
fn a_hidpi_4k_first_frame() {
    // A 1920x1080 window at scale 2: 3840x2160 device pixels in 512-pixel tiles.
    let (w, h, scale) = (3840u32, 2160u32, 2.0f32);
    let mut back = DisplayList::new();
    for i in 0..96 {
        for j in 0..54 {
            let c = Color::rgb8((i * 2) as u8, (j * 4) as u8, 160);
            let r = Rect::new(
                i as f32 * 20.0 + 2.0,
                j as f32 * 20.0 + 2.0,
                i as f32 * 20.0 + 18.0,
                j as f32 * 20.0 + 18.0,
            );
            back.fill_rounded(r, CornerRadii::all(4.0), Brush::Solid(c));
        }
    }
    let tree = LayerTree::new(vec![Layer::new(LayerId(1), Rect::new(0.0, 0.0, 1920.0, 1080.0), back)]);
    let mut times = Vec::new();
    for _ in 0..5 {
        let mut comp = Compositor::new(w, h, scale, ScrollPhysics::CLAMPED);
        comp.commit(tree.clone());
        let plan = comp.frame(Duration::ZERO);
        let mut g = gpu();
        let s = std::time::Instant::now();
        assert!(g.present_to_kept(&plan, w, h, Color::BLACK).is_ok());
        let _ = g
            .renderer()
            .context()
            .device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        times.push(s.elapsed().as_secs_f64() * 1000.0);
        assert!(g.renderer().context().take_errors().is_empty());
        if times.len() == 1 {
            eprintln!("{} tiles rastered", plan.raster.len());
        }
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "4K at scale 2, first frame: median {:.2} ms (of {times:?})",
        times.get(2).copied().unwrap_or(0.0)
    );
}
