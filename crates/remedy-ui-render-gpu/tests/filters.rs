//! Filters on ordinary content (blur, drop shadows of any shape, backdrop blur) drawn by the
//! GPU renderer match the CPU reference, directly and through compositor tiles.

use std::time::Duration;

use remedy_ui_compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_paint::{Brush, DisplayList, Filter, Item, Path};
use remedy_ui_render_cpu::{CpuCompositor, CpuRenderer, Frame};
use remedy_ui_render_gpu::*;

fn gpu() -> GpuRenderer {
    match GpuContext::new(&LadderOptions::default()) {
        Ok(ctx) => GpuRenderer::new(ctx, None),
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    }
}

/// Pixels differing by more than `tol` on any channel, and the largest difference.
fn differing(a: &Frame, b: &Frame, tol: u8) -> (usize, u8) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut worst = 0u8;
    let n = a
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(x, y)| {
            let d = x.iter().zip(y.iter()).map(|(p, q)| p.abs_diff(*q)).max().unwrap_or(0);
            worst = worst.max(d);
            d > tol
        })
        .count();
    (n, worst)
}

fn filtered() -> DisplayList {
    let mut l = DisplayList::new();
    // Stripes for the glass to blur.
    for i in 0..12 {
        let c = if i % 2 == 0 { Color::rgb8(220, 60, 40) } else { Color::rgb8(30, 90, 200) };
        l.fill_rect(Rect::new(i as f32 * 20.0, 0.0, i as f32 * 20.0 + 20.0, 240.0), Brush::Solid(c));
    }
    // A blurred card.
    l.push_filter(Filter::Blur { radius: 4.0 });
    l.fill_rounded(Rect::new(20.0, 20.0, 110.0, 80.0), CornerRadii::all(10.0), Brush::Solid(Color::WHITE));
    l.pop_filter();
    // A drop shadow of a star (any shape, not a box).
    let mut star = Path::new();
    for k in 0..10 {
        let a = k as f32 * std::f32::consts::PI / 5.0;
        let r = if k % 2 == 0 { 34.0 } else { 14.0 };
        let p = Point::new(180.0 + r * a.sin(), 60.0 - r * a.cos());
        if k == 0 {
            star.move_to(p);
        } else {
            star.line_to(p);
        }
    }
    star.close();
    l.push_filter(Filter::DropShadow {
        offset: Vec2::new(4.0, 6.0),
        blur: 8.0,
        color: Color::rgba8(0, 0, 0, 160),
    });
    l.push(Item::FillPath { path: star, brush: Brush::Solid(Color::rgb8(250, 210, 40)), even_odd: false });
    l.pop_filter();
    // Frosted glass over the stripes.
    l.push(Item::Backdrop {
        rect: Rect::new(40.0, 120.0, 200.0, 210.0),
        radii: CornerRadii::all(14.0),
        blur: 6.0,
    });
    l.fill_rounded(
        Rect::new(40.0, 120.0, 200.0, 210.0),
        CornerRadii::all(14.0),
        Brush::Solid(Color::rgba8(255, 255, 255, 60)),
    );
    l
}

#[test]
fn filters_match_between_gpu_and_cpu() {
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    for scale in [1.0f32, 2.0] {
        let (w, h) = ((240.0 * scale) as u32, (240.0 * scale) as u32);
        let list = filtered();
        let (Ok(a), Ok(b)) =
            (c.render(&list, w, h, scale, Color::WHITE), g.render(&list, w, h, scale, Color::WHITE))
        else {
            unreachable!("both render")
        };
        let (visible, worst) = differing(&a, &b, 8);
        eprintln!("{scale}x: {visible} of {} differ visibly, worst {worst}", w * h);
        assert!(visible * 1000 <= (w * h) as usize, "{visible} pixels differ visibly at {scale}x");
        assert!(worst <= 32, "no pixel differs grossly at {scale}x (worst {worst})");
    }
}

#[test]
fn filtered_content_is_seamless_across_tiles() {
    // A layer far larger than a tile, its blurs crossing tile edges.
    let (w, h) = (600, 520);
    let mut content = DisplayList::new();
    for (i, y) in [0.0f32, 250.0].into_iter().enumerate() {
        for (j, x) in [0.0f32, 245.0].into_iter().enumerate() {
            let _ = (i, j);
            content.push(Item::PushTransform(remedy_ui_paint::Affine::translate(Vec2::new(x, y))));
            content.extend(&filtered());
            content.push(Item::PopTransform);
        }
    }
    let tree =
        LayerTree::new(vec![Layer::new(LayerId(1), Rect::new(0.0, 0.0, 600.0, 520.0), content.clone())]);
    let mut comp = Compositor::new(w, h, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(tree);
    let plan = comp.frame(Duration::ZERO);
    assert!(plan.raster.len() > 1, "the layer is tiled");
    let direct = CpuRenderer::new().render(&content, w, h, 1.0, Color::WHITE);
    let cpu = CpuCompositor::new().present(&plan, w, h, Color::WHITE);
    let mut gc = GpuCompositor::new(gpu(), LadderOptions::default());
    let g = gc.present_to_frame(&plan, w, h, Color::WHITE);
    let (Ok(direct), Ok(cpu), Ok(g)) = (direct, cpu, g) else { unreachable!("all draw") };
    let (n, worst) = differing(&direct, &cpu, 2);
    eprintln!("tiles vs direct (cpu): {n} differ, worst {worst}");
    assert_eq!(n, 0, "CPU tiles match drawing the whole list at once (worst {worst})");
    let (visible, worst) = differing(&cpu, &g, 8);
    eprintln!("gpu vs cpu composite: {visible} differ visibly, worst {worst}");
    assert!(visible * 1000 <= (w * h) as usize && worst <= 32);
}

#[test]
#[ignore = "writes pictures for a person to look at"]
fn dump_filters() {
    let dir = std::env::var("DUMP_DIR").unwrap_or_else(|_| ".".into());
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    let list = filtered();
    for (name, f) in [
        ("cpu", c.render(&list, 480, 480, 2.0, Color::WHITE).ok()),
        ("gpu", g.render(&list, 480, 480, 2.0, Color::WHITE).ok()),
    ] {
        if let Some(f) = f {
            let _ = std::fs::write(format!("{dir}/filters_{name}.rgba"), &f.pixels);
        }
    }
}

#[test]
fn hostile_filter_sizes_are_bounded_on_both_renderers() {
    // A theme asking for absurd blurs and shadows: drawn at the bound, quickly, with no huge
    // buffers, and exactly as the bound itself draws.
    let list = |blur: f32, offset: f32| {
        let mut l = DisplayList::new();
        l.push_filter(Filter::Blur { radius: blur });
        l.fill_rect(Rect::new(40.0, 40.0, 80.0, 80.0), Brush::Solid(Color::BLACK));
        l.pop_filter();
        l.push_filter(Filter::DropShadow { offset: Vec2::new(offset, offset), blur, color: Color::BLACK });
        l.fill_rect(Rect::new(100.0, 40.0, 140.0, 80.0), Brush::Solid(Color::rgb8(200, 40, 40)));
        l.pop_filter();
        l.push(Item::Backdrop { rect: Rect::new(10.0, 10.0, 150.0, 110.0), radii: CornerRadii::ZERO, blur });
        l
    };
    let scale = 2.0;
    let bound = remedy_ui_paint::MAX_FILTER_STD_DEV / scale;
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    for (blur, offset) in [(1.0e9, 1.0e9), (f32::INFINITY, f32::NEG_INFINITY), (f32::NAN, f32::NAN)] {
        let started = std::time::Instant::now();
        let hostile = list(blur, offset);
        let (Ok(a), Ok(b)) = (
            c.render(&hostile, 320, 240, scale, Color::WHITE),
            g.render(&hostile, 320, 240, scale, Color::WHITE),
        ) else {
            unreachable!("both render")
        };
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{blur}: bounded work");
        if blur.is_finite() {
            let capped = list(bound, remedy_ui_paint::MAX_FILTER_OFFSET / scale);
            let Ok(want) = c.render(&capped, 320, 240, scale, Color::WHITE) else { unreachable!("renders") };
            assert_eq!(differing(&a, &want, 0).0, 0, "{blur}: drawn exactly as the bound");
        }
        let (visible, _) = differing(&a, &b, 8);
        assert!(visible * 1000 <= (320 * 240) as usize, "{blur}: GPU and CPU agree");
    }
    assert!(g.context().take_errors().is_empty());
}
