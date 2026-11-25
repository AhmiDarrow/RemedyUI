//! app_connect, app_snapshot, app_act, app_command, app_undo, app_wait, app_disconnect.

use crate::devkit::live::{ActInput, CommandInput, ConnectInput, ConnectionInput, Outcome, WaitInput};
use crate::devkit::{Group, ToolError};
use serde_json::Value;

use crate::mcp::{Context, Reply, ToolDef, ToolGroup, args, lock};

pub(crate) struct Live;

fn outcome_text(o: &Outcome) -> String {
    let mut t = o.message.clone();
    for e in &o.earlier {
        t.push_str(&format!("\nearlier: {e}"));
    }
    t
}

impl ToolGroup for Live {
    fn group(&self) -> Group {
        Group::Live
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::of::<ConnectInput>(
                "app_connect",
                "Connect to a running RemedyUI app whose person granted you access: they give you the address its agents listener opened (127.0.0.1:PORT) and the token of their grant. \
                 The grant's scope is theirs: read, act, or act with their confirmation of each action. Returns the first snapshot. \
                 Only loopback addresses are accepted; a wrong, revoked or expired token is refused by the app.",
                false,
            ),
            ToolDef::of::<ConnectionInput>(
                "app_snapshot",
                "What the connected app's window shows and offers now: its elements as a tree (role, name, value, checked/selected/expanded, disabled, the actions each offers, and its id), \
                 the app's commands (with those that ask the person first), and what Undo would take back. Secrets, locked content and the person's own prompts are never shown.",
                true,
            ),
            ToolDef::of::<ActInput>(
                "app_act",
                "Act on an element of the connected app by its id from app_snapshot: click, focus, increment, decrement, expand, collapse, scroll_into_view, or set_value (with value). \
                 Goes through the same code paths as the person's input, recorded in the window's undo history under your name. \
                 With a confirming grant the app asks the person first: the answer is then 'asking' and this waits up to wait_seconds for their decision.",
                false,
            ),
            ToolDef::of::<CommandInput>(
                "app_command",
                "Run one of the connected app's commands by id (from app_snapshot). Commands marked destructive, and every command under a confirming grant, ask the person first; \
                 this waits up to wait_seconds for their answer.",
                false,
            ),
            ToolDef::of::<ConnectionInput>(
                "app_undo",
                "Undo the latest change in the connected app's window (as the person's Undo would).",
                false,
            ),
            ToolDef::of::<WaitInput>(
                "app_wait",
                "Wait for the person's answer to an earlier action or command that is 'asking'. Returns their decision (done, or declined), or 'asking' again if they have not answered yet.",
                true,
            ),
            ToolDef::of::<ConnectionInput>(
                "app_disconnect",
                "Close the connection to an app. (The person can also end your access at any time from the app's indicator.)",
                false,
            ),
        ]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        let mut live = lock(&cx.session.live);
        match name {
            "app_connect" => {
                let c = live.connect(&args(a)?)?;
                let text =
                    format!("connected to {} as `{}`\n\n{}", c.address, c.connection, c.snapshot.text());
                Ok(Reply::new(text, &c))
            }
            "app_snapshot" => {
                let s = live.snapshot(&args(a)?)?;
                Ok(Reply::new(s.text(), &s))
            }
            "app_act" => {
                let o = live.act(&args(a)?)?;
                Ok(Reply::new(outcome_text(&o), &o))
            }
            "app_command" => {
                let o = live.command(&args(a)?)?;
                Ok(Reply::new(outcome_text(&o), &o))
            }
            "app_undo" => {
                let o = live.undo(&args(a)?)?;
                Ok(Reply::new(outcome_text(&o), &o))
            }
            "app_wait" => {
                let o = live.wait(&args(a)?)?;
                Ok(Reply::new(outcome_text(&o), &o))
            }
            "app_disconnect" => Ok(Reply::text(live.disconnect(&args(a)?)?)),
            other => Err(ToolError::new(format!("no tool `{other}` in the live group"))),
        }
    }
}
