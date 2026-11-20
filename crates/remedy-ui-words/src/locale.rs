//! Matching a requested locale to a shipped one, and the plural rules of counted words.

use crate::Locale;

/// A tag's language, script and region, lowercased. Extensions (`-u-…`, `-x-…`) end it.
struct Parts {
    language: String,
    script: Option<String>,
    region: Option<String>,
}

fn parts(tag: &str) -> Option<Parts> {
    let mut subtags = tag.split(['-', '_']).filter(|s| !s.is_empty());
    let language = subtags.next()?.to_ascii_lowercase();
    if !(2..=8).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut script = None;
    let mut region = None;
    for s in subtags {
        if s.len() == 1 {
            break;
        }
        let alpha = s.chars().all(|c| c.is_ascii_alphabetic());
        if s.len() == 4 && alpha && script.is_none() && region.is_none() {
            script = Some(s.to_ascii_lowercase());
        } else if region.is_none()
            && ((s.len() == 2 && alpha) || (s.len() == 3 && s.chars().all(|c| c.is_ascii_digit())))
        {
            region = Some(s.to_ascii_lowercase());
        }
    }
    Some(Parts { language, script, region })
}

/// The script a shipped locale is written in, lowercased.
fn script_of(l: &Locale) -> &'static str {
    match l.tag {
        "zh-Hans" => "hans",
        "hi" => "deva",
        "ar" => "arab",
        "ru" => "cyrl",
        "ja" => "jpan",
        "bn" => "beng",
        _ => "latn",
    }
}

/// The script `p` asks for: its own, or the one its language and region imply.
fn implied_script(p: &Parts) -> Option<&str> {
    if let Some(s) = &p.script {
        return Some(s);
    }
    match (p.language.as_str(), p.region.as_deref()) {
        ("zh", Some("tw" | "hk" | "mo")) => Some("hant"),
        ("zh", _) => Some("hans"),
        _ => None,
    }
}

pub(crate) fn best(shipped: &'static [Locale], requested: &str) -> Option<&'static Locale> {
    let p = parts(requested)?;
    // English is the source; the pseudo-locales rewrite it at draw time.
    let pseudo = matches!(p.region.as_deref(), Some("xa" | "xb" | "xc"));
    if p.language == "en" || pseudo {
        return None;
    }
    let wanted = requested.replace('_', "-");
    if let Some(l) = shipped.iter().find(|l| l.tag.eq_ignore_ascii_case(&wanted)) {
        return Some(l);
    }
    let script = implied_script(&p);
    shipped.iter().find(|l| {
        let lp = l.tag.split('-').next().unwrap_or_default();
        lp.eq_ignore_ascii_case(&p.language) && script.is_none_or(|s| s == script_of(l))
    })
}

/// Which form (from 0) a count of `n` takes under the plural rule named `rule` (see
/// [`Locale::plural`]). An unknown rule is read as `"one-other"`.
///
/// ```
/// use remedy_ui_words::plural_form;
/// assert_eq!([1, 2, 5, 21, 22, 25, 111].map(|n| plural_form("slavic", n)), [0, 1, 2, 0, 1, 2, 2]);
/// assert_eq!([0, 1, 2, 3, 11, 100, 102, 103].map(|n| plural_form("arabic", n)), [0, 1, 2, 3, 4, 5, 5, 3]);
/// assert_eq!([0, 1, 2].map(|n| plural_form("zero-one", n)), [0, 0, 1]);
/// ```
pub fn plural_form(rule: &str, n: u64) -> usize {
    let (n10, n100) = (n % 10, n % 100);
    match rule {
        "none" => 0,
        "zero-one" => usize::from(n > 1),
        "slavic" => {
            if n10 == 1 && n100 != 11 {
                0
            } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                1
            } else {
                2
            }
        }
        "arabic" => match n {
            0 => 0,
            1 => 1,
            2 => 2,
            _ if (3..=10).contains(&n100) => 3,
            _ if (11..=99).contains(&n100) => 4,
            _ => 5,
        },
        _ => usize::from(n != 1),
    }
}

/// How many forms a counted word has under the plural rule named `rule`.
pub fn plural_form_count(rule: &str) -> usize {
    match rule {
        "none" => 1,
        "slavic" => 3,
        "arabic" => 6,
        _ => 2,
    }
}
