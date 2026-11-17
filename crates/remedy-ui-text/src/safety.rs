//! Defenses against text that deceives: hidden direction controls and look-alike names.
//!
//! - Bidi controls (the "Trojan Source" class) can make text display in a different order
//!   than it is stored, so a link or a name reads as something it is not. [`reveal_controls`]
//!   makes them visible in text the app marks as security-sensitive.
//! - Confusable characters (Cyrillic а for Latin a, and thousands more) let one name pass for
//!   another. [`skeleton`] maps a string to its UTS #39 skeleton; two names with the same
//!   skeleton look alike.

/// What [`reveal_controls`] makes visible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reveal {
    /// Also reveal zero-width spaces, word joiners and byte-order marks, which hide breaks
    /// and boundaries. Zero-width joiners and non-joiners are never revealed: emoji and many
    /// scripts need them.
    pub zero_width: bool,
}

fn control_name(c: char, reveal: Reveal) -> Option<&'static str> {
    Some(match c {
        '\u{200E}' => "LRM",
        '\u{200F}' => "RLM",
        '\u{061C}' => "ALM",
        '\u{202A}' => "LRE",
        '\u{202B}' => "RLE",
        '\u{202C}' => "PDF",
        '\u{202D}' => "LRO",
        '\u{202E}' => "RLO",
        '\u{2066}' => "LRI",
        '\u{2067}' => "RLI",
        '\u{2068}' => "FSI",
        '\u{2069}' => "PDI",
        '\u{200B}' if reveal.zero_width => "ZWSP",
        '\u{2060}' if reveal.zero_width => "WJ",
        '\u{FEFF}' if reveal.zero_width => "BOM",
        _ => return None,
    })
}

/// True when `s` holds a bidi control (or, with `reveal.zero_width`, an invisible break).
pub fn has_hidden_controls(s: &str, reveal: Reveal) -> bool {
    s.chars().any(|c| control_name(c, reveal).is_some())
}

/// `s` with each hidden control replaced by a visible tag such as `⟨RLO⟩`, so text that
/// would reorder itself shows what it contains.
///
/// ```
/// use remedy_ui_text::{Reveal, reveal_controls};
/// let tricky = "invoice\u{202E}fdp.exe";
/// assert_eq!(reveal_controls(tricky, Reveal::default()), "invoice⟨RLO⟩fdp.exe");
/// ```
pub fn reveal_controls(s: &str, reveal: Reveal) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match control_name(c, reveal) {
            Some(name) => {
                out.push('⟨');
                out.push_str(name);
                out.push('⟩');
            }
            None => out.push(c),
        }
    }
    out
}

/// The UTS #39 skeleton of `s`: strings that look alike have equal skeletons.
pub fn skeleton(s: &str) -> String {
    unicode_security::skeleton(s).collect()
}

/// True when `a` and `b` would look alike on screen (equal skeletons) but are different
/// strings: the case to warn about for usernames, room names and domains.
///
/// ```
/// use remedy_ui_text::looks_like;
/// assert!(looks_like("paypal", "раураl")); // Cyrillic р, а, у
/// assert!(!looks_like("paypal", "paypal"));
/// ```
pub fn looks_like(a: &str, b: &str) -> bool {
    a != b && skeleton(a) == skeleton(b)
}
