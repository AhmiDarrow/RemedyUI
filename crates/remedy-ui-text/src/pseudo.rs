//! Pseudo-localization: text rewritten the way translations stress a layout, so layouts can
//! be checked before any translation exists. The same pseudo-locales Android and Chromium use:
//!
//! - `en-XA` ([`Pseudo::Accented`]): every letter accented, about 40% longer, in brackets.
//!   Clipped, truncated or overlapping text shows at once, as does text that skipped
//!   localization (it has no accents).
//! - `ar-XB` ([`Pseudo::Bidi`]): every string forced to run right to left. With the
//!   right-to-left layout of `ar`, it shows mirroring mistakes without an Arabic translation.

use crate::rich::RichText;

/// A pseudo-locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Pseudo {
    /// `en-XA`: accented, longer, bracketed.
    Accented,
    /// `ar-XB`: forced right to left.
    Bidi,
}

impl Pseudo {
    /// The pseudo-locale a BCP 47 tag names, if any (`en-XA`, `ar-XB`; any case).
    pub fn for_locale(tag: &str) -> Option<Pseudo> {
        let t = tag.replace('_', "-").to_ascii_lowercase();
        if t == "en-xa" || t.starts_with("en-xa-") {
            Some(Pseudo::Accented)
        } else if t == "ar-xb" || t.starts_with("ar-xb-") {
            Some(Pseudo::Bidi)
        } else {
            None
        }
    }

    /// `text` rewritten for this pseudo-locale, with its spans and inline objects moved with
    /// the characters they covered.
    pub fn apply(self, text: &RichText) -> RichText {
        let src = text.as_str();
        // Numbers, symbols and empty strings are not translated: they stay as they are.
        if !src.chars().any(char::is_alphabetic) {
            return text.clone();
        }
        let mut out = String::with_capacity(src.len() * 2);
        // map[i] is where byte i of the source landed (one past the end at src.len()).
        let mut map = vec![0usize; src.len() + 1];
        match self {
            Pseudo::Accented => {
                out.push('[');
                for (i, c) in src.char_indices() {
                    for k in i..i + c.len_utf8() {
                        if let Some(m) = map.get_mut(k) {
                            *m = out.len();
                        }
                    }
                    out.push(accent(c));
                }
                if let Some(m) = map.get_mut(src.len()) {
                    *m = out.len();
                }
                // About 40% longer, in whole words (at least one for short strings).
                let letters = src.chars().filter(|c| c.is_alphabetic()).count();
                if letters > 0 {
                    let mut extra = (letters * 2).div_ceil(5).max(3);
                    let mut words = FILLER.iter().cycle();
                    while extra > 0 {
                        let w = words.next().copied().unwrap_or("one");
                        out.push(' ');
                        out.push_str(w);
                        extra = extra.saturating_sub(w.len() + 1);
                    }
                }
                out.push(']');
            }
            Pseudo::Bidi => {
                // Right-to-left override … pop directional formatting.
                out.push('\u{202E}');
                for (i, c) in src.char_indices() {
                    for k in i..i + c.len_utf8() {
                        if let Some(m) = map.get_mut(k) {
                            *m = out.len();
                        }
                    }
                    out.push(c);
                }
                if let Some(m) = map.get_mut(src.len()) {
                    *m = out.len();
                }
                out.push('\u{202C}');
            }
        }
        let at = |i: usize| map.get(i).copied().unwrap_or(out.len());
        let mut rich = RichText::plain(out.clone());
        for s in text.spans() {
            rich.add_span(at(s.range.start)..at(s.range.end), s.patch.clone());
        }
        rich.set_objects(text.objects().iter().map(|(i, o)| (at(*i), *o)).collect());
        rich
    }
}

const FILLER: [&str; 10] = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];

/// An accented look-alike of an ASCII letter (others unchanged), as en-XA uses.
fn accent(c: char) -> char {
    const LOWER: &str = "åƀçđéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýž";
    const UPPER: &str = "ÅƁÇĐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ";
    match c {
        'a'..='z' => LOWER.chars().nth((c as u8 - b'a') as usize).unwrap_or(c),
        'A'..='Z' => UPPER.chars().nth((c as u8 - b'A') as usize).unwrap_or(c),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich::OBJECT_REPLACEMENT;
    use crate::{InlineObject, TextStylePatch};

    #[test]
    fn pseudo_locales_rewrite_text_and_keep_its_styles() {
        assert_eq!(Pseudo::for_locale("en-XA"), Some(Pseudo::Accented));
        assert_eq!(Pseudo::for_locale("ar_xb"), Some(Pseudo::Bidi));
        assert_eq!(Pseudo::for_locale("en-US"), None);
        assert_eq!(Pseudo::Accented.apply(&RichText::plain("17")).as_str(), "17", "numbers stay");
        let t = RichText::new().text("Save ").styled("all", TextStylePatch::bold()).text(" 12 notes");
        let p = Pseudo::Accented.apply(&t);
        let s = p.as_str();
        assert!(s.starts_with("[Šåṽé åļļ 12 ñöţéš") && s.ends_with(']'), "{s}");
        assert!(s.chars().count() as f32 >= t.as_str().chars().count() as f32 * 1.35, "{s}");
        let bold = p.spans().first().map(|sp| s.get(sp.range.clone()).unwrap_or(""));
        assert_eq!(bold, Some("åļļ"), "the span moved with its letters");
        let b = Pseudo::Bidi.apply(&RichText::new().text("Hi ").object(InlineObject {
            id: 1,
            width: 8.0,
            height: 8.0,
            baseline: None,
        }));
        assert!(b.as_str().starts_with('\u{202E}') && b.as_str().ends_with('\u{202C}'));
        let (at, _) = b
            .objects()
            .first()
            .copied()
            .unwrap_or((0, InlineObject { id: 0, width: 0.0, height: 0.0, baseline: None }));
        assert_eq!(
            b.as_str()[at..].chars().next(),
            Some(OBJECT_REPLACEMENT),
            "the object stays on its placeholder"
        );
    }
}
