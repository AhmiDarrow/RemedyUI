# 12. Developer experience

An engine is only as good as the apps people manage to build on it. RemedyUI should feel as direct as SwiftUI, as previewable as Compose and as tooled as Flutter, while staying plain Rust.

## API style

- **Plain Rust first.** Components are functions and elements are builders. There is no required macro language, no lifetimes in user-facing types, and no `Rc<RefCell<…>>` dance: signals are `Copy` handles.
- **One obvious way.**
  - A widget is a lowercase function: `button`, `text`, `row`.
  - A property is a builder method named after what it is: `.padding()`, `.enabled()`, `.on_click()`.
  - Every property accepts a constant, a signal or a closure (`impl IntoProp<T>`), so making something reactive never means changing its shape.
- **Names read like the UI.** It is `show(cond, || ...)`, not `ConditionalRenderNode`. Events are `on_<event>`, and commands are verbs.
- **Typed tokens.** `Space::S3`, `Radius::R2`, `TextStyle::Body`, `Color::Accent`. Autocomplete shows the design system, and typos fail to compile.
- **No hidden globals.** Everything flows through context, so tests and previews can supply their own theme, locale, clock and services.
- **Errors at compile time where possible.** Required properties are part of the constructor. Invalid combinations are separate types: a `slider` cannot take a `placeholder`.

```rust
fn unlock_screen(vault: Vault) -> impl Element {
    let password = Signal::new(SecretString::new());
    let error = Signal::new(None::<String>);
    column().gap(Space::S3).align(Center).max_width(360.0)
        .child(text(t("Unlock vault")).style(TextStyle::Title))
        .child(secret_input(password).label(t("Master password")).autofocus())
        .child(show(move || error.get().is_some(), move || text(error.get().unwrap_or_default()).color(Color::Danger)))
        .child(button(t("Unlock")).variant(Variant::Primary).on_click(move || {
            if let Err(e) = vault.unlock(password.take()) { error.set(Some(e.to_string())) }
        }))
}
```

## Getting started

- **`cargo remedy new my-app --template <basic|document|tray|multi-window|android>`** creates a working app: a window, a theme, a localized string, a test and a preview, with packaging ready.
- **`cargo remedy run`** builds and runs with hot reload of `.rui`, themes and message files, and fast incremental rebuild of Rust code.
- **`cargo remedy package`** produces signed installers and bundles: MSIX or NSIS on Windows, `.app` and `.dmg` on macOS, AppImage, `.deb` and Flatpak on Linux, AAB or APK on Android, and a RemedyOS package. It also generates icons from one SVG and wires up auto-update from GitHub releases (an optional updater crate that most Remedy apps already need).
- **Widget gallery.** `cargo remedy gallery` opens every widget in every state and variant, with its code and the theme tokens it uses. It doubles as visual documentation.

## Previews

```rust
#[preview(theme = "forest", size = (400, 600), locales = ["en", "ar"])]
fn unlock_screen_preview() -> impl Element { unlock_screen(Vault::fake_locked()) }
```

- **Where previews show.** Previews render in the devtools window, and in IDEs through a small language-server extension. They update on save.
- **Variations.** A preview can list themes, locales (including an RTL and a pseudo-localized long-text locale), text scales and window sizes, and shows all variations side by side.
- **Previews are tests.** Each preview is also a golden-image test in CI.

## Devtools

- **Inspector.** Pick an element on screen to see its tree position, source location (file and line where it was built), layout box and constraints, resolved style and the tokens behind it, semantics, and the signals it reads. You can edit a token or property live.
- **Signal graph.** See what depends on a signal and why something re-rendered: "row 42 repainted because `message.edited` changed".
- **Performance overlay and Perfetto traces** ([09](09-performance-and-resources.md)).
- **Accessibility view.** The semantics tree as a screen reader sees it, with problems flagged: unnamed buttons, low contrast, unreachable controls.
- **Remote devtools** for Android and RemedyOS devices over a local, authenticated connection, which is debug builds only by default. The app serves a window on a loopback port; a connection opens with the server's token on its own line, then sends one JSON request per line (tree, element, semantics, signals, updates, frames) and gets one JSON answer per line. A device is reached through a port forward (`adb forward tcp:PORT tcp:PORT`).

## Diagnostics

- **Misuse is caught early, with the fix.** Reading a signal outside a tracking scope where it would never update; a list without keys; an unbounded size inside a scroll view; a button with no accessible name. In debug builds each gives a clear message with the source location and the suggested change.
- **Panics are contained, with context.** A panicking handler shows the element path, the command or event, and the source location in the log and the inspector. The rest of the app keeps running.
- **Release builds stay quiet**, logging each problem once, with no content (see [08](08-security-and-privacy.md)).

## Testing

- **Unit tests for components** run headless in milliseconds, with semantic finders, scripted input (including IME and gestures), a fake clock to step animations, and assertions on the tree, semantics or pixels ([11](11-accessibility-and-automation.md)).
- **Golden images** come from previews, rendered by the CPU renderer for deterministic pixels.
- **Fakes** for every host service: clipboard, dialogs, notifications, time, locale, memory pressure, device loss. Tests can cover the cases users hit and developers rarely see.

## Documentation

- **API docs.** Every public item has docs with a runnable example, enforced by `#![deny(missing_docs)]` and doc-tests.
- **Guides** cover thinking in signals, layout, theming, text, lists, motion, accessibility, testing, packaging and performance.
- **Porting guides.** One guide each for moving from egui, from React (most Remedy apps today), from Qt or PyQt (RemedyPDF), and from pywebview or HTML apps (Hearth, Quickcast, TheOven). Each has side-by-side equivalents.
- **Recipes**: chat feed, file explorer, document viewer, settings page, tray app, sticky-note windows, audio meter, unlock screen.
