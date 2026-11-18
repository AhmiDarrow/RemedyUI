//! `cargo remedy new`: an app from a template.

use std::path::{Path, PathBuf};

use crate::flag_value;

/// Templates and what each makes.
pub const TEMPLATES: [(&str, &str); 3] = [
    ("basic", "one window with a heading and a button"),
    ("tray", "an app that lives in the tray, with one instance and a window it hides to the tray"),
    ("multi-window", "a main window that opens document windows, each remembering its place"),
];

/// Templates that arrive with later milestones, and why they are not here yet.
const LATER: [(&str, &str); 2] = [
    ("document", "needs the document packs (doc-rich, doc-pages), which arrive in M8"),
    ("android", "needs the Android bridge, which arrives in M7"),
];

pub fn run(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: cargo remedy new <name> [--template <t>]")?;
    let template = flag_value(args, "--template").unwrap_or_else(|| "basic".into());
    let dir = flag_value(args, "--path").map_or_else(|| PathBuf::from(name), PathBuf::from);
    let remedy = match flag_value(args, "--remedy-path") {
        Some(p) => {
            Dependency::Path(std::fs::canonicalize(&p).map_err(|e| format!("--remedy-path {p}: {e}"))?)
        }
        None => Dependency::Git(env!("CARGO_PKG_REPOSITORY").to_owned()),
    };
    let files = generate(name, &template, &remedy)?;
    if dir.exists() && std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err(format!("{} already exists and is not empty", dir.display()));
    }
    for (rel, content) in &files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    println!("Created {} ({template}) in {}", title(name), dir.display());
    println!("  cd {}", dir.display());
    println!("  cargo remedy run        # F12 in a window opens the inspector");
    println!("  cargo test              # the headless test and the preview's golden images");
    println!("  cargo remedy package    # installer and portable zip");
    Ok(())
}

/// Where the app gets RemedyUI.
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
}

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

fn check_name(name: &str) -> Result<String, String> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(format!(
            "`{name}` is not a package name: use letters, digits, - and _, starting with a letter"
        ));
    }
    Ok(name.replace('-', "_"))
}

/// The files of a new app, relative to its directory.
pub fn generate(name: &str, template: &str, remedy: &Dependency) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let krate = check_name(name)?;
    if let Some((_, why)) = LATER.iter().find(|(t, _)| *t == template) {
        return Err(format!("the `{template}` template {why}"));
    }
    let lib = match template {
        "basic" => BASIC,
        "tray" => TRAY,
        "multi-window" => MULTI,
        other => {
            let list: Vec<String> = TEMPLATES.iter().map(|(t, d)| format!("  {t}: {d}")).collect();
            return Err(format!("no template `{other}`; choose one of:\n{}", list.join("\n")));
        }
    };
    let title = title(name);
    let fill =
        |s: &str| s.replace("{{title}}", &title).replace("{{crate}}", &krate).replace("{{name}}", name);
    let single = template == "tray";
    let main = fill(MAIN).replace("{{config}}", if single { ".single_instance()" } else { "" });
    let cargo = fill(CARGO)
        .replace("{{remedy}}", &remedy.spec(""))
        .replace("{{remedy_dev}}", &remedy.spec("\"headless\", \"devtools\""));
    let icon = crate::ico::write_png(&crate::ico::default_icon([61, 139, 95]))?;
    let mut files = vec![
        (PathBuf::from("Cargo.toml"), cargo.into_bytes()),
        (PathBuf::from("src/lib.rs"), fill(lib).into_bytes()),
        (PathBuf::from("src/main.rs"), main.into_bytes()),
        (PathBuf::from("tests/app.rs"), fill(TEST).into_bytes()),
        (PathBuf::from("remedy.toml"), fill(REMEDY_TOML).into_bytes()),
        (PathBuf::from("assets/icon.png"), icon),
        (PathBuf::from(".gitignore"), b"/target\n*.actual.png\n".to_vec()),
        (PathBuf::from("README.md"), fill(README).into_bytes()),
        (PathBuf::from("i18n/en-US.ftl"), fill(ftl(template)).into_bytes()),
    ];
    finish(&mut files, template, &krate);
    Ok(files)
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
"#;

const MAIN: &str = r#"//! {{title}}.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let config = remedy_ui::AppConfig::new("{{title}}"){{config}};
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
    let button = w::button(ui, "", move || clicks.update(|c| *c += 1)).variant("primary");
    let label = button.child_elements().into_iter().next();
    let u2 = ui.clone();
    let button = button.bind(move || {
        if let Some(l) = &label {
            let _ = l.clone().text(remedy_ui::view::RichText::plain(messages().text(&u2, "click-me")));
        }
    });
    w::column(ui)
        .gap(unit * 3.0)
        .padding(Insets::all(unit * 6.0))
        .child(say(ui, "app-title", "strong").style(remedy_ui::style::StylePatch {
            text_size: Some(ui.theme().type_scale.title + 6.0),
            ..Default::default()
        }))
        .child(w::label(ui, "", "muted").text_dyn(move || {
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
    w::label(ui, "", variant).text_dyn(move || messages().text(&u, id))
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
            a.set_tray(Some(TrayIcon {
                tooltip: m.text(&u, "app-title"),
                icon: tray_icon(),
                menu: vec![item(SHOW, m.text(&u, "tray-show")), TrayItem::Separator, item(QUIT, m.text(&u, "tray-quit"))],
            }));
        });
    });
    let (w2, a2) = (win.clone(), app.clone());
    app.on_tray(move |e| match e {
        TrayEvent::Activate | TrayEvent::Menu(SHOW) => {
            w2.set_visible(true);
            w2.focus();
        }
        TrayEvent::Menu(QUIT) => a2.quit(),
        TrayEvent::Menu(_) => {}
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
        .child(say(ui, "app-title", "strong").style(remedy_ui::style::StylePatch {
            text_size: Some(ui.theme().type_scale.title + 6.0),
            ..Default::default()
        }))
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
    w::label(ui, "", variant).text_dyn(move || messages().text(&u, id))
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
    let button = w::button(ui, "", move || {
        let n = next.get();
        next.set(n + 1);
        let _ = open_document(&app, n);
    })
    .variant("primary");
    let label = button.child_elements().into_iter().next();
    let u = ui.clone();
    let button = button.bind(move || {
        if let Some(l) = &label {
            let _ = l.clone().text(remedy_ui::view::RichText::plain(messages().text(&u, "new-document")));
        }
    });
    w::column(ui)
        .gap(unit * 3.0)
        .padding(Insets::all(unit * 6.0))
        .child(say(ui, "app-title", "strong").style(remedy_ui::style::StylePatch {
            text_size: Some(ui.theme().type_scale.title + 6.0),
            ..Default::default()
        }))
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
    let heading = w::label(ui, "", "strong").style(remedy_ui::style::StylePatch {
        text_size: Some(ui.theme().type_scale.title),
        ..Default::default()
    });
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
    w::label(ui, "", variant).text_dyn(move || messages().text(&u, id))
}
"#;

/// The template's message file (`i18n/en-US.ftl`).
fn ftl(template: &str) -> &'static str {
    match template {
        "tray" => TRAY_FTL,
        "multi-window" => MULTI_FTL,
        _ => BASIC_FTL,
    }
}

const BASIC_FTL: &str = "# {{title}}'s text, in Fluent (https://projectfluent.org). Add a file per language:\n# i18n/de.ftl, i18n/ar.ftl, ... with the same ids.\n\napp-title = {{title}}\nclicks = { $count ->\n    [0] Not clicked yet\n    [one] Clicked once\n   *[other] Clicked { $count } times\n}\nclick-me = Click me\n";

const TRAY_FTL: &str = "# {{title}}'s text, in Fluent (https://projectfluent.org). Add a file per language:\n# i18n/de.ftl, i18n/ar.ftl, ... with the same ids.\n\napp-title = {{title}}\ntray-hint = Close this window and { app-title } keeps running in the tray.\ntray-show = Show { app-title }\ntray-quit = Quit\n";

const MULTI_FTL: &str = "# {{title}}'s text, in Fluent (https://projectfluent.org). Add a file per language:\n# i18n/de.ftl, i18n/ar.ftl, ... with the same ids.\n\napp-title = {{title}}\nnew-document = New document\ndocument-title = Document { $n }\nwrite-here = Write here…\n";

const TEST: &str = r#"//! The app on the in-memory host, and its previews as golden images.

use remedy_ui::devtools::{GoldenError, Preview, check_golden};
use remedy_ui::headless::HeadlessHost;
use remedy_ui::{AppConfig, GeometryStore, Runner};

#[test]
fn the_app_opens_its_window() {
    let config = AppConfig::new("{{title}} test").cpu_only();
    let mut runner = Runner::with_geometry(config, GeometryStore::in_memory(), {{crate}}::app);
    let mut host = HeadlessHost::new();
    host.run_frames(&mut runner, 2);
    assert!(!host.window_ids().is_empty(), "a window opened");
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

const REMEDY_TOML: &str = r#"# How `cargo remedy package` builds and labels the app.

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

# Auto-update: set REMEDY_UPDATE_KEY to a minisign secret key file (and
# REMEDY_UPDATE_KEY_PASSWORD if it has one) and `package` writes a .sig beside the
# installer. Publish both on a GitHub release; the app checks them with
# remedy_ui::update (feature "update") and its public key.
"#;

const README: &str = r#"# {{title}}

A [RemedyUI](https://github.com/AhmiDarrow/RemedyUI) app.

- `cargo remedy run` runs it; press F12 in a window for the inspector. `--watch` rebuilds
  and restarts it when you save.
- `cargo test` runs it on the in-memory host and checks its previews against
  `tests/golden` (the first run writes them; commit them).
- `cargo remedy package` builds the installer and a portable zip (see `remedy.toml`).
"#;

/// The preview expression for a template's test.
pub fn preview_expr(template: &str) -> &'static str {
    match template {
        "multi-window" => "{{crate}}::document(ui, 1)",
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
