# Releasing RemedyUI

Everything below runs before changes are merged and released.

## Tools

| Tool | Version | Install |
|---|---|---|
| Rust (stable) | the current stable | `rustup update stable` |
| Rust at the MSRV | the workspace's `rust-version` (1.96) | `cargo xtask msrv` installs it with rustup when missing |
| cargo-semver-checks | 0.51.0 | `cargo install cargo-semver-checks --version 0.51.0 --locked` |
| cargo-deny | the current release | `cargo install cargo-deny --locked` |

## Before every merge

GitHub Actions runs only when started by hand (`.github/workflows/ci.yml`), so these run
locally, from the repository root:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`, for the host and for
  the Linux, macOS, Android and wasm32 targets
- `cargo test --workspace`
- `cargo doc --workspace --all-features --no-deps` with `RUSTDOCFLAGS="-D warnings"`
- `cargo xtask check` (the layer graph, the unsafe-code boundary, publishing metadata, the docs
  index and `cargo vet --locked`; without cargo-vet installed the audits are skipped with a
  note) and `cargo deny check`
- the reference scenes' budgets, in release builds (the `zoom_budget`, `budget`,
  `compositor_zoom`, `table` and `throughput` tests)

## Before every release

1. **API compatibility.** `cargo xtask semver --baseline <previous release tag>` checks every
   publishable crate's public API against the last release with cargo-semver-checks. From 1.0
   on, a minor release must pass; a breaking change waits for the next major release
   (`docs/STABILITY.md`). Extra arguments go to cargo-semver-checks, for example
   `cargo xtask semver --baseline v1.0.0 --all-features`.
2. **MSRV.** `cargo xtask msrv` checks the whole workspace (all targets and features) with
   the declared `rust-version`. Raising it is a minor-release change with a written reason in
   the changelog; it never goes past the stable release from six months before.
3. **Migrations.** Every rename or call change made in the release has an entry in
   `crates/cargo-remedy/migrations.toml` with its `since` version, and a deprecated alias kept
   for at least one minor release where an app uses the old name. Check with
   `cargo remedy migrate --dry-run --path <an app>` on the Remedy apps.
4. **Changelog.** Move "Unreleased" in `CHANGELOG.md` under the new version and date.
5. **Packages.** `cargo package --workspace --no-verify --allow-dirty --exclude xtask` must
   package every publishable crate (15 since the crates were merged). A crate that is not
   published carries `publish = false`: `xtask`. A dev-dependency on a crate that depends
   back on this one (the facade, or `remedy-ui-widgets` from the engine, media and platform
   crates) is path-only, with no `version`, so Cargo leaves it out of the published manifest
   and the crates have a publish order. Until `remedy-vello-cpu` is on crates.io, add
   `--config 'patch.crates-io.remedy-vello-cpu.path="vendor/vello_cpu"'`. A warning that
   a locked version is yanked is a blocker: move the pin first.
6. **Publish** `remedy-vello-cpu` first when it changed (it lives outside the workspace:
   `cargo publish --manifest-path vendor/vello_cpu/Cargo.toml`), then the workspace in
   dependency order with `cargo publish -p <crate>` (Cargo orders a workspace publish itself
   from 1.90: `cargo publish --workspace`), then tag the release `vX.Y.Z`.
   Published one at a time, the order that works is: `remedy-ui-build`, `remedy-ui-core`,
   `remedy-ui-fonts`, `remedy-ui-macros`, `remedy-ui-bridge-web`, `remedy-ui-fonts-cjk`,
   `remedy-ui-text`, `remedy-ui-engine`, `remedy-ui-platform`, `remedy-ui-media`,
   `remedy-ui-widgets`, `remedy-ui-widgets-term`, `remedy-ui-media-playback`, `remedy-ui`,
   `cargo-remedy`. crates.io lets five new crates through at once and then one every ten
   minutes; new versions of existing crates are not held back that way.

## Editions

A release that changes a default behavior adds an edition (`remedy_ui_core::Edition`): the
new behavior applies only to apps that ask for that edition with `AppConfig::edition`, and
`docs/STABILITY.md` lists what each edition changes. `cargo remedy new` starts apps on the
newest edition.
