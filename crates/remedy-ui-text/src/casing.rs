//! Text transforms (CSS `text-transform`): the text is drawn in capitals, small letters or
//! with each word capitalized, while copying, selection and screen readers keep the text as
//! written. The paragraph is laid out from the cased text and maps its byte offsets back.

use crate::rich::RichText;
use crate::style::{TextStyle, TextTransform};

/// Where each character of the original text landed in the cased text: `(original, cased)`
/// byte offsets at every character boundary, the text's end included, both increasing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CaseMap(Vec<(usize, usize)>);

impl CaseMap {
    /// The cased offset of original offset `i`.
    pub(crate) fn to_cased(&self, i: usize) -> usize {
        let k = self.0.partition_point(|&(o, _)| o <= i);
        self.0.get(k.saturating_sub(1)).map_or(i, |&(_, c)| c)
    }

    /// The original offset of cased offset `j`, at the start of the character it came from.
    pub(crate) fn to_original(&self, j: usize) -> usize {
        let k = self.0.partition_point(|&(_, c)| c <= j);
        self.0.get(k.saturating_sub(1)).map_or(j, |&(o, _)| o)
    }
}

/// The casing rules a language adds to Unicode's default ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Lang {
    /// Unicode's default casing.
    #[default]
    Default,
    /// Turkish and Azerbaijani (and the other Turkic languages with dotted and dotless i).
    Turkic,
    /// Greek: capitals drop their accents and breathings, as ICU's Greek upper case.
    Greek,
    /// Dutch: a word that starts with "ij" capitalizes both letters ("IJsland").
    Dutch,
}

impl Lang {
    /// The rules for a BCP 47 locale.
    pub(crate) fn for_locale(locale: &str) -> Lang {
        match locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase().as_str() {
            "tr" | "az" | "crh" | "tt" | "ba" => Lang::Turkic,
            "el" => Lang::Greek,
            "nl" => Lang::Dutch,
            _ => Lang::Default,
        }
    }
}

/// `text` cased by `base`'s transform and its spans' under `lang`'s rules, with spans and
/// objects moved along, or `None` when nothing is transformed.
pub(crate) fn apply(text: &RichText, base: &TextStyle, lang: Lang) -> Option<(RichText, CaseMap)> {
    let spans = text.spans();
    if base.transform == TextTransform::None && spans.iter().all(|s| s.patch.transform.is_none()) {
        return None;
    }
    let src = text.as_str();
    let transform_at = |i: usize| {
        spans
            .iter()
            .filter(|s| s.range.contains(&i))
            .fold(base.transform, |t, s| s.patch.transform.unwrap_or(t))
    };
    let mut out = String::with_capacity(src.len() + src.len() / 8);
    let mut map = Vec::with_capacity(src.len() + 1);
    let mut prev: Option<char> = None;
    // Set by the letter before for this one: a Greek ι or υ that takes a dialytika (its
    // diphthong was broken by an accent the capitals drop), or Dutch "ij"'s j.
    let mut carried = false;
    let mut chars = src.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        map.push((i, out.len()));
        let next = chars.peek().map(|&(_, n)| n);
        let after = {
            let mut rest = chars.clone();
            rest.next();
            rest.next().map(|(_, n)| n)
        };
        let carry = std::mem::take(&mut carried);
        match transform_at(i) {
            TextTransform::None => out.push(c),
            TextTransform::Upper if lang == Lang::Greek && is_greek(c) => {
                carried = greek_upper(c, prev, next, after, carry, &mut out);
            }
            TextTransform::Upper => upper(c, lang, &mut out),
            TextTransform::Lower => lower(c, prev, next, lang, &mut out),
            TextTransform::Capitalize => {
                if carry {
                    out.extend(c.to_uppercase());
                } else if prev.is_some_and(|p| p.is_alphanumeric() || p == '\'' || p == '\u{2019}') {
                    out.push(c);
                } else {
                    carried =
                        lang == Lang::Dutch && matches!(c, 'i' | 'I') && matches!(next, Some('j' | 'J'));
                    title(c, lang, &mut out);
                }
            }
        }
        prev = Some(c);
    }
    map.push((src.len(), out.len()));
    let map = CaseMap(map);
    let mut rich = RichText::plain(out);
    for s in spans {
        rich.add_span(map.to_cased(s.range.start)..map.to_cased(s.range.end), s.patch.clone());
    }
    rich.set_objects(text.objects().iter().map(|(i, o)| (map.to_cased(*i), *o)).collect());
    Some((rich, map))
}

fn upper(c: char, lang: Lang, out: &mut String) {
    match c {
        'i' if lang == Lang::Turkic => out.push('İ'),
        _ => out.extend(c.to_uppercase()),
    }
}

fn lower(c: char, prev: Option<char>, next: Option<char>, lang: Lang, out: &mut String) {
    match c {
        'I' if lang == Lang::Turkic => out.push('ı'),
        'İ' if lang == Lang::Turkic => out.push('i'),
        // Final sigma: at the end of a word, after a letter.
        'Σ' if prev.is_some_and(char::is_alphabetic) && !next.is_some_and(char::is_alphabetic) => {
            out.push('ς')
        }
        _ => out.extend(c.to_lowercase()),
    }
}

/// The title case of a word's first letter: the capital, except where Unicode has a title
/// form of its own (ß → Ss, the dž, lj, nj and dz digraphs).
fn title(c: char, lang: Lang, out: &mut String) {
    match c {
        'ß' => out.push_str("Ss"),
        'Ǆ' | 'ǅ' | 'ǆ' => out.push('ǅ'),
        'Ǉ' | 'ǈ' | 'ǉ' => out.push('ǈ'),
        'Ǌ' | 'ǋ' | 'ǌ' => out.push('ǋ'),
        'Ǳ' | 'ǲ' | 'ǳ' => out.push('ǲ'),
        _ => upper(c, lang, out),
    }
}

/// Greek letters and the marks Greek writes on them.
fn is_greek(c: char) -> bool {
    matches!(c, '\u{370}'..='\u{3FF}' | '\u{1F00}'..='\u{1FFF}') || is_greek_mark(c)
}

/// The accents and breathings Greek capitals drop (tonos and oxia, varia, perispomeni,
/// psili, dasia, koronis, macron, vrachy) and the dialytika with tonos, which keeps its
/// dialytika.
fn is_greek_mark(c: char) -> bool {
    matches!(
        c,
        '\u{300}'
            | '\u{301}'
            | '\u{304}'
            | '\u{306}'
            | '\u{313}'
            | '\u{314}'
            | '\u{342}'
            | '\u{343}'
            | '\u{344}'
    )
}

/// A Greek letter in capitals as ICU's Greek upper case writes it: accents and breathings
/// dropped, the dialytika kept (and added to an ι or υ whose diphthong the dropped accent had
/// broken: άι → ΑΪ), the ypogegrammeni as a capital iota, and the disjunctive ή alone keeping
/// its tonos. `carry` is the dialytika the letter before handed on; returns whether this one
/// hands one to the next.
fn greek_upper(
    c: char,
    prev: Option<char>,
    next: Option<char>,
    after: Option<char>,
    carry: bool,
    out: &mut String,
) -> bool {
    let nfd = icu_normalizer::DecomposingNormalizerBorrowed::new_nfd();
    let decomposed = nfd.normalize(&c.to_string()).into_owned();
    let mut parts = decomposed.chars();
    let Some(letter) = parts.next() else { return false };
    let marks: Vec<char> = parts.collect();
    // A mark on its own (text already decomposed): the letter before was written already.
    if marks.is_empty() && letter != c || is_greek_mark(letter) || matches!(letter, '\u{308}' | '\u{345}') {
        return match letter {
            '\u{308}' => {
                out.push('\u{308}');
                false
            }
            '\u{344}' => {
                out.push('\u{308}');
                false
            }
            '\u{345}' => {
                out.push('Ι');
                false
            }
            _ => breaks_diphthong(prev, next, after),
        };
    }
    let accented = marks.iter().any(|&m| is_greek_mark(m));
    let dialytika = carry || marks.iter().any(|&m| matches!(m, '\u{308}' | '\u{344}'));
    let alone = !prev.is_some_and(char::is_alphabetic) && !next.is_some_and(char::is_alphabetic);
    let mut piece: String = letter.to_uppercase().collect();
    if dialytika {
        piece.push('\u{308}');
    }
    if matches!(letter, 'η' | 'Η') && marks.contains(&'\u{301}') && alone {
        piece.push('\u{301}');
    }
    if marks.contains(&'\u{345}') {
        piece.push('Ι');
    }
    out.push_str(&icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(&piece));
    accented && !dialytika && breaks_diphthong(Some(letter), next, after)
}

/// Whether the vowel `first` and the bare ι or υ `next` would read as a diphthong in
/// capitals (αι, ει, οι, υι, αυ, ευ, ηυ, ου), so the ι or υ needs a dialytika once the accent
/// on `first` is gone. `after` is the character after `next`: an accent there keeps it apart.
fn breaks_diphthong(first: Option<char>, next: Option<char>, after: Option<char>) -> bool {
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let Some(first) = first.map(lower) else { return false };
    let first = icu_normalizer::DecomposingNormalizerBorrowed::new_nfd()
        .normalize(&first.to_string())
        .chars()
        .next()
        .unwrap_or(first);
    let pairs = match next.map(lower) {
        Some('ι') => "αεου",
        Some('υ') => "αεηο",
        _ => return false,
    };
    pairs.contains(first) && !after.is_some_and(|a| is_greek_mark(a) || a == '\u{308}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextStylePatch;

    fn cased(s: &str, t: TextTransform, lang: Lang) -> String {
        let base = TextStyle { transform: t, ..TextStyle::default() };
        apply(&RichText::plain(s), &base, lang).map(|(r, _)| r.as_str().to_owned()).unwrap_or_default()
    }

    #[test]
    fn casing_follows_the_language() {
        assert_eq!(cased("straße", TextTransform::Upper, Lang::Default), "STRASSE");
        assert_eq!(cased("istanbul", TextTransform::Upper, Lang::Turkic), "İSTANBUL");
        assert_eq!(cased("istanbul", TextTransform::Upper, Lang::Default), "ISTANBUL");
        assert_eq!(cased("DİYARBAKIR", TextTransform::Lower, Lang::Turkic), "diyarbakır");
        assert_eq!(cased("ΟΔΟΣ ΣΟΦΙΑΣ", TextTransform::Lower, Lang::Default), "οδος σοφιας");
        assert_eq!(cased("ΟΔΟΣ", TextTransform::Lower, Lang::Default), "οδος");
        assert_eq!(cased("Σ", TextTransform::Lower, Lang::Default), "σ", "a lone sigma is not final");
        assert_eq!(
            cased("up next: o'neil ßtraße", TextTransform::Capitalize, Lang::Default),
            "Up Next: O'neil Sstraße"
        );
        assert_eq!(cased("ǆungla", TextTransform::Capitalize, Lang::Default), "ǅungla");
        assert_eq!(cased("izmir", TextTransform::Capitalize, Lang::Turkic), "İzmir");
    }

    #[test]
    fn greek_capitals_drop_accents_and_keep_the_dialytika() {
        let up = |s: &str| cased(s, TextTransform::Upper, Lang::Greek);
        assert_eq!(up("άλφα"), "ΑΛΦΑ");
        assert_eq!(up("Καλημέρα κόσμε"), "ΚΑΛΗΜΕΡΑ ΚΟΣΜΕ");
        assert_eq!(up("προϊόν"), "ΠΡΟΪΟΝ", "the dialytika stays");
        assert_eq!(up("ΐ"), "Ϊ", "dialytika with tonos keeps the dialytika");
        assert_eq!(up("άι"), "ΑΪ", "the broken diphthong takes a dialytika");
        assert_eq!(up("αι"), "ΑΙ", "a real diphthong does not");
        assert_eq!(up("ή"), "Ή", "the disjunctive eta keeps its tonos");
        assert_eq!(up("ἀρετή"), "ΑΡΕΤΗ", "breathings go too");
        assert_eq!(up("ᾳ"), "ΑΙ", "ypogegrammeni as a capital iota");
        assert_eq!(up("α\u{301}λφα"), "ΑΛΦΑ", "decomposed text");
        // Other languages keep Unicode's default capitals.
        assert_eq!(cased("άλφα", TextTransform::Upper, Lang::Default), "ΆΛΦΑ");
        assert_eq!(Lang::for_locale("el-GR"), Lang::Greek);
    }

    #[test]
    fn dutch_capitalizes_ij_together() {
        let title = |s: &str, l| cased(s, TextTransform::Capitalize, l);
        assert_eq!(title("ijsland en ijmuiden", Lang::Dutch), "IJsland En IJmuiden");
        assert_eq!(title("ijsland", Lang::Default), "Ijsland");
        assert_eq!(title("india", Lang::Dutch), "India");
        assert_eq!(Lang::for_locale("nl_BE"), Lang::Dutch);
    }

    #[test]
    fn offsets_map_both_ways_and_spans_follow() {
        let t = RichText::new().text("ß ").styled("x", TextStylePatch::bold()).text(" i");
        let base = TextStyle { transform: TextTransform::Upper, ..TextStyle::default() };
        let (r, map) = apply(&t, &base, Lang::Turkic).unwrap_or_default();
        assert_eq!(r.as_str(), "SS X İ");
        assert_eq!(map.to_cased(3), 3);
        assert_eq!(map.to_original(1), 0, "inside SS is the ß");
        assert_eq!(map.to_original(r.as_str().len()), t.as_str().len());
        let bold = r.spans().first().map(|s| &r.as_str()[s.range.clone()]);
        assert_eq!(bold, Some("X"));
        let mut lower_span = RichText::plain("ab");
        lower_span.add_span(0..1, TextStylePatch::transform(TextTransform::None));
        assert_eq!(
            apply(&lower_span, &base, Lang::Default).map(|(r, _)| r.as_str().to_owned()).as_deref(),
            Some("aB")
        );
    }
}
