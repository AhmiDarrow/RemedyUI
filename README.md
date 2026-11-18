# RemedyUI

RemedyUI is the UI engine for every Remedy app, written in Rust. That covers chat (FreeSpeak), documents, notes, audio and music tools, file and secret managers, calculators, media apps, and the RemedyOS desktop. It is built to the standard of the leading UI engines (Flutter, SwiftUI, Jetpack Compose, WinUI 3, Qt Quick, the web platform). Its aims, in order: stable, secure, fast as the hardware allows, light, intuitive for the people using it, and a pleasure to build with.

**Status:** milestones M1 to M5 are built, M6 to M9 are built in large part, and M10 (1.0) is under way: the API review is done ([docs/API-REVIEW.md](docs/API-REVIEW.md)), and the guides, porting guides and recipes are written. [docs/ROADMAP.md](docs/ROADMAP.md) has the details. SecretSticky, SecretFolder and Hearth run on RemedyUI (each repository's `remedyui` branch).

## In short

- **Retained tree with fine-grained signals.** Components run once. A state change updates only what reads it, and nothing runs while nothing changes.
- **A compositor thread.** Scrolling, animation and video stay at the display's rate even when app code is busy.
- **Modular.** Feature packs for data tables, documents, code, terminals, music widgets, media, charts and platform services. An app links only what it uses, and what it links starts only when first used.
- **Real text.** Full shaping and bidi, variable and color fonts, hinting with grayscale and LCD, rich paragraphs, selection across elements, rich editing, each platform's real IME, spell checking.
- **Two renderers, one display list.** A wgpu UI renderer with precompiled pipelines, and a fast CPU renderer as the floor. It recovers from a lost GPU and presents only what changed.
- **Code fluid.** Users can restyle, rebind and rearrange the apps they use: themes, shortcuts and toolbars, and `.rui` layouts where an app allows it. One theme set is shared by every Remedy app.
- **Every platform as a bridge.** Windows, macOS, Linux (Wayland and X11), Android and browser pages, over one shared core.
- **Accessible, and operable by agents with consent.** One semantics tree serves screen readers, tests and AI agents.
- **Safe.** No unsafe code outside the `os`/`ffi` modules of the bridges, the GPU renderer and the few packs that call the platform themselves (updates, desktop services, spelling, media playback), checked in CI. Untrusted content is parsed by fuzzed, memory-safe code off the UI thread, optionally in a sandboxed helper. Secret inputs never leak, and there is no telemetry.

## Quick start

Install `cargo remedy`, make an app and run it:

```text
cargo install --git https://github.com/AhmiDarrow/RemedyUI cargo-remedy
cargo remedy new my-app          # templates: basic, tray, multi-window, document, android
cd my-app
cargo remedy run                 # F12 in a window opens the inspector
cargo test                       # the app on the in-memory host, and its golden previews
cargo remedy package             # Windows installer and zip; --target linux, android or macos
```

An app is a function that opens windows. Components run once and bind elements to signals:

```rust,no_run
use remedy_ui::reactive::Signal;
use remedy_ui::widgets as w;
use remedy_ui::{App, AppConfig, WindowSpec};

fn main() {
    let result = remedy_ui::run(AppConfig::new("Counter"), |app: &App| {
        app.open(WindowSpec::new("Counter"), |ui, _| {
            let count = Signal::new(0);
            ui.root()
                .child(w::text(ui, move || format!("Clicked {} times", count.get())))
                .child(w::button(ui, "Click", move || count.update(|c| *c += 1)).variant("primary"));
        });
    });
    if let Err(e) = result {
        eprintln!("{e}");
    }
}
```

## The guide

The guides, porting guides and recipes are part of the API docs, in `remedy_ui::guide`. Every Rust sample in them is a doctest, so `cargo test` keeps them true. Open them with:

```text
cargo doc -p remedy-ui --all-features --open
```

- **Guides:** getting started, stability and editions, thinking in signals (properties, context, persisted state), showing, failing and loading (`show`, error boundaries, suspense and resources), layout, theming and typed tokens, text and input methods, lists and virtual lists, motion and reduced motion, accessibility, testing, packaging and updates, performance, customization (`.rui` markup and people's own layouts), agents, media, documents, the terminal, and running in a browser.
- **Porting guides** (`remedy_ui::guide::porting`): from egui, from React, from Qt or PyQt, and from pywebview or HTML apps, with their code beside ours.
- **Recipes** (`remedy_ui::guide::recipes`): chat feed, file explorer, document viewer, settings page, tray app, sticky-note windows, audio meter and unlock screen. Each is a runnable example in [crates/remedy-ui/examples](crates/remedy-ui/examples) (`cargo run -p remedy-ui --example chat_feed`) with a headless test in [crates/remedy-ui/tests/recipes.rs](crates/remedy-ui/tests/recipes.rs).

## What is built

Apps use the facade, `remedy-ui` (`run`, `App`, `Window`, and the packs behind features). Each line below is a crate with real code and tests; a crate joins the workspace only when it has both.

**Foundations**
- `remedy-ui-core`: geometry, scale and snapping, colors, generational arena
- `remedy-ui-reactive`: glitch-free signals, memos, effects, scopes, context, resources fetched off the UI thread, persisted signals, `Waker`, `SignalVec`, stores, and the signal graph for devtools
- `remedy-ui-macros`: `#[derive(Store)]`, `#[preview]` and `#[previews]`
- `remedy-ui-tree`: elements, dirty flags and boundaries, events, hit testing, focus, error boundaries
- `remedy-ui-trace`: the phase spans every layer emits, and their clock
- `remedy-ui-test`: the low-level headless harness

**Text, layout, style, input, accessibility, motion**
- `remedy-ui-fonts`: the Remedy families as data
- `remedy-ui-text`: rich paragraph layout over parley with bidi and inline objects; hinted grayscale and LCD glyph rasterization; the glyph atlas; the editing core with undo and IME composition; secret text; deceptive-text defenses
- `remedy-ui-layout`: flex, grid and block through taffy, inherited RTL, pixel snapping, virtual lists with anchoring
- `remedy-ui-style`: themes as tokens, typed tokens (`Space`, `Radius`, `TextRole`, `ColorRole`), contrast-balanced derived tokens, cached style resolution
- `remedy-ui-themes`: the twelve Remedy palettes, light and dark from the system, high contrast, design-token files, the widget style sheet
- `remedy-ui-input`: the event model, gesture arena, scroll physics, commands with rebindable shortcuts
- `remedy-ui-a11y`: semantics to incremental AccessKit updates, merging, protected fields, live regions
- `remedy-ui-motion`: easing, tweens, closed-form springs with interruption, the frame clock

**Rendering and the compositor**
- `remedy-ui-paint`: display lists of shapes, paths, gradients, glyph runs, images, clips, layers and shadows
- `remedy-ui-render-cpu`: the CPU renderer over vello_cpu with RemedyUI's own glyphs, including LCD text; the reference for golden images
- `remedy-ui-render-gpu`: the wgpu renderer over vello_gpu with the same painter, a backend ladder, a pipeline cache, window surfaces and device-loss recovery
- `remedy-ui-compositor`: layer trees, tiles, damage, compositor-side scrolling and animation, progressive zoom, and the compositor thread

**The view and the platforms**
- `remedy-ui-view`: elements, the frame pipeline, input routing, editable fields (plain, secret, rich and code), properties that take a value, a signal or a closure, `show`, error and suspense boundaries, element animation on the frame clock, formatted text on the clipboard, links inside text, selection across elements, the window's undo history, drag and drop within the app and with other apps, semantics
- `remedy-ui-host`: the platform-neutral host model every bridge implements
- `remedy-ui-bridge-headless`: the host in memory, for tests, previews and servers
- `remedy-ui-bridge-winit`: the shared core for the desktop and Android bridges
- `remedy-ui-bridge-windows`, `remedy-ui-bridge-macos`, `remedy-ui-bridge-linux`: clipboard, capture protection, system settings, the tray, file dialogs, single instance, screen readers, and TSF on Windows
- `remedy-ui-bridge-web`: apps in a browser page or PWA on `wasm32-unknown-unknown`, drawn on a canvas, with the page's clipboard, links, input methods, settings and screen readers
- `remedy-ui-bridge-android`: a native activity with input methods, the clipboard, links, capture protection, the safe area, back, the lifecycle, memory trims, system settings and screen readers; a Gradle-free APK build
- `remedy-ui`: the facade and runner; windows start on the CPU renderer and move to the GPU when they need it; window geometry kept per key, full-screen windows, and content that pops out to its own window

**Widgets**
- `remedy-ui-widgets-core`: containers, lists and virtual lists, text, buttons, choices, values, inputs, feedback, overlays, the trusted prompt, the lock screen, icons and images
- `remedy-ui-widgets-forms`: exact-decimal number input, color picker with contrast readout, key capture, calendar, date and time pickers, forms with validation and submit state, all in the user's locale
- `remedy-ui-widgets-nav`: commands and shortcuts, tabs, nav rail, breadcrumbs, a navigation stack, user-arranged toolbars, the command palette, drawers, sheets and a custom title bar
- `remedy-ui-widgets-data`: tables, icon grids, trees, property grids and chat lists, virtual at any size, with selection, editing and drag and drop
- `remedy-ui-widgets-board` and `remedy-ui-script`: boards people arrange, with sandboxed scripted widgets behind one language-neutral interface: `remedy-ui-script-rhai` for widgets written in Rhai, `remedy-ui-script-wasm` for widgets compiled to WebAssembly; both measure their own speed on the device they run on
- `remedy-ui-widgets-extra`: QR codes, rating, carousel, a setup wizard flow and swatch grids
- `remedy-ui-widgets-music`: fretboard, chord diagram, tab staff and note staff
- `remedy-ui-widgets-term`: the terminal element: VT and xterm emulation with true color, the kitty keyboard protocol, sixel pictures, reflow, scrollback sized from the machine, selection, links, input methods and screen readers; the PTY stays in the app
- `remedy-ui-viz`: the speaking ring, level meters, sparklines and bars

**Documents**
- `remedy-ui-doc-rich`: Markdown, a rich text editor with links and mentions that saves Markdown, and a find bar
- `remedy-ui-doc-pages`: paged documents over pages the app supplies, zooming from 1% to 6400% with tiles rendered off the UI thread, spreads, rotation, selection, search, thumbnails, annotations and screen reader support
- `remedy-ui-doc-code`: syntax highlighting, diffs, code views and a code editor with several cursors

**Media and embedding**
- `remedy-ui-media-image` and `remedy-ui-media-svg`: images decoded off the UI thread at the size shown, color managed; SVG at exactly the size shown, and tinted icons
- `remedy-ui-media-anim`: GIF, APNG and animated WebP
- `remedy-ui-media-isolated`: images and animations decoded in a sandboxed helper process
- `remedy-ui-media-video`: video surfaces fed from any thread, converted on the GPU
- `remedy-ui-media-playback`: sound and video through the platform's decoders, kept in step with the sound clock, the `player` widget, and WebVTT, SRT and ASS captions
- `remedy-ui-embed-gpu`: the app's own wgpu rendering as an element
- `remedy-ui-embed-webview`: the platform's web view as an element, with an allow-list for every navigation and one JSON message channel

**Customization, agents and tools**
- `remedy-ui-rui`: `.rui` markup, compiled at build time or loaded at run time, and people's own layouts
- `remedy-ui-customize`: what people change about an app (their theme, shortcuts, toolbars and menus, text and screen layouts) as readable files with undo, resets and safe loading, and the editors for them
- `remedy-ui-automation`: agents reading and operating a window under the person's consent, with every change undoable
- `remedy-ui-devtools`: the inspector (F12) with the accessibility check, the signal graph, the performance overlay, Perfetto traces, remote devtools, and previews that double as golden-image tests
- `remedy-ui-secure`: idle lock and secret copies that clear themselves
- `cargo-remedy`: `cargo remedy new` (basic, tray, multi-window, document and android templates), `run` (devtools, `--watch`), `package` (Windows installer and zip with signing and update signatures, Linux `.tar.gz`, Android APK, macOS `.app`) and `migrate` (applies the renames between releases)

**Services**
- `remedy-ui-services-update`: signed updates from GitHub releases or a feed
- `remedy-ui-services-i18n`: Fluent messages, source-keyed JSON catalogs, locale data and exact decimals
- `remedy-ui-services-spell`: spell checking with the platform's checker or Hunspell dictionaries
- `remedy-ui-services-desktop`: starting with the session, showing files in the file manager, the user's known folders

Checks: `cargo xtask check` (layer graph and unsafe boundary) and `cargo deny check`, alongside fmt, clippy, tests and docs.

## Documents

| Document | What it holds |
|---|---|
| [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) | The bar: what a modern UI engine must provide, what the Remedy apps add, and the egui failures never to repeat |
| [docs/DESIGN.md](docs/DESIGN.md) | The overview: principles, engine shape, crates, stability, and the index of deep dives |
| [docs/design/](docs/design/) | Sixteen deep dives, from threads and state to text, rendering, security, accessibility, developer experience and the design language |
| [docs/APPS.md](docs/APPS.md) | Every Remedy app mapped to the packs and bridges it needs, and the order the apps move over |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones M1 to M10, each with a tested definition of done, and from M5 an app moved onto the engine |
| [docs/dependencies.md](docs/dependencies.md) | The third-party crates, pinned versions, and the rules for adding one |
| [docs/STABILITY.md](docs/STABILITY.md) | What 1.0 promises: semver, deprecation, editions, MSRV, which crates are stable |
| [docs/RELEASING.md](docs/RELEASING.md) | The release checks: the gate, `cargo xtask semver` and `msrv`, packaging |
| [docs/API-REVIEW.md](docs/API-REVIEW.md) | The 1.0 API review: every rename, error and growth change, and why |
| [CHANGELOG.md](CHANGELOG.md) | What changed, by milestone and release |

## License

RemedyUI is licensed under either of

- the [Apache License, Version 2.0](LICENSE-APACHE), or
- the [MIT License](LICENSE-MIT),

at your option. This is the usual license pair for Rust libraries. It works with FreeSpeak's AGPL-3.0, RemedyOS's Apache-2.0 and closed-source apps alike, and Apache-2.0 adds a patent grant.

Unless you say otherwise, any contribution you submit for inclusion in RemedyUI is licensed as above, without any additional terms or conditions.

Hope this helps
- Ahmi
