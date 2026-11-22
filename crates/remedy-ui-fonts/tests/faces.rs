//! Every bundled face parses and says what this crate says it is.

use read_fonts::{FontRef, TableProvider};

#[test]
fn every_face_matches_its_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let faces = remedy_ui_fonts::all();
    assert!(!faces.is_empty());
    for face in faces {
        let font = FontRef::new(face.data)?;
        let os2 = font.os2()?;
        assert_eq!(os2.us_weight_class(), face.weight, "{} {}", face.family, face.weight);
        assert_eq!(os2.fs_type(), 0, "{} is installable and embeddable", face.family);
        // Every text face covers printable ASCII; the symbols face has no letters on purpose,
        // and the script faces are only that script's slice.
        let cmap = font.cmap()?;
        let text_face = !remedy_ui_fonts::is_fallback_only(face.family);
        for c in (' '..='~').filter(|_| text_face) {
            assert!(cmap.map_codepoint(c).is_some(), "{} {} lacks {c:?}", face.family, face.weight);
        }
        // The family name in the font matches; non-RIBBI weights use the typographic name (16).
        let name = font.name()?;
        let found = name.name_record().iter().any(|r| {
            (r.name_id().to_u16() == 1 || r.name_id().to_u16() == 16)
                && r.string(name.string_data()).is_ok_and(|s| s.chars().eq(face.family.chars()))
        });
        assert!(found, "{} {} has no matching family name", face.family, face.weight);
    }
    Ok(())
}

#[cfg(all(
    feature = "remedy-sans-ui",
    feature = "remedy-sans",
    feature = "ahmi-print",
    feature = "ahmi-pixel",
    feature = "remedy-symbols",
    feature = "scripts"
))]
#[test]
fn all_features_give_all_faces() {
    assert_eq!(remedy_ui_fonts::all().len(), 21);
}

/// Remedy Symbols has every character `SYMBOL_CHARS` lists and nothing else but the space,
/// and none of them is a letter or digit (so it never takes text from a script's own font).
#[cfg(feature = "remedy-symbols")]
#[test]
fn symbols_face_has_exactly_its_listed_symbols() -> Result<(), Box<dyn std::error::Error>> {
    let face = remedy_ui_fonts::REMEDY_SYMBOLS.first().ok_or("no symbols face")?;
    let font = FontRef::new(face.data)?;
    let cmap = font.cmap()?;
    for c in remedy_ui_fonts::SYMBOL_CHARS.chars() {
        assert!(cmap.map_codepoint(c).is_some_and(|g| g.to_u32() != 0), "Remedy Symbols lacks {c:?}");
        assert!(!c.is_alphanumeric(), "{c:?} is a letter or digit");
    }
    let mut mapped: Vec<char> = Vec::new();
    for c in (0u32..0x3_0000).filter_map(char::from_u32) {
        if cmap.map_codepoint(c).is_some_and(|g| g.to_u32() != 0) {
            mapped.push(c);
        }
    }
    let listed: Vec<char> = std::iter::once(' ').chain(remedy_ui_fonts::SYMBOL_CHARS.chars()).collect();
    assert_eq!(mapped, listed);
    // A fallback face stays small.
    assert!(face.data.len() < 16 * 1024, "{} bytes", face.data.len());
    Ok(())
}

/// Each script face covers its script's letters, digits and marks, and says which script it
/// is a fallback for.
#[cfg(feature = "scripts")]
#[test]
fn script_faces_cover_their_scripts() -> Result<(), Box<dyn std::error::Error>> {
    let samples = [
        ("Noto Sans Arabic", "Arab", "ابتثجحخدذرزسشصضطظعغفقكلمنهوي٠١٢٣٤٥٦٧٨٩،؛؟ءآأؤإئةپچژگکی"),
        ("Noto Sans Devanagari", "Deva", "अआइईउऊऋएऐओऔकखगघङचछजझञटठडढणतथदधनपफबभमयरलवशषसह़ािीुूृेैोौ्ंःँ०१२३४५६७८९।॥"),
        ("Noto Sans Bengali", "Beng", "অআইঈউঊঋএঐওঔকখগঘঙচছজঝঞটঠডঢণতথদধনপফবভমযরলশষসহ়ড়ঢ়য়ািীুূৃেৈোৌ্ংঃঁ০১২৩৪৫৬৭৮৯৳ৎ"),
    ];
    let fallbacks = remedy_ui_fonts::script_fallbacks();
    for (family, script, chars) in samples {
        assert!(fallbacks.iter().any(|f| f.family == family && f.script == script), "{family} for {script}");
        for face in remedy_ui_fonts::all().into_iter().filter(|f| f.family == family) {
            let font = FontRef::new(face.data)?;
            let cmap = font.cmap()?;
            for c in chars.chars() {
                assert!(
                    cmap.map_codepoint(c).is_some_and(|g| g.to_u32() != 0),
                    "{family} {} lacks {c:?}",
                    face.weight
                );
            }
            // Shaping needs the font's own rules (joining, conjuncts, reordering).
            assert!(font.gsub().is_ok() && font.gpos().is_ok(), "{family} keeps its shaping tables");
        }
    }
    Ok(())
}
