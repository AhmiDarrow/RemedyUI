//! The shipped files, the locale rules and the inventory.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::*;

/// The `{name}` placeholders in `s`, sorted, with repeats.
fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let after = rest.get(open + 1..).unwrap_or_default();
        let name = after.find('}').and_then(|c| after.get(..c)).filter(|n| {
            let mut ch = n.chars();
            ch.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        if let Some(n) = name {
            out.push(n.to_owned());
        }
        rest = after;
    }
    out.sort();
    out
}

/// `REMEDY_WRITE_WORDS` is set: the inventory test is rewriting `en.json`, and the tests that
/// read it (as built, before the rewrite) stand aside.
fn writing() -> bool {
    std::env::var_os("REMEDY_WRITE_WORDS").is_some_and(|v| v != "0")
}

fn english() -> BTreeMap<String, String> {
    match parse(ENGLISH) {
        Ok(p) => p.into_iter().collect(),
        Err(e) => unreachable!("words/en.json does not parse: {e}"),
    }
}

/// Counted keys: those whose English value holds forms.
fn is_counted(en: &BTreeMap<String, String>, key: &str) -> bool {
    en.get(key).is_some_and(|v| v.contains(SEPARATOR))
}

fn shipped_files() -> impl Iterator<Item = (&'static Locale, Vec<(String, String)>)> {
    SHIPPED.iter().zip(FILES.iter()).map(|(l, text)| match parse(text) {
        Ok(p) => (l, p),
        Err(e) => unreachable!("words/{}.json does not parse: {e}", l.tag),
    })
}

#[test]
fn every_shipped_file_parses_and_keeps_the_placeholders() {
    if writing() {
        return;
    }
    let en = english();
    let mut problems = Vec::new();
    for (l, pairs) in shipped_files() {
        for (key, value) in pairs {
            if !en.contains_key(&key) {
                problems.push(format!("{}: {key:?} is not in en.json", l.tag));
                continue;
            }
            if value.is_empty() {
                continue;
            }
            let want = placeholders(&key);
            let forms: Vec<&str> =
                if is_counted(&en, &key) { value.split(SEPARATOR).collect() } else { vec![value.as_str()] };
            for f in forms {
                if placeholders(f) != want {
                    problems
                        .push(format!("{}: {key:?} → {f:?} should have the placeholders {want:?}", l.tag));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_files_are_the_shipped_list() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("words");
    let mut on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    assert!(on_disk.remove("en"), "words/en.json is the inventory");
    let listed: BTreeSet<String> = SHIPPED.iter().map(|l| l.tag.to_owned()).collect();
    assert_eq!(on_disk, listed, "every words/<tag>.json is in SHIPPED and FILES, and only those");
    for (l, text) in SHIPPED.iter().zip(FILES.iter()) {
        let disk = std::fs::read_to_string(dir.join(format!("{}.json", l.tag))).unwrap_or_default();
        assert_eq!(&disk, text, "FILES lists {}.json in SHIPPED's order", l.tag);
    }
}

/// Words that may read the same as the English in a translation: product and technical
/// names.
const SAME_AS_ENGLISH: &[&str] = &["WCAG", "OK"];

/// Words a language writes exactly as English does, checked one by one: everything else
/// that reads the same as the English is an untranslated word.
const SAME_IN: &[(&str, &[&str])] = &[
    ("es", &["Terminal", "Widget", "normal", "vibrato", "{label} hex"]),
    (
        "fr",
        &[
            "Accent",
            "Code",
            "Code, {language}",
            "Compact",
            "Danger",
            "Ellipse",
            "Image",
            "Images",
            "Information",
            "Mention",
            "Menu",
            "Message",
            "Messages",
            "Note",
            "Page {number}",
            "Pages",
            "Pause",
            "Question",
            "Rectangle",
            "Terminal",
            "Volume",
            "Widget",
            "hammer-on",
            "normal",
            "pull-off",
            "vibrato",
        ],
    ),
    (
        "pt-BR",
        &[
            "Layout {file}",
            "Link",
            "Links",
            "Menu",
            "Terminal",
            "Volume",
            "Widget",
            "layout {file}",
            "normal",
            "vibrato",
        ],
    ),
    (
        "de",
        &[
            "Code",
            "Code, {language}",
            "Computer",
            "Ellipse",
            "Information",
            "Layout {file}",
            "Link",
            "Links",
            "Terminal",
            "Text",
            "Widget",
            "normal",
            "{position} · Live",
        ],
    ),
];

/// Whether a word has any words of its own besides its placeholders.
fn has_words(key: &str) -> bool {
    let mut rest = key;
    let mut text = String::new();
    while let Some(open) = rest.find('{') {
        text.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open..).unwrap_or_default();
        rest = after.find('}').and_then(|c| after.get(c + 1..)).unwrap_or_default();
    }
    text.push_str(rest);
    text.chars().any(char::is_alphabetic)
}

#[test]
fn every_shipped_locale_is_complete() {
    let en = english();
    let mut problems = Vec::new();
    for (l, pairs) in shipped_files() {
        let table: BTreeMap<String, String> = pairs.into_iter().collect();
        for (key, english_value) in &en {
            let Some(value) = table.get(key).filter(|v| !v.trim().is_empty()) else {
                problems.push(format!("{}: {key:?} is missing", l.tag));
                continue;
            };
            if is_counted(&en, key) {
                let n = value.split(SEPARATOR).count();
                if n != l.form_count() {
                    problems.push(format!(
                        "{}: {key:?} has {n} forms; the {} rule needs {}",
                        l.tag,
                        l.plural,
                        l.form_count()
                    ));
                }
            }
            let allowed = SAME_IN.iter().find(|(tag, _)| *tag == l.tag).map_or(&[][..], |(_, words)| *words);
            if value == english_value
                && has_words(key)
                && !SAME_AS_ENGLISH.contains(&key.as_str())
                && !allowed.contains(&key.as_str())
            {
                problems.push(format!("{}: {key:?} is still English", l.tag));
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn the_english_inventory_is_sorted_and_self_keyed() {
    if writing() {
        return;
    }
    let pairs = parse(ENGLISH).unwrap_or_default();
    assert!(!pairs.is_empty());
    let keys: Vec<&String> = pairs.iter().map(|(k, _)| k).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "en.json is sorted");
    for (k, v) in &pairs {
        if v.contains(SEPARATOR) {
            assert_eq!(v.split(SEPARATOR).count(), 2, "{k:?}: English has two forms");
            assert!(v.ends_with(&format!("{SEPARATOR}{k}")), "{k:?}: the English plural is the key");
        } else {
            assert_eq!(k, v, "a word maps to itself");
        }
    }
}

#[test]
fn locales_match_by_tag_then_script_then_language() {
    let tag = |s: &str| best_match(s).map(|l| l.tag);
    // Exact, in any case, with `_`.
    assert_eq!(tag("pt-BR"), Some("pt-BR"));
    assert_eq!(tag("pt_br"), Some("pt-BR"));
    assert_eq!(tag("ZH-HANS"), Some("zh-Hans"));
    assert_eq!(tag("de"), Some("de"));
    // Language and script.
    for s in ["zh", "zh-CN", "zh-SG", "zh_CN", "zh-Hans-CN", "zh-Hans-HK", "zh-Hans-SG"] {
        assert_eq!(tag(s), Some("zh-Hans"), "{s}");
    }
    for s in ["zh-TW", "zh-HK", "zh-MO", "zh-Hant", "zh-Hant-TW", "zh-hant-hk", "zh_TW"] {
        assert_eq!(tag(s), None, "{s}: Traditional readers get English, not the wrong script");
    }
    assert_eq!(tag("ru-Latn"), None, "another script is not a match");
    // Language.
    assert_eq!(tag("pt-PT"), Some("pt-BR"));
    assert_eq!(tag("pt"), Some("pt-BR"));
    assert_eq!(tag("es-MX"), Some("es"));
    assert_eq!(tag("es-419"), Some("es"));
    assert_eq!(tag("fr-CA"), Some("fr"));
    assert_eq!(tag("de-AT"), Some("de"));
    assert_eq!(tag("ar-EG"), Some("ar"));
    assert_eq!(tag("ja-JP"), Some("ja"));
    assert_eq!(tag("hi-IN"), Some("hi"));
    assert_eq!(tag("bn-BD"), Some("bn"));
    assert_eq!(tag("ru-RU-u-ca-gregory"), Some("ru"));
    // English, the pseudo-locales and the rest.
    for s in ["en", "en-US", "en-GB", "en-XA", "ar-XB", "ar_XB", "", "-", "nl-NL", "x"] {
        assert_eq!(tag(s), None, "{s}");
    }
    assert!(best_match("ar").is_some_and(|l| l.rtl));
    assert!(SHIPPED.iter().filter(|l| l.rtl).all(|l| l.tag == "ar"));
}

#[test]
fn plural_rules_pick_the_forms() {
    let forms = |rule: &str, ns: &[u64]| ns.iter().map(|&n| plural_form(rule, n)).collect::<Vec<_>>();
    assert_eq!(forms("one-other", &[0, 1, 2, 11, 21]), [1, 0, 1, 1, 1]);
    assert_eq!(forms("zero-one", &[0, 1, 2, 11, 21]), [0, 0, 1, 1, 1]);
    assert_eq!(
        forms("slavic", &[0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 24, 25, 101, 111, 112]),
        [2, 0, 1, 1, 2, 2, 2, 2, 0, 1, 1, 2, 0, 2, 2]
    );
    assert_eq!(
        forms("arabic", &[0, 1, 2, 3, 10, 11, 99, 100, 101, 102, 103, 111, 1000]),
        [0, 1, 2, 3, 3, 4, 4, 5, 5, 5, 3, 4, 5]
    );
    assert_eq!(forms("none", &[0, 1, 2, 100]), [0, 0, 0, 0]);
    let counts: Vec<usize> =
        ["one-other", "zero-one", "slavic", "arabic", "none"].map(plural_form_count).into();
    assert_eq!(counts, [2, 2, 3, 6, 1]);
    for l in &SHIPPED {
        let max = (0..1000).map(|n| l.form(n)).max().unwrap_or_default();
        assert_eq!(max + 1, l.form_count(), "{}: every form is used", l.tag);
    }
}

#[test]
fn pick_and_fill() {
    assert_eq!(pick("ru", "a || b || c", 22), "b");
    assert_eq!(pick("ru-RU", "a || b || c", 11), "c");
    assert_eq!(pick("en", "one || other", 1), "one");
    assert_eq!(pick("nl", "one || other", 0), "other");
    assert_eq!(pick("ar", "a || b", 50), "b", "a missing form falls back to the last");
    assert_eq!(pick("fr", "plain", 9), "plain");
    assert_eq!(fill("{a} and {b}, {c}", &[("a", "1"), ("b", "{a}")]), "1 and {a}, {c}");
    assert_eq!(fill("{ unbalanced", &[]), "{ unbalanced");
    assert_eq!(nth_form("x || y", 5), "y");
}

/// A fixture standing in for translations: the machinery from a locale tag to a form.
#[test]
fn translations_are_found_by_locale_and_count() {
    let mut tables: Vec<HashMap<String, String>> = vec![HashMap::new(); SHIPPED.len()];
    let fixture = [
        (
            "ru",
            r#"{"Close": "Закрыть", "{count} reactions": "{count} реакция || {count} реакции || {count} реакций", "Empty": " "}"#,
        ),
        ("zh-Hans", r#"{"Close": "关闭"}"#),
    ];
    for (tag, json) in fixture {
        let i = SHIPPED.iter().position(|l| l.tag == tag).unwrap_or_default();
        if let (Some(t), Ok(pairs)) = (tables.get_mut(i), parse(json)) {
            *t = pairs.into_iter().collect();
        }
    }
    assert_eq!(word_in(&tables, "ru-RU", "Close"), Some("Закрыть"));
    assert_eq!(word_in(&tables, "zh-CN", "Close"), Some("关闭"));
    assert_eq!(word_in(&tables, "zh-TW", "Close"), None, "no Simplified for Traditional readers");
    assert_eq!(word_in(&tables, "ru", "Empty"), None, "a blank translation is no translation");
    assert_eq!(word_in(&tables, "de", "Close"), None, "not translated yet");
    let say =
        |n| counted_in(&tables, "ru", "{count} reactions", n).map(|f| fill(f, &[("count", &n.to_string())]));
    assert_eq!(say(1).as_deref(), Some("1 реакция"));
    assert_eq!(say(3).as_deref(), Some("3 реакции"));
    assert_eq!(say(11).as_deref(), Some("11 реакций"));
    assert_eq!(say(21).as_deref(), Some("21 реакция"));
}

#[test]
fn lookups_fall_back_to_none() {
    // Whatever the files hold, English and unknown locales never translate.
    assert_eq!(word("en-US", "Close"), None);
    assert_eq!(word("en-XA", "Close"), None);
    assert_eq!(word("tlh", "Close"), None);
    assert_eq!(word("fr", "A word the engine never says"), None);
    assert_eq!(counted("en", "{count} reaction", "{count} reactions", 3), None);
}

#[test]
fn the_parser_is_strict() {
    let ok = |s: &str| parse(s).map(|p| p.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>());
    assert_eq!(ok("{}"), Ok(vec![]));
    assert!(ok(" \u{feff}{}").is_err(), "a byte-order mark only at the start");
    assert_eq!(ok("\u{feff}{ }\n"), Ok(vec![]));
    assert_eq!(ok(r#"{"a":"b" , "c" : "d"}"#), Ok(vec!["a=b".into(), "c=d".into()]));
    assert_eq!(ok(r#"{"q\"\\\/":"\n\té😀"}"#), Ok(vec!["q\"\\/=\n\té\u{1f600}".into()]));
    for bad in [
        "",
        "[]",
        r#"{"a": 1}"#,
        r#"{"a": "b",}"#,
        r#"{"a": "b"} x"#,
        r#"{"a": "b", "a": "c"}"#,
        r#"{"a": "\x"}"#,
        r#"{"a": "\ud83d"}"#,
        r#"{"a": "\u12"}"#,
        "{\"a\": \"line\nbreak\"}",
        r#"{"a" "b"}"#,
        r#"{"a": "b""#,
        r#"{"a": {"b": "c"}}"#,
    ] {
        assert!(parse(bad).is_err(), "{bad:?} must not parse");
    }
    let e = parse(r#"{"a": 1}"#).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("at byte 6"), "{e}");
    let pairs = vec![("q\"\n".to_owned(), "é\\".to_owned()), ("b".to_owned(), String::new())];
    assert_eq!(parse(&json::write(&pairs)), Ok(pairs));
    assert_eq!(json::write(&[]), "{}\n");
}

// ---- The inventory: the engine's words as its sources say them. ----

/// Crates that are not the engine's shipping code, or not code that shows words.
const NOT_SCANNED: &[&str] = &[
    "remedy-ui-words",
    "remedy-ui-devtools",
    "remedy-tools",
    "remedy-mcp",
    "cargo-remedy",
    "remedy-templates",
    "remedy-ui-test",
    "remedy-ui-build",
];

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
}

/// Rust source as identifiers, string literals and punctuation, without comments, numbers
/// or character literals. Good enough to find calls; not a full lexer.
fn tokens(src: &str) -> Vec<Tok> {
    let chars: Vec<char> = src.chars().collect();
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = at(i);
        if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && at(i) != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            while i < chars.len() {
                if at(i) == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if at(i) == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == '"'
            || ((c == 'r' || c == 'b') && (at(i + 1) == '"' || at(i + 1) == '#' || at(i + 1) == 'r'))
        {
            // Byte strings and raw strings: r"…", r#"…"#, b"…", br"…".
            let start = i;
            if c == 'b' {
                i += 1;
            }
            let raw = at(i) == 'r';
            if raw {
                i += 1;
            }
            let mut hashes = 0;
            while at(i) == '#' {
                hashes += 1;
                i += 1;
            }
            if at(i) != '"' {
                // An identifier starting with r or b after all.
                i = start;
                let mut s = String::new();
                while at(i).is_alphanumeric() || at(i) == '_' {
                    s.push(at(i));
                    i += 1;
                }
                out.push(Tok::Ident(s));
                continue;
            }
            i += 1;
            let mut s = String::new();
            loop {
                if i >= chars.len() {
                    break;
                }
                let d = at(i);
                if raw {
                    if d == '"' && (0..hashes).all(|k| at(i + 1 + k) == '#') {
                        i += 1 + hashes;
                        break;
                    }
                    s.push(d);
                    i += 1;
                } else if d == '\\' {
                    let e = at(i + 1);
                    i += 2;
                    match e {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '0' => s.push('\0'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        '\'' => s.push('\''),
                        'u' => {
                            // \u{…}
                            let mut hex = String::new();
                            i += 1;
                            while i < chars.len() && at(i) != '}' {
                                hex.push(at(i));
                                i += 1;
                            }
                            i += 1;
                            if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                s.push(ch);
                            }
                        }
                        '\n' => {
                            // A line continuation skips the next line's leading space.
                            while at(i).is_whitespace() {
                                i += 1;
                            }
                        }
                        other => s.push(other),
                    }
                } else if d == '"' {
                    i += 1;
                    break;
                } else {
                    s.push(d);
                    i += 1;
                }
            }
            out.push(Tok::Str(s));
        } else if c == '\'' {
            // A character literal ('x', '\n', '\u{..}') or a lifetime ('a).
            if at(i + 1) == '\\' {
                i += 2;
                while i < chars.len() && at(i) != '\'' {
                    i += 1;
                }
                i += 1;
            } else if at(i + 2) == '\'' {
                i += 3;
            } else {
                i += 1;
                while at(i).is_alphanumeric() || at(i) == '_' {
                    i += 1;
                }
            }
        } else if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while at(i).is_alphanumeric() || at(i) == '_' {
                s.push(at(i));
                i += 1;
            }
            out.push(Tok::Ident(s));
        } else if c.is_ascii_digit() {
            while at(i).is_alphanumeric() || at(i) == '_' || (at(i) == '.' && at(i + 1).is_ascii_digit()) {
                i += 1;
            }
        } else if c.is_whitespace() {
            i += 1;
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    out
}

/// `toks` without items marked `#[cfg(test)]`.
fn without_tests(toks: Vec<Tok>) -> Vec<Tok> {
    let ident = |t: Option<&Tok>, s: &str| matches!(t, Some(Tok::Ident(x)) if x == s);
    let punct = |t: Option<&Tok>, c: char| matches!(t, Some(Tok::Punct(x)) if *x == c);
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let is_cfg_test = punct(toks.get(i), '#')
            && punct(toks.get(i + 1), '[')
            && ident(toks.get(i + 2), "cfg")
            && punct(toks.get(i + 3), '(')
            && ident(toks.get(i + 4), "test")
            && punct(toks.get(i + 5), ')')
            && punct(toks.get(i + 6), ']');
        if !is_cfg_test {
            if let Some(t) = toks.get(i) {
                out.push(t.clone());
            }
            i += 1;
            continue;
        }
        // Skip the item: to its `;`, or through its braces.
        i += 7;
        let mut depth = 0usize;
        while let Some(t) = toks.get(i) {
            i += 1;
            match t {
                Tok::Punct('{') => depth += 1,
                Tok::Punct('}') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Punct(';') if depth == 0 => break,
                _ => {}
            }
        }
    }
    out
}

/// The string literals among a call's arguments at its own depth, given the index of its
/// opening parenthesis.
fn call_strings(toks: &[Tok], open: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for t in toks.get(open..).unwrap_or_default() {
        match t {
            Tok::Punct('(' | '[' | '{') => depth += 1,
            Tok::Punct(')' | ']' | '}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            Tok::Str(s) if depth == 1 => out.push(s.clone()),
            _ => {}
        }
    }
    out
}

/// The string literals inside the braces that open at `open`.
fn block_strings(toks: &[Tok], open: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for t in toks.get(open..).unwrap_or_default() {
        match t {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            Tok::Str(s) => out.push(s.clone()),
            _ => {}
        }
    }
    out
}

#[derive(Default)]
struct Found {
    words: BTreeSet<String>,
    /// Counted words: the English plural and its singular.
    counted: BTreeMap<String, String>,
    /// `*Labels` defaults.
    labels: BTreeSet<String>,
}

fn extract(src: &str, found: &mut Found, file: &str, problems: &mut Vec<String>) {
    let toks = without_tests(tokens(src));
    let ident = |i: usize| match toks.get(i) {
        Some(Tok::Ident(s)) => Some(s.as_str()),
        _ => None,
    };
    let punct = |i: usize, c: char| matches!(toks.get(i), Some(Tok::Punct(x)) if *x == c);
    // `words::name` or `remedy_ui_words::name` before index `i` (the name).
    let words_path = |i: usize| {
        i >= 3
            && punct(i - 1, ':')
            && punct(i - 2, ':')
            && matches!(ident(i - 3), Some("words" | "remedy_ui_words"))
    };
    // The bodies of `words! { … }` invocations, by token index.
    let mut macro_bodies = Vec::new();
    for i in 0..toks.len() {
        if ident(i) == Some("words") && punct(i + 1, '!') && punct(i + 2, '{') {
            let mut depth = 0usize;
            for (k, t) in toks.iter().enumerate().skip(i + 2) {
                match t {
                    Tok::Punct('{') => depth += 1,
                    Tok::Punct('}') => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            macro_bodies.push(i + 2..k);
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Names a `use … words::{…}` (or `use … words::name;`) brings in: their bare calls count.
    let mut imported = BTreeSet::new();
    for i in 0..toks.len() {
        if !(matches!(ident(i), Some("words" | "remedy_ui_words")) && punct(i + 1, ':') && punct(i + 2, ':'))
        {
            continue;
        }
        let in_use = (0..i)
            .rev()
            .take(16)
            .map_while(|k| (!punct(k, ';')).then_some(k))
            .any(|k| ident(k) == Some("use"));
        if !in_use {
            continue;
        }
        if punct(i + 3, '{') {
            let mut k = i + 4;
            while let Some(t) = toks.get(k) {
                match t {
                    Tok::Ident(n) => {
                        imported.insert(n.clone());
                    }
                    Tok::Punct('}') => break,
                    _ => {}
                }
                k += 1;
            }
        } else if let Some(n) = ident(i + 3) {
            imported.insert(n.to_owned());
        }
    }
    for i in 0..toks.len() {
        let Some(name) = ident(i) else { continue };
        let method = i >= 1 && punct(i - 1, '.') && punct(i + 1, '(');
        let bare = imported.contains(name)
            && punct(i + 1, '(')
            && !(i >= 1 && (punct(i - 1, '.') || punct(i - 1, ':') || ident(i - 1) == Some("fn")));
        let path_call = (words_path(i) || bare) && punct(i + 1, '(');
        let strings = || call_strings(&toks, i + 1);
        match name {
            // A word passed by value was marked where it was written, with `words::en`.
            "word" if method => {
                if let Some(Tok::Str(s)) = toks.get(i + 2) {
                    found.words.insert(s.clone());
                }
            }
            "word" | "word_or" | "en" if path_call => {
                if let Some(s) = strings().first() {
                    found.words.insert(s.clone());
                }
            }
            // Forms passed by value (as `Ui::counted` passes them on) were named where written.
            "counted" | "forms" if method || path_call => match strings().as_slice() {
                [one, other, ..] => {
                    found.counted.insert(other.clone(), one.clone());
                }
                [_] => problems
                    .push(format!("{file}: `{name}(` takes both forms as string literals, or neither")),
                _ => {}
            },
            // `impl Default for SomethingLabels { … }`
            "Default"
                if i >= 1
                    && ident(i - 1) == Some("impl")
                    && ident(i + 1) == Some("for")
                    && ident(i + 2).is_some_and(|n| n.ends_with("Labels")) =>
            {
                if let Some(open) = (i + 3..toks.len()).find(|&k| punct(k, '{')) {
                    found.labels.extend(block_strings(&toks, open));
                }
            }
            // The board's `words! { SomethingLabels { field: "default", … } }`.
            n if n.ends_with("Labels")
                && punct(i + 1, '{')
                && macro_bodies.iter().any(|r| r.contains(&i)) =>
            {
                found.labels.extend(block_strings(&toks, i + 1));
            }
            _ => {}
        }
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests" || n == "benches" || n == "examples") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs")
            && !path
                .file_name()
                .is_some_and(|n| n == "tests.rs" || n.to_string_lossy().ends_with("_tests.rs"))
        {
            out.push(path);
        }
    }
}

/// The engine's words as the workspace's sources say them: `en.json`'s contents.
fn inventory() -> (BTreeMap<String, String>, Vec<String>) {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(&crates).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    dirs.sort();
    let mut found = Found::default();
    let mut problems = Vec::new();
    for dir in dirs {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if NOT_SCANNED.contains(&name.as_str()) {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&dir.join("src"), &mut files);
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            let rel = f.strip_prefix(&crates).unwrap_or(&f).display().to_string();
            extract(&text, &mut found, &rel, &mut problems);
        }
    }
    let mut out = BTreeMap::new();
    for w in found.words.iter().chain(&found.labels) {
        // A `*Labels` field that is one form of a counted word is that counted word.
        let is_singular = found.counted.values().any(|one| one == w);
        if !is_singular && !found.counted.contains_key(w) {
            out.insert(w.clone(), w.clone());
        }
    }
    for (other, one) in &found.counted {
        out.insert(other.clone(), format!("{one}{SEPARATOR}{other}"));
    }
    (out, problems)
}

#[test]
fn the_inventory_is_every_word_the_sources_say() {
    let (found, problems) = inventory();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(found.len() > 100, "the scan found the engine's words ({} of them)", found.len());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("words/en.json");
    if writing() {
        let pairs: Vec<(String, String)> = found.into_iter().collect();
        if let Err(e) = std::fs::write(&path, json::write(&pairs)) {
            unreachable!("could not write {}: {e}", path.display());
        }
        return;
    }
    let en = english();
    let missing: Vec<&String> = found.keys().filter(|k| !en.contains_key(*k)).collect();
    let stale: Vec<&String> = en.keys().filter(|k| !found.contains_key(*k)).collect();
    let changed: Vec<&String> =
        found.iter().filter(|(k, v)| en.get(*k).is_some_and(|e| e != *v)).map(|(k, _)| k).collect();
    assert!(
        missing.is_empty() && stale.is_empty() && changed.is_empty(),
        "words/en.json is out of date with the sources; run `REMEDY_WRITE_WORDS=1 cargo test -p remedy-ui-words` \
         and commit it.\nmissing: {missing:?}\nno longer said: {stale:?}\nforms changed: {changed:?}"
    );
}

#[test]
fn the_extractor_reads_calls_and_labels() {
    let src = r##"
        fn f(ui: &Ui) {
            ui.word("Close");
            let _ = ui.word ( "Show \"password\"" );
            remedy_ui_words::word(&ui.locale(), "Open");
            words::word_or(&locale, "Quit");
            words::word(locale.as_str(), r#"Raw "one""#);
            ui.counted("{count} file", "{count} files", 3);
            words::forms(l, "1 reaction", "{count} reactions");
            const NAMES: &[&str] = &[words::en("Highlight")];
            // ui.word("In a comment");
            let c = '"'; let l: &'static str = "plain";
            other.word_count("not a word");
        }
        fn h(locale: &str) {
            use remedy_ui_view::words::{counted, word_or};
            word_or(locale, "Bare");
            counted(locale, "{count} sharp", "{count} sharps", 2);
            en("not imported here");
        }
        impl Default for MenuLabels {
            fn default() -> Self { MenuLabels { ignore: "Ignore".into() } }
        }
        words! {
            /// Docs "not a word".
            BoardLabels { undo: "Undo", many: "{count} reactions", one: "1 reaction", }
        }
        impl BoardLabels { fn f() { fill("x", &[("count", n)]); } }
        #[cfg(test)]
        mod tests { fn t(ui: &Ui) { ui.word("Test only"); } }
        fn g(ui: &Ui) { ui.word("After tests"); }
    "##;
    let mut found = Found::default();
    let mut problems = Vec::new();
    extract(src, &mut found, "x.rs", &mut problems);
    assert!(problems.is_empty(), "{problems:?}");
    let words: Vec<&str> = found.words.iter().map(String::as_str).collect();
    assert_eq!(
        words,
        ["After tests", "Bare", "Close", "Highlight", "Open", "Quit", "Raw \"one\"", "Show \"password\""]
    );
    assert_eq!(found.counted.get("{count} sharps").map(String::as_str), Some("{count} sharp"));
    let labels: Vec<&str> = found.labels.iter().map(String::as_str).collect();
    assert_eq!(labels, ["1 reaction", "Ignore", "Undo", "{count} reactions"]);
    assert_eq!(found.counted.get("{count} files").map(String::as_str), Some("{count} file"));
    assert_eq!(found.counted.get("{count} reactions").map(String::as_str), Some("1 reaction"));
    let mut bad = Vec::new();
    extract("fn f(ui: &Ui) { ui.counted(\"{count} file\", n, 3); }", &mut Found::default(), "y.rs", &mut bad);
    assert_eq!(bad.len(), 1, "a counted word's forms are both literals");
    let mut fine = Vec::new();
    extract("fn f(ui: &Ui) { words::counted(l, one, other, 3); }", &mut Found::default(), "z.rs", &mut fine);
    assert!(fine.is_empty(), "or both passed on");
}
