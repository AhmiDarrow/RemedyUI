# 0. Principles and modularity

RemedyUI is the one UI engine for every Remedy app. That covers chat (FreeSpeak), documents, notes, audio and music tools, file and secret managers, calculators, launchers and games' menus, and the RemedyOS desktop itself. An engine that serves all of them must not make each of them carry all of it. This document sets the rules every other design document follows.

## Principles

These are in priority order. When two pull against each other, the higher one wins.

1. **Stable.** It runs the same way every time. No panic reaches a frame, no hang reaches the user, and it starts on any machine. Judge by repeat runs, not one pass.
2. **Secure and private.** No unsafe code outside an audited boundary. Untrusted bytes are parsed by memory-safe code off the UI thread. The engine never phones home, logs content, or needs an account or a network. The packs that use the network, `services-update` and `services-net`, are opt-in, talk only to the addresses the app names, and send nothing that identifies the user.
3. **Fast as the hardware allows.** Work is proportional to change. Nothing runs when nothing changes, and input reaches the screen within a frame.
4. **Efficient and light.** An app pays only for what it uses, in binary size, memory and CPU (see Modularity).
5. **Intuitive for people.** Good behavior is the default: platform conventions, accessibility, undo, feedback, motion that explains. An app built quickly on RemedyUI should already feel right ([13-design-language.md](13-design-language.md)).
6. **A pleasure to build with.** One obvious way to do each thing, names that read well, errors that point at the fix, live previews, fast rebuilds ([12-developer-experience.md](12-developer-experience.md)).
7. **Full featured.** What modern engines offer is here, so apps never need forks or workarounds.

Rules that come from the Remedy tenets:

- **Remedy's own design.** RemedyUI has its own model of windows, input, text, rendering and semantics. Windows, macOS, Linux, Android, the web and RemedyOS are **bridges** that translate that model to each platform. No platform's shape leaks into the core, and the RemedyOS native bridge is a first-class target, not an afterthought ([10-platform-bridges.md](10-platform-bridges.md)).
- **Code fluid.** Users can change the apps they use. Themes, layouts, shortcuts, toolbars and text are data the user can edit, safely, through the engine's customization layer ([07-styling-and-customization.md](07-styling-and-customization.md)). Nothing visual is a sealed constant when it could be a token.
- **No fixed caps.** The engine has no fixed capacity limits: caches size from device memory and pressure signals, tables grow, atlases add pages. The only bounds kept are ones that protect correctness or safety, such as overflow checks and decode bounds derived from what can actually be shown, and those are sized from the hardware and the content. Tuning constants (a gesture's time window, a hinting threshold) are not caps; each is documented in the code with its reason ([09-performance-and-resources.md](09-performance-and-resources.md)).
- **No stubs.** Every API either works fully or does not exist on that platform, with a clear `Unsupported` error and a documented reason. Nothing returns success without doing the work.
- **Local first, undoable.** Nothing in the engine needs the cloud. User-facing changes the engine manages (edits, customizations, settings it persists) go through one undo history.
- **Operable by people and by agents.** The semantics tree and the command table that serve screen readers also let an AI agent (the RemedyOS mind, or an assistant inside an app) read and operate the UI. This works only through the same permissions and consent the user grants, and every agent action is visible and undoable ([11-accessibility-and-automation.md](11-accessibility-and-automation.md)).

## Modularity

### Layers

Crates form a strict layered graph. A crate may depend only on crates in lower layers, plus the few sideways edges listed below, and never upward. CI checks the graph against this list.

```
L7  tools:        cargo-remedy::devkit (the developer tools' logic) · cargo-remedy::templates (app templates)
                  remedy-ui-build (build scripts: the Windows executable's icon and version)
                  cargo-remedy (CLI) · remedy-mcp (MCP server for AI agents); none linked into apps
L6  app-facing:   remedy-ui (facade, feature flags) · remedy-ui-widgets::rui (markup compiler and runtime) · remedy-ui-core::testing
L5  feature packs: widgets-* · media-* · doc-* · viz · embed-* · services-* · themes · fonts · devtools · customize
L4  host:          view (elements, the frame pipeline, input routing) · render-gpu · render-cpu · bridge-winit (shared desktop core) · bridge-windows · bridge-macos · bridge-linux
                   bridge-android · bridge-web · bridge-remedyos · bridge-headless
L3  frame:         paint (display list, image and icon sources) · motion · a11y · compositor
                   host (the platform-neutral host model every bridge implements)
L2  engine:        tree · style · layout · text · input
L1  state:         reactive
L0  base:          core (geometry, color, ids, units, clocks, the phase spans every layer emits)
```

The allowed sideways edges inside L5:

- every `widgets-*` pack, `doc-*` pack, `viz` and `embed-*` pack may depend on `widgets-core`
- `widgets-music` may depend on `viz`
- `widgets-stage` may depend on `viz`
- `doc-rich` may depend on `doc-code`, behind its `code-blocks` feature, for colored code blocks
- `media-anim` may depend on `media-image`
- `media-playback` may depend on `media-video` (its pictures go to a video surface) and `widgets-core` (its transport bar), and on `services-desktop` behind its `media-session` feature (what plays shows in the system's media controls) and its `keep-awake` feature (the screen held on while video plays, through `power::inhibit`)

Image and icon *sources* are a trait in `paint`, so `widgets-core` can show images without knowing any decoder; `media-image` registers decoders for them (SVG among them, with its `svg` feature).

**Unsafe code** lives only in the bridges, `render-gpu`, and the few packs that must call the platform themselves: `services-update` (Authenticode), `services-desktop` (shell links), `services-spell` (the platform's checker), `media-playback` (the platform's decoders and sound output), `media-isolated` (the decoder sandbox's processes) and `embed-webview` (the platform's web view as a native child view). Each of those crates denies `unsafe_code` at its root and allows it only on its `os` (or `ffi`) module; every other crate forbids it outright, and CI checks that no other module carries the allow (the list is `UNSAFE_BOUNDARY` in `xtask`). Keeping such calls in the pack that needs them means an app pays for a platform API only when it uses that pack. Packs that need host services shared by everything else (tray and notifications) call bridge traits, so they stay free of unsafe code.

- **Abstractions.** Widgets (L5) never name a renderer or a bridge. They talk to `paint`, `input` and `a11y` abstractions. The app chooses hosts by feature flag, and `remedy-ui` wires them together.
- **Core stays small.** L0 to L3 are the engine every app needs: a few small, heavily tested crates with almost no third-party dependencies beyond taffy, the text stack and AccessKit.
- **Hosts are interchangeable.** An app built only for the RemedyOS desktop links `bridge-remedyos` and nothing for Win32 or Wayland.

### Feature packs

| Pack | Contents | Typical users |
|---|---|---|
| `widgets-core` | Containers, text, buttons, toggles, sliders, selects, inputs, `secret_input`, scroll, virtual list, tooltip, menu, modal, progress, image and icon (over `paint`'s source trait), canvas | Every app |
| `widgets-nav` | Tabs, nav rail, nav stack, breadcrumbs, customizable toolbar, command palette, sheets, drawers, custom title bar | Multi-screen apps |
| `widgets-data` | Table, tree view, property grid, selection, sort and filter helpers | SecretFolder, Hearth, RemedyOS Files and Monitor |
| `widgets-forms` | Number input, color picker, key capture, date and time pickers, form validation | Craft Calc, settings everywhere |
| `widgets-extra` | QR code, rating, carousel, setup wizard, swatch grid | Portal and RemedyAI pairing, onboarding |
| `widgets-stage` | Scenes with art and named anchors, pixel-art sprites, orbits, path layouts, knobs and tuners, faders, timelines, big physical buttons, display boards | FreeSpeak's themes, games and playful apps |
| `widgets-music` | Fretboard, tab staff, chord diagram, note staff | GuitarRemedy, VoiceRemedy |
| `widgets-term` | Terminal emulator element (the PTY stays in the app) | RemedyAI, RemedyOS |
| `doc-rich` | Markdown, rich documents, `RichEditor`, find in page | FreeSpeak, wikis, notes |
| `doc-code` | Code view and editor with syntax colors per grammar feature, diffs | RemedyAI, RemedyOS Editor |
| `doc-pages` | Paged documents: tiled deep zoom, spreads, rotation, text selection, annotation layer (parsing stays in the app) | RemedyPDF |
| `media-image` | Decode pool and cache; formats per sub-feature (`png`, `jpeg`, `webp`, `avif`, `gif`, `ico`); SVG icons and illustrations (`svg`) | Most apps |
| `media-anim` | Animated GIF, WebP and APNG; vector animation | FreeSpeak, onboarding |
| `media-video` | Video surfaces (YUV, RGBA, zero-copy import), external textures | FreeSpeak, Portal |
| `media-playback` | Player transport, subtitles, platform decoders behind a trait | Portal |
| `media-isolated` | Decoding in a sandboxed helper process | FreeSpeak, Hearth, RemedyPDF |
| `viz` | Meters, pitch graph, waveform, spectrum, gauge, sparkline, charts, timeline; retained primitives the compositor draws straight from streaming ring buffers | FreeSpeak voice ring, VoiceRemedy, GuitarRemedy, RemedyAI, RemedyOS Monitor |
| `embed-webview` | Platform web view as an element | RemedyAI |
| `embed-gpu` | The app's own wgpu rendering as an element | Game views, 3D previews |
| `services-desktop` | Tray, notifications, badges, jump lists, file dialogs, single instance, URL activation, autostart, power; each its own sub-feature | Desktop apps |
| `services-update` | Signed updates from GitHub releases or a feed the app names; opt-in | Most shipped apps |
| `services-net` | HTTP client natively (rustls) and in a page (`fetch`): JSON with typed decoding, errors with their detail, multipart uploads, keepalive requests, timeouts, cancellation that follows a `Resource`'s key, cookies; opt-in, contacts only the addresses the app names | Portal, apps with a server |
| `themes` | The shared Remedy theme set | Every Remedy app |
| `fonts` | The Remedy font families, one feature per family | Every Remedy app |
| `services-spell` | Platform spell checker bridge | Editors, chat |
| `services-i18n` | Message formatting with plurals, locale formatting; locale data per locale at run time | Localized apps |
| `services-webstorage` | Read-only import of a web view's localStorage (Chromium's LevelDB store) or the page's own | Apps moving off Tauri, Capacitor or a PWA |
| `devtools` | Inspector, performance overlay, hot reload, previews; debug builds by default | Development |
| `customize` | The code-fluid layer: the person's theme, shortcuts, toolbars and menus, and text as files, with undo, resets and the editors | Every Remedy app |

[15-widgets-and-media.md](15-widgets-and-media.md) lists each pack's widgets.

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

- **Subsystems start lazily.** The spell checker connects on the first editor focus. Locale data loads when that locale is chosen. Decoder workers spawn on the first decode. (GPU pipelines are the exception: the fixed set is built at start-up so nothing compiles mid-session; see [04](04-rendering.md).)
- **Fonts** are parsed on first use, per face.
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
