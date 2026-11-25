//! New apps from RemedyUI's app templates (`remedy-templates`, the same ones `cargo remedy
//! new` uses), written into a folder inside the workspace.

use crate::templates::{Dependency, TEMPLATES, title};
use serde::{Deserialize, Serialize};

use crate::devkit::{Result, ToolError, Workspace};

/// The app to create.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct NewAppInput {
    /// The package name (letters, digits, `-` and `_`, starting with a letter): `my-notes`.
    pub name: String,
    /// The folder to create it in, inside the workspace (a new or empty folder). Defaults to
    /// a folder named after the app in the workspace's first folder.
    #[serde(default)]
    pub folder: Option<String>,
    /// The template: `basic` (one window), `tray` (lives in the tray, one instance),
    /// `multi-window` (a main window opening document windows), `document` (a paged document
    /// viewer) or `android` (one app for Android and the desktop). `basic` when absent.
    #[serde(default)]
    pub template: Option<String>,
    /// A local RemedyUI checkout for the app to depend on (by path) instead of the git
    /// repository.
    #[serde(default)]
    pub remedy_path: Option<String>,
}

impl NewAppInput {
    /// An app called `name`, from the basic template, in a folder named after it.
    pub fn new(name: impl Into<String>) -> Self {
        NewAppInput { name: name.into(), ..Default::default() }
    }
}

/// What was created.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct NewAppOutput {
    /// The app's folder.
    pub folder: String,
    /// The template used.
    pub template: String,
    /// Its files, relative to the folder.
    pub files: Vec<String>,
    /// What to do next.
    pub message: String,
}

/// The templates and what each makes.
pub fn templates() -> Vec<(String, String)> {
    TEMPLATES.iter().map(|(t, d)| ((*t).to_owned(), (*d).to_owned())).collect()
}

/// Creates the app.
pub fn new_app(input: &NewAppInput, ws: &Workspace) -> Result<NewAppOutput> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ToolError::new("the app needs a name"));
    }
    let folder = ws.resolve(input.folder.as_deref().unwrap_or(name))?;
    let template = input.template.clone().unwrap_or_else(|| "basic".into());
    let remedy = match &input.remedy_path {
        Some(p) => {
            let root =
                std::fs::canonicalize(p).map_err(|e| ToolError::new(format!("remedy_path {p}: {e}")))?;
            if !root.join("crates").join("remedy-ui").join("Cargo.toml").is_file() {
                return Err(ToolError::new(format!("remedy_path {p} is not a RemedyUI checkout")));
            }
            Dependency::Path(root)
        }
        None => Dependency::Git(crate::templates::REPOSITORY.to_owned()),
    };
    let files = crate::templates::create(&folder, name, &template, &remedy)
        .map_err(|e| ToolError::new(e.to_string()))?;
    let shown = folder.display().to_string();
    let message = format!(
        "Created {} ({template}) in {shown}. Next: `cargo test` there runs its headless test and writes its preview's golden images (commit them); `cargo remedy run` runs it.",
        title(name)
    );
    Ok(NewAppOutput {
        folder: shown,
        template,
        files: files.iter().map(|f| f.to_string_lossy().replace('\\', "/")).collect(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_made_inside_the_workspace_from_every_template() {
        let base = std::env::temp_dir().join(format!("remedy-tools-scaffold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::create_dir_all(&base);
        let Ok(ws) = Workspace::new([base.clone()]) else { unreachable!("the folder exists") };
        for (t, _) in templates() {
            let mut input = NewAppInput::new("my-notes");
            input.folder = Some(format!("apps/{t}"));
            input.template = Some(t.clone());
            let Ok(out) = new_app(&input, &ws) else { unreachable!("{t} makes an app") };
            assert!(
                out.files.iter().any(|f| f == "src/lib.rs") && out.files.iter().any(|f| f == "Cargo.toml"),
                "{t}"
            );
            assert!(base.join("apps").join(&t).join("i18n").join("en-US.ftl").is_file());
        }
        let mut again = NewAppInput::new("my-notes");
        again.folder = Some("apps/basic".into());
        assert!(new_app(&again, &ws).is_err_and(|e| e.0.contains("not empty")));
        let mut outside = NewAppInput::new("x");
        outside.folder = Some("../escape".into());
        assert!(new_app(&outside, &ws).is_err_and(|e| e.0.contains("outside the workspace")));
        assert!(new_app(&NewAppInput::new("9lives"), &ws).is_err_and(|e| e.0.contains("not a package name")));
        let mut nope = NewAppInput::new("x");
        nope.template = Some("nope".into());
        assert!(new_app(&nope, &ws).is_err_and(|e| e.0.contains("choose one of")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
