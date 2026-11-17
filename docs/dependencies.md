# Dependencies

RemedyUI writes its own engine core: reactivity, the tree, styling, layout integration, motion, input, the compositor, the GPU UI renderer, the CPU compositor, widgets, the bridges' own modules and tooling. For deep, solved problems it uses maintained, memory-safe crates. Each one sits behind RemedyUI's own types, so it can be upgraded or replaced without breaking apps ([design/14](design/14-codebase-quality-and-evolution.md)).

A crate is pinned to an exact version when it is first added, in the milestone that needs it. The versions below are the latest published on crates.io on 2026-11-17, for reference.

## Allowed licenses

MIT, Apache-2.0 (with or without the LLVM exception), BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0 (ICU4X data), and MPL-2.0 for unmodified use. This is a subset of what FreeSpeak's `deny.toml` allows, so every RemedyUI dependency passes FreeSpeak's checks too.

## The crates

| Area | Crate | Version | Pack | Why this one |
|---|---|---|---|---|
| Rich text layout, editing helpers | `parley` | 0.13.0 | `text` | Bidi, line breaking, inline objects, carets. Pre-1.0 with breaking releases: pinned and wrapped |
| Shaping | `harfrust` | 0.15.0 (through parley 0.13; 0.16.0 is the latest) | `text` | Pure-Rust HarfBuzz port |
| Fonts, hinting, color glyphs | `skrifa`, `read-fonts` | 0.49.0 and its read-fonts (through parley 0.13; 0.50.0 is the latest) | `text` | TrueType, CFF and auto hinting with grayscale and LCD targets; COLRv0/v1, CBDT and sbix; fuzzed on OSS-Fuzz; ships in Chrome |
| Font fallback, system fonts | `fontique` | 0.13.0 | `text` | Fallback chains and system font lists; Android coverage checked when `text` lands |
| Segmentation, bidi, CLDR data | ICU4X (`icu_segmenter` and others) | 2.4.0 | `text`, `services-i18n` | Unicode-correct segmentation; locale data loaded per locale (Unicode-3.0 license) |
| Text storage for editing | `ropey` | 1.6.1 | `text` | Rope for large documents |
| Hyphenation | `hyphenation` | 0.8.4 | `text` (feature) | Knuth–Liang dictionaries per language, each a feature |
| Look-alike detection | `unicode-security` | 0.1.3 | `text` | UTS #39 skeletons for confusable names |
| Secret buffers | `zeroize` | 1.9.1 | `text`, `secure` | Zeroing that the compiler cannot optimize away |
| Layout algorithms | `taffy` | 0.15.1 | `layout` | CSS flexbox, grid and block, faithful to the specs |
| GPU | `wgpu` | 30.0.2 | `render-gpu` | D3D12, Vulkan, Metal, GL 3.3 / GLES 3.0 |
| Paths | `vello_gpu`, `vello_cpu` | 0.4.0 | `render-gpu`, `render-cpu` | Sparse-strip paths without compute shaders; fast multithreaded CPU rendering. Pre-1.0: behind RemedyUI's path type |
| Shader validation | `naga` | wgpu's | `render-gpu` | Validates app-supplied WGSL effects |
| CPU present | `softbuffer` | 0.4.8 | `render-cpu` | Pixel blit on every desktop |
| Windows and events | `winit` | 0.30.13 (0.31 when stable) | desktop and Android bridges | Each bridge adds its own modules where winit stops short |
| Accessibility | `accesskit` + adapters | 0.25.2 | `a11y` (adapters per bridge) | UI Automation, NSAccessibility, AT-SPI, Android |
| Clipboard | `arboard` | 3.7.0 | desktop bridges | Text and images; sensitive-item handling added in the bridge |
| File dialogs | `rfd` | 0.18.0 | `bridge-windows`, `bridge-macos` | Native dialogs. On Linux, dialogs go through the XDG portal with RemedyUI's own client over blocking `zbus`, because `rfd`'s portal backend needs an async runtime |
| D-Bus (Linux) | `zbus` (blocking API) | pinned with `bridge-wayland` | Linux bridges | Portal, notifications, StatusNotifierItem, AT-SPI |
| Images | `image` / `zune-image` | 0.25.10 / 0.5.0 | `media-image` | Chosen per format in its milestone, after a RustSec review and fuzzing |
| SVG | `usvg` | pinned with `media-svg` | `media-svg` | SVG to a path tree |
| Syntax colors | `syntect` (with `fancy-regex`) | 5.3.0 | `doc-code` | Pure-Rust highlighting, with no C regex engine |
| Diagnostics | `tracing` | 0.1.44 | every crate | Structured spans and events; Perfetto export in `devtools` |
| Errors | `thiserror` | 2.0.21 | every crate | Typed errors without hand-written boilerplate |
| Derive macros | `syn` 2, `quote`, `proc-macro2` | 2.0.119, 1.0.47, 1.0.107 | `macros` (build time only) | `#[derive(Store)]`; syn 2 rather than the new 3.0 until 3.x has settled |
| Platform APIs | `windows`, `objc2`, `ndk`, `jni` | pinned per bridge | bridges | OS calls, in the bridges' `os` modules only |

Development tools that are not linked into apps: `cargo-fuzz`, `cargo-deny`, `cargo-audit`, `cargo-vet`.

## Rules

- **A new dependency needs a reason** in this file: what it does, why RemedyUI should not write it, its license, and its maintenance health.
- **Minimal features.** `default-features = false`, then only what is used.
- **No async runtime, and no bundled C code.** Platform frameworks the OS provides (D3D, Metal, Vulkan loaders, system GStreamer on Linux) are loaded from the OS, never shipped.
- **Checked in CI**: `cargo deny` (licenses, advisories, bans, duplicate versions), `cargo audit` and `cargo vet`.
- **One owner module.** Each dependency is used from one internal module, and its types do not appear in public APIs.
- **Young crates** (parley, the vello renderers, winit 0.31, accesskit) have behavior tests in RemedyUI for what the engine relies on, so an upgrade that changes behavior is caught before release.
