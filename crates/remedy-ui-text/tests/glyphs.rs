//! Rasterizing real Remedy Sans UI glyphs, and the atlas.

use remedy_ui_text::*;

fn regular() -> Option<RunFont> {
    let mut e = {
        let mut fonts = FontCollection::new();
        for face in remedy_ui_fonts::all() {
            fonts.add_static(face.data).ok()?;
        }
        TextEngine::new(fonts)
    };
    let p = e.layout(&RichText::plain("H"), &TextStyle::default(), ParagraphOptions::default());
    p.glyph_runs().into_iter().next().map(|r| r.font)
}

fn glyph_of(text: &str) -> Option<(RunFont, u32)> {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        fonts.add_static(face.data).ok()?;
    }
    let mut e = TextEngine::new(fonts);
    let p = e.layout(&RichText::plain(text), &TextStyle::default(), ParagraphOptions::default());
    let run = p.glyph_runs().into_iter().next()?;
    let id = run.glyphs.first()?.id;
    Some((run.font, id))
}

/// Columns whose pixels in the middle rows are all fully covered.
fn solid_columns(img: &GlyphImage) -> usize {
    let (w, h) = (img.width as usize, img.height as usize);
    (0..w)
        .filter(|&x| (h / 4..h * 3 / 4).all(|y| img.data.get(y * w + x).copied().unwrap_or(0) >= 240))
        .count()
}

/// Columns that are partly covered (neither empty nor solid) in the middle rows.
fn blurry_columns(img: &GlyphImage) -> usize {
    let (w, h) = (img.width as usize, img.height as usize);
    (0..w)
        .filter(|&x| {
            let mid = h / 2;
            let c = img.data.get(mid * w + x).copied().unwrap_or(0);
            (30..225).contains(&c)
        })
        .count()
}

#[test]
fn hinted_stems_land_on_whole_pixels() -> Result<(), &'static str> {
    let (font, h) = glyph_of("H").ok_or("no glyph")?;
    let mut r = GlyphRasterizer::new();
    let hinted = r.rasterize(&font, h, 13.0, 0, Hinting::Full, Antialias::Grayscale).ok_or("no image")?;
    let raw = r.rasterize(&font, h, 13.0, 2, Hinting::None, Antialias::Grayscale).ok_or("no image")?;
    // Both stems of H are solid columns when hinted.
    assert!(solid_columns(&hinted) >= 2, "hinted H has {} solid columns", solid_columns(&hinted));
    // Hinting leaves fewer half-covered columns than an unhinted glyph at a half-pixel offset.
    assert!(
        blurry_columns(&hinted) < blurry_columns(&raw),
        "{} vs {}",
        blurry_columns(&hinted),
        blurry_columns(&raw)
    );
    // The crossbar is one fully solid row when hinted, and no row is solid unhinted.
    let solid_rows = |img: &GlyphImage| {
        img.data
            .chunks(img.width as usize)
            .filter(|row| row.iter().skip(1).take(row.len().saturating_sub(2)).all(|&c| c >= 240))
            .count()
    };
    assert_eq!(solid_rows(&hinted), 1);
    assert_eq!(solid_rows(&raw), 0);
    // The image sits on the baseline: its bottom is at or just below y = 0.
    assert!(hinted.top > 0 && hinted.top - hinted.height as i32 <= 1);
    Ok(())
}

#[test]
fn lcd_images_have_three_channels_and_coloured_edges() -> Result<(), &'static str> {
    let (font, o) = glyph_of("o").ok_or("no glyph")?;
    let mut r = GlyphRasterizer::new();
    let img =
        r.rasterize(&font, o, 14.0, 0, Hinting::Full, Antialias::Lcd { bgr: false }).ok_or("no image")?;
    assert_eq!(img.data.len(), (img.width * img.height * 3) as usize);
    let fringe = img.data.chunks(3).any(|px| matches!(px, [r, _, b] if r != b));
    assert!(fringe, "edges differ per subpixel");
    let bgr =
        r.rasterize(&font, o, 14.0, 0, Hinting::Full, Antialias::Lcd { bgr: true }).ok_or("no image")?;
    // BGR is the same coverage with red and blue swapped.
    let swapped: Vec<u8> = img
        .data
        .chunks(3)
        .flat_map(|px| {
            [
                px.get(2).copied().unwrap_or(0),
                px.get(1).copied().unwrap_or(0),
                px.first().copied().unwrap_or(0),
            ]
        })
        .collect();
    assert_eq!(bgr.data, swapped);
    Ok(())
}

#[test]
fn spaces_have_no_image() -> Result<(), &'static str> {
    let font = regular().ok_or("no font")?;
    let mut atlas = GlyphAtlas::new(Antialias::Grayscale);
    // Glyph 3 is not drawn in every font; find the space through layout instead.
    let (_, space) = glyph_of(" x").ok_or("no glyph")?;
    assert_eq!(atlas.glyph(&font, space, 14.0, 0.0, Hinting::Full), None);
    assert_eq!(atlas.stats().glyphs, 0);
    Ok(())
}

#[test]
fn the_atlas_packs_reuses_and_reports_uploads() -> Result<(), &'static str> {
    let font = regular().ok_or("no font")?;
    let mut atlas = GlyphAtlas::new(Antialias::Grayscale);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        fonts.add_static(face.data).map_err(|_| "font")?;
    }
    let mut e = TextEngine::new(fonts);
    let text: String = (' '..='~').collect();
    let p = e.layout(&RichText::plain(text), &TextStyle::default(), ParagraphOptions::default());
    let ids: Vec<u32> = p.glyph_runs().iter().flat_map(|r| r.glyphs.iter().map(|g| g.id)).collect();
    let mut placed = Vec::new();
    for size in [12.0, 13.0, 16.0, 20.0, 32.0] {
        for &id in &ids {
            if let Some(entry) = atlas.glyph(&font, id, size, 0.0, Hinting::Full) {
                placed.push(entry);
            }
        }
    }
    assert!(placed.len() > 400);
    // No two glyphs overlap on a page.
    for (i, a) in placed.iter().enumerate() {
        for b in placed.iter().skip(i + 1) {
            let apart = a.page != b.page
                || a.x + a.width <= b.x
                || b.x + b.width <= a.x
                || a.y + a.height <= b.y
                || b.y + b.height <= a.y;
            assert!(apart, "{a:?} overlaps {b:?}");
        }
    }
    // Asking again is a cache hit with the same place.
    let again = ids.iter().filter_map(|&id| atlas.glyph(&font, id, 12.0, 0.0, Hinting::Full)).count();
    assert_eq!(atlas.stats().glyphs, placed.len());
    assert!(again > 0);
    // Uploads cover what was drawn, once.
    let dirty = atlas.take_dirty();
    assert!(!dirty.is_empty());
    assert!(atlas.take_dirty().is_empty());
    Ok(())
}

#[test]
fn unused_pages_are_dropped_under_pressure() -> Result<(), &'static str> {
    let font = regular().ok_or("no font")?;
    let (_, h) = glyph_of("H").ok_or("no glyph")?;
    let mut atlas = GlyphAtlas::new(Antialias::Grayscale);
    atlas.glyph(&font, h, 14.0, 0.0, Hinting::Full).ok_or("no entry")?;
    for _ in 0..100 {
        atlas.begin_frame();
    }
    assert_eq!(atlas.evict_unused(10), 1);
    assert_eq!(atlas.stats(), AtlasStats::default());
    // It comes back when needed.
    assert!(atlas.glyph(&font, h, 14.0, 0.0, Hinting::Full).is_some());
    assert_eq!(atlas.stats().pages, 1);
    Ok(())
}

#[test]
fn a_glyph_larger_than_a_page_gets_its_own() -> Result<(), &'static str> {
    let font = regular().ok_or("no font")?;
    let (_, h) = glyph_of("H").ok_or("no glyph")?;
    let mut atlas = GlyphAtlas::new(Antialias::Grayscale);
    let e = atlas.glyph(&font, h, 1500.0, 0.0, Hinting::None).ok_or("no entry")?;
    let page = atlas.page(e.page).ok_or("no page")?;
    assert!(page.height() > PAGE_SIZE);
    Ok(())
}

#[test]
fn the_app_sets_the_page_size() -> Result<(), &'static str> {
    let font = regular().ok_or("no font")?;
    let (_, h) = glyph_of("H").ok_or("no glyph")?;
    let mut atlas = GlyphAtlas::new(Antialias::Grayscale);
    atlas.set_page_size(256);
    let e = atlas.glyph(&font, h, 14.0, 0.0, Hinting::Full).ok_or("no entry")?;
    let page = atlas.page(e.page).ok_or("no page")?;
    assert_eq!((page.width(), page.height()), (256, 256));
    Ok(())
}
