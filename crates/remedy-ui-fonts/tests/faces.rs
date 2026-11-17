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
        // Every face covers printable ASCII.
        let cmap = font.cmap()?;
        for c in ' '..='~' {
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
    feature = "ahmi-pixel"
))]
#[test]
fn all_features_give_all_faces() {
    assert_eq!(remedy_ui_fonts::all().len(), 11);
}
