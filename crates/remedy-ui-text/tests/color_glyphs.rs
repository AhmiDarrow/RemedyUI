//! Color emoji rasterize to premultiplied RGBA, from the bundled Twemoji face and from a
//! system CBDT face when one is installed.

use remedy_ui_core::Color;
use remedy_ui_text::*;

fn twemoji() -> FontCollection {
    let mut fonts = FontCollection::new();
    let face = remedy_ui_fonts::TWEMOJI_MOZILLA.first().expect("twemoji feature");
    assert!(fonts.add_static(face.data).is_ok());
    fonts
}

fn glyph_id(fonts: FontCollection, family: &str, ch: char) -> (RunFont, u32) {
    let mut style = TextStyle::default();
    style.families = vec![family.into()];
    let p =
        TextEngine::new(fonts).layout(&RichText::plain(&ch.to_string()), &style, ParagraphOptions::default());
    let run = p.glyph_runs().into_iter().next().unwrap_or_else(|| panic!("no run for {ch} in {family}"));
    let id = run.glyphs.first().expect("glyph").id;
    assert_ne!(id, 0, "missing glyph for {ch} in {family}");
    (run.font, id)
}

fn is_colorful(img: &GlyphImage) -> bool {
    assert!(img.is_color(), "expected a color image, got {:?}", img.format);
    assert!(img.width >= 8 && img.height >= 8, "too small: {}x{}", img.width, img.height);
    let mut colorful = 0usize;
    let mut ink = 0usize;
    for px in img.data.chunks_exact(4) {
        let [r, g, b, a] = [px[0], px[1], px[2], px[3]];
        if a < 32 {
            continue;
        }
        ink += 1;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if max.saturating_sub(min) > 20 {
            colorful += 1;
        }
    }
    ink > 20 && colorful > 10
}

#[test]
fn bundled_twemoji_paints_reactions_in_color() {
    let fonts = twemoji();
    let mut raster = GlyphRasterizer::new();
    for ch in ['👍', '👎', '❤', '💔'] {
        let (font, id) = glyph_id(fonts.clone(), "Twemoji Mozilla", ch);
        assert!(GlyphRasterizer::is_color_glyph(&font, id), "{ch} should be a color glyph");
        let img = raster
            .rasterize(&font, id, 32.0, 0, Hinting::None, Antialias::Grayscale)
            .unwrap_or_else(|| panic!("no image for {ch}"));
        assert!(is_colorful(&img), "{ch} did not paint in color ({}x{})", img.width, img.height);
    }
}

#[test]
fn thumbs_up_is_yellow_not_tinted_black() {
    let fonts = twemoji();
    let (font, id) = glyph_id(fonts, "Twemoji Mozilla", '👍');
    let img = GlyphRasterizer::new()
        .rasterize_foreground(&font, id, 48.0, 0, Hinting::None, Antialias::Grayscale, Color::BLACK)
        .expect("image");
    // The Twemoji thumbs-up is a golden yellow hand. A black-tinted coverage glyph would be
    // dark; a color glyph keeps the yellow even when the foreground is black.
    let mut yellow = 0usize;
    for px in img.data.chunks_exact(4) {
        let [r, g, b, a] = [px[0], px[1], px[2], px[3]];
        if a > 80 && r > 140 && g > 100 && b < 80 {
            yellow += 1;
        }
    }
    assert!(yellow > 30, "expected a yellow hand, found {yellow} yellow pixels");
}

/// The Noto Color Emoji most Linux machines install is a CBDT face with no outlines. If it is
/// present, the rasterizer must paint it too, not return empty.
#[test]
fn system_cbdt_emoji_paints_when_installed() {
    let path = "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf";
    let Ok(bytes) = std::fs::read(path) else { return };
    let mut fonts = FontCollection::new();
    let file = FontFile::from_vec(bytes).expect("parse Noto Color Emoji");
    assert!(fonts.add_file(&file).is_ok());
    let (font, id) = glyph_id(fonts, "Noto Color Emoji", '👍');
    assert!(GlyphRasterizer::is_color_glyph(&font, id));
    let img = GlyphRasterizer::new()
        .rasterize(&font, id, 32.0, 0, Hinting::None, Antialias::Grayscale)
        .expect("CBDT thumbs-up must paint, not return None");
    assert!(is_colorful(&img), "CBDT thumbs-up stayed blank or monochrome");
}
