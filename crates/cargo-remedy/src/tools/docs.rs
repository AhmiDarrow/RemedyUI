//! `cargo remedy docs` and `cargo remedy widgets`.

use std::path::Path;

use cargo_remedy::devkit::docs::{CatalogInput, DocKind, Library, ReadInput, SearchInput, describe_widget};

use super::{Args, err};

fn library(a: &Args) -> Result<Library, String> {
    match a.get("--docs-root") {
        Some(root) => Library::from_checkout(Path::new(&root)).map_err(err),
        None => Library::embedded().map_err(err),
    }
}

/// `docs <query>`, `docs read <path>`, `docs list`.
pub fn docs(a: &Args) -> Result<(), String> {
    let lib = library(a)?;
    match a.pos(0) {
        Some("read") => {
            let path = a.pos(1).ok_or("usage: cargo remedy docs read <path> [--heading <h>]")?;
            let mut input = ReadInput::new(path);
            input.heading = a.get("--heading");
            input.from_line = a.number("--from")?;
            input.lines = a.number("--lines")?;
            let out = lib.read(&input).map_err(err)?;
            a.print(&out.text, &out)
        }
        Some("list") => {
            let out = lib.read(&ReadInput::default()).map_err(err)?;
            let paths: Vec<&str> = lib.docs().iter().map(|d| d.path.as_str()).collect();
            a.print(&out.text, &paths)
        }
        Some(_) => {
            let query = a.positional.join(" ");
            let kind = match a.get("--kind").as_deref() {
                None => None,
                Some("guide") => Some(DocKind::Guide),
                Some("rustdoc") => Some(DocKind::Rustdoc),
                Some(other) => return Err(format!("--kind takes guide or rustdoc, not `{other}`")),
            };
            let mut input = SearchInput::new(query);
            input.kind = kind;
            input.limit = a.number("--limit")?;
            let hits = lib.search(&input).map_err(err)?;
            let mut text = String::new();
            for h in &hits {
                text.push_str(&format!("{}:{} — {}\n{}\n\n", h.path, h.line, h.heading, h.excerpt));
            }
            if hits.is_empty() {
                text.push_str("nothing found\n");
            }
            a.print(&text, &hits)
        }
        None => Err("usage: cargo remedy docs <query> | docs read <path> | docs list".into()),
    }
}

/// `widgets [name] [--feature <f>]`.
pub fn widgets(a: &Args) -> Result<(), String> {
    let lib = library(a)?;
    let mut input = CatalogInput::default();
    input.name = a.pos(0).map(str::to_owned);
    input.feature = a.get("--feature");
    let ws = lib.widgets(&input);
    if ws.is_empty() {
        return Err("no widget matches".into());
    }
    let text: Vec<String> = ws.iter().map(describe_widget).collect();
    a.print(&text.join("\n"), &ws)
}
