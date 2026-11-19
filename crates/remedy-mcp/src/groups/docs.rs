//! docs_search, docs_read, widget_catalog.

use remedy_tools::docs::{CatalogInput, ReadInput, SearchInput, describe_widget};
use remedy_tools::{Group, ToolError};
use serde_json::Value;

use crate::{Context, Reply, ToolDef, ToolGroup, args};

pub(crate) struct Docs;

impl ToolGroup for Docs {
    fn group(&self) -> Group {
        Group::Docs
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::of::<SearchInput>(
                "docs_search",
                "Search RemedyUI's documentation: the guides, porting guides and recipes (remedy_ui::guide), the design documents (docs/), and the API docs of every crate (rustdoc, one page per source file). \
                 Returns the best-matching sections with their path, heading, line and an excerpt; read more with docs_read. \
                 Search before writing RemedyUI code: the engine's API is its own (signals, El builders, widgets taking &Ui), not another framework's.",
                true,
            ),
            ToolDef::of::<ReadInput>(
                "docs_read",
                "Read a RemedyUI document by the path docs_search returned (such as docs/DESIGN.md or rustdoc/remedy-ui-view/src/element.rs): \
                 all of it, the section under one heading, or a range of lines. An empty path lists every document.",
                true,
            ),
            ToolDef::of::<CatalogInput>(
                "widget_catalog",
                "List the widgets the remedy_ui facade offers (every public builder taking &Ui), each with its path (remedy_ui::widgets::button), \
                 the remedy-ui feature it needs, its signature, doc summary, the element names the theme's style sheet keys its rules by \
                 (and which of them the Remedy sheet styles), its style variants, an example call and its source line. \
                 Filter by name (part of it) or feature.",
                true,
            ),
        ]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        let lib = cx.library()?;
        match name {
            "docs_search" => {
                let input: SearchInput = args(a)?;
                let hits = lib.search(&input)?;
                let mut text = format!("{} hit(s) for {:?}:\n", hits.len(), input.query);
                for h in &hits {
                    text.push_str(&format!("\n{}:{} — {}\n{}\n", h.path, h.line, h.heading, h.excerpt));
                }
                Ok(Reply::new(text, &hits))
            }
            "docs_read" => {
                let input: ReadInput = args(a)?;
                let out = lib.read(&input)?;
                let text = if out.path.is_empty() {
                    out.text.clone()
                } else {
                    format!(
                        "{} (lines {}-{} of {})\n\n{}",
                        out.path,
                        out.from_line,
                        out.from_line + out.text.lines().count().saturating_sub(1),
                        out.total_lines,
                        out.text
                    )
                };
                Ok(Reply { text, structured: None, png: Vec::new() })
            }
            "widget_catalog" => {
                let input: CatalogInput = args(a)?;
                let ws = lib.widgets(&input);
                if ws.is_empty() {
                    let mut features: Vec<&str> =
                        lib.all_widgets().iter().map(|w| w.feature.as_str()).collect();
                    features.sort_unstable();
                    features.dedup();
                    return Err(ToolError::new(format!(
                        "no widget matches; features: {}",
                        features.join(", ")
                    )));
                }
                let mut text = format!("{} widget(s):\n", ws.len());
                for w in &ws {
                    text.push('\n');
                    text.push_str(&describe_widget(w));
                }
                Ok(Reply::new(text, &ws))
            }
            other => Err(ToolError::new(format!("no tool `{other}` in the docs group"))),
        }
    }
}
