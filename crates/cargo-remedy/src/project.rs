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
    /// The icon as drawn at other sizes (`[app] icon_sizes`, square PNGs), used as they are
    /// where those sizes are shown.
    pub icon_sizes: Vec<PathBuf>,
    /// Other sizes are made by the nearest pixel (`[app] icon_scaling = "pixel"`, for pixel
    /// art) rather than by area averages (`"smooth"`, the default).
    pub icon_pixel_art: bool,
    /// The one-color shape of the icon (`[android] icon_monochrome`, a square PNG whose
    /// alpha is the shape): Android 13's themed icons and the status bar's notification icon.
    pub android_icon_monochrome: Option<PathBuf>,
    /// The picture in front in Android's adaptive icon (`[android] icon_foreground`, a square
    /// PNG placed like the icon): the icon without the backdrop it is drawn on, so the
    /// launcher's mask cuts the backdrop and not the picture. By default the icon itself.
    pub android_icon_foreground: Option<PathBuf>,
    /// What fills Android's adaptive icon behind it (`[android] icon_background`): one
    /// `#rrggbb` color, or two for a gradient from top to bottom; empty for the icon's corner
    /// color when it is opaque, else white.
    pub android_icon_background: Vec<String>,
    /// The code-signing certificate's thumbprint, if any.
    pub sign_thumbprint: Option<String>,
    /// The arguments the installer's "start when I sign in" choice runs the app with
    /// (`[windows] autostart_args`); unset for no such choice.
    pub windows_autostart: Option<String>,
    /// An earlier per-user install this one takes over (`[windows.replaces]`), such as the
    /// same app's old NSIS build; unset when there is none.
    pub windows_replaces: Option<Replaces>,
    /// The oldest glibc the Linux build runs on (`[linux] glibc`, such as `"2.28"`), built
    /// with `cargo zigbuild`; unset builds against this machine's glibc.
    pub linux_glibc: Option<String>,
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
    /// The kinds of foreground work the app does (`[android] foreground`), as Android's
    /// foreground service types (`microphone`, `mediaPlayback`, `camera`, `dataSync`): the
    /// manifest declares the foreground service with them, and their permissions.
    pub android_foreground: Vec<String>,
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
    /// The URL schemes that open the app (`[app] url_schemes`, `["hearth"]`): the macOS
    /// bundle declares them, the way macOS requires (other desktops register at run time),
    /// and the Android manifest takes links in them (`remedy_ui::android::launch_url`).
    pub url_schemes: Vec<String>,
}

/// The theme color pages get when `remedy.toml` names none: Dark Forest's background, the
/// default theme's.
pub const DEFAULT_THEME_COLOR: &str = "#1e2a24";

fn string(t: &toml::Table, section: &str, key: &str) -> Option<String> {
    t.get(section)?.get(key)?.as_str().map(str::to_owned)
}

/// An earlier install the Windows installer removes before it installs: only the program
/// files named here, its Start menu and desktop shortcuts, its uninstall entry and the
/// registry keys named here. Nothing else in its folder, and never the app's data.
#[derive(Clone, Debug, PartialEq)]
pub struct Replaces {
    /// Its key under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall`, whose
    /// `InstallLocation` says where it is.
    pub uninstall_key: String,
    /// The file names in that folder it installed; the first is the program, which must be
    /// closed before anything is removed.
    pub files: Vec<String>,
    /// Further `HKCU` keys it wrote, each under `Software\`.
    pub registry: Vec<String>,
    /// A named mutex the old program holds while it runs (`mutex`); setup waits for it.
    pub mutex: Option<String>,
    /// The display name of a per-machine MSI build of it (`msi_name`), which setup
    /// uninstalls with `msiexec` (Windows asks for administrator rights).
    pub msi_name: Option<String>,
}

fn windows_replaces(t: &toml::Table) -> Result<Option<Replaces>, String> {
    let Some(r) = t.get("windows").and_then(|w| w.get("replaces")) else { return Ok(None) };
    let bad = |what: &str| format!("remedy.toml: [windows.replaces] {what}");
    let list = |key: &str| -> Result<Vec<String>, String> {
        let Some(v) = r.get(key) else { return Ok(Vec::new()) };
        v.as_array()
            .ok_or_else(|| bad(&format!("{key} must be a list of strings")))?
            .iter()
            .map(|i| {
                i.as_str().map(str::to_owned).ok_or_else(|| bad(&format!("{key} must be a list of strings")))
            })
            .collect()
    };
    let key = r.get("uninstall_key").and_then(|k| k.as_str()).unwrap_or_default().to_owned();
    if key.is_empty() || key.contains(['\\', '/', '\'']) {
        return Err(bad("uninstall_key must be the name of one uninstall entry, such as \"Notes\""));
    }
    let files = list("files")?;
    // Plain names only: a path or a wildcard could reach past the old program's own files.
    let plain = |f: &String| {
        !f.is_empty()
            && f != "."
            && f != ".."
            && !f.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|', '\''])
    };
    if files.is_empty() || !files.iter().all(plain) {
        return Err(bad(
            "files must list the old program's file names, the program first, such as [\"notes.exe\"]",
        ));
    }
    let registry = list("registry")?;
    let under_software = |k: &String| {
        k.strip_prefix("Software\\")
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('\'') && !rest.ends_with('\\'))
            && !k.contains("\\\\")
    };
    if !registry.iter().all(under_software) {
        return Err(bad("registry keys must be under Software\\, such as 'Software\\Maker\\Notes'"));
    }
    let text = |key: &str| -> Result<Option<String>, String> {
        match r.get(key) {
            None => Ok(None),
            Some(v) => match v.as_str() {
                Some(s) if !s.is_empty() && !s.contains(['\'', '"', '\\']) => Ok(Some(s.to_owned())),
                _ => Err(bad(&format!("{key} must be a name without quotes or backslashes"))),
            },
        }
    };
    Ok(Some(Replaces {
        uninstall_key: key,
        files,
        registry,
        mutex: text("mutex")?,
        msi_name: text("msi_name")?,
    }))
}

/// `[linux] glibc`: a version such as `2.28` (digits and one or two dots).
fn linux_glibc(t: &toml::Table) -> Result<Option<String>, String> {
    let Some(v) = string(t, "linux", "glibc") else { return Ok(None) };
    let parts: Vec<&str> = v.split('.').collect();
    let ok = (2..=3).contains(&parts.len())
        && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !ok {
        return Err(format!("remedy.toml: [linux] glibc: `{v}` is not a glibc version such as 2.28"));
    }
    Ok(Some(v))
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

/// The schemes `[app] url_schemes` lists, lowercased and checked as RFC 3986 says.
fn url_schemes(t: &toml::Table) -> Result<Vec<String>, String> {
    let Some(v) = t.get("app").and_then(|a| a.get("url_schemes")) else { return Ok(Vec::new()) };
    let bad = "remedy.toml: [app] url_schemes must be a list of schemes such as \"hearth\"";
    let mut out: Vec<String> = Vec::new();
    for item in v.as_array().ok_or(bad)? {
        let s = item.as_str().ok_or(bad)?.to_ascii_lowercase();
        let mut chars = s.chars();
        let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && !["http", "https", "file"].contains(&s.as_str());
        if !ok {
            return Err(format!(
                "remedy.toml: [app] url_schemes: `{s}` is not a scheme an app can take (a letter, then letters, digits, '+', '-' or '.')"
            ));
        }
        if !out.contains(&s) {
            out.push(s);
        }
    }
    Ok(out)
}

/// The files `[app] icon_sizes` lists.
fn icon_sizes(t: &toml::Table) -> Result<Vec<String>, String> {
    let Some(v) = t.get("app").and_then(|a| a.get("icon_sizes")) else { return Ok(Vec::new()) };
    let bad = "remedy.toml: [app] icon_sizes must be a list of PNG files";
    v.as_array()
        .ok_or(bad)?
        .iter()
        .map(|i| i.as_str().map(str::to_owned).ok_or_else(|| bad.to_owned()))
        .collect()
}

/// `[app] icon_scaling`: `"pixel"` is true, `"smooth"` (the default) false.
fn icon_pixel_art(t: &toml::Table) -> Result<bool, String> {
    match string(t, "app", "icon_scaling").as_deref() {
        None | Some("smooth") => Ok(false),
        Some("pixel") => Ok(true),
        Some(o) => Err(format!("remedy.toml: [app] icon_scaling: `{o}` is not smooth or pixel")),
    }
}

/// `[android] icon_background`: a `#rrggbb` color or a list of two (a gradient from top to
/// bottom), each checked.
fn android_icon_background(t: &toml::Table) -> Result<Vec<String>, String> {
    let Some(v) = t.get("android").and_then(|a| a.get("icon_background")) else { return Ok(Vec::new()) };
    let bad = "remedy.toml: [android] icon_background must be a #rrggbb color or a list of two";
    let colors: Vec<&str> = match v {
        toml::Value::String(c) => vec![c.as_str()],
        toml::Value::Array(list) => list.iter().map(|c| c.as_str().ok_or(bad)).collect::<Result<_, _>>()?,
        _ => return Err(bad.into()),
    };
    if colors.is_empty() || colors.len() > 2 {
        return Err(bad.into());
    }
    colors
        .into_iter()
        .map(|c| {
            let ok =
                c.strip_prefix('#').is_some_and(|h| h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()));
            if ok { Ok(c.to_ascii_lowercase()) } else { Err(format!("{bad}: `{c}` is not #rrggbb")) }
        })
        .collect()
}

/// The foreground service types `[android] foreground` lists (`microphone`, `media-playback`,
/// `camera`, `data-sync`), as Android names them, in order and without repeats.
fn android_foreground(t: &toml::Table) -> Result<Vec<String>, String> {
    let Some(v) = t.get("android").and_then(|a| a.get("foreground")) else { return Ok(Vec::new()) };
    let bad = "remedy.toml: [android] foreground must be a list of kinds";
    let list = v.as_array().ok_or(bad)?;
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let kind = match item.as_str().ok_or(bad)? {
            "microphone" => "microphone",
            "media-playback" => "mediaPlayback",
            "camera" => "camera",
            "data-sync" => "dataSync",
            k => {
                return Err(format!(
                    "remedy.toml: [android] foreground: `{k}` is not microphone, media-playback, camera or data-sync"
                ));
            }
        };
        if !out.iter().any(|o| o == kind) {
            out.push(kind.to_owned());
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
        let metadata = metadata(root);
        // A version inherited from the workspace (`version.workspace = true`) is Cargo's to
        // resolve: read it back from `cargo metadata`.
        let version = string(&cargo, "package", "version")
            .or_else(|| metadata.as_ref().and_then(|m| package_version(m, &package)))
            .unwrap_or_else(|| "0.1.0".into());
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
        let target_dir = metadata
            .as_ref()
            .and_then(|m| m.get("target_directory")?.as_str().map(PathBuf::from))
            .unwrap_or_else(|| root.join("target"));
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
            icon_sizes: icon_sizes(&remedy)?.into_iter().map(|p| root.join(p)).collect(),
            icon_pixel_art: icon_pixel_art(&remedy)?,
            android_icon_monochrome: string(&remedy, "android", "icon_monochrome").map(|p| root.join(p)),
            android_icon_foreground: string(&remedy, "android", "icon_foreground").map(|p| root.join(p)),
            android_icon_background: android_icon_background(&remedy)?,
            sign_thumbprint: string(&remedy, "windows", "sign_thumbprint"),
            windows_autostart: string(&remedy, "windows", "autostart_args"),
            windows_replaces: windows_replaces(&remedy)?,
            linux_glibc: linux_glibc(&remedy)?,
            timestamp_url: string(&remedy, "windows", "timestamp_url")
                .unwrap_or_else(|| "http://timestamp.digicert.com".into()),
            android_target: string(&remedy, "android", "target")
                .unwrap_or_else(|| "aarch64-linux-android".into()),
            android_min_api: number(&remedy, "android", "min_api").unwrap_or(26),
            android_target_api: number(&remedy, "android", "target_api").unwrap_or(35),
            android_keystore: string(&remedy, "android", "keystore").map(|k| root.join(k)),
            android_key_alias: string(&remedy, "android", "key_alias"),
            android_permissions: android_permissions(&remedy)?,
            url_schemes: url_schemes(&remedy)?,
            android_foreground: android_foreground(&remedy)?,
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

/// `cargo metadata` (without dependencies) for the package at `root`.
fn metadata(root: &Path) -> Option<serde_json::Value> {
    let out = std::process::Command::new(cargo())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

/// The version `cargo metadata` gives the package named `name`.
fn package_version(metadata: &serde_json::Value, name: &str) -> Option<String> {
    metadata
        .get("packages")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))?
        .get("version")?
        .as_str()
        .map(str::to_owned)
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
            icon_sizes: Vec::new(),
            icon_pixel_art: false,
            android_icon_monochrome: None,
            android_icon_foreground: None,
            android_icon_background: Vec::new(),
            sign_thumbprint: None,
            windows_autostart: None,
            windows_replaces: None,
            linux_glibc: None,
            timestamp_url: String::new(),
            android_target: "aarch64-linux-android".into(),
            android_min_api: 26,
            android_target_api: 35,
            android_keystore: None,
            android_permissions: Vec::new(),
            android_foreground: Vec::new(),
            android_key_alias: None,
            web_short_name: None,
            web_theme: DEFAULT_THEME_COLOR.into(),
            web_icon: None,
            uses_remedy_ui: true,
            url_schemes: Vec::new(),
        }
    }

    #[test]
    fn workspace_versions_come_from_cargo_metadata() {
        let m = serde_json::json!({ "packages": [{ "name": "a", "version": "1.0.0" }, { "name": "b", "version": "0.3.0" }] });
        assert_eq!(package_version(&m, "b").as_deref(), Some("0.3.0"));
        assert_eq!(package_version(&m, "c"), None);
    }

    #[test]
    fn icon_settings_are_read_and_checked() {
        let parse = |s: &str| s.parse::<toml::Table>().unwrap_or_default();
        let t = parse(
            "[app]\nicon_sizes = [\"a.png\", \"b.png\"]\nicon_scaling = \"pixel\"\n[android]\nicon_background = [\"#AABBCC\", \"#000000\"]\n",
        );
        assert_eq!(icon_sizes(&t).unwrap_or_default(), ["a.png", "b.png"]);
        assert_eq!(icon_pixel_art(&t), Ok(true));
        assert_eq!(android_icon_background(&t).unwrap_or_default(), ["#aabbcc", "#000000"]);
        assert_eq!(
            android_icon_background(&parse("[android]\nicon_background = \"#123456\"\n")).unwrap_or_default(),
            ["#123456"]
        );
        assert!(android_icon_background(&parse("[android]\nicon_background = \"blue\"\n")).is_err());
        assert!(
            android_icon_background(&parse(
                "[android]\nicon_background = [\"#000000\", \"#111111\", \"#222222\"]\n"
            ))
            .is_err()
        );
        assert!(icon_pixel_art(&parse("[app]\nicon_scaling = \"blurry\"\n")).is_err());
        assert_eq!(icon_pixel_art(&parse("")), Ok(false));
    }

    #[test]
    fn a_replaced_install_names_plain_files_and_software_keys() {
        let parse = |s: &str| windows_replaces(&toml::from_str(s).unwrap_or_default());
        let ok = parse(
            "[windows.replaces]\nuninstall_key = \"Notes\"\nfiles = [\"notes.exe\", \"uninstall.exe\"]\nregistry = ['Software\\maker\\Notes']\nmutex = \"notes-running\"\nmsi_name = \"Notes\"",
        );
        assert_eq!(
            ok,
            Ok(Some(Replaces {
                uninstall_key: "Notes".into(),
                files: vec!["notes.exe".into(), "uninstall.exe".into()],
                registry: vec!["Software\\maker\\Notes".into()],
                mutex: Some("notes-running".into()),
                msi_name: Some("Notes".into()),
            }))
        );
        assert_eq!(parse("[windows]\nx = 1"), Ok(None));
        for bad in [
            "[windows.replaces]\nfiles = [\"notes.exe\"]",
            "[windows.replaces]\nuninstall_key = \"Notes\"",
            "[windows.replaces]\nuninstall_key = \"Notes\"\nfiles = [\"..\\\\vault.json\"]",
            "[windows.replaces]\nuninstall_key = \"Notes\"\nfiles = [\"*.json\"]",
            "[windows.replaces]\nuninstall_key = \"Notes\"\nfiles = [\"notes.exe\"]\nregistry = ['Software']",
            "[windows.replaces]\nuninstall_key = \"Notes\"\nfiles = [\"notes.exe\"]\nregistry = ['System\\x']",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn linux_glibc_is_a_version() {
        let parse = |s: &str| linux_glibc(&toml::from_str(s).unwrap_or_default());
        assert_eq!(parse("[linux]\nglibc = \"2.28\""), Ok(Some("2.28".into())));
        assert_eq!(parse("[linux]\nx = 1"), Ok(None));
        for bad in ["2", "2.28; rm", "a.b", "2..28", "2.28.1.0"] {
            assert!(parse(&format!("[linux]\nglibc = \"{bad}\"")).is_err(), "{bad}");
        }
    }

    #[test]
    fn url_schemes_are_checked_and_lowercased() {
        let t: toml::Table = toml::from_str("[app]\nurl_schemes = [\"Hearth\", \"hearth\", \"web+hearth\"]")
            .unwrap_or_default();
        assert_eq!(url_schemes(&t), Ok(vec!["hearth".to_owned(), "web+hearth".to_owned()]));
        let bad: toml::Table = toml::from_str("[app]\nurl_schemes = [\"https\"]").unwrap_or_default();
        assert!(url_schemes(&bad).is_err());
        assert_eq!(url_schemes(&toml::Table::new()), Ok(Vec::new()));
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

    #[test]
    fn android_foreground_kinds_become_service_types_once() {
        let t: toml::Table = r#"
            [android]
            foreground = ["microphone", "media-playback", "microphone", "data-sync", "camera"]
        "#
        .parse()
        .unwrap_or_default();
        assert_eq!(
            android_foreground(&t),
            Ok(vec!["microphone".to_owned(), "mediaPlayback".into(), "dataSync".into(), "camera".into()])
        );
        let bad: toml::Table = "[android]\nforeground = [\"location\"]".parse().unwrap_or_default();
        assert!(android_foreground(&bad).is_err_and(|e| e.contains("`location`")));
        assert_eq!(android_foreground(&toml::Table::new()), Ok(Vec::new()));
    }
}
