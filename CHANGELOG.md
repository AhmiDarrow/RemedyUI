# Changelog

All notable changes to RemedyUI are in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and RemedyUI follows
[Semantic Versioning](https://semver.org/) from 1.0 ([docs/STABILITY.md](docs/STABILITY.md)).
Before 1.0, minor versions may break; each break is listed under "Changed" or "Removed"
with what to do, and `cargo remedy migrate` applies the mechanical ones.

## [Unreleased]

Everything since the start of the project, by milestone ([docs/ROADMAP.md](docs/ROADMAP.md)).

### Added

- **M1, foundations.** `remedy-ui-core` (geometry, scale and pixel snapping, colors in sRGB,
  linear light and Oklab, generational arenas, constant-time tokens); `remedy-ui-reactive`
  (glitch-free signals, memos, effects, scopes, `SignalVec`, stores with field-level
  reactivity, the `Waker` for posts from other threads); `remedy-ui-macros`
  (`#[derive(Store)]`); `remedy-ui-tree` (elements, dirty flags, events, hit testing, focus,
  error boundaries); the headless test harness; the workspace rules (lints, layer graph,
  unsafe boundary, `cargo deny`).
- **M2, text.** `remedy-ui-fonts` (the Remedy families as data); `remedy-ui-text` (rich
  paragraph layout over parley with bidi and inline objects, hinted grayscale and LCD glyph
  rasterization, the glyph atlas, the editing core with undo and IME composition, secret
  text, deceptive-text defenses).
- **M3, layout, style, input and semantics.** `remedy-ui-layout` (flex, grid and block through
  taffy, right-to-left, snapping, virtual lists with anchoring); `remedy-ui-style` (themes as
  tokens, contrast-balanced derived tokens, cached resolution); `remedy-ui-input` (the event
  model, gesture arena, scroll physics, rebindable commands); `remedy-ui-a11y` (semantics to
  incremental AccessKit updates); `remedy-ui-motion` (easing, tweens, springs, the frame clock).
- **M4, rendering, the compositor and the bridges.** `remedy-ui-paint` (display lists);
  `remedy-ui-render-cpu` (vello_cpu with RemedyUI's glyphs, LCD text); `remedy-ui-render-gpu`
  (wgpu and vello_gpu, a backend ladder, a pipeline cache, device-loss recovery);
  `remedy-ui-compositor` (layer trees, tiles, damage, compositor-side scrolling and
  animation, its own thread); `remedy-ui-host` (the platform-neutral host model);
  `remedy-ui-bridge-headless`; `remedy-ui-bridge-winit` and the Windows (with TSF), macOS and
  Linux bridges (clipboard, capture protection, settings, tray, file dialogs, single
  instance, screen readers).
- **M5, the first app (SecretSticky).** `remedy-ui-view` (elements, the frame pipeline, input
  routing, editable fields); the `remedy-ui` facade and runner (CPU-first windows that move
  to the GPU when they need it); `remedy-ui-themes` (the twelve Remedy palettes, light and
  dark, high contrast); `remedy-ui-widgets-core`; `remedy-ui-widgets-forms` (exact numbers,
  colors, key capture, dates, times, forms); `remedy-ui-devtools` (the inspector, previews as
  golden images); `remedy-ui-secure` (idle lock, self-clearing secret copies, privacy cover);
  `remedy-ui-services-update` (signed updates); `cargo-remedy` (`new`, `run`, `package` for
  Windows).
- **M6, data apps (SecretFolder, Hearth).** `remedy-ui-widgets-nav` (commands, tabs, rails,
  stacks, toolbars people arrange, the command palette, drawers, sheets, a custom title bar);
  `remedy-ui-widgets-data` (tables, grids, trees and property grids at any size);
  `remedy-ui-media-image` and `remedy-ui-media-svg`; `remedy-ui-services-i18n` (Fluent,
  locale data, exact decimals); `remedy-ui-services-desktop`; drag and drop within the app
  and with other apps, Linux included.
- **M7, FreeSpeak.** `remedy-ui-doc-rich` (Markdown, the rich text editor, find);
  `remedy-ui-viz` (the speaking ring, meters, sparklines, bars); `remedy-ui-services-spell`;
  `remedy-ui-media-anim`; `remedy-ui-media-isolated` (decoding in a sandboxed helper on every
  desktop platform); `remedy-ui-media-video`; boards people arrange with scripted widgets
  (Rhai and WebAssembly script engines behind one interface); chat lists, selection across
  messages, pop-out windows; `remedy-ui-bridge-android`.
- **M8, breadth.** `remedy-ui-doc-pages` (paged documents with progressive zoom, spreads,
  selection, search, thumbnails, annotations); `remedy-ui-doc-code` (code views and the
  editor with several cursors); `remedy-ui-media-playback` (the platforms' decoders, the
  player, captions); `remedy-ui-widgets-music`; `remedy-ui-widgets-extra`; full-screen
  windows; `remedy-ui-bridge-web` (apps in a browser page, with `remedy_ui_core::exec` and
  `remedy_ui_core::time` so the engine runs without threads there).
- **M9, power tools.** `remedy-ui-automation` (agents with consent); `remedy-ui-rui` (`.rui`
  markup compiled at build time or loaded for people's own layouts); `remedy-ui-embed-gpu`;
  `remedy-ui-embed-webview`; `remedy-ui-widgets-term`; `remedy-ui-customize` (themes,
  shortcuts, toolbars, menus, text and layouts people change, with undo).
- **M10, the 1.0 review** ([docs/API-REVIEW.md](docs/API-REVIEW.md)):
  - Editions: `remedy_ui::Edition` (`E2026`), `AppConfig::edition`, `Ui::edition`;
    `cargo remedy new` writes the newest edition.
  - `cargo remedy migrate` (`--dry-run`) with `crates/cargo-remedy/migrations.toml`.
  - `cargo xtask semver --baseline <rev>` (cargo-semver-checks 0.51.0) and `cargo xtask msrv`;
    `cargo xtask check` also checks publishing metadata. [docs/RELEASING.md](docs/RELEASING.md)
    and [docs/STABILITY.md](docs/STABILITY.md).
  - `remedy_ui::testing::TestApp`: tests find, press and type by role and accessible name.
  - `remedy_ui::a11y`, `remedy_ui::paint`, `remedy_ui::Store` (feature `macros`) and
    `remedy_ui::isolated` (feature `isolated`) through the facade.
  - `button_dyn`, `nav_item_dyn`, `App::tray_dyn`, `App::background_cancellable` and
    `BackgroundJob::cancel`, `Commands::export_bindings` / `import_bindings`,
    `Window::attach_agents`, `Agents::request_access`.
  - Constructors and setters for every type that became `#[non_exhaustive]`
    (`StylePatch::new()`, `KeyEvent::press`, `PointerEvent::new`, a setter per options field).
  - `cargo remedy new --template document|android`; `cargo remedy package --target
    linux|android|macos`.
  - Typed errors: `PngError`, `DesktopError`, `BackendError`, `PrepareError`, `Rejection`,
    `BindingsError`, `ParseEditionError`.

### Changed

Breaking before 1.0; `cargo remedy migrate` applies the renames marked (m).

- `remedy_ui::text::StylePatch` is `TextStylePatch` (m); `remedy_ui::view::TextPatch` is
  `TextStylePatch` (m).
- Boolean states read `is_*`: `HeadlessWindow::{is_maximized, is_fullscreen, is_minimized,
  is_frame_requested}` (m), `HeadlessHost::has_exited` (m), `Player::{is_muted,
  is_fullscreen}` (m), `WebView::is_loading` (m), `Ui::is_occluded` (m),
  `Terminal::is_monospace` (m).
- `Ui::set_time(ms: u64)` is `Ui::set_clock(Duration)`, and the runner now sets it (undo
  steps group by time as they always should have).
- `widgets::label(ui, s, variant)` is deprecated for `widgets::text(ui, s).variant(variant)`
  (m).
- `remedy_ui::RunError` is an enum (`SingleInstance`, `Bridge`, `UnsupportedPlatform`,
  `NeedsFeature`); the bridges' and `exec::SpawnError`'s reasons are behind `reason()`.
- `App::open_url` reports `host::Unsupported` instead of a `String`; `App::background`
  returns a `BackgroundJob`.
- Error types replace `String` errors: `Snapshot::to_png` / `from_png` (`PngError`),
  `autostart::set`, `set_in` and `reveal` (`DesktopError`), `SpellBackend` and
  `BackendFactory` (`BackendError`; `SpellError::Backend` carries it), `GpuContent::prepare`
  (`PrepareError`), table edits, tree imports and moves and form validators (`Rejection`:
  `Err("why".into())` still works).
- Enums and option, config, settings, stats and report structs that will grow are
  `#[non_exhaustive]`: `match` needs a wildcard arm, and struct literals become the type's
  constructor or `Default` plus setters (`StylePatch::new().background(c)`,
  `MeterOptions::default().threshold(t)`, `KeyEvent::press(k).with_modifiers(m)`).
- Compiled `.rui` code reaches the runtime through `remedy_ui::markup` by default;
  `build::compile_dir_to` takes the root path first.
- `#[derive(Store)]` names the runtime the way the crate depends on RemedyUI.
- `.github/workflows/ci.yml` runs only when started by hand: the owner's local gate is CI.
- Every published crate's manifest has keywords, categories and a readme; `remedy-ui-test` is
  `publish = false`.

### Fixed

- A pause in typing starts a new undo step (the view's edit clock never advanced).
- Assistive technology can no longer press a disabled control, which let a screen reader (or
  anything driving one) press a trusted prompt's buttons during the moment they stay disabled.
- `submit_on_enter` works for plain multi-line fields, not only rich ones.
