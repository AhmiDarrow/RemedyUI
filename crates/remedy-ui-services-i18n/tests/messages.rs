//! Fluent messages chosen for the window's locale, with plurals, numbers, overrides and reload.

use std::time::Duration;

use remedy_ui_core::reactive::{Effect, Signal};
use remedy_ui_services_i18n::{Arg, Messages, Problem};
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::Ui;

const EN: &str = r#"
app-title = Hearth
files = { $count ->
    [one] One file
   *[other] { $count } files
}
greeting = Hello, { $name }!
share = Share
    .tooltip = Share a folder with the house
"#;

const DE: &str = r#"
app-title = Hearth
files = { $count ->
    [one] Eine Datei
   *[other] { $count } Dateien
}
share = Freigeben
"#;

fn ui() -> Ui {
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(remedy_ui_core::style::Theme::dark_forest(), remedy_ui_core::style::StyleSheet::new(), fonts)
}

/// Without the bidi isolation marks Fluent puts around placeables.
fn plain(s: String) -> String {
    s.chars().filter(|c| !matches!(c, '\u{2068}' | '\u{2069}')).collect()
}

fn messages() -> Messages {
    Messages::new("en-US").embedded("en-US", EN).embedded("de", DE)
}

#[test]
fn messages_follow_the_locale_with_fallback_plurals_and_local_numbers() {
    let ui = ui();
    let m = messages();
    ui.set_locale("en-US");
    assert_eq!(plain(m.format(&ui, "files", &[("count", Arg::from(1))])), "One file");
    assert_eq!(plain(m.format(&ui, "files", &[("count", Arg::from(1234))])), "1,234 files");
    ui.set_locale("de-DE");
    assert_eq!(
        plain(m.format(&ui, "files", &[("count", Arg::from(1234))])),
        "1.234 Dateien",
        "German plural and grouping"
    );
    assert_eq!(m.text(&ui, "share"), "Freigeben");
    // Not translated into German: English, the default.
    assert_eq!(plain(m.format(&ui, "greeting", &[("name", Arg::from("Ada"))])), "Hello, Ada!");
    assert_eq!(m.attribute(&ui, "share", "tooltip", &[]), "Share a folder with the house");
    ui.set_locale("fr-FR");
    assert_eq!(m.text(&ui, "share"), "Share", "no French: the default");
    assert_eq!(m.locales(), vec!["de".to_owned(), "en-US".to_owned()]);
}

#[test]
fn missing_messages_and_broken_files_are_visible_not_silent() {
    let ui = ui();
    let m = Messages::new("en-US").embedded("en-US", "good = Fine\nbroken = { $\nalso-good = Also fine\n");
    assert_eq!(m.text(&ui, "good"), "Fine");
    assert_eq!(m.text(&ui, "also-good"), "Also fine", "the rest of a broken file still works");
    assert_eq!(m.text(&ui, "nope"), "\u{27E8}nope\u{27E9}");
    let problems = m.problems();
    assert!(problems.iter().any(|p| matches!(p, Problem::Syntax { .. })), "{problems:?}");
    assert!(problems.contains(&Problem::Missing { id: "nope".into() }), "{problems:?}");
}

#[test]
fn user_folders_override_strings_and_reload_live() {
    let ui = ui();
    let dir = std::env::temp_dir().join(format!("remedy-i18n-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("de"));
    let _ = std::fs::write(dir.join("en-US.ftl"), "share = Share with the house\n");
    let _ = std::fs::write(dir.join("de").join("extra.ftl"), "share = Teilen\n");
    let m = messages().folder(&dir);
    ui.set_locale("en-US");
    assert_eq!(m.text(&ui, "share"), "Share with the house", "the override wins");
    assert_eq!(m.text(&ui, "app-title"), "Hearth", "everything else stays");
    ui.set_locale("de");
    assert_eq!(m.text(&ui, "share"), "Teilen", "a locale subfolder");

    // A label bound to the message follows locale changes and file edits.
    let shown = Signal::new(String::new());
    let (m2, u2) = (m.clone(), ui.clone());
    let _e = Effect::new(move || shown.set(m2.text(&u2, "share")));
    assert_eq!(shown.get(), "Teilen");
    ui.set_locale("en-US");
    assert_eq!(shown.get(), "Share with the house");
    m.watch();
    std::thread::sleep(Duration::from_millis(50));
    let _ = std::fs::write(dir.join("en-US.ftl"), "share = Share it all\n");
    for _ in 0..60 {
        remedy_ui_core::reactive::run_posted();
        if shown.get() == "Share it all" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(shown.get(), "Share it all", "the file edit reached the screen");
    drop(m);
    let _ = std::fs::remove_dir_all(&dir);
}
