//! The compositor path (layers, tiles, scroll, opacity) draws what direct rendering draws.

use std::sync::Arc;
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

/// A tree with frosted glass the square passes under.
fn under_glass(x: f32) -> LayerTree {
    let mut tint = DisplayList::new();
    tint.fill_rounded(
        Rect::new(0.0, 0.0, 80.0, 50.0),
        CornerRadii::all(10.0),
        Brush::Solid(Color::rgba8(255, 255, 255, 40)),
    );
    let glass = Layer::new(LayerId(3), Rect::new(70.0, 30.0, 150.0, 80.0), tint)
        .with_backdrop(4.0, CornerRadii::all(10.0));
    let mut t = moving(x);
    if let Some(root) = t.roots.first_mut().and_then(Arc::get_mut) {
        root.children.push(Arc::new(glass));
    }
    t
}

#[test]
fn damage_under_glass_matches_a_full_frame_exactly() {
    let mut comp = Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED);
    let mut cpu = CpuCompositor::new();
    for (k, x) in [10.0f32, 40.5, 61.0, 90.25, 120.0, 170.0].into_iter().enumerate() {
        comp.commit(under_glass(x));
        let plan = comp.frame(Duration::from_millis(16 * k as u64));
        let partial = cpu.present(&plan, 200, 120, Color::BLACK).unwrap_or_else(|e| unreachable!("{e:?}"));
        if k > 0 {
            assert!(cpu.last_composited() < 200 * 120, "frame {k} composites only its damage");
        }
        let mut fresh = Compositor::new(200, 120, 1.0, ScrollPhysics::CLAMPED);
        fresh.commit(under_glass(x));
        let full = CpuCompositor::new()
            .present(&fresh.frame(Duration::ZERO), 200, 120, Color::BLACK)
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        let worst = partial.pixels.iter().zip(&full.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert_eq!(worst, 0, "frame {k}: the glass over damage equals a full frame");
    }
}

#[test]
fn a_hole_clears_its_box_and_what_draws_after_it_draws_over() {
    // Page, a hole where a page's video shows, then a control over the hole.
    let page = Layer::new(LayerId(1), Rect::new(0.0, 0.0, 100.0, 100.0), {
        let mut l = DisplayList::new();
        l.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Brush::Solid(Color::BLACK));
        l
    });
    let mut hole = Layer::new(LayerId(2), Rect::new(20.0, 20.0, 80.0, 80.0), DisplayList::new());
    hole.hole = true;
    let control = Layer::new(LayerId(3), Rect::new(40.0, 40.0, 60.0, 60.0), {
        let mut l = DisplayList::new();
        l.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), Brush::Solid(Color::WHITE));
        l
    });
    let mut comp = Compositor::new(100, 100, 1.0, ScrollPhysics::CLAMPED);
    comp.commit(LayerTree::new(vec![page, hole, control]));
    let plan = comp.frame(Duration::ZERO);
    let mut cpu = CpuCompositor::new();
    let f = cpu.present(&plan, 100, 100, Color::BLACK).unwrap_or_else(|e| unreachable!("{e:?}"));
    let px = |x: usize, y: usize| f.pixels[(y * 100 + x) * 4..][..4].to_vec();
    assert_eq!(px(5, 5), vec![0, 0, 0, 255], "the page around it");
    assert_eq!(px(25, 25), vec![0, 0, 0, 0], "cleared: what is under the window shows");
    assert_eq!(px(50, 50), vec![255, 255, 255, 255], "the control over it");
}
