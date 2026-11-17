# 0. Principles and modularity

RemedyUI is the one UI engine for every Remedy app. That covers chat (FreeSpeak), documents, notes, audio and music tools, file and secret managers, calculators, launchers and games' menus, and the RemedyOS desktop itself. An engine that serves all of them must not make each of them carry all of it. This document sets the rules every other design document follows.

## Principles

These are in priority order. When two pull against each other, the higher one wins.

1. **Stable.** It runs the same way every time. No panic reaches a frame, no hang reaches the user, and it starts on any machine. Judge by repeat runs, not one pass.
2. **Secure and private.** No unsafe code outside an audited boundary. Untrusted bytes are parsed by memory-safe code off the UI thread. The engine never phones home, logs content, or needs an account or a network.
3. **Fast as the hardware allows.** Work is proportional to change. Nothing runs when nothing changes, and input reaches the screen within a frame.
4. **Efficient and light.** An app pays only for what it uses, in binary size, memory and CPU (see Modularity).
5. **Intuitive for people.** Good behavior is the default: platform conventions, accessibility, undo, feedback, motion that explains. An app built quickly on RemedyUI should already feel right ([13-design-language.md](13-design-language.md)).
6. **A pleasure to build with.** One obvious way to do each thing, names that read well, errors that point at the fix, live previews, fast rebuilds ([12-developer-experience.md](12-developer-experience.md)).
7. **Full featured.** What modern engines offer is here, so apps never need forks or workarounds.

Rules that come from the Remedy tenets:

- **Remedy's own design.** RemedyUI has its own model of windows, input, text, rendering and semantics. Windows, macOS, Linux, Android, the web and RemedyOS are **bridges** that translate that model to each platform. No platform's shape leaks into the core, and the RemedyOS native bridge is a first-class target, not an afterthought ([10-platform-bridges.md](10-platform-bridges.md)).
- **Code fluid.** Users can change the apps they use. Themes, layouts, shortcuts, toolbars and text are data the user can edit, safely, through the engine's customization layer ([07-styling-and-customization.md](07-styling-and-customization.md)). Nothing visual is a sealed constant when it could be a token.
- **No fixed caps.** The engine sizes itself from need and from the hardware: caches from device memory and pressure signals, tables that grow, atlases that add pages. The only bounds kept are ones that protect correctness or safety, such as overflow checks and decode bounds derived from what can actually be shown. Those are sized from the hardware and the content, not guessed ([09-performance-and-resources.md](09-performance-and-resources.md)).
- **No stubs.** Every API either works fully or does not exist on that platform, with a clear `Unsupported` error and a documented reason. Nothing returns success without doing the work.
- **Local first, undoable.** Nothing in the engine needs the cloud. User-facing changes the engine manages (edits, customizations, settings it persists) go through one undo history.
- **Operable by people and by agents.** The semantics tree and the command table that serve screen readers also let an AI agent (the RemedyOS mind, or an assistant inside an app) read and operate the UI. This works only through the same permissions and consent the user grants, and every agent action is visible and undoable ([11-accessibility-and-automation.md](11-accessibility-and-automation.md)).

## Modularity

### Layers

Crates form a strict layered graph. A crate may depend only on crates in lower layers, never sideways into another feature pack and never upward. CI checks the graph.

```
L6  app-facing:   remedy-ui (facade, feature flags)          remedy-ui-cli (tooling, not linked into apps)
L5  feature packs: widgets-* · media-* · doc-* · viz-* · services-* · devtools
L4  host:          render-gpu · render-cpu · bridge-windows · bridge-macos · bridge-wayland · bridge-x11
                   bridge-android · bridge-web · bridge-remedyos · bridge-headless
L3  frame:         paint · motion · a11y · compositor
L2  engine:        tree · style · layout · text · input
L1  state:         reactive
L0  base:          core (geometry, color, ids, units, errors; no dependencies)
```

- **Abstractions.** Widgets (L5) never name a renderer or a bridge. They talk to `paint`, `input` and `a11y` abstractions. The app chooses hosts by feature flag, and `remedy-ui` wires them together.
- **Core stays small.** L0 to L3 are the engine every app needs: a few small, heavily tested crates with almost no third-party dependencies beyond taffy, the text stack and AccessKit.
- **Hosts are interchangeable.** An app built only for the RemedyOS desktop links `bridge-remedyos` and nothing for Win32 or Wayland.

### Feature packs

| Pack | Contents | Typical users |
|---|---|---|
| `widgets-core` | Containers, text, buttons, toggles, sliders, selects, inputs, `secret_input`, scroll, virtual list, tooltip, menu, modal, progress, image, icon, canvas | Every app |
| `widgets-nav` | Tabs, nav rail, nav stack, breadcrumbs, customizable toolbar, command palette, sheets, drawers, custom title bar | Multi-screen apps |
| `widgets-data` | Table, tree view, property grid, selection, sort and filter helpers | SecretFolder, Hearth, RemedyOS Files and Monitor |
| `widgets-forms` | Number input, color picker, key capture, date and time pickers, form validation | Craft Calc, settings everywhere |
| `widgets-extra` | QR code, rating, carousel, setup wizard, swatch grid | Portal and RemedyAI pairing, onboarding |
| `widgets-music` | Fretboard, tab staff, chord diagram, note staff | GuitarRemedy, VoiceRemedy |
| `widgets-term` | Terminal emulator element (the PTY stays in the app) | RemedyAI, RemedyOS |
| `doc-rich` | Markdown, rich documents, `RichEditor`, find in page | FreeSpeak, wikis, notes |
| `doc-code` | Code view and editor with syntax colors per grammar feature, diffs | RemedyAI, RemedyOS Editor |
| `doc-pages` | Paged documents: tiled deep zoom, spreads, rotation, text selection, annotation layer (parsing stays in the app) | RemedyPDF |
| `media-image` | Decode pool and cache; formats per sub-feature (`png`, `jpeg`, `webp`, `avif`, `gif`, `ico`) | Most apps |
| `media-anim` | Animated GIF, WebP and APNG; vector animation | FreeSpeak, onboarding |
| `media-svg` | SVG icons and illustrations | Most apps |
| `media-video` | Video surfaces (YUV, RGBA, zero-copy import), external textures | FreeSpeak, Portal, Quickcast |
| `media-playback` | Player transport, subtitles, platform decoders behind a trait | Portal, Quickcast |
| `media-isolated` | Decoding in a sandboxed helper process | FreeSpeak, Hearth, RemedyPDF |
| `viz` | Meters, pitch graph, waveform, spectrum, gauge, sparkline, charts, timeline; streaming inputs | VoiceRemedy, GuitarRemedy, RemedyAI, RemedyOS Monitor |
| `embed-webview` | Platform web view as an element | RemedyAI |
| `embed-gpu` | The app's own wgpu rendering as an element | TheOven |
| `services-desktop` | Tray, notifications, badges, jump lists, file dialogs, single instance, URL activation, autostart, power; each its own sub-feature | Desktop apps |
| `services-update` | Signed updates from GitHub releases or a feed | Most shipped apps |
| `services-spell` | Platform spell checker bridge | Editors, chat |
| `services-i18n` | Message formatting with plurals, locale formatting; locale data per locale at run time | Localized apps |
| `devtools` | Inspector, performance overlay, hot reload, previews; debug builds by default | Development |

[../APPS.md](../APPS.md) maps every Remedy app to the packs it uses. [15-widgets-and-media.md](15-widgets-and-media.md) lists each pack's widgets.

Each pack is its own crate, with its third-party dependencies behind its own feature flags. Turning a pack off removes its code and dependencies from the build entirely.

### Profiles

The facade offers three starting points, and any pack can be added or removed on top:

```toml
remedy-ui = { version = "1", default-features = false, features = ["minimal"] }   # core widgets, CPU renderer, one bridge
remedy-ui = { version = "1" }                                                      # "app": core + nav + forms + image + svg, GPU and CPU renderers, platform bridge
remedy-ui = { version = "1", features = ["full"] }                                 # every pack
```

### Paying only for what is used, at run time too

Compiling a pack in does not make it cost anything until it is used:

- **Subsystems start lazily.** The spell checker connects on the first editor focus. Locale data loads when that locale is chosen. The path pipeline compiles its GPU shaders the first time a path draws, from the on-disk pipeline cache when there is one. Decoder workers spawn on the first decode.
- **Fonts** are memory-mapped and parsed on first use, per face.
- **Accessibility** builds its tree only when an assistive technology or agent connects. It then stays incremental.
- **The compositor thread** is not started for a window that never animates or scrolls. A small utility window renders on the UI thread until it first needs one.
- **Workers** are created as work arrives and park when idle. The pool's size comes from the machine's cores.

### Footprint targets

CI measures these on every merge, for each profile, on Windows x64 and Linux x64 release builds with the binary stripped:

| Profile | Binary added by RemedyUI | Idle memory of a one-window sample | Cold start to first frame |
|---|---|---|---|
| `minimal` (CPU renderer) | ≤ 3 MB | ≤ 20 MB | ≤ 80 ms |
| `app` (GPU + CPU) | ≤ 8 MB | ≤ 45 MB | ≤ 150 ms |
| `full` | ≤ 16 MB | ≤ 60 MB | ≤ 150 ms |

These are targets for the engine's own cost. They are not limits on what an app may do. A regression past a target blocks the merge until it is fixed or the target is changed with a written reason.
