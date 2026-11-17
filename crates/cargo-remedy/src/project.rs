//! What `run` and `package` need to know about the app in the current directory.

use std::path::{Path, PathBuf};

/// The app: its Cargo package and its `remedy.toml`.
#[derive(Clone, Debug)]
pub struct Project {
    /// The directory holding Cargo.toml.
    pub root: PathBuf,
    /// The executable's name.
    pub bin: String,
    /// The package version.
    pub version: String,
    /// Where Cargo puts builds.
    pub target_dir: PathBuf,
    /// The shown name.
    pub name: String,
    /// A reverse-DNS id.
    pub id: String,
    /// The publisher.
    pub publisher: String,
    /// One line about the app.
    pub description: String,
    /// The icon PNG (square, 256 px or more), if any.
    pub icon: Option<PathBuf>,
    /// The code-signing certificate's thumbprint, if any.
    pub sign_thumbprint: Option<String>,
    /// The timestamp server for signatures.
    pub timestamp_url: String,
}

fn string(t: &toml::Table, section: &str, key: &str) -> Option<String> {
    t.get(section)?.get(key)?.as_str().map(str::to_owned)
}

impl Project {
    /// Reads the app at `root`.
    pub fn load(root: &Path) -> Result<Project, String> {
        let cargo_path = root.join("Cargo.toml");
        let cargo: toml::Table = std::fs::read_to_string(&cargo_path)
            .map_err(|e| format!("cannot read {}: {e}", cargo_path.display()))?
            .parse()
            .map_err(|e| format!("{}: {e}", cargo_path.display()))?;
        let package = string(&cargo, "package", "name").ok_or("Cargo.toml has no [package] name")?;
        let version = string(&cargo, "package", "version").unwrap_or_else(|| "0.1.0".into());
        let bin = cargo
            .get("bin")
            .and_then(|b| b.as_array())
            .and_then(|a| a.first())
            .and_then(|b| b.get("name"))
            .and_then(|n| n.as_str())
            .map_or_else(|| package.clone(), str::to_owned);
        let remedy_path = root.join("remedy.toml");
        let remedy: toml::Table = match std::fs::read_to_string(&remedy_path) {
            Ok(s) => s.parse().map_err(|e| format!("{}: {e}", remedy_path.display()))?,
            Err(_) => toml::Table::new(),
        };
        let target_dir = target_dir(root).unwrap_or_else(|| root.join("target"));
        Ok(Project {
            root: root.to_path_buf(),
            name: string(&remedy, "app", "name").unwrap_or_else(|| package.clone()),
            id: string(&remedy, "app", "id").unwrap_or_else(|| format!("app.remedy.{package}")),
            publisher: string(&remedy, "app", "publisher").unwrap_or_else(|| package.clone()),
            description: string(&remedy, "app", "description").unwrap_or_default(),
            icon: string(&remedy, "app", "icon").map(|p| root.join(p)),
            sign_thumbprint: string(&remedy, "windows", "sign_thumbprint"),
            timestamp_url: string(&remedy, "windows", "timestamp_url")
                .unwrap_or_else(|| "http://timestamp.digicert.com".into()),
            bin,
            version,
            target_dir,
        })
    }

    /// The built executable for a profile.
    pub fn exe(&self, release: bool) -> PathBuf {
        let name = if cfg!(windows) { format!("{}.exe", self.bin) } else { self.bin.clone() };
        self.target_dir.join(if release { "release" } else { "debug" }).join(name)
    }
}

/// Cargo's target directory for the package at `root`.
fn target_dir(root: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new(cargo())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("target_directory")?.as_str().map(PathBuf::from)
}

/// The cargo that ran us, or `cargo`.
pub fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// Runs a command, failing with its name when it fails.
pub fn status(cmd: &mut std::process::Command) -> Result<(), String> {
    let s = cmd.status().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if s.success() { Ok(()) } else { Err(format!("{:?} failed ({s})", cmd.get_program())) }
}
