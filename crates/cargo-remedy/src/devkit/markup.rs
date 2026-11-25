//! `.rui` markup: check a file against the built-in elements and the app's own, with errors
//! at file, line and column; compile it to the Rust the build script would generate.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

use remedy_ui_widgets::rui::spec::{self, Kind, Param, Spec};
use remedy_ui_widgets::rui::{Custom, Markup, RuiError};
use serde::{Deserialize, Serialize};

use crate::devkit::{Result, ToolError, Workspace};

/// One argument an app-declared element takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ParamSpec {
    /// Its name.
    pub name: String,
    /// What it takes: label, text, flag, number, choice, items, level, command, range,
    /// count, space, switch or word.
    pub kind: String,
    /// It must be given (true when absent).
    #[serde(default = "yes")]
    pub required: bool,
}

impl ParamSpec {
    /// A required parameter `name` of `kind` (see [`kind`]).
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        ParamSpec { name: name.into(), kind: kind.into(), required: true }
    }
}

fn yes() -> bool {
    true
}

/// One of the app's own elements.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ElementSpec {
    /// Its name in markup.
    pub name: String,
    /// What it takes, besides the layout and style arguments every element takes.
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    /// It holds children.
    #[serde(default)]
    pub children: bool,
    /// The Rust path of its builder (`crate::devkit::ui::fretboard`, a `fn(&Ui, Call) -> El`); needed
    /// to compile, not to check.
    #[serde(default)]
    pub path: Option<String>,
}

impl ElementSpec {
    /// An element `name` taking `params`, holding no children, with no builder path.
    pub fn new(name: impl Into<String>, params: Vec<ParamSpec>) -> Self {
        ElementSpec { name: name.into(), params, children: false, path: None }
    }
}

/// Markup to check or compile.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct MarkupInput {
    /// The markup itself. Give this or `path`.
    #[serde(default)]
    pub source: Option<String>,
    /// A `.rui` file to read (inside the workspace). Give this or `source`.
    #[serde(default)]
    pub path: Option<String>,
    /// The file name errors name (the path's file name, or `screen.rui`, when absent).
    #[serde(default)]
    pub file: Option<String>,
    /// The app's own elements, beside the built-in ones.
    #[serde(default)]
    pub elements: Vec<ElementSpec>,
}

impl MarkupInput {
    /// Markup given as text.
    pub fn from_source(source: impl Into<String>) -> Self {
        MarkupInput { source: Some(source.into()), ..Default::default() }
    }

    /// Markup in the file at `path`.
    pub fn from_path(path: impl Into<String>) -> Self {
        MarkupInput { path: Some(path.into()), ..Default::default() }
    }
}

/// A problem in a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct MarkupError {
    /// The file.
    pub file: String,
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1, in characters.
    pub column: u32,
    /// What is wrong.
    pub message: String,
    /// What would be right.
    pub help: Option<String>,
}

impl MarkupError {
    fn from(e: RuiError) -> Self {
        MarkupError { file: e.file, line: e.at.line, column: e.at.col, message: e.message, help: e.help }
    }
}

impl std::fmt::Display for MarkupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}: {}", self.file, self.line, self.column, self.message)?;
        if let Some(h) = &self.help {
            write!(f, " ({h})")?;
        }
        Ok(())
    }
}

/// A signal a file binds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct SignalUse {
    /// Its name (`$name` in the file).
    pub name: String,
    /// The kind it is used as.
    pub kind: String,
    /// The Rust type its signal holds (`bool`, `String`, …).
    pub rust_type: String,
}

/// The outcome of a check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct CheckOutput {
    /// The file is valid.
    pub ok: bool,
    /// The first problem, if any (checking stops there).
    pub error: Option<MarkupError>,
    /// The signals it binds, which the app must expose.
    pub signals: Vec<SignalUse>,
    /// The commands it names (`!name`), which the app must expose.
    pub commands: Vec<String>,
    /// The elements it may use, built-in and the app's.
    pub elements: Vec<String>,
}

/// The outcome of compiling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct CompileOutput {
    /// It compiled.
    pub ok: bool,
    /// The problem, if any.
    pub error: Option<MarkupError>,
    /// The Rust module (a `Bindings` struct and `build`), when it compiled.
    pub rust: Option<String>,
}

/// The kind called `name`.
pub fn kind(name: &str) -> Option<Kind> {
    Some(match name.trim().to_lowercase().as_str() {
        "label" => Kind::Label,
        "text" => Kind::Text,
        "flag" => Kind::Flag,
        "number" => Kind::Number,
        "choice" => Kind::Choice,
        "items" => Kind::Items,
        "level" => Kind::Level,
        "command" => Kind::Command,
        "range" => Kind::Range,
        "count" => Kind::Count,
        "space" => Kind::Space,
        "switch" => Kind::Switch,
        "word" => Kind::Word,
        _ => return None,
    })
}

/// The name of `k`, as [`kind`] reads it (`flag`, `range`, …). Kinds the markup crate adds
/// later are named the same way, from their Rust name.
pub fn kind_name(k: Kind) -> String {
    let mut out = String::new();
    for (i, c) in format!("{k:?}").chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Parameter names live as long as the program (the markup crate's specs hold
/// `&'static str`); each distinct name is kept once.
fn intern(name: &str) -> &'static str {
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    let set = names.get_or_insert_with(HashSet::new);
    if let Some(n) = set.get(name) {
        return n;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    set.insert(leaked);
    leaked
}

fn params(e: &ElementSpec) -> Result<Vec<Param>> {
    e.params
        .iter()
        .map(|p| {
            let Some(k) = kind(&p.kind) else {
                return Err(ToolError::new(format!(
                    "element `{}`: parameter `{}` has no kind `{}` (label, text, flag, number, choice, items, level, command, range, count, space, switch, word)",
                    e.name, p.name, p.kind
                )));
            };
            if p.name.is_empty() || !p.name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(ToolError::new(format!("element `{}`: `{}` is not a parameter name", e.name, p.name)));
            }
            Ok(Param { name: intern(&p.name), kind: k, required: p.required })
        })
        .collect()
}

fn source(input: &MarkupInput, ws: &Workspace) -> Result<(String, String)> {
    match (&input.source, &input.path) {
        (Some(s), _) => Ok((input.file.clone().unwrap_or_else(|| "screen.rui".into()), s.clone())),
        (None, Some(p)) => {
            let path = ws.resolve(p)?;
            let text = std::fs::read_to_string(&path)
                .map_err(|e| ToolError::new(format!("{}: {e}", path.display())))?;
            let file = input.file.clone().unwrap_or_else(|| {
                path.file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned())
            });
            Ok((file, text))
        }
        (None, None) => Err(ToolError::new("give the markup as `source` or a file as `path`")),
    }
}

/// Checks markup against the built-in elements and `input.elements`.
pub fn check(input: &MarkupInput, ws: &Workspace) -> Result<CheckOutput> {
    let (file, text) = source(input, ws)?;
    let mut specs: Vec<Spec> = spec::built_in();
    for e in &input.elements {
        let ps = params(e)?;
        specs.retain(|s| s.name != e.name);
        specs.push(Spec { name: e.name.clone(), params: ps, children: e.children });
    }
    let elements = specs.iter().map(|s| s.name.clone()).collect();
    let checked = Markup::parse(&file, &text).and_then(|m| spec::check(&m, &specs));
    Ok(match checked {
        Ok(uses) => CheckOutput {
            ok: true,
            error: None,
            signals: uses
                .signals
                .iter()
                .map(|(n, k)| SignalUse {
                    name: n.clone(),
                    kind: kind_name(*k),
                    rust_type: k.signal_type().unwrap_or("()").to_owned(),
                })
                .collect(),
            commands: uses.commands,
            elements,
        },
        Err(e) => CheckOutput {
            ok: false,
            error: Some(MarkupError::from(e)),
            signals: Vec::new(),
            commands: Vec::new(),
            elements,
        },
    })
}

/// Compiles markup to Rust, as `remedy_ui_widgets::rui::build` does in a build script.
pub fn compile(input: &MarkupInput, ws: &Workspace) -> Result<CompileOutput> {
    let (file, text) = source(input, ws)?;
    let mut customs = Vec::new();
    for e in &input.elements {
        let Some(path) = e.path.clone().filter(|p| !p.trim().is_empty()) else {
            return Err(ToolError::new(format!(
                "element `{}` needs `path` (its builder, such as crate::ui::{}) to compile",
                e.name, e.name
            )));
        };
        customs.push(Custom { name: e.name.clone(), params: params(e)?, children: e.children, path });
    }
    Ok(match remedy_ui_widgets::rui::codegen::compile(&file, &text, &customs) {
        Ok(rust) => CompileOutput { ok: true, error: None, rust: Some(rust) },
        Err(e) => CompileOutput { ok: false, error: Some(MarkupError::from(e)), rust: None },
    })
}

/// The built-in elements and what each takes, for reference.
pub fn built_in() -> Vec<ElementSpec> {
    spec::built_in()
        .into_iter()
        .map(|s| ElementSpec {
            name: s.name,
            params: s
                .params
                .iter()
                .map(|p| ParamSpec { name: p.name.to_owned(), kind: kind_name(p.kind), required: p.required })
                .collect(),
            children: s.children,
            path: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "column(gap: s3) {\n    switch(label: \"Noise\", checked: $noise)\n    button(label: \"Save\", on_press: !save)\n}\n";

    #[test]
    fn valid_markup_reports_what_it_binds() {
        let out =
            check(&MarkupInput { source: Some(GOOD.into()), ..Default::default() }, &Workspace::anywhere());
        let Ok(out) = out else { unreachable!("the input is readable") };
        assert!(out.ok, "{:?}", out.error);
        assert_eq!(
            out.signals,
            vec![SignalUse { name: "noise".into(), kind: "flag".into(), rust_type: "bool".into() }]
        );
        assert_eq!(out.commands, vec!["save".to_owned()]);
    }

    #[test]
    fn errors_name_the_line_and_column() {
        let bad = "column {\n    switch(label: \"Noise\", checked: 3)\n}\n";
        let out = check(
            &MarkupInput { source: Some(bad.into()), file: Some("voice.rui".into()), ..Default::default() },
            &Workspace::anywhere(),
        );
        let Ok(out) = out else { unreachable!("readable") };
        let Some(e) = out.error else { unreachable!("an error") };
        assert_eq!((e.file.as_str(), e.line), ("voice.rui", 2));
        assert!(e.column > 20, "{e}");
        assert!(e.message.contains("checked"), "{e}");
        let unknown = check(
            &MarkupInput { source: Some("fretboard(strings: 6)".into()), ..Default::default() },
            &Workspace::anywhere(),
        );
        assert!(unknown.is_ok_and(|o| o.error.is_some_and(|e| e.message.contains("fretboard"))));
    }

    #[test]
    fn app_elements_are_checked_and_compiled() {
        let fret = ElementSpec {
            name: "fretboard".into(),
            params: vec![ParamSpec { name: "strings".into(), kind: "count".into(), required: true }],
            children: false,
            path: Some("crate::ui::fretboard".into()),
        };
        let input = MarkupInput {
            source: Some("column { fretboard(strings: 6) }".into()),
            elements: vec![fret.clone()],
            ..Default::default()
        };
        assert!(check(&input, &Workspace::anywhere()).is_ok_and(|o| o.ok));
        let out = compile(&input, &Workspace::anywhere());
        let Ok(out) = out else { unreachable!("compiles") };
        assert!(out.rust.is_some_and(|r| r.contains("crate::ui::fretboard") && r.contains("pub fn build")));
        let no_path = MarkupInput { elements: vec![ElementSpec { path: None, ..fret }], ..input };
        assert!(compile(&no_path, &Workspace::anywhere()).is_err());
        let bad_kind = MarkupInput {
            elements: vec![ElementSpec {
                name: "x".into(),
                params: vec![ParamSpec { name: "a".into(), kind: "colour".into(), required: true }],
                children: false,
                path: None,
            }],
            ..Default::default()
        };
        assert!(
            check(&MarkupInput { source: Some("x(a: 1)".into()), ..bad_kind }, &Workspace::anywhere())
                .is_err()
        );
    }

    #[test]
    fn kinds_round_trip_by_name() {
        for name in [
            "label", "text", "flag", "number", "choice", "items", "level", "command", "range", "count",
            "space", "switch", "word",
        ] {
            assert_eq!(kind(name).map(kind_name).as_deref(), Some(name));
        }
    }

    #[test]
    fn interned_names_are_shared() {
        assert!(std::ptr::eq(intern("gain"), intern("gain")));
        assert!(built_in().iter().any(|e| e.name == "slider" && e.params.iter().any(|p| p.kind == "range")));
    }
}
