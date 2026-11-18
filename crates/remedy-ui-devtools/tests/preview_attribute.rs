//! Previews declared with `#[preview]` and listed with `#[previews]`: the gallery reads
//! them, and each is a golden-image test (`swatch_matches_its_golden_images`, generated
//! below, against `tests/golden`).

use remedy_ui_devtools::{GoldenError, golden_test, previews};

#[previews(crate = remedy_ui_devtools)]
mod gallery {
    use remedy_ui_core::Color;
    use remedy_ui_devtools::preview;
    use remedy_ui_style::StylePatch;
    use remedy_ui_view::{El, Ui};

    fn swatch_box(ui: &Ui, width: f32) -> El {
        ui.el("box")
            .width(width)
            .height(30.0)
            .style(StylePatch::new().background(Color::new(0.1, 0.6, 0.3, 1.0)))
    }

    #[preview(crate = remedy_ui_devtools, themes = ["forest", "light"], size = (120, 80), locales = ["en-US", "ar-EG"])]
    pub fn swatch(ui: &Ui) -> El {
        ui.el("row").row().child(swatch_box(ui, 40.0))
    }

    #[preview(crate = remedy_ui_devtools, name = "wide swatch", sizes = [(200, 50), (100.5, 50)], text_scale = 1.5, test = false)]
    pub fn wide(ui: &Ui) -> El {
        ui.el("row").row().child(swatch_box(ui, 90.0))
    }
}

#[test]
fn the_attribute_declares_previews_the_gallery_lists() {
    let all = gallery::all();
    let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["swatch", "wide swatch"]);
    let swatch = gallery::SWATCH.preview();
    assert_eq!(swatch.themes, ["forest", "light"]);
    assert_eq!(swatch.locales, ["en-US", "ar-EG"]);
    assert_eq!(swatch.sizes, [(120.0, 80.0)]);
    assert_eq!(swatch.variations().len(), 4);
    let wide = gallery::WIDE.preview();
    assert_eq!(wide.sizes, [(200.0, 50.0), (100.5, 50.0)]);
    assert_eq!(wide.text_scales, [1.5]);
    assert_eq!(wide.locales, ["en-US"], "unset lists keep the defaults");
    // Rendering follows the declaration: the box mirrors in a right-to-left locale.
    let ltr =
        swatch.render(&swatch.variations().into_iter().next().unwrap_or_else(|| unreachable!("a variation")));
    assert_eq!((ltr.width, ltr.height), (120, 80));
}

#[test]
fn golden_tests_write_missing_images_then_catch_changes() {
    let dir = std::env::temp_dir().join(format!("remedy-preview-attr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(golden_test(&gallery::WIDE, &dir), Ok(()), "a missing image is written for review");
    assert!(dir.join("wide_swatch-forest-en-US-200x50-1.5.png").exists());
    assert_eq!(golden_test(&gallery::WIDE, &dir), Ok(()), "the same rendering passes");
    // Another preview under the same name differs.
    let changed = remedy_ui_devtools::PreviewSpec { build: gallery::swatch, ..gallery::WIDE };
    assert!(matches!(golden_test(&changed, &dir), Err(GoldenError::Differs { .. })));
    let _ = std::fs::remove_dir_all(&dir);
}
