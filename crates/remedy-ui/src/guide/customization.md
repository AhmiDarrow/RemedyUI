# Customization: markup, layouts, themes, shortcuts and toolbars

People can restyle, rebind and rearrange the apps they use. This guide covers the parts of
RemedyUI that make that possible:

- `.rui` markup: screens described as data, compiled to Rust at build time or built at run
  time ([`crate::markup`], feature `markup`)
- people's own layouts: their version of a screen, loaded from their config folder, with a
  fallback when it does not load ([`crate::markup::Layouts`])
- themes as design-tokens files ([`crate::themes::from_tokens`])
- shortcuts people rebind ([`crate::nav::Commands`])
- toolbars people arrange ([`crate::nav::toolbar`] and [`crate::nav::toolbar_editor`])
- all of these kept as files with undo and resets, and the editors for them
  ([`crate::customize`], feature `customize`)
- boards people arrange, with scripted widgets ([`crate::board`], feature `board`)

Rust builders stay the main way to build screens. Markup earns its place for live editing,
for files a designer can change without a Rust toolchain, and for people's own layouts.

## A screen in markup

A `.rui` file holds one element. An element is a name, optional arguments in parentheses
and optional children in braces:

```text
column(gap: s3, padding: s4) {
    switch(label: @t("Noise suppression"), checked: $noise_suppression)
    slider(label: @t("Gain"), value: $input_gain, range: 0..200, step: 1)
    button(label: @t("Save"), on_press: !save, variant: primary)
}
```

Argument values are literals (numbers, strings, `true`, `false`, ranges `a..b`), tokens
(`s3`, `center`), translations (`@t("text")`), and the signals (`$name`) and commands
(`!name`) the app exposes to the file. There are no other expressions. A file cannot run
code, read files or reach the network, so loading a stranger's file is as safe as loading
any other data.

The app exposes its state with a [`crate::markup::Scope`] and builds the file with
[`crate::markup::build_markup`]. The elements read and write the app's own signals:

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::markup::{Markup, Scope, Widgets, build_markup};
use remedy_ui::reactive::Signal;
use remedy_ui::view::A11yAction;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

const SETTINGS: &str = r#"
column(gap: s3, padding: s4) {
    switch(label: @t("Noise suppression"), checked: $noise_suppression)
    slider(label: @t("Gain"), value: $input_gain, range: 0..200, step: 1)
}
"#;

let noise = Signal::new(false);
let gain = Signal::new(100.0);
let scope = Scope::new()
    .flag("noise_suppression", noise)
    .number("input_gain", gain)
    .translator(|s| s.to_owned()); // the app's message catalog goes here

let markup = Markup::parse("settings_voice.rui", SETTINGS)?;
let screen = build_markup(&ui, &markup, &scope, &Widgets::new())?;
ui.root().child(screen);
# app.frames(1);

// The switch is bound to the app's signal, both ways.
let reader = ui.reader_view();
let Some(switch) = reader.iter().find(|n| n.name.as_deref() == Some("Noise suppression")) else {
    unreachable!("the switch is on screen")
};
assert!(ui.accessibility_action(switch.id, A11yAction::Click));
assert!(noise.get());
# Ok::<(), remedy_ui::markup::RuiError>(())
```

A [`crate::markup::Scope`] exposes text ([`Scope::text`](crate::markup::Scope::text)),
flags, numbers, choices, lists of choices, levels and commands. Each has its own method, so
a file that uses `$input_gain` as a flag is refused before anything is built.

## Errors at file, line and column

Everything is checked before the first element is made, so a broken file builds nothing.
A [`crate::markup::RuiError`] says where and what, and its `help` lists what would be right:
the names the app exposes, or the arguments an element takes.

```
use remedy_ui::markup::{Markup, Scope, Widgets, check};
use remedy_ui::reactive::Signal;

let scope = Scope::new().flag("noise_suppression", Signal::new(false));
let markup = Markup::parse("mine.rui", "switch(label: \"Quiet\", checked: $noise)")?;
let Err(e) = check(&markup, &scope, &Widgets::new()) else {
    unreachable!("`$noise` is not exposed")
};
assert_eq!(e.to_string(), "mine.rui:1:33: `$noise` is not exposed to this file (exposed: $noise_suppression)");

// Syntax errors point at the place too.
let Err(e) = Markup::parse("mine.rui", "column {\n  switch(label: \"A\" checked: $x)\n}") else {
    unreachable!("a comma is missing")
};
assert_eq!((e.at.line, e.at.col), (2, 21));
# Ok::<(), remedy_ui::markup::RuiError>(())
```

The built-in elements are the core widgets markup can describe: `column`, `row`, `wrap`,
`card`, `scroll`, `section`, `spacer`, `divider`, `text`, `heading`, `badge`, `button`,
`link`, `switch`, `check`, `toggle_button`, `chip`, `lamp`, `select`, `radio_group`,
`segmented`, `slider`, `stepper`, `progress`, `spinner`, `empty_state`, `text_input` and
`text_area`. Every element also takes `gap`, `padding`, `grow`, `fill_width`, `variant`,
`align`, `justify` and `visible` ([`crate::markup::spec::COMMON`]).
[`Widgets::specs`](crate::markup::Widgets::specs) lists what each one takes, which is what
an editor shows as people type.

## The app's own elements

An app adds its own elements with [`Widgets::register`](crate::markup::Widgets::register):
a name, the parameters it takes, whether it holds children, and a builder that gets the
resolved arguments as a [`crate::markup::Call`].

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::markup::spec::{Kind, Param};
use remedy_ui::markup::{Markup, Scope, Widgets, build_markup};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let mut widgets = Widgets::new();
widgets.register(
    "meter",
    &[Param { name: "level", kind: Kind::Level, required: true }],
    false,
    |ui, call| {
        let level = call.level("level").unwrap_or_else(|| Signal::new(None));
        w::progress(ui, "Input level", level)
    },
);

let level = Signal::new(Some(0.25_f32));
let scope = Scope::new().level("input_level", level);
let markup = Markup::parse("meter.rui", "column { meter(level: $input_level) }")?;
ui.root().child(build_markup(&ui, &markup, &scope, &widgets)?);
# app.frames(1);
assert!(ui.reader_view().iter().any(|n| n.name.as_deref() == Some("Input level")));
# Ok::<(), remedy_ui::markup::RuiError>(())
```

## Compiled at build time

Release builds need not interpret anything. In `build.rs`,
[`build::compile_dir`](crate::markup::build::compile_dir) compiles every `.rui` file in a
folder to `$OUT_DIR/rui/<name>.rs` and tells Cargo to rebuild when one changes:

```no_run
// build.rs
fn main() -> Result<(), Box<dyn std::error::Error>> {
    remedy_ui::markup::build::compile_dir("ui", &[])?;
    Ok(())
}
```

The app includes the file it wants, for example
`mod settings_voice { include!(concat!(env!("OUT_DIR"), "/rui/settings_voice.rs")); }`.
Each compiled file has:

- `Bindings`: a struct with a field for every signal and command the file uses. Each
  field's type follows from where the file uses it (`$noise_suppression` in `checked:` is a
  `Signal<bool>`).
- `build(ui, &bindings, translate)`: builds the same elements the interpreter would.
- `Bindings::scope(translate)`: the same names as a [`crate::markup::Scope`], so a
  person's own version of the file can load against the same signals.
- `FILE` and `SOURCE`: the file's name and text.

The generated code reaches the runtime through one `use` of a root path, the facade's
`remedy_ui::markup` ([`codegen::FACADE_ROOT`](crate::markup::codegen::FACADE_ROOT)), so the
facade is the only dependency an app needs. [`codegen::compile`](crate::markup::codegen::compile)
shows what a file becomes:

```
use remedy_ui::markup::codegen;

let rust = codegen::compile("quiet.rui", "switch(label: \"Quiet\", checked: $noise)", &[])?;
assert!(rust.contains("pub struct Bindings"));
assert!(rust.contains("::remedy_ui::markup"));
assert!(rust.contains("pub noise:"));
# Ok::<(), remedy_ui::markup::RuiError>(())
```

The app's own elements are passed to the compiler as [`crate::markup::Custom`] entries:
the element's name and parameters, and the Rust path of its builder.

## People's own layouts

[`crate::markup::Layouts`] reads a person's version of a screen from a folder (the app's
user-config folder; or, while developing, the app's own `ui` sources, for live editing).
[`Layouts::build`](crate::markup::Layouts::build) returns a [`crate::markup::Layout`]:

- with no file of theirs, it shows the built-in screen;
- with a file that loads, it shows theirs, bound to the same signals;
- with a file that does not load, it shows the built-in screen with a notice saying why
  and a Reset button, so the app always opens.

[`Layout::save`](crate::markup::Layout::save) checks a new version before writing it, and
saving and resetting go through the window's undo history.
[`Layout::watch`](crate::markup::Layout::watch) reloads the screen when the file changes on
disk. The app's state lives in its signals, so it carries over a reload.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::markup::{Layouts, SaveError, Scope, Widgets};
use remedy_ui::reactive::Signal;
use remedy_ui::view::A11yAction;
use remedy_ui::widgets as w;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };
# let dir = std::env::temp_dir().join(format!("remedy-guide-layouts-{}", std::process::id()));
# let _ = std::fs::remove_dir_all(&dir);

let noise = Signal::new(false);
let scope = Scope::new().flag("noise_suppression", noise);
// The built-in screen; in an app this is usually a compiled `.rui` file's `build`.
let built_in = move |ui: &remedy_ui::Ui| {
    w::column(ui).child(w::switch(ui, "Noise suppression", noise))
};

let layouts = Layouts::new(&dir, Widgets::new());
let layout = layouts.build(&ui, "settings_voice.rui", &scope, built_in);
ui.root().child(layout.el());
# app.frames(1);
assert!(!layout.is_custom());

// A version that does not fit the screen is refused before it is written.
let refused = layout.save("row { switch(label: \"A\", checked: $nope) }");
assert!(matches!(refused, Err(SaveError::Invalid(_))));

// Their own version: the same signal, a different arrangement.
layout.save("row(gap: s4) { switch(label: \"Quiet\", checked: $noise_suppression) }")?;
# app.frames(1);
assert!(layout.is_custom());
let reader = ui.reader_view();
let Some(quiet) = reader.iter().find(|n| n.name.as_deref() == Some("Quiet")) else {
    unreachable!("their switch is on screen")
};
assert!(ui.accessibility_action(quiet.id, A11yAction::Click));
assert!(noise.get(), "bound to the app's signal");

// Saving is a change in the window's history, so it can be undone.
assert_eq!(ui.next_undo().map(|h| h.label).as_deref(), Some("Change layout settings_voice.rui"));
assert!(ui.undo_change());
# app.frames(1);
assert!(!layout.is_custom());
# let _ = std::fs::remove_dir_all(&dir);
# Ok::<(), Box<dyn std::error::Error>>(())
```

[`Layouts::check`](crate::markup::Layouts::check) checks a version without building it,
for an editor that says what is wrong as people type, and
[`Layout::error`](crate::markup::Layout::error) says why a person's version did not load.

## Themes people bring

A theme is a set of design tokens ([`crate::style::Theme`]). The Remedy theme set is in
[`crate::themes`]: twelve palettes, `System` following the platform's light or dark, and a
high-contrast form. Themes travel as W3C design-tokens files:
[`crate::themes::to_tokens`] writes every token, and [`crate::themes::from_tokens`] reads a
file over a base theme, changing only the tokens the file has. A file can be a whole theme
or a few edits, and the same file works in every Remedy app.

```
use remedy_ui::themes::{ThemeChoice, from_tokens, to_tokens};

let base = ThemeChoice::default().resolve(None, false);
let edits = r##"{ "$name": "Ocean",
                  "color": { "bg": { "$type": "color", "$value": "#0d1b2a" } } }"##;
let ocean = from_tokens(edits, &base)?;
assert_eq!(ocean.name, "Ocean");
assert_eq!(ocean.colors.accent, base.colors.accent, "tokens the file leaves out stay");

// A whole theme round-trips through its file.
let again = from_tokens(&to_tokens(&ocean), &base)?;
assert_eq!(again.colors.bg, ocean.colors.bg);
# Ok::<(), remedy_ui::themes::TokenError>(())
```

[`App::set_theme`](crate::App::set_theme) restyles every window with the new theme. The
widget style sheet ([`crate::themes::sheet`]) is built only from tokens, so a person's edit
to a token restyles every widget that uses it.

## Shortcuts people rebind

[`crate::nav::Commands`] holds the app's commands: an id, a label, default shortcuts, an
optional icon and the action. Keyboard shortcuts, toolbars, menus, the command palette and
agents all reach the same commands. A command's shortcuts come in three layers, the topmost
winning:

- the defaults it was added with;
- the app's own binding, [`Commands::rebind`](crate::nav::Commands::rebind) (restored from the
  app's settings, or a mode), which refuses a clash with another command in the same scope;
  [`Commands::reset`](crate::nav::Commands::reset) takes it away;
- the person's own, [`Commands::set_bindings`](crate::nav::Commands::set_bindings) and
  [`Commands::import_bindings`](crate::nav::Commands::import_bindings), which
  [`crate::customize`] keeps for them (below).

```
use remedy_ui::input::{Chord, Key, Platform};
use remedy_ui::nav::Commands;
use std::cell::Cell;
use std::rc::Rc;

let commands = Commands::new();
let sent = Rc::new(Cell::new(0));
let s = sent.clone();
let ctrl = |c: &str| Chord::primary(Platform::Standard, Key::Character(c.into()));
commands.add("message.send", "Send", vec![ctrl("s")], None, move || s.set(s.get() + 1));
commands.add("message.reply", "Reply", vec![ctrl("r")], None, || {});

// Taken by Reply: refused, and the conflict names it.
let Err(conflict) = commands.rebind("message.send", vec![ctrl("r")]) else {
    unreachable!("Ctrl+R belongs to Reply")
};
assert_eq!(conflict.existing, "message.reply");

commands.rebind("message.send", vec![ctrl("e")]).map_err(|c| c.existing)?;
let Some(info) = commands.info("message.send") else { unreachable!("registered") };
assert_eq!(info.chords.first().map(|c| c.label(Platform::Standard)).as_deref(), Some("Ctrl+E"));

commands.reset("message.send");
let Some(info) = commands.info("message.send") else { unreachable!("registered") };
assert_eq!(info.chords.first().map(|c| c.label(Platform::Standard)).as_deref(), Some("Ctrl+S"));

assert!(commands.run("message.send"));
assert_eq!(sent.get(), 1);

// The person's own binding sits over the app's, and goes to a file and back
// (another machine, a backup).
commands.import_bindings(r#"{ "message.send": ["Ctrl+Shift+S"] }"#).map_err(|e| e.to_string())?;
assert_eq!(commands.user_chords("message.send").map(|c| c.len()), Some(1));
let saved = commands.export_bindings();
assert!(saved.contains("Ctrl+Shift+S"));
commands.set_bindings(Default::default());
assert_eq!(commands.user_chords("message.send"), None, "back to the app's");
commands.import_bindings(&saved).map_err(|e| e.to_string())?;
assert!(commands.import_bindings("not a shortcuts file").is_err(), "and nothing changed");
assert_eq!(commands.user_chords("message.send").map(|c| c.len()), Some(1));
# Ok::<(), String>(())
```

[`Commands::export_bindings`](crate::nav::Commands::export_bindings) writes the person's
bindings as JSON (command id to chords such as `"Ctrl+Shift+Z"`);
[`Commands::import_bindings`](crate::nav::Commands::import_bindings) reads them back and
changes nothing when the text is not a set of bindings. The same format is
[`nav::bindings_to_json`](crate::nav::bindings_to_json) and
[`nav::bindings_from_json`](crate::nav::bindings_from_json).

[`Commands::attach`](crate::nav::Commands::attach) makes the shortcuts work in a window.
The forms pack's [`crate::forms::key_capture`] is the field a shortcut editor uses to record
a new chord.

## Toolbars people arrange

A toolbar shows the commands whose ids are in a `Signal<Vec<String>>` the app keeps (and
saves). [`crate::nav::toolbar_editor`] lets people turn commands on and off and move them,
writing straight into the same signal, so the toolbar follows as they edit.

```
# use remedy_ui::testing::TestApp;
# use remedy_ui::{App, AppConfig, WindowSpec};
use remedy_ui::nav::{Commands, toolbar, toolbar_editor};
use remedy_ui::reactive::Signal;
use remedy_ui::widgets::Icon;
# let config = AppConfig::new("Guide").cpu_only();
# let mut app = TestApp::new(config, |app: &App| {
#     app.open(WindowSpec::new("Guide").size(600.0, 400.0), |_, _| {});
# });
# let Some(ui) = app.ui() else {
#     unreachable!("the window is open")
# };

let commands = Commands::new();
commands.add("file.save", "Save", vec![], Some(Icon::Check), || {});
commands.add("file.print", "Print", vec![], None, || {});

let slot = Signal::new(vec!["file.save".to_owned()]);
let bar = toolbar(&ui, "Main toolbar", &commands, slot);
ui.root().child(bar.clone()).child(toolbar_editor(&ui, &commands, slot));
# app.frames(1);
assert_eq!(bar.child_elements().len(), 1, "Save only");

// The person turns Print on; the editor writes the same signal the toolbar reads.
let reader = ui.reader_view();
let Some(print) = reader.iter().find(|n| n.name.as_deref() == Some("Print")) else {
    unreachable!("the editor lists every command")
};
assert!(ui.accessibility_action(print.id, remedy_ui::view::A11yAction::Click));
# app.frames(1);
assert_eq!(slot.get(), ["file.save", "file.print"]);
assert_eq!(bar.child_elements().len(), 2);
```

## All of it, kept for the person

[`crate::customize`] (feature `customize`) turns these on together and keeps them: the theme,
shortcuts, toolbars and menus, the app's text, and screen layouts, each as a readable file
in a config folder holding only what the person changed. Every change goes into the window's
undo history, each item and each kind has a reset, and a file that does not load is skipped
with a notice and a one-click reset, so the app always starts.
[`App::customizations`](crate::App::customizations) keeps them in the app's data folder and
uses the platform's file dialogs for importing and exporting themes; the app turns on each
kind it supports:

```no_run
use remedy_ui::nav::Commands;
use remedy_ui::{App, AppConfig, WindowSpec};

remedy_ui::run(AppConfig::new("Notes"), |app: &App| {
    let Some(custom) = app.customizations() else { return };
    let commands = Commands::new();
    commands.add("file.save", "Save", vec![], None, || {});
    app.open(WindowSpec::new("Notes"), move |ui, _| {
        custom.enable_theme(ui.theme());
        custom.enable_shortcuts(&commands);
        custom.enable_toolbars(&commands);
        custom.attach(ui);
        let main = custom.toolbar_slot(ui, "main", "Main toolbar", &["file.save"]);
        let _ = main;
        // A settings page with every editor, and the notices for files that did not load.
        ui.root().child(remedy_ui::customize::settings_page(ui, &custom));
    });
})
.ok();
```

| What people change | Kept as (in the config folder) | Editor |
|---|---|---|
| Theme | `theme.json`: design tokens, only those changed | [`customize::theme_editor`](crate::customize::theme_editor) |
| Shortcuts | `shortcuts.json`: command id to chords | [`customize::shortcut_editor`](crate::customize::shortcut_editor) |
| Toolbars and menus | `toolbars.json`: slot to command ids | [`customize::toolbars_editor`](crate::customize::toolbars_editor) |
| Text | `text/<locale>.json` and `text/<locale>.ftl` | [`customize::text_editor`](crate::customize::text_editor) |
| Screen layouts | `layouts/<screen>.rui` | [`customize::layouts_editor`](crate::customize::layouts_editor) |

A theme someone shares is data from a stranger:
[`Customizations::import_theme`](crate::customize::Customizations::import_theme) checks every
token before anything changes and refuses shaders and outside links unless the person turns
them on ([`customize::ImportPolicy`](crate::customize::ImportPolicy)).

## Boards people arrange

Some screens are not laid out by the app at all: a FreeSpeak Home is a board its owner
builds from widgets. [`crate::board`] (feature `board`) holds the board as a document,
renders it for visitors and for the person arranging it (select, drag, drop, every move from
the keyboard too, undo), and edits each widget's settings. Besides built-in widgets and the
app's own kinds, people write scripted widgets in Rhai or compile them to WebAssembly. Each
runs sandboxed, with no files, network or processes and with limits on time and memory, and
returns a description of elements that the board builds as ordinary RemedyUI elements, themed
and accessible like built-in ones.

An app that builds on boards shapes them to its own model:

- **Its words.** Every string a board says is in
  [`board::BoardLabels`](crate::board::BoardLabels) on the context, English by default.
  `BoardLabels::default().translated(|en| catalog.text(en))` sends them all through the
  app's catalog; placeholders such as `{name}` are filled after translation. Built-in
  widgets take the words with
  [`Registry::with_builtins_in`](crate::board::Registry::with_builtins_in).
- **How it behaves.** [`board::BoardOptions`](crate::board::BoardOptions) hold each area to
  one widget, fix the row height, name areas the app's way, and move the panel out:
  with `Panel::Off`, an app showing several boards on a page shows one
  [`board::board_panel`](crate::board::board_panel) for the board being arranged, or drives
  editing itself with `BoardState`'s operations and its `selected` signal.
- **What people may add.** [`Registry::set_addable`](crate::board::Registry::set_addable)
  keeps a kind on screen but out of "Add widget", for a viewer without the right to place it.
- **Typed settings.** Besides text, numbers, switches and choices, a setting can be a moment
  (`SettingKind::Time`, seconds since 1970, with the form pack's date and time pickers), a
  choice whose labels differ from what is stored (`LabeledChoice`, such as rooms by id), a
  list, or a picture the app shows and chooses (`Context::picture`,
  `Context::choose_picture`).
- **One change, not one a keystroke.** [`BoardState::settled`](crate::board::BoardState::settled)
  follows the document, but typing in a setting reaches it once, when the typing finishes.
  An app that sends each change to a server watches `settled` rather than `doc`:

```
# #[cfg(feature = "board")]
# {
use remedy_ui::board::{Board, BoardOptions, BoardState};

let (doc, area) = Board::default().add_area("News", 12, 1)?;
let (doc, note) = doc.add_widget(&area, "text", Default::default())?;
let state = BoardState::with_options(doc, BoardOptions::default().one_widget_per_area(true));

// Typing in a setting is one change for whoever watches `settled`.
for text in ["H", "He", "Hello"] {
    state.type_setting(&note, "title", text.into());
}
assert!(state.is_typing());
state.finish_typing(); // as when the field loses focus
let sent = state.settled.get();
let title = sent.widget(&note).and_then(|w| w.settings.get("title")).cloned();
assert_eq!(title, Some("Hello".into()));
# }
# Ok::<(), remedy_ui::board::BoardError>(())
```
