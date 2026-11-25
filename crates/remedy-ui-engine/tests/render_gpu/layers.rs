//! What a layer costs: many small sibling layers against the same drawing merged into their
//! parent, on both compositors. The measurement behind not merging or squashing layers
//! (docs/design/04-rendering.md, "Not yet in code").

use std::time::{Duration, Instant};

use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::{Color, CornerRadii, Rect, Vec2};
use remedy_ui_engine::compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_engine::paint::{Affine, Brush, DisplayList, Item};
use remedy_ui_engine::render_cpu::CpuCompositor;
use remedy_ui_engine::render_gpu::*;

const W: u32 = 1280;
const H: u32 = 800;

/// A 24-pixel icon; `frame` changes the first one's colour (what changes each frame).
fn icon(k: u64, frame: u64) -> DisplayList {
    let mut l = DisplayList::new();
    let c = Color::rgb8(((k * 37 + if k == 0 { frame * 50 } else { 0 }) % 255) as u8, 120, 200);
    l.fill_rounded(Rect::new(0.0, 0.0, 24.0, 24.0), CornerRadii::all(6.0), Brush::Solid(c));
    l
}

/// `n` icons on a white window: each a layer of its own, or all drawn in the window's layer.
fn tree(n: u64, merged: bool, frame: u64) -> LayerTree {
    let at = |k: u64| Vec2::new((k % 40) as f32 * 32.0, (k / 40) as f32 * 32.0);
    let window = Rect::new(0.0, 0.0, W as f32, H as f32);
    let mut back = DisplayList::new();
    back.fill_rect(window, Brush::Solid(Color::WHITE));
    if merged {
        for k in 0..n {
            back.push(Item::PushTransform(Affine::translate(at(k))));
            back.extend(&icon(k, frame));
            back.push(Item::PopTransform);
        }
        let mut root = Layer::new(LayerId(1), window, back);
        root.content_version = frame;
        return LayerTree::new(vec![root]);
    }
    let mut root = Layer::new(LayerId(1), window, back);
    for k in 0..n {
        let p = at(k);
        let mut c = Layer::new(LayerId(100 + k), Rect::new(p.x, p.y, p.x + 24.0, p.y + 24.0), icon(k, frame));
        c.content_version = if k == 0 { frame } else { 0 };
        root = root.with_child(c);
    }
    LayerTree::new(vec![root])
}

#[test]
#[ignore = "a measurement: cargo test --release -p remedy-ui-engine --features render-gpu --test layers -- --ignored --nocapture"]
fn sibling_layers_cost_against_merging_them() {
    const FRAMES: u32 = 20;
    for n in [50u64, 400, 1000] {
        for merged in [false, true] {
            let Ok(ctx) = GpuContext::new(&LadderOptions::default()) else { unreachable!("a device") };
            let mut gpu = GpuCompositor::new(GpuRenderer::new(ctx, None), LadderOptions::default());
            let mut cpu = CpuCompositor::new();
            let (mut cc, mut gc) = (
                Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED),
                Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED),
            );
            let wait = |g: &GpuCompositor| {
                if let Ok(t) = g.renderer().create_target(1, 1) {
                    let _ = g.renderer().read_back(&t);
                }
            };
            cc.commit(tree(n, merged, 0));
            gc.commit(tree(n, merged, 0));
            let p = cc.frame(Duration::ZERO);
            let t = Instant::now();
            let _ = cpu.present(&p, W, H, Color::WHITE);
            let first_cpu = t.elapsed();
            let p = gc.frame(Duration::ZERO);
            let t = Instant::now();
            let _ = gpu.present_to_kept(&p, W, H, Color::WHITE);
            wait(&gpu);
            let first_gpu = t.elapsed();
            let (mut plan, mut c, mut g) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
            for f in 1..=u64::from(FRAMES) {
                cc.commit(tree(n, merged, f));
                gc.commit(tree(n, merged, f));
                let t = Instant::now();
                let p = cc.frame(Duration::from_millis(16 * f));
                plan += t.elapsed();
                let t = Instant::now();
                let _ = cpu.present(&p, W, H, Color::WHITE);
                c += t.elapsed();
                let p = gc.frame(Duration::from_millis(16 * f));
                let t = Instant::now();
                let _ = gpu.present_to_kept(&p, W, H, Color::WHITE);
                wait(&gpu);
                g += t.elapsed();
            }
            eprintln!(
                "{n:4} icons, {}: plan {:?}/frame; first frame cpu {first_cpu:?} gpu {first_gpu:?}; one icon changing cpu {:?} gpu {:?}; cpu tiles {} ({} KB)",
                if merged { "merged     " } else { "own layers " },
                plan / FRAMES,
                c / FRAMES,
                g / FRAMES,
                cpu.tile_count(),
                cpu.tile_bytes() / 1024
            );
        }
    }
}
