//! LCD (subpixel) text: per-channel coverage over opaque pixels, valid pixels over
//! transparent ones, translucent colors, and the grayscale fallback inside groups.

use remedy_ui_core::{Color, Point, Rect};
use remedy_ui_paint::{Blend, Brush, DisplayList, GlyphRun, Item};
use remedy_ui_render_cpu::{CpuRenderer, Frame};
use remedy_ui_text::{Antialias, FontCollection, Hinting, ParagraphOptions, RichText, TextEngine, TextStyle};

fn runs(text: &str, color: Color, aa: Antialias) -> DisplayList {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    let mut engine = TextEngine::new(fonts);
    let p = engine.layout(&RichText::plain(text), &TextStyle::default(), ParagraphOptions::default());
    let mut l = DisplayList::new();
    for run in p.glyph_runs() {
        l.push(Item::Glyphs(GlyphRun {
            font: run.font,
            size: run.size,
            color,
            glyphs: run.glyphs.iter().map(|g| (g.id, Point::new(g.x + 4.0, g.y + 4.0))).collect(),
            hinting: Hinting::Full,
            antialias: aa,
        }));
    }
    l
}

fn render(list: &DisplayList, bg: Color) -> Frame {
    match CpuRenderer::new().render(list, 120, 40, 1.0, bg) {
        Ok(f) => f,
        Err(e) => unreachable!("{e}"),
    }
}

fn px(f: &Frame) -> &[[u8; 4]] {
    f.pixels.as_chunks::<4>().0
}

const LCD: Antialias = Antialias::Lcd { bgr: false };

#[test]
fn lcd_text_has_colored_fringes_and_a_dark_core() {
    let f = render(&runs("Hill", Color::BLACK, LCD), Color::WHITE);
    let fringe = px(&f).iter().filter(|p| p[0].abs_diff(p[2]) > 40).count();
    assert!(fringe > 10, "subpixel edges differ per channel: {fringe}");
    assert!(px(&f).iter().any(|p| p[0] < 30 && p[1] < 30 && p[2] < 30), "stems are solid");
    assert!(px(&f).iter().all(|p| p[3] == 255), "an opaque background stays opaque");
    let gray = render(&runs("Hill", Color::BLACK, Antialias::Grayscale), Color::WHITE);
    assert!(px(&gray).iter().all(|p| p[0] == p[1] && p[1] == p[2]), "grayscale text has no fringes");
}

#[test]
fn lcd_blends_each_channel_with_its_own_coverage() {
    // On a colored background, every pixel is bg * (1 - c) + color * c per channel for some
    // coverage c in 0..=1 per channel; with red text on blue, red only rises and blue only
    // falls.
    let bg = Color::rgb8(0, 0, 255);
    let f = render(&runs("Hill", Color::rgb8(255, 0, 0), LCD), bg);
    assert!(px(&f).iter().all(|p| p[1] == 0 && p[3] == 255), "green stays 0 and alpha opaque");
    assert!(px(&f).iter().any(|p| p[0] > 200 && p[2] < 60), "full coverage is red");
    assert!(
        px(&f).iter().any(|p| p[0] > 40 && p[2] > 40 && p[0].abs_diff(255 - p[2]) > 30),
        "channels cover independently"
    );
}

#[test]
fn lcd_inside_an_opacity_group_falls_back_to_grayscale() {
    let wrap = |aa| {
        let mut l = DisplayList::new();
        l.push(Item::PushLayer { opacity: 0.8, blend: Blend::Normal });
        l.extend(&runs("Hill", Color::BLACK, aa));
        l.push(Item::PopLayer);
        l
    };
    assert_eq!(render(&wrap(LCD), Color::WHITE), render(&wrap(Antialias::Grayscale), Color::WHITE));
}

#[test]
fn lcd_over_transparent_pixels_stays_valid() {
    let f = render(&runs("Hill", Color::BLACK, LCD), Color::TRANSPARENT);
    assert!(px(&f).iter().any(|p| p[3] > 200), "glyph cores are opaque");
    assert!(
        px(&f).iter().all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]),
        "valid premultiplied pixels"
    );
    // Away from the text nothing is drawn.
    assert_eq!(f.pixel(119, 39), Some([0, 0, 0, 0]));
    // Over a filled rect in the same list (a tile's own background), it is real LCD.
    let mut l = DisplayList::new();
    l.fill_rect(Rect::new(0.0, 0.0, 120.0, 40.0), Brush::Solid(Color::WHITE));
    l.extend(&runs("Hill", Color::BLACK, LCD));
    let on_tile = render(&l, Color::TRANSPARENT);
    assert_eq!(on_tile, render(&runs("Hill", Color::BLACK, LCD), Color::WHITE));
}

#[test]
fn translucent_lcd_text_scales_its_coverage_by_alpha() {
    // Half-transparent black over white: every pixel lies between white and the opaque
    // result, about halfway.
    let solid = render(&runs("Hill", Color::BLACK, LCD), Color::WHITE);
    let half = render(&runs("Hill", Color::rgba8(0, 0, 0, 128), LCD), Color::WHITE);
    for (s, h) in px(&solid).iter().zip(px(&half)) {
        for (sc, hc) in s.iter().zip(h.iter()).take(3) {
            let expect = (255 + u16::from(*sc)) / 2;
            assert!(u16::from(*hc).abs_diff(expect) <= 3, "{s:?} {h:?}");
        }
    }
}
