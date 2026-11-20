//! RemedyUI's own words, in the window's language.
//!
//! The engine shows a few hundred words of its own: the names of icon buttons ("Close"),
//! the password field's "Show password", menu items such as "Add to dictionary", the media
//! player's controls, the customization pages. This crate holds every one of them with
//! built-in translations for the locales RemedyUI ships, so an app shows them in the
//! person's language without translating the engine itself.
//!
//! Widgets ask through `Ui::word` (in `remedy-ui-view`, re-exported by `remedy-ui`), which reads
//! the window's tracked locale (so they show the new words when it changes) and falls back to
//! the English. Code without a `Ui`, such as a bridge that knows only a locale string, calls
//! [`word`] directly:
//!
//! ```
//! // English, unknown locales and missing translations give `None`: show the English.
//! let close = remedy_ui_words::word("en-GB", "Close").unwrap_or("Close");
//! assert_eq!(close, "Close");
//! // Matching: Mexican Spanish reads the Spanish words, Traditional Chinese none.
//! assert_eq!(remedy_ui_words::best_match("es-MX").map(|l| l.tag), Some("es"));
//! assert_eq!(remedy_ui_words::best_match("zh-TW"), None);
//! ```
//!
//! # The files
//!
//! `words/en.json` is the inventory: a JSON object whose keys are the English words. It has
//! the source-keyed shape of `remedy_ui_services_i18n::Catalog` files
//! (`{"Close": "Close", …}`), so apps can reuse or override them. `{name}` placeholders are
//! filled after translation; a translation keeps them and may move them.
//!
//! `words/<tag>.json` holds one shipped locale (see [`shipped`]): the same keys, each with its
//! translation. They are embedded in the binary and parsed once, on first use.
//!
//! **Counted words** change with a number ("1 reaction", "{count} reactions"). Their key is
//! the English plural, and in `en.json` their value is the English forms, singular first,
//! separated by `" || "` ([`SEPARATOR`]). A translation holds the forms of its language's
//! plural rule ([`Locale::plural`]) in the rule's order, separated the same way. Code asks
//! with [`counted`] (or `Ui::counted`).
//!
//! # Keeping the inventory complete
//!
//! A test of this crate reads the workspace's sources like a gettext extractor: every
//! `.word("…")`, `.counted("…", "…", n)`, `words::word(locale, "…")`, `words::word_or(locale, "…")`,
//! `words::counted(locale, "…", "…", n)`, `words::forms(locale, "…", "…")` and
//! [`en("…")`](en) call, and every default of a `*Labels` struct, outside tests. The set must
//! equal `en.json`'s keys exactly. After adding or changing a word, rewrite `en.json` with
//!
//! ```text
//! REMEDY_WRITE_WORDS=1 cargo test -p remedy-ui-words
//! ```
//!
//! and commit it. Translators then fill the new keys in each `<tag>.json`. Other tests check
//! that every shipped file parses, has no key `en.json` lacks, and keeps each key's
//! placeholders (in every form of a counted word).

use std::collections::HashMap;
use std::sync::OnceLock;

mod json;
mod locale;

pub use json::{ParseError, parse};
pub use locale::{plural_form, plural_form_count};

/// Separates the forms of a counted word: `"{count} reaction || {count} reactions"`.
pub const SEPARATOR: &str = " || ";

/// A locale RemedyUI ships its words in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Locale {
    /// The BCP 47 tag, such as `pt-BR` or `zh-Hans`.
    pub tag: &'static str,
    /// The language's name in itself, for a language menu: "Español", "日本語".
    pub native_name: &'static str,
    /// Written right to left.
    pub rtl: bool,
    /// The plural rule counted words follow, which also fixes how many forms they have:
    /// `"one-other"` (1 | other), `"zero-one"` (0 and 1 | other), `"slavic"` (1, 21, 31… |
    /// 2–4, 22–24… | other), `"arabic"` (0 | 1 | 2 | 3–10 | 11–99 | other, by the last two
    /// digits) or `"none"` (one form). See [`plural_form`].
    pub plural: &'static str,
}

impl Locale {
    /// Which of this locale's forms a count of `n` takes (from 0).
    pub fn form(&self, n: u64) -> usize {
        plural_form(self.plural, n)
    }

    /// How many forms a counted word has in this locale.
    pub fn form_count(&self) -> usize {
        plural_form_count(self.plural)
    }
}

/// The shipped locales, in the order their files are listed.
static SHIPPED: [Locale; 10] = [
    Locale { tag: "es", native_name: "Español", rtl: false, plural: "one-other" },
    Locale { tag: "zh-Hans", native_name: "简体中文", rtl: false, plural: "none" },
    Locale { tag: "hi", native_name: "हिन्दी", rtl: false, plural: "zero-one" },
    Locale { tag: "ar", native_name: "العربية", rtl: true, plural: "arabic" },
    Locale { tag: "fr", native_name: "Français", rtl: false, plural: "zero-one" },
    Locale { tag: "pt-BR", native_name: "Português (Brasil)", rtl: false, plural: "zero-one" },
    Locale { tag: "ru", native_name: "Русский", rtl: false, plural: "slavic" },
    Locale { tag: "ja", native_name: "日本語", rtl: false, plural: "none" },
    Locale { tag: "de", native_name: "Deutsch", rtl: false, plural: "one-other" },
    Locale { tag: "bn", native_name: "বাংলা", rtl: false, plural: "zero-one" },
];

/// Each shipped locale's file, in [`SHIPPED`]'s order.
static FILES: [&str; 10] = [
    include_str!("../words/es.json"),
    include_str!("../words/zh-Hans.json"),
    include_str!("../words/hi.json"),
    include_str!("../words/ar.json"),
    include_str!("../words/fr.json"),
    include_str!("../words/pt-BR.json"),
    include_str!("../words/ru.json"),
    include_str!("../words/ja.json"),
    include_str!("../words/de.json"),
    include_str!("../words/bn.json"),
];

/// The English inventory.
#[cfg(test)]
static ENGLISH: &str = include_str!("../words/en.json");

/// The locales RemedyUI ships translations of its own words for.
pub fn shipped() -> &'static [Locale] {
    &SHIPPED
}

/// The shipped locale that serves `locale` (a BCP 47 tag; `_` works as `-`, case does not
/// matter), or `None` for English, the pseudo-locales `en-XA` and `ar-XB`, and languages
/// RemedyUI does not ship.
///
/// An exact tag matches first; then the language and script, so `zh-CN`, `zh-SG` and
/// `zh-Hans-HK` read Simplified Chinese, while `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant` match
/// nothing (Traditional readers get English rather than the wrong script); then the language
/// alone, so `pt-PT` reads `pt-BR`, `es-MX` reads `es` and `fr-CA` reads `fr`.
pub fn best_match(locale: &str) -> Option<&'static Locale> {
    locale::best(&SHIPPED, locale)
}

/// Each shipped locale's table, parsed on first use. A file that does not parse gives an
/// empty table (its words show in English); the tests keep that from shipping.
fn tables() -> &'static [HashMap<String, String>] {
    static TABLES: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();
    TABLES.get_or_init(|| {
        FILES.iter().map(|text| parse(text).map(|pairs| pairs.into_iter().collect()).unwrap_or_default()).collect()
    })
}

/// The raw translation of `english` in the shipped locale `l` from `tables` (in
/// [`SHIPPED`]'s order), when it has a non-empty one.
fn lookup<'t>(tables: &'t [HashMap<String, String>], l: &Locale, english: &str) -> Option<&'t str> {
    let i = SHIPPED.iter().position(|s| s.tag == l.tag)?;
    tables.get(i)?.get(english).map(String::as_str).filter(|t| !t.trim().is_empty())
}

/// [`word`] over `tables`.
fn word_in<'t>(tables: &'t [HashMap<String, String>], locale: &str, english: &str) -> Option<&'t str> {
    lookup(tables, best_match(locale)?, english)
}

/// [`counted`] over `tables`.
fn counted_in<'t>(tables: &'t [HashMap<String, String>], locale: &str, other: &str, n: u64) -> Option<&'t str> {
    let l = best_match(locale)?;
    Some(nth_form(lookup(tables, l, other)?, l.form(n)))
}

/// `english` in the shipped locale that best serves `locale` ([`best_match`]), or `None`
/// for English, unknown locales and words not translated yet: show the English then.
///
/// For a counted word this is every form, separated by [`SEPARATOR`]; use [`counted`].
pub fn word(locale: &str, english: &str) -> Option<&'static str> {
    word_in(tables(), locale, english)
}

/// `english` in the shipped locale that best serves `locale`, or `english` itself: [`word`]
/// with its fallback, for code that knows only a locale string.
///
/// ```
/// assert_eq!(remedy_ui_words::word_or("en-US", "Close"), "Close");
/// ```
pub fn word_or(locale: &str, english: &'static str) -> &'static str {
    word(locale, english).unwrap_or(english)
}

/// Every form of the counted word whose English forms are `one` and `other`, in the locale
/// that best serves `locale`, separated by [`SEPARATOR`] (pick one with [`pick`]); `None`
/// when that locale has no translation. For words kept as a whole, such as a `*Labels`
/// field. `one` is the English singular: with `other` it names the word for the inventory.
pub fn forms(locale: &str, one: &str, other: &str) -> Option<&'static str> {
    let _ = one;
    word(locale, other)
}

/// The form of a counted word that a count of `n` takes in the locale that best serves
/// `locale`, with its placeholders (`{count}` and any other) still to fill; `None` when that
/// locale has no translation, and English shows `one` for 1 and `other` otherwise.
///
/// ```
/// // No shipped translation here: the caller shows the English.
/// let n = 3;
/// let text = remedy_ui_words::counted("en", "{count} reaction", "{count} reactions", n)
///     .unwrap_or(if n == 1 { "{count} reaction" } else { "{count} reactions" });
/// assert_eq!(remedy_ui_words::fill(text, &[("count", &n.to_string())]), "3 reactions");
/// ```
pub fn counted(locale: &str, one: &str, other: &str, n: u64) -> Option<&'static str> {
    let _ = one;
    counted_in(tables(), locale, other, n)
}

/// The form a count of `n` takes from `forms` (several separated by [`SEPARATOR`]), by the
/// plural rule of the locale that best serves `locale`; English's rule (`"one-other"`) for
/// locales RemedyUI does not ship. Text with no separator is its own only form.
pub fn pick<'a>(locale: &str, forms: &'a str, n: u64) -> &'a str {
    let rule = best_match(locale).map_or("one-other", |l| l.plural);
    nth_form(forms, plural_form(rule, n))
}

/// Form `i` of `all`, or its last form when it has fewer.
fn nth_form(all: &str, i: usize) -> &str {
    let mut last = all;
    for (k, f) in all.split(SEPARATOR).enumerate() {
        last = f;
        if k == i {
            break;
        }
    }
    last
}

/// Marks `english` as one of the engine's words without translating it, for words kept in a
/// table and looked up later by value (gettext's `N_`): the inventory test finds it here.
pub const fn en(english: &'static str) -> &'static str {
    english
}

/// `template` with each `{name}` filled from `args`, in one pass (a value's own braces are
/// left alone). Names without a value stay as written.
///
/// ```
/// let s = remedy_ui_words::fill("Close {name}", &[("name", "{tab}")]);
/// assert_eq!(s, "Close {tab}");
/// ```
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open + 1..).unwrap_or_default();
        let value = after
            .find('}')
            .and_then(|close| Some((after.get(..close)?, close)))
            .and_then(|(name, close)| args.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v, close)));
        match value {
            Some((v, close)) => {
                out.push_str(v);
                rest = after.get(close + 1..).unwrap_or_default();
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
