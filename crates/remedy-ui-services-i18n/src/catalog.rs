//! Catalogs keyed by source text: `{"Share a folder": "Partager un dossier"}` per locale, the
//! gettext way, for apps whose strings are their English text and whose catalogs are shared
//! with other implementations (a Python app, a web page). A string missing from a catalog
//! falls back to the source text; `{name}` placeholders are filled after translation.
//!
//! Text that arrives in the source language from elsewhere (an engine's error, a reply from
//! another program) is translated by [`Catalog::translate`], which also recognizes keys with
//! placeholders: "Could not finish that transfer: {text}" matches "Could not finish that
//! transfer: File is too large", and the part in the placeholder is translated in turn.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use remedy_ui_view::Ui;
use unic_langid::LanguageIdentifier;

/// One key with placeholders, split into literal pieces and names, for matching text.
#[derive(Clone, Debug)]
struct Template {
    key: String,
    /// Literal, name, literal, name, …, literal (always one more literal than names).
    literals: Vec<String>,
    names: Vec<String>,
}

#[derive(Debug, Default)]
struct Inner {
    source: String,
    tables: RefCell<HashMap<String, Rc<HashMap<String, String>>>>,
    templates: RefCell<Vec<Template>>,
    chosen: RefCell<HashMap<String, Option<String>>>,
    problems: RefCell<Vec<String>>,
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

impl Catalog {
    /// A catalog whose keys are in `source_locale` (the fallback for every string).
    pub fn new(source_locale: &str) -> Catalog {
        Catalog(Rc::new(Inner { source: source_locale.to_owned(), ..Inner::default() }))
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
        let tables = self.0.tables.borrow();
        let max = |tag: &str| -> Option<LanguageIdentifier> {
            let mut l: LanguageIdentifier = tag.replace('_', "-").parse().ok()?;
            l.maximize();
            Some(l)
        };
        let chosen = max(requested).and_then(|w| {
            let mut keys: Vec<&String> = tables.keys().collect();
            keys.sort();
            let exact = keys.iter().find(|k| k.eq_ignore_ascii_case(&requested.replace('_', "-")));
            let same = |region: bool| {
                keys.iter().find(|k| {
                    max(k).is_some_and(|a| {
                        a.language == w.language && a.script == w.script && (!region || a.region == w.region)
                    })
                })
            };
            exact.or_else(|| same(true)).or_else(|| same(false)).map(|k| (*k).clone())
        });
        drop(tables);
        self.0.chosen.borrow_mut().insert(requested.to_owned(), chosen.clone());
        chosen
    }

    fn table_for(&self, locale: &str) -> Option<Rc<HashMap<String, String>>> {
        let best = self.best_locale(locale)?;
        self.0.tables.borrow().get(&best).cloned()
    }

    /// `source` in `locale` (the source text when there is no translation).
    pub fn text_in(&self, locale: &str, source: &str) -> String {
        self.table_for(locale)
            .and_then(|t| t.get(source).filter(|s| !s.trim().is_empty()).cloned())
            .unwrap_or_else(|| source.to_owned())
    }

    /// `source` in `locale` with `{name}` placeholders filled.
    pub fn format_in(&self, locale: &str, source: &str, args: &[(&str, String)]) -> String {
        fill(&self.text_in(locale, source), args)
    }

    /// `source` in the window's language (tracked: a closure that calls this re-runs when the
    /// language changes).
    pub fn text(&self, ui: &Ui, source: &str) -> String {
        self.text_in(&ui.locale(), source)
    }

    /// [`Catalog::text`] with placeholders filled.
    pub fn format(&self, ui: &Ui, source: &str, args: &[(&str, String)]) -> String {
        self.format_in(&ui.locale(), source, args)
    }

    /// Text in the source language from elsewhere, in `locale`: an exact key, or a key with
    /// placeholders whose values are translated in turn. Unknown text stays as it is.
    pub fn translate_in(&self, locale: &str, text: &str) -> String {
        let Some(table) = self.table_for(locale) else { return text.to_owned() };
        if let Some(t) = table.get(text).filter(|s| !s.trim().is_empty()) {
            return t.clone();
        }
        let templates = self.0.templates.borrow().clone();
        for t in &templates {
            if let Some(values) = t.matches(text) {
                let args: Vec<(&str, String)> =
                    values.iter().map(|(n, v)| (n.as_str(), self.translate_in(locale, v))).collect();
                return fill(&self.text_in(locale, &t.key), &args);
            }
        }
        text.to_owned()
    }

    /// [`Catalog::translate_in`] in the window's language (tracked).
    pub fn translate(&self, ui: &Ui, text: &str) -> String {
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
    fn placeholders_fill_and_odd_braces_stay() {
        assert_eq!(fill("{a} and {b}", &[("a", "1".into()), ("b", "2".into())]), "1 and 2");
        assert_eq!(fill("{a} and {missing}", &[("a", "1".into())]), "1 and {missing}");
        assert_eq!(fill("json {\"x\": 1} {a}", &[("a", "y".into())]), "json {\"x\": 1} y");
        assert_eq!(fill("no placeholders", &[]), "no placeholders");
    }

    #[test]
    fn a_broken_catalog_is_reported_not_fatal() {
        let c = Catalog::new("en").embedded_json("fr", "{not json");
        assert_eq!(c.problems().len(), 1);
        assert_eq!(c.text_in("fr", "Share a folder"), "Share a folder");
    }
}
