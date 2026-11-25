//! Catalogs keyed by source text: `{"Share a folder": "Partager un dossier"}` per locale, the
//! gettext way, for apps whose strings are their English text and whose catalogs are shared
//! with other implementations (a Python app, a web page). A string missing from a catalog
//! falls back to the source text; `{name}` placeholders are filled after translation.
//!
//! Text that arrives in the source language from elsewhere (an engine's error, a reply from
//! another program) is translated by [`Catalog::translate`], which also recognizes keys with
//! placeholders: "Could not finish that transfer: {text}" matches "Could not finish that
//! transfer: File is too large", and the part in the placeholder is translated in turn.
//!
//! **User overrides.** With [`Catalog::override_folder`], every string can be rewritten by
//! the person using the app: `<locale>.json` files in that folder, in the same source-text
//! format, take precedence over the app's translations (and over the source text, for the
//! source locale). A file that does not parse is skipped and reported by
//! [`Catalog::override_problems`]; the rest still apply. [`Catalog::sources`] lists every
//! string the app has (its catalogs' keys and every string it has asked for), for editors.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::Ui;
use unic_langid::LanguageIdentifier;

/// A user override file that could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OverrideProblem {
    /// The file (or the folder, when it could not be read).
    pub path: PathBuf,
    /// What went wrong.
    pub detail: String,
}

impl std::fmt::Display for OverrideProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.detail)
    }
}

/// One key with placeholders, split into literal pieces and names, for matching text.
#[derive(Clone, Debug)]
struct Template {
    key: String,
    /// Literal, name, literal, name, …, literal (always one more literal than names).
    literals: Vec<String>,
    names: Vec<String>,
}

type Table = Rc<HashMap<String, String>>;

/// How deep [`Catalog::translate_in`] follows placeholders inside placeholders: text from
/// elsewhere can repeat a key's words without end ("Error: Error: Error: …"), and what lies
/// deeper stays as it is.
const TRANSLATE_DEPTH: u8 = 8;

#[derive(Debug)]
struct Inner {
    source: String,
    tables: RefCell<HashMap<String, Table>>,
    templates: RefCell<Vec<Template>>,
    chosen: RefCell<HashMap<String, Option<String>>>,
    problems: RefCell<Vec<String>>,
    /// The user's override folder, and its tables by canonical locale.
    override_dir: RefCell<Option<PathBuf>>,
    overrides: RefCell<HashMap<String, Table>>,
    override_chosen: RefCell<HashMap<String, Option<String>>>,
    override_problems: RefCell<Vec<OverrideProblem>>,
    /// Every source string asked for, so editors can offer strings no catalog lists.
    seen: RefCell<BTreeSet<String>>,
    /// Bumped when the overrides change: text that read the catalog reads it again.
    revision: Signal<u64>,
}

/// Source-keyed translations for a set of locales. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Catalog(Rc<Inner>);

/// Splits `{name}` placeholders out of `s`: literals and names, or `None` without any.
fn split(s: &str) -> Option<(Vec<String>, Vec<String>)> {
    let mut literals = Vec::new();
    let mut names = Vec::new();
    let mut lit = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let after = rest.get(open + 1..).unwrap_or_default();
        let close = after.find('}');
        let name = close.and_then(|c| after.get(..c)).filter(|n| {
            let mut ch = n.chars();
            ch.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        match (name, close) {
            (Some(n), Some(c)) => {
                lit.push_str(rest.get(..open).unwrap_or_default());
                literals.push(std::mem::take(&mut lit));
                names.push(n.to_owned());
                rest = after.get(c + 1..).unwrap_or_default();
            }
            _ => {
                lit.push_str(rest.get(..=open).unwrap_or_default());
                rest = after;
            }
        }
    }
    lit.push_str(rest);
    literals.push(lit);
    (!names.is_empty()).then_some((literals, names))
}

/// Fills `{name}` in `s` from `args` (names without a value stay as written).
pub fn fill(s: &str, args: &[(&str, String)]) -> String {
    let Some((literals, names)) = split(s) else { return s.to_owned() };
    let mut out = String::new();
    for (i, lit) in literals.iter().enumerate() {
        out.push_str(lit);
        if let Some(n) = names.get(i) {
            match args.iter().find(|(k, _)| k == n) {
                Some((_, v)) => out.push_str(v),
                None => {
                    out.push('{');
                    out.push_str(n);
                    out.push('}');
                }
            }
        }
    }
    out
}

impl Template {
    /// The placeholder values if `text` reads as this key with something in each placeholder.
    fn matches(&self, text: &str) -> Option<Vec<(String, String)>> {
        let first = self.literals.first()?;
        let mut rest = text.strip_prefix(first.as_str())?;
        let mut got = Vec::new();
        for (i, name) in self.names.iter().enumerate() {
            let next = self.literals.get(i + 1)?;
            let last = i + 1 == self.names.len();
            let value = if last {
                // The last placeholder takes everything up to the final literal.
                let v = rest.strip_suffix(next.as_str())?;
                rest = "";
                v
            } else if next.is_empty() {
                return None;
            } else {
                let at = rest.find(next.as_str())?;
                let v = rest.get(..at)?;
                rest = rest.get(at + next.len()..)?;
                v
            };
            if value.is_empty() {
                return None;
            }
            got.push((name.clone(), value.to_owned()));
        }
        rest.is_empty().then_some(got)
    }
}

/// `requested` as a maximized language identifier (`pt` → `pt-Latn-BR`), or `None`.
fn maximized(tag: &str) -> Option<LanguageIdentifier> {
    let mut l: LanguageIdentifier = tag.replace('_', "-").parse().ok()?;
    l.maximize();
    Some(l)
}

/// The key among `keys` that best serves `requested`: an exact match, then the same
/// language, script and region, then the same language and script.
fn best_of<'a>(keys: impl Iterator<Item = &'a String>, requested: &str) -> Option<String> {
    let w = maximized(requested)?;
    let mut keys: Vec<&String> = keys.collect();
    keys.sort();
    let exact = keys.iter().find(|k| k.eq_ignore_ascii_case(&requested.replace('_', "-")));
    let same = |region: bool| {
        keys.iter().find(|k| {
            maximized(k).is_some_and(|a| {
                a.language == w.language && a.script == w.script && (!region || a.region == w.region)
            })
        })
    };
    exact.or_else(|| same(true)).or_else(|| same(false)).map(|k| (*k).clone())
}

/// `de_DE` or `de-de` as `de-DE` (the tag as given when it does not parse).
pub fn canonical(tag: &str) -> String {
    tag.replace('_', "-").parse::<LanguageIdentifier>().map_or_else(|_| tag.to_owned(), |l| l.to_string())
}

/// Reads `<locale>.json` override files in `dir`.
fn read_overrides(dir: &Path) -> (HashMap<String, Table>, Vec<OverrideProblem>) {
    let mut tables = HashMap::new();
    let mut problems = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (tables, problems),
        Err(e) => {
            problems.push(OverrideProblem { path: dir.to_owned(), detail: e.to_string() });
            return (tables, problems);
        }
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for path in files {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if stem.replace('_', "-").parse::<LanguageIdentifier>().is_err() {
            // Not a locale's file: other tools may keep their own files beside these.
            continue;
        }
        let read = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|text| {
            serde_json::from_str::<HashMap<String, String>>(&text)
                .map_err(|e| format!("not an object of source text to text: {e}"))
        });
        match read {
            Ok(table) => {
                tables.insert(canonical(&stem), Rc::new(table));
            }
            Err(detail) => problems.push(OverrideProblem { path, detail }),
        }
    }
    (tables, problems)
}

impl Catalog {
    /// A catalog whose keys are in `source_locale` (the fallback for every string).
    pub fn new(source_locale: &str) -> Catalog {
        Catalog(Rc::new(Inner {
            source: source_locale.to_owned(),
            tables: RefCell::default(),
            templates: RefCell::default(),
            chosen: RefCell::default(),
            problems: RefCell::default(),
            override_dir: RefCell::default(),
            overrides: RefCell::default(),
            override_chosen: RefCell::default(),
            override_problems: RefCell::default(),
            seen: RefCell::default(),
            revision: Signal::new(0),
        }))
    }

    /// The locale the keys are written in.
    pub fn source_locale(&self) -> String {
        self.0.source.clone()
    }

    /// Reads the user's overrides from `dir` (see the module docs); a missing folder is
    /// simply no overrides.
    pub fn override_folder(self, dir: impl Into<PathBuf>) -> Catalog {
        self.set_override_folder(Some(dir.into()));
        self
    }

    /// Changes (or, with `None`, removes) the override folder, and reads it.
    pub fn set_override_folder(&self, dir: Option<PathBuf>) {
        *self.0.override_dir.borrow_mut() = dir;
        self.reload_overrides();
    }

    /// The override folder, if there is one.
    pub fn override_dir(&self) -> Option<PathBuf> {
        self.0.override_dir.borrow().clone()
    }

    /// Reads the override files again (after they changed), and has every text that came
    /// from the catalog read it again.
    pub fn reload_overrides(&self) {
        let dir = self.0.override_dir.borrow().clone();
        let (tables, problems) = dir.as_deref().map(read_overrides).unwrap_or_default();
        *self.0.overrides.borrow_mut() = tables;
        *self.0.override_problems.borrow_mut() = problems;
        self.0.override_chosen.borrow_mut().clear();
        self.0.revision.update(|r| *r += 1);
    }

    /// Override files that could not be used (they are skipped; the others apply).
    pub fn override_problems(&self) -> Vec<OverrideProblem> {
        self.0.override_problems.borrow().clone()
    }

    /// The locales with override files, as canonical tags.
    pub fn override_locales(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.overrides.borrow().keys().cloned().collect();
        v.sort();
        v
    }

    /// The overrides in the file for exactly `locale` (empty without one).
    pub fn overrides(&self, locale: &str) -> HashMap<String, String> {
        self.0.overrides.borrow().get(&canonical(locale)).map(|t| (**t).clone()).unwrap_or_default()
    }

    /// The override file locale that serves `requested`: an existing file's locale when one
    /// matches (so a person in `en-US` edits `en.json` if that is what they have), otherwise
    /// `requested` itself.
    pub fn override_locale_for(&self, requested: &str) -> String {
        self.override_best(requested).unwrap_or_else(|| canonical(requested))
    }

    fn override_best(&self, requested: &str) -> Option<String> {
        if let Some(c) = self.0.override_chosen.borrow().get(requested) {
            return c.clone();
        }
        let chosen = best_of(self.0.overrides.borrow().keys(), requested);
        self.0.override_chosen.borrow_mut().insert(requested.to_owned(), chosen.clone());
        chosen
    }

    /// The person's override of `source` in `locale`, if there is one (a blank override is
    /// never shown, like a blank translation).
    pub fn override_in(&self, locale: &str, source: &str) -> Option<String> {
        if self.0.overrides.borrow().is_empty() {
            return None;
        }
        let best = self.override_best(locale)?;
        self.0
            .overrides
            .borrow()
            .get(&best)
            .and_then(|t| t.get(source))
            .filter(|s| !s.trim().is_empty())
            .cloned()
    }

    /// Every source string the app has: its catalogs' keys and every string it has asked
    /// the catalog for, sorted. For editors that let people rewrite the app's text.
    pub fn sources(&self) -> Vec<String> {
        self.0.revision.get();
        let mut all = self.0.seen.borrow().clone();
        for t in self.0.tables.borrow().values() {
            all.extend(t.keys().cloned());
        }
        all.into_iter().collect()
    }

    fn note_seen(&self, source: &str) {
        if !self.0.seen.borrow().contains(source) {
            self.0.seen.borrow_mut().insert(source.to_owned());
        }
    }

    /// Adds the translations for `locale` from a JSON object of source text to translation.
    /// Keys of the source locale itself are where placeholders are learned from. A file that
    /// does not parse is reported by [`Catalog::problems`] and left out.
    pub fn embedded_json(self, locale: &str, json: &str) -> Catalog {
        match serde_json::from_str::<HashMap<String, String>>(json) {
            Ok(table) => {
                let mut templates = self.0.templates.borrow_mut();
                for key in table.keys() {
                    if let Some((literals, names)) = split(key)
                        && !templates.iter().any(|t| t.key == *key)
                    {
                        templates.push(Template { key: key.clone(), literals, names });
                    }
                }
                // Longer keys first: the most specific template wins.
                templates.sort_by_key(|t| std::cmp::Reverse(t.key.len()));
                drop(templates);
                self.0.tables.borrow_mut().insert(locale.to_owned(), Rc::new(table));
                self.0.chosen.borrow_mut().clear();
            }
            Err(e) => self.0.problems.borrow_mut().push(format!("{locale}: {e}")),
        }
        self
    }

    /// The locales with translations, and the source locale.
    pub fn locales(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.tables.borrow().keys().cloned().collect();
        if !v.contains(&self.0.source) {
            v.push(self.0.source.clone());
        }
        v.sort();
        v
    }

    /// Catalogs that did not parse.
    pub fn problems(&self) -> Vec<String> {
        self.0.problems.borrow().clone()
    }

    /// The available locale that best serves `requested` (a BCP 47 tag: an exact match, then
    /// the same language and script — so `pt-PT` takes `pt-BR`, while `zh-TW` does not take
    /// `zh-CN`), or `None`: the source text then.
    pub fn best_locale(&self, requested: &str) -> Option<String> {
        if let Some(c) = self.0.chosen.borrow().get(requested) {
            return c.clone();
        }
        let chosen = best_of(self.0.tables.borrow().keys(), requested);
        self.0.chosen.borrow_mut().insert(requested.to_owned(), chosen.clone());
        chosen
    }

    fn table_for(&self, locale: &str) -> Option<Table> {
        let best = self.best_locale(locale)?;
        self.0.tables.borrow().get(&best).cloned()
    }

    /// `source` in `locale` without the person's override: the app's translation, or the
    /// source text when there is none. What an override editor shows as the default.
    pub fn translation_in(&self, locale: &str, source: &str) -> String {
        self.table_for(locale)
            .and_then(|t| t.get(source).filter(|s| !s.trim().is_empty()).cloned())
            .unwrap_or_else(|| source.to_owned())
    }

    /// `source` in `locale`: the person's override, else the translation, else the source
    /// text.
    pub fn text_in(&self, locale: &str, source: &str) -> String {
        self.note_seen(source);
        self.override_in(locale, source).unwrap_or_else(|| self.translation_in(locale, source))
    }

    /// `source` in `locale` with `{name}` placeholders filled.
    pub fn format_in(&self, locale: &str, source: &str, args: &[(&str, String)]) -> String {
        fill(&self.text_in(locale, source), args)
    }

    /// `source` in the window's language (tracked: a closure that calls this re-runs when the
    /// language changes).
    pub fn text(&self, ui: &Ui, source: &str) -> String {
        self.0.revision.get();
        self.text_in(&ui.locale(), source)
    }

    /// [`Catalog::text`] with placeholders filled.
    pub fn format(&self, ui: &Ui, source: &str, args: &[(&str, String)]) -> String {
        self.0.revision.get();
        self.format_in(&ui.locale(), source, args)
    }

    /// Text in the source language from elsewhere, in `locale`: an exact key, or a key with
    /// placeholders whose values are translated in turn. Unknown text stays as it is.
    pub fn translate_in(&self, locale: &str, text: &str) -> String {
        self.translate_within(locale, text, TRANSLATE_DEPTH)
    }

    /// [`Catalog::translate_in`], following placeholders `depth` deep.
    fn translate_within(&self, locale: &str, text: &str, depth: u8) -> String {
        if let Some(o) = self.override_in(locale, text) {
            return o;
        }
        if self.table_for(locale).is_none() && self.0.overrides.borrow().is_empty() {
            return text.to_owned();
        }
        if let Some(t) =
            self.table_for(locale).and_then(|table| table.get(text).filter(|s| !s.trim().is_empty()).cloned())
        {
            return t;
        }
        let Some(deeper) = depth.checked_sub(1) else { return text.to_owned() };
        let templates = self.0.templates.borrow().clone();
        for t in &templates {
            if let Some(values) = t.matches(text) {
                let args: Vec<(&str, String)> = values
                    .iter()
                    .map(|(n, v)| (n.as_str(), self.translate_within(locale, v, deeper)))
                    .collect();
                return fill(&self.text_in(locale, &t.key), &args);
            }
        }
        text.to_owned()
    }

    /// [`Catalog::translate_in`] in the window's language (tracked).
    pub fn translate(&self, ui: &Ui, text: &str) -> String {
        self.0.revision.get();
        self.translate_in(&ui.locale(), text)
    }
}

#[cfg(test)]
mod tests {
    use super::{Catalog, fill};

    fn catalog() -> Catalog {
        Catalog::new("en")
            .embedded_json(
                "fr",
                r#"{"Share a folder": "Partager un dossier", "Could not finish that transfer: {text}": "Impossible de terminer ce transfert : {text}", "File is too large": "Fichier trop volumineux", "Downloading {count} files": "Téléchargement de {count} fichiers", "Empty": ""}"#,
            )
            .embedded_json("pt-BR", r#"{"Share a folder": "Compartilhar uma pasta"}"#)
            .embedded_json("zh-CN", r#"{"Share a folder": "共享文件夹"}"#)
    }

    #[test]
    fn strings_translate_and_fall_back_to_the_source() {
        let c = catalog();
        assert_eq!(c.text_in("fr-FR", "Share a folder"), "Partager un dossier");
        assert_eq!(c.text_in("fr", "Not in the catalog"), "Not in the catalog");
        assert_eq!(c.text_in("fr", "Empty"), "Empty", "a blank translation is never shown");
        assert_eq!(c.text_in("de-DE", "Share a folder"), "Share a folder");
        assert_eq!(
            c.format_in("fr", "Downloading {count} files", &[("count", "3".into())]),
            "Téléchargement de 3 fichiers"
        );
    }

    #[test]
    fn locales_match_by_language_and_script() {
        let c = catalog();
        assert_eq!(c.best_locale("pt-PT").as_deref(), Some("pt-BR"));
        assert_eq!(c.best_locale("pt").as_deref(), Some("pt-BR"));
        assert_eq!(c.best_locale("zh-Hans-SG").as_deref(), Some("zh-CN"));
        assert_eq!(c.best_locale("zh-TW"), None, "Traditional Chinese is not Simplified");
        assert_eq!(c.best_locale("fr_CA").as_deref(), Some("fr"));
        assert_eq!(c.best_locale("en-US"), None);
        assert_eq!(c.locales(), ["en", "fr", "pt-BR", "zh-CN"]);
    }

    #[test]
    fn text_from_elsewhere_is_recognized_with_its_placeholders() {
        let c = catalog();
        assert_eq!(
            c.translate_in("fr", "Could not finish that transfer: File is too large"),
            "Impossible de terminer ce transfert : Fichier trop volumineux"
        );
        assert_eq!(
            c.translate_in("fr", "Could not finish that transfer: disk on fire"),
            "Impossible de terminer ce transfert : disk on fire"
        );
        assert_eq!(c.translate_in("fr", "Something else entirely"), "Something else entirely");
        assert_eq!(c.translate_in("de", "Share a folder"), "Share a folder");
    }

    #[test]
    fn text_that_repeats_a_key_without_end_is_followed_only_so_deep() {
        let c = catalog().embedded_json("fr", r#"{"Error: {text}": "Erreur : {text}", "{text}": "{text}"}"#);
        let nested = format!("{}disk on fire", "Error: ".repeat(20_000));
        let out = c.translate_in("fr", &nested);
        assert!(out.starts_with("Erreur : Erreur : "), "the outer levels are translated");
        assert!(out.ends_with("Error: disk on fire"), "the rest stays as it came");
        // A key that is only a placeholder matches everything, itself included.
        assert_eq!(c.translate_in("fr", "anything"), "anything");
    }

    #[test]
    fn placeholders_fill_and_odd_braces_stay() {
        assert_eq!(fill("{a} and {b}", &[("a", "1".into()), ("b", "2".into())]), "1 and 2");
        assert_eq!(fill("{a} and {missing}", &[("a", "1".into())]), "1 and {missing}");
        assert_eq!(fill("json {\"x\": 1} {a}", &[("a", "y".into())]), "json {\"x\": 1} y");
        assert_eq!(fill("no placeholders", &[]), "no placeholders");
    }

    #[test]
    fn the_persons_overrides_win_and_a_broken_file_is_skipped() {
        let dir = std::env::temp_dir().join(format!("remedy-catalog-overrides-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("temp dir: {e}"));
        let write = |name: &str, text: &str| {
            std::fs::write(dir.join(name), text).unwrap_or_else(|e| unreachable!("write {name}: {e}"));
        };
        write(
            "fr.json",
            r#"{"Share a folder": "Partager un répertoire", "Downloading {count} files": "{count} fichiers arrivent"}"#,
        );
        write("en.json", r#"{"Share a folder": "Share a directory", "Blank": "  "}"#);
        write("de.json", "{ not json");
        write("notes.txt", "not an override file, left alone");
        let c = catalog().override_folder(&dir);
        assert_eq!(c.text_in("fr-FR", "Share a folder"), "Partager un répertoire");
        assert_eq!(c.translation_in("fr-FR", "Share a folder"), "Partager un dossier");
        assert_eq!(c.text_in("en-US", "Share a folder"), "Share a directory", "the source locale too");
        assert_eq!(c.text_in("en", "Blank"), "Blank", "a blank override is never shown");
        assert_eq!(
            c.format_in("fr", "Downloading {count} files", &[("count", "3".into())]),
            "3 fichiers arrivent"
        );
        assert_eq!(c.translate_in("fr", "Share a folder"), "Partager un répertoire");
        assert_eq!(c.text_in("pt-BR", "Share a folder"), "Compartilhar uma pasta", "no override there");
        let problems = c.override_problems();
        assert_eq!(problems.len(), 1);
        assert!(problems.iter().all(|p| p.path.ends_with("de.json")));
        assert_eq!(c.override_locales(), ["en", "fr"]);
        assert_eq!(c.override_locale_for("en-GB"), "en");
        assert_eq!(c.override_locale_for("de_AT"), "de-AT");
        assert!(c.sources().contains(&"Blank".to_owned()), "strings asked for are listed");
        assert!(c.sources().contains(&"Downloading {count} files".to_owned()), "and catalog keys");

        // Edited while the app runs: read again.
        write("fr.json", r#"{"Share a folder": "Partager"}"#);
        c.reload_overrides();
        assert_eq!(c.text_in("fr", "Share a folder"), "Partager");
        c.set_override_folder(None);
        assert_eq!(c.text_in("fr", "Share a folder"), "Partager un dossier");
        assert!(c.override_problems().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_catalog_is_reported_not_fatal() {
        let c = Catalog::new("en").embedded_json("fr", "{not json");
        assert_eq!(c.problems().len(), 1);
        assert_eq!(c.text_in("fr", "Share a folder"), "Share a folder");
    }
}
