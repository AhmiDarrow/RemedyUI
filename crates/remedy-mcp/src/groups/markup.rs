//! rui_check, rui_compile, rui_elements.

use remedy_tools::markup::{MarkupInput, built_in, check, compile};
use remedy_tools::{Group, ToolError};
use serde_json::Value;

use super::Nothing;
use crate::{Context, Reply, ToolDef, ToolGroup, args};

pub(crate) struct Markup;

impl ToolGroup for Markup {
    fn group(&self) -> Group {
        Group::Markup
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::of::<MarkupInput>(
                "rui_check",
                "Check RemedyUI .rui markup (screens as data: elements with arguments and children, $signals, !commands, @t(\"…\") translations) \
                 against the built-in elements and the app's own (pass them in `elements`). \
                 Returns ok, or the first problem at file:line:column with what would be right; and the signals (with their Rust types) and commands the app must expose.",
                true,
            ),
            ToolDef::of::<MarkupInput>(
                "rui_compile",
                "Compile .rui markup to the Rust that remedy_ui_rui::build generates in a build script: a Bindings struct with a field per signal and command, and build(&ui, &bindings, &translate). \
                 App elements need their builder's path. Use it to see exactly what the app must provide.",
                true,
            ),
            ToolDef::of::<Nothing>(
                "rui_elements",
                "List the built-in .rui elements and what each takes (argument names, kinds, required or not, whether it holds children). \
                 Every element also takes gap, padding, grow, fill_width, variant, align, justify and visible.",
                true,
            ),
        ]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        let ws = &cx.config.workspace;
        match name {
            "rui_check" => {
                let input: MarkupInput = args(a)?;
                let out = check(&input, ws)?;
                let text = match &out.error {
                    Some(e) => format!("error: {e}"),
                    None => {
                        let mut t = String::from("ok");
                        for s in &out.signals {
                            t.push_str(&format!(
                                "\nsignal ${}: {} (Signal<{}>)",
                                s.name, s.kind, s.rust_type
                            ));
                        }
                        for c in &out.commands {
                            t.push_str(&format!("\ncommand !{c}"));
                        }
                        t
                    }
                };
                Ok(Reply::new(text, &out))
            }
            "rui_compile" => {
                let input: MarkupInput = args(a)?;
                let out = compile(&input, ws)?;
                let text = match (&out.error, &out.rust) {
                    (Some(e), _) => format!("error: {e}"),
                    (None, Some(r)) => r.clone(),
                    (None, None) => "nothing was generated".to_owned(),
                };
                Ok(Reply::new(text, &out))
            }
            "rui_elements" => {
                let els = built_in();
                let mut text = String::new();
                for e in &els {
                    let ps: Vec<String> = e
                        .params
                        .iter()
                        .map(|p| format!("{}: {}{}", p.name, p.kind, if p.required { "" } else { "?" }))
                        .collect();
                    let kids = if e.children { " { children }" } else { "" };
                    text.push_str(&format!("{}({}){kids}\n", e.name, ps.join(", ")));
                }
                Ok(Reply::new(text, &els))
            }
            other => Err(ToolError::new(format!("no tool `{other}` in the markup group"))),
        }
    }
}
