//! Repository checks for RemedyUI. Run `cargo xtask check`.
//!
//! - **Layers.** Every `remedy-ui-*` crate has a layer (docs/design/00-principles-and-modularity.md).
//!   A crate may depend on crates in lower layers, plus the sideways edges listed in
//!   [`SIDEWAYS`]. Anything else, and any crate missing from [`LAYERS`], fails the check.
//! - **Unsafe code.** Every crate inherits the workspace lints (`unsafe_code = "deny"`; deny
//!   rather than forbid, because forbid cannot be lifted even where it must be). Only the
//!   crates in [`UNSAFE_BOUNDARY`] may contain `allow(unsafe_code)`, and only on a module named
//!   `os` or `ffi`; this check is what holds that line.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Each RemedyUI crate and its layer: L0 base … L6 app-facing.
const LAYERS: &[(&str, u8)] = &[
    ("remedy-ui-core", 0),
    ("remedy-ui-reactive", 1),
    // Proc macros run at build time; they sit with the state layer they generate code for.
    ("remedy-ui-macros", 1),
    ("remedy-ui-tree", 2),
    ("remedy-ui-style", 2),
    ("remedy-ui-layout", 2),
    ("remedy-ui-text", 2),
    ("remedy-ui-input", 2),
    ("remedy-ui-paint", 3),
    ("remedy-ui-motion", 3),
    ("remedy-ui-a11y", 3),
    ("remedy-ui-compositor", 3),
    ("remedy-ui-secure", 3),
    // The host model: the platform-neutral contract bridges implement.
    ("remedy-ui-host", 3),
    ("remedy-ui-render-gpu", 4),
    ("remedy-ui-render-cpu", 4),
    // The desktop bridges share one winit core.
    ("remedy-ui-bridge-winit", 4),
    ("remedy-ui-bridge-windows", 4),
    ("remedy-ui-bridge-macos", 4),
    ("remedy-ui-bridge-wayland", 4),
    ("remedy-ui-bridge-x11", 4),
    ("remedy-ui-bridge-android", 4),
    ("remedy-ui-bridge-web", 4),
    ("remedy-ui-bridge-remedyos", 4),
    ("remedy-ui-bridge-ios", 4),
    ("remedy-ui-bridge-headless", 4),
    ("remedy-ui-widgets-core", 5),
    ("remedy-ui-widgets-nav", 5),
    ("remedy-ui-widgets-data", 5),
    ("remedy-ui-widgets-forms", 5),
    ("remedy-ui-widgets-extra", 5),
    ("remedy-ui-widgets-music", 5),
    ("remedy-ui-widgets-term", 5),
    ("remedy-ui-doc-rich", 5),
    ("remedy-ui-doc-code", 5),
    ("remedy-ui-doc-pages", 5),
    ("remedy-ui-media-image", 5),
    ("remedy-ui-media-anim", 5),
    ("remedy-ui-media-svg", 5),
    ("remedy-ui-media-video", 5),
    ("remedy-ui-media-playback", 5),
    ("remedy-ui-media-isolated", 5),
    ("remedy-ui-viz", 5),
    ("remedy-ui-embed-webview", 5),
    ("remedy-ui-embed-gpu", 5),
    ("remedy-ui-services-desktop", 5),
    ("remedy-ui-services-update", 5),
    ("remedy-ui-services-spell", 5),
    ("remedy-ui-services-i18n", 5),
    ("remedy-ui-themes", 5),
    ("remedy-ui-fonts", 5),
    ("remedy-ui-devtools", 5),
    ("remedy-ui", 6),
    ("remedy-ui-rui", 6),
    ("remedy-ui-test", 6),
];

/// Allowed dependencies inside one layer: (crate, dependency).
const SIDEWAYS: &[(&str, &str)] = &[
    ("remedy-ui-widgets-nav", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-data", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-forms", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-extra", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-music", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-term", "remedy-ui-widgets-core"),
    ("remedy-ui-doc-rich", "remedy-ui-widgets-core"),
    ("remedy-ui-doc-code", "remedy-ui-widgets-core"),
    ("remedy-ui-doc-pages", "remedy-ui-widgets-core"),
    ("remedy-ui-viz", "remedy-ui-widgets-core"),
    ("remedy-ui-embed-webview", "remedy-ui-widgets-core"),
    ("remedy-ui-embed-gpu", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-music", "remedy-ui-viz"),
    ("remedy-ui-doc-rich", "remedy-ui-doc-code"),
    ("remedy-ui-media-anim", "remedy-ui-media-image"),
    // The compositor holds display lists and runs motion curves on its own thread.
    ("remedy-ui-compositor", "remedy-ui-paint"),
    ("remedy-ui-compositor", "remedy-ui-motion"),
    // The GPU renderer shares the CPU renderer's painter, and falls back to it.
    ("remedy-ui-render-gpu", "remedy-ui-render-cpu"),
    ("remedy-ui-bridge-windows", "remedy-ui-bridge-winit"),
    ("remedy-ui-bridge-macos", "remedy-ui-bridge-winit"),
    ("remedy-ui-bridge-wayland", "remedy-ui-bridge-winit"),
    ("remedy-ui-bridge-x11", "remedy-ui-bridge-winit"),
];

/// Crates that may hold unsafe code, each in one `os` or `ffi` module.
const UNSAFE_BOUNDARY: &[&str] = &[
    "remedy-ui-render-gpu",
    "remedy-ui-bridge-windows",
    "remedy-ui-bridge-macos",
    "remedy-ui-bridge-wayland",
    "remedy-ui-bridge-x11",
    "remedy-ui-bridge-android",
    "remedy-ui-bridge-web",
    "remedy-ui-bridge-remedyos",
    "remedy-ui-bridge-ios",
];

fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

struct Package {
    name: String,
    manifest: PathBuf,
    deps: Vec<String>,
}

fn packages() -> Result<Vec<Package>, String> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("could not run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata output: {e}"))?;
    let list = json.get("packages").and_then(|p| p.as_array()).ok_or("cargo metadata has no packages")?;
    let mut pkgs = Vec::new();
    for p in list {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let manifest = PathBuf::from(p.get("manifest_path").and_then(|v| v.as_str()).unwrap_or_default());
        let deps = p
            .get("dependencies")
            .and_then(|d| d.as_array())
            .map(|d| {
                d.iter()
                    .filter(|dep| dep.get("kind").and_then(|k| k.as_str()) != Some("dev"))
                    .filter_map(|dep| dep.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        pkgs.push(Package { name, manifest, deps });
    }
    Ok(pkgs)
}

fn check_layers(pkgs: &[Package], problems: &mut Vec<String>) {
    for p in pkgs.iter().filter(|p| p.name.starts_with("remedy-ui")) {
        let Some(own) = layer(&p.name) else {
            problems.push(format!("{} has no layer; add it to LAYERS in xtask/src/main.rs", p.name));
            continue;
        };
        for d in p.deps.iter().filter(|d| d.starts_with("remedy-ui")) {
            let Some(dl) = layer(d) else {
                problems.push(format!("{} depends on {d}, which has no layer", p.name));
                continue;
            };
            let sideways_ok = SIDEWAYS.contains(&(p.name.as_str(), d.as_str()));
            if dl > own || (dl == own && !sideways_ok) {
                problems.push(format!(
                    "{} (L{own}) must not depend on {d} (L{dl}); see docs/design/00-principles-and-modularity.md",
                    p.name
                ));
            }
        }
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

fn check_unsafe(pkgs: &[Package], problems: &mut Vec<String>) {
    for p in pkgs {
        let manifest = std::fs::read_to_string(&p.manifest).unwrap_or_default();
        let inherits =
            manifest.lines().collect::<Vec<_>>().windows(2).any(
                |w| matches!(w, [a, b] if a.trim() == "[lints]" && b.replace(' ', "") == "workspace=true"),
            );
        if !inherits {
            problems
                .push(format!("{} does not inherit the workspace lints ([lints] workspace = true)", p.name));
        }
        let Some(dir) = p.manifest.parent() else { continue };
        let mut files = Vec::new();
        rust_files(dir, &mut files);
        let boundary = UNSAFE_BOUNDARY.contains(&p.name.as_str());
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let t = line.trim_start();
                let is_attribute = (t.starts_with("#[") || t.starts_with("#![")) && t.contains("allow(");
                if !(is_attribute && t.contains("unsafe_code")) {
                    continue;
                }
                let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or_default();
                let on_boundary_module =
                    matches!(next, "mod os;" | "mod ffi;" | "pub(crate) mod os;" | "pub(crate) mod ffi;");
                if !(boundary && on_boundary_module) {
                    problems.push(format!(
                        "{}:{}: allow(unsafe_code) is only permitted on an `os`/`ffi` module of a boundary crate",
                        f.display(),
                        i + 1
                    ));
                }
            }
        }
    }
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    if task != "check" {
        eprintln!("usage: cargo xtask check");
        return ExitCode::from(2);
    }
    let pkgs = match packages() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut problems = Vec::new();
    check_layers(&pkgs, &mut problems);
    check_unsafe(&pkgs, &mut problems);
    if problems.is_empty() {
        println!("ok: {} crates follow the layer graph and the unsafe-code boundary", pkgs.len());
        ExitCode::SUCCESS
    } else {
        for p in &problems {
            eprintln!("error: {p}");
        }
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, deps: &[&str]) -> Package {
        Package {
            name: name.into(),
            manifest: PathBuf::new(),
            deps: deps.iter().map(|d| d.to_string()).collect(),
        }
    }

    fn problems(pkgs: &[Package]) -> Vec<String> {
        let mut out = Vec::new();
        check_layers(pkgs, &mut out);
        out
    }

    #[test]
    fn upward_dependencies_are_rejected() {
        let p = problems(&[pkg("remedy-ui-core", &["remedy-ui-tree"])]);
        assert_eq!(p.len(), 1);
        assert!(p.iter().all(|m| m.contains("must not depend")));
    }

    #[test]
    fn sideways_needs_an_entry() {
        assert!(problems(&[pkg("remedy-ui-widgets-nav", &["remedy-ui-widgets-core"])]).is_empty());
        assert_eq!(problems(&[pkg("remedy-ui-widgets-data", &["remedy-ui-widgets-nav"])]).len(), 1);
    }

    #[test]
    fn downward_and_unknown() {
        assert!(problems(&[pkg("remedy-ui-tree", &["remedy-ui-reactive", "remedy-ui-core"])]).is_empty());
        let p = problems(&[pkg("remedy-ui-unlisted", &[])]);
        assert!(p.iter().any(|m| m.contains("has no layer")));
    }
}
