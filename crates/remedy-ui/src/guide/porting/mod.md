# Porting guides

The Remedy apps are moving onto RemedyUI from four kinds of UI. Each guide maps the ideas you
know onto RemedyUI's, shows the same small pieces side by side (theirs as text, ours as
doctests that compile and run), and lists the habits that cause trouble.

| Guide | For | Apps |
|---|---|---|
| [`egui`](crate::guide::porting::egui) | egui and eframe apps | FreeSpeak |
| [`react`](crate::guide::porting::react) | React apps in Tauri or Capacitor, and PWAs | SecretSticky, SecretFolder, Alex's Craft Calc, GuitarRemedy, VoiceRemedy, the RemedyAI desktop |
| [`qt`](crate::guide::porting::qt) | Qt Widgets, PyQt and PySide apps | RemedyPDF |
| [`web`](crate::guide::porting::web) | pywebview and other HTML-in-a-window apps | Hearth |

## What is the same in every guide

Five ideas carry over from every one of these, and the guides lean on them.

- **Components run once.** A component is a Rust function that builds elements with
  builder methods and returns an [`crate::El`]. It runs when the window or row is made, not
  every frame and not on every change.
- **State is signals.** A [`crate::reactive::Signal`] holds a value. Every property that
  can change takes a value, a signal or a closure ([`crate::view::IntoProp`]):
  `.text("Saved")` sets it once, `.text(move || format!("{} new", count.get()))` follows the
  signals the closure reads. Only that property updates when they change. Signals are `Copy`
  handles: capture them in closures with `move`. Derived values are
  [`crate::reactive::Memo`]s. When nothing changes, nothing runs.
- **Builders, not markup.** Widgets are lowercase functions in [`crate::widgets`]
  (`button`, `row`, `text`), and properties are methods (`.gap(8.0)`, `.variant("primary")`,
  `.on_click(..)`). Screens can also be written as `.rui` markup when an app wants people to
  rearrange them (see [`crate::guide::customization`]).
- **Look comes from theme tokens.** A [`crate::style::Theme`] holds colors by role, the type
  scale, spacing, radii and motion, and typed tokens name them in code
  (`.gap(Space::S3)`, `Radius::R2`, `ColorRole::Accent`; see [`crate::style::Space`]). A [`crate::style::StyleSheet`] says how each widget
  looks in each state, in tokens only, so every Remedy palette, light and dark, and high
  contrast work with no extra code. The shared Remedy themes are [`crate::themes`].
- **The accessibility tree is the test surface.** Every widget declares what it is, its
  name, value and states. Screen readers read that tree, agents act on it, and tests check
  it: [`crate::Ui::reader_view`] gives the window as a screen reader meets it, and
  [`crate::Ui::accessibility_action`] presses, focuses and steps controls as a reader would.

## The pattern the samples use

Most samples on these pages run the app with [`crate::testing::TestApp`], on the in-memory
host instead of the screen, and find what they check by role and accessible name, as a screen
reader would. The same setup function runs on the real platform with [`crate::run`]:

```
use remedy_ui::a11y::Role;
use remedy_ui::reactive::Signal;
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

let greeted = Signal::new(false);
let app = move |app: &App| {
    app.open(WindowSpec::new("Hello"), move |ui, _| {
        ui.root()
            .child(w::heading(ui, 1, "Hello"))
            .child(w::button(ui, "Wave", move || greeted.set(true)));
    });
};

// For real: remedy_ui::run(AppConfig::new("Hello"), app)
let mut app = TestApp::new(AppConfig::new("Hello"), app);
assert!(app.find(Role::Heading(1), "Hello").is_some());
assert!(app.press(Role::Button, "Wave"));
assert!(greeted.get());
```

Read [`crate::guide::getting_started`] and [`crate::guide::signals`] first if you have not.
