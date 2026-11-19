//! Reads a RemedyUI checkout: its guides and design documents, the rustdoc comments of every
//! crate, and the widget catalog (every widget builder the `remedy-ui` facade offers).
//!
//! `cargo xtask docs-index` uses this to write the docs index committed in
//! `crates/remedy-tools/data` (which the tools embed, so a build anywhere has the docs);
//! `remedy_tools::docs::Library::from_checkout` uses it at run time for a live checkout. It needs
//! only the standard library, so xtask can include it as is.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A document found in a checkout: its path relative to the checkout (with `/`) and where it
/// is on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Relative path, `/`-separated (`docs/DESIGN.md`).
    pub path: String,
    /// The file.
    pub file: PathBuf,
}

/// Markdown documents: `README.md`, everything under `docs/`, and the guides, porting
/// guides and recipes in the crates' sources (`crates/remedy-ui/src/guide/`), sorted by path.
pub fn markdown(root: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let readme = root.join("README.md");
    if readme.is_file() {
        out.push(Found { path: "README.md".into(), file: readme });
    }
    let mut files = Vec::new();
    walk(&root.join("docs"), &mut files, &|p| p.extension().is_some_and(|e| e == "md"));
    // The guides, porting guides and recipes the crates' API docs include
    // (`crates/remedy-ui/src/guide/*.md`, `remedy_ui::guide`).
    for (_, dir) in crates(root) {
        walk(&dir.join("src"), &mut files, &|p| p.extension().is_some_and(|e| e == "md"));
    }
    for f in files {
        if let Some(rel) = relative(root, &f) {
            out.push(Found { path: rel, file: f });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Every crate directory under `crates/` that has a `Cargo.toml`, sorted by name.
pub fn crates(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(root.join("crates")) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Cargo.toml").is_file())
        .filter_map(|p| Some((p.file_name()?.to_str()?.to_owned(), p)))
        .collect();
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, keep: &dyn Fn(&Path) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target" || n == ".git") {
                continue;
            }
            walk(&p, out, keep);
        } else if keep(&p) {
            out.push(p);
        }
    }
}

/// `file` relative to `root`, `/`-separated.
pub fn relative(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    Some(parts.join("/"))
}

/// The Rust sources of a crate (`src/**/*.rs`), sorted.
pub fn sources(krate: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(&krate.join("src"), &mut files, &|p| p.extension().is_some_and(|e| e == "rs"));
    files
}

/// One rustdoc page: a source file's module docs and its documented public items, as
/// Markdown. `path` is `rustdoc/<crate>/<file relative to the crate>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// `rustdoc/remedy-ui-widgets-core/src/actions.rs`.
    pub path: String,
    /// The page.
    pub text: String,
}

/// The rustdoc pages of every engine crate in the checkout (files with no docs are left out).
pub fn rustdoc(root: &Path) -> Vec<Page> {
    let mut out = Vec::new();
    // The engine's crates; the tools' own (remedy-tools, remedy-mcp, cargo-remedy) are
    // documented in docs/MCP.md and docs/CLI.md.
    for (name, dir) in crates(root).into_iter().filter(|(n, _)| n.starts_with("remedy-ui")) {
        for file in sources(&dir) {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let Some(rel) = relative(&dir, &file) else { continue };
            let page = rustdoc_page(&name, &rel, &text);
            if !page.is_empty() {
                out.push(Page { path: format!("rustdoc/{name}/{rel}"), text: page });
            }
        }
    }
    out
}

/// The module path a source file is, from its path in the crate: `src/lib.rs` is the crate
/// root, `src/a/b.rs` and `src/a/b/mod.rs` are `a::b`.
pub fn module_path(rel: &str) -> String {
    let inner = rel.strip_prefix("src/").unwrap_or(rel).trim_end_matches(".rs");
    let parts: Vec<&str> = inner.split('/').filter(|p| !matches!(*p, "lib" | "main" | "mod")).collect();
    parts.join("::")
}

fn doc_line(t: &str, marker: &str) -> Option<String> {
    let rest = t.strip_prefix(marker)?;
    Some(rest.strip_prefix(' ').unwrap_or(rest).to_owned())
}

/// The Markdown page for one source file: its `//!` docs, then each documented public item
/// with its signature and docs. Empty when the file documents nothing public.
pub fn rustdoc_page(krate: &str, rel: &str, source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut module_docs = Vec::new();
    let mut items = Vec::new();
    let mut docs: Vec<String> = Vec::new();
    let mut context: Option<String> = None;
    let mut i = 0;
    while i < lines.len() {
        let raw = lines.get(i).copied().unwrap_or_default();
        let t = raw.trim_start();
        let indent = raw.len() - t.len();
        if let Some(d) = doc_line(t, "//!") {
            module_docs.push(d);
            i += 1;
            continue;
        }
        if let Some(d) = doc_line(t, "///") {
            docs.push(d);
            i += 1;
            continue;
        }
        if t.starts_with("#[") || t.starts_with("#!") || t.is_empty() && docs.is_empty() {
            i += 1;
            continue;
        }
        if indent == 0 && (t.starts_with("impl") || t.starts_with("unsafe impl")) {
            context = Some(signature(&lines, i));
        } else if indent == 0 && raw.starts_with('}') {
            context = None;
        } else if indent == 0
            && (t.starts_with("pub struct") || t.starts_with("pub enum") || t.starts_with("pub trait"))
        {
            context = Some(signature(&lines, i));
        }
        let public = t.starts_with("pub ") && !t.starts_with("pub use");
        if public && !docs.is_empty() {
            let sig = signature(&lines, i);
            let within = if indent > 0 { context.clone() } else { None };
            items.push((sig, within, std::mem::take(&mut docs)));
        } else {
            docs.clear();
        }
        i += 1;
    }
    if module_docs.is_empty() && items.is_empty() {
        return String::new();
    }
    let module = module_path(rel);
    let title = if module.is_empty() {
        krate.replace('-', "_")
    } else {
        format!("{}::{module}", krate.replace('-', "_"))
    };
    let mut out = format!("# {title}\n\nSource: `crates/{krate}/{rel}`\n\n");
    for d in &module_docs {
        out.push_str(d);
        out.push('\n');
    }
    for (sig, within, docs) in items {
        out.push_str(&format!("\n## `{sig}`\n\n"));
        if let Some(c) = within {
            out.push_str(&format!("In `{c}`.\n\n"));
        }
        for d in docs {
            out.push_str(&d);
            out.push('\n');
        }
    }
    out
}

/// The declaration starting at line `at`, up to its body or end, on one line.
fn signature(lines: &[&str], at: usize) -> String {
    let mut sig = String::new();
    let mut depth = 0i32;
    for line in lines.iter().skip(at).take(40) {
        let code = strip_line_comment(line.trim());
        for c in code.chars() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                '{' if depth <= 0 => return tidy(&sig),
                ';' if depth <= 0 => return tidy(&sig),
                _ => {}
            }
            sig.push(c);
        }
        let ends_field = depth <= 0 && code.ends_with(',');
        if ends_field {
            return tidy(sig.trim_end_matches(','));
        }
        sig.push(' ');
    }
    tidy(&sig)
}

fn strip_line_comment(s: &str) -> &str {
    let mut in_str = false;
    let mut prev = ' ';
    for (i, c) in s.char_indices() {
        if c == '"' && prev != '\\' {
            in_str = !in_str;
        }
        if !in_str && c == '/' && prev == '/' {
            return s.get(..i - 1).unwrap_or(s);
        }
        prev = c;
    }
    s
}

fn tidy(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() && !(out.ends_with('(') || w.starts_with(')') || w.starts_with(',')) {
            out.push(' ');
        }
        out.push_str(w);
    }
    out.replace("( ", "(").replace(" )", ")").replace(",)", ")")
}

/// One widget builder of the facade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Widget {
    /// Its name (`button`).
    pub name: String,
    /// Its path through the facade (`remedy_ui::widgets::button`).
    pub path: String,
    /// The facade feature that brings it (`widgets`).
    pub feature: String,
    /// The crate defining it.
    pub krate: String,
    /// Its source file, relative to the checkout.
    pub file: String,
    /// Its line there, from 1.
    pub line: usize,
    /// Its signature.
    pub signature: String,
    /// Its doc comment.
    pub docs: String,
    /// The element names it creates directly (`ui.el("button")`): the style sheet's rules
    /// are keyed by these names.
    pub elements: Vec<String>,
    /// Those of its element names the Remedy widget sheet styles.
    pub sheet_rules: Vec<String>,
    /// Style variants its docs name that the sheet styles (`primary`, `quiet`, …).
    pub variants: Vec<String>,
    /// An example call: from its docs when they have one, else a call with every argument
    /// shown by its type.
    pub example: String,
    /// The example came from its docs (rather than from its signature).
    pub example_from_docs: bool,
}

/// The facade's optional modules: (crate, facade module, feature), from
/// `crates/remedy-ui/src/lib.rs`.
pub fn facade_modules(root: &Path) -> Vec<(String, String, String)> {
    let Ok(text) = std::fs::read_to_string(root.join("crates/remedy-ui/src/lib.rs")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut feature = String::from("(always)");
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("#[cfg(feature = \"") {
            feature = rest.split('"').next().unwrap_or_default().to_owned();
            continue;
        }
        if let Some(rest) = t.strip_prefix("pub use ")
            && let Some((krate, alias)) = rest.trim_end_matches(';').split_once(" as ")
            && krate.starts_with("remedy_ui_")
        {
            out.push((krate.replace('_', "-"), alias.trim().to_owned(), feature.clone()));
        }
        if !t.starts_with("#[") && !t.starts_with("///") {
            feature = String::from("(always)");
        }
    }
    out
}

/// Every string literal in `source`.
fn literals(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = source;
    while let Some(start) = rest.find('"') {
        let after = rest.get(start + 1..).unwrap_or_default();
        let mut end = None;
        let mut prev = ' ';
        for (i, c) in after.char_indices() {
            if c == '"' && prev != '\\' {
                end = Some(i);
                break;
            }
            prev = if prev == '\\' && c == '\\' { ' ' } else { c };
        }
        let Some(end) = end else { break };
        let lit = after.get(..end).unwrap_or_default();
        if !lit.is_empty() && lit.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.insert(lit.to_owned());
        }
        rest = after.get(end + 1..).unwrap_or_default();
    }
    out
}

/// The code of `source` from byte `open` (a `{`) to its matching `}`, skipping strings,
/// characters and comments.
fn block(source: &str, open: usize) -> &str {
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'"' => {
                i += 1;
                while let Some(&c) = bytes.get(i) {
                    if c == b'\\' {
                        i += 1;
                    } else if c == b'"' {
                        break;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                // A character literal ('{', '\n'), or a lifetime ('a), which has no closing quote.
                if bytes.get(i + 2) == Some(&b'\'') {
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'\\') {
                    while let Some(&c) = bytes.get(i + 2) {
                        i += 1;
                        if c == b'\'' {
                            break;
                        }
                    }
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while let Some(&c) = bytes.get(i) {
                    if c == b'\n' {
                        break;
                    }
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return source.get(open..=i).unwrap_or_default();
                }
            }
            _ => {}
        }
        i += 1;
    }
    source.get(open..).unwrap_or_default()
}

/// The parameters of a signature, split at its top-level commas.
pub fn params(signature: &str) -> Vec<(String, String)> {
    let Some(open) = signature.find('(') else { return Vec::new() };
    let mut depth = 0i32;
    let mut current = String::new();
    let mut parts = Vec::new();
    for c in signature.get(open + 1..).unwrap_or_default().chars() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' if depth == 0 => break,
            ')' | ']' => depth -= 1,
            '>' if !current.ends_with('-') => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
        .into_iter()
        .filter_map(|p| {
            let (name, ty) = p.split_once(':')?;
            Some((name.trim().trim_start_matches("mut ").to_owned(), ty.trim().to_owned()))
        })
        .collect()
}

/// An argument expression for a parameter of type `ty`, for example calls.
fn argument(name: &str, ty: &str) -> String {
    let t = ty.replace(' ', "");
    // A property takes a constant, a signal or a closure: the example gives a constant.
    if let Some(inner) = t.strip_prefix("implIntoProp<").and_then(|r| r.strip_suffix('>')) {
        return argument(name, inner);
    }
    if t == "&Ui" {
        return "ui".into();
    }
    if t.contains("Into<String>") || t == "&str" || t == "String" || t.contains("Into<RichText>") {
        return format!("{:?}", title_case(name));
    }
    if t.starts_with("implFn") || t.starts_with("implFnMut") {
        let arity = t
            .split_once('(')
            .and_then(|(_, r)| r.split_once(')'))
            .map_or(0, |(args, _)| if args.is_empty() { 0 } else { args.split(',').count() });
        let ps: Vec<String> = (0..arity).map(|_| "_".to_owned()).collect();
        return format!("|{}| {{}}", ps.join(", "));
    }
    if let Some(inner) = t.strip_prefix("Signal<").and_then(|r| r.strip_suffix('>')) {
        let v = match inner {
            "bool" => "false".to_owned(),
            "String" => "String::new()".to_owned(),
            "f64" | "f32" => "0.0".to_owned(),
            "usize" | "u32" | "i32" | "u64" | "i64" => "0".to_owned(),
            other if other.starts_with("Option<") => "None".to_owned(),
            other if other.starts_with("Vec<") => "Vec::new()".to_owned(),
            _ => format!("{name} /* {inner} */"),
        };
        return format!("Signal::new({v})");
    }
    match t.as_str() {
        "bool" => "false".into(),
        "f32" | "f64" => "1.0".into(),
        "usize" | "u32" | "u8" | "u16" | "u64" | "i32" | "i64" => "1".into(),
        "El" => "w::text(ui, \"Content\")".into(),
        _ if t.starts_with("Option<") => "None".into(),
        _ if t.starts_with("Vec<") || t.starts_with("implIntoIterator") => "Vec::new()".into(),
        _ if t.starts_with("&[") => "&[]".into(),
        _ => format!("{name} /* {ty} */"),
    }
}

fn title_case(name: &str) -> String {
    let mut c = name.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().replace('_', " ")).unwrap_or_default()
}

/// The first fenced code block in `docs`.
fn doc_example(docs: &str) -> Option<String> {
    let mut lines = docs.lines();
    lines.find(|l| l.trim_start().starts_with("```"))?;
    let body: Vec<&str> = lines.take_while(|l| !l.trim_start().starts_with("```")).collect();
    let code: Vec<&str> = body.into_iter().filter(|l| !l.trim_start().starts_with("# ")).collect();
    if code.is_empty() { None } else { Some(code.join("\n")) }
}

/// The variant names a widget's docs mention as `.variant("a" | "b")` or `variant("a")`.
fn doc_variants(docs: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in docs.match_indices("variant(") {
        let rest = docs.get(at + 8..).unwrap_or_default();
        let inner = rest.split(')').next().unwrap_or_default();
        out.extend(literals(inner));
    }
    out.dedup();
    out
}

/// The names a crate's `lib.rs` makes public: re-exported names (`*` re-exports a whole
/// module), functions defined in it, and its public modules.
struct Exports {
    names: BTreeSet<String>,
    glob_modules: BTreeSet<String>,
    pub_modules: BTreeSet<String>,
}

fn exports(lib: &str) -> Exports {
    let mut e =
        Exports { names: BTreeSet::new(), glob_modules: BTreeSet::new(), pub_modules: BTreeSet::new() };
    let mut statement = String::new();
    let mut in_use = false;
    for line in lib.lines() {
        let t = line.trim();
        if let Some(m) = t.strip_prefix("pub mod ").and_then(|r| r.strip_suffix(';')) {
            e.pub_modules.insert(m.trim().to_owned());
        }
        if t.starts_with("pub use ") {
            in_use = true;
            statement.clear();
        }
        if in_use {
            statement.push_str(t);
            statement.push(' ');
            if t.ends_with(';') {
                in_use = false;
                let body = statement.trim_start_matches("pub use ").trim().trim_end_matches(';');
                if let Some(m) = body.strip_suffix("::*") {
                    e.glob_modules.insert(m.split("::").last().unwrap_or(m).to_owned());
                }
                for word in body.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    if !word.is_empty() {
                        e.names.insert(word.to_owned());
                    }
                }
            }
        }
    }
    e
}

/// Every widget builder the facade offers: public functions taking `&Ui`, reachable through
/// a facade module, with what styles them.
pub fn widgets(root: &Path) -> Vec<Widget> {
    let sheet =
        std::fs::read_to_string(root.join("crates/remedy-ui-themes/src/sheet.rs")).unwrap_or_default();
    let styled = literals(&sheet);
    let mut out = Vec::new();
    for (krate, module, feature) in facade_modules(root) {
        let dir = root.join("crates").join(&krate);
        let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap_or_default();
        let ex = exports(&lib);
        for file in sources(&dir) {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let Some(rel) = relative(&dir, &file) else { continue };
            let module_path = module_path(&rel);
            let top_module = module_path.split("::").next().unwrap_or_default().to_owned();
            let path_of = |name: &str| -> Option<String> {
                if module_path.is_empty() {
                    Some(format!("remedy_ui::{module}::{name}"))
                } else if ex.pub_modules.contains(&top_module) {
                    Some(format!("remedy_ui::{module}::{module_path}::{name}"))
                } else if ex.names.contains(name) || ex.glob_modules.contains(&top_module) {
                    Some(format!("remedy_ui::{module}::{name}"))
                } else {
                    None
                }
            };
            let lines: Vec<&str> = text.lines().collect();
            let mut offset = 0usize;
            let mut docs: Vec<String> = Vec::new();
            let (mut in_attr, mut deprecated) = (false, false);
            for (n, line) in lines.iter().enumerate() {
                let line_start = offset;
                offset += line.len() + 1;
                if let Some(d) = doc_line(line.trim_start(), "///").filter(|_| line.starts_with("///")) {
                    docs.push(d);
                    continue;
                }
                // Attributes, which may span lines; deprecated builders are left out of the
                // catalog (their replacements are in it).
                if line.starts_with("#[") {
                    deprecated |= line.starts_with("#[deprecated");
                    in_attr = !line.trim_end().ends_with(']');
                    continue;
                }
                if in_attr {
                    in_attr = !line.trim_end().ends_with(']');
                    continue;
                }
                let Some(rest) = line.strip_prefix("pub fn ") else {
                    docs.clear();
                    deprecated = false;
                    continue;
                };
                let doc_text = std::mem::take(&mut docs).join("\n");
                if std::mem::take(&mut deprecated) {
                    continue;
                }
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                let sig = signature(&lines, n);
                let ps = params(&sig);
                if !ps.iter().any(|(_, t)| t.replace(' ', "") == "&Ui") {
                    continue;
                }
                let Some(path) = path_of(&name) else { continue };
                let open = text.get(line_start..).and_then(|s| s.find('{')).map(|i| i + line_start);
                let body = open.map(|o| block(&text, o)).unwrap_or_default();
                let mut elements = Vec::new();
                for (at, _) in body.match_indices("el(\"") {
                    let lit = body.get(at + 4..).and_then(|r| r.split('"').next()).unwrap_or_default();
                    if !lit.is_empty() && !elements.iter().any(|e| e == lit) {
                        elements.push(lit.to_owned());
                    }
                }
                let sheet_rules: Vec<String> =
                    elements.iter().filter(|e| styled.contains(*e)).cloned().collect();
                let variants: Vec<String> =
                    doc_variants(&doc_text).into_iter().filter(|v| styled.contains(v)).collect();
                let (example, from_docs) = match doc_example(&doc_text) {
                    Some(e) => (e, true),
                    None => {
                        let args: Vec<String> = ps.iter().map(|(n, t)| argument(n, t)).collect();
                        (format!("{path}({})", args.join(", ")), false)
                    }
                };
                out.push(Widget {
                    name,
                    path,
                    feature: feature.clone(),
                    krate: krate.clone(),
                    file: format!("crates/{krate}/{rel}"),
                    line: n + 1,
                    signature: sig,
                    docs: doc_text,
                    elements,
                    sheet_rules,
                    variants,
                    example,
                    example_from_docs: from_docs,
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// `s` as a JSON string literal.
#[allow(dead_code, reason = "xtask writes the docs index as JSON")]
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The widget catalog as JSON (an array of objects with [`Widget`]'s fields).
#[allow(dead_code, reason = "xtask writes the docs index as JSON")]
pub fn widgets_json(ws: &[Widget]) -> String {
    let list = |v: &[String]| format!("[{}]", v.iter().map(|s| json_string(s)).collect::<Vec<_>>().join(","));
    let items: Vec<String> = ws
        .iter()
        .map(|w| {
            format!(
                "{{\"name\":{},\"path\":{},\"feature\":{},\"krate\":{},\"file\":{},\"line\":{},\"signature\":{},\"docs\":{},\"elements\":{},\"sheet_rules\":{},\"variants\":{},\"example\":{},\"example_from_docs\":{}}}",
                json_string(&w.name),
                json_string(&w.path),
                json_string(&w.feature),
                json_string(&w.krate),
                json_string(&w.file),
                w.line,
                json_string(&w.signature),
                json_string(&w.docs),
                list(&w.elements),
                list(&w.sheet_rules),
                list(&w.variants),
                json_string(&w.example),
                w.example_from_docs
            )
        })
        .collect();
    format!("[{}]", items.join(",\n"))
}

/// The docs index of the checkout at `root`, as JSON: `{"version": 1, "markdown": [[path,
/// text], ...], "rustdoc": [[path, text], ...], "widgets": [...]}`, with line endings
/// normalized, so the same checkout always gives the same index.
#[allow(dead_code, reason = "xtask writes the docs index as JSON")]
pub fn index_json(root: &Path) -> String {
    let pair = |path: &str, text: &str| {
        format!("[{},{}]", json_string(path), json_string(&text.replace("\r\n", "\n")))
    };
    let markdown: Vec<String> = markdown(root)
        .iter()
        .map(|f| pair(&f.path, &std::fs::read_to_string(&f.file).unwrap_or_default()))
        .collect();
    let pages: Vec<String> = rustdoc(root).iter().map(|p| pair(&p.path, &p.text)).collect();
    let widgets = widgets(root)
        .into_iter()
        .map(|mut w| {
            w.docs = w.docs.replace("\r\n", "\n");
            w.example = w.example.replace("\r\n", "\n");
            w
        })
        .collect::<Vec<_>>();
    format!(
        "{{\"version\":1,\n\"markdown\":[{}],\n\"rustdoc\":[{}],\n\"widgets\":{}}}\n",
        markdown.join(",\n"),
        pages.join(",\n"),
        widgets_json(&widgets)
    )
}
