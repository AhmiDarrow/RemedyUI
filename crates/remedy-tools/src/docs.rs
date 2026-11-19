//! RemedyUI's documentation, searchable: the guides and design documents, the rustdoc of
//! every crate, and the widget catalog.
//!
//! A [`Library`] is either what was embedded when these tools were built
//! ([`Library::embedded`]) or a RemedyUI checkout read now ([`Library::from_checkout`], to
//! follow a checkout as it changes). Searching ranks sections (each heading of a guide, each
//! documented item) with BM25, headings and paths weighing more than body text.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Result, ToolError};

/// The docs index `cargo xtask docs-index` writes (zlib-compressed JSON, see
/// `extract::index_json`), committed so every build has the docs.
static INDEX: &[u8] = include_bytes!("../data/docs-index.json.zz");

/// The committed index, read.
#[derive(Deserialize)]
struct Index {
    version: u32,
    markdown: Vec<(String, String)>,
    rustdoc: Vec<(String, String)>,
    widgets: Vec<Widget>,
}

/// What kind of document a page is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DocKind {
    /// A guide, design document or README.
    Guide,
    /// A crate's API docs (one page per source file).
    Rustdoc,
}

/// One document.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Doc {
    /// Its path (`docs/DESIGN.md`, `rustdoc/remedy-ui-view/src/element.rs`).
    pub path: String,
    /// Guide or rustdoc.
    pub kind: DocKind,
    /// Its text (Markdown).
    pub text: String,
}

impl Doc {
    /// A document at `path` of `kind` holding `text`.
    pub fn new(path: impl Into<String>, kind: DocKind, text: impl Into<String>) -> Self {
        Doc { path: path.into(), kind, text: text.into() }
    }
}

#[derive(Clone, Debug)]
struct Section {
    doc: usize,
    heading: String,
    /// Its first line in the document, from 1.
    line: usize,
    /// Its lines.
    lines: Vec<String>,
    /// Term counts, headings and path counted several times.
    terms: HashMap<String, f32>,
    len: f32,
}

/// The documents and their search index.
#[derive(Clone, Debug)]
pub struct Library {
    docs: Vec<Doc>,
    sections: Vec<Section>,
    /// How many sections hold each term.
    df: HashMap<String, usize>,
    avg_len: f32,
    widgets: Vec<Widget>,
    source: String,
}

/// A search hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Hit {
    /// The document's path, for [`Library::read`].
    pub path: String,
    /// Guide or rustdoc.
    pub kind: DocKind,
    /// The section's heading.
    pub heading: String,
    /// The section's first line, from 1.
    pub line: usize,
    /// How well it matches (higher is better).
    pub score: f32,
    /// The lines that match best, with their line numbers.
    pub excerpt: String,
}

/// What to search for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct SearchInput {
    /// Words to look for, such as "virtual list keys" or "agents consent token".
    pub query: String,
    /// Only guides (`guide`) or only API docs (`rustdoc`); both when absent.
    #[serde(default)]
    pub kind: Option<DocKind>,
    /// How many hits to return (10 when absent).
    #[serde(default)]
    pub limit: Option<usize>,
}

impl SearchInput {
    /// A search for `query`, in every document, ten hits.
    pub fn new(query: impl Into<String>) -> Self {
        SearchInput { query: query.into(), ..Default::default() }
    }
}

/// What to read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ReadInput {
    /// The document's path from a search hit or the listing (`docs/DESIGN.md`,
    /// `rustdoc/remedy-ui-view/src/element.rs`). Empty lists every document.
    #[serde(default)]
    pub path: String,
    /// Read only the section with this heading (case-insensitive, may be part of it).
    #[serde(default)]
    pub heading: Option<String>,
    /// The first line to read, from 1 (1 when absent).
    #[serde(default)]
    pub from_line: Option<usize>,
    /// How many lines to read (all when absent).
    #[serde(default)]
    pub lines: Option<usize>,
}

impl ReadInput {
    /// The whole document at `path`.
    pub fn new(path: impl Into<String>) -> Self {
        ReadInput { path: path.into(), ..Default::default() }
    }
}

/// What was read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ReadOutput {
    /// The document (empty for the listing).
    pub path: String,
    /// Its first line returned, from 1.
    pub from_line: usize,
    /// How many lines it has in all.
    pub total_lines: usize,
    /// The text.
    pub text: String,
}

/// One widget builder of the facade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Widget {
    /// Its name (`button`).
    pub name: String,
    /// Its path through the facade (`remedy_ui::widgets::button`).
    pub path: String,
    /// The `remedy-ui` feature that brings it (`widgets`, `forms`, `nav`, …).
    pub feature: String,
    /// The crate defining it.
    pub krate: String,
    /// Its source file in the RemedyUI checkout.
    pub file: String,
    /// Its line there, from 1.
    pub line: usize,
    /// Its signature.
    pub signature: String,
    /// Its doc comment.
    pub docs: String,
    /// The element names it creates (`ui.el("button")`); theme style sheets key their rules by
    /// these names.
    pub elements: Vec<String>,
    /// The element names the Remedy widget sheet has rules for.
    pub sheet_rules: Vec<String>,
    /// Style variants its docs name that the sheet styles (pass to `.variant(..)`).
    pub variants: Vec<String>,
    /// An example: from its docs when they have one, else a call with each argument shown
    /// for its type.
    pub example: String,
    /// The example came from its docs.
    pub example_from_docs: bool,
}

/// Which widgets to list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct CatalogInput {
    /// Only widgets whose name or path contains this (`button`, `forms::`); all when absent.
    #[serde(default)]
    pub name: Option<String>,
    /// Only widgets of this `remedy-ui` feature (`widgets`, `forms`, `nav`, `data`, …).
    #[serde(default)]
    pub feature: Option<String>,
}

/// Lowercase words of letters, digits and `_`, split also at `_` and `::` so `text_input`
/// matches "text input".
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if word.is_empty() {
            continue;
        }
        let w = word.to_lowercase();
        if w.contains('_') {
            out.extend(w.split('_').filter(|p| p.len() > 1).map(str::to_owned));
        }
        out.push(w);
    }
    out
}

const STOP: &[&str] =
    &["the", "a", "an", "of", "to", "and", "or", "in", "is", "it", "for", "on", "with", "how", "do", "i"];

impl Library {
    /// The documentation committed with these tools (`crates/remedy-tools/data`, kept current
    /// by `cargo xtask docs-index` and checked by `cargo xtask check`).
    pub fn embedded() -> Result<Library> {
        let mut json = String::new();
        flate2::read::ZlibDecoder::new(INDEX)
            .read_to_string(&mut json)
            .map_err(|e| ToolError::new(format!("the embedded docs index is damaged: {e}")))?;
        let index: Index = serde_json::from_str(&json)
            .map_err(|e| ToolError::new(format!("the embedded docs index is damaged: {e}")))?;
        if index.version != 1 {
            return Err(ToolError::new(format!(
                "the embedded docs index is version {}, not 1",
                index.version
            )));
        }
        let mut docs: Vec<Doc> =
            index.markdown.into_iter().map(|(p, t)| Doc { path: p, kind: DocKind::Guide, text: t }).collect();
        docs.extend(index.rustdoc.into_iter().map(|(p, t)| Doc { path: p, kind: DocKind::Rustdoc, text: t }));
        Ok(Library::new(docs, index.widgets, "the index committed with these tools".into()))
    }

    /// The documentation of the RemedyUI checkout at `root`, read now.
    pub fn from_checkout(root: &Path) -> Result<Library> {
        if !root.join("crates").join("remedy-ui").join("Cargo.toml").is_file() {
            return Err(ToolError::new(format!("{} is not a RemedyUI checkout", root.display())));
        }
        let mut docs = Vec::new();
        for f in crate::extract::markdown(root) {
            let text =
                std::fs::read_to_string(&f.file).map_err(|e| ToolError::new(format!("{}: {e}", f.path)))?;
            docs.push(Doc { path: f.path, kind: DocKind::Guide, text });
        }
        for p in crate::extract::rustdoc(root) {
            docs.push(Doc { path: p.path, kind: DocKind::Rustdoc, text: p.text });
        }
        let widgets = crate::extract::widgets(root)
            .into_iter()
            .map(|w| Widget {
                name: w.name,
                path: w.path,
                feature: w.feature,
                krate: w.krate,
                file: w.file,
                line: w.line,
                signature: w.signature,
                docs: w.docs,
                elements: w.elements,
                sheet_rules: w.sheet_rules,
                variants: w.variants,
                example: w.example,
                example_from_docs: w.example_from_docs,
            })
            .collect();
        Ok(Library::new(docs, widgets, format!("read from {}", root.display())))
    }

    /// A library of `docs` and `widgets` (`source` says where they came from).
    pub fn new(docs: Vec<Doc>, widgets: Vec<Widget>, source: String) -> Library {
        let mut sections = Vec::new();
        for (i, d) in docs.iter().enumerate() {
            let path_terms = tokens(&d.path.replace(['/', '.', '-'], " "));
            let mut current = Section {
                doc: i,
                heading: d.path.clone(),
                line: 1,
                lines: Vec::new(),
                terms: HashMap::new(),
                len: 0.0,
            };
            for (n, line) in d.text.lines().enumerate() {
                if line.starts_with('#') && !current.lines.is_empty() {
                    sections.push(std::mem::replace(
                        &mut current,
                        Section {
                            doc: i,
                            heading: line.trim_start_matches('#').trim().to_owned(),
                            line: n + 1,
                            lines: Vec::new(),
                            terms: HashMap::new(),
                            len: 0.0,
                        },
                    ));
                } else if line.starts_with('#') {
                    current.heading = line.trim_start_matches('#').trim().to_owned();
                    current.line = n + 1;
                }
                current.lines.push(line.to_owned());
            }
            sections.push(current);
            for s in sections.iter_mut().filter(|s| s.doc == i) {
                let mut terms: HashMap<String, f32> = HashMap::new();
                let mut len = 0.0f32;
                for t in s.lines.iter().flat_map(|l| tokens(l)) {
                    *terms.entry(t).or_default() += 1.0;
                    len += 1.0;
                }
                for t in tokens(&s.heading) {
                    *terms.entry(t).or_default() += 3.0;
                }
                for t in &path_terms {
                    *terms.entry(t.clone()).or_default() += 1.0;
                }
                s.terms = terms;
                s.len = len.max(1.0);
            }
        }
        let mut df: HashMap<String, usize> = HashMap::new();
        for s in &sections {
            for t in s.terms.keys() {
                *df.entry(t.clone()).or_default() += 1;
            }
        }
        let avg_len = if sections.is_empty() {
            1.0
        } else {
            sections.iter().map(|s| s.len).sum::<f32>() / sections.len() as f32
        };
        Library { docs, sections, df, avg_len, widgets, source }
    }

    /// Where the documents came from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every document.
    pub fn docs(&self) -> &[Doc] {
        &self.docs
    }

    /// The document at `path`.
    pub fn doc(&self, path: &str) -> Option<&Doc> {
        let p = path.trim().trim_start_matches("./").replace('\\', "/");
        self.docs.iter().find(|d| d.path == p)
    }

    /// The sections that best match `input.query`, best first.
    pub fn search(&self, input: &SearchInput) -> Result<Vec<Hit>> {
        if self.docs.is_empty() {
            return Err(ToolError::new("no documentation is loaded"));
        }
        let query: Vec<String> =
            tokens(&input.query).into_iter().filter(|t| !STOP.contains(&t.as_str())).collect();
        if query.is_empty() {
            return Err(ToolError::new("the query has no words to look for"));
        }
        let n = self.sections.len() as f32;
        let (k1, b) = (1.2f32, 0.75f32);
        let mut scored: Vec<(f32, usize)> = Vec::new();
        for (i, s) in self.sections.iter().enumerate() {
            if let Some(k) = input.kind
                && self.docs.get(s.doc).map(|d| d.kind) != Some(k)
            {
                continue;
            }
            let mut score = 0.0;
            let mut matched = 0;
            for q in &query {
                let Some(tf) = s.terms.get(q) else { continue };
                matched += 1;
                let df = *self.df.get(q).unwrap_or(&0) as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                score += idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * s.len / self.avg_len));
            }
            if matched > 0 {
                // Sections matching every word come first.
                score *= 1.0 + matched as f32 / query.len() as f32;
                scored.push((score, i));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let limit = input.limit.unwrap_or(10).max(1);
        Ok(scored
            .into_iter()
            .take(limit)
            .filter_map(|(score, i)| {
                let s = self.sections.get(i)?;
                let d = self.docs.get(s.doc)?;
                Some(Hit {
                    path: d.path.clone(),
                    kind: d.kind,
                    heading: s.heading.clone(),
                    line: s.line,
                    score,
                    excerpt: excerpt(s, &query),
                })
            })
            .collect())
    }

    /// A document, a section of it, or a range of its lines. An empty path lists every
    /// document.
    pub fn read(&self, input: &ReadInput) -> Result<ReadOutput> {
        if input.path.trim().is_empty() {
            let mut text = format!("Documents ({}):\n", self.source);
            for d in &self.docs {
                let kind = match d.kind {
                    DocKind::Guide => "guide",
                    DocKind::Rustdoc => "rustdoc",
                };
                text.push_str(&format!("{kind}\t{}\n", d.path));
            }
            let total = text.lines().count();
            return Ok(ReadOutput { path: String::new(), from_line: 1, total_lines: total, text });
        }
        let Some(doc) = self.doc(&input.path) else {
            let wanted = input.path.to_lowercase();
            let near: Vec<&str> = self
                .docs
                .iter()
                .filter(|d| {
                    let p = d.path.to_lowercase();
                    p.contains(&wanted) || wanted.split('/').next_back().is_some_and(|last| p.contains(last))
                })
                .take(10)
                .map(|d| d.path.as_str())
                .collect();
            let hint = if near.is_empty() {
                "read an empty path for the list".to_owned()
            } else {
                format!("did you mean: {}", near.join(", "))
            };
            return Err(ToolError::new(format!("no document {}; {hint}", input.path)));
        };
        let lines: Vec<&str> = doc.text.lines().collect();
        let total = lines.len();
        let (mut start, mut end) = (0usize, total);
        if let Some(h) = &input.heading {
            let want = h.to_lowercase();
            let found = lines.iter().position(|l| l.starts_with('#') && l.to_lowercase().contains(&want));
            let Some(at) = found else {
                let headings: Vec<&str> =
                    lines.iter().filter(|l| l.starts_with('#')).copied().take(40).collect();
                return Err(ToolError::new(format!(
                    "{} has no heading like `{h}`; its headings: {}",
                    doc.path,
                    headings.join(" | ")
                )));
            };
            let level = lines.get(at).map_or(1, |l| l.chars().take_while(|c| *c == '#').count());
            start = at;
            end = lines
                .iter()
                .enumerate()
                .skip(at + 1)
                .find(|(_, l)| l.starts_with('#') && l.chars().take_while(|c| *c == '#').count() <= level)
                .map_or(total, |(i, _)| i);
        }
        if let Some(from) = input.from_line {
            start = start.max(from.saturating_sub(1));
        }
        if let Some(n) = input.lines {
            end = end.min(start.saturating_add(n));
        }
        let text = lines.get(start..end.max(start)).unwrap_or_default().join("\n");
        Ok(ReadOutput { path: doc.path.clone(), from_line: start + 1, total_lines: total, text })
    }

    /// The widget catalog, filtered.
    pub fn widgets(&self, input: &CatalogInput) -> Vec<Widget> {
        let name = input.name.as_deref().map(str::to_lowercase);
        self.widgets
            .iter()
            .filter(|w| {
                name.as_ref()
                    .is_none_or(|n| w.name.to_lowercase().contains(n) || w.path.to_lowercase().contains(n))
            })
            .filter(|w| input.feature.as_ref().is_none_or(|f| &w.feature == f))
            .cloned()
            .collect()
    }

    /// Every widget.
    pub fn all_widgets(&self) -> &[Widget] {
        &self.widgets
    }
}

/// The best few lines of a section for `query`, numbered.
fn excerpt(s: &Section, query: &[String]) -> String {
    let hits: Vec<usize> = s
        .lines
        .iter()
        .map(|l| {
            let ts = tokens(l);
            query.iter().filter(|q| ts.contains(q)).count()
        })
        .collect();
    let best = hits.iter().enumerate().max_by_key(|(i, h)| (**h, usize::MAX - i)).map_or(0, |(i, _)| i);
    let from = best.saturating_sub(1);
    let to = (best + 3).min(s.lines.len());
    let mut out = String::new();
    for (i, l) in s.lines.iter().enumerate().take(to).skip(from) {
        let trimmed: String = l.chars().take(300).collect();
        out.push_str(&format!("{}: {trimmed}\n", s.line + i));
    }
    out.trim_end().to_owned()
}

/// A widget as a few lines of text.
pub fn describe_widget(w: &Widget) -> String {
    let mut out = format!("{} (feature `{}`)\n  {}\n", w.path, w.feature, w.signature);
    if let Some(first) = w.docs.lines().next().filter(|l| !l.is_empty()) {
        out.push_str(&format!("  {first}\n"));
    }
    if !w.elements.is_empty() {
        out.push_str(&format!("  elements: {}", w.elements.join(", ")));
        if !w.sheet_rules.is_empty() {
            out.push_str(&format!(" (sheet rules: {})", w.sheet_rules.join(", ")));
        }
        out.push('\n');
    }
    if !w.variants.is_empty() {
        out.push_str(&format!("  variants: {}\n", w.variants.join(", ")));
    }
    let label = if w.example_from_docs { "example" } else { "call" };
    out.push_str(&format!("  {label}: {}\n", w.example.replace('\n', "\n    ")));
    out.push_str(&format!("  source: {}:{}\n", w.file, w.line));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> Library {
        Library::new(
            vec![
                Doc {
                    path: "docs/guides/lists.md".into(),
                    kind: DocKind::Guide,
                    text: "# Lists\n\nIntro.\n\n## Virtual lists\n\nA virtual list mounts only visible rows.\nKeys keep rows stable.\n\n## Other\n\nNothing here.\n".into(),
                },
                Doc {
                    path: "rustdoc/remedy-ui-automation/src/lib.rs".into(),
                    kind: DocKind::Rustdoc,
                    text: "# remedy_ui_automation\n\n## `pub fn grant(&self, agent: &str, scope: Scope) -> String`\n\nThe person grants an agent access; returns the token.\n".into(),
                },
            ],
            vec![Widget {
                name: "button".into(),
                path: "remedy_ui::widgets::button".into(),
                feature: "widgets".into(),
                krate: "remedy-ui-widgets-core".into(),
                file: "crates/remedy-ui-widgets-core/src/actions.rs".into(),
                line: 19,
                signature: "pub fn button(ui: &Ui, label: impl Into<String>, on_press: impl Fn() + 'static) -> El".into(),
                docs: "A button.".into(),
                elements: vec!["button".into(), "button_label".into()],
                sheet_rules: vec!["button".into()],
                variants: vec!["primary".into()],
                example: "remedy_ui::widgets::button(ui, \"Label\", || {})".into(),
                example_from_docs: false,
            }],
            "test".into(),
        )
    }

    #[test]
    fn search_ranks_the_matching_section_first_with_an_excerpt() {
        let l = lib();
        let hits = l.search(&SearchInput { query: "virtual list keys".into(), ..Default::default() });
        let Ok(hits) = hits else { unreachable!("documents are loaded") };
        let Some(first) = hits.first() else { unreachable!("a hit") };
        assert_eq!(first.heading, "Virtual lists");
        assert_eq!(first.line, 5);
        assert!(first.excerpt.contains("7: A virtual list"), "{}", first.excerpt);
        let api =
            l.search(&SearchInput { query: "grant token".into(), kind: Some(DocKind::Rustdoc), limit: None });
        assert!(api.is_ok_and(|h| h.first().is_some_and(|h| h.path.starts_with("rustdoc/"))));
        assert!(l.search(&SearchInput { query: "the".into(), ..Default::default() }).is_err());
    }

    #[test]
    fn reading_returns_documents_sections_and_line_ranges() {
        let l = lib();
        let all = l.read(&ReadInput::default());
        assert!(all.is_ok_and(|r| r.text.contains("guide\tdocs/guides/lists.md")));
        let sec = l.read(&ReadInput {
            path: "docs/guides/lists.md".into(),
            heading: Some("virtual".into()),
            ..Default::default()
        });
        let Ok(sec) = sec else { unreachable!("the section exists") };
        assert!(
            sec.text.starts_with("## Virtual lists") && !sec.text.contains("Nothing here"),
            "{}",
            sec.text
        );
        let range = l.read(&ReadInput {
            path: "docs/guides/lists.md".into(),
            from_line: Some(5),
            lines: Some(1),
            ..Default::default()
        });
        assert!(range.is_ok_and(|r| r.text == "## Virtual lists" && r.from_line == 5));
        let missing = l.read(&ReadInput { path: "lists.md".into(), ..Default::default() });
        assert!(missing.is_err_and(|e| e.0.contains("docs/guides/lists.md")));
    }

    #[test]
    fn the_catalog_filters_by_name_and_feature() {
        let l = lib();
        assert_eq!(l.widgets(&CatalogInput { name: Some("butt".into()), feature: None }).len(), 1);
        assert!(l.widgets(&CatalogInput { name: None, feature: Some("forms".into()) }).is_empty());
        let Some(b) = l.all_widgets().first() else { unreachable!("one widget") };
        assert!(describe_widget(b).contains("sheet rules: button"));
    }

    #[test]
    fn the_embedded_library_has_the_design_docs_and_widgets() {
        let Ok(l) = Library::embedded() else { unreachable!("the committed index reads") };
        assert!(l.doc("docs/DESIGN.md").is_some(), "the design overview is embedded");
        assert!(l.doc("crates/remedy-ui/src/guide/getting_started.md").is_some(), "and the guides");
        assert!(l.docs().iter().any(|d| d.path.starts_with("rustdoc/remedy-ui-view/")));
        let buttons =
            l.widgets(&CatalogInput { name: Some("button".into()), feature: Some("widgets".into()) });
        let Some(b) = buttons.iter().find(|w| w.name == "button") else {
            unreachable!("button is in the catalog")
        };
        assert_eq!(b.path, "remedy_ui::widgets::button");
        assert!(b.elements.contains(&"button".to_owned()) && b.sheet_rules.contains(&"button".to_owned()));
        assert!(b.variants.contains(&"primary".to_owned()), "{:?}", b.variants);
        let hits = l.search(&SearchInput { query: "agents consent token".into(), ..Default::default() });
        assert!(hits.is_ok_and(|h| !h.is_empty()));
    }
}
