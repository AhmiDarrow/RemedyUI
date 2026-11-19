//! new_app.

use remedy_tools::scaffold::{NewAppInput, new_app};
use remedy_tools::{Group, ToolError};
use serde_json::Value;

use crate::{Context, Reply, ToolDef, ToolGroup, args};

pub(crate) struct Scaffold;

impl ToolGroup for Scaffold {
    fn group(&self) -> Group {
        Group::Scaffold
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![ToolDef::of::<NewAppInput>(
            "new_app",
            "Create a RemedyUI app from the app templates (the same as `cargo remedy new`) in a new folder inside the workspace: Cargo.toml, the app as a library (src/lib.rs with a `home(ui) -> El` screen) and a main, \
             Fluent strings in i18n/en-US.ftl, a headless test and a golden-image preview test, remedy.toml for packaging, and an icon. \
             Templates: basic, tray, multi-window, document (a paged document viewer), android (Android and the desktop). Returns the files created.",
            false,
        )]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        if name != "new_app" {
            return Err(ToolError::new(format!("no tool `{name}` in the scaffold group")));
        }
        let input: NewAppInput = args(a)?;
        let out = new_app(&input, &cx.config.workspace)?;
        let text = format!("{}\n\nfiles:\n  {}", out.message, out.files.join("\n  "));
        Ok(Reply::new(text, &out))
    }
}
