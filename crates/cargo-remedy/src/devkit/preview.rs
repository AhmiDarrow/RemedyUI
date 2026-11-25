//! Previews of an app's UI, rendered headlessly in a subprocess: a PNG, the window as a
//! screen reader meets it, and the accessibility audit.
//!
//! An app exposes what to preview as plain functions in its library, `pub fn home(ui: &Ui)
//! -> El` (the templates' `home` is one, and so is every public `#[preview]` function, which
//! [`list`] finds). To render one, the tools write a small harness
//! crate (in the app's `target/remedy-tools/preview/<crate>`) that depends on the app and on
//! the same `remedy-ui` with its `devtools` feature, build it with Cargo into the app's own
//! target folder (so it shares the app's compiled dependencies) and run it. The harness
//! renders with the devtools' `Preview` (the CPU renderer, deterministic pixels: the same as
//! golden images) and writes the PNG and a JSON report. A function that does not compile or
//! panics comes back as an error with the compiler's or the panic's message.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::devkit::process::{cargo, compile_errors, run, target_dir};
use crate::devkit::{Result, ToolError, Workspace};

/// What to render.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct PreviewInput {
    /// The app's folder (its Cargo.toml), inside the workspace.
    pub app: String,
    /// A public function of the app's library building the UI to preview, `fn(&Ui) -> El`,
    /// by its path in the crate: `home`, `screens::settings`.
    pub function: String,
    /// Window width in logical pixels (400 when absent).
    #[serde(default)]
    pub width: Option<f32>,
    /// Window height in logical pixels (300 when absent).
    #[serde(default)]
    pub height: Option<f32>,
    /// Device pixels per logical pixel (1 when absent; 2 for a high-density screen).
    #[serde(default)]
    pub scale: Option<f32>,
    /// `dark` (the default), `light`, or a Remedy palette id (`forest`, `neutral`, `light`,
    /// `deutan`, …).
    #[serde(default)]
    pub theme: Option<String>,
    /// A BCP 47 locale (`en-US` when absent). Right-to-left ones (`ar-EG`, `he`) lay out
    /// mirrored; the pseudo-locales `en-XA` (accented, longer) and `ar-XB` (forced right to
    /// left) stress layouts before any translation exists.
    #[serde(default)]
    pub locale: Option<String>,
    /// Text scale (1 when absent; 1.5 and 2 check large text).
    #[serde(default)]
    pub text_scale: Option<f32>,
    /// Stop building and rendering after this many seconds (no limit when absent).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

impl PreviewInput {
    /// Function `function` of the app in `app`, at the defaults (400 × 300, dark, en-US).
    pub fn new(app: impl Into<String>, function: impl Into<String>) -> Self {
        PreviewInput { app: app.into(), function: function.into(), ..Default::default() }
    }
}

/// A node as a screen reader meets it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ReaderNode {
    /// Its accessibility node id.
    pub id: u64,
    /// How deep it is.
    pub depth: usize,
    /// What a screen reader says on reaching it.
    pub announcement: String,
    /// The problems found on it.
    pub problems: Vec<String>,
}

/// An accessibility problem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Problem {
    /// The element's accessibility node id.
    pub element: u64,
    /// What is wrong, with where the element was built.
    pub message: String,
}

/// A rendered preview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct PreviewOutput {
    /// The PNG (straight-alpha RGBA). Not serialized: see `png_file`.
    #[serde(skip)]
    pub png: Vec<u8>,
    /// Where the PNG was also written, when asked.
    #[serde(default)]
    pub png_file: Option<String>,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// The palette it was drawn in.
    pub theme: String,
    /// The palette is dark.
    pub dark: bool,
    /// The locale.
    pub locale: String,
    /// The locale lays out right to left.
    pub rtl: bool,
    /// The window as a screen reader meets it, in reading order.
    pub reader: Vec<ReaderNode>,
    /// Every accessibility problem the audit found (unnamed controls, low contrast, small
    /// targets, pointer-only controls, content past the window's edge, unreachable controls).
    pub problems: Vec<Problem>,
}

impl PreviewOutput {
    /// The screen reader's view as indented lines, with problems marked.
    pub fn reader_text(&self) -> String {
        let mut out = String::new();
        for n in &self.reader {
            out.push_str(&"  ".repeat(n.depth));
            out.push_str(&n.announcement);
            out.push_str(&format!("  [#{}]", n.id));
            for p in &n.problems {
                out.push_str(&format!("\n{}  ! {p}", "  ".repeat(n.depth)));
            }
            out.push('\n');
        }
        out
    }
}

/// Where the harness gets the facade: the app's own `remedy-ui` dependency, as written.
fn facade_source(app: &Path, manifest: &toml::Table) -> Result<toml::Table> {
    let find = |t: &toml::Table| -> Option<toml::Value> {
        for section in ["dependencies", "dev-dependencies"] {
            if let Some(v) = t.get(section).and_then(|d| d.get("remedy-ui")) {
                return Some(v.clone());
            }
        }
        None
    };
    let Some(dep) = find(manifest) else {
        return Err(ToolError::new(format!(
            "{} does not depend on remedy-ui",
            app.join("Cargo.toml").display()
        )));
    };
    let (dep, base) = if dep.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
        let (root, table) = workspace_manifest(app)?;
        let inherited = table
            .get("workspace")
            .and_then(|w| w.get("dependencies"))
            .and_then(|d| d.get("remedy-ui"))
            .cloned()
            .ok_or_else(|| {
                ToolError::new("remedy-ui comes from the workspace, whose manifest does not list it")
            })?;
        (inherited, root)
    } else {
        (dep, app.to_path_buf())
    };
    let mut out = toml::Table::new();
    match dep {
        toml::Value::String(v) => {
            out.insert("version".into(), toml::Value::String(v));
        }
        toml::Value::Table(t) => {
            for (k, v) in t {
                match k.as_str() {
                    "path" => {
                        let p = base.join(v.as_str().unwrap_or_default());
                        let p = std::fs::canonicalize(&p)
                            .map(|c| crate::devkit::workspace::plain(&c))
                            .unwrap_or(p);
                        out.insert(k, toml::Value::String(p.display().to_string().replace('\\', "/")));
                    }
                    "git" | "branch" | "tag" | "rev" | "version" | "registry" | "package" => {
                        out.insert(k, v);
                    }
                    _ => {}
                }
            }
        }
        _ => return Err(ToolError::new("the remedy-ui dependency is neither a version nor a table")),
    }
    out.insert("default-features".into(), toml::Value::Boolean(false));
    out.insert(
        "features".into(),
        toml::Value::Array(vec![
            toml::Value::String("devtools".into()),
            toml::Value::String("widgets".into()),
        ]),
    );
    Ok(out)
}

/// The manifest of the workspace `app` belongs to, and its folder.
fn workspace_manifest(app: &Path) -> Result<(PathBuf, toml::Table)> {
    let mut dir = app.parent();
    while let Some(d) = dir {
        let m = d.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&m)
            && let Ok(t) = text.parse::<toml::Table>()
            && t.contains_key("workspace")
        {
            return Ok((d.to_path_buf(), t));
        }
        dir = d.parent();
    }
    Err(ToolError::new(format!("{} inherits from a workspace that cannot be found", app.display())))
}

/// `function` is a plain Rust path (it is written into the harness's source).
fn check_function(function: &str) -> Result<String> {
    let f = function.trim().trim_start_matches("crate::");
    let ok = !f.is_empty()
        && f.split("::").all(|seg| {
            seg.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
                && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
        });
    if ok {
        Ok(f.to_owned())
    } else {
        Err(ToolError::new(format!(
            "`{function}` is not a function path (such as `home` or `screens::settings`)"
        )))
    }
}

const HARNESS: &str = r#"//! Generated by remedy-tools to render one preview of the app. Rewritten on every preview.

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::devtools::{Preview, Variation, audit, is_rtl, reader_lines};

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn fail(message: &str) -> ! {
    eprintln!("remedy-preview: {message}");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let num = |i: usize, d: f32| arg(i).parse::<f32>().unwrap_or(d);
    let out = std::path::PathBuf::from(arg(1));
    let (width, height, scale, text_scale) = (num(2, 400.0), num(3, 300.0), num(4, 1.0), num(7, 1.0));
    let (theme, locale) = (arg(5), arg(6));
    let palettes = remedy_ui::themes::PALETTES;
    let palette = match theme.as_str() {
        "dark" | "" => palettes.iter().find(|p| p.dark),
        "light" => palettes.iter().find(|p| !p.dark),
        id => palettes.iter().find(|p| p.id == id),
    };
    let Some(palette) = palette else {
        let ids: Vec<&str> = palettes.iter().map(|p| p.id).collect();
        fail(&format!("no theme `{theme}`: use dark, light or a palette id ({})", ids.join(", ")))
    };
    let seen: Rc<RefCell<Option<remedy_ui::Ui>>> = Rc::default();
    let keep = seen.clone();
    let mut preview = Preview::new("preview", move |ui| {
        *keep.borrow_mut() = Some(ui.clone());
        app::{{function}}(ui)
    });
    preview.scale = scale;
    let variation = Variation { theme: palette.id.to_owned(), locale: locale.clone(), size: (width, height), text_scale };
    let shot = preview.render(&variation);
    let png = match shot.to_png() {
        Ok(p) => p,
        Err(e) => fail(&format!("encoding the PNG: {e}")),
    };
    if let Err(e) = std::fs::write(out.join("preview.png"), png) {
        fail(&format!("writing the PNG: {e}"));
    }
    let ui = seen.borrow().clone();
    let Some(ui) = ui else { fail("the preview function was never called") };
    let mut reader = Vec::new();
    for line in reader_lines(&ui) {
        let problems: Vec<String> = line.problems.iter().map(|p| json(&p.message())).collect();
        reader.push(format!(
            "{{\"id\":{},\"depth\":{},\"announcement\":{},\"problems\":[{}]}}",
            line.node.id,
            line.node.depth,
            json(&line.node.announcement()),
            problems.join(",")
        ));
    }
    let problems: Vec<String> = audit(&ui)
        .iter()
        .map(|p| format!("{{\"element\":{},\"message\":{}}}", ui.element(p.element).node_id(), json(&p.message())))
        .collect();
    let report = format!(
        "{{\"width\":{},\"height\":{},\"theme\":{},\"dark\":{},\"locale\":{},\"rtl\":{},\"reader\":[{}],\"problems\":[{}]}}",
        shot.width,
        shot.height,
        json(palette.id),
        palette.dark,
        json(&locale),
        is_rtl(&locale),
        reader.join(","),
        problems.join(",")
    );
    if let Err(e) = std::fs::write(out.join("report.json"), report) {
        fail(&format!("writing the report: {e}"));
    }
}
"#;

/// A preview an app declares with `#[preview]`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
pub struct DeclaredPreview {
    /// The function, by its path in the crate (`home_preview`, `previews::dark_home`): what
    /// [`PreviewInput::function`] takes.
    pub function: String,
    /// It can be rendered by path: it and its modules are public. Private ones are shown by
    /// the app's own gallery and golden tests, not by [`render`].
    pub public: bool,
    /// The attribute as written, with the variations it asks for.
    pub attribute: String,
    /// Its source file, relative to the app.
    pub file: String,
    /// Its line there, from 1.
    pub line: usize,
}

/// Which app's previews to list.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "devkit-schema", derive(schemars::JsonSchema))]
pub struct ListInput {
    /// The app's folder (its Cargo.toml), inside the workspace.
    pub app: String,
}

impl ListInput {
    /// The previews of the app in `app`.
    pub fn new(app: impl Into<String>) -> Self {
        ListInput { app: app.into() }
    }
}

/// The module path of a source file in `src/` (`src/a/b.rs` and `src/a/b/mod.rs` are `a::b`).
fn module_of(rel: &str) -> Vec<String> {
    let inner = rel.strip_prefix("src/").unwrap_or(rel).trim_end_matches(".rs");
    inner.split('/').filter(|p| !matches!(*p, "lib" | "main" | "mod")).map(str::to_owned).collect()
}

/// Lines of code without string contents, for counting braces.
fn braces(line: &str) -> i32 {
    let mut depth = 0;
    let mut in_str = false;
    let mut prev = ' ';
    for c in line.chars() {
        match c {
            '"' if prev != '\\' => in_str = !in_str,
            '/' if !in_str && prev == '/' => break,
            '{' if !in_str => depth += 1,
            '}' if !in_str => depth -= 1,
            _ => {}
        }
        prev = c;
    }
    depth
}

/// The previews declared in one source file (`rel` is its path in the app).
pub fn declared_in(rel: &str, source: &str) -> Vec<DeclaredPreview> {
    let mut out = Vec::new();
    // Inline modules: (name, public, the depth their body is at).
    let mut mods: Vec<(String, bool, i32)> = Vec::new();
    let mut depth = 0i32;
    let mut attribute: Option<(String, usize)> = None;
    for (n, line) in source.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("#[preview") && !t.starts_with("#[previews") {
            attribute = Some((t.to_owned(), n + 1));
        } else if let Some((attr, at)) = attribute.take() {
            let mut rest = t;
            let public = rest.starts_with("pub ") || rest.starts_with("pub(crate) ");
            for prefix in ["pub(crate) ", "pub ", "const ", "async "] {
                rest = rest.strip_prefix(prefix).unwrap_or(rest);
            }
            if let Some(after) = rest.strip_prefix("fn ") {
                let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                let mut path = module_of(rel);
                path.extend(mods.iter().map(|(m, _, _)| m.clone()));
                path.push(name);
                // Reachable by path from the harness: the function and its inline modules are
                // `pub` (file modules are the app's to make public; a wrong guess fails to build
                // with the compiler's own explanation).
                let reachable = t.starts_with("pub ") && mods.iter().all(|(_, p, _)| *p);
                out.push(DeclaredPreview {
                    function: path.join("::"),
                    public: public && reachable,
                    attribute: attr,
                    file: rel.to_owned(),
                    line: n + 1,
                });
            } else if t.starts_with("#[") || t.starts_with("///") {
                attribute = Some((attr, at));
            }
        }
        let opens = t.strip_prefix("pub ").unwrap_or(t);
        if let Some(name) = opens.strip_prefix("mod ").and_then(|r| r.strip_suffix('{')) {
            mods.push((name.trim().to_owned(), t.starts_with("pub "), depth + 1));
        }
        depth += braces(line);
        while mods.last().is_some_and(|(_, _, d)| depth < *d) {
            mods.pop();
        }
    }
    out
}

/// The previews an app declares with `#[preview]`, in its `src/`.
pub fn list(input: &ListInput, ws: &Workspace) -> Result<Vec<DeclaredPreview>> {
    let app = ws.resolve(&input.app)?;
    if !app.join("Cargo.toml").is_file() {
        return Err(ToolError::new(format!("{} has no Cargo.toml", app.display())));
    }
    let mut files = Vec::new();
    let mut stack = vec![app.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let rel = f.strip_prefix(&app).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        out.extend(declared_in(&rel, &text));
    }
    Ok(out)
}

/// One preview at a time: they share the harness and the target folder.
static BUSY: Mutex<()> = Mutex::new(());

fn write_if_changed(path: &Path, text: &str) -> Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    std::fs::write(path, text).map_err(|e| ToolError::new(format!("{}: {e}", path.display())))
}

/// The nearest `Cargo.lock` at or above `dir`.
fn lock_above(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(p) = d {
        if p.join("Cargo.lock").is_file() {
            return Some(p.join("Cargo.lock"));
        }
        d = p.parent();
    }
    None
}

/// Renders `input`. The PNG comes back in [`PreviewOutput::png`]; `png_file`, when given,
/// also receives it (it must be inside the workspace).
pub fn render(input: &PreviewInput, ws: &Workspace, png_file: Option<&str>) -> Result<PreviewOutput> {
    let app = ws.resolve(&input.app)?;
    let function = check_function(&input.function)?;
    let manifest_text = std::fs::read_to_string(app.join("Cargo.toml"))
        .map_err(|e| ToolError::new(format!("{}: {e}", app.join("Cargo.toml").display())))?;
    let manifest: toml::Table = manifest_text
        .parse()
        .map_err(|e| ToolError::new(format!("{}: {e}", app.join("Cargo.toml").display())))?;
    let package = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| ToolError::new("the app's Cargo.toml has no package name"))?
        .to_owned();
    let has_lib = app.join("src/lib.rs").is_file() || manifest.get("lib").is_some();
    if !has_lib {
        return Err(ToolError::new(format!(
            "{package} has no library (src/lib.rs): previews call functions of the app's library"
        )));
    }
    let facade = facade_source(&app, &manifest)?;
    let png_target = png_file.map(|p| ws.resolve(p)).transpose()?;
    let _busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
    let target = target_dir(&app)?;
    let safe: String = package.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    // A harness for each previewed function: the in-process lock does not reach other
    // processes (two MCP servers, `cargo remedy preview`), which would otherwise rewrite one
    // shared main.rs under each other's builds and run each other's binaries.
    let fnv = function
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let bin = format!("remedy-preview-{safe}-{fnv:016x}");
    let harness = target.join("remedy-tools").join("preview").join(&safe).join(format!("{fnv:016x}"));
    std::fs::create_dir_all(&harness).map_err(|e| ToolError::new(format!("{}: {e}", harness.display())))?;
    let mut deps = toml::Table::new();
    let mut app_dep = toml::Table::new();
    app_dep.insert("package".into(), toml::Value::String(package.clone()));
    app_dep.insert("path".into(), toml::Value::String(app.display().to_string().replace('\\', "/")));
    deps.insert("app".into(), toml::Value::Table(app_dep));
    deps.insert("remedy-ui".into(), toml::Value::Table(facade));
    let mut pkg = toml::Table::new();
    pkg.insert("name".into(), toml::Value::String(bin.clone()));
    pkg.insert("version".into(), toml::Value::String("0.0.0".into()));
    pkg.insert("edition".into(), toml::Value::String("2024".into()));
    pkg.insert("publish".into(), toml::Value::Boolean(false));
    let mut bin_table = toml::Table::new();
    bin_table.insert("name".into(), toml::Value::String(bin.clone()));
    bin_table.insert("path".into(), toml::Value::String("main.rs".into()));
    let mut doc = toml::Table::new();
    doc.insert("package".into(), toml::Value::Table(pkg));
    doc.insert("bin".into(), toml::Value::Array(vec![toml::Value::Table(bin_table)]));
    doc.insert("dependencies".into(), toml::Value::Table(deps));
    doc.insert("workspace".into(), toml::Value::Table(toml::Table::new()));
    let cargo_toml =
        toml::to_string(&doc).map_err(|e| ToolError::new(format!("writing the harness manifest: {e}")))?;
    write_if_changed(&harness.join("Cargo.toml"), &format!("# Generated by remedy-tools.\n{cargo_toml}"))?;
    write_if_changed(&harness.join("main.rs"), &HARNESS.replace("{{function}}", &function))?;
    // Start from a lock file that already pins the engine's dependencies: the app's, or the
    // RemedyUI checkout's for a path dependency.
    if !harness.join("Cargo.lock").is_file() {
        let engine = doc
            .get("dependencies")
            .and_then(|d| d.get("remedy-ui"))
            .and_then(|r| r.get("path"))
            .and_then(toml::Value::as_str)
            .map(PathBuf::from);
        let lock = lock_above(&app).or_else(|| engine.as_deref().and_then(lock_above));
        if let Some(l) = lock {
            let _ = std::fs::copy(l, harness.join("Cargo.lock"));
        }
    }
    let timeout = input.timeout_seconds.map(Duration::from_secs);
    let mut build = Command::new(cargo());
    build
        .arg("build")
        .arg("--manifest-path")
        .arg(harness.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .args(["--bin", &bin, "--color", "never"]);
    let built = run(build, &harness, timeout)?;
    if !built.ok {
        if built.timed_out {
            return Err(ToolError::new("building the preview ran out of time"));
        }
        let errors = compile_errors(&built.stderr);
        let detail = if errors.is_empty() { built.stderr.trim().to_owned() } else { errors.join("\n") };
        return Err(ToolError::new(format!("the preview did not build:\n{detail}")));
    }
    let exe = target.join("debug").join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out = harness.join(format!("out-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&out).map_err(|e| ToolError::new(format!("{}: {e}", out.display())))?;
    let mut cmd = Command::new(&exe);
    cmd.arg(&out)
        .arg(input.width.unwrap_or(400.0).to_string())
        .arg(input.height.unwrap_or(300.0).to_string())
        .arg(input.scale.unwrap_or(1.0).to_string())
        .arg(input.theme.clone().unwrap_or_else(|| "dark".into()))
        .arg(input.locale.clone().unwrap_or_else(|| "en-US".into()))
        .arg(input.text_scale.unwrap_or(1.0).to_string());
    let ran = run(cmd, &app, timeout);
    let result = (|| {
        let ran = ran?;
        if !ran.ok {
            if ran.timed_out {
                return Err(ToolError::new("rendering the preview ran out of time"));
            }
            let msg = ran.stderr.trim().trim_start_matches("remedy-preview: ");
            return Err(ToolError::new(format!("the preview failed: {msg}")));
        }
        let png = std::fs::read(out.join("preview.png"))
            .map_err(|e| ToolError::new(format!("the preview's PNG: {e}")))?;
        let report = std::fs::read_to_string(out.join("report.json"))
            .map_err(|e| ToolError::new(format!("the preview's report: {e}")))?;
        let mut output: PreviewOutput = serde_json::from_str(&report)
            .map_err(|e| ToolError::new(format!("the preview's report: {e}")))?;
        output.png = png;
        if let Some(target) = &png_target {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(target, &output.png)
                .map_err(|e| ToolError::new(format!("{}: {e}", target.display())))?;
            output.png_file = Some(target.display().to_string());
        }
        Ok(output)
    })();
    let _ = std::fs::remove_dir_all(&out);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_previews_are_found_with_their_paths() {
        let src = r#"
use remedy_ui::{El, Ui};

#[preview(theme = "forest", locales = ["en-US", "ar-EG"])]
pub fn home_preview(ui: &Ui) -> El { crate::home(ui) }

#[preview]
fn private_preview(ui: &Ui) -> El { crate::home(ui) }

#[previews]
pub mod previews {
    use super::*;

    /// Dark.
    #[preview(size = (400, 600))]
    pub fn dark(ui: &Ui) -> El { crate::home(ui) }
}

mod hidden {
    #[preview]
    pub fn inner(ui: &Ui) -> El { crate::home(ui) }
}
"#;
        let found = declared_in("src/screens.rs", src);
        let names: Vec<(&str, bool)> = found.iter().map(|d| (d.function.as_str(), d.public)).collect();
        assert_eq!(
            names,
            vec![
                ("screens::home_preview", true),
                ("screens::private_preview", false),
                ("screens::previews::dark", true),
                ("screens::hidden::inner", false),
            ]
        );
        let Some(first) = found.first() else { unreachable!("found") };
        assert_eq!(
            (first.line, first.attribute.as_str()),
            (5, "#[preview(theme = \"forest\", locales = [\"en-US\", \"ar-EG\"])]")
        );
        assert!(
            declared_in("src/lib.rs", "#[preview]\npub fn p(ui: &Ui) -> El { x(ui) }\n")
                .first()
                .is_some_and(|d| d.function == "p")
        );
    }

    #[test]
    fn function_paths_are_plain_paths() {
        assert_eq!(check_function("home").ok().as_deref(), Some("home"));
        assert_eq!(check_function("crate::screens::settings").ok().as_deref(), Some("screens::settings"));
        assert!(check_function("home(ui); std::process::exit(0)").is_err());
        assert!(check_function("").is_err());
        assert!(check_function("a::").is_err());
    }

    #[test]
    fn the_facade_dependency_keeps_its_source_and_gains_devtools() {
        let m: toml::Table = r#"
[package]
name = "x"
[dependencies]
remedy-ui = { git = "https://example.invalid/RemedyUI", rev = "abc", features = ["data"] }
"#
        .parse()
        .unwrap_or_default();
        let Ok(dep) = facade_source(Path::new("."), &m) else { unreachable!("it has the dependency") };
        assert_eq!(dep.get("rev").and_then(toml::Value::as_str), Some("abc"));
        assert_eq!(dep.get("default-features").and_then(toml::Value::as_bool), Some(false));
        let features = dep.get("features").and_then(toml::Value::as_array).cloned().unwrap_or_default();
        assert!(features.iter().any(|f| f.as_str() == Some("devtools")));
        let none: toml::Table = "[package]\nname = \"x\"\n".parse().unwrap_or_default();
        assert!(facade_source(Path::new("."), &none).is_err());
    }

    #[test]
    fn reader_text_indents_and_flags_problems() {
        let out = PreviewOutput {
            png: Vec::new(),
            png_file: None,
            width: 1,
            height: 1,
            theme: "forest".into(),
            dark: true,
            locale: "en-US".into(),
            rtl: false,
            reader: vec![
                ReaderNode { id: 1, depth: 0, announcement: "group".into(), problems: Vec::new() },
                ReaderNode {
                    id: 2,
                    depth: 1,
                    announcement: "button".into(),
                    problems: vec!["Control with no accessible name".into()],
                },
            ],
            problems: Vec::new(),
        };
        assert_eq!(out.reader_text(), "group  [#1]\n  button  [#2]\n    ! Control with no accessible name\n");
    }
}
