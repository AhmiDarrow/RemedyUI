//! Letter-spaced text drawn upright and turned: the spacing is in the glyphs' places, so it
//! turns with the line.

use remedy_ui_core::{Color, Point, Vec2};
use remedy_ui_engine::paint::{Affine, DisplayList, GlyphRun, Item};
use remedy_ui_engine::render_cpu::{CpuRenderer, Frame};
use remedy_ui_text::{Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle};

/// "HHHHHHHH" at 20 px, `spacing` apart, under `t`.
fn text(t: Affine, spacing: f32) -> DisplayList {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let mut style = TextStyle::default();
    style.size = 20.0;
    style.letter_spacing = spacing;
    let p = engine.layout(&RichText::plain("HHHHHHHH"), &style, ParagraphOptions::default());
    let mut list = DisplayList::new();
    list.push(Item::PushTransform(t));
    for run in p.glyph_runs() {
        list.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color: Color::BLACK,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(g.x, g.y))).collect(),
            hinting: Hinting::Full,
            antialias: Antialias::Grayscale,
        }));
    }
    list.push(Item::PopTransform);
    list
}

/// The ink's extent across and down.
fn ink(list: &DisplayList, w: u32, h: u32) -> (u32, u32) {
    let f: Frame =
        CpuRenderer::new().render(list, w, h, 1.0, Color::WHITE).unwrap_or_else(|e| unreachable!("{e}"));
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..f.height {
        for x in 0..f.width {
            if f.pixel(x, y).is_some_and(|p| p[0] < 128) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    assert!(x1 >= x0 && y1 >= y0, "ink");
    (x1 - x0, y1 - y0)
}

#[test]
fn spaced_text_spreads_along_its_line_upright_and_turned() {
    // Eight letters: seven gaps between their ink, 6 px wider each.
    let across = |s: f32| ink(&text(Affine::translate(Vec2::new(10.0, 40.0)), s), 260, 60).0;
    let grown = across(6.0) as f32 - across(0.0) as f32;
    assert!((grown - 42.0).abs() <= 2.0, "upright: {grown}");
    // Turned a quarter: the same growth, down the screen.
    let turn = |_: f32| Affine::rotate(90f32.to_radians()).then(Affine::translate(Vec2::new(30.0, 10.0)));
    let down = |s: f32| ink(&text(turn(s), s), 60, 260).1;
    let grown = down(6.0) as f32 - down(0.0) as f32;
    assert!((grown - 42.0).abs() <= 2.0, "turned: {grown}");
}
