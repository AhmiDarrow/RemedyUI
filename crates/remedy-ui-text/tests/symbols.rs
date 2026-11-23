//! Symbols from Unicode's Common script (✦ ◆ ⚠ ☰ ⌕ ⇄ …) reach a font that has them, with
//! the bundled Remedy Symbols alone (as in a browser page) and with the system's fonts.

use remedy_ui_core::Color;
use remedy_ui_text::*;

/// The mode toggle and wiki icons of a real app, with arrows and marks between words.
const SAMPLE: &str = "✦ Soap ✧ Candle ◈◆◇◎⚠☰⌕ ← × • ≈ Ø ⇄";

fn bundled(symbols: bool) -> FontCollection {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        // Leave out every bundled symbol/emoji fallback when the test wants a collection
        // with none of them, so missing glyphs stay missing.
        if symbols || !matches!(face.family, "Remedy Symbols" | "Twemoji Mozilla") {
            assert!(fonts.add_static(face.data).is_ok(), "{} parses", face.family);
        }
    }
    fonts
}

fn layout(fonts: FontCollection, text: &RichText, style: &TextStyle) -> Paragraph {
    TextEngine::new(fonts).layout(text, style, ParagraphOptions::default())
}

/// Every glyph id in the paragraph, with the text of the run it is in.
fn missing(p: &Paragraph, text: &str) -> Vec<String> {
    p.glyph_runs()
        .into_iter()
        .filter(|r| r.glyphs.iter().any(|g| g.id == 0))
        .map(|r| text.get(r.range.clone()).unwrap_or_default().to_string())
        .collect()
}

fn symbols_face() -> &'static [u8] {
    remedy_ui_fonts::REMEDY_SYMBOLS.first().map_or(&[], |f| f.data)
}

fn run_font_of(p: &Paragraph, text: &str, needle: &str) -> Option<RunFont> {
    let at = text.find(needle)?;
    p.glyph_runs().into_iter().find(|r| r.range.contains(&at)).map(|r| r.font)
}

#[test]
fn bundled_symbols_draw_every_symbol_without_system_fonts() {
    let p = layout(bundled(true), &RichText::plain(SAMPLE), &TextStyle::default());
    assert_eq!(missing(&p, SAMPLE), Vec::<String>::new());
    // The symbols come from Remedy Symbols; the words and the marks the text face has stay
    // in Remedy Sans UI.
    for s in ["✦", "✧", "◈", "◆", "◇", "◎", "⚠", "☰", "⌕", "⇄"] {
        let font = run_font_of(&p, SAMPLE, s);
        assert!(font.is_some_and(|f| f.bytes() == symbols_face()), "{s} is not drawn from Remedy Symbols");
    }
    for s in ["Soap", "Candle", "←", "×", "•", "≈", "Ø"] {
        let font = run_font_of(&p, SAMPLE, s);
        assert!(font.is_some_and(|f| f.bytes() != symbols_face()), "{s} left its own font");
    }
}

#[test]
fn every_bundled_symbol_draws_under_any_bundled_family() {
    let text = remedy_ui_fonts::SYMBOL_CHARS;
    for family in ["Remedy Sans UI", "AhmiPrint", "A family nobody has"] {
        let mut style = TextStyle::default();
        style.families = vec![family.into()];
        let p = layout(bundled(true), &RichText::plain(text), &style);
        assert_eq!(missing(&p, text), Vec::<String>::new(), "under {family}");
    }
}

#[test]
fn without_any_symbol_font_the_missing_glyph_box_stays() {
    let p = layout(bundled(false), &RichText::plain(SAMPLE), &TextStyle::default());
    assert!(!missing(&p, SAMPLE).is_empty());
    // Turning the fallback off leaves the box even when Remedy Symbols is there.
    let mut fonts = bundled(true);
    fonts.set_symbol_fallbacks(Vec::<String>::new());
    let p = layout(fonts, &RichText::plain(SAMPLE), &TextStyle::default());
    assert!(!missing(&p, SAMPLE).is_empty());
}

#[test]
fn symbol_fallback_keeps_spans_links_and_colors() {
    let red = Color::rgb8(200, 30, 30);
    let text = RichText::new()
        .styled("✦ Soap", TextStylePatch::link(7))
        .text(" and ")
        .styled("⚠ hot", TextStylePatch::color(red));
    let p = layout(bundled(true), &text, &TextStyle::default());
    assert_eq!(missing(&p, text.as_str()), Vec::<String>::new());
    let runs = p.glyph_runs();
    let star = runs.iter().find(|r| r.range.contains(&0));
    assert!(star.is_some_and(|r| r.link == Some(7) && r.font.bytes() == symbols_face()));
    let warn_at = text.as_str().find('⚠').unwrap_or(usize::MAX);
    let warn = runs.iter().find(|r| r.range.contains(&warn_at));
    assert!(warn.is_some_and(|r| r.color == red && r.font.bytes() == symbols_face()));
    assert_eq!(p.link_rects(7).len(), 1);
}

#[test]
fn a_symbol_renders_as_its_shape_not_the_box() -> Result<(), &'static str> {
    let p = layout(bundled(true), &RichText::plain("✦"), &TextStyle::default());
    let run = p.glyph_runs().into_iter().next().ok_or("no run")?;
    let id = run.glyphs.first().ok_or("no glyph")?.id;
    assert_ne!(id, 0);
    let img = GlyphRasterizer::new()
        .rasterize(&run.font, id, 32.0, 0, Hinting::None, Antialias::Grayscale)
        .ok_or("no image")?;
    let (w, h) = (img.width as usize, img.height as usize);
    let at = |x: usize, y: usize| img.data.get(y * w + x).copied().unwrap_or(0);
    // A four-pointed star: solid in the middle, empty in the corners (the missing-glyph box
    // is the other way round).
    assert!(at(w / 2, h / 2) > 200, "the centre is ink");
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        assert!(at(x, y) < 40, "corner ({x}, {y}) is empty");
    }
    // As tall as it is wide, and about two-thirds of the size.
    assert!(w.abs_diff(h) <= 2 && (18..=26).contains(&h), "{w}×{h}");
    Ok(())
}

/// Windows always has Segoe UI Symbol, so with system fonts and no bundled symbols face
/// every symbol still finds a glyph.
#[cfg(all(feature = "system-fonts", windows))]
#[test]
fn system_symbol_fonts_fill_in_on_windows() {
    let mut fonts = FontCollection::with_system_fonts();
    for face in remedy_ui_fonts::all().into_iter().filter(|f| f.family != "Remedy Symbols") {
        assert!(fonts.add_static(face.data).is_ok());
    }
    let p = layout(fonts, &RichText::plain(SAMPLE), &TextStyle::default());
    assert_eq!(missing(&p, SAMPLE), Vec::<String>::new());
}

/// With system fonts and Remedy Symbols both there, every symbol finds a glyph, and those the
/// script's fallback fonts lack (on Windows: ✦ ✧ ☰ for Latin text) come from Remedy Symbols,
/// ahead of the system's symbol fonts. (⚠ has the Emoji property, so shaping offers it to the
/// emoji family first, as it does any emoji.)
#[cfg(feature = "system-fonts")]
#[test]
fn bundled_symbols_come_first_with_system_fonts() {
    let mut fonts = FontCollection::with_system_fonts();
    for face in remedy_ui_fonts::all() {
        assert!(fonts.add_static(face.data).is_ok());
    }
    let p = layout(fonts, &RichText::plain(SAMPLE), &TextStyle::default());
    assert_eq!(missing(&p, SAMPLE), Vec::<String>::new());
    #[cfg(windows)]
    for s in ["✦", "✧", "☰"] {
        let font = run_font_of(&p, SAMPLE, s);
        assert!(font.is_some_and(|f| f.bytes() == symbols_face()), "{s} is not drawn from Remedy Symbols");
    }
}
