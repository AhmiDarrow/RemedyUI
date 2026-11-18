# RemedyUI

RemedyUI is the UI engine for every Remedy app, written in Rust. That covers chat (FreeSpeak), documents, notes, audio and music tools, file and secret managers, calculators, media apps, and the RemedyOS desktop. It is built to the standard of the leading UI engines (Flutter, SwiftUI, Jetpack Compose, WinUI 3, Qt Quick, the web platform). Its aims, in order: stable, secure, fast as the hardware allows, light, intuitive for the people using it, and a pleasure to build with.

**Status:** M1 (foundations), M2 (text), M3 (layout, style, input, semantics, motion), M4 (rendering, the compositor and the platform bridges) and M5 (the first app, SecretSticky) are built. These crates exist, with real code and tests:

- `remedy-ui-core`: geometry, scale and snapping, colors, generational arena
- `remedy-ui-reactive`: glitch-free signals, memos, effects, scopes, `Waker`, `SignalVec`, stores
- `remedy-ui-macros`: `#[derive(Store)]`
- `remedy-ui-tree`: elements, dirty flags and boundaries, events, hit testing, focus, error boundaries
- `remedy-ui-test`: the headless harness
- `remedy-ui-fonts`: the Remedy families as data
- `remedy-ui-text`: rich paragraph layout over parley with bidi and inline objects; hinted grayscale and LCD glyph rasterization; the glyph atlas; the editing core with undo and IME composition; secret text; deceptive-text defenses

- `remedy-ui-layout`: flex, grid and block through taffy, inherited RTL, pixel snapping, virtual lists with anchoring
- `remedy-ui-style`: themes as tokens (Dark Forest from the shared Remedy palette), contrast-balanced derived tokens, cached style resolution
- `remedy-ui-input`: the event model, gesture arena, scroll physics, commands with rebindable shortcuts
- `remedy-ui-a11y`: semantics to incremental AccessKit updates, merging, protected fields, live regions
- `remedy-ui-motion`: easing, tweens, closed-form springs with interruption, the frame clock

- `remedy-ui-paint`: display lists of shapes, paths, gradients, glyph runs, images, clips, layers and shadows
- `remedy-ui-render-cpu`: the CPU renderer over vello_cpu with RemedyUI's own glyphs, including LCD text; the reference for golden images
- `remedy-ui-render-gpu`: the wgpu renderer over vello_gpu with the same painter, a backend ladder, a pipeline cache, window surfaces and device-loss recovery
- `remedy-ui-compositor`: layer trees, tiles, damage, compositor-side scrolling and animation, and the compositor thread
- `remedy-ui-host`: the platform-neutral host model every bridge implements
- `remedy-ui-bridge-headless`: the host in memory, for tests, previews and servers
- `remedy-ui-bridge-winit`: the shared desktop core for the Windows, macOS and Linux bridges
- `remedy-ui-bridge-windows`, `remedy-ui-bridge-macos`, `remedy-ui-bridge-linux`: clipboard, capture protection, system settings, the tray, file dialogs, single instance, screen readers, and TSF on Windows

M5 added:

- `remedy-ui-view`: elements, the frame pipeline, input routing, editable fields, semantics
- `remedy-ui`: the facade and runner (`run`, `App`, `Window`); windows start on the CPU renderer and move to the GPU when they need it
- `remedy-ui-themes`: the twelve Remedy palettes, light/dark from the system, high contrast, the widget style sheet
- `remedy-ui-widgets-core`: containers, lists, text, buttons, choices, values, inputs, feedback, overlays, icons and images
- `remedy-ui-widgets-forms`: exact-decimal number input, color picker with contrast readout, key capture, calendar, date and time pickers, forms with validation and submit state, all in the user's locale
- `remedy-ui-devtools`: the inspector (F12) with an accessibility check and live theme colors, and previews in many variations that double as golden-image tests
- `remedy-ui-secure`: idle lock and secret copies that clear themselves
- `remedy-ui-services-update`: signed updates from GitHub releases or a feed
- `cargo-remedy`: `cargo remedy new` (basic, tray and multi-window templates), `run` (devtools, `--watch`) and `package` (Windows installer, portable zip, signing, update signatures)

M6 (in progress) added:

- `remedy-ui-widgets-nav`: commands and shortcuts, tabs, nav rail, breadcrumbs, a navigation stack, user-arranged toolbars, the command palette, drawers, sheets and a custom title bar
- `remedy-ui-widgets-data`: tables, icon grids, trees and property grids, virtual at any size, with selection, editing and drag and drop
- `remedy-ui-media-image` and `remedy-ui-media-svg`: images decoded off the UI thread at the size shown, color managed; SVG at exactly the size shown, and tinted icons
- `remedy-ui-services-i18n`: Fluent messages, source-keyed JSON catalogs, locale data and exact decimals
- `remedy-ui-services-desktop`: starting with the session, showing files in the file manager, the user's known folders
- Drag and drop within the app, out to other apps and in from them

M7 (in progress) added:

- `remedy-ui-doc-rich`: Markdown, a rich text editor with links and mentions that saves Markdown, and a find bar
- `remedy-ui-widgets-board` and `remedy-ui-script`: boards people arrange, with widgets written in sandboxed Luau
- `remedy-ui-viz`: the speaking ring, level meters, sparklines and bars
- `remedy-ui-services-spell`: spell checking with the platform's checker or Hunspell dictionaries
- `remedy-ui-media-anim`: GIF, APNG and animated WebP
- `remedy-ui-media-isolated`: images and animations decoded in a sandboxed helper process (AppContainer on Windows, seccomp-bpf and Landlock on Linux, Seatbelt on macOS), so a decoder that crashes, hangs or runs away with memory takes down only the helper
- Rich text fields, formatted text on the clipboard, links inside text, chat lists, selection across messages, and content that pops out to its own window

SecretSticky, SecretFolder and Hearth run on RemedyUI (each repository's `remedyui` branch). A crate is added to the workspace only when it has real code and tests.

Checks: `cargo xtask check` (layer graph and unsafe boundary) and `cargo deny check`, alongside fmt, clippy, tests and docs.

## In short

- **Retained tree with fine-grained signals.** Components run once. A state change updates only what reads it, and nothing runs while nothing changes.
- **A compositor thread.** Scrolling, animation and video stay at the display's rate even when app code is busy.
- **Modular.** Feature packs for data tables, documents, code, terminals, music widgets, media, charts and platform services. An app links only what it uses, and what it links starts only when first used.
- **Real text.** Full shaping and bidi, variable and color fonts, hinting with grayscale and LCD, rich paragraphs, selection across elements, rich editing, each platform's real IME, spell checking.
- **Two renderers, one display list.** A wgpu UI renderer with precompiled pipelines, and a fast CPU renderer as the floor. It recovers from a lost GPU and presents only what changed.
- **Code fluid.** Users can restyle, rebind and rearrange the apps they use: themes, shortcuts and toolbars, and `.rui` layouts where an app allows it. One theme set is shared by every Remedy app.
- **Every platform as a bridge.** Windows, macOS and Linux (Wayland and X11) today over one shared core, then Android, RemedyOS, and later the web and iOS.
- **Accessible, and operable by agents with consent.** One semantics tree serves screen readers, tests and AI agents.
- **Safe.** No unsafe code outside the `os`/`ffi` modules of the bridges and the GPU renderer, checked in CI. Untrusted content is parsed by fuzzed, memory-safe code off the UI thread, optionally in a sandboxed helper. Secret inputs never leak, and there is no telemetry.

## Documents

| Document | What it holds |
|---|---|
| [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) | The bar: what a modern UI engine must provide, what the Remedy apps add, and the egui failures never to repeat |
| [docs/DESIGN.md](docs/DESIGN.md) | The overview: principles, engine shape, crates, stability, and the index of deep dives |
| [docs/design/](docs/design/) | Sixteen deep dives, from threads and state to text, rendering, security, accessibility, developer experience and the design language |
| [docs/APPS.md](docs/APPS.md) | Every Remedy app mapped to the packs and bridges it needs, and the order the apps move over |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones M1 to M10, each with a tested definition of done, and from M5 an app moved onto the engine |
| [docs/dependencies.md](docs/dependencies.md) | The third-party crates, pinned versions, and the rules for adding one |

## License

RemedyUI is licensed under either of

- the [Apache License, Version 2.0](LICENSE-APACHE), or
- the [MIT License](LICENSE-MIT),

at your option. This is the usual license pair for Rust libraries. It works with FreeSpeak's AGPL-3.0, RemedyOS's Apache-2.0 and closed-source apps alike, and Apache-2.0 adds a patent grant.

Unless you say otherwise, any contribution you submit for inclusion in RemedyUI is licensed as above, without any additional terms or conditions.

Hope this helps
- Ahmi
