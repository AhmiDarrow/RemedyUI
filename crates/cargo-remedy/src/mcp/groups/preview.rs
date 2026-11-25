//! render_preview, list_previews, a11y_audit.

use crate::devkit::preview::{ListInput, PreviewInput, list, render};
use crate::devkit::{Group, ToolError};
use serde::Deserialize;
use serde_json::Value;

use crate::mcp::{Context, Reply, ToolDef, ToolGroup, args};

pub(crate) struct Preview;

/// A preview to render, and where to save it.
#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct RenderInput {
    #[serde(flatten)]
    preview: PreviewInput,
    /// Also save the PNG here (inside the workspace), such as `previews/home-dark.png`.
    #[serde(default)]
    save_png: Option<String>,
}

impl ToolGroup for Preview {
    fn group(&self) -> Group {
        Group::Preview
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::of::<RenderInput>(
                "render_preview",
                "Render part of a RemedyUI app headlessly and return it as a PNG image, with the window as a screen reader meets it and the accessibility problems found. \
                 `function` is a public `fn(&Ui) -> El` in the app's library (the templates' `home`). Options: size, scale, theme (dark, light or a palette id), \
                 locale (right-to-left ones such as ar-EG mirror; en-XA and ar-XB are pseudo-locales for long and right-to-left text), text scale. \
                 The first render of an app builds a small harness against its dependencies (minutes); later ones take seconds. \
                 Rendered by the CPU renderer, exactly like the app's golden-image tests.",
                true,
            ),
            ToolDef::of::<ListInput>(
                "list_previews",
                "List the previews an app declares with #[preview] (functions `fn(&Ui) -> El` with the themes, locales and sizes to show them in): \
                 each one's path for render_preview, whether it is public (render_preview calls public ones by path), its attribute and source line.",
                true,
            ),
            ToolDef::of::<PreviewInput>(
                "a11y_audit",
                "Audit part of a RemedyUI app for accessibility, rendered headlessly as render_preview does: controls without names, text below WCAG AA contrast, \
                 targets smaller than 24x24, pointer-only controls, focus that cannot be seen, content past the window's edge, and controls a screen reader cannot reach, \
                 each with the source line that built the element. Run it in every theme and locale the app ships.",
                true,
            ),
        ]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        let ws = &cx.config.workspace;
        match name {
            "render_preview" => {
                let input: RenderInput = args(a)?;
                let out = render(&input.preview, ws, input.save_png.as_deref())?;
                let mut text = format!(
                    "{}×{} px, theme {} ({}), locale {}{}\n",
                    out.width,
                    out.height,
                    out.theme,
                    if out.dark { "dark" } else { "light" },
                    out.locale,
                    if out.rtl { ", right to left" } else { "" }
                );
                if let Some(f) = &out.png_file {
                    text.push_str(&format!("saved to {f}\n"));
                }
                text.push_str("\nscreen reader view:\n");
                text.push_str(&out.reader_text());
                text.push_str(&format!("\n{} accessibility problem(s)", out.problems.len()));
                for p in &out.problems {
                    text.push_str(&format!("\n  #{}: {}", p.element, p.message));
                }
                let png = out.png.clone();
                let mut reply = Reply::new(text, &out);
                reply.png.push(png);
                Ok(reply)
            }
            "list_previews" => {
                let found = list(&args(a)?, ws)?;
                let mut text = format!("{} preview(s)\n", found.len());
                for d in &found {
                    let reach = if d.public { "" } else { " (private: make it pub to render it here)" };
                    text.push_str(&format!(
                        "{}{reach}\n  {} at {}:{}\n",
                        d.function, d.attribute, d.file, d.line
                    ));
                }
                Ok(Reply::new(text, &found))
            }
            "a11y_audit" => {
                let input: PreviewInput = args(a)?;
                let out = render(&input, ws, None)?;
                let mut text = if out.problems.is_empty() {
                    format!(
                        "no problems found ({} {}, {})",
                        out.theme,
                        out.locale,
                        format_args!("{}×{}", out.width, out.height)
                    )
                } else {
                    format!("{} problem(s):", out.problems.len())
                };
                for p in &out.problems {
                    text.push_str(&format!("\n  #{}: {}", p.element, p.message));
                }
                Ok(Reply::new(
                    text,
                    &serde_json::json!({ "problems": out.problems, "theme": out.theme, "locale": out.locale }),
                ))
            }
            other => Err(ToolError::new(format!("no tool `{other}` in the preview group"))),
        }
    }
}
