# Dependencies

RemedyUI writes its own engine core. That covers reactivity, the tree, styling, layout integration, motion, input, the compositor, the GPU UI renderer, the CPU compositor, widgets, bridges' own modules and tooling. For deep, solved problems it uses maintained, memory-safe crates, each one behind RemedyUI's own types so it can be upgraded or replaced without breaking apps ([design/14](design/14-codebase-quality-and-evolution.md)).

Versions are as published on crates.io on 2026-11-17, and are pinned exactly in the workspace. Every crate is MIT, Apache-2.0 or both, and none needs an async runtime.

| Area | Crate | Version | Pack | Why this one |
|---|---|---|---|---|
| Rich text layout, editing helpers | `parley` | 0.13.0 | core | Bidi, line breaking, inline objects, carets. Pre-1.0 with breaking releases: pinned and wrapped |
| Shaping | `harfrust` | 0.16.0 | core | Pure-Rust HarfBuzz port |
| Fonts, hinting, color glyphs | `skrifa`, `read-fonts` | 0.50.0, 0.47.0 | core | TrueType, CFF and auto hinting with grayscale and LCD targets; COLRv0/v1, CBDT and sbix; fuzzed on OSS-Fuzz; ships in Chrome |
| Font fallback, system fonts | `fontique` | 0.13.0 | core | Fallback chains; Android coverage checked in M2 |
| Segmentation, bidi, CLDR data | ICU4X (`icu_segmenter` and others) | 2.4.0 | core, `services-i18n` | Unicode-correct segmentation; locale data loaded per locale |
| Layout algorithms | `taffy` | 0.15.1 | core | CSS flexbox, grid and block, faithful to the specs |
| GPU | `wgpu` | 30.0.2 | `render-gpu` | D3D12, Vulkan, Metal, GL 3.3 / GLES 3.0 |
| Paths | `vello_gpu`, `vello_cpu` | 0.4.0 | `render-gpu`, `render-cpu` | Sparse-strip paths without compute shaders; fast multithreaded CPU rendering. Pre-1.0: behind RemedyUI's path type |
| Shader validation | `naga` | wgpu's | `render-gpu` | Validates app-supplied WGSL effects |
| Windows and events | `winit` | 0.30.13 (0.31 when stable) | desktop and Android bridges | Each bridge adds its own modules where winit stops short |
| Accessibility | `accesskit` + adapters | 0.25.2 | core (adapters per bridge) | UI Automation, NSAccessibility, AT-SPI, Android |
| Clipboard | `arboard` | 3.7.0 | desktop bridges | Text and images; sensitive-item handling added in the bridge |
| File dialogs | `rfd` | 0.18.0 | `services-desktop` | Native dialogs; the XDG portal on Linux, used without an async runtime |
| CPU present | `softbuffer` | pinned at M3 | `render-cpu` | Pixel blit on every desktop |
| Images | `image` / `zune-image` | 0.25.10 / 0.5.0 | `media-image` | Chosen per format in M5 after RustSec review and fuzzing |
| SVG | `usvg` | pinned at M5 | `media-svg` | SVG to a path tree |
| Syntax colors | `tree-sitter` + grammars | pinned at its milestone | `doc-code` | Incremental parsing; each grammar a feature |
| Platform APIs | `windows`, `objc2`, `ndk`, `jni`, `zbus` (blocking) | pinned per bridge | bridges | OS calls in the bridges' `os` modules only |

## Rules

- **A new dependency needs a reason** in this file: what it does, why RemedyUI should not write it, its license and its maintenance health.
- **Minimal features.** `default-features = false`, then only what is used.
- **Checked in CI**: `cargo deny` (licenses, advisories, bans, duplicate versions), `cargo audit` and `cargo vet`.
- **One owner module.** Each dependency is used from one internal module. Its types do not appear in public APIs.
- **Young crates** (parley, the vello renderers, winit 0.31, accesskit) have behavior tests in RemedyUI for what the engine relies on, so an upgrade that changes behavior is caught before release.
