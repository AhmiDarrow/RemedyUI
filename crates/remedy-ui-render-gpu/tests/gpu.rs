//! GPU golden images against the CPU reference, composite equivalence, device-loss recovery,
//! the backend ladder and the pipeline cache.

use std::sync::Arc;
use std::time::Duration;

use remedy_ui_compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_paint::{Blend, Brush, DisplayList, GlyphRun, Image, Item, Path, Sampling, Stop, Stroke};
use remedy_ui_render_cpu::{CpuCompositor, CpuRenderer, Frame};
use remedy_ui_render_gpu::*;
use remedy_ui_text::{Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle};

fn gpu() -> GpuRenderer {
    match GpuContext::new(&LadderOptions::default()) {
        Ok(ctx) => GpuRenderer::new(ctx, None),
        Err(e) => unreachable!("every machine has at least the software rung: {e}"),
    }
}

fn scene() -> DisplayList {
    let mut l = DisplayList::new();
    l.fill_rect(Rect::new(4.0, 4.0, 60.0, 40.0), Brush::Solid(Color::rgb8(220, 40, 40)));
    l.fill_rounded(
        Rect::new(70.0, 4.0, 150.0, 40.0),
        CornerRadii::all(10.0),
        Brush::Solid(Color::rgb8(30, 120, 60)),
    );
    l.border(Rect::new(160.0, 4.0, 230.0, 40.0), CornerRadii::all(6.0), 3.0, Color::BLACK);
    let stops = vec![
        Stop { offset: 0.0, color: Color::BLACK },
        Stop { offset: 1.0, color: Color::rgb8(80, 160, 255) },
    ];
    l.fill_rect(
        Rect::new(4.0, 50.0, 230.0, 70.0),
        Brush::Linear { start: Point::new(4.0, 0.0), end: Point::new(230.0, 0.0), stops },
    );
    l.push(Item::Shadow {
        rect: Rect::new(20.0, 90.0, 80.0, 130.0),
        radii: CornerRadii::all(8.0),
        offset: Vec2::new(0.0, 4.0),
        blur: 10.0,
        spread: 0.0,
        color: Color::rgba8(0, 0, 0, 120),
    });
    l.push(Item::FillPath {
        path: Path::circle(Point::new(140.0, 110.0), 22.0),
        brush: Brush::Solid(Color::rgb8(200, 150, 0)),
        even_odd: false,
    });
    let mut line = Path::new();
    line.move_to(Point::new(180.0, 90.0)).line_to(Point::new(225.0, 135.0));
    l.push(Item::StrokePath { path: line, stroke: Stroke::new(3.0), brush: Brush::Solid(Color::BLACK) });
    l.push_clip(Rect::new(4.0, 140.0, 60.0, 170.0), CornerRadii::all(4.0));
    l.push(Item::PushLayer { opacity: 0.5, blend: Blend::Normal });
    l.fill_rect(Rect::new(0.0, 130.0, 100.0, 180.0), Brush::Solid(Color::rgb8(0, 0, 200)));
    l.push(Item::PopLayer);
    l.pop_clip();
    let pixels: Arc<[u8]> =
        Arc::from(vec![255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 255, 255, 255, 255, 255]);
    l.push(Item::Image {
        image: Image { id: 1, width: 2, height: 2, pixels },
        rect: Rect::new(80.0, 140.0, 120.0, 180.0),
        radii: CornerRadii::ZERO,
        sampling: Sampling::Nearest,
    });
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let p = engine.layout(&RichText::plain("Remedy GPU"), &TextStyle::default(), ParagraphOptions::default());
    for run in p.glyph_runs() {
        l.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color: Color::BLACK,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(g.x + 130.0, g.y + 145.0))).collect(),
            hinting: Hinting::Full,
            antialias: Antialias::Grayscale,
        }));
    }
    l
}

/// Pixels differing by more than `tol` on any channel.
fn differing(a: &Frame, b: &Frame, tol: u8) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > tol))
        .count()
}

#[test]
fn gpu_golden_images_match_the_cpu_reference() {
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    for scale in [1.0, 2.0] {
        let (w, h) = ((240.0 * scale) as u32, (190.0 * scale) as u32);
        let list = scene();
        let cpu = c.render(&list, w, h, scale, Color::WHITE);
        let gpu = g.render(&list, w, h, scale, Color::WHITE);
        let (Ok(cpu), Ok(gpu)) = (cpu, gpu) else { unreachable!("both render") };
        let total = (w * h) as usize;
        // Rasterizers differ by rounding along antialiased edges; the bar is that almost no
        // pixel differs visibly and none differs grossly.
        let visible = differing(&cpu, &gpu, 8);
        eprintln!(
            "{}: {visible} of {total} pixels differ visibly at {scale}x, {} at all",
            g.context().describe(),
            differing(&cpu, &gpu, 0)
        );
        assert!(
            visible * 1000 <= total,
            "{visible} of {total} pixels differ visibly at {scale}x on {}",
            g.context().describe()
        );
        assert_eq!(differing(&cpu, &gpu, 64), 0, "no pixel differs grossly at {scale}x");
    }
}

fn page() -> LayerTree {
    let content = Layer::new(LayerId(2), Rect::new(0.0, 0.0, 240.0, 900.0), {
        let mut l = DisplayList::new();
        for i in 0..5 {
            l.push(Item::PushTransform(remedy_ui_paint::Affine::translate(Vec2::new(0.0, i as f32 * 180.0))));
            l.extend(&scene());
            l.push(Item::PopTransform);
        }
        l
    });
    let scroller = Layer::new(LayerId(1), Rect::new(10.0, 10.0, 250.0, 290.0), DisplayList::new())
        .scrolling(Size::new(240.0, 900.0))
        .with_child(content);
    LayerTree::new(vec![scroller])
}

#[test]
fn gpu_composite_matches_cpu_composite() {
    let (w, h) = (260, 300);
    let mut comp = Compositor::new(w, h, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(page());
    let mut cpu = CpuCompositor::new();
    let mut g = GpuCompositor::new(gpu(), LadderOptions::default());
    for (t, scroll) in [(0, 0.0), (16, 333.0)] {
        comp.scroll_by(LayerId(1), Vec2::new(0.0, scroll));
        let plan = comp.frame(Duration::from_millis(t));
        let a = cpu.present(&plan, w, h, Color::WHITE);
        let b = g.present_to_frame(&plan, w, h, Color::WHITE);
        let (Ok(a), Ok(b)) = (a, b) else { unreachable!("both present") };
        let visible = differing(&a, &b, 8);
        assert!(
            visible * 1000 <= (w * h) as usize,
            "{visible} pixels differ visibly after scrolling {scroll}"
        );
        assert_eq!(differing(&a, &b, 64), 0);
    }
    assert!(g.tile_count() > 0);
}

#[test]
fn a_forced_device_loss_recovers_with_no_app_code() {
    let (w, h) = (260, 300);
    let mut comp = Compositor::new(w, h, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(page());
    let mut g = GpuCompositor::new(gpu(), LadderOptions::default());
    let plan = comp.frame(Duration::from_millis(0));
    let before = g.present_to_frame(&plan, w, h, Color::WHITE);
    assert!(before.is_ok());

    g.renderer().context().force_loss();
    assert!(g.is_lost());
    let plan = comp.frame(Duration::from_millis(16));
    assert_eq!(g.present_to_frame(&plan, w, h, Color::WHITE).err(), Some(GpuError::Lost));

    // The window's frame loop: recover, then plan again. The layer tree the app committed is
    // still the compositor's; nothing is asked of the app.
    assert!(g.recover(&mut comp).is_ok());
    assert!(!g.is_lost());
    let plan = comp.frame(Duration::from_millis(32));
    assert!(!plan.raster.is_empty(), "every visible tile rasters again");
    assert!(!plan.damage.is_empty(), "the whole window redraws");
    let after = g.present_to_frame(&plan, w, h, Color::WHITE);
    assert_eq!(after.ok(), before.ok(), "the same pixels as before the loss");
}

#[test]
fn the_ladder_reaches_software_and_reports_its_rung() {
    let only_software = LadderOptions { high_performance: false, rungs: vec![Rung::Software] };
    match GpuContext::new(&only_software) {
        Ok(ctx) => {
            assert_eq!(ctx.rung, Rung::Software);
            let mut r = GpuRenderer::new(ctx, None);
            let f = r.render(&scene(), 240, 190, 1.0, Color::WHITE);
            assert!(f.is_ok_and(|f| f.pixel(10, 10) == Some([220, 40, 40, 255])));
        }
        // Some systems ship no software adapter; then the hardware rungs must work instead.
        Err(e) => assert!(GpuContext::new(&LadderOptions::default()).is_ok(), "no rung works: {e}"),
    }
    let none = LadderOptions { high_performance: true, rungs: Vec::new() };
    assert_eq!(GpuContext::new(&none).err(), Some(GpuError::NoAdapter));
}

#[test]
fn the_pipeline_cache_round_trips_where_the_driver_supports_one() {
    let dir = std::env::temp_dir().join(format!("remedy-ui-pipelines-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ctx = match GpuContext::new(&LadderOptions::default()) {
        Ok(c) => c,
        Err(e) => unreachable!("{e}"),
    };
    let supported = ctx.device.features().contains(wgpu::Features::PIPELINE_CACHE);
    let store = PipelineCacheStore::new(&dir);
    let r = GpuRenderer::new(ctx.clone(), Some(store.clone()));
    let saved = r.save_pipeline_cache();
    assert!(saved.is_ok());
    assert_eq!(saved.ok(), Some(supported), "a file exactly when the driver has a cache");
    if supported {
        let files: Vec<_> = std::fs::read_dir(&dir).into_iter().flatten().flatten().collect();
        assert_eq!(files.len(), 1);
        // A corrupted file is ignored, not trusted.
        if let Some(f) = files.first() {
            let _ = std::fs::write(f.path(), b"RUIPIPE1garbage-not-a-cache");
        }
        assert!(store.open(&ctx).is_some(), "falls back to an empty cache");
    } else {
        assert!(store.open(&ctx).is_none());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lcd_text_matches_between_gpu_and_cpu() {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let p =
        engine.layout(&RichText::plain("Subpixel text"), &TextStyle::default(), ParagraphOptions::default());
    let mut l = DisplayList::new();
    l.fill_rect(Rect::new(0.0, 0.0, 160.0, 40.0), Brush::Solid(Color::rgb8(240, 236, 220)));
    for run in p.glyph_runs() {
        l.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color: Color::rgb8(20, 40, 90),
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(g.x + 4.0, g.y + 6.0))).collect(),
            hinting: Hinting::Full,
            antialias: Antialias::Lcd { bgr: false },
        }));
    }
    let cpu = CpuRenderer::new().render(&l, 160, 40, 1.0, Color::WHITE);
    let gpu = gpu().render(&l, 160, 40, 1.0, Color::WHITE);
    let (Ok(cpu), Ok(gpu)) = (cpu, gpu) else { unreachable!("both render") };
    assert!(
        cpu.pixels.as_chunks::<4>().0.iter().any(|p| p[0].abs_diff(p[2]) > 30),
        "the reference has fringes"
    );
    assert!(
        cpu.pixels.as_chunks::<4>().0.iter().any(|p| p[2] > p[0].saturating_add(30)),
        "the text color shows: blue over beige"
    );
    assert_eq!(differing(&cpu, &gpu, 8), 0, "LCD text matches on the GPU");
}

#[test]
fn no_pipeline_or_shader_is_created_after_start_up() {
    let mut r = gpu();
    let count = |r: &GpuRenderer| {
        let c = r.context().device.get_internal_counters();
        (c.hal.render_pipelines.read(), c.hal.compute_pipelines.read(), c.hal.shader_modules.read())
    };
    let at_start = count(&r);
    // Every kind of content, at several sizes and scales, including LCD text and layers.
    for (i, scale) in [1.0, 1.5, 2.0, 1.0, 3.0].iter().enumerate() {
        let w = 120 + 40 * i as u32;
        let f = r.render(&scene(), w, w, *scale, Color::WHITE);
        assert!(f.is_ok());
    }
    let comp_plan = {
        let mut c = Compositor::new(260, 300, 1.0, ScrollPhysics::CLAMPED);
        c.commit(page());
        c.frame(Duration::from_millis(0))
    };
    let ctx = r.context().clone();
    let mut g = GpuCompositor::new(r, LadderOptions::default());
    assert!(g.present_to_frame(&comp_plan, 260, 300, Color::WHITE).is_ok());
    let c = ctx.device.get_internal_counters();
    let after = (c.hal.render_pipelines.read(), c.hal.compute_pipelines.read(), c.hal.shader_modules.read());
    assert!(at_start.0 > 0, "counters are live: {at_start:?}");
    assert_eq!(after, at_start, "(render pipelines, compute pipelines, shader modules)");
}
