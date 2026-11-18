//! The Windows spell checker (`ISpellChecker`), on a Windows machine that has English installed.
#![cfg(windows)]

use remedy_ui_services_spell::{Availability, SpellChecker, SpellError};

fn windows_only() -> SpellChecker {
    SpellChecker::builder().system_dictionaries(false).build()
}

fn ok<T>(r: Result<T, SpellError>) -> T {
    r.unwrap_or_else(|e| unreachable!("{e}"))
}

#[test]
fn windows_reports_itself_and_the_languages_it_has() {
    let c = windows_only();
    let reports = ok(c.backends());
    assert_eq!(reports.len(), 1, "{reports:?}");
    let r = reports.first().unwrap_or_else(|| unreachable!("one report"));
    assert_eq!(r.name, "windows");
    assert!(r.running, "the Windows spell checker did not start: {:?}", r.reason);
    let langs = ok(c.available_languages());
    assert!(langs.iter().any(|l| l.starts_with("en")), "English is installed here: {langs:?}");
    match ok(c.availability("en-US")) {
        Availability::Ready { backend, dictionary } => {
            assert_eq!(backend, "windows");
            assert!(dictionary.to_lowercase().starts_with("en"), "{dictionary}");
        }
        other => unreachable!("{other:?}"),
    }
}

#[test]
fn windows_finds_misspelled_english_words_and_offers_corrections() {
    let c = windows_only();
    let text = "This is a smple sentense with @mentions and https://example.com/x links.";
    let found = ok(c.misspellings_in(text, &["en-US"]));
    let words: Vec<&str> = found.iter().filter_map(|r| text.get(r.clone())).collect();
    assert_eq!(words, ["smple", "sentense"]);
    assert!(ok(c.check_word_in("sentence", &["en-US"])));
    assert!(!ok(c.check_word_in("sentense", &["en-US"])));
    let s = ok(c.suggest_in("sentense", &["en-US"]));
    assert!(s.iter().any(|w| w == "sentence"), "{s:?}");
    // A region Windows has no dictionary of its own for falls back to the language's.
    assert!(ok(c.check_word_in("sentence", &["en"])));
}

#[test]
fn windows_leaves_the_personal_dictionary_to_this_crate() {
    let c = windows_only();
    assert!(!ok(c.check_word_in("Remedyxyz", &["en-US"])));
    c.add_word("Remedyxyz");
    assert!(ok(c.check_word_in("Remedyxyz", &["en-US"])));
    c.personal().remove("Remedyxyz");
    assert!(!ok(c.check_word_in("Remedyxyz", &["en-US"])), "nothing was written to Windows' own list");
}

#[test]
fn a_language_windows_has_no_dictionary_for_is_missing_not_guessed() {
    let c = windows_only();
    // Klingon (tlh) has no Windows dictionary.
    assert_eq!(ok(c.availability("tlh")), Availability::Missing { reasons: vec![] });
    assert!(matches!(c.check_word_in("qapla", &["tlh"]), Err(SpellError::Unavailable { .. })));
}
