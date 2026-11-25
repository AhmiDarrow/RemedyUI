//! The service's own words: every locale has every message, and they follow the locale.

use remedy_ui_platform::services_i18n::{Decimal, DecimalError};
use remedy_ui_engine::view::Ui;

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_engine::view::strings::check(&remedy_ui_platform::services_i18n::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn number_errors_are_told_in_the_window_language() {
    assert_eq!(DecimalError::Invalid.message_in("en"), "Not a number");
    assert_eq!(DecimalError::Invalid.message_in("en"), DecimalError::Invalid.to_string());
    assert_eq!(DecimalError::Empty.message_in("fr-CA"), "Saisissez un nombre");
    assert_eq!(DecimalError::Overflow.message_in("ja"), "桁数が多すぎます");
    let ui = Ui::new(
        remedy_ui_core::style::Theme::dark_forest(),
        remedy_ui_core::style::StyleSheet::new(),
        remedy_ui_text::FontCollection::new(),
    );
    let err = Decimal::parse("12a").err().unwrap_or_else(|| unreachable!("not a number"));
    ui.set_locale("en");
    assert_eq!(err.message(&ui), "Not a number");
    ui.set_locale("de");
    assert_eq!(err.message(&ui), "Keine gültige Zahl");
}
