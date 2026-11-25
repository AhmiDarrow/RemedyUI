//! The view's own words: every locale has every message, and they follow the window's locale.

use remedy_ui_core::style::{StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::Ui;

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_engine::view::strings::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn the_undo_item_speaks_the_window_language() {
    let ui = Ui::new(Theme::dark_forest(), StyleSheet::new(), FontCollection::new());
    ui.record_change("Make rest", None, || {}, || {});
    let item = ui.next_undo().unwrap_or_else(|| unreachable!("a change was recorded"));
    let plain = |s: String| s.replace(['\u{2068}', '\u{2069}'], "");
    assert_eq!(item.undo_label(), "Undo Make rest", "the English name stays as it was");
    ui.set_locale("en");
    assert_eq!(plain(item.localized_undo_label(&ui)), "Undo Make rest");
    ui.set_locale("de");
    assert_eq!(plain(item.localized_undo_label(&ui)), "Rückgängig: Make rest");
    ui.record_change("Add highlight", Some("assistant".into()), || {}, || {});
    let by = ui.next_undo().unwrap_or_else(|| unreachable!("a change was recorded"));
    assert_eq!(plain(by.localized_undo_label(&ui)), "Rückgängig: Add highlight (von assistant)");
    assert_eq!(plain(by.undo_label_in("fr")), "Annuler Add highlight (par assistant)");
    assert_eq!(plain(by.undo_label_in("en-GB")), "Undo Add highlight, by assistant");
}
