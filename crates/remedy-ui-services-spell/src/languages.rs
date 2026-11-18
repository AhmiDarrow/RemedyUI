//! Language tags, and which scripts a language is written in.

use icu_locale_core::LanguageIdentifier;

use crate::SpellError;

/// A language tag in its standard form (`en-US`, `de`, `sr-Latn`), or an error for a tag that
/// is not BCP 47. The pseudo-locales `en-XA` and `ar-XB` (used to test translations) stand for
/// their plain language.
pub fn normalize(tag: &str) -> Result<String, SpellError> {
    let tag = tag.trim().replace('_', "-");
    let id = LanguageIdentifier::try_from_str(&tag).map_err(|_| SpellError::BadLanguage(tag.clone()))?;
    if id.language.is_unknown() {
        return Err(SpellError::BadLanguage(tag));
    }
    let pseudo = id.region.is_some_and(|r| matches!(r.as_str(), "XA" | "XB"));
    if pseudo {
        return Ok(id.language.as_str().to_owned());
    }
    Ok(id.to_string())
}

/// The language part of a normalized tag (`en` for `en-US`).
pub(crate) fn primary(tag: &str) -> &str {
    tag.split('-').next().unwrap_or(tag)
}

/// Writing systems, as far as deciding whether a dictionary can judge a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Devanagari,
    Han,
    Kana,
    Hangul,
    Thai,
    /// Any other script with letters of its own (Bengali, Tamil, Georgian, …).
    Other,
}

/// The script of a letter, or `None` for characters that belong to no one script (digits,
/// apostrophes, hyphens, combining marks, symbols).
pub(crate) fn script_of(c: char) -> Option<Script> {
    if !c.is_alphabetic() {
        return None;
    }
    let u = u32::from(c);
    Some(match u {
        0x0041..=0x024F
        | 0x1E00..=0x1EFF
        | 0x2C60..=0x2C7F
        | 0xA720..=0xA7FF
        | 0xFF21..=0xFF3A
        | 0xFF41..=0xFF5A => Script::Latin,
        0x0250..=0x02AF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Script::Cyrillic,
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            Script::Arabic
        }
        0x0900..=0x097F | 0xA8E0..=0xA8FF => Script::Devanagari,
        0x0E00..=0x0E7F => Script::Thai,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Script::Kana,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Hangul,
        0x2E80..=0x2FDF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF => {
            Script::Han
        }
        _ => Script::Other,
    })
}

/// The scripts a language is written in.
pub(crate) fn scripts_of_language(tag: &str) -> &'static [Script] {
    match primary(tag) {
        "ru" | "uk" | "bg" | "be" | "mk" | "kk" | "ky" | "mn" | "tg" | "tt" | "ba" | "cv" => {
            &[Script::Cyrillic]
        }
        "sr" => {
            if tag.contains("Latn") {
                &[Script::Latin, Script::Cyrillic]
            } else {
                &[Script::Cyrillic, Script::Latin]
            }
        }
        "el" => &[Script::Greek],
        "ar" | "fa" | "ur" | "ps" | "sd" | "ug" | "ckb" | "ku" => &[Script::Arabic],
        "he" | "iw" | "yi" => &[Script::Hebrew],
        "hi" | "mr" | "ne" | "sa" | "kok" => &[Script::Devanagari],
        "th" => &[Script::Thai],
        "ja" => &[Script::Han, Script::Kana],
        "zh" => &[Script::Han],
        "ko" => &[Script::Hangul, Script::Han],
        "bn" | "gu" | "pa" | "ta" | "te" | "kn" | "ml" | "si" | "lo" | "km" | "my" | "ka" | "hy" | "am"
        | "ti" | "or" | "as" => &[Script::Other],
        _ => &[Script::Latin],
    }
}

/// Whether `word` is written in a script `language` uses. A word in another script (Japanese
/// inside English text) is not for that language's dictionary to judge.
pub(crate) fn in_script_of(word: &str, language: &str) -> bool {
    let allowed = scripts_of_language(language);
    let mut letters = word.chars().filter_map(script_of).peekable();
    letters.peek().is_some() && letters.all(|s| allowed.contains(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_normalized_and_bad_ones_refused() {
        assert_eq!(normalize("en_US").ok().as_deref(), Some("en-US"));
        assert_eq!(normalize(" de ").ok().as_deref(), Some("de"));
        assert_eq!(normalize("en-XA").ok().as_deref(), Some("en"), "a pseudo-locale is its language");
        assert!(matches!(normalize("not a tag!"), Err(SpellError::BadLanguage(_))));
        assert!(matches!(normalize(""), Err(SpellError::BadLanguage(_))));
    }

    #[test]
    fn dictionaries_judge_only_words_in_their_script() {
        assert!(in_script_of("naïve", "en-US"));
        assert!(!in_script_of("日本語", "en-US"));
        assert!(!in_script_of("привет", "en-US"));
        assert!(in_script_of("привет", "ru"));
        assert!(in_script_of("don't", "en"), "an apostrophe belongs to no script");
        assert!(!in_script_of("123", "en"), "no letters, nothing to judge");
        assert!(in_script_of("日本語", "ja"));
        assert!(in_script_of("שלום", "he"));
        assert!(in_script_of("γειά", "el"));
    }
}
