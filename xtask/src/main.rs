//! Repository checks and release tools for RemedyUI.
//!
//! - `cargo xtask check`: the layer graph, the unsafe-code boundary, publishing metadata, the
//!   committed docs index, and `cargo vet --locked` (skipped, with a note, when cargo-vet is
//!   not installed).
//! - `cargo xtask semver --baseline <git revision or tag>`: every publishable crate's API
//!   against the baseline, with `cargo-semver-checks` ([`SEMVER_CHECKS_VERSION`]).
//! - `cargo xtask msrv`: the workspace builds with the Rust version it declares
//!   (`rust-version`), installing that toolchain with rustup when it is missing.
//!
//! - **Layers.** Every `remedy-ui-*` crate has a layer (docs/design/00-principles-and-modularity.md).
//!   A crate may depend on crates in lower layers; any crate missing from [`LAYERS`] fails the
//!   check. The parts of the merged crates ([`MODULES`]) keep their own layers: inside a crate
//!   a module may use modules of lower layers, plus the sideways edges in [`SIDEWAYS`].
//! - **Unsafe code.** Every crate inherits the workspace lints (`unsafe_code = "deny"`; deny
//!   rather than forbid, because forbid cannot be lifted even where it must be). Only the
//!   crates and modules in [`UNSAFE_BOUNDARY`] may contain `allow(unsafe_code)`, and only on a
//!   module named `os` or `ffi`; this check is what holds that line.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// What the developer tools know of the checkout: the same code `remedy-tools` reads a live
/// checkout with, so the committed index and `--docs-root` agree.
#[path = "../../crates/cargo-remedy/src/devkit/extract.rs"]
#[allow(dead_code, reason = "xtask uses the index writer; remedy-tools the readers")]
mod extract;

/// The committed docs index (zlib-compressed JSON), embedded by `remedy-tools`.
const DOCS_INDEX: &str = "crates/cargo-remedy/remedy-tools/data/docs-index.json.zz";

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

/// Each RemedyUI crate and its layer: a crate may depend only on crates in lower layers.
const LAYERS: &[(&str, u8)] = &[
    ("remedy-ui-core", 0),
    ("remedy-ui-build", 0),
    ("remedy-ui-macros", 1),
    ("remedy-ui-text", 1),
    ("remedy-ui-fonts", 1),
    ("remedy-ui-fonts-cjk", 2),
    ("remedy-ui-engine", 2),
    ("remedy-ui-bridge-web", 2),
    ("remedy-ui-platform", 3),
    ("remedy-ui-media", 4),
    ("remedy-ui-widgets", 5),
    ("remedy-ui-widgets-term", 6),
    ("remedy-ui-media-playback", 6),
    ("remedy-ui", 7),
    ("cargo-remedy", 8),
];

/// The parts of the merged crates and their layers: (crate, module, layer). Inside a crate a
/// module may use modules of lower layers, plus the sideways edges in [`SIDEWAYS`]. They are
/// the layers the parts had as crates of their own (docs/design/00-principles-and-modularity.md).
const MODULES: &[(&str, &str, u8)] = &[
    ("remedy-ui-core", "input", 2),
    ("remedy-ui-core", "a11y", 3),
    ("remedy-ui-core", "layout", 2),
    ("remedy-ui-core", "motion", 3),
    ("remedy-ui-core", "tree", 2),
    ("remedy-ui-core", "style", 2),
    ("remedy-ui-core", "host", 3),
    ("remedy-ui-core", "reactive", 1),
    ("remedy-ui-core", "testing", 3),
    ("remedy-ui-engine", "paint", 3),
    ("remedy-ui-engine", "view", 4),
    ("remedy-ui-engine", "compositor", 3),
    ("remedy-ui-engine", "render_cpu", 4),
    ("remedy-ui-engine", "render_gpu", 4),
    ("remedy-ui-engine", "effects", 3),
    ("remedy-ui-engine", "media_effects", 5),
    ("remedy-ui-engine", "script", 3),
    ("remedy-ui-engine", "script_lua", 3),
    ("remedy-ui-engine", "script_wasm", 3),
    ("remedy-ui-platform", "bridge_winit", 4),
    ("remedy-ui-platform", "bridge_windows", 4),
    ("remedy-ui-platform", "bridge_macos", 4),
    ("remedy-ui-platform", "bridge_linux", 4),
    ("remedy-ui-platform", "bridge_android", 4),
    ("remedy-ui-platform", "bridge_headless", 4),
    ("remedy-ui-platform", "services_desktop", 5),
    ("remedy-ui-platform", "services_net", 5),
    ("remedy-ui-platform", "services_i18n", 5),
    ("remedy-ui-platform", "services_webstorage", 5),
    ("remedy-ui-platform", "embed_webview", 5),
    ("remedy-ui-platform", "embed_gpu", 5),
    ("remedy-ui-media", "media_image", 5),
    ("remedy-ui-media", "media_anim", 5),
    ("remedy-ui-media", "media_video", 5),
    ("remedy-ui-media", "media_isolated", 5),
    ("remedy-ui-widgets", "base", 5),
    ("remedy-ui-widgets", "nav", 5),
    ("remedy-ui-widgets", "forms", 5),
    ("remedy-ui-widgets", "data", 5),
    ("remedy-ui-widgets", "extra", 5),
    ("remedy-ui-widgets", "stage", 5),
    ("remedy-ui-widgets", "music", 5),
    ("remedy-ui-widgets", "board", 5),
    ("remedy-ui-widgets", "viz", 5),
    ("remedy-ui-widgets", "themes", 5),
    ("remedy-ui-widgets", "customize", 5),
    ("remedy-ui-widgets", "rui", 5),
    ("remedy-ui-widgets", "doc_rich", 5),
    ("remedy-ui-widgets", "doc_code", 5),
    ("remedy-ui-widgets", "doc_pages", 5),
    ("remedy-ui-widgets", "devtools", 5),
    ("remedy-ui-widgets", "automation", 5),
    ("remedy-ui-widgets", "spell", 5),
    ("remedy-ui-widgets", "update", 5),
    ("cargo-remedy", "devkit", 1),
    ("cargo-remedy", "templates", 0),
    ("cargo-remedy", "mcp", 2),
];

/// Allowed uses between modules of one layer in one crate: (crate, module, the module it uses).
const SIDEWAYS: &[(&str, &str, &str)] = &[
    ("cargo-remedy", "devkit", "templates"),
    ("cargo-remedy", "mcp", "devkit"),
    ("remedy-ui-engine", "compositor", "paint"),
    ("remedy-ui-engine", "effects", "paint"),
    ("remedy-ui-engine", "render_gpu", "render_cpu"),
    ("remedy-ui-engine", "script_lua", "script"),
    ("remedy-ui-engine", "script_wasm", "script"),
    ("remedy-ui-engine", "view", "render_cpu"),
    ("remedy-ui-media", "media_anim", "media_image"),
    ("remedy-ui-media", "media_isolated", "media_anim"),
    ("remedy-ui-media", "media_isolated", "media_image"),
    ("remedy-ui-platform", "bridge_android", "bridge_winit"),
    ("remedy-ui-platform", "bridge_linux", "bridge_winit"),
    ("remedy-ui-platform", "bridge_macos", "bridge_winit"),
    ("remedy-ui-platform", "bridge_windows", "bridge_winit"),
    ("remedy-ui-widgets", "automation", "base"),
    ("remedy-ui-widgets", "automation", "nav"),
    ("remedy-ui-widgets", "board", "base"),
    ("remedy-ui-widgets", "board", "data"),
    ("remedy-ui-widgets", "board", "forms"),
    ("remedy-ui-widgets", "customize", "base"),
    ("remedy-ui-widgets", "customize", "doc_code"),
    ("remedy-ui-widgets", "customize", "forms"),
    ("remedy-ui-widgets", "customize", "nav"),
    ("remedy-ui-widgets", "customize", "rui"),
    ("remedy-ui-widgets", "customize", "themes"),
    ("remedy-ui-widgets", "data", "base"),
    ("remedy-ui-widgets", "devtools", "base"),
    ("remedy-ui-widgets", "devtools", "forms"),
    ("remedy-ui-widgets", "devtools", "themes"),
    ("remedy-ui-widgets", "doc_code", "base"),
    ("remedy-ui-widgets", "doc_pages", "base"),
    ("remedy-ui-widgets", "doc_rich", "base"),
    ("remedy-ui-widgets", "doc_rich", "doc_code"),
    ("remedy-ui-widgets", "extra", "base"),
    ("remedy-ui-widgets", "extra", "viz"),
    ("remedy-ui-widgets", "forms", "base"),
    ("remedy-ui-widgets", "music", "base"),
    ("remedy-ui-widgets", "music", "viz"),
    ("remedy-ui-widgets", "nav", "base"),
    ("remedy-ui-widgets", "rui", "base"),
    ("remedy-ui-widgets", "spell", "base"),
    ("remedy-ui-widgets", "stage", "viz"),
    ("remedy-ui-widgets", "update", "base"),
    ("remedy-ui-widgets", "viz", "base"),
];

/// Where unsafe code may be, as folders under `crates/`: whole crates that talk to a platform,
/// and the modules of merged crates that do. Inside them it sits in one `os` or `ffi` module.
const UNSAFE_BOUNDARY: &[&str] = &[
    "remedy-ui-engine/src/render_gpu/",
    "remedy-ui-engine/examples/render_gpu/",
    "remedy-ui-engine/tests/render_gpu/",
    "remedy-ui-platform/src/bridge_windows/",
    "remedy-ui-platform/examples/bridge_windows/",
    "remedy-ui-platform/tests/bridge_windows/",
    "remedy-ui-platform/src/bridge_macos/",
    "remedy-ui-platform/examples/bridge_macos/",
    "remedy-ui-platform/tests/bridge_macos/",
    "remedy-ui-platform/src/bridge_linux/",
    "remedy-ui-platform/examples/bridge_linux/",
    "remedy-ui-platform/tests/bridge_linux/",
    "remedy-ui-platform/src/bridge_android/",
    "remedy-ui-platform/examples/bridge_android/",
    "remedy-ui-platform/tests/bridge_android/",
    "remedy-ui-bridge-web/",
    "remedy-ui-bridge-remedyos/",
    "remedy-ui-bridge-ios/",
    "remedy-ui-widgets/src/update/",
    "remedy-ui-widgets/examples/update/",
    "remedy-ui-widgets/tests/update/",
    "remedy-ui-platform/src/services_desktop/",
    "remedy-ui-platform/examples/services_desktop/",
    "remedy-ui-platform/tests/services_desktop/",
    "remedy-ui-widgets/src/spell/",
    "remedy-ui-widgets/examples/spell/",
    "remedy-ui-widgets/tests/spell/",
    "remedy-ui-media/src/media_isolated/",
    "remedy-ui-media/examples/media_isolated/",
    "remedy-ui-media/tests/media_isolated/",
    "remedy-ui-media-playback/",
    "remedy-ui-platform/src/embed_webview/",
    "remedy-ui-platform/examples/embed_webview/",
    "remedy-ui-platform/tests/embed_webview/",
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
            if dl >= own {
                problems.push(format!(
                    "{} (L{own}) must not depend on {d} (L{dl}); see docs/design/00-principles-and-modularity.md",
                    p.name
                ));
            }
        }
    }
}

/// The modules of merged crates keep the layers they had as crates: a file under
/// `src/<module>/` may name `crate::<other>` only when the other module is in a lower layer
/// or the edge is in [`SIDEWAYS`].
fn check_modules(pkgs: &[Package], problems: &mut Vec<String>) {
    for p in pkgs {
        let own: Vec<(&str, u8)> =
            MODULES.iter().filter(|(c, ..)| *c == p.name).map(|(_, m, l)| (*m, *l)).collect();
        let Some(dir) = p.manifest.parent() else { continue };
        for (module, layer) in &own {
            let mut files = Vec::new();
            rust_files(&dir.join("src").join(module), &mut files);
            for f in files {
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                for (i, line) in text.lines().enumerate() {
                    // Comments may name anything: a link is not a use.
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    for rest in line.split("crate::").skip(1) {
                        let used: String =
                            rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                        let Some((_, other)) = own.iter().find(|(m, _)| *m == used && m != module) else {
                            continue;
                        };
                        let sideways = SIDEWAYS.contains(&(p.name.as_str(), module, used.as_str()));
                        if other > layer || (other == layer && !sideways) {
                            problems.push(format!(
                                "{}:{}: `{module}` (L{layer}) must not use `{used}` (L{other}); see docs/design/00-principles-and-modularity.md",
                                f.display(),
                                i + 1
                            ));
                        }
                    }
                }
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
        for f in files {
            let shown = f.to_string_lossy().replace('\\', "/");
            let boundary = UNSAFE_BOUNDARY.iter().any(|b| shown.contains(&format!("crates/{b}")));
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

/// Every dependency must be audited or exempted in `supply-chain/` (docs/dependencies.md).
/// `--locked` uses the committed `imports.lock`, so the check reads no network. Machines
/// without cargo-vet skip it and say so; it is not a pass.
fn check_vet(problems: &mut Vec<String>) {
    let installed = cargo_cmd()
        .args(["vet", "--version"])
        .current_dir(repo_root())
        .output()
        .is_ok_and(|o| o.status.success());
    if !installed {
        eprintln!(
            "note: skipping the supply-chain audits: cargo-vet is not installed (cargo install cargo-vet --locked)"
        );
        return;
    }
    match cargo_cmd().args(["vet", "--locked"]).current_dir(repo_root()).status() {
        Ok(s) if s.success() => {}
        Ok(_) => problems.push(
            "cargo vet --locked failed (see above): audit the new or changed dependencies, or record them with `cargo vet` (see docs/dependencies.md)".into(),
        ),
        Err(e) => problems.push(format!("could not run cargo vet: {e}")),
    }
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
    check_modules(&pkgs, &mut problems);
    check_unsafe(&pkgs, &mut problems);
    check_metadata(&pkgs, &mut problems);
    check_docs_index(&mut problems);
    check_vet(&mut problems);
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
