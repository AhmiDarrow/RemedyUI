//! The compositor path (layers, tiles, scroll, opacity) draws what direct rendering draws.

use std::time::Duration;

use remedy_ui_compositor::{Compositor, Layer, LayerId, LayerTree};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_paint::{Affine, Blend, Brush, DisplayList, GlyphRun, Item};
use remedy_ui_render_cpu::{CpuCompositor, CpuRenderer, Frame};
use remedy_ui_text::{Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle};

fn content() -> DisplayList {
    let mut l = DisplayList::new();
    for i in 0..40 {
        let y = i as f32 * 25.0;
        let c = Color::rgb8((i * 6) as u8, 120, 255 - (i * 6) as u8);
        l.fill_rounded(Rect::new(8.0, y + 2.0, 292.0, y + 22.0), CornerRadii::all(6.0), Brush::Solid(c));
    }
    // A shape crossing tile boundaries at an angle.
    let mut p = remedy_ui_paint::Path::new();
    p.move_to(Point::new(20.0, 200.0))
        .line_to(Point::new(280.0, 300.0))
        .line_to(Point::new(40.0, 600.0))
        .close();
    l.push(Item::FillPath { path: p, brush: Brush::Solid(Color::rgba8(0, 0, 0, 200)), even_odd: false });
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let para =
        engine.layout(&RichText::plain("Scrolled text"), &TextStyle::default(), ParagraphOptions::default());
    for run in para.glyph_runs() {
        l.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color: Color::BLACK,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(g.x + 10.0, g.y + 250.0))).collect(),
            hinting: Hinting::Full,
            antialias: Antialias::Grayscale,
        }));
    }
    l
}

fn tree(opacity: f32) -> LayerTree {
    let mut page = Layer::new(LayerId(2), Rect::new(0.0, 0.0, 300.0, 1000.0), content());
    page.opacity = opacity;
    let scroller = Layer::new(LayerId(1), Rect::new(20.0, 10.0, 320.0, 210.0), DisplayList::new())
        .scrolling(Size::new(300.0, 1000.0))
        .with_child(page);
    LayerTree::new(vec![scroller])
}

fn direct(scroll: f32, opacity: f32, w: u32, h: u32, scale: f32) -> Frame {
    let mut l = DisplayList::new();
    l.push_clip(Rect::new(20.0, 10.0, 320.0, 210.0), CornerRadii::ZERO);
    l.push(Item::PushTransform(Affine::translate(Vec2::new(20.0, 10.0 - scroll))));
    if opacity < 1.0 {
        l.push(Item::PushLayer { opacity, blend: Blend::Normal });
    }
    l.extend(&content());
    if opacity < 1.0 {
        l.push(Item::PopLayer);
    }
    l.push(Item::PopTransform);
    l.pop_clip();
    match CpuRenderer::new().render(&l, w, h, scale, Color::WHITE) {
        Ok(f) => f,
        Err(e) => unreachable!("{e}"),
    }
}

fn mismatches(a: &Frame, b: &Frame, tol: u8) -> usize {
    a.pixels.iter().zip(b.pixels.iter()).filter(|(x, y)| x.abs_diff(**y) > tol).count()
}

fn composited(c: &mut Compositor, cpu: &mut CpuCompositor, t: u64) -> Frame {
    let plan = c.frame(Duration::from_millis(t));
    let (w, h) = c.size();
    match cpu.present(&plan, w, h, Color::WHITE) {
        Ok(f) => f,
        Err(e) => unreachable!("{e}"),
    }
}

#[test]
fn composited_scroll_matches_direct_rendering() {
    for scale in [1.0, 2.0] {
        let (w, h) = ((340.0 * scale) as u32, (230.0 * scale) as u32);
        let mut c = Compositor::new(w, h, scale, ScrollPhysics::CLAMPED);
        let mut cpu = CpuCompositor::new();
        c.commit(tree(1.0));
        let f0 = composited(&mut c, &mut cpu, 0);
        assert_eq!(mismatches(&f0, &direct(0.0, 1.0, w, h, scale), 1), 0, "unscrolled at {scale}x");
        c.scroll_by(LayerId(1), Vec2::new(0.0, 237.0));
        let f1 = composited(&mut c, &mut cpu, 16);
        assert_eq!(
            mismatches(&f1, &direct(237.0, 1.0, w, h, scale), 1),
            0,
            "scrolled across tile rows at {scale}x"
        );
    }
}

#[test]
fn composited_opacity_matches_direct_rendering() {
    let mut c = Compositor::new(340, 230, 1.0, ScrollPhysics::CLAMPED);
    let mut cpu = CpuCompositor::new();
    c.commit(tree(0.6));
    let f = composited(&mut c, &mut cpu, 0);
    assert_eq!(mismatches(&f, &direct(0.0, 0.6, 340, 230, 1.0), 2), 0);
}

#[test]
fn unchanged_frames_reuse_tiles_and_released_tiles_are_freed() {
    let mut c = Compositor::new(340, 230, 1.0, ScrollPhysics::CLAMPED);
    let mut cpu = CpuCompositor::new();
    c.commit(tree(1.0));
    let a = composited(&mut c, &mut cpu, 0);
    let held = cpu.tile_count();
    assert!(held > 0 && cpu.tile_bytes() > 0);
    let plan = c.frame(Duration::from_millis(16));
    assert!(plan.raster.is_empty() && plan.damage.is_empty());
    let b = cpu.present(&plan, 340, 230, Color::WHITE);
    assert_eq!(b.as_ref().ok(), Some(&a), "the same pixels from kept tiles");
    c.commit(LayerTree::default());
    let _ = composited(&mut c, &mut cpu, 32);
    assert_eq!(cpu.tile_count(), 0, "every tile of the removed layers is freed");
}

/// A tree with a big patterned backdrop and a small square at `x`.
fn moving(x: f32) -> LayerTree {
    let mut back = DisplayList::new();
    for i in 0..20 {
        let c = Color::rgb8((i * 12) as u8, 80, 200 - (i * 9) as u8);
        back.fill_rect(Rect::new(i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 120.0), Brush::Solid(c));
    }
    let mut dot = DisplayList::new();
    dot.fill_rounded(Rect::new(0.0, 0.0, 16.0, 16.0), CornerRadii::all(8.0), Brush::Solid(Color::WHITE));
    let root = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 200.0, 120.0), back).with_child(Layer::new(
        LayerId(2),
        Rect::new(x, 40.0, x + 16.0, 56.0),
        dot,
    ));
    LayerTree::new(vec![root])
}

#[test]
fn composites_only_the_damage_and_matches_a_full_frame() {
    let mut comp = Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED);
    let mut cpu = CpuCompositor::new();
    for (k, x) in [10.0f32, 30.5, 61.0, 140.25, 141.0].into_iter().enumerate() {
        comp.commit(moving(x));
        let plan = comp.frame(Duration::from_millis(16 * k as u64));
        let partial = cpu.present(&plan, 200, 120, Color::BLACK).unwrap_or_else(|e| unreachable!("{e:?}"));
        if k > 0 {
            assert!(plan.damage.area() < 200.0 * 120.0 * 0.3, "a small move damages a little");
        }
        // The same tree drawn whole from scratch.
        let mut fresh_comp = Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED);
        fresh_comp.commit(moving(x));
        let full_plan = fresh_comp.frame(Duration::ZERO);
        let full = CpuCompositor::new()
            .present(&full_plan, 200, 120, Color::BLACK)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let worst = partial.pixels.iter().zip(&full.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(worst <= 1, "frame {k}: kept pixels plus redrawn damage equal a full frame (worst {worst})");
    }
}
