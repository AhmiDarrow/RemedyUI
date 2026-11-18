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
    let only_software = {
        let mut v_ = LadderOptions::default();
        v_.high_performance = false;
        v_.rungs = vec![Rung::Software];
        v_
    };
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
    let none = {
        let mut v_ = LadderOptions::default();
        v_.high_performance = true;
        v_.rungs = Vec::new();
        v_
    };
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

#[test]
fn uploads_of_images_the_app_drops_are_freed() {
    let mut g = gpu();
    // A filtered image large enough for a texture of its own, and a small nearest-sampled
    // one that goes on an atlas page.
    let big = Image { id: 9_001, width: 300, height: 200, pixels: Arc::from(vec![200u8; 300 * 200 * 4]) };
    let small = Image { id: 9_002, width: 8, height: 8, pixels: Arc::from(vec![90u8; 8 * 8 * 4]) };
    let mut list = DisplayList::new();
    list.push(Item::Image {
        image: big.clone(),
        rect: Rect::new(0.0, 0.0, 60.0, 40.0),
        radii: CornerRadii::ZERO,
        sampling: Sampling::Linear,
    });
    list.push(Item::Image {
        image: small.clone(),
        rect: Rect::new(60.0, 0.0, 68.0, 8.0),
        radii: CornerRadii::ZERO,
        sampling: Sampling::Nearest,
    });
    let _ = g.render(&list, 80, 40, 1.0, Color::WHITE);
    let with_both = g.image_bytes();
    assert!(with_both >= 300 * 200 * 4, "{with_both}");
    drop(list);
    drop(big);
    let _ = g.render(&DisplayList::new(), 80, 40, 1.0, Color::WHITE);
    assert_eq!(g.image_bytes(), with_both - 300 * 200 * 4, "the big texture is freed");
    // The page stays allocated while the small image lives, and is reused once it goes.
    drop(small);
    let _ = g.render(&DisplayList::new(), 80, 40, 1.0, Color::WHITE);
    let again = Image { id: 9_003, width: 8, height: 8, pixels: Arc::from(vec![10u8; 8 * 8 * 4]) };
    let mut list = DisplayList::new();
    list.push(Item::Image {
        image: again,
        rect: Rect::new(0.0, 0.0, 8.0, 8.0),
        radii: CornerRadii::ZERO,
        sampling: Sampling::Nearest,
    });
    let _ = g.render(&list, 80, 40, 1.0, Color::WHITE);
    assert_eq!(g.image_bytes(), with_both - 300 * 200 * 4, "no new page: the empty one is reused");
}

/// A 64 × 48 frame: a horizontal ramp of luma with four colored quarters of chroma.
fn video_frame(stream: u64, id: u64, format: remedy_ui_paint::PixelFormat) -> remedy_ui_paint::VideoFrame {
    use remedy_ui_paint::{PixelFormat, VideoFrame, YuvMatrix};
    let (w, h) = (64u32, 48u32);
    let (cw, ch) = (w / 2, h / 2);
    let mut data: Vec<u8> = (0..h).flat_map(|_| (0..w).map(|x| (16 + x * 3) as u8)).collect();
    let chroma = |x: u32, y: u32| -> (u8, u8) {
        match (x < cw / 2, y < ch / 2) {
            (true, true) => (90, 240),
            (false, true) => (54, 34),
            (true, false) => (240, 110),
            (false, false) => (128, 128),
        }
    };
    match format {
        PixelFormat::Nv12 => {
            for y in 0..ch {
                for x in 0..cw {
                    let (u, v) = chroma(x, y);
                    data.extend([u, v]);
                }
            }
        }
        PixelFormat::I420 => {
            for plane in 0..2 {
                for y in 0..ch {
                    for x in 0..cw {
                        let (u, v) = chroma(x, y);
                        data.push(if plane == 0 { u } else { v });
                    }
                }
            }
        }
        PixelFormat::Rgba => {
            data = (0..w * h).flat_map(|i| [(i % 256) as u8, 40, 200, 255]).collect();
        }
    }
    let mut f = VideoFrame::packed(stream, id, w, h, format, data.into());
    f.matrix = YuvMatrix::Bt709;
    f
}

#[test]
fn video_frames_convert_on_the_gpu_like_the_reference() {
    use remedy_ui_paint::PixelFormat;
    let mut g = gpu();
    let mut c = CpuRenderer::new();
    for (stream, format) in [(1, PixelFormat::Nv12), (2, PixelFormat::I420), (3, PixelFormat::Rgba)] {
        let frame = video_frame(stream, 1, format);
        let mut list = DisplayList::new();
        // Drawn at its own size, so every pixel is one sample of the frame.
        list.push(Item::Video {
            frame: frame.clone(),
            rect: Rect::new(0.0, 0.0, 64.0, 48.0),
            radii: CornerRadii::ZERO,
        });
        let (Ok(cpu), Ok(gpu)) =
            (c.render(&list, 64, 48, 1.0, Color::WHITE), g.render(&list, 64, 48, 1.0, Color::WHITE))
        else {
            unreachable!("both render")
        };
        let reference = frame.to_rgba().unwrap_or_default();
        let as_frame = Frame { width: 64, height: 48, pixels: reference };
        assert_eq!(differing(&cpu, &as_frame, 2), 0, "{format:?}: the CPU renderer shows the conversion");
        assert_eq!(differing(&gpu, &as_frame, 2), 0, "{format:?}: the GPU converts to the same colors");
    }
}

#[test]
fn a_streams_texture_is_reused_per_frame_and_freed_with_its_frames() {
    use remedy_ui_paint::PixelFormat;
    let mut g = gpu();
    let before = g.image_bytes();
    let draw = |g: &mut GpuRenderer, f: &remedy_ui_paint::VideoFrame| {
        let mut list = DisplayList::new();
        list.push(Item::Video {
            frame: f.clone(),
            rect: Rect::new(0.0, 0.0, 64.0, 48.0),
            radii: CornerRadii::ZERO,
        });
        g.render(&list, 64, 48, 1.0, Color::BLACK)
    };
    let f1 = video_frame(7, 1, PixelFormat::Nv12);
    let first = draw(&mut g, &f1);
    let held = g.image_bytes();
    assert!(held > before);
    // Next frame of the same stream: the same textures, new colors.
    let mut f2 = video_frame(7, 2, PixelFormat::Nv12);
    if let Some(y) = std::sync::Arc::get_mut(&mut f2.data) {
        y.iter_mut().take(64 * 48).for_each(|b| *b = 235);
    }
    let second = draw(&mut g, &f2);
    assert_eq!(g.image_bytes(), held, "no new textures for a new frame");
    let (Ok(a), Ok(b)) = (first, second) else { unreachable!("both draw") };
    assert!(differing(&a, &b, 8) > 0, "the new frame shows");
    // When the stream's frames are gone, so are its textures.
    drop(f1);
    drop(f2);
    let _ = g.render(&DisplayList::new(), 64, 48, 1.0, Color::BLACK);
    assert_eq!(g.image_bytes(), before);
}
