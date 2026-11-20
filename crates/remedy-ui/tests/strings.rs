//! The runner's own words: every locale has every message, with every plural form.

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui::view::strings::check(&remedy_ui::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}
