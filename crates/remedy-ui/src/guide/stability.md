# Stability, editions and migrating

RemedyUI carries every Remedy app for years, so it says what it promises and checks it before
every release. The full promise is in
[docs/STABILITY.md](https://github.com/AhmiDarrow/RemedyUI/blob/main/docs/STABILITY.md), and the
release checks are in
[docs/RELEASING.md](https://github.com/AhmiDarrow/RemedyUI/blob/main/docs/RELEASING.md).
This page is the part an app writer needs day to day.

## What does not break

- **From 1.0**, a minor release never breaks an app that compiled against an earlier 1.x: no
  public item is removed or renamed, and no signature changes incompatibly. `cargo xtask
  semver` checks every published crate against the last release.
- **Before 1.0**, minor versions may break. Every break is in `CHANGELOG.md` with what to do,
  and the mechanical ones are fixed for you by `cargo remedy migrate` (below).
- **Replaced items are deprecated first**, for at least one minor release, with a note naming
  what replaces them.
- **Errors are typed.** Every fallible function returns an error type that implements
  `std::error::Error` and says what failed, such as [`RunError`](crate::RunError).

## Types that grow

Some types are open on purpose, so RemedyUI can grow without breaking apps:

- **Enums that will gain variants are `#[non_exhaustive]`:** events, errors, states, kinds and
  choices. A `match` on one needs a wildcard arm.
- **Options, config, settings and report structs are `#[non_exhaustive]`:** they gain fields.
  Make one with its constructor or `Default` and set what differs with its setters.
- **Input events** are made with constructors and `with_*` methods.

```
use remedy_ui::input::{Key, KeyEvent, Modifiers};
use remedy_ui::style::StylePatch;
use remedy_ui::themes::ThemeChoice;
use remedy_ui::viz::MeterOptions;

let theme = ThemeChoice::default().resolve(None, false);
let pill = StylePatch::new().background(theme.colors.accent).radius(theme.radii.large);
let meter = MeterOptions::default().threshold(remedy_ui::reactive::Signal::new(0.8));
let save = KeyEvent::press(Key::Character("s".into())).with_modifiers(Modifiers::CTRL);

assert_eq!(pill.radius, Some(theme.radii.large));
assert!(save.pressed);
# let _ = meter;
```

Struct literals of these types do not compile outside their crate, which is the point: a new
field never breaks an app.

## Editions

When a default behavior has to change (better scroll physics, a new focus rule), the new
behavior arrives in a new edition. An app chooses its edition with
[`AppConfig::edition`](crate::AppConfig::edition); one that never names an edition gets
[`Edition::FIRST`](crate::Edition::FIRST) and keeps the behavior it was written against for
all of 1.x. `cargo remedy new` writes the newest edition into new apps. Code that depends on
an edition asks the window with [`Ui::edition`](crate::Ui::edition) and
[`Edition::at_least`](crate::Edition::at_least).

```
use remedy_ui::testing::TestApp;
use remedy_ui::{App, AppConfig, Edition, WindowSpec};

let config = AppConfig::new("Notes").edition(Edition::E2026);
let test = TestApp::new(config, |app: &App| {
    app.open(WindowSpec::new("Notes"), |_, _| {});
});
let Some(ui) = test.ui() else { unreachable!("open") };
assert_eq!(ui.edition(), Edition::E2026);
assert!(ui.edition().at_least(Edition::FIRST));
assert_eq!("2026".parse::<Edition>(), Ok(Edition::E2026));
```

Editions are rare. Each one is listed in STABILITY.md with every behavior it changes; today
there is one, `E2026`, RemedyUI 1.0's behavior.

## Migrating an app

`cargo remedy migrate` applies the mechanical fixes for RemedyUI's renames to an app's
sources. It resolves each path through the file's own `use` items, so it renames the item
however the file names it, and never touches strings or comments. The renames it knows are
in `crates/cargo-remedy/migrations.toml`, each with the release that made it.

```text
cargo remedy migrate --dry-run         # show the changes as a diff
cargo remedy migrate                   # apply them
cargo remedy migrate --path ../my-app  # in another directory
```

Changes that are not mechanical (a struct literal of a type that became `#[non_exhaustive]`,
a `match` that needs a wildcard arm) are listed in CHANGELOG.md with the replacement for
each; the compiler points at every one.

## Minimum Rust version

The minimum supported Rust version is the workspace's `rust-version`. It is raised only in
minor releases, with the reason in the changelog, and `cargo xtask msrv` checks it before
every release.
