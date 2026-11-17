# RemedyUI

RemedyUI is the UI engine for every Remedy app, written in Rust. That covers chat (FreeSpeak), documents, notes, audio and music tools, file and secret managers, calculators, media apps, and the RemedyOS desktop. It is built to the standard of the leading UI engines (Flutter, SwiftUI, Jetpack Compose, WinUI 3, Qt Quick, the web platform). Its aims, in order: stable, secure, fast as the hardware allows, light, intuitive for the people using it, and a pleasure to build with.

**Status: design.** The requirements, design, app map and roadmap are written. Engine code comes next, milestone by milestone. A crate is added to the workspace only when it has real code and tests.

## In short

- **Retained tree with fine-grained signals.** Components run once. A state change updates only what reads it, and nothing runs while nothing changes.
- **A compositor thread.** Scrolling, animation and video stay at the display's rate even when app code is busy.
- **Modular.** Feature packs for data tables, documents, code, terminals, music widgets, media, charts and platform services. An app links only what it uses, and what it links starts only when first used.
- **Real text.** Full shaping and bidi, variable and color fonts, hinting with grayscale and LCD, rich paragraphs, selection across elements, rich editing, each platform's real IME, spell checking.
- **Two renderers, one display list.** A wgpu UI renderer with precompiled pipelines, and a fast CPU renderer as the floor. It recovers from a lost GPU and presents only what changed.
- **Code fluid.** Users can restyle, rebind and rearrange the apps they use: themes, shortcuts and toolbars, and `.rui` layouts where an app allows it. One theme set is shared by every Remedy app.
- **Every platform as a bridge.** Windows, macOS, Linux (Wayland and X11), Android, RemedyOS, and later the web and iOS.
- **Accessible, and operable by agents with consent.** One semantics tree serves screen readers, tests and AI agents.
- **Safe.** `#![forbid(unsafe_code)]` outside one audited boundary. Untrusted content is parsed by fuzzed, memory-safe code off the UI thread, optionally in a sandboxed helper. Secret inputs never leak, and there is no telemetry.

## Documents

| Document | What it holds |
|---|---|
| [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md) | The bar: what a modern UI engine must provide, what the Remedy apps add, and the egui failures never to repeat |
| [docs/DESIGN.md](docs/DESIGN.md) | The overview: principles, engine shape, crates, stability, and the index of deep dives |
| [docs/design/](docs/design/) | Sixteen deep dives, from threads and state to text, rendering, security, accessibility, developer experience and the design language |
| [docs/APPS.md](docs/APPS.md) | Every Remedy app mapped to the packs and bridges it needs, and the order the apps move over |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones M1 to M9, each with a tested definition of done and an app moved onto the engine |
| [docs/dependencies.md](docs/dependencies.md) | The third-party crates, pinned versions, and the rules for adding one |

## License

RemedyUI is licensed under either of

- the [Apache License, Version 2.0](LICENSE-APACHE), or
- the [MIT License](LICENSE-MIT),

at your option. This is the usual license pair for Rust libraries. It works with FreeSpeak's AGPL-3.0, RemedyOS's Apache-2.0 and closed-source apps alike, and Apache-2.0 adds a patent grant.

Unless you say otherwise, any contribution you submit for inclusion in RemedyUI is licensed as above, without any additional terms or conditions.

Hope this helps
- Ahmi
