//! Cased and clamped text as drawn: capitals from a transform give the pixels typed capitals
//! give, and a clamped paragraph's ink stays inside its lines, upright and turned.

use remedy_ui_core::{Color, Point, Vec2};
use remedy_ui_engine::paint::{Affine, DisplayList, GlyphRun, Item};
use remedy_ui_engine::render_cpu::{CpuRenderer, Frame};
use remedy_ui_text::{
    Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle, TextTransform,
};

fn draw(text: &str, transform: TextTransform, options: ParagraphOptions, t: Affine) -> DisplayList {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let mut style = TextStyle::default();
    style.size = 16.0;
    style.letter_spacing = 1.5;
    style.transform = transform;
    let p = engine.layout(&RichText::plain(text), &style, options);
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

fn render(list: &DisplayList, w: u32, h: u32) -> Frame {
    CpuRenderer::new().render(list, w, h, 1.0, Color::WHITE).unwrap_or_else(|e| unreachable!("{e}"))
}

/// The ink's extent across and down.
fn ink(f: &Frame) -> (u32, u32) {
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
fn cased_text_draws_as_capitals_and_clamped_text_stays_in_its_lines() {
    let at = Affine::translate(Vec2::new(10.0, 10.0));
    let o = ParagraphOptions::default();
    let cased = render(&draw("up next: straße", TextTransform::Upper, o, at), 240, 40);
    let typed = render(&draw("UP NEXT: STRASSE", TextTransform::None, o, at), 240, 40);
    let same = (0..40).all(|y| (0..240).all(|x| cased.pixel(x, y) == typed.pixel(x, y)));
    assert!(same, "the same pixels");

    let long = "The quick brown fox jumps over the lazy dog while the cat watches from the wall";
    let clamped = ParagraphOptions::default().max_width(120.0).max_lines(2);
    let (w, h) = ink(&render(&draw(long, TextTransform::None, clamped, at), 200, 200));
    assert!(w <= 121 && h < 45, "two lines, 120 px wide: {w} x {h}");
    // Turned a quarter, the same two lines run down the screen.
    let turn = Affine::rotate(90f32.to_radians()).then(Affine::translate(Vec2::new(60.0, 10.0)));
    let (w2, h2) = ink(&render(&draw(long, TextTransform::None, clamped, turn), 200, 200));
    assert!(h2.abs_diff(w) <= 2 && w2.abs_diff(h) <= 2, "turned: {w2} x {h2} vs {w} x {h}");
}
