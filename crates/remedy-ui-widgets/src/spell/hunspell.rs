//! Hunspell-format dictionaries (`.aff` and `.dic` files), read by the pure-Rust `spellbook`
//! engine. This is the back end on Linux and the fallback everywhere.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use spellbook::Dictionary;

use crate::spell::backend::{BackendError, Prepared, SpellBackend};

/// The folders Hunspell dictionaries are usually in, most specific first: the `DICPATH`
/// variable Hunspell itself honors, the user's and the system's XDG data folders, the classic
/// `/usr/share/hunspell` and `/usr/share/myspell` places, and the macOS and Homebrew spelling
/// folders. Folders that do not exist are listed too; they just hold nothing.
pub fn system_directories() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(v) = std::env::var_os("DICPATH") {
        for p in std::env::split_paths(&v) {
            push(p);
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let mut data: Vec<PathBuf> = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(v) => data.push(PathBuf::from(v)),
        None => data.extend(home.iter().map(|h| h.join(".local").join("share"))),
    }
    match std::env::var_os("XDG_DATA_DIRS").filter(|v| !v.is_empty()) {
        Some(v) => data.extend(std::env::split_paths(&v)),
        None => data.extend([PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]),
    }
    for d in data {
        push(d.join("hunspell"));
        push(d.join("myspell"));
        push(d.join("myspell").join("dicts"));
    }
    push(PathBuf::from("/usr/share/hunspell"));
    push(PathBuf::from("/usr/share/myspell"));
    push(PathBuf::from("/usr/share/myspell/dicts"));
    if let Some(h) = &home {
        push(h.join("Library").join("Spelling"));
    }
    push(PathBuf::from("/Library/Spelling"));
    push(PathBuf::from("/opt/homebrew/share/hunspell"));
    push(PathBuf::from("/usr/local/share/hunspell"));
    out
}

/// How close a dictionary named `stem` is to `language`: 0 is the same language and region, 1
/// a variant of it, 2 the language without a region, 3 another region of the language. `None`
/// when it is another language.
fn rank(stem: &str, language: &str) -> Option<u8> {
    let flat = |s: &str| s.to_lowercase().replace('-', "_");
    let (stem, want) = (flat(stem), flat(language));
    let primary = want.split('_').next().unwrap_or(&want).to_owned();
    if stem == want {
        Some(0)
    } else if stem.starts_with(&format!("{want}_")) {
        Some(1)
    } else if stem == primary {
        Some(2)
    } else if stem.starts_with(&format!("{primary}_")) {
        Some(3)
    } else {
        None
    }
}

/// A dictionary file pair found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Found {
    stem: String,
    aff: PathBuf,
    dic: PathBuf,
}

/// The `.aff`/`.dic` pairs in `dir`.
fn pairs_in(dir: &Path) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| !x.eq_ignore_ascii_case("dic")) {
            continue;
        }
        let aff = path.with_extension("aff");
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if aff.is_file() {
            out.push(Found { stem: stem.to_owned(), aff, dic: path });
        }
    }
    out.sort_by(|a, b| a.stem.cmp(&b.stem));
    out
}

/// The charset an `.aff` file declares with `SET`, if it declares one.
fn declared_charset(aff: &[u8]) -> Option<String> {
    aff.split(|b| *b == b'\n').find_map(|line| {
        let line = line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(line);
        let rest = line.strip_prefix(b"SET")?;
        if !rest.first().is_some_and(u8::is_ascii_whitespace) {
            return None;
        }
        let label = String::from_utf8_lossy(rest).trim().to_owned();
        (!label.is_empty()).then_some(label)
    })
}

/// Reads a dictionary's text in the charset the `.aff` declares.
fn decode(bytes: &[u8], charset: Option<&str>) -> Result<String, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let label = charset.unwrap_or("UTF-8");
    if label.eq_ignore_ascii_case("UTF-8") || label.eq_ignore_ascii_case("UTF8") {
        return std::str::from_utf8(bytes).map(str::to_owned).map_err(|e| format!("not valid UTF-8: {e}"));
    }
    let lower = label.to_ascii_lowercase();
    let label = match lower.as_str() {
        "tis620-2533" | "tis620" | "tis-620" => "windows-874".to_owned(),
        other => other.trim_start_matches("microsoft-").to_owned(),
    };
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
        .ok_or_else(|| format!("the charset {label} is not supported"))?;
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|c| c.into_owned())
        .ok_or_else(|| format!("not valid {label}"))
}

fn parse(aff: &str, dic: &str) -> Result<Dictionary, String> {
    Dictionary::new(aff, dic).map_err(|e| e.to_string())
}

fn load_files(found: &Found) -> Result<Dictionary, String> {
    let aff = std::fs::read(&found.aff).map_err(|e| format!("{}: {e}", found.aff.display()))?;
    let dic = std::fs::read(&found.dic).map_err(|e| format!("{}: {e}", found.dic.display()))?;
    let charset = declared_charset(&aff);
    let aff = decode(&aff, charset.as_deref()).map_err(|e| format!("{}: {e}", found.aff.display()))?;
    let dic = decode(&dic, charset.as_deref()).map_err(|e| format!("{}: {e}", found.dic.display()))?;
    parse(&aff, &dic).map_err(|e| format!("{}: {e}", found.stem))
}

enum Loaded {
    Ready { name: String, dictionary: Box<Dictionary> },
    Missing,
    Broken(String),
}

/// The Hunspell back end: dictionaries found in folders, or given as text by the app.
///
/// For a language such as `en-AU` it uses the closest dictionary it has: `en_AU` if there is
/// one, else `en`, else another region of English. Which one it chose is reported with the
/// language's availability. Files in other charsets than UTF-8 (`ISO8859-1`, `KOI8-R`, …) are
/// read as declared.
#[derive(Default)]
pub struct HunspellBackend {
    dirs: Vec<PathBuf>,
    memory: Vec<(String, String, String)>,
    loaded: HashMap<String, Loaded>,
    label: Option<String>,
}

impl HunspellBackend {
    /// A back end that searches the usual system folders ([`system_directories`]).
    pub fn new() -> Self {
        HunspellBackend { dirs: system_directories(), ..Self::default() }
    }

    /// A back end that searches only `dirs`, in order.
    pub fn with_directories(dirs: Vec<PathBuf>) -> Self {
        HunspellBackend { dirs, ..Self::default() }
    }

    /// Names the back end in reports (the default is `hunspell`).
    pub fn labeled(mut self, label: &str) -> Self {
        self.label = Some(label.to_owned());
        self
    }

    /// Searches `dir` before the folders it already searches.
    pub fn prepend_directory(&mut self, dir: PathBuf) {
        self.dirs.retain(|d| *d != dir);
        self.dirs.insert(0, dir);
        self.loaded.clear();
    }

    /// Adds a dictionary the app holds in memory, for `language` (a BCP 47 tag): the text of
    /// its `.aff` and `.dic` files, both UTF-8. It is preferred to files for the same language.
    pub fn add_dictionary(&mut self, language: &str, aff: String, dic: String) {
        self.memory.push((language.to_owned(), aff, dic));
        self.loaded.clear();
    }

    fn load(&self, language: &str) -> Loaded {
        // The best in-memory dictionary, then the best on disk; closer beats farther, and
        // memory beats disk and earlier folders beat later ones when equally close.
        let mut candidates: Vec<(u8, Source<'_>)> = Vec::new();
        for (lang, aff, dic) in &self.memory {
            if let Some(r) = rank(lang, language) {
                candidates.push((r, Source::Memory(lang, aff, dic)));
            }
        }
        for dir in &self.dirs {
            let mut here: Vec<(u8, Found)> =
                pairs_in(dir).into_iter().filter_map(|f| rank(&f.stem, language).map(|r| (r, f))).collect();
            here.sort_by(|a, b| (a.0, a.1.stem.len(), &a.1.stem).cmp(&(b.0, b.1.stem.len(), &b.1.stem)));
            if let Some((r, f)) = here.into_iter().next() {
                candidates.push((r, Source::Files(f)));
            }
        }
        // The closest wins; on a tie the earlier one does.
        let best = candidates.into_iter().reduce(|a, b| if b.0 < a.0 { b } else { a });
        match best {
            None => Loaded::Missing,
            Some((_, Source::Memory(name, aff, dic))) => match parse(aff, dic) {
                Ok(d) => Loaded::Ready { name: name.to_owned(), dictionary: Box::new(d) },
                Err(e) => Loaded::Broken(format!("{name}: {e}")),
            },
            Some((_, Source::Files(f))) => match load_files(&f) {
                Ok(d) => Loaded::Ready { name: f.stem, dictionary: Box::new(d) },
                Err(e) => Loaded::Broken(e),
            },
        }
    }

    fn dictionary(&mut self, language: &str) -> Option<&Dictionary> {
        if !self.loaded.contains_key(language) {
            let l = self.load(language);
            self.loaded.insert(language.to_owned(), l);
        }
        match self.loaded.get(language) {
            Some(Loaded::Ready { dictionary, .. }) => Some(dictionary),
            _ => None,
        }
    }
}

enum Source<'a> {
    Memory(&'a str, &'a str, &'a str),
    Files(Found),
}

/// A typographic apostrophe as the plain one dictionaries are written with.
fn plain_apostrophes(word: &str) -> String {
    word.replace(['\u{2019}', '\u{02BC}'], "'")
}

impl SpellBackend for HunspellBackend {
    fn name(&self) -> &str {
        self.label.as_deref().unwrap_or("hunspell")
    }

    fn prepare(&mut self, language: &str) -> Prepared {
        let _ = self.dictionary(language);
        match self.loaded.get(language) {
            Some(Loaded::Ready { name, .. }) => Prepared::Ready { dictionary: name.clone() },
            Some(Loaded::Broken(why)) => Prepared::Broken(why.clone()),
            _ => Prepared::Missing,
        }
    }

    fn languages(&mut self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (lang, _, _) in &self.memory {
            out.push(lang.clone());
        }
        for dir in &self.dirs {
            for f in pairs_in(dir) {
                if let Ok(tag) = crate::spell::languages::normalize(&f.stem)
                    && !out.contains(&tag)
                {
                    out.push(tag);
                }
            }
        }
        out
    }

    fn check_word(&mut self, language: &str, word: &str) -> Result<bool, BackendError> {
        let dict = self.dictionary(language).ok_or_else(|| format!("no dictionary for {language}"))?;
        if dict.check(word) {
            return Ok(true);
        }
        let plain = plain_apostrophes(word);
        Ok(plain != word && dict.check(&plain))
    }

    fn suggest(&mut self, language: &str, word: &str) -> Result<Vec<String>, BackendError> {
        let dict = self.dictionary(language).ok_or_else(|| format!("no dictionary for {language}"))?;
        let curly = word.contains(['\u{2019}', '\u{02BC}']);
        let mut out = Vec::new();
        dict.suggest(&plain_apostrophes(word), &mut out);
        if curly {
            for s in &mut out {
                *s = s.replace('\'', "\u{2019}");
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny English dictionary: plurals and past tense by affix, and a compound-free word list.
    const AFF: &str = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwzESIANRTOLCDUGMPHBYFVKWZ'\n\nPFX U Y 1\nPFX U 0 un .\n\nSFX S Y 1\nSFX S 0 s [^s]\n\nSFX D Y 1\nSFX D 0 ed .\n";
    const DIC: &str = "9\nhello/S\nworld/S\nfriend/SU\nquick\nbrown\nfox/S\njump/SD\nover\ndon't\n";

    fn backend() -> HunspellBackend {
        let mut b = HunspellBackend::with_directories(Vec::new());
        b.add_dictionary("en-US", AFF.into(), DIC.into());
        b
    }

    #[test]
    fn words_and_their_affixed_forms_are_accepted() {
        let mut b = backend();
        assert_eq!(b.prepare("en-US"), Prepared::Ready { dictionary: "en-US".into() });
        for ok in ["hello", "Hello", "worlds", "unfriend", "jumped", "foxs", "don't", "over"] {
            assert_eq!(b.check_word("en-US", ok), Ok(true), "{ok}");
        }
        for bad in ["helo", "wrold", "jumpd", "teh", "unquick"] {
            assert_eq!(b.check_word("en-US", bad), Ok(false), "{bad}");
        }
    }

    #[test]
    fn a_typographic_apostrophe_is_the_plain_one() {
        let mut b = backend();
        assert_eq!(b.check_word("en-US", "don\u{2019}t"), Ok(true));
    }

    #[test]
    fn suggestions_come_from_the_dictionary() {
        let mut b = backend();
        let s = b.suggest("en-US", "helo").unwrap_or_else(|e| unreachable!("{e}"));
        assert!(s.iter().any(|w| w == "hello"), "{s:?}");
        let s = b.suggest("en-US", "wrold").unwrap_or_else(|e| unreachable!("{e}"));
        assert!(s.iter().any(|w| w == "world"), "{s:?}");
        assert!(b.suggest("en-US", "zzzzqx").unwrap_or_default().iter().all(|w| w != "zzzzqx"));
    }

    #[test]
    fn the_closest_dictionary_is_used_for_a_language() {
        let mut b = HunspellBackend::with_directories(Vec::new());
        b.add_dictionary("en", AFF.into(), DIC.into());
        assert_eq!(b.prepare("en-AU"), Prepared::Ready { dictionary: "en".into() });
        assert_eq!(b.prepare("de"), Prepared::Missing);
        b.add_dictionary("en-GB", AFF.into(), DIC.into());
        assert_eq!(b.prepare("en-GB"), Prepared::Ready { dictionary: "en-GB".into() });
    }

    #[test]
    fn a_dictionary_that_does_not_parse_is_reported_broken() {
        let mut b = HunspellBackend::with_directories(Vec::new());
        b.add_dictionary("fr", "SET UTF-8\nPFX\n".into(), "not a count\nfoo\n".into());
        assert!(matches!(b.prepare("fr"), Prepared::Broken(_)));
    }

    #[test]
    fn dictionaries_are_found_in_folders_and_read_in_their_charset() {
        let dir = std::env::temp_dir().join(format!("remedy-spell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("{e}"));
        // A Latin-1 dictionary with an accented word, in the old Hunspell layout.
        std::fs::write(dir.join("fr_FR.aff"), b"SET ISO8859-1\n").unwrap_or_else(|e| unreachable!("{e}"));
        std::fs::write(dir.join("fr_FR.dic"), b"2\ncaf\xE9\nvoil\xE0\n")
            .unwrap_or_else(|e| unreachable!("{e}"));
        // And one that is not a dictionary pair, which is ignored.
        std::fs::write(dir.join("hyph_fr_FR.dic"), b"ISO8859-1\n").unwrap_or_else(|e| unreachable!("{e}"));
        let mut b = HunspellBackend::with_directories(vec![dir.clone()]);
        assert_eq!(b.languages(), ["fr-FR"]);
        assert_eq!(b.prepare("fr-CA"), Prepared::Ready { dictionary: "fr_FR".into() });
        assert_eq!(b.check_word("fr-CA", "cafe"), Ok(false), "without the accent it is another word");
        assert_eq!(b.check_word("fr-CA", "caf\u{e9}"), Ok(true));
        assert_eq!(b.check_word("fr-CA", "voil\u{e0}"), Ok(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_earlier_folder_wins_when_dictionaries_are_equally_close() {
        let base = std::env::temp_dir().join(format!("remedy-spell-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, c) = (base.join("a"), base.join("c"));
        for (d, word) in [(&a, "alpha"), (&c, "gamma")] {
            std::fs::create_dir_all(d).unwrap_or_else(|e| unreachable!("{e}"));
            std::fs::write(d.join("xx.aff"), "SET UTF-8\n").unwrap_or_else(|e| unreachable!("{e}"));
            std::fs::write(d.join("xx.dic"), format!("1\n{word}\n")).unwrap_or_else(|e| unreachable!("{e}"));
        }
        let mut b = HunspellBackend::with_directories(vec![a, c]);
        assert!(matches!(b.prepare("xx"), Prepared::Ready { .. }));
        assert_eq!(b.check_word("xx", "alpha"), Ok(true));
        assert_eq!(b.check_word("xx", "gamma"), Ok(false));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ranking_prefers_the_same_region_then_the_bare_language() {
        assert_eq!(rank("en_US", "en-US"), Some(0));
        assert_eq!(rank("en_US_large", "en-US"), Some(1));
        assert_eq!(rank("en", "en-US"), Some(2));
        assert_eq!(rank("en_GB", "en-US"), Some(3));
        assert_eq!(rank("de_DE", "en-US"), None);
        assert_eq!(rank("english", "en"), None);
    }

    #[test]
    fn declared_charsets_are_read_from_set() {
        assert_eq!(declared_charset(b"SET ISO8859-1\nTRY a\n").as_deref(), Some("ISO8859-1"));
        assert_eq!(declared_charset(b"TRY a\n"), None);
        assert_eq!(declared_charset(b"\xEF\xBB\xBFSET UTF-8\r\n").as_deref(), Some("UTF-8"));
        assert_eq!(declared_charset(b"SETX y\n"), None);
    }
}
