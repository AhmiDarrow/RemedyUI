//! RemedyUI's app templates: the files of a new app, for `cargo remedy new` and for the
//! developer tools' `new_app` (so a new app is the same whichever made it).
//!
//! ```no_run
//! # fn main() -> Result<(), remedy_templates::TemplateError> {
//! let files = remedy_templates::create(
//!     std::path::Path::new("my-notes"),
//!     "my-notes",
//!     "basic",
//!     &remedy_templates::Dependency::Git(remedy_templates::REPOSITORY.to_owned()),
//! )?;
//! assert!(files.iter().any(|f| f.ends_with("src/lib.rs")));
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};

pub mod image;
mod more;

/// The RemedyUI repository new apps depend on by default.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Why an app could not be made.
#[non_exhaustive]
#[derive(Debug)]
pub enum TemplateError {
    /// The name is not a package name.
    BadName(String),
    /// No template has this name; the known ones are in [`TEMPLATES`].
    NoTemplate(String),
    /// The folder exists and is not empty.
    NotEmpty(PathBuf),
    /// Writing a picture failed.
    Image(image::ImageError),
    /// Writing a file failed.
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::BadName(name) => write!(
                f,
                "`{name}` is not a package name: use letters, digits, - and _, starting with a letter"
            ),
            TemplateError::NoTemplate(name) => {
                let list: Vec<String> = TEMPLATES.iter().map(|(t, d)| format!("  {t}: {d}")).collect();
                write!(f, "no template `{name}`; choose one of:\n{}", list.join("\n"))
            }
            TemplateError::NotEmpty(dir) => write!(f, "{} already exists and is not empty", dir.display()),
            TemplateError::Image(e) => write!(f, "{e}"),
            TemplateError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for TemplateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TemplateError::Image(e) => Some(e),
            TemplateError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<image::ImageError> for TemplateError {
    fn from(e: image::ImageError) -> Self {
        TemplateError::Image(e)
    }
}

/// Writes the app `name` from `template` into `dir` (which must be new or empty) and returns
/// the files written, relative to `dir`.
pub fn create(
    dir: &Path,
    name: &str,
    template: &str,
    remedy: &Dependency,
) -> Result<Vec<PathBuf>, TemplateError> {
    let files = generate(name, template, remedy)?;
    if dir.exists() && std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err(TemplateError::NotEmpty(dir.to_path_buf()));
    }
    for (rel, content) in &files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| TemplateError::Io { path: parent.to_path_buf(), source })?;
        }
        std::fs::write(&path, content).map_err(|source| TemplateError::Io { path: path.clone(), source })?;
    }
    Ok(files.into_iter().map(|(p, _)| p).collect())
}

/// Templates and what each makes.
pub const TEMPLATES: [(&str, &str); 5] = [
    ("basic", "one window with a heading and a button"),
    ("tray", "an app that lives in the tray, with one instance and a window it hides to the tray"),
    ("multi-window", "a main window that opens document windows, each remembering its place"),
    ("document", "a viewer for documents made of page images, with zoom, spreads and thumbnails"),
    ("android", "one app for Android and the desktop, packaged as an APK"),
];

/// Where the app gets RemedyUI.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dependency {
    /// A git repository.
    Git(String),
    /// A local checkout (its root).
    Path(PathBuf),
}

impl Dependency {
    fn spec(&self, features: &str) -> String {
        let src = match self {
            Dependency::Git(url) => format!("git = \"{url}\""),
            Dependency::Path(root) => {
                let p = root.join("crates").join("remedy-ui").display().to_string();
                // `canonicalize` gives Windows verbatim paths (`\\?\C:\…`); Cargo wants plain ones.
                let p = p.strip_prefix(r"\\?\").unwrap_or(&p);
                format!("path = \"{}\"", p.replace('\\', "/"))
            }
        };
        if features.is_empty() {
            format!("{{ {src} }}")
        } else {
            format!("{{ {src}, features = [{features}] }}")
        }
    }

    /// The engine's `[patch.crates-io]` for the app, which builds outside the engine's
    /// workspace: vendored crates from the checkout or the repository.
    fn patch(&self) -> String {
        let vendored = match self {
            Dependency::Git(url) => format!("{{ git = \"{url}\" }}"),
            Dependency::Path(root) => {
                let p = root.join("vendor").join("vello_cpu").display().to_string();
                let p = p.strip_prefix(r"\\?\").unwrap_or(&p);
                format!("{{ path = \"{}\" }}", p.replace('\\', "/"))
            }
        };
        PATCH.replace("{ path = \"vendor/vello_cpu\" }", &vendored)
    }
}

/// The workspace's `[patch.crates-io]` lines, which every app needs until the patched crates
/// are released: the wgpu fork's damage-only presents and the vendored `vello_cpu`. A test
/// keeps it equal to the workspace's.
const PATCH: &str = r#"vello_cpu = { path = "vendor/vello_cpu" }
wgpu = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-core = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-hal = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-types = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-naga-bridge = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
naga = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
naga-types = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-core-deps-apple = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-core-deps-emscripten = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-core-deps-wasm = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
wgpu-core-deps-windows-linux-android = { git = "https://github.com/AhmiDarrow/wgpu", rev = "a30dba29196796037ddcd7f25954a717109d5c28" }
"#;

/// `my-app` → `My App`.
pub fn title(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn check_name(name: &str) -> Result<String, TemplateError> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(TemplateError::BadName(name.to_owned()));
    }
    Ok(name.replace('-', "_"))
}

/// The files of a new app, relative to its directory.
pub fn generate(
    name: &str,
    template: &str,
    remedy: &Dependency,
) -> Result<Vec<(PathBuf, Vec<u8>)>, TemplateError> {
    let krate = check_name(name)?;
    let lib = match template {
        "basic" => BASIC,
        "document" => crate::more::DOCUMENT,
        "android" => crate::more::ANDROID,
        "tray" => TRAY,
        "multi-window" => MULTI,
        other => return Err(TemplateError::NoTemplate(other.to_owned())),
    };
    let title = title(name);
    let fill =
        |s: &str| s.replace("{{title}}", &title).replace("{{crate}}", &krate).replace("{{name}}", name);
    let single = template == "tray";
    // New apps start on the newest edition's behavior; existing apps keep theirs.
    let main = fill(MAIN)
        .replace("{{edition}}", &remedy_ui_core::Edition::LATEST.to_string())
        .replace("{{config}}", if single { ".single_instance()" } else { "" });
    // Packs a template needs on top of the defaults.
    let packs = if template == "document" { "\"pages\", \"image\"" } else { "" };
    let dev = if packs.is_empty() {
        "\"headless\", \"devtools\"".to_owned()
    } else {
        format!("{packs}, \"headless\", \"devtools\"")
    };
    let mut cargo = fill(CARGO)
        .replace("{{remedy}}", &remedy.spec(packs))
        .replace("{{remedy_dev}}", &remedy.spec(&dev))
        .replace("{{patch}}", &remedy.patch());
    let mut remedy_toml = fill(REMEDY_TOML);
    if template == "android" {
        cargo =
            cargo.replacen("\n[dependencies]", &format!("{}\n[dependencies]", crate::more::ANDROID_LIB), 1);
        remedy_toml.push_str(crate::more::ANDROID_TOML);
    }
    let icon = crate::image::write_png(&crate::image::default_icon([61, 139, 95]))?;
    let mut files = vec![
        (PathBuf::from("Cargo.toml"), cargo.into_bytes()),
        (PathBuf::from("src/lib.rs"), fill(lib).into_bytes()),
        (PathBuf::from("src/main.rs"), main.into_bytes()),
        (PathBuf::from("tests/app.rs"), fill(TEST).into_bytes()),
        (PathBuf::from("remedy.toml"), remedy_toml.into_bytes()),
        (PathBuf::from("assets/icon.png"), icon),
        (PathBuf::from(".gitignore"), b"/target\n*.actual.png\n".to_vec()),
        (PathBuf::from("README.md"), fill(README).into_bytes()),
        (PathBuf::from("i18n/en-US.ftl"), fill(ftl(template)).into_bytes()),
    ];
    if template == "document" {
        for (n, color) in [(1u32, [61u8, 139, 95]), (2, [90, 110, 170])] {
            files.push((
                PathBuf::from(format!("pages/page-{n:03}.png")),
                crate::image::write_png(&sample_page(n, color))?,
            ));
        }
    }
    let lib_text = files
        .iter()
        .find(|(p, _)| p == Path::new("src/lib.rs"))
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned());
    if let Some(text) = lib_text {
        let text = text.replace("{{edition}}", &remedy_ui_core::Edition::LATEST.to_string());
        if let Some(f) = files.iter_mut().find(|(p, _)| p == Path::new("src/lib.rs")) {
            f.1 = text.into_bytes();
        }
    }
    finish(&mut files, template, &krate);
    Ok(files)
}

/// A sample page for the document template (US Letter at 150 dpi): a band of `color` across
/// the top and `n` marks under it, so the pages tell apart. People replace them with theirs.
fn sample_page(n: u32, color: [u8; 3]) -> crate::image::Rgba {
    let (width, height) = (1275u32, 1650u32);
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let band = (150..330).contains(&y) && (150..1125).contains(&x);
            let mark = (420..480).contains(&y) && (0..n).any(|i| (150 + i * 90..210 + i * 90).contains(&x));
            let [r, g, b] = if band || mark { color } else { [255, 255, 255] };
            data.extend([r, g, b, 255]);
        }
    }
    crate::image::Rgba { width, height, data }
}

const CARGO: &str = r#"[package]
name = "{{name}}"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
remedy-ui = {{remedy}}

[dev-dependencies]
# Tests run the app on the in-memory host and check its previews.
remedy-ui = {{remedy_dev}}

[profile.release]
lto = "fat"
opt-level = "s"
codegen-units = 1
strip = true
panic = "abort"

# The engine's patched crates (the app builds outside its workspace).
[patch.crates-io]
{{patch}}"#;

const MAIN: &str = r#"//! {{title}}.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let config = remedy_ui::AppConfig::new("{{title}}").edition(remedy_ui::Edition::E{{edition}}){{config}};
    if let Err(e) = remedy_ui::run(config, {{crate}}::app) {
        eprintln!("{{title}}: {e:?}");
        std::process::exit(1);
    }
}
"#;

const BASIC: &str = r#"//! {{title}}: the app, as a library so tests and previews can build it too.

use remedy_ui::core::Insets;
use remedy_ui::i18n::{Arg, Messages};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, El, Ui, WindowSpec};

/// Opens the app's window.
pub fn app(app: &App) {
    app.open(WindowSpec::new("{{title}}").size(420.0, 300.0).persist_geometry("main"), |ui, window| {
        let (u, win) = (ui.clone(), window.clone());
        ui.root().child(home(ui)).bind(move || win.set_title(messages().text(&u, "app-title")));
    });
}

/// The main screen.
pub fn home(ui: &Ui) -> El {
    let clicks = Signal::new(0u32);
    let unit = ui.theme().spacing.unit;
    let u = ui.clone();
    let u2 = ui.clone();
    let button = w::button(ui, move || messages().text(&u2, "click-me"), move || clicks.update(|c| *c += 1))
        .variant("primary");
    w::column(ui)
        .gap(unit * 3.0)
        .padding(Insets::all(unit * 6.0))
        .child(say(ui, "app-title", "strong").style(remedy_ui::style::StylePatch::new().text_size(ui.theme().type_scale.title + 6.0)))
        .child(w::text(ui, "").variant("muted").text(move || {
            messages().format(&u, "clicks", &[("count", Arg::from(clicks.get()))])
        }))
        .child(button)
}

/// The app's strings, from `i18n/<locale>.ftl`. Release builds carry them inside; debug builds
/// read the folder and reload it as you edit, so text changes show without a rebuild.
pub fn messages() -> Messages {
    thread_local!(static MESSAGES: Messages = {
        let m = Messages::new("en-US").embedded("en-US", include_str!("../i18n/en-US.ftl"));
        if cfg!(debug_assertions) {
            let m = m.folder(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n"));
            m.watch();
            m
        } else {
            m
        }
    });
    MESSAGES.with(Clone::clone)
}

/// A label showing message `id`, following the window's locale and message edits.
fn say(ui: &Ui, id: &'static str, variant: &'static str) -> El {
    let u = ui.clone();
    w::text(ui, "").variant(variant).text(move || messages().text(&u, id))
}
"#;

const TRAY: &str = r#"//! {{title}}: lives in the tray; closing the window hides it there.

use remedy_ui::core::Insets;
use remedy_ui::host::{TrayEvent, TrayIcon, TrayImage, TrayItem};
use remedy_ui::i18n::Messages;
use remedy_ui::widgets as w;
use remedy_ui::{App, El, Ui, WindowSpec};

const SHOW: u32 = 1;
const QUIT: u32 = 2;

/// Opens the window and puts the app in the tray.
pub fn app(app: &App) {
    app.keep_running(true);
    let a = app.clone();
    let win = app.open(WindowSpec::new("{{title}}").size(360.0, 240.0).hide_on_close(), move |ui, window| {
        // The tray and the title speak the window's language, and follow it when it changes.
        let (u, win, a) = (ui.clone(), window.clone(), a.clone());
        ui.root().child(home(ui)).bind(move || {
            let m = messages();
            win.set_title(m.text(&u, "app-title"));
            let item = |id, label: String| TrayItem::Item { id, label, enabled: true };
            a.set_tray(Some(TrayIcon::new(m.text(&u, "app-title"), tray_icon(), vec![item(SHOW, m.text(&u, "tray-show")), TrayItem::Separator, item(QUIT, m.text(&u, "tray-quit"))])));
        });
    });
    let (w2, a2) = (win.clone(), app.clone());
    app.on_tray(move |e| match e {
        TrayEvent::Activate | TrayEvent::Menu(SHOW) => {
            w2.set_visible(true);
            w2.focus();
        }
        TrayEvent::Menu(QUIT) => a2.quit(),
        _ => {}
    });
    // Starting it again shows the running one.
    app.on_second_instance(move |_| {
        win.set_visible(true);
        win.focus();
    });
}

/// The window's content.
pub fn home(ui: &Ui) -> El {
    let unit = ui.theme().spacing.unit;
    w::column(ui)
        .gap(unit * 3.0)
        .padding(Insets::all(unit * 6.0))
        .child(say(ui, "app-title", "strong").style(remedy_ui::style::StylePatch::new().text_size(ui.theme().type_scale.title + 6.0)))
        .child(say(ui, "tray-hint", "muted"))
}

/// A 32 px tray icon drawn in code: a green dot, premultiplied RGBA.
pub fn tray_icon() -> TrayImage {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0);
            let a = (13.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let c = |v: f32| (v * a * 255.0).round() as u8;
            rgba.extend([c(0.24), c(0.55), c(0.37), (a * 255.0).round() as u8]);
        }
    }
    TrayImage { size, rgba }
}

/// The app's strings, from `i18n/<locale>.ftl`. Release builds carry them inside; debug builds
/// read the folder and reload it as you edit, so text changes show without a rebuild.
pub fn messages() -> Messages {
    thread_local!(static MESSAGES: Messages = {
        let m = Messages::new("en-US").embedded("en-US", include_str!("../i18n/en-US.ftl"));
        if cfg!(debug_assertions) {
            let m = m.folder(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n"));
            m.watch();
            m
        } else {
            m
        }
    });
    MESSAGES.with(Clone::clone)
}

/// A label showing message `id`, following the window's locale and message edits.
fn say(ui: &Ui, id: &'static str, variant: &'static str) -> El {
    let u = ui.clone();
    w::text(ui, "").variant(variant).text(move || messages().text(&u, id))
}
"#;

const MULTI: &str = r#"//! {{title}}: a main window that opens document windows.

use std::cell::Cell;
use std::rc::Rc;

use remedy_ui::core::Insets;
use remedy_ui::i18n::{Arg, Messages};
use remedy_ui::widgets as w;
use remedy_ui::{App, El, Ui, Window, WindowSpec};

/// Opens the main window.
pub fn app(app: &App) {
    let a = app.clone();
    app.open(WindowSpec::new("{{title}}").size(420.0, 300.0).persist_geometry("main"), move |ui, window| {
        let (u, win) = (ui.clone(), window.clone());
        ui.root().child(home(ui, a)).bind(move || win.set_title(messages().text(&u, "app-title")));
    });
}

/// The main window's content: a button that opens numbered documents.
pub fn home(ui: &Ui, app: App) -> El {
    let unit = ui.theme().spacing.unit;
    let next = Rc::new(Cell::new(1u32));
    let u = ui.clone();
    let button = w::button(ui, move || messages().text(&u, "new-document"), move || {
        let n = next.get();
        next.set(n + 1);
        let _ = open_document(&app, n);
    })
    .variant("primary");
    w::column(ui)
        .gap(unit * 3.0)
        .padding(Insets::all(unit * 6.0))
        .child(say(ui, "app-title", "strong").style(remedy_ui::style::StylePatch::new().text_size(ui.theme().type_scale.title + 6.0)))
        .child(button)
}

/// Opens document `n` in its own window, where it was last time.
pub fn open_document(app: &App, n: u32) -> Window {
    let spec = WindowSpec::new(format!("{n}")).size(480.0, 360.0).persist_geometry(format!("document-{n}"));
    app.open(spec, move |ui, window| {
        let (u, win) = (ui.clone(), window.clone());
        ui.root()
            .child(document(ui, n))
            .bind(move || win.set_title(messages().format(&u, "document-title", &[("n", Arg::from(n))])));
    })
}

/// A document: its title and its text.
pub fn document(ui: &Ui, n: u32) -> El {
    let unit = ui.theme().spacing.unit;
    let body = w::text_area(ui, "Text");
    let (b, u) = (body.clone(), ui.clone());
    let heading = w::text(ui, "").variant("strong").style(remedy_ui::style::StylePatch::new().text_size(ui.theme().type_scale.title));
    let (h, u2) = (heading.clone(), ui.clone());
    w::column(ui)
        .gap(unit * 2.0)
        .padding(Insets::all(unit * 4.0))
        .grow(1.0)
        .fill_width()
        .child(heading)
        .child(body.el().grow(1.0).fill_width())
        .bind(move || {
            let m = messages();
            let _ = b.clone().placeholder(m.text(&u, "write-here"));
            let _ = h.clone().text(remedy_ui::view::RichText::plain(m.format(&u2, "document-title", &[("n", Arg::from(n))])));
        })
}

/// The app's strings, from `i18n/<locale>.ftl`. Release builds carry them inside; debug builds
/// read the folder and reload it as you edit, so text changes show without a rebuild.
pub fn messages() -> Messages {
    thread_local!(static MESSAGES: Messages = {
        let m = Messages::new("en-US").embedded("en-US", include_str!("../i18n/en-US.ftl"));
        if cfg!(debug_assertions) {
            let m = m.folder(concat!(env!("CARGO_MANIFEST_DIR"), "/i18n"));
            m.watch();
            m
        } else {
            m
        }
    });
    MESSAGES.with(Clone::clone)
}

/// A label showing message `id`, following the window's locale and message edits.
fn say(ui: &Ui, id: &'static str, variant: &'static str) -> El {
    let u = ui.clone();
    w::text(ui, "").variant(variant).text(move || messages().text(&u, id))
}
"#;

/// The template's message file (`i18n/en-US.ftl`).
fn ftl(template: &str) -> &'static str {
    match template {
        "tray" => TRAY_FTL,
        "multi-window" => MULTI_FTL,
        "document" => crate::more::DOCUMENT_FTL,
        "android" => crate::more::ANDROID_FTL,
        _ => BASIC_FTL,
    }
}

const BASIC_FTL: &str = "# {{title}}'s text, in Fluent (https://projectfluent.org). Add a file per language:\n# i18n/de.ftl, i18n/ar.ftl, ... with the same ids.\n\napp-title = {{title}}\nclicks = { $count ->\n    [0] Not clicked yet\n    [one] Clicked once\n   *[other] Clicked { $count } times\n}\nclick-me = Click me\n";

const TRAY_FTL: &str = "# {{title}}'s text, in Fluent (https://projectfluent.org). Add a file per language:\n# i18n/de.ftl, i18n/ar.ftl, ... with the same ids.\n\napp-title = {{title}}\ntray-hint = Close this window and { app-title } keeps running in the tray.\ntray-show = Show { app-title }\ntray-quit = Quit\n";

const MULTI_FTL: &str = "# {{title}}'s text, in Fluent (https://projectfluent.org). Add a file per language:\n# i18n/de.ftl, i18n/ar.ftl, ... with the same ids.\n\napp-title = {{title}}\nnew-document = New document\ndocument-title = Document { $n }\nwrite-here = Write here…\n";

const TEST: &str = r#"//! The app on the in-memory host, and its previews as golden images.

use remedy_ui::AppConfig;
use remedy_ui::devtools::{GoldenError, Preview, check_golden};
use remedy_ui::testing::TestApp;

#[test]
fn the_app_opens_its_window() {
    let app = TestApp::new(AppConfig::new("{{title}} test"), {{crate}}::app);
    assert!(app.ui().is_some(), "a window opened");
}

#[test]
fn previews_match_their_golden_images() {
    let preview = Preview::new("home", |ui| {{preview}})
        .sizes(&[(420.0, 300.0)])
        .locales(&["en-US", "ar-EG"]);
    // The first run writes the images to tests/golden for review; commit them.
    match check_golden(&preview, std::path::Path::new("tests/golden")) {
        Ok(()) | Err(GoldenError::Missing(_)) => {}
        Err(e) => panic!("{e}"),
    }
}
"#;

const REMEDY_TOML: &str = r##"# How `cargo remedy package` builds and labels the app.

[app]
name = "{{title}}"
# A reverse-DNS id, unique to this app.
id = "app.remedy.{{name}}"
publisher = "{{title}} authors"
description = "{{title}}, built with RemedyUI"
# A square PNG, 256 px or larger.
icon = "assets/icon.png"

[windows]
# Sign the executable and installer with this certificate from your certificate store
# (its SHA-1 thumbprint). Without it, packages are unsigned.
# sign_thumbprint = "0123456789ABCDEF0123456789ABCDEF01234567"
# timestamp_url = "http://timestamp.digicert.com"

[web]
# `cargo remedy package --target web`: the name under the icon once installed, the color of
# the browser's own bars, and the icon (SVG or PNG; the app's icon if not set).
# short_name = "{{title}}"
# theme_color = "#1e2a24"
# icon = "assets/icon.svg"

# Auto-update: set REMEDY_UPDATE_KEY to a minisign secret key file (and
# REMEDY_UPDATE_KEY_PASSWORD if it has one) and `package` writes a .sig beside the
# installer. Publish both on a GitHub release; the app checks them with
# remedy_ui::update (feature "update") and its public key.
"##;

const README: &str = r#"# {{title}}

A [RemedyUI](https://github.com/AhmiDarrow/RemedyUI) app.

- `cargo remedy run` runs it; press F12 in a window for the inspector. `--watch` rebuilds
  and restarts it when you save.
- `cargo test` runs it on the in-memory host and checks its previews against
  `tests/golden` (the first run writes them; commit them).
- `cargo remedy package` builds the installer and a portable zip (see `remedy.toml`);
  `--target linux`, `--target android` and `--target macos` build for those systems.
- `cargo remedy package --target web` builds an installable page (a PWA that works
  offline) in `target/remedy/web` and a zip of it, from the same code (see `[web]` in
  `remedy.toml`). Serve the folder from any static web host.
"#;

/// The preview expression for a template's test.
pub fn preview_expr(template: &str) -> &'static str {
    match template {
        "multi-window" => "{{crate}}::document(ui, 1)",
        "document" => crate::more::DOCUMENT_TEST_PREVIEW,
        _ => "{{crate}}::home(ui)",
    }
}

/// Fills in the test's preview for the template (separate so the test text stays readable).
pub fn finish(files: &mut [(PathBuf, Vec<u8>)], template: &str, krate: &str) {
    for (path, content) in files.iter_mut() {
        if path == Path::new("tests/app.rs") {
            let s = String::from_utf8_lossy(content)
                .replace("{{preview}}", &preview_expr(template).replace("{{crate}}", krate));
            *content = s.into_bytes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_makes_a_complete_app_with_no_placeholders_left() {
        let dep = Dependency::Git(REPOSITORY.to_owned());
        for (t, _) in TEMPLATES {
            let Ok(files) = generate("my-notes", t, &dep) else { unreachable!("{t} generates") };
            for want in [
                "Cargo.toml",
                "src/lib.rs",
                "src/main.rs",
                "tests/app.rs",
                "i18n/en-US.ftl",
                "assets/icon.png",
            ] {
                assert!(files.iter().any(|(p, _)| p == Path::new(want)), "{t} has {want}");
            }
            for (p, bytes) in &files {
                if let Ok(text) = std::str::from_utf8(bytes) {
                    assert!(!text.contains("{{"), "{t}: {} has a placeholder left", p.display());
                }
            }
        }
    }

    #[test]
    fn apps_carry_the_workspace_patches() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ws = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
        assert!(ws.contains(&format!("[patch.crates-io]\n{PATCH}")), "PATCH differs from the workspace's");
        let cargo = |dep: &Dependency| {
            let files = generate("my-notes", "basic", dep).unwrap_or_else(|e| unreachable!("{e}"));
            let found = files.into_iter().find(|(p, _)| p == Path::new("Cargo.toml"));
            String::from_utf8(found.map(|f| f.1).unwrap_or_default()).unwrap_or_default()
        };
        let git = cargo(&Dependency::Git(REPOSITORY.to_owned()));
        assert!(git.contains(&format!("vello_cpu = {{ git = \"{REPOSITORY}\" }}")), "{git}");
        let path = cargo(&Dependency::Path(PathBuf::from("/src/remedy")));
        assert!(path.contains("vello_cpu = { path = \"/src/remedy/vendor/vello_cpu\" }"), "{path}");
        assert!(path.contains("wgpu = { git = \"https://github.com/AhmiDarrow/wgpu\""), "{path}");
    }

    #[test]
    fn names_and_templates_are_checked() {
        let dep = Dependency::Git(REPOSITORY.to_owned());
        assert!(matches!(generate("9lives", "basic", &dep), Err(TemplateError::BadName(_))));
        let Err(e) = generate("app", "nope", &dep) else { unreachable!("no such template") };
        assert!(e.to_string().contains("multi-window"), "{e}");
        assert_eq!(title("my-notes_app"), "My Notes App");
    }

    #[test]
    fn create_refuses_a_folder_with_files() {
        let dir = std::env::temp_dir().join(format!("remedy-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dep = Dependency::Git(REPOSITORY.to_owned());
        let made = create(&dir, "my-app", "tray", &dep);
        assert!(made.is_ok_and(|f| f.iter().any(|p| p.ends_with("remedy.toml"))));
        assert!(matches!(create(&dir, "my-app", "tray", &dep), Err(TemplateError::NotEmpty(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
