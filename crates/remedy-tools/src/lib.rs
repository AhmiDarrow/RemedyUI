//! The logic behind RemedyUI's developer tools, shared by the `remedy-mcp` server (for AI
//! agents) and `cargo remedy`'s commands (for people and scripts). Each tool group is a
//! feature, `full` (the default) has them all, and every function takes typed input and
//! returns typed output, with no protocol or terminal code here:
//!
//! | Feature | Module | What it does |
//! |---|---|---|
//! | `docs` | [`docs`] | Search and read the guides, design docs and rustdoc; the widget catalog |
//! | `markup` | [`markup`] | Check `.rui` markup and compile it to Rust |
//! | `effects` | [`effects`] | Check Prism `.rfx` effects and what they cost; render a frame to PNG |
//! | `scaffold` | [`scaffold`] | Create an app from the app templates (`remedy-templates`) |
//! | `preview` | [`preview`] | Render an app's UI headlessly: a PNG, the accessibility tree, the audit |
//! | `testing` | [`testing`] | Run an app's tests and the engine's budget scenes, summarized |
//! | `live` | [`live`] | Operate a running app through its agents listener, under consent |
//! | `devtools` | [`devtools`] | Read a running app's remote devtools |
//!
//! Paths go through a [`Workspace`]: tools working for an agent stay inside the folders the
//! person named; the command line works anywhere.

#[cfg(feature = "devtools")]
pub mod devtools;
#[cfg(feature = "docs")]
pub mod docs;
#[cfg(feature = "effects")]
pub mod effects;
#[cfg(feature = "docs")]
mod extract;
#[cfg(any(feature = "live", feature = "devtools"))]
mod lines;
#[cfg(feature = "live")]
pub mod live;
#[cfg(feature = "markup")]
pub mod markup;
#[cfg(feature = "preview")]
pub mod preview;
#[cfg(feature = "scaffold")]
pub mod scaffold;
#[cfg(feature = "testing")]
pub mod testing;
// Builds with only some of the groups use only some of its helpers.
#[cfg(any(feature = "scaffold", feature = "preview", feature = "testing"))]
#[cfg_attr(not(all(feature = "scaffold", feature = "preview", feature = "testing")), allow(dead_code))]
mod process;
mod workspace;

pub use workspace::Workspace;

/// Why a tool could not do what was asked, in a sentence for whoever asked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolError(pub String);

impl ToolError {
    /// An error saying `message`.
    pub fn new(message: impl Into<String>) -> Self {
        ToolError(message.into())
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolError {}

/// A tool's result.
pub type Result<T> = std::result::Result<T, ToolError>;

/// A tool group: a feature of this crate and a set of tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Group {
    /// [`docs`].
    Docs,
    /// [`markup`].
    Markup,
    /// [`effects`].
    Effects,
    /// [`scaffold`].
    Scaffold,
    /// [`preview`].
    Preview,
    /// [`testing`].
    Testing,
    /// [`live`].
    Live,
    /// [`devtools`].
    Devtools,
}

impl Group {
    /// Every group, in order.
    pub const ALL: [Group; 8] = [
        Group::Docs,
        Group::Markup,
        Group::Effects,
        Group::Scaffold,
        Group::Preview,
        Group::Testing,
        Group::Live,
        Group::Devtools,
    ];

    /// Its name, which is also its feature's (`docs`).
    pub fn name(self) -> &'static str {
        match self {
            Group::Docs => "docs",
            Group::Markup => "markup",
            Group::Effects => "effects",
            Group::Scaffold => "scaffold",
            Group::Preview => "preview",
            Group::Testing => "testing",
            Group::Live => "live",
            Group::Devtools => "devtools",
        }
    }

    /// The group called `name`.
    pub fn from_name(name: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.name() == name.trim())
    }

    /// What it does, in a line.
    pub fn summary(self) -> &'static str {
        match self {
            Group::Docs => "search and read RemedyUI's guides, design docs and rustdoc; the widget catalog",
            Group::Markup => "check .rui markup and compile it to Rust",
            Group::Effects => {
                "check Prism .rfx effects (problems under their lines, cost pass by pass), render a frame to PNG, list the built-in effects"
            }
            Group::Scaffold => "create an app from the app templates (the same as cargo remedy new)",
            Group::Preview => "render an app's UI headlessly to a PNG with its accessibility tree and audit",
            Group::Testing => "run an app's tests and the engine's budget scenes",
            Group::Live => "operate a running app through its agents listener, with the person's consent",
            Group::Devtools => "read a running app's remote devtools: tree, elements, signals, timings",
        }
    }

    /// Whether this build has the group (its feature is on).
    pub fn built(self) -> bool {
        match self {
            Group::Docs => cfg!(feature = "docs"),
            Group::Markup => cfg!(feature = "markup"),
            Group::Effects => cfg!(feature = "effects"),
            Group::Scaffold => cfg!(feature = "scaffold"),
            Group::Preview => cfg!(feature = "preview"),
            Group::Testing => cfg!(feature = "testing"),
            Group::Live => cfg!(feature = "live"),
            Group::Devtools => cfg!(feature = "devtools"),
        }
    }

    /// The groups this build has.
    pub fn built_groups() -> Vec<Group> {
        Group::ALL.into_iter().filter(|g| g.built()).collect()
    }

    /// Parses a comma-separated list of group names (`docs,live`), refusing names that are
    /// unknown or not built into this binary.
    pub fn parse_list(list: &str) -> Result<Vec<Group>> {
        let mut out = Vec::new();
        for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let Some(g) = Group::from_name(name) else {
                let all: Vec<&str> = Group::ALL.iter().map(|g| g.name()).collect();
                return Err(ToolError::new(format!(
                    "no tool group `{name}`; the groups are {}",
                    all.join(", ")
                )));
            };
            if !g.built() {
                return Err(ToolError::new(format!(
                    "the `{name}` tools are not in this build (build with the `{name}` feature)"
                )));
            }
            if !out.contains(&g) {
                out.push(g);
            }
        }
        Ok(out)
    }
}

/// The RemedyUI checkout these tools were built from, when it is still there: the default
/// for reading docs from disk and for running the engine's budget scenes.
pub fn built_from_checkout() -> Option<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let root = std::fs::canonicalize(root).ok()?;
    root.join("crates").join("remedy-ui").join("Cargo.toml").is_file().then(|| workspace::plain(&root))
}
