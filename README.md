# RemedyUI

RemedyUI is a UI engine for Rust. It is built to be fast, efficient, stable and secure, and full featured enough to carry real apps on its own. It is built to the standard of modern UI engines (Flutter, SwiftUI, Jetpack Compose, WinUI 3, Qt Quick, the web platform). It powers [FreeSpeak](https://github.com/AhmiDarrow/FreeSpeak) first, and is meant for RemedyOS and other apps too.

**Status: design.** The design, requirements and roadmap are written. Engine code comes next, milestone by milestone. A crate is added to the workspace only when it has real code and tests.

## In short

- **Retained tree with fine-grained signals.** A state change updates only what reads it. Nothing runs, and nothing wakes the CPU, while nothing changes.
- **Same-frame layout.** Flexbox, grid and block layout, plus virtual lists and grids with height caches and scroll anchoring for lists of any length.
- **Real text.** Full shaping and bidi for every script, variable and color fonts, hinting at the pixel size with grayscale and LCD subpixel output. Rich paragraphs with inline objects, selection across paragraphs, rich text editing, IME and spell checking.
- **Two renderers, one display list.** A wgpu UI renderer (D3D12, Vulkan, Metal, GL 3.3 / GLES 3) and a fast CPU renderer for machines with no usable GPU. It recovers from device loss and redraws only damaged regions.
- **Motion.** One frame clock synced to the display; transitions, springs, enter/exit, shared-element and gesture-driven animation; Reduced Motion everywhere.
- **Whole platform.** Multi-window with per-monitor DPI, custom window chrome, system theme following, tray, notifications, native dialogs, drag and drop, and clipboard with rich types. Windows, macOS, Linux and Android.
- **Accessible by default.** AccessKit semantics on every widget, for UI Automation, NSAccessibility, AT-SPI and Android.
- **Safe.** `#![forbid(unsafe_code)]` outside one small GPU and platform boundary. Untrusted images, fonts and themes are parsed by fuzzed, memory-safe code with limits, off the UI thread.

## Documents

| Document | What it holds |
|---|---|
| [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) | The bar: what a modern UI engine must provide, what FreeSpeak needs, and the egui failures never to repeat |
| [docs/DESIGN.md](docs/DESIGN.md) | The architecture: crates, programming model, frame, layout, rendering, text, widgets, media, motion, input, platform, localization, theming, accessibility, stability, security, budgets, tooling, building blocks |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones M1 to M8 and what "done" means for each |

## License

RemedyUI is licensed under either of

- the [Apache License, Version 2.0](LICENSE-APACHE), or
- the [MIT License](LICENSE-MIT),

at your option. This is the usual license pair for Rust libraries. It works with FreeSpeak's AGPL-3.0, RemedyOS's Apache-2.0 and closed-source apps alike, and Apache-2.0 adds a patent grant.

Unless you say otherwise, any contribution you submit for inclusion in RemedyUI is licensed as above, without any additional terms or conditions.

Hope this helps
- Ahmi
