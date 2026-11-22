//! `cargo remedy package --target web`: any RemedyUI app as an installable page (a PWA).
//!
//! 1. The app is built for `wasm32-unknown-unknown` with remedy-ui's `web` feature: its
//!    binary (whose `main` calls `remedy_ui::run`, which starts the app in the page and
//!    returns), or an example (`--example`). A release is built with the `remedy-web`
//!    profile, made for size (the release profile with `opt-level = "s"`, fat LTO, one
//!    codegen unit, symbols stripped); an app that defines `[profile.remedy-web]` in its
//!    workspace's `Cargo.toml` gets its own instead.
//! 2. `wasm-bindgen --target web` writes the module and its JavaScript glue (`app.js`,
//!    `app_bg.wasm`). The CLI must be the version of `wasm-bindgen` the app's `Cargo.lock`
//!    resolved; anything else makes glue that does not fit the module. A release module is
//!    then optimized for size by Binaryen's `wasm-opt -Os` when it is on the `PATH` (or
//!    `WASM_OPT` names it; `WASM_OPT=off` skips it).
//! 3. The files the module fetches when it needs them (the fallback fonts, which a page does
//!    not build in) are copied beside the page, as the marks the module carries for them name
//!    them (`remedy_ui_fonts::asset_mark!`); the marks, which hold this machine's paths, are
//!    then blanked in the module.
//! 4. The page (`index.html`, with the `#remedy` element the first window mounts in), a web
//!    app manifest, the icons (`[web] icon` in `remedy.toml`, SVG or PNG, else the app's
//!    PNG icon, else the default one) and a service worker that caches exactly this build,
//!    so the page starts offline and a new build replaces the old cache. The fetched-later
//!    files are cached as they are first used and in the background once the page runs, so
//!    they are there offline too without holding up the first start.
//! 5. The folder is zipped into `dist` for uploading.
//!
//! `remedy.toml`'s `[web]` section names the short name, the theme color for the browser's
//! chrome, and the icon; `[app]` gives the name, id and description.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::{Project, cargo, status};

const TARGET: &str = "wasm32-unknown-unknown";
const INDEX: &str = include_str!("../web/index.html");
const MANIFEST: &str = include_str!("../web/manifest.webmanifest");
const WORKER: &str = include_str!("../web/sw.js");

/// What to build, from the command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// An example of the package instead of its binary.
    pub example: Option<String>,
    /// A debug build (quicker to make, larger and slower) instead of a release.
    pub dev: bool,
    /// Where the page goes, instead of `target/remedy/web/<name>`.
    pub out: Option<PathBuf>,
}

impl Options {
    /// The options in `args` (`--example <name>`, `--dev`, `--out <dir>`).
    pub fn parse(args: &[String]) -> Options {
        Options {
            example: crate::flag_value(args, "--example"),
            dev: args.iter().any(|a| a == "--dev"),
            out: crate::flag_value(args, "--out").map(PathBuf::from),
        }
    }
}

/// Builds the page; returns the page's folder and its zip.
pub fn package(
    p: &Project,
    opts: &Options,
    dist: &Path,
    notes: &mut Vec<String>,
) -> Result<Vec<PathBuf>, String> {
    if !target_installed() {
        return Err(format!("the {TARGET} target is not installed: rustup target add {TARGET}"));
    }
    let have = wasm_bindgen_version().ok_or(
        "the wasm-bindgen CLI was not found: cargo install wasm-bindgen-cli --version <the version in Cargo.lock>",
    )?;

    // 1. The module.
    let mut build = Command::new(cargo());
    build.current_dir(&p.root).args(["build", "--target", TARGET]);
    if !opts.dev {
        build.args(["--profile", PROFILE]);
        if !defines_profile(&p.root) {
            for (key, value) in SIZE_PROFILE {
                build.args(["--config", &format!("profile.{PROFILE}.{key}={value}")]);
            }
        }
    }
    match &opts.example {
        Some(e) => build.args(["--example", e]),
        None => build.args(["--bin", &p.bin]),
    };
    if p.uses_remedy_ui {
        build.args(["--features", "remedy-ui/web"]);
    }
    status(&mut build)?;
    let dir = p.target_dir.join(TARGET).join(if opts.dev { "debug" } else { PROFILE });
    let name = opts.example.clone().unwrap_or_else(|| p.bin.clone());
    let wasm = module_path(&dir, &name, opts.example.is_some())
        .ok_or_else(|| format!("no {name}.wasm under {}", dir.display()))?;

    // 2. The glue, from the CLI that matches the app's wasm-bindgen.
    let want = locked_version(&p.root, "wasm-bindgen").ok_or(
        "the app does not use wasm-bindgen: it needs remedy-ui with the `web` feature (its [dependencies] or a target table)",
    )?;
    if want != have {
        return Err(format!(
            "the wasm-bindgen CLI is {have}, but the app's Cargo.lock has wasm-bindgen {want}: cargo install wasm-bindgen-cli --version {want}"
        ));
    }
    let out = opts.out.clone().unwrap_or_else(|| p.target_dir.join("remedy").join("web").join(&name));
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    status(
        Command::new("wasm-bindgen")
            .args(["--target", "web", "--no-typescript", "--out-name", "app", "--out-dir"])
            .arg(&out)
            .arg(&wasm),
    )?;
    let module = out.join("app_bg.wasm");
    let read = |path: &Path| std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()));
    let built = read(&wasm)?;
    let bound = read(&module)?;
    let blanked = blank_marks(&bound);
    if blanked != bound {
        std::fs::write(&module, &blanked).map_err(|e| format!("{}: {e}", module.display()))?;
    }
    if !opts.dev {
        optimize(&module, notes)?;
    }

    // 3. What the module fetches later.
    let mut later: Vec<String> = Vec::new();
    for (served, source) in assets(&built)? {
        let to = out.join(&served);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::copy(&source, &to).map_err(|e| format!("{} -> {}: {e}", source.display(), to.display()))?;
        later.push(served);
    }

    // 4. The page around it.
    let icons = icons(p, notes)?;
    let mut files: Vec<String> = vec!["app.js".into(), "app_bg.wasm".into()];
    for (file, data) in page_files(p, &icons) {
        std::fs::write(out.join(&file), data).map_err(|e| format!("{}: {e}", out.join(&file).display()))?;
        files.push(file);
    }
    let mut contents = Vec::new();
    for f in files.iter().chain(&later) {
        contents.push(std::fs::read(out.join(f)).map_err(|e| format!("{}: {e}", out.join(f).display()))?);
    }
    std::fs::write(out.join("sw.js"), service_worker(&p.id, &files, &later, &contents))
        .map_err(|e| format!("{}: {e}", out.join("sw.js").display()))?;
    files.push("sw.js".into());
    files.extend(later);

    // 5. A zip of the page.
    let zip_path = dist.join(format!("{}_{}_web.zip", name, p.version));
    write_zip(&zip_path, &out, &files)?;
    notes.push(format!(
        "serve {} over HTTPS (or localhost) to install it: service workers need a secure context",
        out.display()
    ));
    Ok(vec![out, zip_path])
}

/// The profile a release page is built with.
const PROFILE: &str = "remedy-web";

/// The `remedy-web` profile, for size, when the app defines none: the release profile with
/// everything that makes a module smaller.
const SIZE_PROFILE: [(&str, &str); 5] = [
    ("inherits", "\"release\""),
    ("opt-level", "\"s\""),
    ("lto", "\"fat\""),
    ("codegen-units", "1"),
    ("strip", "true"),
];

/// True when a `Cargo.toml` at `root` or above (the workspace's) defines the
/// `remedy-web` profile itself.
fn defines_profile(root: &Path) -> bool {
    root.ancestors().map(|d| d.join("Cargo.toml")).filter(|p| p.is_file()).any(|p| {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|s| s.parse::<toml::Table>().ok())
            .is_some_and(|t| t.get("profile").and_then(|p| p.get(PROFILE)).is_some())
    })
}

/// Runs `wasm-opt -Os` on `module` in place: the one on the `PATH`, or `WASM_OPT`'s (`off`
/// skips it). Without one the module stays as wasm-bindgen wrote it, with a note.
fn optimize(module: &Path, notes: &mut Vec<String>) -> Result<(), String> {
    let tool = std::env::var("WASM_OPT").unwrap_or_else(|_| "wasm-opt".into());
    if tool == "off" {
        return Ok(());
    }
    if Command::new(&tool).arg("--version").output().is_err() {
        notes.push(
            "wasm-opt (Binaryen) was not found, so the module is not optimized for size further: install Binaryen, or set WASM_OPT to its wasm-opt".into(),
        );
        return Ok(());
    }
    status(Command::new(&tool).arg("-Os").arg(module).arg("-o").arg(module))
}

/// Starts and ends the mark a module carries for each file it fetches when it needs it
/// (`remedy_ui_fonts::asset_mark!`): `\u{1}remedy-asset:<served>\t<source>\u{1}`.
const MARK: &[u8] = b"\x01remedy-asset:";

/// Where each mark in `module` is, as a byte range, with what it says.
fn marks(module: &[u8]) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = module.get(at..).and_then(|rest| rest.windows(MARK.len()).position(|w| w == MARK)) {
        let start = at + found;
        let body = start + MARK.len();
        let Some(len) = module.get(body..).and_then(|rest| rest.iter().position(|&b| b == 1)) else { break };
        let end = body + len + 1;
        out.push((start..end, String::from_utf8_lossy(module.get(body..body + len).unwrap_or_default()).into_owned()));
        at = end;
    }
    out
}

/// The files `module` fetches when it needs them, from its marks: where each is served (a
/// path beside the page) and the file it is copied from.
pub fn assets(module: &[u8]) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for (_, line) in marks(module) {
        let (served, source) = line.split_once('\t').ok_or_else(|| format!("not an asset mark: {line}"))?;
        if served.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
            return Err(format!("an asset may not be served at {served}"));
        }
        if !out.iter().any(|(s, _)| s == served) {
            out.push((served.to_owned(), PathBuf::from(source)));
        }
    }
    Ok(out)
}

/// `module` with every mark's bytes zeroed, so the shipped module holds no paths of this
/// machine; its size and layout stay the same.
pub fn blank_marks(module: &[u8]) -> Vec<u8> {
    let mut out = module.to_vec();
    for (range, _) in marks(module) {
        if let Some(bytes) = out.get_mut(range) {
            bytes.fill(0);
        }
    }
    out
}

/// The module cargo wrote for `name` in `dir`.
fn module_path(dir: &Path, name: &str, example: bool) -> Option<PathBuf> {
    let base = if example { dir.join("examples") } else { dir.to_path_buf() };
    [name.to_owned(), name.replace('-', "_")]
        .into_iter()
        .map(|n| base.join(format!("{n}.wasm")))
        .find(|p| p.is_file())
}

/// True when the standard library for the web target is installed.
fn target_installed() -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let Ok(out) = Command::new(rustc).args(["--print", "sysroot"]).output() else { return false };
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Path::new(&sysroot).join("lib").join("rustlib").join(TARGET).join("lib").is_dir()
}

/// The installed wasm-bindgen CLI's version.
fn wasm_bindgen_version() -> Option<String> {
    let out = Command::new("wasm-bindgen").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    text.split_whitespace().nth(1).map(str::to_owned)
}

/// The version of `package` in the Cargo.lock that governs the app at `root` (its own, or
/// its workspace's).
pub fn locked_version(root: &Path, package: &str) -> Option<String> {
    let lock = root.ancestors().map(|d| d.join("Cargo.lock")).find(|p| p.is_file())?;
    let parsed: toml::Table = std::fs::read_to_string(lock).ok()?.parse().ok()?;
    parsed.get("package")?.as_array()?.iter().find_map(|p| {
        (p.get("name")?.as_str()? == package).then(|| p.get("version")?.as_str().map(str::to_owned))?
    })
}

/// The page's icons.
#[derive(Clone, Debug, PartialEq)]
pub enum Icons {
    /// An SVG, at any size.
    Svg(Vec<u8>),
    /// PNGs at 192 and 512 pixels.
    Png {
        /// 192 × 192.
        small: Vec<u8>,
        /// 512 × 512.
        large: Vec<u8>,
    },
}

fn icons(p: &Project, notes: &mut Vec<String>) -> Result<Icons, String> {
    if let Some(path) = &p.web_icon
        && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        return std::fs::read(path).map(Icons::Svg).map_err(|e| format!("icon {}: {e}", path.display()));
    }
    let set = match &p.web_icon {
        Some(path) => crate::ico::IconSet::new(vec![crate::ico::read_png_file(path)?], p.icon_pixel_art)?,
        None => crate::platforms::icon_set(p, notes)?,
    };
    Ok(Icons::Png {
        small: crate::ico::write_png(&set.at(192))?,
        large: crate::ico::write_png(&set.at(512))?,
    })
}

/// Text for HTML.
fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A JSON (and JavaScript) string literal.
fn json(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

/// `index.html`, `manifest.webmanifest` and the icons, with their names.
pub fn page_files(p: &Project, icons: &Icons) -> Vec<(String, Vec<u8>)> {
    let (links, list, mut files) = match icons {
        Icons::Svg(svg) => (
            r#"<link rel="icon" href="icon.svg" type="image/svg+xml">"#.to_owned(),
            serde_json::json!([{ "src": "icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" }]),
            vec![("icon.svg".to_owned(), svg.clone())],
        ),
        Icons::Png { small, large } => (
            "<link rel=\"icon\" href=\"icon-192.png\" type=\"image/png\">\n<link rel=\"apple-touch-icon\" href=\"icon-192.png\">"
                .to_owned(),
            serde_json::json!([
                { "src": "icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
                { "src": "icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" }
            ]),
            vec![("icon-192.png".to_owned(), small.clone()), ("icon-512.png".to_owned(), large.clone())],
        ),
    };
    let short = p.web_short_name.clone().unwrap_or_else(|| p.name.clone());
    let description = if p.description.is_empty() { p.name.clone() } else { p.description.clone() };
    let index = INDEX
        .replace("@ICON_LINKS@", &links)
        .replace("@NAME_JS@", &json(&p.name))
        .replace("@NAME@", &html(&p.name))
        .replace("@DESCRIPTION@", &html(&description))
        .replace("@THEME@", &html(&p.web_theme));
    let manifest = MANIFEST
        .replace("@ICONS@", &list.to_string())
        .replace("@ID@", &json(&format!("./?app={}", p.id)))
        .replace("@NAME@", &json(&p.name))
        .replace("@SHORT@", &json(&short))
        .replace("@DESCRIPTION@", &json(&description))
        .replace("@THEME@", &json(&p.web_theme));
    files.insert(0, ("manifest.webmanifest".into(), manifest.into_bytes()));
    files.insert(0, ("index.html".into(), index.into_bytes()));
    files
}

/// The service worker for a build of `files` and the files fetched `later`, with `contents`
/// (of both, in that order): its cache is named after the app and the build's contents, so a
/// new build replaces the old cache and other apps on the same site keep theirs. `files` are
/// cached as it installs; `later` as each is first fetched and in the background once it
/// runs, so they hold up nothing.
pub fn service_worker(id: &str, files: &[String], later: &[String], contents: &[Vec<u8>]) -> String {
    // FNV-1a over every file, in order: a name for this build's cache.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, data) in files.iter().chain(later).zip(contents) {
        for b in name.bytes().chain(data.iter().copied()) {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    let prefix = format!("remedy-{id}-");
    let mut list: Vec<String> = vec![json("./")];
    list.extend(files.iter().map(|f| json(&format!("./{f}"))));
    let later: Vec<String> = later.iter().map(|f| json(&format!("./{f}"))).collect();
    WORKER
        .replace("@CACHE@", &json(&format!("{prefix}{hash:016x}")))
        .replace("@PREFIX@", &json(&prefix))
        .replace("@FILES@", &format!("[{}]", list.join(", ")))
        .replace("@LATER@", &format!("[{}]", later.join(", ")))
}

fn write_zip(path: &Path, dir: &Path, files: &[String]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for f in files {
        let data = std::fs::read(dir.join(f)).map_err(|e| format!("{}: {e}", dir.join(f).display()))?;
        zip.start_file(f.as_str(), opts).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let mut p = crate::project::tests::sample();
        p.name = "Notes <&> \"Plus\"".into();
        p.web_short_name = Some("Notes".into());
        p
    }

    #[test]
    fn the_page_carries_the_app_with_names_escaped() {
        let p = project();
        let files = page_files(&p, &Icons::Png { small: b"s".to_vec(), large: b"l".to_vec() });
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["index.html", "manifest.webmanifest", "icon-192.png", "icon-512.png"]);
        let file =
            |files: &[(String, Vec<u8>)], i: usize| files.get(i).map(|(_, d)| d.clone()).unwrap_or_default();
        let at = |v: &serde_json::Value, path: &str| {
            v.pointer(path).and_then(serde_json::Value::as_str).map(str::to_owned).unwrap_or_default()
        };
        let index = String::from_utf8_lossy(&file(&files, 0)).to_string();
        assert!(index.contains("<title>Notes &lt;&amp;&gt; &quot;Plus&quot;</title>"), "{index}");
        assert!(index.contains(r#"<div id="remedy"></div>"#) && index.contains("icon-192.png"), "{index}");
        assert!(!index.contains('@'), "every placeholder filled: {index}");
        let manifest: serde_json::Value = serde_json::from_slice(&file(&files, 1)).unwrap_or_default();
        assert_eq!(at(&manifest, "/name"), "Notes <&> \"Plus\"");
        assert_eq!(at(&manifest, "/short_name"), "Notes");
        assert_eq!(at(&manifest, "/theme_color"), p.web_theme.as_str());
        assert_eq!(at(&manifest, "/icons/1/sizes"), "512x512");

        let svg = page_files(&p, &Icons::Svg(b"<svg/>".to_vec()));
        assert_eq!(svg.get(2).map(|(n, _)| n.as_str()), Some("icon.svg"));
        let manifest: serde_json::Value = serde_json::from_slice(&file(&svg, 1)).unwrap_or_default();
        assert_eq!(at(&manifest, "/icons/0/type"), "image/svg+xml");
    }

    #[test]
    fn the_worker_caches_exactly_this_build() {
        let files = vec!["index.html".to_owned(), "app.js".to_owned()];
        let later = vec!["fonts/NotoSansSC-Regular.woff2".to_owned()];
        let a = service_worker("app.remedy.notes", &files, &later, &[b"one".to_vec(), b"two".to_vec(), b"f".to_vec()]);
        let b = service_worker("app.remedy.notes", &files, &later, &[b"one".to_vec(), b"two".to_vec(), b"g".to_vec()]);
        assert!(a.contains(r#"const FILES = ["./", "./index.html", "./app.js"];"#), "{a}");
        assert!(a.contains(r#"const LATER = ["./fonts/NotoSansSC-Regular.woff2"];"#), "{a}");
        assert!(a.contains(r#"k.startsWith("remedy-app.remedy.notes-")"#), "{a}");
        assert_ne!(a, b, "a new build is a new cache");
        assert!(!a.contains('@'), "{a}");
    }

    #[test]
    fn the_modules_assets_are_found_by_their_marks_and_blanked() {
        let mut m = b"\0asm\x01\0\0\0data".to_vec();
        for line in ["fonts/A.ttf\t/src/fonts/A.ttf", "fonts/B.woff2\t/src/B.woff2", "fonts/A.ttf\t/src/fonts/A.ttf"] {
            m.extend_from_slice(b"\x01remedy-asset:");
            m.extend_from_slice(line.as_bytes());
            m.extend_from_slice(b"\x01 more");
        }
        let got = assets(&m).unwrap_or_default();
        assert_eq!(
            got,
            [("fonts/A.ttf".to_owned(), PathBuf::from("/src/fonts/A.ttf")), ("fonts/B.woff2".to_owned(), PathBuf::from("/src/B.woff2"))]
        );
        let b = blank_marks(&m);
        assert_eq!(b.len(), m.len(), "the layout stays");
        assert!(assets(&b).is_ok_and(|a| a.is_empty()) && !b.windows(4).any(|w| w == b"/src"), "gone");
        assert!(b.ends_with(b"\0 more"));
        assert!(assets(b"\x01remedy-asset:../etc/passwd\t/x\x01").is_err(), "nothing is served outside the page");
    }

    #[test]
    fn options_and_lock_versions_read_back() {
        let args: Vec<String> =
            ["--target", "web", "--example", "web-demo", "--dev"].map(String::from).to_vec();
        let o = Options::parse(&args);
        assert_eq!(o.example.as_deref(), Some("web-demo"));
        assert!(o.dev && o.out.is_none());
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(locked_version(&root, "toml").is_some(), "the workspace lock is found from a member");
        assert_eq!(locked_version(&root, "no-such-package-anywhere"), None);
    }
}
