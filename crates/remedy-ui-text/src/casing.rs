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

/// `text` cased by `base`'s transform and its spans' (`turkic` for Turkish and Azerbaijani
/// casing of i), with spans and objects moved along, or `None` when nothing is transformed.
pub(crate) fn apply(text: &RichText, base: &TextStyle, turkic: bool) -> Option<(RichText, CaseMap)> {
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
    let mut chars = src.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        map.push((i, out.len()));
        let next = chars.peek().map(|&(_, n)| n);
        match transform_at(i) {
            TextTransform::None => out.push(c),
            TextTransform::Upper => upper(c, turkic, &mut out),
            TextTransform::Lower => lower(c, prev, next, turkic, &mut out),
            TextTransform::Capitalize => {
                if prev.is_some_and(|p| p.is_alphanumeric() || p == '\'' || p == '\u{2019}') {
                    out.push(c);
                } else {
                    title(c, turkic, &mut out);
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

fn upper(c: char, turkic: bool, out: &mut String) {
    match c {
        'i' if turkic => out.push('İ'),
        // shortcut: Greek keeps its accents in capitals (CSS asks browsers to drop the
        // tonos); upgrade if a Greek interface needs it.
        _ => out.extend(c.to_uppercase()),
    }
}

fn lower(c: char, prev: Option<char>, next: Option<char>, turkic: bool, out: &mut String) {
    match c {
        'I' if turkic => out.push('ı'),
        'İ' if turkic => out.push('i'),
        // Final sigma: at the end of a word, after a letter.
        'Σ' if prev.is_some_and(char::is_alphabetic) && !next.is_some_and(char::is_alphabetic) => {
            out.push('ς')
        }
        _ => out.extend(c.to_lowercase()),
    }
}

/// The title case of a word's first letter: the capital, except where Unicode has a title
/// form of its own (ß → Ss, the dž, lj, nj and dz digraphs).
fn title(c: char, turkic: bool, out: &mut String) {
    match c {
        'ß' => out.push_str("Ss"),
        'Ǆ' | 'ǅ' | 'ǆ' => out.push('ǅ'),
        'Ǉ' | 'ǈ' | 'ǉ' => out.push('ǈ'),
        'Ǌ' | 'ǋ' | 'ǌ' => out.push('ǋ'),
        'Ǳ' | 'ǲ' | 'ǳ' => out.push('ǲ'),
        _ => upper(c, turkic, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextStylePatch;

    fn cased(s: &str, t: TextTransform, turkic: bool) -> String {
        let base = TextStyle { transform: t, ..TextStyle::default() };
        apply(&RichText::plain(s), &base, turkic).map(|(r, _)| r.as_str().to_owned()).unwrap_or_default()
    }

    #[test]
    fn casing_follows_the_language() {
        assert_eq!(cased("straße", TextTransform::Upper, false), "STRASSE");
        assert_eq!(cased("istanbul", TextTransform::Upper, true), "İSTANBUL");
        assert_eq!(cased("istanbul", TextTransform::Upper, false), "ISTANBUL");
        assert_eq!(cased("DİYARBAKIR", TextTransform::Lower, true), "diyarbakır");
        assert_eq!(cased("ΟΔΟΣ ΣΟΦΙΑΣ", TextTransform::Lower, false), "οδος σοφιας");
        assert_eq!(cased("ΟΔΟΣ", TextTransform::Lower, false), "οδος");
        assert_eq!(cased("Σ", TextTransform::Lower, false), "σ", "a lone sigma is not final");
        assert_eq!(
            cased("up next: o'neil ßtraße", TextTransform::Capitalize, false),
            "Up Next: O'neil Sstraße"
        );
        assert_eq!(cased("ǆungla", TextTransform::Capitalize, false), "ǅungla");
        assert_eq!(cased("izmir", TextTransform::Capitalize, true), "İzmir");
    }

    #[test]
    fn offsets_map_both_ways_and_spans_follow() {
        let t = RichText::new().text("ß ").styled("x", TextStylePatch::bold()).text(" i");
        let base = TextStyle { transform: TextTransform::Upper, ..TextStyle::default() };
        let (r, map) = apply(&t, &base, true).unwrap_or_default();
        assert_eq!(r.as_str(), "SS X İ");
        assert_eq!(map.to_cased(3), 3);
        assert_eq!(map.to_original(1), 0, "inside SS is the ß");
        assert_eq!(map.to_original(r.as_str().len()), t.as_str().len());
        let bold = r.spans().first().map(|s| &r.as_str()[s.range.clone()]);
        assert_eq!(bold, Some("X"));
        let mut lower_span = RichText::plain("ab");
        lower_span.add_span(0..1, TextStylePatch::transform(TextTransform::None));
        assert_eq!(
            apply(&lower_span, &base, false).map(|(r, _)| r.as_str().to_owned()).as_deref(),
            Some("aB")
        );
    }
}
