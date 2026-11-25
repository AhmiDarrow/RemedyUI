# RemedyUI design

RemedyUI is the UI engine for every Remedy app: chat, documents, notes, audio and music tools, file and secret managers, calculators, media apps, and the RemedyOS desktop. It is built to the standard of the leading UI engines (Flutter, SwiftUI, Jetpack Compose, WinUI 3, Qt Quick, the web platform), not to the limits of egui, which FreeSpeak is leaving.

This page is the overview. Each area has a deep dive in [`design/`](design/). The bar the design meets is in [REQUIREMENTS.md](REQUIREMENTS.md), how each Remedy app uses it in [APPS.md](APPS.md), and the order of work in [ROADMAP.md](ROADMAP.md).

## Principles, in priority order

1. **Stable**
2. **Secure and private**
3. **Fast as the hardware allows**
4. **Efficient and light**, paying only for what is used
5. **Intuitive for people**, good by default
6. **A pleasure to build with**
7. **Full featured**

The Remedy tenets also shape it:

- Remedy's own design, with platforms as bridges
- code fluid: users can restyle and rearrange apps
- no fixed caps: sized from need and hardware
- no stubs
- local first and undoable
- operable by people and by consenting agents

[design/00](design/00-principles-and-modularity.md) explains each and sets the modularity rules.

## Shape of the engine

```
             ┌──────────────────────── UI thread ────────────────────────┐
 app code ─▶ │ signals ─▶ elements ─▶ style ─▶ layout ─▶ paint ─▶ semantics │ ─commit─▶ ┌──── compositor thread ────┐
             └────────────────────────────────────────────────────────────┘           │ scroll, compositor animation, │
 other threads ── Waker.post ──▶ (batched into the next frame)                          │ raster dirty tiles, composite, │ ─▶ present (damage only)
 worker pool: shaping, glyphs, decoding, tiles                                          │ video and animated images      │
                                                                                         └───────────────────────────────┘
```

- **Retained tree, fine-grained signals.** Components run once. Signals update only the properties that read them, and dirty flags limit each phase to what changed ([02](design/02-state-and-elements.md)).
- **Two threads where it matters.** App code can never stall scrolling, animation or video: the compositor thread owns them ([01](design/01-threads-and-frames.md)).
- **Nothing when nothing changes.** There is no polling, no repaint loop and no vsync while idle.

```rust
fn composer(send: impl Fn(String) + Copy + 'static) -> impl Element {
    let text = Signal::new(String::new());
    let can_send = Memo::new(move || !text.get().trim().is_empty());
    row().gap(Space::S2)
        .child(text_input(text).placeholder(t("Message")).grow(1.0)
            .on_key(Key::Enter, move |k| if !k.shift() && can_send.get() { send(text.take()); Handled::Yes } else { Handled::No }))
        .child(button(t("Send")).variant(Variant::Primary).enabled(can_send).on_click(move || send(text.take())))
}
```

## The deep dives

| # | Area | In one line |
|---|---|---|
| [00](design/00-principles-and-modularity.md) | Principles and modularity | Priorities, tenets, the layered crate graph, feature packs, profiles, lazy subsystems, footprint targets |
| [01](design/01-threads-and-frames.md) | Threads and frames | UI thread, compositor thread and workers; the commit; compositor-side scrolling and animation; scheduling and idle |
| [02](design/02-state-and-elements.md) | State and elements | Glitch-free push-pull signals, stores, reactive collections, data-oriented elements, dirty flags, relayout and repaint boundaries, context, error boundaries, suspense, resources, navigation, persistence |
| [03](design/03-layout.md) | Layout | One constraints protocol over flex, grid, block and custom layouts; logical directions; pixel snapping; caching and parallel layout; virtualization with anchoring; responsive size classes |
| [04](design/04-rendering.md) | Rendering | Paint chunks, layerization, tiles, the wgpu UI renderer with precompiled pipelines, the CPU renderer, backend ladder and recovery, damage-only presenting, effects and validated custom shaders, PDF and printing |
| [05](design/05-text.md) | Text | Itemize, shape, break, position; rich paragraphs with inline objects; fonts, variable and color fonts; hinting and LCD; selection across elements; editing with each platform's real IME; spell checking; text-borne attacks |
| [06](design/06-input-and-motion.md) | Input and motion | One event model, hit testing, gesture arena, scroll physics, focus, layout-aware shortcuts, commands, drag and drop; frame clock, springs, interruption, enter/exit, shared elements, gesture-driven motion, reduced motion |
| [07](design/07-styling-and-customization.md) | Styling and customization | Design tokens, styles without a selector cascade, `.rui` markup, and the code-fluid layer that lets users restyle and rearrange apps safely |
| [08](design/08-security-and-privacy.md) | Security and privacy | Threat model, memory safety, untrusted bytes with bounds from need, isolated decoding, secrets on screen, spoofing defenses, no telemetry, supply chain |
| [09](design/09-performance-and-resources.md) | Performance and resources | Where time goes, no allocation in steady frames, sizing from the device with pressure-driven eviction, real-time and large content, benchmarks as CI gates |
| [10](design/10-platform-bridges.md) | Platform bridges | The host model; Windows, macOS, Wayland, X11, Android, RemedyOS, web, iOS and headless bridges; embedding; platform conventions |
| [11](design/11-accessibility-and-automation.md) | Accessibility and automation | Semantics, AccessKit, merging, live regions, testing through semantics, and consent-gated operation by AI agents |
| [12](design/12-developer-experience.md) | Developer experience | API style, project templates and packaging, previews, devtools, diagnostics, testing, documentation and porting guides |
| [13](design/13-design-language.md) | Design language | The Remedy interface guidelines the widgets enforce: feel, familiarity, clarity, calm, one family across apps |
| [14](design/14-codebase-quality-and-evolution.md) | Codebase quality and evolution | Structure, standards, testing bar, decision records, semver, deprecation, editions, futureproofing |
| [15](design/15-widgets-and-media.md) | Widgets and media | The widget catalog by pack, windows as options, the media pipeline |
| [16](design/16-effects.md) | Effects (Prism) | The `.rfx` language, its compiler and limits, the GPU kernel and CPU interpreter, effects as elements |
| [17](design/17-3d.md) | 3D | 3D in effects (shipped), 3D element transforms and a 3D scene element (designed) |

[dependencies.md](dependencies.md) lists the third-party crates, their versions, and the rules for adding one.

## Crates

| Layer | Crates |
|---|---|
| L0 base | `remedy-ui-core` |
| L1 state | `remedy-ui-core::reactive` |
| L2 engine | `remedy-ui-core::tree`, `remedy-ui-core::style`, `remedy-ui-core::layout`, `remedy-ui-text`, `remedy-ui-core::input` |
| L3 frame | `remedy-ui-engine::paint`, `remedy-ui-core::motion`, `remedy-ui-core::a11y`, `remedy-ui-engine::compositor` |
| L4 host | `remedy-ui-engine::render_gpu`, `remedy-ui-engine::render_cpu`, `remedy-ui-bridge-{windows,macos,wayland,x11,android,remedyos,web,ios,headless}` |
| L5 packs | `remedy-ui-widgets-{core,nav,data,forms,extra,stage,music,term}`, `remedy-ui-doc-{rich,code,pages}`, `remedy-ui-media-{image,anim,video,playback,isolated}` (SVG is `media-image`'s `svg` feature), `remedy-ui-widgets::viz`, `remedy-ui-embed-{webview,gpu}`, `remedy-ui-services-{desktop,update,spell,i18n}`, `remedy-ui-widgets::themes`, `remedy-ui-fonts`, `remedy-ui-widgets::devtools` |
| L6 | `remedy-ui` (facade with profiles `minimal`, `app`, `full`), `remedy-ui-widgets::rui` (markup compiler, and the run-time loader behind its `runtime` feature), `remedy-ui-core::testing`, `cargo-remedy` (CLI, not linked into apps) |
| L7 | Developer tools, not linked into apps: `cargo-remedy::devkit` (their logic, a feature per tool group), `cargo-remedy::templates` (the app templates), `remedy-ui-build` (build-script help: the Windows executable's icon and version), `cargo-remedy` (the command line) and `remedy-mcp` (the MCP server for AI agents); see [CLI.md](CLI.md) and [MCP.md](MCP.md) |

A crate joins the workspace when its first real code and tests do. None exists as a placeholder.

## Stability in one place

Stability is the first principle, so here is how the design delivers it, gathered from across the deep dives:

- **No panic reaches a frame.** The engine's frame path denies `unwrap`, `expect` and panicking indexing. App callbacks run inside `catch_unwind`, and error boundaries replace a failed subtree with a fallback ([02](design/02-state-and-elements.md), [14](design/14-codebase-quality-and-evolution.md)).
- **It starts on any machine.** GPU backends are self-tested at start, with the CPU renderer as the floor ([04](design/04-rendering.md)).
- **A lost GPU recovers** from retained data without the app knowing. Repeated failure demotes to the next backend ([04](design/04-rendering.md)).
- **Bad input degrades, never crashes.** Undecodable images show their error state, broken fonts drop out of fallback, broken customizations fall back to the default with a notice ([07](design/07-styling-and-customization.md), [08](design/08-security-and-privacy.md)).
- **Determinism.** The same tree, theme, size and clock give the same pixels, which the golden tests rely on ([01](design/01-threads-and-frames.md), [12](design/12-developer-experience.md)).
- **Repeat-run testing.** Every scenario suite runs several times in CI; flakes are bugs ([14](design/14-codebase-quality-and-evolution.md)).
