//! Color emoji rasterize to premultiplied RGBA, from the bundled Remedy Emoji face and from a
//! system CBDT face when one is installed.

use remedy_ui_core::Color;
use remedy_ui_text::*;

fn remedy_emoji() -> Result<FontCollection, String> {
    let mut fonts = FontCollection::new();
    let face = remedy_ui_fonts::REMEDY_EMOJI.first().ok_or("remedy-emoji feature")?;
    assert!(fonts.add_static(face.data).is_ok());
    Ok(fonts)
}

fn glyph_id(fonts: FontCollection, family: &str, ch: char) -> Result<(RunFont, u32), String> {
    let mut style = TextStyle::default();
    style.families = vec![family.into()];
    let p =
        TextEngine::new(fonts).layout(&RichText::plain(ch.to_string()), &style, ParagraphOptions::default());
    let run = p.glyph_runs().into_iter().next().ok_or_else(|| format!("no run for {ch} in {family}"))?;
    let id = run.glyphs.first().ok_or("glyph")?.id;
    assert_ne!(id, 0, "missing glyph for {ch} in {family}");
    Ok((run.font, id))
}

fn is_colorful(img: &GlyphImage) -> bool {
    assert!(img.is_color(), "expected a color image, got {:?}", img.format);
    assert!(img.width >= 8 && img.height >= 8, "too small: {}x{}", img.width, img.height);
    let mut colorful = 0usize;
    let mut ink = 0usize;
    for &[r, g, b, a] in img.data.as_chunks::<4>().0 {
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
fn bundled_emoji_paint_reactions_in_color() -> Result<(), String> {
    let fonts = remedy_emoji()?;
    let mut raster = GlyphRasterizer::new();
    for ch in ['👍', '👎', '❤', '💔'] {
        let (font, id) = glyph_id(fonts.clone(), "Remedy Emoji", ch)?;
        assert!(GlyphRasterizer::is_color_glyph(&font, id), "{ch} should be a color glyph");
        let img = raster
            .rasterize(&font, id, 32.0, 0, Hinting::None, Antialias::Grayscale)
            .ok_or_else(|| format!("no image for {ch}"))?;
        assert!(is_colorful(&img), "{ch} did not paint in color ({}x{})", img.width, img.height);
    }
    Ok(())
}

#[test]
fn thumbs_up_is_yellow_not_tinted_black() -> Result<(), String> {
    let fonts = remedy_emoji()?;
    let (font, id) = glyph_id(fonts, "Remedy Emoji", '👍')?;
    let img = GlyphRasterizer::new()
        .rasterize_foreground(&font, id, 48.0, 0, Hinting::None, Antialias::Grayscale, Color::BLACK)
        .ok_or("image")?;
    // The thumbs-up is a golden yellow hand. A black-tinted coverage glyph would be
    // dark; a color glyph keeps the yellow even when the foreground is black.
    let yellow = img
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|&&[r, g, b, a]| a > 80 && r > 140 && g > 100 && b < 80)
        .count();
    assert!(yellow > 30, "expected a yellow hand, found {yellow} yellow pixels");
    Ok(())
}

/// The Noto Color Emoji most Linux machines install is a CBDT face with no outlines. If it is
/// present, the rasterizer must paint it too, not return empty.
#[test]
fn system_cbdt_emoji_paints_when_installed() -> Result<(), String> {
    let path = "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf";
    let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    let mut fonts = FontCollection::new();
    let file = FontFile::from_vec(bytes).map_err(|e| format!("parse Noto Color Emoji: {e:?}"))?;
    assert!(fonts.add_file(&file).is_ok());
    let (font, id) = glyph_id(fonts, "Noto Color Emoji", '👍')?;
    assert!(GlyphRasterizer::is_color_glyph(&font, id));
    let img = GlyphRasterizer::new()
        .rasterize(&font, id, 32.0, 0, Hinting::None, Antialias::Grayscale)
        .ok_or("CBDT thumbs-up must paint, not return None")?;
    assert!(is_colorful(&img), "CBDT thumbs-up stayed blank or monochrome");
    Ok(())
}

/// Remedy Emoji follows Unicode for skin tones (the emoji, then a modifier, is one glyph of
/// that tone) and keeps our own glyphs, which have no Unicode code point, in the private
/// use area.
#[test]
fn skin_tones_and_our_own_glyphs_paint() -> Result<(), String> {
    let fonts = remedy_emoji()?;
    let glyphs = |text: &str| {
        let mut style = TextStyle::default();
        style.families = vec!["Remedy Emoji".into()];
        let p = TextEngine::new(fonts.clone()).layout(
            &RichText::plain(text),
            &style,
            ParagraphOptions::default(),
        );
        p.glyph_runs()
            .into_iter()
            .flat_map(|r| r.glyphs.iter().map(|g| (r.font.clone(), g.id)).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let mut raster = GlyphRasterizer::new();
    let mut paint = |text: &str| -> Result<(u32, GlyphImage), String> {
        let [(font, id)] =
            glyphs(text).try_into().map_err(|g: Vec<_>| format!("{text:?} is {} glyphs", g.len()))?;
        let img = raster
            .rasterize(&font, id, 48.0, 0, Hinting::None, Antialias::Grayscale)
            .ok_or_else(|| format!("no image for {text:?}"))?;
        Ok((id, img))
    };
    // The share of inked pixels that are light (the default hand is yellow, the darkest brown).
    let light = |img: &GlyphImage| {
        let ink: Vec<_> = img.data.as_chunks::<4>().0.iter().filter(|p| p[3] > 200).collect();
        ink.iter().filter(|p| p[0] > 180).count() as f32 / ink.len().max(1) as f32
    };
    let (plain_id, plain) = paint("👍")?;
    let (dark_id, dark) = paint("👍\u{1F3FF}")?;
    assert_ne!(plain_id, dark_id, "the modifier picks the toned glyph");
    assert!(light(&plain) > 0.5 && light(&dark) < 0.1, "tones: {} and {}", light(&plain), light(&dark));
    // U+FE0F after an emoji is absorbed, not drawn as a second glyph.
    assert_eq!(paint("❤\u{FE0F}")?.0, paint("❤")?.0);
    // `facepalm`, the first of our own glyphs (RemedyEmoji's font/codepoints.txt).
    let (_, own) = paint("\u{F0000}")?;
    assert!(is_colorful(&own), "our own glyph did not paint in color");
    Ok(())
}
