# Releasing RemedyUI

The owner's local gate is RemedyUI's CI. GitHub Actions runs only when started by hand
(`.github/workflows/ci.yml` is `workflow_dispatch`). Everything below runs on the release
machine.

## Tools

| Tool | Version | Install |
|---|---|---|
| Rust (stable) | the current stable | `rustup update stable` |
| Rust at the MSRV | the workspace's `rust-version` (1.96) | `cargo xtask msrv` installs it with rustup when missing |
| cargo-semver-checks | 0.51.0 | `cargo install cargo-semver-checks --version 0.51.0 --locked` |
| cargo-deny | as the gate uses | `cargo install cargo-deny --locked` |

On the release machine `CARGO_HOME` is not writable by the build user, so cargo-semver-checks
went into the user's own root: `cargo install cargo-semver-checks --version 0.51.0 --locked
--root "$HOME/.cargo"` (with `~/.cargo/bin` on `PATH`).

## Before every merge

From the repository root:

```sh
GATE_DIR="$PWD" bash ~/gate.sh      # must end GATE-OK
bash ~/budget-gate.sh               # the reference scenes' budgets
```

The gate runs formatting, clippy for Windows, Linux, macOS, Android and wasm32, the tests,
the docs with warnings denied, `cargo xtask check` (the layer graph, the unsafe-code boundary,
publishing metadata on every published crate, the docs index and `cargo vet --locked`) and
`cargo deny check`. Install cargo-vet (`cargo install cargo-vet --locked`) on the gate
machine: without it `cargo xtask check` skips the audits with a note.

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
   package every publishable crate (64 at 1.0.0-rc.1). Crates that are not published carry
   `publish = false`: `xtask` and `remedy-ui-core::testing`. Dev-dependencies on the facade are
   path-only, so packaging does not need the facade on crates.io first.
6. **Publish** `remedy-vello-cpu` first when it changed (it lives outside the workspace:
   `cargo publish --manifest-path vendor/vello_cpu/Cargo.toml`), then the workspace in
   dependency order with `cargo publish -p <crate>` (Cargo orders a workspace publish itself
   from 1.90: `cargo publish --workspace`), then tag the release `vX.Y.Z`.

## Editions

A release that changes a default behavior adds an edition (`remedy_ui_core::Edition`): the
new behavior applies only to apps that ask for that edition with `AppConfig::edition`, and
`docs/STABILITY.md` lists what each edition changes. `cargo remedy new` starts apps on the
newest edition.
