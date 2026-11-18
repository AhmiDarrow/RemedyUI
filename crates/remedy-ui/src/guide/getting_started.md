# Getting started

This page makes a first app with `cargo remedy new`, explains each file it writes, and then
builds a small app by hand so you see every piece.

## Install `cargo remedy`

`cargo remedy` creates, runs and packages RemedyUI apps. Install it from the repository:

```text
cargo install --git https://github.com/AhmiDarrow/RemedyUI cargo-remedy
```

## Make an app

```text
cargo remedy new sticky-notes
cd sticky-notes
cargo remedy run
```

A window opens with a heading and a button. Press F12 in it for the inspector: the element
tree, each element's style, layout box and bindings, the window as a screen reader meets it,
and frame timings.

`cargo remedy new` takes a template:

| Template | What it makes |
|---|---|
| `basic` (the default) | One window with a heading and a button |
| `tray` | An app that lives in the tray, with one instance and a window it hides to the tray |
| `multi-window` | A main window that opens document windows, each remembering its place |
| `document` | A viewer for documents made of page images, with zoom, spreads and thumbnails |
| `android` | One app for Android and the desktop, packaged as an APK |

`--path <dir>` puts the app somewhere other than a folder named after it, and
`--remedy-path <checkout>` uses a local RemedyUI checkout instead of the git repository.

## What it wrote

| File | What it is for |
|---|---|
| `Cargo.toml` | The app's package: `remedy-ui` as a dependency, and again for tests with `headless` and `devtools`; a release profile tuned for small executables |
| `src/lib.rs` | The app itself, as a library: `app` opens the windows, `home` builds the main screen |
| `src/main.rs` | Ten lines: `remedy_ui::run` with the app's name and `app` |
| `tests/app.rs` | A test that runs the app on the in-memory host, and a preview checked against golden images |
| `i18n/en-US.ftl` | The app's text in Fluent; add a file per language |
| `remedy.toml` | Name, id, publisher, icon and signing, for `cargo remedy package` |
| `assets/icon.png` | The app's icon |

The app is a library so the test and the previews can build exactly what `main` runs.

- `cargo remedy run` builds and runs it, with devtools in debug builds. `--watch` rebuilds
  and restarts it when you save; `--release` runs the release build.
- `cargo test` runs it headless and checks its previews. The first run writes the golden
  images to `tests/golden`; commit them.
- `cargo remedy package` builds the installer and a portable zip, or with `--target` a Linux
  archive, an Android APK or a macOS app (see [`crate::guide::packaging`]).
- `cargo remedy migrate` applies RemedyUI's renames to the app when you move to a newer
  release (see [`crate::guide::stability`]).

## A first app, by hand

A RemedyUI app is three things:

- [`run`](crate::run) starts the platform's bridge with an [`AppConfig`](crate::AppConfig) and
  a setup function.
- The setup function gets an [`App`](crate::App) and opens windows with
  [`App::open`](crate::App::open), each from a [`WindowSpec`](crate::WindowSpec).
- Each window's build function gets the window's [`Ui`](crate::Ui) and adds elements
  ([`El`](crate::El)) to its root. Widgets ([`widgets`](crate::widgets)) are functions that
  build elements.

State lives in [`Signal`](crate::reactive::Signal)s, and elements follow them through
properties given a closure, such as `.text(move || ..)`. The build function runs once; after
that, only what reads a changed signal updates.

Here is a small notes app, written the way the template writes apps: the app in functions a
test can call, and `main` running them.

```
use remedy_ui::core::Insets;
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::a11y::Role;
use remedy_ui::testing::TestApp;
use remedy_ui::{App, AppConfig, El, Ui, WindowSpec};

/// Opens the main window, where it was last time.
pub fn app(app: &App) {
    let spec = WindowSpec::new("Notes").size(420.0, 320.0).persist_geometry("main");
    app.open(spec, |ui, _window| {
        ui.root().child(home(ui));
    });
}

/// The main screen: a field, an Add button, and the notes so far.
pub fn home(ui: &Ui) -> El {
    let notes = Signal::new(Vec::<String>::new());
    let space = ui.theme().spacing;

    let field = w::text_input(ui, "New note").placeholder("Write a note");
    let f = field.clone();
    let add = move || {
        let text = f.value();
        if !text.trim().is_empty() {
            notes.update(|n| n.push(text));
            f.set_value("");
        }
    };
    // Enter in the field and the Add button do the same.
    let field = field.on_submit(add.clone());

    let list = w::column(ui).gap(space.s(1));
    let u = ui.clone();
    let list = w::for_each(&list, notes, |n| n.clone(), move |n| w::text(&u, n.clone()));

    w::column(ui)
        .gap(space.s(3))
        .padding(Insets::all(space.s(4)))
        .child(w::heading(ui, 1, "Notes"))
        .child(w::row(ui).gap(space.s(2)).child(field.el().grow(1.0)).child(w::button(ui, "Add", add)))
        .child(w::text(ui, move || match notes.with(Vec::len) {
            0 => "No notes yet".into(),
            1 => "1 note".into(),
            n => format!("{n} notes"),
        }).variant("muted"))
        .child(list)
}

// `main` would be: remedy_ui::run(AppConfig::new("Notes"), app)

// The test: run the app on the in-memory host and use it as a person would.
let mut test = TestApp::new(AppConfig::new("Notes"), app);
let Some(ui) = test.ui() else { unreachable!("a window opened") };
let names = |ui: &Ui| ui.reader_view().into_iter().filter_map(|n| n.name).collect::<Vec<_>>();
assert!(names(&ui).contains(&"No notes yet".to_string()));

// Focus the field, type, and press Add, as a person would.
let Some(field) = test.find(Role::TextInput, "New note") else { unreachable!("on screen") };
field.focus();
test.type_text("Buy bread");
assert!(test.press(Role::Button, "Add"));

let shown = names(&ui);
assert!(shown.contains(&"Buy bread".to_string()), "{shown:?}");
assert!(shown.contains(&"1 note".to_string()));
```

To run it for real, `main` is one call:

```no_run
# fn app(_: &remedy_ui::App) {}
fn main() {
    if let Err(e) = remedy_ui::run(remedy_ui::AppConfig::new("Notes"), app) {
        eprintln!("Notes: {e}");
        std::process::exit(1);
    }
}
```

## What happened

- `home` ran once. Typing went into the field's own editor. Pressing Add pushed to `notes`,
  which updated the `for_each` (one new row) and the count's text closure (one label). Nothing
  else ran.
- The window remembers where it was ([`WindowSpec::persist_geometry`](crate::WindowSpec::persist_geometry)).
- The look came from the first Remedy palette and the widget style sheet, which
  [`AppConfig::new`](crate::AppConfig::new) sets up. Spacing came from the theme's spacing
  unit.
- Every control is named for screen readers by its label, which is also how the test found
  them.

## Where to go next

- [`crate::guide::signals`]: how state, memos, effects and bindings fit together.
- [`crate::guide::layout`] and [`crate::guide::theming`]: placing things and how they look.
- [`crate::guide::testing`]: the in-memory host, finding elements, golden previews.
- [`crate::guide::recipes`]: whole small apps to start from.
- [`crate::guide::porting`]: if you are moving an app from egui, React, Qt or HTML.
