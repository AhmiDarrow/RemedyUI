# What RemedyUI 1.0 promises

RemedyUI carries every Remedy app for years. From 1.0 these rules hold, and the tools that
check them run before every release ([RELEASING.md](RELEASING.md)).

## Semantic versioning

- **From 1.0**, a minor release (1.x) never breaks an app that compiled against an earlier
  1.x: no public item is removed or renamed, no signature changes incompatibly, no trait gains
  a method without a default. `cargo xtask semver` (cargo-semver-checks) checks every
  publishable crate against the last release.
- **Before 1.0**, minor versions may break. Every break is listed in `CHANGELOG.md` with what
  to do, and the mechanical ones are fixed by `cargo remedy migrate`.

## Room to grow

So that RemedyUI can grow without breaking apps, some types are open on purpose:

- **Enums that will gain variants are `#[non_exhaustive]`:** events, errors, states, kinds and
  choices. A `match` on one needs a wildcard arm (`_ => ...`). Enums that are truly closed stay
  exhaustive, each with a comment saying why: the display list (`remedy_ui::paint`) and the
  host contract (`remedy_ui::host`) that every renderer and bridge must handle completely,
  where a new kind is a major-release change by design; and sets that cannot grow (two axes,
  four window edges and corners, seven weekdays, the QR standard's levels).
- **Option, config, settings, stats and report structs are `#[non_exhaustive]`:** they gain
  fields. Make one from its constructor or `Default` and set what differs, with the setters
  every options type has, or field by field:

  ```rust,ignore
  let meter = level_meter(ui, "Microphone", level, MeterOptions::default().threshold(t));
  let style = StylePatch::new().background(accent).radius(6.0);
  let mut layout = LayoutStyle::row();
  layout.gap = (8.0, 0.0);
  ```

  Struct literals of these types do not compile outside their crate, so a new field never
  breaks an app.
- **Input events** (`PointerEvent`, `KeyEvent`, `WheelEvent`, `PinchEvent`) are made with their
  constructors (`KeyEvent::press(Key::Enter)`, `PointerEvent::new(phase, at)`) and `with_*`
  methods. Plain value types (geometry, `Color`, `Modifiers`, ids) stay exhaustive.

## Deprecation before removal

An item that is replaced is deprecated for at least one minor release before it is removed,
with `#[deprecated(since = "...", note = "use X")]` naming the replacement. Where the change is
mechanical, `crates/cargo-remedy/migrations.toml` carries it and `cargo remedy migrate` applies
it to an app's sources (`--dry-run` shows the diff first). Before 1.0, a deprecated alias is
kept where a Remedy app already uses the old name; otherwise the rename is direct and the
migration table carries it.

## Editions for behavior changes

When a default must change (better scroll physics, a new focus rule), the new behavior comes
in a new edition: `AppConfig::edition(Edition::E20xx)`, read by engine code through
`Ui::edition()` and `Edition::at_least`. An app that never names an edition gets
`Edition::FIRST` and keeps the behavior it was written against for all of 1.x; `cargo remedy
new` writes the newest edition into new apps. Editions are rare.

| Edition | Behaviors it changes |
|---|---|
| `E2026` | The first edition: RemedyUI 1.0's behavior. |

## Typed errors

Every fallible public function returns an error type of its crate that implements
`std::error::Error` and `Display` and says what failed. None returns `Result<_, String>`.
Variants of error enums may be added in minor releases (they are `#[non_exhaustive]`).

## MSRV

The minimum supported Rust version is the workspace's `rust-version` (1.96). It is the stable
release from about six months before, raised only in minor releases with the reason in the
changelog, and checked by `cargo xtask msrv` before every release.

## Dependencies in the API

Third-party types stay behind RemedyUI's own types, so a dependency upgrade does not break
apps. The exceptions are deliberate interop, re-exported so apps name the exact version
RemedyUI uses, and versioned with it:

- `remedy_ui::gpu_embed::wgpu` (and `remedy_ui_engine::render_gpu::wgpu`): an app's own GPU rendering
  is wgpu; a wgpu major upgrade is a RemedyUI major release.
- `remedy_ui::webview::serde_json`: the one JSON message channel between an app and its page.
- `remedy_ui::host::{accesskit, raw_window_handle}`: the accessibility tree and native window
  handles that bridges and embedded native views exchange.

## Which crates are stable

- **Stable from 1.0:** the facade `remedy-ui` and everything it re-exports for apps: `core`,
  `reactive`, `view`, `style`, `text`, `layout`, `input`, `a11y`, `paint`, `themes`, the widget
  packs (`widgets`, `nav`, `data`, `forms`, `extra`, `music`, `term`), the document packs
  (`rich`, `code`, `pages`), media (`image`, `svg`, `anim`, `video`, `playback`, `isolated`),
  `viz`, `customize`, `markup`, `i18n`, `spell`, `update`, `webstorage`, `agents`, `devtools`,
  `gpu_embed`, `webview`, `testing`, and `cargo-remedy`'s commands and `remedy.toml`.
- **Engine parts** (`remedy-ui-core::host`, `::tree` and `::trace`, `remedy-ui-engine::compositor`,
  `::render_cpu` and `::render_gpu`, the bridges in `remedy-ui-platform` and
  `remedy-ui-bridge-web`): published, semver-checked, and stable for what the
  facade exposes of them; their engine-to-engine surface follows the release cadence of the
  renderers and bridges.
- **Experimental:** scripted widgets (`remedy-ui-engine::script`, `remedy-ui-widgets::board`) are being
  built on RemedyLua and WebAssembly engines behind one interface, and are not covered by the
  1.0 promise until that lands. The stage pack (`remedy-ui-widgets::stage`, `stage`) is new
  and may still change while the first presentations are built on it.
