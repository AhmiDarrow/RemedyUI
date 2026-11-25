//! A picture of a GPU window is read back from the frame the GPU compositor keeps, so it shows
//! what the screen does: Prism effects drawn by the GPU kernel into textures, blurs, drop
//! shadows and frosted glass. It matches the CPU renderer drawing the same window.

use std::time::Duration;

use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::{Color, CornerRadii, Rect, Vec2};
use remedy_ui_engine::compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_engine::effects::{CpuEffect, FrameInput, Instance};
use remedy_ui_engine::paint::{Brush, DisplayList, ExternalTexture, Filter, Image, Item, Sampling};
use remedy_ui_engine::render_cpu::{CpuCompositor, Frame};
use remedy_ui_engine::render_gpu::effects::GpuEffect;
use remedy_ui_engine::render_gpu::{FORMAT, GpuCompositor, GpuContext, GpuRenderer, LadderOptions, wgpu};

const W: u32 = 300;
const H: u32 = 200;

/// The window: a Prism effect filling a panel (`effect` draws it), a blurred badge over it, a
/// card with a drop shadow, and a pane of frosted glass across the effect.
fn window(effect: Item) -> LayerTree {
    let mut back = DisplayList::new();
    back.fill_rect(Rect::new(0.0, 0.0, 300.0, 200.0), Brush::Solid(Color::rgb8(24, 26, 34)));
    let mut fx = DisplayList::new();
    fx.push(effect);
    fx.push_filter(Filter::Blur { radius: 3.0 });
    fx.fill_rounded(Rect::new(20.0, 20.0, 70.0, 44.0), CornerRadii::all(6.0), Brush::Solid(Color::WHITE));
    fx.pop_filter();
    let mut card = DisplayList::new();
    card.push_filter(Filter::DropShadow {
        offset: Vec2::new(0.0, 5.0),
        blur: 10.0,
        color: Color::rgba8(0, 0, 0, 170),
    });
    card.fill_rounded(
        Rect::new(0.0, 0.0, 70.0, 40.0),
        CornerRadii::all(8.0),
        Brush::Solid(Color::rgb8(240, 236, 220)),
    );
    card.pop_filter();
    let mut tint = DisplayList::new();
    tint.fill_rounded(
        Rect::new(0.0, 0.0, 140.0, 60.0),
        CornerRadii::all(14.0),
        Brush::Solid(Color::rgba8(255, 255, 255, 40)),
    );
    let root = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 300.0, 200.0), back)
        .with_child(
            Layer::new(LayerId(2), Rect::new(10.0, 10.0, 210.0, 150.0), fx).clipped(CornerRadii::all(10.0)),
        )
        .with_child(Layer::new(LayerId(3), Rect::new(215.0, 120.0, 285.0, 160.0), card))
        .with_child(
            Layer::new(LayerId(4), Rect::new(40.0, 90.0, 180.0, 150.0), tint)
                .with_backdrop(6.0, CornerRadii::all(14.0)),
        );
    LayerTree::new(vec![root])
}

/// (pixels differing by more than `tol`, mean absolute difference per channel).
fn compare(a: &Frame, b: &Frame, tol: u8) -> (usize, f64) {
    let mut over = 0;
    let mut sum = 0u64;
    for (p, q) in a.pixels.chunks(4).zip(b.pixels.chunks(4)) {
        if p.iter().zip(q).any(|(x, y)| x.abs_diff(*y) > tol) {
            over += 1;
        }
        sum += p.iter().zip(q).map(|(x, y)| u64::from(x.abs_diff(*y))).sum::<u64>();
    }
    (over, sum as f64 / a.pixels.len().max(1) as f64)
}

#[test]
fn gpu_snapshots_show_effects_and_filters_like_the_cpu() {
    let ctx = match GpuContext::new(&LadderOptions::default()) {
        Ok(c) => c,
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    };
    let Some(effect) = remedy_ui_engine::effects::library("plasma") else { unreachable!("plasma compiles") };
    let instance = Instance::new(effect.clone());
    let input = FrameInput::new(200.0, 140.0, 1.0, 1.5);
    let rect = Rect::new(0.0, 0.0, 200.0, 140.0);

    // The GPU draws the effect with its kernel into a texture the window places.
    let (tw, th) = GpuEffect::output_size(&effect, &input);
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("effect"),
        size: wgpu::Extent3d { width: tw, height: th, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut gfx = GpuEffect::new(&ctx);
    assert!(gfx.render(&ctx, &instance, &input, &texture).is_ok());
    let Ok(id) = ctx.register_texture(&texture) else { unreachable!("registers") };
    let on_gpu = Item::Texture {
        texture: ExternalTexture { id, frame: 1, width: tw, height: th },
        rect,
        radii: CornerRadii::ZERO,
    };
    // The CPU draws the same effect with its interpreter as an image.
    let pic = CpuEffect::new().render(&instance, &input);
    let on_cpu = Item::Image {
        image: Image::new(pic.width, pic.height, pic.pixels.into()),
        rect,
        radii: CornerRadii::ZERO,
        sampling: Sampling::Linear,
    };

    let mut gpu = GpuCompositor::new(GpuRenderer::new(ctx.clone(), None), LadderOptions::default());
    let mut comp = Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(window(on_gpu));
    let plan = comp.frame(Duration::ZERO);
    // What a GPU window's snapshot reads: the frame its compositor keeps.
    let Ok(snapshot) = gpu.present_to_frame(&plan, W, H, Color::BLACK) else { unreachable!("presents") };
    // The old workaround, a CPU renderer drawing the GPU window's plan, cannot see the
    // effect's texture: the panel comes out empty there.
    let Ok(workaround) = CpuCompositor::new().present(&plan, W, H, Color::BLACK) else {
        unreachable!("draws")
    };

    let mut cpu_comp = Compositor::new(W, H, 1.0, ScrollPhysics::CLAMPED);
    cpu_comp.commit(window(on_cpu));
    let Ok(reference) = CpuCompositor::new().present(&cpu_comp.frame(Duration::ZERO), W, H, Color::BLACK)
    else {
        unreachable!("draws")
    };

    let (over, mean) = compare(&snapshot, &reference, 24);
    eprintln!("snapshot vs CPU window: mean {mean:.3}, {over} of {} beyond 24", W * H);
    assert!(mean < 1.0, "the GPU snapshot matches the CPU window (mean difference {mean:.3})");
    assert!(over * 200 <= (W * H) as usize, "{over} pixels differ by more than 24");
    let (_, missing) = compare(&workaround, &reference, 24);
    assert!(
        missing > mean * 4.0,
        "the CPU workaround misses the effect ({missing:.3}), the snapshot does not"
    );
    assert!(ctx.take_errors().is_empty());
}
