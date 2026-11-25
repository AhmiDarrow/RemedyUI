//! Finding the words in a text that are worth checking.
//!
//! Words are found by ICU word segmentation in the text's language. What people write in chat
//! has a lot in it that is not for a dictionary to judge, so these are left out:
//!
//! - links (`https://…`, `www.…`, `mailto:…`), email addresses, domain names and paths
//! - `@mentions`, `#channels`, `/commands` and `:emoji:` codes
//! - numbers, and words with digits in them (`3rd`, `mp3`)
//! - code: text in backticks, and words with underscores, `::`, `=`, `()` and the like
//! - camelCase names (`getValue`) and all-capital acronyms (`NASA`, `HTTP`)
//! - abbreviations with dots inside (`e.g`), and single letters

use std::ops::Range;

use icu_locale_core::LanguageIdentifier;
use icu_segmenter::WordSegmenter;
use icu_segmenter::WordSegmenterBorrowed;
use icu_segmenter::options::{WordBreakInvariantOptions, WordBreakOptions, WordType};

/// Runs `f` with a word segmenter tailored to `language` where ICU has tailoring for it.
fn with_segmenter<R>(language: &str, f: impl FnOnce(WordSegmenterBorrowed<'_>) -> R) -> R {
    let id = LanguageIdentifier::try_from_str(language).ok();
    let mut options = WordBreakOptions::default();
    options.content_locale = id.as_ref();
    #[cfg(feature = "spell-complex-scripts")]
    let made = WordSegmenter::try_new_dictionary(options);
    #[cfg(not(feature = "spell-complex-scripts"))]
    let made = WordSegmenter::try_new_for_non_complex_scripts(options);
    match made {
        Ok(s) => f(s.as_borrowed()),
        #[cfg(feature = "spell-complex-scripts")]
        Err(_) => f(WordSegmenter::new_dictionary(WordBreakInvariantOptions::default())),
        #[cfg(not(feature = "spell-complex-scripts"))]
        Err(_) => f(WordSegmenter::new_for_non_complex_scripts(WordBreakInvariantOptions::default())),
    }
}

/// The byte ranges of code spans: text in backticks, with fences of any length.
fn code_regions(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let tick = |i: usize| bytes.get(i) == Some(&b'`');
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !tick(i) {
            i += 1;
            continue;
        }
        let start = i;
        while tick(i) {
            i += 1;
        }
        let fence = i - start;
        // The span ends at the next run of exactly as many backticks.
        let mut j = i;
        let mut end = None;
        while j < bytes.len() {
            if tick(j) {
                let run = j;
                while tick(j) {
                    j += 1;
                }
                if j - run == fence {
                    end = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        if let Some(end) = end {
            out.push(start..end);
            i = end;
        }
    }
    out
}

/// The runs of text between whitespace.
fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push(s..i);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(s..text.len());
    }
    out
}

/// True for a whitespace-delimited token that is not prose: a link, address, mention, path or
/// piece of code.
fn skips_token(token: &str) -> bool {
    let t = token.trim_start_matches(|c: char| {
        matches!(c, '(' | '[' | '{' | '"' | '\'' | '“' | '‘' | '«' | '<' | '*' | '_' | '~' | '>')
    });
    let lower = t.to_lowercase();
    let first = t.chars().next();
    let leading = matches!(first, Some('@' | '#' | '/' | '\\'));
    let link = t.contains("://")
        || ["www.", "mailto:", "tel:", "file:", "magnet:"].iter().any(|p| lower.starts_with(p));
    // `name@host`, `@name`: addresses and mentions.
    let address = t.contains('@');
    // `:smile:` style emoji codes.
    let shortcode = {
        let core = t.trim_end_matches(|c: char| !c.is_alphanumeric() && c != ':');
        core.len() > 2 && core.starts_with(':') && core.ends_with(':')
    };
    let slashes = t.matches('/').count();
    let path = slashes >= 2 || (slashes == 1 && t.contains('.')) || t.contains('\\');
    let code = t.contains('`')
        || t.contains('_')
        || t.contains("::")
        || t.contains("=>")
        || t.contains("->")
        || t.contains('=')
        || t.contains("()")
        || t.contains("[]")
        || t.contains("{}");
    leading || link || address || shortcode || path || code
}

/// True for a word that is not for a dictionary to judge.
fn skips_word(word: &str) -> bool {
    let mut letters = 0usize;
    let mut upper = 0usize;
    let mut previous_lower = false;
    for c in word.chars() {
        if c.is_numeric() || c == '.' || c == '_' {
            return true;
        }
        if c.is_alphabetic() {
            letters += 1;
            if c.is_uppercase() {
                upper += 1;
                if previous_lower {
                    return true;
                }
            }
            previous_lower = c.is_lowercase();
        } else {
            previous_lower = false;
        }
    }
    // One letter is too little to judge; capitals only is an acronym.
    letters < 2 || (upper == letters)
}

/// The byte ranges of the words in `text` that a dictionary should judge, found by word
/// segmentation in `language` (a BCP 47 tag). The rules are listed in the module docs.
///
/// ```
/// use remedy_ui_widgets::spell::find_words;
/// let text = "Meet @sam at https://example.com about NASA and teh plan";
/// let words: Vec<&str> = find_words(text, "en").into_iter().filter_map(|r| text.get(r)).collect();
/// assert_eq!(words, ["Meet", "at", "about", "and", "teh", "plan"]);
/// ```
pub fn find_words(text: &str, language: &str) -> Vec<Range<usize>> {
    let code = code_regions(text);
    let mut out = Vec::new();
    with_segmenter(language, |segmenter| {
        for range in tokens(text) {
            // Code spans are in order and apart: only the first that ends at or after the
            // token can hold it (a search, so a long text full of code is not checked span
            // by span for every token).
            let span = code.partition_point(|c| c.end < range.end);
            if code.get(span).is_some_and(|c| c.start <= range.start) {
                continue;
            }
            let Some(token) = text.get(range.clone()) else { continue };
            if skips_token(token) {
                continue;
            }
            let mut previous = 0;
            for (end, kind) in segmenter.segment_str(token).iter_with_word_type() {
                if end > previous
                    && kind == WordType::Letter
                    && let Some(word) = token.get(previous..end)
                    && !skips_word(word)
                {
                    out.push(range.start + previous..range.start + end);
                }
                previous = end;
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words<'a>(text: &'a str, language: &str) -> Vec<&'a str> {
        find_words(text, language).into_iter().filter_map(|r| text.get(r)).collect()
    }

    #[test]
    fn plain_text_gives_its_words() {
        assert_eq!(words("The quick brown fox.", "en"), ["The", "quick", "brown", "fox"]);
        assert_eq!(words("  two   spaces\nand\ta tab ", "en"), ["two", "spaces", "and", "tab"]);
        assert!(words("", "en").is_empty());
    }

    #[test]
    fn apostrophes_and_hyphens_follow_word_segmentation() {
        assert_eq!(words("don't stop", "en"), ["don't", "stop"]);
        assert_eq!(words("well-known fact", "en"), ["well", "known", "fact"]);
        assert_eq!(words("don\u{2019}t", "en"), ["don\u{2019}t"], "a typographic apostrophe stays inside");
    }

    #[test]
    fn links_addresses_and_domains_are_skipped() {
        assert_eq!(words("see https://example.com/page now", "en"), ["see", "now"]);
        assert_eq!(words("go to www.example.org today", "en"), ["go", "to", "today"]);
        assert_eq!(words("mail bob@example.com please", "en"), ["mail", "please"]);
        assert_eq!(words("open mailto:bob@example.com soon", "en"), ["open", "soon"]);
        assert_eq!(words("visit example.com tomorrow", "en"), ["visit", "tomorrow"]);
        assert_eq!(words("(https://example.com/a_b) is long", "en"), ["is", "long"]);
    }

    #[test]
    fn mentions_channels_commands_and_emoji_codes_are_skipped() {
        assert_eq!(words("thanks @sam_k and #general", "en"), ["thanks", "and"]);
        assert_eq!(words("(@sam) said hi", "en"), ["said", "hi"]);
        assert_eq!(words("/giphy cats are great", "en"), ["cats", "are", "great"]);
        assert_eq!(words("nice :thumbsup: work", "en"), ["nice", "work"]);
        assert_eq!(words("nice :tada:!", "en"), ["nice"]);
    }

    #[test]
    fn numbers_and_words_with_digits_are_skipped() {
        assert_eq!(
            words("on the 3rd of 2026 at 10:30 with mp3 files", "en"),
            ["on", "the", "of", "at", "with", "files"]
        );
        assert_eq!(words("3.14 is pi", "en"), ["is", "pi"]);
    }

    #[test]
    fn code_is_skipped() {
        assert_eq!(words("call `fooo()` first", "en"), ["call", "first"]);
        assert_eq!(words("run ``a b c`` then", "en"), ["run", "then"]);
        assert_eq!(words("```\nlet xx = yy;\n```\nafter", "en"), ["after"]);
        assert_eq!(words("use snake_case here", "en"), ["use", "here"]);
        assert_eq!(words("std::vec and x=1", "en"), ["and"]);
        assert_eq!(words("set the getValue call", "en"), ["set", "the", "call"], "camelCase is a name");
        assert_eq!(words("a lone ` tick", "en"), ["lone", "tick"], "an unmatched backtick opens no span");
    }

    #[test]
    fn acronyms_abbreviations_and_single_letters_are_skipped() {
        assert_eq!(words("NASA and HTTP are acronyms", "en"), ["and", "are", "acronyms"]);
        assert_eq!(words("e.g. this", "en"), ["this"]);
        assert_eq!(words("Plan B or C", "en"), ["Plan", "or"]);
        assert_eq!(words("I am", "en"), ["am"]);
        assert_eq!(words("Hello World", "en"), ["Hello", "World"], "capitalized words stay");
    }

    #[test]
    fn paths_are_skipped_but_and_or_is_two_words() {
        assert_eq!(words("in /usr/share/hunspell now", "en"), ["in", "now"]);
        assert_eq!(words(r"in C:\Users\me now", "en"), ["in", "now"]);
        assert_eq!(words("and/or both", "en"), ["and", "or", "both"]);
    }

    #[test]
    fn ranges_index_the_original_text_including_multibyte_characters() {
        let text = "café über naïve";
        assert_eq!(words(text, "fr"), ["café", "über", "naïve"]);
        let r = find_words(text, "fr");
        assert_eq!(r.first().cloned(), Some(0..5));
    }

    #[test]
    fn other_scripts_are_found_as_words() {
        assert_eq!(words("привет мир", "ru"), ["привет", "мир"]);
        assert_eq!(words("שלום עולם", "he"), ["שלום", "עולם"]);
    }
}
