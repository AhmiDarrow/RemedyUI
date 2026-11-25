//! Filters on ordinary content (blur, drop shadows of any shape, backdrop blur) drawn by the
//! GPU renderer match the CPU reference, directly and through compositor tiles.

use std::time::Duration;

use remedy_ui_core::input::ScrollPhysics;
use remedy_ui_core::{Color, ColorMatrix, CornerRadii, Point, Rect, Vec2};
use remedy_ui_engine::compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_engine::paint::{Blend, Brush, DisplayList, Filter, Item, Path};
use remedy_ui_engine::render_cpu::{CpuCompositor, CpuRenderer, Frame};
use remedy_ui_engine::render_gpu::*;

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
            content.push(Item::PushTransform(remedy_ui_engine::paint::Affine::translate(Vec2::new(x, y))));
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
    let bound = remedy_ui_engine::paint::MAX_FILTER_STD_DEV;
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
            let capped = list(bound, remedy_ui_engine::paint::MAX_FILTER_OFFSET / scale);
            let Ok(want) = c.render(&capped, 320, 240, scale, Color::WHITE) else { unreachable!("renders") };
            assert_eq!(differing(&a, &want, 0).0, 0, "{blur}: drawn exactly as the bound");
        }
        let (visible, _) = differing(&a, &b, 8);
        assert!(visible * 1000 <= (320 * 240) as usize, "{blur}: GPU and CPU agree");
    }
    assert!(g.context().take_errors().is_empty());
}

#[test]
fn shadows_thrown_off_the_top_or_left_edge_are_not_drawn_on_both_renderers() {
    // A drop shadow thrown further up or left than its content's filter area is wide or tall
    // lands wholly off the canvas: nothing of it shows. (vello_cpu 0.4.0 once drew it unshifted
    // instead; found by `random_unbalanced_lists_draw_without_failing_on_both_renderers`.)
    let card = |l: &mut DisplayList| {
        l.fill_rounded(
            Rect::new(-5.84, -15.488, 28.8195, 30.1691),
            CornerRadii::all(8.16),
            Brush::Solid(Color::rgba8(246, 193, 234, 207)),
        );
    };
    let shadowed = |offset: Vec2| {
        let mut l = DisplayList::new();
        l.push(Item::PushFilter(Filter::DropShadow {
            offset,
            blur: 0.714,
            color: Color::rgba8(125, 23, 239, 252),
        }));
        l.push(Item::PushLayer { opacity: 0.0531, blend: Blend::Normal });
        l.push(Item::PopLayer);
        card(&mut l);
        // Left open: the painter closes the group at the end.
        l
    };
    let mut plain = DisplayList::new();
    card(&mut plain);
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    let Ok(want) = c.render(&plain, 96, 96, 1.0, Color::WHITE) else { unreachable!("renders") };
    for offset in [Vec2::new(-1.0e9, -4.622), Vec2::new(-60.0, 3.0), Vec2::new(2.0, -2048.0)] {
        let list = shadowed(offset);
        let (Ok(a), Ok(b)) =
            (c.render(&list, 96, 96, 1.0, Color::WHITE), g.render(&list, 96, 96, 1.0, Color::WHITE))
        else {
            unreachable!("both render")
        };
        assert_eq!(a.pixel(10, 10), Some([248, 205, 238, 255]), "{offset:?}: CPU, no shadow");
        // Drawn through a filter layer, edges may round a little differently: no more.
        assert_eq!(
            differing(&a, &want, 2),
            (0, differing(&a, &want, 2).1),
            "{offset:?}: CPU draws no shadow"
        );
        assert_eq!(
            differing(&b, &want, 2),
            (0, differing(&b, &want, 2).1),
            "{offset:?}: GPU draws no shadow"
        );
        assert_eq!(differing(&a, &b, 2), (0, differing(&a, &b, 2).1), "{offset:?}: GPU and CPU agree");
    }
    assert!(g.context().take_errors().is_empty());
}

#[test]
fn nested_filters_share_one_bound_on_both_renderers() {
    // Shadows nested at the bound: each level's buffers would be thousands of pixels wider
    // than the last, past what the GPU holds and with work growing with the square of the
    // depth. The chain reaches as far as one filter at the bound: drawn quickly, alike on both
    // renderers, and exactly as the bound draws it (the outer shadow at the bound, the ones
    // inside it with nothing left).
    let nested = |shadows: &[Filter]| {
        let mut l = DisplayList::new();
        for f in shadows {
            l.push_filter(*f);
        }
        l.fill_rect(Rect::new(10.0, 10.0, 60.0, 60.0), Brush::Solid(Color::rgb8(200, 40, 40)));
        for _ in shadows {
            l.pop_filter();
        }
        l
    };
    let color = Color::rgba8(0, 0, 0, 128);
    let shadow = |offset: Vec2, blur: f32| Filter::DropShadow { offset, blur, color };
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    for (x, y) in [(1.0e9, 0.0), (1.0e9, 1.0e9), (-1.0e9, -1.0e9)] {
        let started = std::time::Instant::now();
        let deep = nested(&[shadow(Vec2::new(x, y), 1.0e9); 8]);
        let (Ok(a), Ok(b)) =
            (c.render(&deep, 96, 96, 1.0, Color::WHITE), g.render(&deep, 96, 96, 1.0, Color::WHITE))
        else {
            unreachable!("({x}, {y}): both render")
        };
        assert!(started.elapsed() < Duration::from_secs(20), "({x}, {y}): bounded work");
        let bound = remedy_ui_engine::paint::MAX_FILTER_OFFSET;
        let at_bound =
            Vec2::new(bound.copysign(x) * f32::from(x != 0.0), bound.copysign(y) * f32::from(y != 0.0));
        let mut want = vec![shadow(at_bound, remedy_ui_engine::paint::MAX_FILTER_STD_DEV * 2.0)];
        want.extend([shadow(Vec2::ZERO, 0.0); 7]);
        let Ok(want) = c.render(&nested(&want), 96, 96, 1.0, Color::WHITE) else { unreachable!("renders") };
        assert_eq!(differing(&a, &want, 0).0, 0, "({x}, {y}): drawn exactly as the bound");
        let (visible, worst) = differing(&a, &b, 8);
        assert!(visible * 1000 <= 96 * 96, "({x}, {y}): GPU and CPU agree ({visible} differ, by {worst})");
    }
    assert!(g.context().take_errors().is_empty());
}

#[test]
fn nested_layer_filters_share_one_bound_on_both_compositors() {
    // The same through the compositor: layers nested in layers, each with a shadow at the
    // bound, drawn as the bound draws them, quickly, alike on the CPU and the GPU.
    let color = Color::rgba8(0, 0, 0, 128);
    let shadow = |offset: Vec2, blur: f32| Filter::DropShadow { offset, blur, color };
    let tree = |shadows: &[Filter]| {
        let mut card = DisplayList::new();
        card.fill_rect(Rect::new(0.0, 0.0, 50.0, 50.0), Brush::Solid(Color::rgb8(200, 40, 40)));
        let mut layer = Layer::new(LayerId(100), Rect::new(10.0, 10.0, 60.0, 60.0), card);
        for (i, f) in shadows.iter().enumerate().rev() {
            let id = LayerId(10 + i as u64);
            layer = Layer::new(id, Rect::new(0.0, 0.0, 96.0, 96.0), DisplayList::new())
                .with_filters(vec![*f])
                .with_child(layer);
        }
        LayerTree::new(vec![layer])
    };
    let present = |tree: LayerTree, gpu: Option<&mut GpuCompositor>| {
        let mut comp = Compositor::new(96, 96, 1.0, ScrollPhysics::CLAMPED);
        comp.commit(tree);
        let plan = comp.frame(Duration::ZERO);
        match gpu {
            Some(g) => g.present_to_frame(&plan, 96, 96, Color::WHITE).ok(),
            None => CpuCompositor::new().present(&plan, 96, 96, Color::WHITE).ok(),
        }
    };
    let mut g = GpuCompositor::new(gpu(), LadderOptions::default());
    let bound = remedy_ui_engine::paint::MAX_FILTER_OFFSET;
    for (x, y) in [(1.0e9f32, 1.0e9f32), (-1.0e9, 0.0)] {
        let started = std::time::Instant::now();
        let deep = [shadow(Vec2::new(x, y), 1.0e9); 6];
        let (Some(a), Some(b)) = (present(tree(&deep), None), present(tree(&deep), Some(&mut g))) else {
            unreachable!("({x}, {y}): both composite")
        };
        assert!(started.elapsed() < Duration::from_secs(20), "({x}, {y}): bounded work");
        let at_bound =
            Vec2::new(bound.copysign(x) * f32::from(x != 0.0), bound.copysign(y) * f32::from(y != 0.0));
        let mut want = vec![shadow(at_bound, remedy_ui_engine::paint::MAX_FILTER_STD_DEV * 2.0)];
        want.extend([shadow(Vec2::ZERO, 0.0); 5]);
        let Some(want) = present(tree(&want), None) else { unreachable!("composites") };
        assert_eq!(differing(&a, &want, 0).0, 0, "({x}, {y}): drawn exactly as the bound");
        let (visible, worst) = differing(&a, &b, 8);
        assert!(visible * 1000 <= 96 * 96, "({x}, {y}): GPU and CPU agree ({visible} differ, by {worst})");
    }
}

/// A small deterministic generator (xorshift), so a failing list can be found again.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn f(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.below(10_000) as f32 / 10_000.0) * (hi - lo)
    }
    /// Now and then a value no one should send.
    fn wild(&mut self, lo: f32, hi: f32) -> f32 {
        match self.below(12) {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => -1.0e9,
            3 => 1.0e9,
            _ => self.f(lo, hi),
        }
    }
}

/// A random list: shapes among pushes and pops of every kind, in any order and number
/// (unbalanced on purpose), with filters, glass and transforms, some of them absurd.
fn random_list(rng: &mut Rng, mats: &mut Rng) -> DisplayList {
    let mut l = DisplayList::new();
    for _ in 0..rng.below(40) + 5 {
        // Colour-matrix groups opened and closed among the rest, from a stream of their own
        // (the other items are the same lists as before matrices existed). Only matrices
        // that never amplify: nested gains above 1 multiply the renderers' one-step rounding
        // differences into visible ones, so agreement on those is checked over plain content
        // (colour_matrices_match_between_gpu_and_cpu, hostile_colour_matrices_...).
        match mats.below(10) {
            0 | 1 => {
                let a = mats.f(0.0, 1.0);
                let matrix = match mats.below(6) {
                    0 => ColorMatrix::grayscale(a),
                    1 => ColorMatrix::invert(a),
                    2 => ColorMatrix::saturate(a),
                    3 => ColorMatrix::opacity(a),
                    4 => ColorMatrix::brightness(a),
                    _ => ColorMatrix::contrast(a),
                };
                l.push(Item::PushFilter(Filter::ColorMatrix { matrix }));
            }
            2 => l.push(Item::PopFilter),
            _ => {}
        }
        let c = Color::rgba8(
            rng.below(256) as u8,
            rng.below(256) as u8,
            rng.below(256) as u8,
            rng.below(256) as u8,
        );
        let (x, y) = (rng.f(-20.0, 100.0), rng.f(-20.0, 100.0));
        let r = Rect::new(x, y, x + rng.f(1.0, 60.0), y + rng.f(1.0, 60.0));
        match rng.below(14) {
            0..=3 => l.fill_rounded(r, CornerRadii::all(rng.f(0.0, 12.0)), Brush::Solid(c)),
            4 => l.push(Item::PushFilter(Filter::Blur { radius: rng.wild(0.0, 8.0) })),
            5 => l.push(Item::PushFilter(Filter::DropShadow {
                offset: Vec2::new(rng.wild(-10.0, 10.0), rng.wild(-10.0, 10.0)),
                blur: rng.wild(0.0, 12.0),
                color: c,
            })),
            6 => l.push(Item::PopFilter),
            7 => l.push(Item::PushLayer {
                opacity: rng.f(0.0, 1.0),
                blend: remedy_ui_engine::paint::Blend::Normal,
            }),
            8 => l.push(Item::PopLayer),
            9 => l.push_clip(r, CornerRadii::all(rng.f(0.0, 8.0))),
            10 => l.pop_clip(),
            11 => l.push(Item::Backdrop {
                rect: r,
                radii: CornerRadii::all(rng.f(0.0, 8.0)),
                blur: rng.wild(0.0, 8.0),
            }),
            12 => l.push(Item::PushTransform(remedy_ui_engine::paint::Affine::translate(Vec2::new(
                rng.f(-10.0, 10.0),
                rng.f(-10.0, 10.0),
            )))),
            _ => l.push(Item::PopTransform),
        }
    }
    l
}

#[test]
fn random_unbalanced_lists_draw_without_failing_on_both_renderers() {
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let mut mats = Rng(0x5eed_c010_4a77_0002);
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    let mut gc = GpuCompositor::new(gpu(), LadderOptions::default());
    let mut cc = CpuCompositor::new();
    let mut comp = Compositor::new(96, 96, 1.0, ScrollPhysics::CLAMPED);
    for case in 0..60 {
        let list = random_list(&mut rng, &mut mats);
        let (Ok(a), Ok(b)) =
            (c.render(&list, 96, 96, 1.0, Color::WHITE), g.render(&list, 96, 96, 1.0, Color::WHITE))
        else {
            unreachable!("case {case}: both render")
        };
        let (visible, _) = differing(&a, &b, 24);
        assert!(
            visible * 50 <= 96 * 96,
            "case {case}: GPU and CPU agree ({visible} pixels differ)\n{list:?}"
        );
        // Through the compositor too: the list as a layer's content, tiled.
        let mut layer = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 96.0, 96.0), list);
        layer.content_version = case;
        comp.commit(LayerTree::new(vec![layer]));
        let plan = comp.frame(Duration::from_millis(16 * case));
        assert!(cc.present(&plan, 96, 96, Color::WHITE).is_ok(), "case {case}: CPU composite");
        assert!(gc.present_to_frame(&plan, 96, 96, Color::WHITE).is_ok(), "case {case}: GPU composite");
    }
    assert!(g.context().take_errors().is_empty(), "no GPU validation errors");
}

/// The pixel at `(x, y)`.
fn pixel(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    let at = (y * f.width + x) as usize;
    f.pixels.as_chunks::<4>().0.get(at).copied().unwrap_or_default()
}

#[test]
fn deeply_nested_groups_are_bounded_on_both_renderers() {
    // A list from a stranger nesting thousands of groups: drawn within the depth bound, on a
    // test thread's stack, quickly. What the groups past the bound hold is still drawn, and
    // their pops close nothing, so the half-opacity group around them stays open until its own
    // pop.
    const N: usize = 3000;
    const { assert!(N > 4 * remedy_ui_engine::paint::MAX_GROUP_DEPTH) };
    let blue = Color::rgb8(20, 60, 200);
    let mut l = DisplayList::new();
    l.push(Item::PushLayer { opacity: 0.5, blend: remedy_ui_engine::paint::Blend::Normal });
    for i in 0..N {
        match i % 3 {
            0 => l.push_clip(Rect::new(0.0, 0.0, 320.0, 240.0), CornerRadii::all(4.0)),
            1 => l.push(Item::PushLayer { opacity: 1.0, blend: remedy_ui_engine::paint::Blend::Normal }),
            _ => l.push_filter(Filter::Blur { radius: 0.0 }),
        }
    }
    l.fill_rect(Rect::new(10.0, 10.0, 100.0, 100.0), Brush::Solid(Color::BLACK));
    for i in (0..N).rev() {
        match i % 3 {
            0 => l.pop_clip(),
            1 => l.push(Item::PopLayer),
            _ => l.pop_filter(),
        }
    }
    l.fill_rect(Rect::new(110.0, 10.0, 200.0, 100.0), Brush::Solid(blue));
    l.push(Item::PopLayer);
    l.fill_rect(Rect::new(210.0, 10.0, 300.0, 100.0), Brush::Solid(blue));
    let started = std::time::Instant::now();
    let (Ok(c), Ok(g)) = (
        CpuRenderer::new().render(&l, 320, 240, 1.0, Color::WHITE),
        gpu().render(&l, 320, 240, 1.0, Color::WHITE),
    ) else {
        unreachable!("both render")
    };
    for (name, f) in [("cpu", &c), ("gpu", &g)] {
        let near = |p: [u8; 4], q: [u8; 4]| p.iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 2);
        assert!(near(pixel(f, 50, 50), [127, 127, 127, 255]), "{name}: the deep fill, at half");
        assert!(near(pixel(f, 150, 50), [137, 157, 227, 255]), "{name}: still in the half group");
        assert!(near(pixel(f, 250, 50), [20, 60, 200, 255]), "{name}: after it, opaque");
    }
    assert_eq!(differing(&c, &g, 2).0, 0, "GPU matches CPU");

    // The same depth through the compositor: nested layers, each clipped and faded, walked,
    // composited and dropped on this thread's stack.
    let full = Rect::new(0.0, 0.0, 320.0, 240.0);
    let mut leaf = DisplayList::new();
    leaf.fill_rect(Rect::new(10.0, 10.0, 100.0, 100.0), Brush::Solid(Color::BLACK));
    let mut layer = Layer::new(LayerId(N as u64 + 1), full, leaf);
    for i in 0..N / 2 {
        let mut parent = Layer::new(LayerId(i as u64 + 1), full, DisplayList::new());
        parent.opacity = if i == N / 2 - 1 { 0.5 } else { 0.999 };
        parent.clip = Some(CornerRadii::ZERO);
        parent.children.push(std::sync::Arc::new(layer));
        layer = parent;
    }
    let mut comp = Compositor::new(320, 240, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(LayerTree::new(vec![layer]));
    let plan = comp.frame(Duration::ZERO);
    let (Ok(c), Ok(g)) = (
        CpuCompositor::new().present(&plan, 320, 240, Color::WHITE),
        GpuCompositor::new(gpu(), LadderOptions::default()).present_to_frame(&plan, 320, 240, Color::WHITE),
    ) else {
        unreachable!("both composite")
    };
    for (name, f) in [("cpu", &c), ("gpu", &g)] {
        let [r, gr, b, a] = pixel(f, 50, 50);
        assert!(r == gr && gr == b && a == 255 && (100..=140).contains(&r), "{name}: drawn ({r})");
    }
    assert_eq!(differing(&c, &g, 2).0, 0, "GPU composite matches CPU");
    let took = started.elapsed();
    eprintln!("{N} nested groups, both renderers, both ways: {took:?}");
    assert!(took < Duration::from_secs(60), "bounded work ({took:?})");
}

/// Stripes and a soft-edged card, run through colour matrices in every way a theme can:
/// plain, chained, nested, over transparency, under a rotation, around a blur and glass.
fn recoloured() -> DisplayList {
    let mut l = DisplayList::new();
    for i in 0..12 {
        let c = Color::rgba8((i * 20) as u8, 200 - (i * 15) as u8, 60 + (i * 10) as u8, 255);
        l.fill_rect(Rect::new(i as f32 * 20.0, 0.0, i as f32 * 20.0 + 20.0, 240.0), Brush::Solid(c));
    }
    l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::grayscale(1.0) });
    l.fill_rounded(
        Rect::new(10.0, 10.0, 110.0, 70.0),
        CornerRadii::all(12.0),
        Brush::Solid(Color::rgb8(220, 40, 30)),
    );
    l.fill_rounded(
        Rect::new(40.0, 30.0, 90.0, 60.0),
        CornerRadii::all(6.0),
        Brush::Solid(Color::rgba8(30, 90, 250, 120)),
    );
    l.pop_filter();
    // Saturate past 1 (out of range, clamped) nested in sepia.
    l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::sepia(0.8) });
    l.fill_rect(Rect::new(130.0, 10.0, 230.0, 70.0), Brush::Solid(Color::rgb8(40, 160, 200)));
    l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::saturate(3.0) });
    l.fill_rect(Rect::new(150.0, 20.0, 210.0, 60.0), Brush::Solid(Color::rgb8(120, 140, 90)));
    l.pop_filter();
    l.pop_filter();
    // Inverted half-transparent content, rotated.
    l.push(Item::PushTransform(
        remedy_ui_engine::paint::Affine::rotate(0.4)
            .then(remedy_ui_engine::paint::Affine::translate(Vec2::new(60.0, 90.0))),
    ));
    l.push_filter(Filter::ColorMatrix {
        matrix: ColorMatrix::invert(1.0).then(&ColorMatrix::hue_rotate(90.0)),
    });
    l.fill_rounded(
        Rect::new(0.0, 0.0, 80.0, 50.0),
        CornerRadii::all(10.0),
        Brush::Solid(Color::rgba8(250, 200, 20, 160)),
    );
    l.pop_filter();
    l.push(Item::PopTransform);
    // A blur inside a matrix, and glass inside one (its backdrop root is the group).
    l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::hue_rotate(200.0) });
    l.push_filter(Filter::Blur { radius: 3.0 });
    l.fill_rect(Rect::new(140.0, 100.0, 200.0, 140.0), Brush::Solid(Color::rgb8(240, 30, 120)));
    l.pop_filter();
    l.fill_rect(Rect::new(20.0, 160.0, 220.0, 230.0), Brush::Solid(Color::rgb8(30, 120, 60)));
    l.push(Item::Backdrop {
        rect: Rect::new(40.0, 170.0, 200.0, 220.0),
        radii: CornerRadii::all(8.0),
        blur: 4.0,
    });
    l.pop_filter();
    // A matrix that adds alpha: only where something was drawn.
    let mut add = ColorMatrix::IDENTITY;
    add.0[19] = 0.5;
    l.push_filter(Filter::ColorMatrix { matrix: add });
    l.fill_rect(Rect::new(200.0, 160.0, 230.0, 230.0), Brush::Solid(Color::rgba8(0, 0, 0, 40)));
    l.pop_filter();
    l
}

#[test]
fn colour_matrices_match_between_gpu_and_cpu() {
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    for scale in [1.0f32, 2.0] {
        let (w, h) = ((240.0 * scale) as u32, (240.0 * scale) as u32);
        let list = recoloured();
        let (Ok(a), Ok(b)) =
            (c.render(&list, w, h, scale, Color::WHITE), g.render(&list, w, h, scale, Color::WHITE))
        else {
            unreachable!("both render")
        };
        let (visible, worst) = differing(&a, &b, 8);
        eprintln!("{scale}x: {visible} of {} differ visibly, worst {worst}", w * h);
        assert!(visible * 1000 <= (w * h) as usize, "{visible} pixels differ visibly at {scale}x");
        assert!(worst <= 32, "no pixel differs grossly at {scale}x (worst {worst})");
        // Inside a flat area the arithmetic itself agrees to a step of rounding.
        let px = |f: &Frame| f.pixel((60.0 * scale) as u32, (20.0 * scale) as u32).unwrap_or([0; 4]);
        let want = ColorMatrix::grayscale(1.0).apply_premultiplied([220, 40, 30, 255]);
        assert_eq!(px(&a), want, "the CPU runs the reference arithmetic");
        assert!(px(&b).iter().zip(want).all(|(p, q)| p.abs_diff(q) <= 1), "{:?} vs {want:?}", px(&b));
    }
    assert!(g.context().take_errors().is_empty(), "no GPU validation errors");
}

#[test]
fn colour_matrices_are_seamless_across_tiles_and_layers() {
    let (w, h) = (600, 520);
    let mut content = DisplayList::new();
    for y in [0.0f32, 250.0] {
        for x in [0.0f32, 245.0] {
            content.push(Item::PushTransform(remedy_ui_engine::paint::Affine::translate(Vec2::new(x, y))));
            content.extend(&recoloured());
            content.push(Item::PopTransform);
        }
    }
    // The same content as a layer's own list, and as a layer the compositor recolours (the
    // path an element with a colour filter over layers of its own takes).
    let inline =
        LayerTree::new(vec![Layer::new(LayerId(1), Rect::new(0.0, 0.0, 600.0, 520.0), content.clone())]);
    let matrix = ColorMatrix::sepia(0.5).then(&ColorMatrix::hue_rotate(-60.0));
    let child = Layer::new(LayerId(3), Rect::new(0.0, 0.0, 600.0, 520.0), content.clone());
    let outer = Layer::new(LayerId(2), Rect::new(0.0, 0.0, 600.0, 520.0), DisplayList::new())
        .with_filters(vec![Filter::ColorMatrix { matrix }])
        .with_child(child);
    let filtered = LayerTree::new(vec![outer]);
    let mut wrapped = DisplayList::new();
    wrapped.push_filter(Filter::ColorMatrix { matrix });
    wrapped.extend(&content);
    wrapped.pop_filter();
    for (tree, list) in [(inline, content.clone()), (filtered, wrapped)] {
        let mut comp = Compositor::new(w, h, 1.0, ScrollPhysics::CLAMPED);
        comp.commit(tree);
        let plan = comp.frame(Duration::ZERO);
        assert!(plan.raster.len() > 1, "the layer is tiled");
        let direct = CpuRenderer::new().render(&list, w, h, 1.0, Color::WHITE);
        let cpu = CpuCompositor::new().present(&plan, w, h, Color::WHITE);
        let mut gc = GpuCompositor::new(gpu(), LadderOptions::default());
        let g = gc.present_to_frame(&plan, w, h, Color::WHITE);
        let (Ok(direct), Ok(cpu), Ok(g)) = (direct, cpu, g) else { unreachable!("all draw") };
        let (n, worst) = differing(&direct, &cpu, 2);
        eprintln!("tiles vs direct (cpu): {n} differ, worst {worst}");
        assert!(
            n * 1000 <= (w * h) as usize,
            "CPU tiles match drawing the whole list at once ({n}, worst {worst})"
        );
        let (visible, worst) = differing(&cpu, &g, 8);
        eprintln!("gpu vs cpu composite: {visible} differ visibly, worst {worst}");
        assert!(visible * 1000 <= (w * h) as usize && worst <= 32);
        assert!(gc.renderer().context().take_errors().is_empty(), "no GPU validation errors");
    }
}

#[test]
fn hostile_colour_matrices_are_bounded_on_both_renderers() {
    // A theme nesting thousands of matrices, and putting one on thousands of siblings: drawn
    // quickly with a bounded number of pictures, the same on both renderers.
    let mut nested = DisplayList::new();
    for _ in 0..2000 {
        nested.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::hue_rotate(7.0) });
    }
    nested.fill_rect(Rect::new(10.0, 10.0, 300.0, 200.0), Brush::Solid(Color::rgb8(200, 60, 20)));
    let mut siblings = DisplayList::new();
    for i in 0..5000 {
        let (x, y) = ((i % 80) as f32 * 4.0, (i / 80) as f32 * 4.0);
        siblings.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::invert(1.0) });
        siblings.fill_rect(Rect::new(x, y, x + 3.0, y + 3.0), Brush::Solid(Color::rgb8(20, 200, 90)));
        siblings.pop_filter();
    }
    let mut wild = DisplayList::new();
    wild.push_filter(Filter::ColorMatrix { matrix: ColorMatrix([f32::NAN; 20]) });
    wild.push_filter(Filter::ColorMatrix { matrix: ColorMatrix([f32::INFINITY; 20]) });
    wild.push_filter(Filter::ColorMatrix { matrix: ColorMatrix([-1.0e30; 20]) });
    wild.fill_rect(Rect::new(10.0, 10.0, 300.0, 200.0), Brush::Solid(Color::rgb8(200, 60, 20)));
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    for (name, list) in [("nested", nested), ("siblings", siblings), ("wild", wild)] {
        let started = std::time::Instant::now();
        let (Ok(a), Ok(b)) =
            (c.render(&list, 320, 240, 1.0, Color::WHITE), g.render(&list, 320, 240, 1.0, Color::WHITE))
        else {
            unreachable!("{name}: both render")
        };
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "{name}: bounded work");
        let (visible, worst) = differing(&a, &b, 8);
        assert!(
            visible * 1000 <= (320 * 240) as usize,
            "{name}: GPU and CPU agree ({visible}, worst {worst})"
        );
    }
    assert!(g.context().take_errors().is_empty());
}

#[test]
#[ignore = "a measurement: cargo test --release -p remedy-ui-engine --features render-gpu --test render_gpu_filters -- --ignored --nocapture"]
fn colour_matrix_cost() {
    // A 512 x 512 tile of cards, drawn plain, with one matrix over a quarter of it, and with
    // one over all of it: what a matrix group adds on each renderer.
    let cards = |l: &mut DisplayList, x0: f32, y0: f32, n: usize| {
        for i in 0..n {
            let (x, y) = (x0 + (i % 8) as f32 * 32.0, y0 + (i / 8) as f32 * 32.0);
            l.fill_rounded(
                Rect::new(x, y, x + 28.0, y + 28.0),
                CornerRadii::all(6.0),
                Brush::Solid(Color::rgb8(200, (i * 3) as u8, 90)),
            );
        }
    };
    let plain = {
        let mut l = DisplayList::new();
        cards(&mut l, 0.0, 0.0, 256);
        l
    };
    let quarter = {
        let mut l = DisplayList::new();
        l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::grayscale(1.0) });
        cards(&mut l, 0.0, 0.0, 64);
        l.pop_filter();
        cards(&mut l, 0.0, 256.0, 128);
        cards(&mut l, 256.0, 0.0, 64);
        l
    };
    let all = {
        let mut l = DisplayList::new();
        l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::hue_rotate(90.0) });
        cards(&mut l, 0.0, 0.0, 256);
        l.pop_filter();
        l
    };
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    let target = g.create_target(512, 512).unwrap_or_else(|_| unreachable!("a target"));
    let base = remedy_ui_engine::paint::Affine::IDENTITY;
    for (name, list) in [("plain", &plain), ("quarter", &quarter), ("all", &all)] {
        let runs = 30;
        let started = std::time::Instant::now();
        for _ in 0..runs {
            let _ = c.render(list, 512, 512, 1.0, Color::WHITE);
        }
        let cpu = started.elapsed() / runs;
        let _ = g.draw(list, base, Color::WHITE, &target);
        let _ = g.read_back(&target);
        let started = std::time::Instant::now();
        for _ in 0..runs {
            let _ = g.draw(list, base, Color::WHITE, &target);
        }
        let _ = g.read_back(&target);
        let gpu = started.elapsed() / runs;
        eprintln!("{name}: cpu {cpu:?} gpu {gpu:?} ({})", g.context().adapter.get_info().name);
    }
}

#[test]
fn shadows_beyond_the_left_or_top_edge_are_not_drawn_unmoved() {
    // vello_cpu shifts a shadow inside the filter's buffer, and a shift past the buffer's left
    // or top edge wrapped and left the shadow under the shape. Such a shadow shows nothing:
    // the frame must equal the shape drawn with no shadow at all, on both renderers, directly
    // and through tiles and filtered layers.
    let shadow =
        |off: Vec2| Filter::DropShadow { offset: off, blur: 0.714, color: Color::rgba8(125, 23, 239, 252) };
    let shape = |l: &mut DisplayList| {
        l.fill_rounded(
            Rect::new(-5.84, -15.488, 28.8195, 30.1691),
            CornerRadii::all(8.16),
            Brush::Solid(Color::rgba8(246, 193, 234, 207)),
        );
        l.fill_rect(Rect::new(40.0, 50.0, 70.0, 90.0), Brush::Solid(Color::rgb8(30, 120, 200)));
    };
    // The shape drawn as a group, as a filter group without its shadow draws.
    let mut plain = DisplayList::new();
    plain.push(Item::PushLayer { opacity: 1.0, blend: remedy_ui_engine::paint::Blend::Normal });
    shape(&mut plain);
    plain.push(Item::PopLayer);
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    let (Ok(want), Ok(want_gpu)) =
        (c.render(&plain, 96, 96, 1.0, Color::WHITE), g.render(&plain, 96, 96, 1.0, Color::WHITE))
    else {
        unreachable!("both render")
    };
    for off in [
        Vec2::new(-1.0e9, -4.622),
        Vec2::new(-200.0, 0.0),
        Vec2::new(-100.0, 3.0),
        Vec2::new(0.0, -150.0),
        Vec2::new(-120.0, -120.0),
        // Past the right and bottom: vello_gpu sized buffers to reach these.
        Vec2::new(1.0e9, 0.0),
        Vec2::new(150.0, 150.0),
    ] {
        for (closed, layer) in [(true, false), (false, false), (false, true)] {
            let mut l = DisplayList::new();
            l.push(Item::PushFilter(shadow(off)));
            if layer {
                l.push(Item::PushLayer { opacity: 0.0531, blend: remedy_ui_engine::paint::Blend::Normal });
                l.push(Item::PopLayer);
            }
            shape(&mut l);
            if closed {
                l.pop_filter();
            }
            let (Ok(a), Ok(b)) =
                (c.render(&l, 96, 96, 1.0, Color::WHITE), g.render(&l, 96, 96, 1.0, Color::WHITE))
            else {
                unreachable!("both render")
            };
            assert_eq!(differing(&a, &want, 0).0, 0, "{off:?}: the CPU draws no shadow");
            let (n, worst) = differing(&b, &want_gpu, 2);
            assert_eq!(n, 0, "{off:?}: the GPU draws no shadow (worst {worst})");
        }
    }
    // The same through the compositor: the shadow on a layer's list, and on a filtered layer.
    for as_layer in [false, true] {
        let off = Vec2::new(-150.0, -2.0);
        let tree = if as_layer {
            let content = Layer::new(LayerId(3), Rect::new(0.0, 0.0, 96.0, 96.0), plain.clone());
            LayerTree::new(vec![
                Layer::new(LayerId(2), Rect::new(0.0, 0.0, 96.0, 96.0), DisplayList::new())
                    .with_filters(vec![shadow(off)])
                    .with_child(content),
            ])
        } else {
            let mut l = DisplayList::new();
            l.push_filter(shadow(off));
            shape(&mut l);
            l.pop_filter();
            LayerTree::new(vec![Layer::new(LayerId(1), Rect::new(0.0, 0.0, 96.0, 96.0), l)])
        };
        let mut comp = Compositor::new(96, 96, 1.0, ScrollPhysics::CLAMPED);
        comp.commit(tree);
        let plan = comp.frame(Duration::ZERO);
        let mut gc = GpuCompositor::new(gpu(), LadderOptions::default());
        let (Ok(a), Ok(b)) = (
            CpuCompositor::new().present(&plan, 96, 96, Color::WHITE),
            gc.present_to_frame(&plan, 96, 96, Color::WHITE),
        ) else {
            unreachable!("both composite")
        };
        assert_eq!(differing(&a, &want, 2).0, 0, "layer {as_layer}: the CPU composite draws no shadow");
        assert_eq!(differing(&b, &want_gpu, 2).0, 0, "layer {as_layer}: the GPU composite draws no shadow");
    }
    // Shadows moved far off the target, nested: the GPU asked for a texture past the device's
    // limit and failed the whole frame (inside a colour-matrix group, the group vanished).
    for matrix in [false, true] {
        let mut l = DisplayList::new();
        if matrix {
            l.push_filter(Filter::ColorMatrix { matrix: ColorMatrix::invert(0.7) });
        }
        l.push_filter(shadow(Vec2::new(1.0e9, 0.0)));
        l.push_filter(shadow(Vec2::new(1.0e9, 0.0)));
        shape(&mut l);
        let (Ok(a), Ok(b)) =
            (c.render(&l, 96, 96, 1.0, Color::WHITE), g.render(&l, 96, 96, 1.0, Color::WHITE))
        else {
            unreachable!("matrix {matrix}: both render")
        };
        let (n, worst) = differing(&a, &b, 8);
        assert_eq!(n, 0, "matrix {matrix}: the renderers agree (worst {worst})");
        assert!(a.pixel(50, 70).is_some_and(|p| p != [255; 4]), "matrix {matrix}: the shape is drawn");
    }
    assert!(g.context().take_errors().is_empty());
}

#[test]
fn stacked_glass_costs_a_bounded_amount_on_both_renderers() {
    // Each pane draws again what is under it, glass included: 16 stacked panes took 40 s on
    // the CPU, doubling with each pane. Now glass seen through more than MAX_BACKDROP_DEPTH
    // panes, or past MAX_BACKDROP_REPLAYS panes in a raster, goes without its frost.
    let mut l = DisplayList::new();
    for i in 0..8 {
        let c = if i % 2 == 0 { Color::rgb8(220, 60, 40) } else { Color::rgb8(30, 90, 200) };
        l.fill_rect(Rect::new(i as f32 * 12.0, 0.0, i as f32 * 12.0 + 12.0, 96.0), Brush::Solid(c));
    }
    for k in 0..40 {
        let o = (k % 10) as f32;
        l.push(Item::Backdrop {
            rect: Rect::new(4.0 + o, 4.0 + o, 80.0 + o, 80.0),
            radii: CornerRadii::all(6.0),
            blur: 3.0,
        });
        l.fill_rounded(
            Rect::new(4.0 + o, 4.0 + o, 80.0 + o, 80.0),
            CornerRadii::all(6.0),
            Brush::Solid(Color::rgba8(255, 255, 255, 12)),
        );
    }
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    let started = std::time::Instant::now();
    let (Ok(a), Ok(b)) = (c.render(&l, 96, 96, 1.0, Color::WHITE), g.render(&l, 96, 96, 1.0, Color::WHITE))
    else {
        unreachable!("both render")
    };
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "bounded: {:?}", started.elapsed());
    // Forty blurs stacked add up the renderers' small per-blur differences: held to the
    // random-list test's tolerance.
    let (visible, worst) = differing(&a, &b, 24);
    assert!(visible * 50 <= 96 * 96, "the renderers agree ({visible}, worst {worst})");
    // The same panes as glass layers the compositor draws.
    let mut back = DisplayList::new();
    for i in 0..8 {
        let c = if i % 2 == 0 { Color::rgb8(220, 60, 40) } else { Color::rgb8(30, 90, 200) };
        back.fill_rect(Rect::new(i as f32 * 12.0, 0.0, i as f32 * 12.0 + 12.0, 96.0), Brush::Solid(c));
    }
    let mut root = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 96.0, 96.0), back);
    for k in 0..40u64 {
        let o = (k % 10) as f32;
        let mut pane = DisplayList::new();
        pane.fill_rounded(
            Rect::new(0.0, 0.0, 76.0, 76.0 - o),
            CornerRadii::all(6.0),
            Brush::Solid(Color::rgba8(255, 255, 255, 12)),
        );
        root = root.with_child(
            Layer::new(LayerId(10 + k), Rect::new(4.0 + o, 4.0 + o, 80.0 + o, 80.0), pane)
                .with_backdrop(3.0, CornerRadii::all(6.0)),
        );
    }
    let mut comp = Compositor::new(96, 96, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(LayerTree::new(vec![root]));
    let plan = comp.frame(Duration::ZERO);
    let started = std::time::Instant::now();
    let mut gc = GpuCompositor::new(gpu(), LadderOptions::default());
    let (Ok(a), Ok(b)) = (
        CpuCompositor::new().present(&plan, 96, 96, Color::WHITE),
        gc.present_to_frame(&plan, 96, 96, Color::WHITE),
    ) else {
        unreachable!("both composite")
    };
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "bounded: {:?}", started.elapsed());
    let (visible, worst) = differing(&a, &b, 24);
    assert!(visible * 50 <= 96 * 96, "the composites agree ({visible}, worst {worst})");
    assert!(g.context().take_errors().is_empty());
}
