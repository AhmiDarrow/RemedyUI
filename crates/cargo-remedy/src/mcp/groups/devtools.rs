//! devtools_connect, devtools_tree, devtools_element, devtools_semantics, devtools_signals,
//! devtools_updates, devtools_timings, devtools_disconnect.

use crate::devkit::devtools::{
    Devtools as Client, DevtoolsConnectInput, DevtoolsOutput, ElementInput, SignalsInput,
};
use crate::devkit::{Group, ToolError};
use serde_json::Value;

use super::Nothing;
use crate::mcp::{Context, Reply, ToolDef, ToolGroup, args, lock};

pub(crate) struct Devtools;

fn reply(out: DevtoolsOutput) -> Reply {
    Reply::new(out.text.clone(), &out.answer)
}

impl ToolGroup for Devtools {
    fn group(&self) -> Group {
        Group::Devtools
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::of::<DevtoolsConnectInput>(
                "devtools_connect",
                "Connect to a running RemedyUI app's remote devtools (debug builds; started by Window::serve_devtools or RemoteServer, which print the loopback address and token). \
                 Replaces an earlier devtools connection.",
                false,
            ),
            ToolDef::of::<Nothing>(
                "devtools_tree",
                "The connected window's element tree, depth first: each element's widget and description, id and box (x, y, width, height in logical pixels), hidden ones marked.",
                true,
            ),
            ToolDef::of::<ElementInput>(
                "devtools_element",
                "One element's details as the inspector shows them: widget, the source line that built it, box and constraints, resolved style, states, semantics and bindings.",
                true,
            ),
            ToolDef::of::<Nothing>(
                "devtools_semantics",
                "The connected window as a screen reader meets it, in reading order, with each node's accessibility problems, and every problem found (unreachable controls included).",
                true,
            ),
            ToolDef::of::<SignalsInput>(
                "devtools_signals",
                "With an element id: its bindings, the signals and memos they read, and what else depends on those. Without: every node of the window thread's reactive graph.",
                true,
            ),
            ToolDef::of::<Nothing>(
                "devtools_updates",
                "Recent \"why did this update\" entries: which element was marked for work, by which binding, because of which signal change (and where it was written).",
                true,
            ),
            ToolDef::of::<Nothing>(
                "devtools_timings",
                "Recent frames' timings: p50, p95 and worst frame time, and the last frame's time per phase (events, binds, style, layout, paint, layers, semantics, composite, raster, present).",
                true,
            ),
            ToolDef::of::<Nothing>("devtools_disconnect", "Close the devtools connection to the app.", false),
        ]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        let mut slot = lock(&cx.session.devtools);
        if name == "devtools_connect" {
            let input: DevtoolsConnectInput = args(a)?;
            let client = Client::connect(&input)?;
            let text =
                format!("connected to the devtools of {} (protocol {})", client.address(), client.version());
            *slot = Some(client);
            return Ok(Reply::text(text));
        }
        if name == "devtools_disconnect" {
            return match slot.take() {
                Some(c) => Ok(Reply::text(format!("disconnected from {}", c.address()))),
                None => Err(ToolError::new("not connected to devtools")),
            };
        }
        let Some(client) = slot.as_mut() else {
            return Err(ToolError::new(
                "not connected: devtools_connect with the address and token the app printed",
            ));
        };
        let out = match name {
            "devtools_tree" => client.tree(),
            "devtools_element" => client.element(&args(a)?),
            "devtools_semantics" => client.semantics(),
            "devtools_signals" => client.signals(&args::<SignalsInput>(a)?),
            "devtools_updates" => client.updates(),
            "devtools_timings" => client.timings(),
            other => return Err(ToolError::new(format!("no tool `{other}` in the devtools group"))),
        };
        // A connection the app closed is no use; the next call says to connect again.
        if out.as_ref().is_err_and(|e| e.0.contains("closed the connection")) {
            *slot = None;
        }
        out.map(reply)
    }
}
