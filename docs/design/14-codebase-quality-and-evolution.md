# 14. Codebase quality and evolution

RemedyUI is meant to carry every Remedy app for years. The codebase has to stay small where it can, readable everywhere, and able to change without breaking the apps on top of it.

## Structure

- **The layer graph** from [00](00-principles-and-modularity.md) is enforced. A CI check fails any dependency that points upward or sideways between feature packs.
- **Crates are small and named for one job.** A module longer than about 1,500 lines, or a function longer than about 80, is a signal to split. These are not hard rules, but review asks why.
- **Public API is deliberate.** Each crate exports through one `lib.rs` with explicit `pub use`. Internal helpers are `pub(crate)`. Third-party types never appear in public signatures: taffy, parley, wgpu and winit stay behind RemedyUI types, so they can be upgraded or replaced.
- **Tests sit beside the code they test**, with integration tests per crate and cross-crate scenario tests in `remedy-ui-test`.

## Standards

| Area | Rule |
|---|---|
| Formatting | `rustfmt` with the repo config; CI rejects unformatted code |
| Lints | `clippy` with `-D warnings`, plus `unwrap_used`, `expect_used`, `indexing_slicing`, `panic` and `arithmetic_side_effects` denied in engine crates outside tests; `missing_docs` denied in public crates |
| Unsafe | `forbid` in every crate except the bridges and `render-gpu`, which `deny` it at the root and `allow` it only on their `os`/`ffi` module (CI checks no other allow exists); every `unsafe` block has a `// SAFETY:` comment; changes to those modules need a second review |
| Errors | `thiserror`-style typed errors per crate; no `Box<dyn Error>` in public APIs; errors say what failed and with what input (never content), and how to fix it when known |
| Logging | `tracing` with structured fields; no text content, input or clipboard data ([08](08-security-and-privacy.md)) |
| Names | Full words (`layout`, not `lyt`); one name per concept across crates (it is "element" everywhere, never "node" in one crate and "widget" in another, except where `Widget` means a built-in component) |
| Comments | Explain why, not what; each module starts with a short doc comment on its role and invariants |
| Generated code | Kept out of the repo except committed snapshots and corpora; generators run in the build |

## Testing bar

- **Every change comes with tests** that fail without it.
- **Coverage** is measured per crate and shown in CI. Engine crates aim for high line coverage, but CI does not chase a number. The gates that matter are the scenario suites, golden images, fuzzing and benchmarks.
- **Repeat runs.** Flaky tests are bugs. Each scenario suite runs several times in CI, and a test that passes only sometimes is fixed or removed the same week, with the root cause written down.
- **Platforms.** CI runs on Windows, Linux and macOS, plus Android emulator scenarios and the RemedyOS bridge under QEMU, at least nightly.

## Decisions are written down

- **Architecture decision records** (`docs/adr/NNNN-title.md`) cover each decision that shapes the engine: why taffy, why a compositor thread, why no selector cascade. Each gives the context, the options, the decision and its consequences. A later decision that reverses one links to it.
- **Design documents** (`docs/design/`) describe how the engine works now. A change that alters behavior updates the document in the same pull request.

## Evolving without breaking

- **Semantic versioning** from 1.0. Before 1.0, minor versions may break, with changes collected in the changelog under "Breaking" and a migration note for each.
- **Deprecation before removal.** An API is deprecated for at least one minor release with a `#[deprecated(note = "use X")]` that names the replacement. A machine-applicable fix is provided where the change is mechanical (`cargo remedy migrate`).
- **Editions for behavior changes.** When a default must change (a better scroll physics, a new focus rule), the new behavior arrives behind an edition flag on the app (`edition = "2027"`). Old apps keep the old behavior until they opt in, like Rust editions. Editions are rare and documented.
- **MSRV policy.** The minimum Rust version is the stable release from six months ago, raised only in minor releases, and tracked in CI.
- **Dependency upgrades** go through the internal wrapper for that dependency, with its behavior tests. An upgrade that changes behavior apps can see is treated as a RemedyUI change, with a changelog entry.

## Futureproofing

- **New platforms** are new bridges ([10](10-platform-bridges.md)), and new renderers implement the same display list. Nothing above L4 changes for either.
- **New input kinds** (pen, gaze, spatial controllers) extend the pointer model and the gesture arena without touching widgets.
- **New display tech:**
  - Wide gamut and HDR are already in the color model.
  - Variable refresh and high frame rates are in the frame clock.
  - Fractional scales are in layout snapping.
- **AI operation** of apps comes from the semantics tree and command table ([11](11-accessibility-and-automation.md)), which grow with the engine rather than as a separate system.
- **A smaller future.** The same core runs on a phone, a desktop, the RemedyOS desktop and inside a headless test. That is the guard against the engine growing assumptions it cannot keep.
