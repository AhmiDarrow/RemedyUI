//! Repository checks and release tools for RemedyUI.
//!
//! - `cargo xtask check`: the layer graph, the unsafe-code boundary and publishing metadata.
//! - `cargo xtask semver --baseline <git revision or tag>`: every publishable crate's API
//!   against the baseline, with `cargo-semver-checks` ([`SEMVER_CHECKS_VERSION`]).
//! - `cargo xtask msrv`: the workspace builds with the Rust version it declares
//!   (`rust-version`), installing that toolchain with rustup when it is missing.
//!
//! - **Layers.** Every `remedy-ui-*` crate has a layer (docs/design/00-principles-and-modularity.md).
//!   A crate may depend on crates in lower layers, plus the sideways edges listed in
//!   [`SIDEWAYS`]. Anything else, and any crate missing from [`LAYERS`], fails the check.
//! - **Unsafe code.** Every crate inherits the workspace lints (`unsafe_code = "deny"`; deny
//!   rather than forbid, because forbid cannot be lifted even where it must be). Only the
//!   crates in [`UNSAFE_BOUNDARY`] may contain `allow(unsafe_code)`, and only on a module named
//!   `os` or `ffi`; this check is what holds that line.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// What the developer tools know of the checkout: the same code `remedy-tools` reads a live
/// checkout with, so the committed index and `--docs-root` agree.
#[path = "../../crates/remedy-tools/src/extract.rs"]
#[allow(dead_code, reason = "xtask uses the index writer; remedy-tools the readers")]
mod extract;

/// The committed docs index (zlib-compressed JSON), embedded by `remedy-tools`.
const DOCS_INDEX: &str = "crates/remedy-tools/data/docs-index.json.zz";

/// The checkout's root.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Writes the docs index from the guides, design docs, rustdoc and widget catalog.
fn docs_index() -> Result<(), String> {
    let root = repo_root();
    let json = extract::index_json(&root);
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    let bytes = enc.finish().map_err(|e| e.to_string())?;
    let path = root.join(DOCS_INDEX);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {DOCS_INDEX}: {} KB ({} KB of JSON)", bytes.len() / 1024, json.len() / 1024);
    Ok(())
}

/// The committed docs index must be what `cargo xtask docs-index` writes now: the tools
/// would otherwise describe an engine that no longer exists.
fn check_docs_index(problems: &mut Vec<String>) {
    let root = repo_root();
    let committed = std::fs::read(root.join(DOCS_INDEX)).unwrap_or_default();
    let mut json = String::new();
    let readable = flate2::read::ZlibDecoder::new(committed.as_slice()).read_to_string(&mut json).is_ok();
    if !readable || json != extract::index_json(&root) {
        problems.push(format!(
            "{DOCS_INDEX} is out of date with the docs and rustdoc: run `cargo xtask docs-index` and commit it"
        ));
    }
}

/// Each RemedyUI crate and its layer: L0 base … L6 app-facing.
const LAYERS: &[(&str, u8)] = &[
    ("remedy-ui-core", 0),
    // Phase spans and the trace clock over `tracing`: every layer emits them.
    ("remedy-ui-trace", 0),
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
    // Prism, the effects system: the .rfx language, its validator and the CPU kernels. Plain
    // data and arithmetic over the display list's types; renderers and elements build on it.
    ("remedy-ui-effects", 3),
    ("remedy-ui-secure", 3),
    // Scripted widgets: the language-neutral interface for untrusted code that describes
    // elements, with no UI of its own, and its engines (Rhai, WebAssembly).
    ("remedy-ui-script", 3),
    ("remedy-ui-script-rhai", 3),
    ("remedy-ui-script-wasm", 3),
    // The host model: the platform-neutral contract bridges implement.
    ("remedy-ui-host", 3),
    // The view layer: elements, the frame pipeline and input routing, over L0-L3.
    ("remedy-ui-view", 4),
    ("remedy-ui-render-gpu", 4),
    ("remedy-ui-render-cpu", 4),
    // The desktop bridges share one winit core.
    ("remedy-ui-bridge-winit", 4),
    ("remedy-ui-bridge-windows", 4),
    ("remedy-ui-bridge-macos", 4),
    ("remedy-ui-bridge-linux", 4),
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
    ("remedy-ui-widgets-stage", 5),
    ("remedy-ui-widgets-music", 5),
    ("remedy-ui-widgets-term", 5),
    ("remedy-ui-widgets-board", 5),
    ("remedy-ui-doc-rich", 5),
    ("remedy-ui-doc-code", 5),
    ("remedy-ui-automation", 5),
    ("remedy-ui-rui", 5),
    ("remedy-ui-doc-pages", 5),
    ("remedy-ui-media-image", 5),
    ("remedy-ui-media-anim", 5),
    ("remedy-ui-media-svg", 5),
    ("remedy-ui-media-video", 5),
    ("remedy-ui-media-playback", 5),
    ("remedy-ui-media-isolated", 5),
    // Prism effects as elements: the GPU kernel or the CPU renderer, under the ambient policy.
    ("remedy-ui-media-effects", 5),
    ("remedy-ui-viz", 5),
    ("remedy-ui-embed-webview", 5),
    ("remedy-ui-embed-gpu", 5),
    ("remedy-ui-services-desktop", 5),
    ("remedy-ui-services-update", 5),
    ("remedy-ui-services-spell", 5),
    ("remedy-ui-services-i18n", 5),
    // Reads a web view's localStorage store (LevelDB) so apps moving off one keep their data.
    ("remedy-ui-services-webstorage", 5),
    ("remedy-ui-themes", 5),
    ("remedy-ui-fonts", 5),
    // Chinese and Japanese fallback faces: a crate of their own, larger than every other face
    // together.
    ("remedy-ui-fonts-cjk", 5),
    ("remedy-ui-devtools", 5),
    // The code-fluid layer: what people change about an app, and its editors.
    ("remedy-ui-customize", 5),
    ("remedy-ui", 6),
    ("remedy-ui-test", 6),
    // Tools: build on everything, nothing builds on them. remedy-tools holds the developer
    // tools' logic; cargo-remedy (the command line) and remedy-mcp (for AI agents) present it.
    ("cargo-remedy", 7),
    ("remedy-tools", 7),
    ("remedy-mcp", 7),
    // The app templates, shared by `cargo remedy new` and remedy-tools' `new_app`. A crate of
    // their own: remedy-tools cannot depend on cargo-remedy, which depends on remedy-tools.
    ("remedy-templates", 7),
    // Build-script help for apps (the Windows executable's icon and version); cargo-remedy
    // writes the same resources when it packages.
    ("remedy-ui-build", 7),
];

/// Allowed dependencies inside one layer: (crate, dependency).
const SIDEWAYS: &[(&str, &str)] = &[
    ("remedy-ui-widgets-nav", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-data", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-forms", "remedy-ui-widgets-core"),
    // Form values and locale data live in the localization service.
    ("remedy-ui-widgets-forms", "remedy-ui-services-i18n"),
    ("remedy-ui-widgets-extra", "remedy-ui-widgets-core"),
    // The carousel's slide runs on the viz pack's timer loop, which rests while nothing moves.
    ("remedy-ui-widgets-extra", "remedy-ui-viz"),
    // Stage widgets animate on the viz pack's timer loop, and a fader shows its level meter.
    ("remedy-ui-widgets-stage", "remedy-ui-viz"),
    ("remedy-ui-widgets-music", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-term", "remedy-ui-widgets-core"),
    // Boards are built from widgets, and edit settings in the property grid.
    ("remedy-ui-widgets-board", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-board", "remedy-ui-widgets-data"),
    // Moment settings use the form pack's date and time pickers; the board's words are
    // templates filled the localization service's way.
    ("remedy-ui-widgets-board", "remedy-ui-widgets-forms"),
    ("remedy-ui-widgets-board", "remedy-ui-services-i18n"),
    ("remedy-ui-doc-rich", "remedy-ui-widgets-core"),
    ("remedy-ui-doc-code", "remedy-ui-widgets-core"),
    ("remedy-ui-automation", "remedy-ui-widgets-core"),
    ("remedy-ui-rui", "remedy-ui-widgets-core"),
    ("remedy-ui-automation", "remedy-ui-widgets-nav"),
    ("remedy-ui-doc-pages", "remedy-ui-widgets-core"),
    ("remedy-ui-viz", "remedy-ui-widgets-core"),
    ("remedy-ui-embed-webview", "remedy-ui-widgets-core"),
    ("remedy-ui-embed-gpu", "remedy-ui-widgets-core"),
    ("remedy-ui-widgets-music", "remedy-ui-viz"),
    ("remedy-ui-doc-rich", "remedy-ui-doc-code"),
    ("remedy-ui-media-anim", "remedy-ui-media-image"),
    // SVG is an image format: it plugs into the image loader.
    ("remedy-ui-media-svg", "remedy-ui-media-image"),
    // The sandboxed helper decodes with the image pack and is the loader's router.
    ("remedy-ui-media-isolated", "remedy-ui-media-image"),
    // ...and the animation pack's opener, playing animations decoded in the helper.
    ("remedy-ui-media-isolated", "remedy-ui-media-anim"),
    // The player shows its pictures in a video surface, and its transport bar is built from
    // the core widgets.
    ("remedy-ui-media-playback", "remedy-ui-media-video"),
    ("remedy-ui-media-playback", "remedy-ui-widgets-core"),
    // The update banner is a widget.
    ("remedy-ui-services-update", "remedy-ui-widgets-core"),
    // The spelling menu is built from the core menu widgets.
    ("remedy-ui-services-spell", "remedy-ui-widgets-core"),
    // The inspector and preview gallery are built from widgets, and previews render with
    // the theme pack's palettes and the bundled fonts.
    ("remedy-ui-devtools", "remedy-ui-widgets-core"),
    ("remedy-ui-devtools", "remedy-ui-widgets-forms"),
    ("remedy-ui-devtools", "remedy-ui-themes"),
    ("remedy-ui-devtools", "remedy-ui-fonts"),
    ("remedy-ui-devtools", "remedy-ui-fonts-cjk"),
    // The CJK faces are described the way the other bundled faces are.
    ("remedy-ui-fonts-cjk", "remedy-ui-fonts"),
    // Customization edits the theme pack's tokens and restyles with its sheet; its editors
    // are built from widgets (the color picker and shortcut recorder from the form pack, the
    // toolbar editor from navigation, whose commands it rebinds); text overrides are a layer
    // of the localization service's catalogs.
    ("remedy-ui-customize", "remedy-ui-themes"),
    ("remedy-ui-customize", "remedy-ui-widgets-core"),
    ("remedy-ui-customize", "remedy-ui-widgets-forms"),
    ("remedy-ui-customize", "remedy-ui-widgets-nav"),
    ("remedy-ui-customize", "remedy-ui-services-i18n"),
    // People's own screen layouts are the markup pack's, edited in the code editor; a theme's
    // background image goes through the image pack's loader (and its sandboxed decoder).
    ("remedy-ui-customize", "remedy-ui-rui"),
    ("remedy-ui-customize", "remedy-ui-doc-code"),
    ("remedy-ui-customize", "remedy-ui-media-image"),
    // Each script engine implements the language-neutral widget interface.
    ("remedy-ui-script-rhai", "remedy-ui-script"),
    ("remedy-ui-script-wasm", "remedy-ui-script"),
    // Effects take pictures (textures) and blend modes from the display list.
    ("remedy-ui-effects", "remedy-ui-paint"),
    // The compositor holds display lists and runs motion curves on its own thread.
    ("remedy-ui-compositor", "remedy-ui-paint"),
    ("remedy-ui-compositor", "remedy-ui-motion"),
    // The GPU renderer shares the CPU renderer's painter, and falls back to it.
    ("remedy-ui-render-gpu", "remedy-ui-render-cpu"),
    // Painters cache art they draw once as pictures (`rasterize`, `PaintCx::cached`).
    ("remedy-ui-view", "remedy-ui-render-cpu"),
    ("remedy-ui-bridge-windows", "remedy-ui-bridge-winit"),
    ("remedy-ui-bridge-macos", "remedy-ui-bridge-winit"),
    ("remedy-ui-bridge-linux", "remedy-ui-bridge-winit"),
    ("remedy-ui-bridge-android", "remedy-ui-bridge-winit"),
    // The command line and the MCP server both present the tools' logic.
    ("cargo-remedy", "remedy-tools"),
    ("remedy-mcp", "remedy-tools"),
    ("cargo-remedy", "remedy-templates"),
    ("remedy-tools", "remedy-templates"),
    ("cargo-remedy", "remedy-ui-build"),
];

/// Crates that may hold unsafe code, each in one `os` or `ffi` module.
const UNSAFE_BOUNDARY: &[&str] = &[
    "remedy-ui-render-gpu",
    "remedy-ui-bridge-windows",
    "remedy-ui-bridge-macos",
    "remedy-ui-bridge-linux",
    "remedy-ui-bridge-android",
    "remedy-ui-bridge-web",
    "remedy-ui-bridge-remedyos",
    "remedy-ui-bridge-ios",
    // Reads Authenticode signatures (WinVerifyTrust, CryptQueryObject) to verify updates.
    "remedy-ui-services-update",
    // Writes Windows shortcuts (IShellLink) for start-with-the-session entries.
    "remedy-ui-services-desktop",
    // Talks to the platform's spell checker (Windows ISpellChecker, macOS NSSpellChecker).
    "remedy-ui-services-spell",
    // Starts the sandboxed decoding helper: AppContainer, job objects and sections on
    // Windows; seccomp-bpf, Landlock, memfds and descriptor passing on Linux; Seatbelt on macOS.
    "remedy-ui-media-isolated",
    // Decodes and plays media through the platform (Media Foundation and WASAPI, GStreamer,
    // AVFoundation and Audio Queues, MediaCodec and AAudio).
    "remedy-ui-media-playback",
    // Hosts the platform's web view: WebView2's COM interfaces and child windows on Windows,
    // GTK and WebKitGTK through dlopen on Linux.
    "remedy-ui-embed-webview",
];

fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

struct Package {
    name: String,
    manifest: PathBuf,
    deps: Vec<String>,
    /// False for `publish = false`.
    publishable: bool,
    /// Publishing metadata fields that are missing.
    missing: Vec<&'static str>,
    /// `rust-version`, when set.
    rust_version: Option<String>,
}

/// The cargo-semver-checks release `cargo xtask semver` is written against
/// (`cargo install cargo-semver-checks --version 0.51.0 --locked`).
const SEMVER_CHECKS_VERSION: &str = "0.51.0";

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
        // `publish = false` is an empty list of registries.
        let publishable = p.get("publish").and_then(|v| v.as_array()).is_none_or(|a| !a.is_empty());
        let text = |k: &str| p.get(k).and_then(|v| v.as_str()).is_some_and(|s| !s.trim().is_empty());
        let list = |k: &str| p.get(k).and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty());
        let mut missing = Vec::new();
        for (k, ok) in [
            ("description", text("description")),
            ("license", text("license")),
            ("repository", text("repository")),
            ("keywords", list("keywords")),
            ("categories", list("categories")),
            ("readme", text("readme")),
        ] {
            if !ok {
                missing.push(k);
            }
        }
        let rust_version = p.get("rust_version").and_then(|v| v.as_str()).map(str::to_owned);
        pkgs.push(Package { name, manifest, deps, publishable, missing, rust_version });
    }
    Ok(pkgs)
}

/// RemedyUI's own crates: the engine's (`remedy-ui*`) and the tools in [`LAYERS`].
fn ours(name: &str) -> bool {
    name.starts_with("remedy-ui") || layer(name).is_some()
}

fn check_layers(pkgs: &[Package], problems: &mut Vec<String>) {
    for p in pkgs.iter().filter(|p| ours(&p.name)) {
        let Some(own) = layer(&p.name) else {
            problems.push(format!("{} has no layer; add it to LAYERS in xtask/src/main.rs", p.name));
            continue;
        };
        for d in p.deps.iter().filter(|d| ours(d)) {
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

/// Every crate that is published carries what crates.io shows and searches by.
fn check_metadata(pkgs: &[Package], problems: &mut Vec<String>) {
    for p in pkgs.iter().filter(|p| p.publishable) {
        if !p.missing.is_empty() {
            problems.push(format!(
                "{} is published but its Cargo.toml has no {} (or mark it publish = false)",
                p.name,
                p.missing.join(", ")
            ));
        }
    }
}

const USAGE: &str = "usage: cargo xtask check | docs-index | semver --baseline <git revision or tag> [cargo-semver-checks options] | msrv";

fn cargo_cmd() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

/// Runs cargo-semver-checks on every publishable crate against `baseline`.
fn semver(args: &[String]) -> Result<(), String> {
    let at = args.iter().position(|a| a == "--baseline").ok_or(USAGE)?;
    let baseline = args.get(at + 1).ok_or(USAGE)?;
    let extra: Vec<&String> =
        args.iter().enumerate().filter(|(i, _)| *i != at && *i != at + 1).map(|(_, a)| a).collect();
    let version = cargo_cmd()
        .args(["semver-checks", "--version"])
        .output()
        .map_err(|e| format!("could not run cargo semver-checks: {e}"))?;
    if !version.status.success() {
        return Err(format!(
            "cargo-semver-checks is not installed: cargo install cargo-semver-checks --version {SEMVER_CHECKS_VERSION} --locked"
        ));
    }
    let found = String::from_utf8_lossy(&version.stdout).trim().to_owned();
    if !found.ends_with(SEMVER_CHECKS_VERSION) {
        eprintln!("note: {found} is installed; these checks are written against {SEMVER_CHECKS_VERSION}");
    }
    let pkgs = packages()?;
    let published: Vec<&Package> = pkgs.iter().filter(|p| p.publishable).collect();
    println!("checking {} publishable crates against {baseline}", published.len());
    // cargo-semver-checks builds in `<target>/semver-checks/<a long project name>/target/...`.
    // On Windows that passes the linker's 260-character path limit from a deep checkout, so
    // there it builds under a short folder in the temporary directory instead
    // (REMEDY_SEMVER_TARGET_DIR chooses another).
    let target_dir = std::env::var_os("REMEDY_SEMVER_TARGET_DIR")
        .map(PathBuf::from)
        .or_else(|| cfg!(windows).then(|| std::env::temp_dir().join("rsv")));
    let mut cmd = cargo_cmd();
    if let Some(dir) = &target_dir {
        println!("building the API descriptions in {}", dir.display());
        cmd.env("CARGO_TARGET_DIR", dir);
    }
    // One run for the workspace builds every crate's rustdoc once per side; crates marked
    // publish = false are skipped by cargo-semver-checks itself.
    let status = cmd
        .args(["semver-checks", "check-release", "--workspace", "--baseline-rev", baseline])
        .args(extra)
        .status()
        .map_err(|e| format!("could not run cargo semver-checks: {e}"))?;
    if status.success() {
        println!("ok: no breaking change against {baseline}");
        Ok(())
    } else {
        Err(format!("cargo semver-checks found breaking changes against {baseline} (see above)"))
    }
}

/// Checks the workspace with the Rust version it declares.
fn msrv() -> Result<(), String> {
    let pkgs = packages()?;
    let mut versions: Vec<&str> = pkgs.iter().filter_map(|p| p.rust_version.as_deref()).collect();
    versions.sort_unstable();
    versions.dedup();
    let [version] = versions.as_slice() else {
        return Err(format!(
            "the crates declare different rust-versions: {versions:?}; keep one in [workspace.package]"
        ));
    };
    let rustup = |args: &[&str]| Command::new("rustup").args(args).env_remove("RUSTUP_TOOLCHAIN").status();
    let have = Command::new("rustup")
        .args(["run", version, "rustc", "--version"])
        .env_remove("RUSTUP_TOOLCHAIN")
        .output()
        .is_ok_and(|o| o.status.success());
    if !have {
        println!("installing Rust {version} with rustup");
        let ok = rustup(&["toolchain", "install", version, "--profile", "minimal"])
            .map_err(|e| format!("could not run rustup: {e}"))?;
        if !ok.success() {
            return Err(format!("rustup could not install Rust {version}"));
        }
    }
    println!("checking the workspace with Rust {version}");
    // A target directory of its own, so the stable toolchain's builds stay as they are.
    let status = Command::new("rustup")
        .args(["run", version, "cargo", "check", "--workspace", "--all-targets", "--all-features"])
        .args(["--target-dir", "target/msrv"])
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("CARGO")
        .env_remove("RUSTC")
        .env_remove("RUSTDOC")
        .status()
        .map_err(|e| format!("could not run cargo {version}: {e}"))?;
    if status.success() {
        println!("ok: the workspace builds with Rust {version}, its declared rust-version");
        Ok(())
    } else {
        Err(format!(
            "the workspace does not build with Rust {version}: raise rust-version (with the reason) or keep the code within it"
        ))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().cloned().unwrap_or_default();
    let tool = match task.as_str() {
        "semver" => Some(semver(args.get(1..).unwrap_or_default())),
        "msrv" => Some(msrv()),
        "docs-index" => Some(docs_index()),
        "check" => None,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Some(r) = tool {
        return match r {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
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
    check_metadata(&pkgs, &mut problems);
    check_docs_index(&mut problems);
    if problems.is_empty() {
        println!(
            "ok: {} crates follow the layer graph and the unsafe-code boundary, and published ones carry their metadata",
            pkgs.len()
        );
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
            publishable: true,
            missing: Vec::new(),
            rust_version: None,
        }
    }

    #[test]
    fn published_crates_need_their_metadata() {
        let mut p = pkg("remedy-ui-core", &[]);
        p.missing = vec!["keywords"];
        let mut out = Vec::new();
        check_metadata(&[p], &mut out);
        assert!(out.iter().any(|m| m.contains("keywords")), "{out:?}");
        let mut tool = pkg("xtask", &[]);
        tool.publishable = false;
        tool.missing = vec!["readme"];
        let mut out = Vec::new();
        check_metadata(&[tool], &mut out);
        assert!(out.is_empty(), "unpublished crates are not checked");
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

    #[test]
    fn the_tools_are_checked_too() {
        assert!(problems(&[pkg("remedy-mcp", &["remedy-tools", "remedy-ui-core"])]).is_empty());
        assert!(problems(&[pkg("cargo-remedy", &["remedy-tools"])]).is_empty());
        // Nothing builds on the tools, and the tools do not build on each other unlisted.
        assert_eq!(problems(&[pkg("remedy-ui", &["remedy-tools"])]).len(), 1);
        assert_eq!(problems(&[pkg("remedy-tools", &["remedy-mcp"])]).len(), 1);
    }
}
