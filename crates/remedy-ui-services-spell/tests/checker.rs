//! The checker end to end with Hunspell dictionaries written into the tests.

use std::time::{Duration, Instant};

use remedy_ui_services_spell::{Availability, Prepared, SpellBackend, SpellChecker, SpellError};

const EN_AFF: &str = "SET UTF-8\nSFX S Y 1\nSFX S 0 s [^s]\n";
const EN_DIC: &str = "9\nthe/S\nquick\nbrown\nfox/S\njumps\nover\nlazy\ndog/S\nhello/S\n";
const DE_AFF: &str = "SET UTF-8\n";
const DE_DIC: &str = "5\nder\nschnelle\nbraune\nFuchs\nHund\n";

fn english() -> SpellChecker {
    SpellChecker::builder()
        .platform(false)
        .system_dictionaries(false)
        .dictionary("en-US", EN_AFF.into(), EN_DIC.into())
        .build()
}

fn marked<'a>(text: &'a str, ranges: &[std::ops::Range<usize>]) -> Vec<&'a str> {
    ranges.iter().filter_map(|r| text.get(r.clone())).collect()
}

fn ok<T>(r: Result<T, SpellError>) -> T {
    r.unwrap_or_else(|e| unreachable!("{e}"))
}

#[test]
fn misspelled_words_are_found_and_the_rest_left_alone() {
    let c = english();
    let text =
        "The quick brwn fox jumps ovr the lazy dogs. @bob https://x.example/a NASA fooBar() `zzzz yyyy`";
    let found = ok(c.misspellings_in(text, &["en-US"]));
    assert_eq!(marked(text, &found), ["brwn", "ovr"], "mentions, links, acronyms and code are not judged");
}

#[test]
fn misspellings_with_the_checkers_own_languages() {
    let c = english();
    assert!(
        matches!(c.misspellings("teh fox"), Err(SpellError::Unavailable { .. })),
        "no language is chosen"
    );
    ok(c.set_languages(&["en-US"]));
    assert_eq!(c.languages(), ["en-US"]);
    assert_eq!(marked("teh fox", &ok(c.misspellings("teh fox"))), ["teh"]);
    assert!(matches!(c.set_languages(&["???"]), Err(SpellError::BadLanguage(_))));
    assert_eq!(c.languages(), ["en-US"], "a bad tag changes nothing");
}

#[test]
fn several_languages_at_once_accept_a_word_any_of_them_knows() {
    let c = SpellChecker::builder()
        .platform(false)
        .system_dictionaries(false)
        .dictionary("en-US", EN_AFF.into(), EN_DIC.into())
        .dictionary("de", DE_AFF.into(), DE_DIC.into())
        .build();
    let text = "the Fuchs und der brown Hund jumps";
    let both = ok(c.misspellings_in(text, &["en-US", "de"]));
    assert_eq!(marked(text, &both), ["und"]);
    let english_only = ok(c.misspellings_in(text, &["en-US"]));
    assert_eq!(marked(text, &english_only), ["Fuchs", "und", "der", "Hund"]);
    // Corrections from each language in turn.
    let s = ok(c.suggest_in("Fuch", &["en-US", "de"]));
    assert!(s.iter().any(|w| w == "Fuchs"), "{s:?}");
}

#[test]
fn a_word_in_another_script_is_not_judged_by_a_dictionary_for_a_different_one() {
    let c = english();
    let text = "hello \u{65e5}\u{672c}\u{8a9e} \u{43f}\u{440}\u{438}\u{432}\u{435}\u{442} wrld";
    let found = ok(c.misspellings_in(text, &["en-US"]));
    assert_eq!(marked(text, &found), ["wrld"], "Japanese and Russian are not English's to judge");
}

#[test]
fn the_personal_dictionary_accepts_added_and_ignored_words() {
    let c = english();
    let text = "hello remedy teh";
    assert_eq!(marked(text, &ok(c.misspellings_in(text, &["en-US"]))), ["remedy", "teh"]);
    assert!(c.add_word("remedy"));
    c.ignore_word("teh");
    assert!(ok(c.misspellings_in(text, &["en-US"])).is_empty());
    assert_eq!(c.personal().to_text(), "remedy\n", "ignored words are not saved");
    c.personal().remove("remedy");
    assert_eq!(marked(text, &ok(c.misspellings_in(text, &["en-US"]))), ["remedy"]);
}

#[test]
fn a_language_nothing_can_check_is_reported_honestly() {
    let c = english();
    assert_eq!(
        c.availability("en-GB"),
        Ok(Availability::Ready { backend: "hunspell (app dictionaries)".into(), dictionary: "en-US".into() })
    );
    assert_eq!(c.availability("ja"), Ok(Availability::Missing { reasons: vec![] }));
    match c.misspellings_in("konnichiwa", &["ja"]) {
        Err(SpellError::Unavailable { languages, .. }) => assert_eq!(languages, ["ja"]),
        other => unreachable!("{other:?}"),
    }
    // With one checkable language among several, the others are simply not used.
    assert!(c.misspellings_in("hello", &["ja", "en-US"]).is_ok());
    assert!(matches!(c.check_word_in("x", &["not a tag"]), Err(SpellError::BadLanguage(_))));
}

#[test]
fn a_broken_dictionary_is_reported_with_its_reason() {
    let c = SpellChecker::builder()
        .platform(false)
        .system_dictionaries(false)
        .dictionary("fr", "SET UTF-8\nPFX\n".into(), "not a count\nfoo\n".into())
        .build();
    match c.availability("fr") {
        Ok(Availability::Missing { reasons }) => {
            assert_eq!(reasons.len(), 1);
            assert!(reasons.iter().all(|r| r.contains("hunspell")), "{reasons:?}");
        }
        other => unreachable!("{other:?}"),
    }
    assert!(
        matches!(c.check_word_in("foo", &["fr"]), Err(SpellError::Unavailable { reasons, .. }) if !reasons.is_empty())
    );
}

#[test]
fn engines_that_cannot_start_are_listed_with_why() {
    let c = SpellChecker::builder()
        .platform(false)
        .system_dictionaries(false)
        .backend("sandboxed", Box::new(|| Err("the sandbox forbids it".to_owned())))
        .dictionary("en-US", EN_AFF.into(), EN_DIC.into())
        .build();
    let reports = ok(c.backends());
    assert_eq!(reports.len(), 2);
    let broken = reports.iter().find(|r| r.name == "sandboxed").unwrap_or_else(|| unreachable!("listed"));
    assert!(!broken.running);
    assert_eq!(broken.reason.as_deref(), Some("the sandbox forbids it"));
    assert!(reports.iter().any(|r| r.running));
    assert!(ok(c.check_word_in("hello", &["en-US"])), "the other engine still works");
}

/// An engine of the app's own: knows a fixed word list for one language.
struct Fixed;

impl SpellBackend for Fixed {
    fn name(&self) -> &str {
        "fixed"
    }

    fn prepare(&mut self, language: &str) -> Prepared {
        if language == "tlh" { Prepared::Ready { dictionary: "klingon".into() } } else { Prepared::Missing }
    }

    fn languages(&mut self) -> Vec<String> {
        vec!["tlh".into()]
    }

    fn check_word(&mut self, _language: &str, word: &str) -> Result<bool, String> {
        Ok(word == "qapla")
    }

    fn suggest(&mut self, _language: &str, word: &str) -> Result<Vec<String>, String> {
        Ok(if word == "qapl" { vec!["qapla".into()] } else { Vec::new() })
    }
}

#[test]
fn apps_can_bring_their_own_engine_and_it_is_tried_first() {
    let c = SpellChecker::builder()
        .platform(false)
        .system_dictionaries(false)
        .backend("fixed", Box::new(|| Ok(Box::new(Fixed))))
        .dictionary("en-US", EN_AFF.into(), EN_DIC.into())
        .build();
    assert_eq!(
        c.availability("tlh"),
        Ok(Availability::Ready { backend: "fixed".into(), dictionary: "klingon".into() })
    );
    assert!(ok(c.check_word_in("qapla", &["tlh"])));
    assert!(!ok(c.check_word_in("qapl", &["tlh"])));
    assert_eq!(ok(c.suggest_in("qapl", &["tlh"])), ["qapla"]);
    let langs = ok(c.available_languages());
    assert!(langs.iter().any(|l| l == "tlh") && langs.iter().any(|l| l == "en-US"), "{langs:?}");
}

#[test]
fn suggestions_exclude_the_word_itself_and_repeats() {
    let c = english();
    let s = ok(c.suggest_in("helo", &["en-US"]));
    assert!(s.iter().any(|w| w == "hello"), "{s:?}");
    assert!(s.iter().all(|w| w != "helo"));
    let mut unique = s.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), s.len());
}

#[test]
fn checks_run_in_the_background_and_dropped_ones_are_skipped() {
    let c = english();
    let text = "hello wrld ".repeat(2000);
    let mut pending = ok(c.check_async(&text, &["en-US"]));
    let started = Instant::now();
    let found = loop {
        if let Some(r) = pending.poll() {
            break ok(r);
        }
        assert!(started.elapsed() < Duration::from_secs(30), "the check never finished");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(found.len(), 2000);
    // A dropped check never answers anyone, and the next one is not held up.
    drop(ok(c.check_async(&text, &["en-US"])));
    assert!(ok(c.check_word_in("hello", &["en-US"])));
}

#[test]
fn suggestions_do_not_wait_for_a_long_check() {
    let c = english();
    let text = "hello wrld ".repeat(100_000);
    let long = ok(c.check_async(&text, &["en-US"]));
    let started = Instant::now();
    let s = ok(c.suggest_in("helo", &["en-US"]));
    let took = started.elapsed();
    assert!(s.iter().any(|w| w == "hello"));
    assert!(took < Duration::from_secs(5), "suggestions took {took:?} behind a long check");
    drop(long);
}

#[test]
fn notify_is_called_when_the_answer_is_waiting() {
    let c = english();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut p = ok(c.check_async_notify(
        "hello wrld",
        &["en-US"],
        Some(Box::new(move || {
            let _ = tx.send(());
        })),
    ));
    assert!(rx.recv_timeout(Duration::from_secs(10)).is_ok(), "no notice");
    assert_eq!(ok(p.poll().unwrap_or_else(|| unreachable!("the answer is there"))).len(), 1);
}

#[test]
fn handles_share_one_checker_across_threads() {
    let c = english();
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let c = c.clone();
            std::thread::spawn(move || {
                ok(c.check_word_in(if i % 2 == 0 { "hello" } else { "helo" }, &["en-US"]))
            })
        })
        .collect();
    let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap_or(false)).collect();
    assert_eq!(results, [true, false, true, false]);
}
