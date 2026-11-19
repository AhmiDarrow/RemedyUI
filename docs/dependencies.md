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
| Grapheme and word boundaries | `icu_segmenter` | 2.3.0 (the version parley uses) | `text` | Editing never splits a user-perceived character |
| Spell checking | `spellbook` | 0.4.2 | `services-spell` | Pure-Rust reader of Hunspell `.aff`/`.dic` dictionaries (MPL-2.0, used unmodified); the engine on Linux and the fallback elsewhere |
| Dictionary charsets | `encoding_rs` | 0.8.35 | `services-spell` | Reads old Hunspell files in ISO-8859-x and KOI8 (`(Apache-2.0 OR MIT) AND BSD-3-Clause`) |
| Hyphenation | `hyphenation` | 0.8.4 | `text` (feature) | Knuth–Liang dictionaries per language, each a feature |
| Look-alike detection | `unicode-security` | 0.1.3 | `text` | UTS #39 skeletons for confusable names (`skeleton`, `looks_like`) |
| Secret buffers | `zeroize` | 1.9.1 | `text`, `secure` | Zeroing that the compiler cannot optimize away |
| Layout algorithms | `taffy` | 0.15.1 (flexbox, grid, block, content-size) | `layout` | CSS flexbox, grid and block, faithful to the specs. Its leaf callback runs `compute_leaf_layout` so styles apply; it keeps padding physical, so RemedyUI mirrors logical insets for right to left itself |
| GPU | `wgpu` | 30.0.2 | `render-gpu` | D3D12, Vulkan, Metal, GL 3.3 / GLES 3.0 |
| Paths | `vello_gpu`, `vello_cpu` | 0.4.0 | `render-gpu`, `render-cpu` | Sparse-strip paths without compute shaders; fast multithreaded CPU rendering. Pre-1.0: behind RemedyUI's path type |
| Shader validation | `naga` | wgpu's | `render-gpu` | Validates app-supplied WGSL effects |
| CPU present | `softbuffer` | 0.4.8 | `render-cpu` | Pixel blit on every desktop |
| Windows and events | `winit` | 0.30.13 (0.31 when stable) | desktop and Android bridges | Each bridge adds its own modules where winit stops short |
| Accessibility | `accesskit` + adapters | 0.25.2 (core types in `a11y`; adapters per bridge) | `a11y` | UI Automation, NSAccessibility, AT-SPI, Android |
| Clipboard | `arboard` | 3.7.0 | desktop bridges | Text and images; sensitive-item handling added in the bridge |
| File dialogs | `rfd` | 0.18.0 | `bridge-windows`, `bridge-macos` | Native dialogs. On Linux, dialogs go through the XDG portal with RemedyUI's own client over blocking `zbus`, because `rfd`'s portal backend needs an async runtime |
| D-Bus (Linux) | `zbus` (blocking API) | 5.19.0 | `bridge-linux` | Portal, notifications, StatusNotifierItem, AT-SPI |
| Images | `image` / `zune-image` | 0.25.10 / 0.5.0 | `media-image`, `media-isolated` (its PNG encoder makes the picture the helper is timed on) | Chosen per format in its milestone, after a RustSec review and fuzzing |
| SVG | `usvg` | pinned with `media-svg` | `media-svg` | SVG to a path tree |
| Syntax colors | `syntect` (with `fancy-regex`) | 5.3.0 | `doc-code` | Pure-Rust highlighting, with no C regex engine |
| Terminal parsing | `vte` | 0.15.0 | `widgets-term` | Paul Williams' DEC state machine as Alacritty uses it (Apache-2.0 OR MIT, maintained by the Alacritty project); bytes to print, execute, CSI, OSC and DCS actions. The screen model on top is RemedyUI's own. Its unbounded OSC buffer is cut by the terminal's own string limit |
| Character widths | `unicode-width` | 0.2.3 (already in the tree through the text stack) | `widgets-term` | UAX #11 widths: which characters take two cells |
| Base64 | `base64` | 0.23.1 (already used by `services-update`) | `widgets-term` | OSC 52 clipboard payloads |
| Diagnostics | `tracing` | 0.1.44 | every crate | Structured spans and events; Perfetto export in `devtools` |
| Errors | `thiserror` | 2.0.21 | every crate | Typed errors without hand-written boilerplate |
| Derive macros | `syn` 2, `quote`, `proc-macro2` | 2.0.119, 1.0.47, 1.0.107 | `macros` (build time only) | `#[derive(Store)]`; syn 2 rather than the new 3.0 until 3.x has settled |
| Source migrations | `syn` 2 (`full`, `visit`) and `proc-macro2` (`span-locations`) | 2.0.119, 1.0.107 (the macro crates' versions) | `cargo-remedy` (a tool, not linked into apps) | `cargo remedy migrate` parses an app's sources and edits at token spans, so renames never touch strings or comments |
| Gzip | `flate2` (`rust_backend`) | 1.1.11 (already in the tree through `zip`) | `cargo-remedy` | Linux packages are `.tar.gz`; MIT OR Apache-2.0 |
| Crate paths in derives | `proc-macro-crate` | 3.5.0 (already in the tree through `toml_edit` users) | `macros` (build time only) | Finds how the deriving crate names RemedyUI (the `remedy-ui` facade, or `remedy-ui-reactive` directly, renamed or not), so generated code works for both; MIT OR Apache-2.0 |
| Platform APIs | `windows`, `objc2`, `ndk`, `jni` | pinned per bridge | bridges | OS calls, in the bridges' `os` modules only |
| Raw platform calls | `windows-sys`, `libc` | 0.61.2, 0.2.191 | `bridge-windows`, `bridge-linux`, `services-update`, `media-isolated`, `embed-webview` | The decoding helper's sandbox (AppContainer, restricted token, job object; seccomp-bpf, Landlock, memfds, descriptor passing; Seatbelt), in `media-isolated`'s `os` module only; `dlopen` of GTK 3 and WebKitGTK in `embed-webview`'s `os` module, so apps build and start without them |
| Android activity | `android-activity` (through winit's `android-native-activity`) | 0.6.1 | `bridge-android` | The `NativeActivity` glue winit runs on: lifecycle, input queue, soft keyboard, the Java main thread |
| Java calls (Android) | `jni` | 0.21.1 (the release `accesskit_android` uses) | `bridge-android` | Clipboard, links, settings, `FLAG_SECURE` and the bridge's own Java half, in the `os` module; android-activity uses 0.22 internally, so two releases are built on Android only |
| Screen readers (Android) | `accesskit_android` (`embedded-dex`) | 0.9.1 | `bridge-android` | AccessKit's adapter injected into the native activity's view (accesskit_winit's Android adapter only reaches GameActivity) |
| Web view (Windows) | `webview2-com` (with `webview2-com-sys`, `webview2-com-macros`) | 0.39.1 | `embed-webview` | WebView2's COM interfaces and callback helpers over `windows` 0.62 (MIT). Its `WebView2LoaderStatic.lib` (Microsoft's loader from the WebView2 SDK, BSD-3-Clause) is linked statically: the one prebuilt library RemedyUI links, chosen over shipping `WebView2Loader.dll` beside every app (a file to plant) or reimplementing Microsoft's undocumented loader protocol. It only finds and starts the Evergreen runtime the system keeps updated |
| Browser bindings | `wasm-bindgen`, `js-sys`, `web-sys` | 0.2.130, 0.3.107, 0.3.107 | `bridge-web`, `media-playback` (web: WebCodecs and WebAudio), and wgpu's web backends | The one maintained way for Rust in a page to call the DOM and the browser's APIs; `web-sys` with only the features the bridge uses. MIT/Apache-2.0, maintained by the Rust and WebAssembly working group. Browser-only: built for `wasm32-unknown-unknown` alone |
| Browser promises | `wasm-bindgen-futures` | 0.4.80 | `bridge-web`, `render-gpu`, `media-playback` (web) | Turns the browser's promises (GPU adapters and devices, file reads, the clipboard, Web Locks) into futures run by the page's own microtask queue: a bridge to the browser's event loop, not an async runtime |
| Browser clocks | `web-time` | 1.1.0 | `core` (web) | `Instant` and `SystemTime` from `performance.now()` and `Date.now()` with std's API, since std's panic in a page. On native targets the engine uses std's directly. MIT/Apache-2.0 |
| Browser tests | `wasm-bindgen-test` | 0.3.80 | `bridge-web`, `media-playback`, `widgets-board` (dev only) | Runs the browser tests in headless Firefox and Chrome through WebDriver |
| Script language | `rhai` | 1.27.0 (`std`, `no_time`, `debugging`) | `script-rhai` | The language for widgets people write by hand: pure Rust (so it runs in a page too), made for embedding untrusted scripts (no file, network or process access of its own; operation, call-depth, expression-depth and data-size limits; a progress hook that can stop a script), MIT/Apache-2.0, actively maintained. Its debugger interface is the one hook that sees a running call's variables, which is how the engine measures memory, since Rhai has no allocator of its own |
| WebAssembly interpreter | `wasmi` | 2.0.1 (`std`, `validate`, `deterministic`, `auto-dispatch`) | `script-wasm` | Runs compiled widgets: pure Rust with no JIT (so it runs in a page, where a JIT cannot be had), validates modules, meters fuel with calls that resume after running out (how the engine checks the clock in a tight loop), limits memory, tables, call depth and its stack, and has deterministic floats; MIT/Apache-2.0, maintained, audited, used by blockchains and Parity for exactly this kind of untrusted code |
| WebAssembly text | `wat` | 1.263.0 | `script-wasm`, `widgets-board` (dev only) | Writes the hostile test modules as text; from the Bytecode Alliance's wasm-tools (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) |
| Thread processor time | `cpu-time` | 1.0.0 | `core` (off the web) | `ThreadTime` and `WorkClock`: the calling thread's own processor time (`GetThreadTimes` on Windows, `CLOCK_THREAD_CPUTIME_ID` elsewhere), so script time limits and speed measurements count the work done on their thread, not time the system gave other work. Its unsafe calls stay in the dependency (MIT or Apache-2.0). |
| JSON | `serde_json` | 1.0.154 | `script`, `script-rhai`, `script-wasm`, `themes`, `services-i18n`, `services-update`, `widgets-board`, `embed-webview` | JSON data files; in `embed-webview`, the values of the one message channel between the app and a page |

Development tools that are not linked into apps: `cargo-fuzz`, `cargo-deny`, `cargo-audit`, `cargo-vet`.

### The developer tools

`remedy-tools`, `cargo-remedy` and `remedy-mcp` are tools, and no app links them. They add:

| Area | Crate | Version | Tool | Why this one |
|---|---|---|---|---|
| Model Context Protocol | `rmcp` | 3.11.0 | `remedy-mcp` | The official Rust MCP SDK (Apache-2.0): its server, the stdio and streamable-HTTP transports, and the client the tests script. It brings `chrono`, which is marked unmaintained (a maintenance notice, recorded in `deny.toml`) |
| Async runtime | `tokio` | 1.54.1 | `remedy-mcp` | rmcp runs on it. The tools themselves block, and the server runs them on tokio's blocking pool |
| HTTP server | `hyper`, `hyper-util`, `http-body-util`, `bytes`, `tower-service` | 1.12.0, 0.1.21, 0.1.5, 1.12.1, 0.3.3 | `remedy-mcp` (feature `http`) | Serves rmcp's streamable-HTTP service on loopback, behind the bearer-token check |
| JSON schemas | `schemars` | 1.2.3 | `remedy-mcp`, `remedy-tools` (feature `schema`) | Every tool's input schema comes from its typed input, with the field docs as descriptions (the version rmcp uses) |
| Compressed docs index | `flate2` (miniz_oxide back end) | 1.1.11 | `remedy-tools` (feature `docs`), `xtask` | The committed docs index is zlib-compressed JSON (about 470 KB instead of 1.6 MB); the version `cargo-remedy` already uses for packaging |
| Manifests | `toml` | 1.2.1 | `cargo-remedy`, `remedy-tools` (feature `preview`) | Reads an app's Cargo.toml and writes the preview harness's |

## Rules

- **A new dependency needs a reason** in this file: what it does, why RemedyUI should not write it, its license, and its maintenance health.
- **Minimal features.** `default-features = false`, then only what is used.
- **No async runtime, and no bundled C code,** in anything an app links. (`remedy-mcp` runs on tokio because its protocol SDK does, and no app links it.) Platform frameworks the OS provides (D3D, Metal, Vulkan loaders, system GStreamer on Linux, WebKitGTK) are loaded from the OS, never shipped. The one exception is the WebView2 loader, explained in its row above.
- **Checked in CI**: `cargo deny` (licenses, advisories, bans, duplicate versions), `cargo audit` and `cargo vet`.
- **One owner module.** Each dependency is used from one internal module, and its types do not appear in public APIs.
- **Young crates** (parley, the vello renderers, winit 0.31, accesskit) have behavior tests in RemedyUI for what the engine relies on, so an upgrade that changes behavior is caught before release.
