//! What `run` and `package` need to know about the app in the current directory.

use std::path::{Path, PathBuf};

/// The app: its Cargo package and its `remedy.toml`.
#[derive(Clone, Debug)]
pub struct Project {
    /// The directory holding Cargo.toml.
    pub root: PathBuf,
    /// The executable's name.
    pub bin: String,
    /// The library's crate name (what an Android APK loads).
    pub lib_name: String,
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
    /// The Rust target Android packages build for (`[android] target`).
    pub android_target: String,
    /// The lowest Android API level (`[android] min_api`, 26 by default: the bridge loads its
    /// Java half with `InMemoryDexClassLoader`).
    pub android_min_api: u32,
    /// The Android API level built against (`[android] target_api`).
    pub android_target_api: u32,
    /// The release keystore (`[android] keystore`); a debug key is made when there is none.
    pub android_keystore: Option<PathBuf>,
    /// The key's alias in it (`[android] key_alias`).
    pub android_key_alias: Option<String>,
    /// The Android permissions the app's manifest declares (`[android] permissions`), as
    /// manifest names: `microphone`, `internet`, `notifications` and `camera` stand for
    /// theirs, and a full name (`android.permission.VIBRATE`) is taken as it is.
    pub android_permissions: Vec<String>,
    /// The short name an installed page shows under its icon (`[web] short_name`; the name
    /// when unset).
    pub web_short_name: Option<String>,
    /// The color of the browser's chrome around the page and of its splash screen
    /// (`[web] theme_color`).
    pub web_theme: String,
    /// The page's icon (`[web] icon`, SVG or PNG), when it differs from the app's.
    pub web_icon: Option<PathBuf>,
    /// The package depends on `remedy-ui` itself (so its `web` feature can be turned on).
    pub uses_remedy_ui: bool,
}

/// The theme color pages get when `remedy.toml` names none: Dark Forest's background, the
/// default theme's.
pub const DEFAULT_THEME_COLOR: &str = "#1e2a24";

fn string(t: &toml::Table, section: &str, key: &str) -> Option<String> {
    t.get(section)?.get(key)?.as_str().map(str::to_owned)
}

fn number(t: &toml::Table, section: &str, key: &str) -> Option<u32> {
    t.get(section)?.get(key)?.as_integer().and_then(|n| u32::try_from(n).ok())
}

/// The manifest names `[android] permissions` lists, in order and without repeats.
fn android_permissions(t: &toml::Table) -> Result<Vec<String>, String> {
    let Some(v) = t.get("android").and_then(|a| a.get("permissions")) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or("remedy.toml: [android] permissions must be a list of names")?;
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let name = item.as_str().ok_or("remedy.toml: [android] permissions must be a list of names")?;
        let full = match name {
            "microphone" => "android.permission.RECORD_AUDIO".to_owned(),
            "internet" => "android.permission.INTERNET".to_owned(),
            "notifications" => "android.permission.POST_NOTIFICATIONS".to_owned(),
            "camera" => "android.permission.CAMERA".to_owned(),
            n if n.contains('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') => {
                n.to_owned()
            }
            n => {
                return Err(format!(
                    "remedy.toml: [android] permissions: `{n}` is not microphone, internet, notifications, \
                     camera or a full name such as android.permission.VIBRATE"
                ));
            }
        };
        if !out.contains(&full) {
            out.push(full);
        }
    }
    Ok(out)
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
        let lib_name = cargo
            .get("lib")
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str())
            .map_or_else(|| package.replace('-', "_"), str::to_owned);
        let remedy_path = root.join("remedy.toml");
        let remedy: toml::Table = match std::fs::read_to_string(&remedy_path) {
            Ok(s) => s.parse().map_err(|e| format!("{}: {e}", remedy_path.display()))?,
            Err(_) => toml::Table::new(),
        };
        let target_dir = target_dir(root).unwrap_or_else(|| root.join("target"));
        let direct = |t: &toml::Value| t.get("dependencies").and_then(|d| d.get("remedy-ui")).is_some();
        let uses_remedy_ui = cargo.get("dependencies").and_then(|d| d.get("remedy-ui")).is_some()
            || cargo.get("target").and_then(|t| t.as_table()).is_some_and(|t| t.values().any(direct));
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
            android_target: string(&remedy, "android", "target")
                .unwrap_or_else(|| "aarch64-linux-android".into()),
            android_min_api: number(&remedy, "android", "min_api").unwrap_or(26),
            android_target_api: number(&remedy, "android", "target_api").unwrap_or(35),
            android_keystore: string(&remedy, "android", "keystore").map(|k| root.join(k)),
            android_key_alias: string(&remedy, "android", "key_alias"),
            android_permissions: android_permissions(&remedy)?,
            web_short_name: string(&remedy, "web", "short_name"),
            web_theme: string(&remedy, "web", "theme_color").unwrap_or_else(|| DEFAULT_THEME_COLOR.into()),
            web_icon: string(&remedy, "web", "icon").map(|p| root.join(p)),
            uses_remedy_ui,
            bin,
            lib_name,
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A project as `remedy.toml` describes one, for the packaging tests.
    pub(crate) fn sample() -> Project {
        Project {
            root: PathBuf::from("."),
            bin: "my-notes".into(),
            lib_name: "my_notes".into(),
            version: "1.2.3".into(),
            target_dir: PathBuf::from("target"),
            name: "My Notes".into(),
            id: "app.remedy.my-notes".into(),
            publisher: "Me".into(),
            description: "Notes & more".into(),
            icon: None,
            sign_thumbprint: None,
            timestamp_url: String::new(),
            android_target: "aarch64-linux-android".into(),
            android_min_api: 26,
            android_target_api: 35,
            android_keystore: None,
            android_permissions: Vec::new(),
            android_key_alias: None,
            web_short_name: None,
            web_theme: DEFAULT_THEME_COLOR.into(),
            web_icon: None,
            uses_remedy_ui: true,
        }
    }

    #[test]
    fn android_permissions_take_short_and_full_names_once() {
        let t: toml::Table = r#"
            [android]
            permissions = ["microphone", "internet", "android.permission.VIBRATE", "microphone"]
        "#
        .parse()
        .unwrap_or_default();
        assert_eq!(
            android_permissions(&t),
            Ok(vec![
                "android.permission.RECORD_AUDIO".to_owned(),
                "android.permission.INTERNET".to_owned(),
                "android.permission.VIBRATE".to_owned(),
            ])
        );
        let bad: toml::Table = "[android]\npermissions = [\"mic\"]".parse().unwrap_or_default();
        assert!(android_permissions(&bad).is_err_and(|e| e.contains("`mic`")));
        assert_eq!(android_permissions(&toml::Table::new()), Ok(Vec::new()));
    }
}
