//! Web fonts: WOFF and WOFF2 files, and a family split into subsets per script.
//!
//! The fixtures are Inter (SIL Open Font License 1.1, `fixtures/inter/OFL.txt`) as Fontsource
//! ships it: one file per script subset and weight.

use remedy_ui_text::*;

const LATIN_400_WOFF2: &[u8] = include_bytes!("fixtures/inter/inter-latin-400-normal.woff2");
const LATIN_400_WOFF: &[u8] = include_bytes!("fixtures/inter/inter-latin-400-normal.woff");
const LATIN_700_WOFF2: &[u8] = include_bytes!("fixtures/inter/inter-latin-700-normal.woff2");
const CYRILLIC_400_WOFF2: &[u8] = include_bytes!("fixtures/inter/inter-cyrillic-400-normal.woff2");

fn with_remedy(fonts: &mut FontCollection) {
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
}

fn inter() -> TextStyle {
    let mut s = TextStyle::default();
    s.families = vec!["Inter".into(), "Remedy Sans UI".into()];
    s
}

/// The font bytes each run of `text` was shaped with, with the text it covers.
fn runs(e: &mut TextEngine, text: &str, style: &TextStyle) -> Vec<(String, Vec<u8>)> {
    let p = e.layout(&RichText::plain(text), style, ParagraphOptions::default());
    p.glyph_runs()
        .into_iter()
        .map(|r| (text.get(r.range.clone()).unwrap_or_default().trim().to_owned(), r.font.bytes().to_vec()))
        .filter(|(t, _)| !t.is_empty())
        .collect()
}

fn is_remedy(bytes: &[u8]) -> bool {
    remedy_ui_fonts::all().iter().any(|f| f.data == bytes)
}

#[test]
fn woff_and_woff2_files_load_as_their_family() {
    for (what, data) in [("WOFF2", LATIN_400_WOFF2), ("WOFF", LATIN_400_WOFF)] {
        let mut fonts = FontCollection::new();
        assert_eq!(fonts.add_static(data), Ok(vec!["Inter".to_string()]), "{what}");
        assert_eq!(FontCollection::new().add_owned(data.to_vec()), Ok(vec!["Inter".to_string()]), "{what}");
        with_remedy(&mut fonts);
        let mut e = TextEngine::new(fonts);
        let shaped = runs(&mut e, "Hello", &inter());
        assert!(!shaped.is_empty() && shaped.iter().all(|(_, f)| !is_remedy(f)), "{what}: drawn in Inter");
        // The unwrapped font is TrueType or OpenType, as the rasterizer reads.
        assert!(shaped.iter().all(|(_, f)| f.starts_with(&[0, 1, 0, 0]) || f.starts_with(b"OTTO")), "{what}");
    }
}

#[test]
fn weights_of_a_web_font_are_faces_of_one_family() {
    let mut fonts = FontCollection::new();
    assert!(fonts.add_static(LATIN_400_WOFF2).is_ok());
    assert!(fonts.add_static(LATIN_700_WOFF2).is_ok());
    with_remedy(&mut fonts);
    assert_eq!(fonts.families().iter().filter(|f| f.as_str() == "Inter").count(), 1);
    let mut e = TextEngine::new(fonts);
    let mut bold = inter();
    bold.weight = 700.0;
    let regular = runs(&mut e, "Save", &inter());
    let heavy = runs(&mut e, "Save", &bold);
    assert!(regular.first().zip(heavy.first()).is_some_and(|(r, h)| r.1 != h.1), "700 has its own face");
}

#[test]
fn subsets_of_a_family_join_as_css_unicode_range_does() {
    let text = "Hello Привет";
    // Latin alone: the Cyrillic falls back to the next family in the style.
    let mut fonts = FontCollection::new();
    assert!(fonts.add_static(LATIN_400_WOFF2).is_ok());
    with_remedy(&mut fonts);
    let mut e = TextEngine::new(fonts);
    let alone = runs(&mut e, text, &inter());
    let cyrillic = alone.iter().find(|(t, _)| t == "Привет").map(|(_, f)| is_remedy(f));
    assert_eq!(cyrillic, Some(true), "{:?}", alone.iter().map(|r| &r.0).collect::<Vec<_>>());

    // With the Cyrillic subset added, both scripts are drawn in Inter, each by its own part.
    let mut fonts = FontCollection::new();
    assert!(fonts.add_static(LATIN_400_WOFF2).is_ok());
    assert_eq!(fonts.add_static(CYRILLIC_400_WOFF2), Ok(vec!["Inter".to_string()]), "a part of Inter");
    with_remedy(&mut fonts);
    assert_eq!(
        fonts.families().iter().filter(|f| f.starts_with("Inter")).collect::<Vec<_>>(),
        ["Inter"],
        "parts are not families of their own"
    );
    let mut e = TextEngine::new(fonts);
    let joined = runs(&mut e, text, &inter());
    let font = |word: &str| joined.iter().find(|(t, _)| t == word).map(|(_, f)| f.clone());
    let (latin, cyr) = (font("Hello"), font("Привет"));
    assert!(latin.as_deref().is_some_and(|f| !is_remedy(f)), "Latin in Inter");
    assert!(cyr.as_deref().is_some_and(|f| !is_remedy(f)), "Cyrillic in Inter's Cyrillic part");
    assert_ne!(latin, cyr, "each script from its own part");
}

#[test]
fn malformed_web_fonts_are_refused_without_harm() {
    let mut fonts = FontCollection::new();
    assert!(fonts.add_owned(b"wOFF".to_vec()).is_err());
    assert!(fonts.add_owned(b"wOF2\0\0\0\0".to_vec()).is_err());
    // A WOFF that claims a 4 GB table from 4 bytes of data is refused before unpacking.
    let mut bomb = LATIN_400_WOFF.to_vec();
    if let Some(len) = bomb.get_mut(44 + 12..44 + 16) {
        len.copy_from_slice(&u32::MAX.to_be_bytes());
    }
    assert!(fonts.add_owned(bomb).is_err());
    // Damaged files: cut short, or with bytes changed anywhere, never crash the app.
    for data in [LATIN_400_WOFF, LATIN_400_WOFF2] {
        for cut in (0..data.len()).step_by(997) {
            let _ = FontCollection::new().add_owned(data.get(..cut).unwrap_or_default().to_vec());
        }
        let mut seed = 0x2545_f491_u32;
        for _ in 0..300 {
            let mut bad = data.to_vec();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let at = seed as usize % bad.len();
                if let Some(b) = bad.get_mut(at) {
                    *b ^= (seed >> 24) as u8 | 1;
                }
            }
            let _ = FontCollection::new().add_owned(bad);
        }
    }
    assert!(fonts.families().is_empty(), "nothing was added");
}
