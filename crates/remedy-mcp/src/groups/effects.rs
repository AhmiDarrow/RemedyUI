//! rfx_check, rfx_render, rfx_library.

use remedy_tools::effects::{EffectInput, RenderInput, check, check_text, library, render};
use remedy_tools::{Group, ToolError};
use serde_json::Value;

use super::Nothing;
use crate::{Context, Reply, ToolDef, ToolGroup, args};

pub(crate) struct Effects;

impl ToolGroup for Effects {
    fn group(&self) -> Group {
        Group::Effects
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::of::<EffectInput>(
                "rfx_check",
                "Check a Prism .rfx effect (RemedyUI's language for animated, procedural pictures: fire, water, glow, particles, raymarched 3D) \
                 given as `source`, a file `path` or a built-in `name`, against the limits an app's own effects get (or, with `untrusted`, a theme's or script's). \
                 Returns ok with its parameters and what each pass costs, or every problem drawn under its line with a caret (misspelled names get a suggestion). \
                 Reference: docs/effects/rfx.md.",
                true,
            ),
            ToolDef::of::<RenderInput>(
                "rfx_render",
                "Render a frame of a Prism effect on the CPU (the reference renderer the GPU kernel is tested against) and return it as a PNG. \
                 Set the element's `width` and `height` (logical pixels), `time`, parameter `values` by name, and the `background` it is drawn over. \
                 Look at the image to judge an effect you are writing.",
                true,
            ),
            ToolDef::of::<Nothing>(
                "rfx_library",
                "List RemedyUI's built-in Prism effects: name, what it is, its parameters and its cost. Any of them can be used by name (library(\"fire\")) or read as an example.",
                true,
            ),
        ]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        let ws = &cx.config.workspace;
        match name {
            "rfx_check" => {
                let input: EffectInput = args(a)?;
                let out = check(&input, ws)?;
                Ok(Reply::new(check_text(&out), &out))
            }
            "rfx_render" => {
                let input: RenderInput = args(a)?;
                let out = render(&input, ws, None)?;
                let mut text = format!(
                    "{} x {} pixels at t = {} s, {:.2} ms on the CPU",
                    out.width, out.height, out.time, out.cpu_ms
                );
                if !out.ignored.is_empty() {
                    text.push_str(&format!(
                        "\nignored values (no such parameter, or another type): {}",
                        out.ignored.join(", ")
                    ));
                }
                let mut reply = Reply::new(text, &out);
                reply.png.push(out.png.clone());
                Ok(reply)
            }
            "rfx_library" => {
                let lib = library();
                let text: String = lib
                    .iter()
                    .map(|e| {
                        format!("{} (cost {}): {} [{}]\n", e.name, e.cost, e.summary, e.params.join(", "))
                    })
                    .collect();
                Ok(Reply::new(text, &lib))
            }
            other => Err(ToolError::new(format!("no tool `{other}` in the effects group"))),
        }
    }
}
