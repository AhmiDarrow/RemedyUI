# RemedyUI roadmap

Each milestone ends with something real running and tested. From M5 on, each also ends with a Remedy app moved onto the engine. Apps go smallest first, so each one proves a new part of the engine before the large ones depend on it. Every pack an app needs lands in that app's milestone or earlier. This file is the source of truth for milestone numbers; [APPS.md](APPS.md) refers to it.

Nothing is merged as a placeholder: a crate enters the workspace with its first real code and tests. Each milestone is judged by repeat runs of its scenario suites, its benchmarks against their targets, and its fuzzers.

## M1. Foundations

**Status:** built, except `cargo vet`. The crates, their tests and the local checks pass, with 10 of 10 repeat runs green. `cargo vet` is not set up yet: installing the tool failed on the build machine with an access-denied error. GitHub Actions cannot run on this private repository until the account's Actions billing is sorted, so the same checks run locally before each merge.

**Crates:** `core`, `reactive`, `tree`, and the first cut of `test` (the headless runner with a fake clock and scripted input).

**Contents:**
- glitch-free signals, memos, effects, scopes and disposal
- stores and `SignalVec`
- the `Waker`
- the element arena, dirty flags, event dispatch, focus and error boundaries
- workspace rules in CI: lints, formatting, docs, `cargo deny`, `audit` and `vet`, and the layer-graph check

**Done when:**
- dirty counters prove that only the dependents of a changed signal update
- an idle runtime does no work
- posts from other threads are batched into one frame
- a panicking handler is contained by its boundary
- the CI rules reject an upward dependency

## M2. Text

**Status:** built, with the open items listed below. Done:

- `remedy-ui-fonts`
- `remedy-ui-text` layout: fonts and fallback, rich spans, inline objects, wrapping and rewrapping, alignment, bidi runs, hit testing, carets, selection rects, words, visual movement

- glyph rasterization: skrifa hinting at the pixel size, an exact-area coverage rasterizer, grayscale and LCD with the FreeType filter
- the glyph atlas: shelf-packed pages that grow on demand, upload regions, page eviction under pressure

- the editing core: a rope buffer, grapheme-safe deletion and movement, Unicode word motion, selection, undo and redo grouped by word and pause, IME composition (preedit, commit, cancel)
- secrets: `SecretString`, which zeroes memory on growth, removal and drop and never prints, and `SecretEditor` with no undo

- deceptive text: invisible bidi and zero-width controls revealed (Trojan Source), UTS #39 confusable skeletons for look-alike names
- measured: 50,000 combining marks on one letter lay out in about 11 ms and a 1 MB single paragraph in about 150 ms (release build), both linear

Open:

- Per-platform IME protocols. These arrive with each bridge (TSF in M4).
- The Arabic and CJK reference-shape test. It needs OFL fixture fonts for those scripts, since the Remedy families have none.
- The FreeType comparison of hinted glyphs. FreeType is C and not in the build, so the current check is crispness: hinted stems and crossbars land on whole pixels where unhinted ones do not.
- Laziness inside one huge paragraph. This comes with `doc-rich`, where documents lay out line by line.
- the Arabic shaping reference test, which needs an OFL Arabic fixture font (the Remedy families have no Arabic)

**Crates:** `text` and `fonts` (the Remedy families, for tests and apps).

**Contents:**
- itemize, shape, break and position
- hinting and LCD rasterization, and the atlases' CPU side
- rich paragraphs with inline objects
- selection, and the editing core with undo and the zeroizing secret buffer
- IME through the headless runner's IME scripts

**Done when:**
- Latin, Arabic, Hebrew and CJK paragraphs lay out to match reference shapes
- hinted Remedy Sans UI glyphs match FreeType within tolerance
- selection spans paragraphs
- Zalgo and bidi-spoofing cases stay within frame time and render visibly
- a secret buffer is verified zeroed after drop

## M3. Layout, style, input and semantics

**Status:** built, with the open items listed at the end. `remedy-ui-layout` is built: flex, grid and block through taffy behind RemedyUI's own styles; a viewport root, so an auto-width root fills the window like a page body; inherited direction with logical insets mirrored for right to left; measured leaves; absolute and pixel-snapped rects; and the virtual list (estimates, Fenwick offsets, anchoring through inserts, removals and re-measures, stick-to-end, jump to any row).
`remedy-ui-motion` is built: cubic-Bézier easing with the standard curves, tweens, closed-form springs (stiffness and damping, or duration and bounce), retargeting that keeps position and velocity, animatable geometry and Oklab colors, the frame clock with presentation-time prediction and deadlines, and reduced motion. `remedy-ui-style` is built: typed themes (color roles, type scale, spacing, radii, motion, density), Dark Forest from RemedyAI's palette, derived tokens with Oklab-preserving contrast balancing (text AAA, muted and on-accent AA), state flags, and variant- and state-aware resolution cached by theme version. `remedy-ui-input` is built: the event model (pointers with kind, buttons, pressure and coalesced history; layout-aware keys with physical position and composing state; wheels), least-squares velocity, the gesture arena (tap, long press, pan, axis pans, platform slops), scroll axes with exponential fling decay and clamped or rubber-band edges, and the command table with scopes, enabled state, rebinding with conflict checks and platform chord labels. `remedy-ui-a11y` is built. It provides:
- RemedyUI's semantics model (roles, labels, values, numeric ranges, states, actions, set positions, live regions, hidden)
- incremental AccessKit tree updates that start only when a client connects
- merged rows with composed labels
- password inputs whose text is never exposed, not even through a merged parent

Open in M3:
- AccessKit ships no schema validator. The tests check that the produced updates have the expected roles, names, values and structure.
- Wiring these crates into the tree and frame (bindings that set semantics, styles and layout from elements) arrives with the first widgets in M5, which is where they first have callers.

**Crates:** `layout` (taffy behind RemedyUI types, pixel snapping, virtualization with anchoring), `style` (tokens, derived tokens, states, contrast helpers), `input` (event model, hit testing, gesture arena, scroll physics, focus, commands), `a11y` (semantics tree and incremental AccessKit updates), and `motion` (frame clock, transitions, springs). All of it is tested headless.

**Done when:**
- layout snapshots match references
- a virtual list keeps its anchor through inserts above it
- the gesture arena resolves nested pans
- the semantics tree passes AccessKit's schema checks for test scenes

## M4. Rendering, the compositor and Windows

**Status:** built (PRs #18-#27). `remedy-ui-paint` is built: display lists of rounded rects, borders, shadows, paths and strokes, gradients, glyph runs, images, clips, layers and transforms, with conservative bounds. `remedy-ui-render-cpu` is built: it draws display lists through vello_cpu at any device scale, with glyphs rasterized by RemedyUI's own hinted rasterizer, cached per size and subpixel bin, and composited in order so clips and layers apply. `remedy-ui-compositor` is built: layer trees shared through `Arc` and handed over in a single-slot mailbox; compositor-owned scroll offsets with fling and overscroll physics and wheel routing that hands an exhausted inner scroller outward; compositor-side offset, scale and opacity animations (reduced motion honored); 256/512-pixel tiles, rastered visible-first with a one-tile prefetch ring inside scroll containers, released on memory pressure or scale change; and per-layer damage in device pixels. `CpuCompositor` runs the plans, and its frames match direct rendering. `remedy-ui-render-gpu` is built: a backend ladder (D3D12/Vulkan/Metal, then GL, then a software adapter), display lists drawn through vello_gpu with the CPU renderer's own painter (glyphs and nearest-sampled images packed into RGBA atlas pages, filtered or large images in their own textures), compositor tiles in textures, a persistent pipeline cache where the driver has one (one unsafe call, in `ffi`), and device-loss recovery that rebuilds the device and re-rasters from the retained layer tree with no app code. Golden images match the CPU renderer (RTX 3080 on Vulkan: no pixel differs visibly at 1x or 2x). `remedy-ui-host` is built: the platform-neutral host model (windows, frames, input, input-method sessions, clipboard, timers, wake, settings, memory pressure, CPU presents with damage, native handles for GPU surfaces) that every bridge implements, with `Unsupported` for what a platform cannot do. `CompositorThread` runs the compositor on its own thread: scrolling keeps the display's rate while the UI thread is stalled, and an unchanged frame presents nothing (both tested). `remedy-ui-bridge-headless` is built: the host in memory with a scripted clock, input, IME, clipboard, timers and wakes; presents copy only their damage, so tests prove damage is complete. `remedy-ui-bridge-winit` is built: one host implementation for every desktop winit serves (windows, pointer, touch, wheel and keys, IME sessions with caret area and private-input purpose, frames on redraw, timers, cross-thread wakes, softbuffer presents writing only damage when the buffer holds the previous frame, native handles for GPU surfaces); platform bridges add only `PlatformHooks`. `remedy-ui-bridge-windows` is built on it: the Win32 clipboard (sensitive copies excluded from monitors, clipboard history and cloud sync), capture protection, and real system settings; a real-window test runs the whole contract. GPU frames present to windows: `GpuSurface` picks an 8-bit RGBA/BGRA swapchain format with its own pipelines, paces to vsync, handles occluded, outdated and lost swapchains, and rebuilds itself on the new device after a device loss (tested on a real window through a resize and a forced loss). GPU validation errors are recorded rather than panicking. wgpu does not expose partial presents, so GPU windows present whole frames while the CPU path presents damage. `remedy-ui-bridge-macos` (AppKit pasteboard with concealed/transient markers for sensitive copies, `NSWindowSharingNone`, appearance, accent, contrast, motion, transparency, locale, double-click interval) and `remedy-ui-bridge-linux` (one crate for Wayland and X11: arboard clipboard with the password-manager hint, XDG portal settings over zbus, honest `Unsupported` for capture protection) are built on the shared core and lint clean when cross-compiled for macOS and Linux here; they still need a run on those machines. LCD text draws on both renderers (two passes per run: a multiply by `1 - coverage` per channel, then the color added per channel; exact over opaque pixels, grayscale inside groups), and the GPU matches the CPU. TSF is built: a safe UTF-16 text store (selection, composition, reconversion, surrounding text, edits turned into input-method events) behind a COM `ITextStoreACP` with TSF's lock protocol, one document per window, password fields with no input method; tested by driving the COM store exactly as TSF does and by TSF connecting to it on a real window. No pipeline or shader is created after start-up (wgpu resource counters, tested). Every M4 done-when item has a test. Moved on: showing old tiles scaled while new ones raster during a zoom goes with `doc-pages` in M8, its first user; today a zoom re-rasters visible tiles at once.

**Crates:** `paint`, `compositor`, `render-gpu`, `render-cpu`, `bridge-windows` (with TSF), `bridge-headless`.

**Contents:**
- paint chunks, layers, tiles and the commit
- compositor-side scrolling and animation
- damage-only presenting
- the start-up pipeline set, the backend ladder and device-loss recovery

**Done when:**
- golden images match between the GPU and CPU renderers
- a forced device loss recovers with no app code
- scrolling keeps the display's rate while the UI thread is artificially stalled
- an unchanged frame presents nothing
- no shader compiles after start-up in a recorded session

## M5. First app: SecretSticky

**Status:** built (PRs #28-#39). `remedy-ui-view` is built (an L4 crate, so every widget pack builds on the same public API apps use): elements with theme-styled widget names, variants and states; reactive bindings that mark only the work a change needs; text and editable fields (single-line, multi-line, secret) with undo, clipboard requests and input-method composition; per-element paint chunks re-recorded only when dirty; layers assembled with scroll containers on the compositor and paint order kept around them; hover, press, click, focus and Tab routing; hit testing that follows scroll offsets; semantics for AccessKit with secrets never exposed. `KeyEvent` gained the typed text (case, layout, AltGr). `remedy-ui` is built: `run` starts the platform's bridge and drives every window (view frames into a compositor, presented with the GPU renderer sharing one device across windows, falling back to the CPU renderer with damage-only presents); `App` and `Window` handles for opening, closing, hiding, focusing, always-on-top, titles, custom-chrome moves and resizes, timers, the clipboard and quitting; hide-on-close and keep-running for tray apps; window geometry persisted per key in the app's data directory. The host contract gained window position and moves, focus, minimize, and user move/resize. Tested headless and on a real Windows window with the GPU. `remedy-ui-themes` is built: the twelve Remedy palettes (generated from the shared source by `tools/gen_palettes.py`), `System` following the platform's light/dark, high contrast, and the widget style sheet built only from tokens; every widget color pair meets WCAG contrast in all 24 themes (tested). The view gained an overlay layer for dialogs and menus (click-through, always above content), timers, key and pointer handlers, focus scopes with first-control focus, layout shorthand, images, and a `mono` style property. `remedy-ui-widgets-core` is built with the full set from the design: containers (column, row, wrap, stack, spacer, divider, card, section, scroll, split), keyed `for_each`, selectable `list` and a variable-height `virtual_list`, text (text, heading, label variants, link), buttons (button, icon_button with required label and tooltip, toggle, menu button), choices (check, switch, radio group, segmented, select with type-ahead, chip), values (slider, stepper), inputs (text input, text area, search, secret input, labeled field), feedback (progress, spinner that holds still under reduced motion, lamp, badge, self-dismissing announced toast, empty state, skeleton), overlays (tooltip, menu, context menu, popover, modal with focus trap and focus return, dialog, trusted prompt in a layer above all app overlays with delayed buttons, lock screen that removes content from the frame and the accessibility tree), and media (26 vector icons, image, avatar, canvas). Absolute boxes can leave inset sides `AUTO`. `remedy-ui-secure` is built: an idle timer fed only by real input (the runner feeds it pointer, key, wheel and input-method events and nothing else) and sensitive copies remembered by a keyed fingerprint, cleared after their time only if the clipboard still holds them. `App::idle_lock`, `App::copy_secret`, `WindowSpec::privacy_cover`, a reveal toggle for secret inputs, and `Host::clear_clipboard` on every bridge. Desktop services for M5 are built into the host contract and every bridge: the tray (Windows `Shell_NotifyIcon` on a message-only window with a native menu, macOS `NSStatusItem` with an `NSMenu`, Linux StatusNotifierItem with dbusmenu over zbus 6, reported `Unsupported` where a desktop has no tray host), native file dialogs (rfd on Windows and macOS, the XDG portal's FileChooser on Linux), and single instance (a session mutex and named pipe on Windows, a per-user Unix socket elsewhere; a second launch hands over its arguments and exits). Tested headless and, on Windows, with a real tray icon and a real pipe hand-off; the macOS and Linux parts lint clean cross-compiled and still need a run on those machines. The rest of `services-desktop` (notifications, badges, jump lists, autostart, URL activation, power) arrives with its first app. Screen readers are connected on every desktop through AccessKit's platform adapters (UI Automation, NSAccessibility, AT-SPI): the tree goes out only once a reader asks for it, and click, focus, increment, decrement and scroll-into-view requests come back to the elements; tested end to end headless. `deny.toml` records a narrow exception for three zbus 5 maintenance notices that AccessKit's Linux adapter still brings. `remedy-ui-services-update` is built: checks a GitHub repository's latest release or a JSON feed, downloads, refuses anything that does not verify against the app's minisign key (plain or Tauri's base64-wrapped signatures) and the published SHA-256, refuses plain http beyond the loopback interface, and applies by running the installer or swapping the executable; plus `update_banner`. TLS is rustls on ring with the platform's certificate verifier. `App::background` runs slow work off the UI thread and hands the result back.

SecretSticky runs on RemedyUI (the `remedyui` branch of its repository, sharing the Tauri app's vault, crypto and error code instead of copying them): the manager with setup, the once-shown recovery key, password or recovery-key unlock with the vault's throttle, Argon2 off the UI thread, the note list, new notes in eight colors, delete with confirmation, auto-lock presets, password change and signed updates; frameless capture-protected note windows with a drag-region title bar, a corner resize grip, color, pin (always on top), copy with a 30-second clear, a save indicator, and edits and geometry saved 400 ms after they settle; a tray icon drawn in code with its menu; single instance; idle lock that saves and closes every note. An end-to-end test drives the whole app headless through the accessibility tree (every control has a name) on a temporary vault. The engine gained what it needed: `Window::on_geometry`, `Window::on_closed`, `El::drag_region`, `App::on_activity`, `Window::renderer`, and `Reaction` in the reactive crate, which makes custom painters repaint when the signals they read change. Footprint: windows now start on the CPU renderer and move to the GPU once their frames average over 6 ms (or one frame stalls 50 ms), tested on a real window; wgpu is built with only each platform's backend and vello without its text and PNG features; Thai, Lao, Khmer and Burmese dictionary line breaking is the opt-in `complex-scripts` feature (in `full`). Measured on Windows x64 (release, stripped, `app` profile, one-window sample): RemedyUI adds 6.0 MB to the binary (target 8), idle memory is 20 MB (target 45), and the window is up 24-32 ms after launch (386 ms on a cold first launch). SecretSticky itself: 12.2 MB, 26 MB idle. `remedy-ui-widgets-forms` is built: `number_input` over an exact `Decimal` (38 digits, half-away rounding, never a float) with units, precision, range errors and the locale's digits and separators (read from ICU4X data: en, de, fr, en-IN lakh grouping, Arabic-Indic digits all tested), accepting either decimal convention when unambiguous; `color_picker` (saturation/brightness area, hue and opacity strips, hex, WCAG contrast readout against a background, all keyboard operable); `key_capture`; `calendar`, `date_picker` and `time_picker` with the locale's month and weekday names, first weekday and 12/24-hour clock, keyboard navigation and min/max; and `Form` with validation as values change, dirty tracking against the last save, and submit state (`Submitting`, `Saved`, `Failed`). The a11y layer gained spin button, color well, date and time input, grid, grid cell and form roles and an invalid flag; the view gained `Ui::locale` (set by the runner from the system), `Ui::platform`, `El::on_focus_change` and `El::blur`; assistive increment and decrement now arrive as Up and Down. Times are written and read the locale's way (Arabic digits and day periods, Korean day period first). `remedy-ui-devtools` v1 is built: the inspector (pick an element in a window, or choose it in the element tree, to see its widget and variant, the source line that built it (every widget builder is `#[track_caller]`), its box, padding, resolved style, states, semantics and bindings; an accessibility check for unnamed controls, low-contrast text and targets under 24×24; theme colors edited live with the color picker) and previews (a piece of UI rendered by the CPU renderer in every combination of palettes, locales with right-to-left ones mirrored, window sizes and text scales; shown side by side in a gallery; checked against golden PNGs with a small tolerance, writing a baseline or the differing rendering for review). The facade's `devtools` feature opens the inspector with F12 in any window or `Window::open_inspector`, and `show_previews` opens the gallery. The view gained `El::info`, `El::parent`, `Ui::element_at`, `Ui::layers` and a pointer interceptor for tools. System font fallback is now on by default (`system-fonts`), so scripts the bundled fonts lack (Arabic, CJK, emoji) render; on Linux fontconfig is loaded at run time. The one-window sample stays at +6.06 MB and 22 MB idle with it. `cargo-remedy` is built: `new` writes a complete app from the `basic`, `tray` or `multi-window` template (library and thin `main`, a headless test, a preview that is also a golden-image test, `remedy.toml`, a default icon), and explains that `document` (M8) and `android` (M7) are not here yet; `run` builds with devtools in debug and with `--watch` rebuilds and restarts on every change to sources, assets or manifests; `package` builds a Windows release with the icon and version info compiled in (`llvm-rc` or the SDK's `rc`), a multi-size icon from one PNG, a per-user Inno Setup installer (Start menu entry, uninstaller), a portable zip, signtool signing when `remedy.toml` names a certificate, and minisign `.sig` files for the updater when `REMEDY_UPDATE_KEY` is set. Tested: all three templates generate apps that build and pass their own tests, and a packaged app installs, runs and uninstalls cleanly. The facade gained the `widgets`, `forms`, `update` and `headless` features (re-exported as `remedy_ui::widgets`, `forms`, `themes`, `update`, `headless`) and styles apps with the first Remedy palette by default.

Gaps found while finishing M5, each fixed at its root with tests: paragraph alignment never reached parley (it aligns while breaking lines), so center, end and right-to-left alignment did nothing; text alignment now follows the layout direction (a right-to-left window's start is its right edge) with `Paragraph::x_offset` placing unwrapped text in its box and `LineInfo::left` reporting each line's start. Text properties (color, size, weight, monospace) now inherit from the parent as in CSS, so text inside a primary button or a selected segment takes the button's readable color; the sheet's rule cache holds rule patches so inherited values never leak between elements. Dates come from the platform's local time zone (a new `Host::local_time`: Windows dynamic time zone data, `NSTimeZone`, `localtime_r`), the runner keeps each window's `Ui::local_time` current with a timer at local midnight, and the calendar marks today. The inspector shows the signals and memos each element reads (`Effect::sources`, `Reaction::sources`, with where each was created). The accessibility check also flags pointer-only controls (with `El::keyboard_alternative` for deliberate ones) and focusable elements with no size, sees the semantics screen readers get, and every core and form widget passes it (choice rows, chips and links are now at least 24 px tall). Previews and the view support the pseudo-locales `en-XA` (accented, about 40% longer, bracketed) and `ar-XB` (forced right to left), applied to the app's strings and never to typed text or text marked `El::data_text`. The inspector window is titled after the window it inspects. The sample stays at +6.1 MB and 22 MB idle.

**Crates:** `widgets-core`, `widgets-forms`, `themes`, `secure` (idle lock, sensitive clipboard, capture protection), `services-desktop` (tray, single instance, dialogs), `services-update`, `devtools` v1 (inspector, previews), and `cargo-remedy` (`new`, `run`, `package` for Windows).

**Done when:**
- SecretSticky runs on RemedyUI on Windows with all its features: many frameless always-on-top notes with persisted geometry, a manager hidden to the tray, the unlock screen, idle lock, clipboard clear and auto-update
- it is within the `app` footprint targets and passes the accessibility checks

## M6. Data apps: SecretFolder and Hearth

**Crates:** `widgets-nav`, `widgets-data`, `media-image`, `media-svg`, `services-i18n`, plus OS drag and drop in and out. Also run `bridge-macos` and `bridge-linux` on their machines (their hooks are built in M4 and cross-compiled here).

**Status:** in progress. Built:
- `services-i18n`: Fluent messages (embedded, folder, live reload), locale data and exact decimals; app templates are localized. Source-keyed catalogs (`Catalog`) load gettext-style JSON shared with other implementations, match locales by language and script, and translate English text that arrives from elsewhere, placeholders included. `App::set_locale` gives an app its own language over the system's.
- `widgets-nav`: commands and shortcuts, tabs, nav rail, bottom bar, breadcrumbs, a navigation stack, user-arranged toolbars, the command palette, drawers, sheets and a custom title bar.
- `widgets-data`: icon grids (virtual rows of tiles that wrap to the width, the table's selection, rubber-band selection, two-dimensional keys, drag and drop); tables (sorting off the UI thread past 20,000 rows, resizable and reorderable columns, frozen columns kept in step by compositor scroll links, virtual rows, multi-select, inline editing, row drag and drop); trees (lazy children, type-ahead, moves by drag or cut and paste, import and export with other apps); property grids. The 50,000-file explorer scene meets its budget in release builds (steady scroll under 2 ms a frame, p95 about 6 ms with flicks, against 8 ms).
- `media-image`: off-thread decoding at the size shown, ICC to sRGB, EXIF orientation, shared while shown and freed after; the renderers now free cached images the app drops.
- `media-svg`: resvg at exactly the size shown, as an image format, plus tinted icons; untrusted SVG reads no files.
- Drag and drop: in the app, out to other apps and in from them. On Windows this is our own OLE drop target and `SHDoDragDrop`; on macOS an AppKit dragging session out and winit's file drops in; on Linux winit's file drops in, with dragging out left for the Linux run.
- `Host::open_url` on every bridge.
- `services-update` trusts releases by a signed minisign key or by a published checksum file plus the Authenticode signer; `services-desktop` starts apps with the session, shows files in the file manager and finds the user's known folders (following redirection).
- SecretFolder runs on RemedyUI with its full feature set (the `remedyui` branch of its repository), tested end to end through the accessibility tree.

The engine also gained:
- Keyboard focus draws the theme's focus ring, and the audit flags focus that does not show.
- `hidden` takes elements out of layout and the tab order.
- Pointer capture, linked and programmatic scrolling, and `on_resize`.
- Posted work reaches real apps.
- Scroll layers sit where their containers are, and paint order holds around them.
- `App::completion`: results from any thread (an async runtime's tasks) reach the UI thread without belonging to a reactive scope, so work started inside an effect is never lost.
- `El::on_drag_hover` tells an element when a drag comes and goes, accepted or not.
- Explorer icons (folder, file, download, upload, send, arrows), accent cards, avatars whose circle the contrast audit sees, context menus that declare their keyboard route.

Hearth runs on RemedyUI (the `remedyui` branch of its repository): wire-compatible with Hearth 0.4, tested end to end against a second engine and the real Python app.

Next: Hearth's eleven languages, then the macOS and Linux runs.

**Done when:**
- SecretFolder and Hearth run on RemedyUI with their full feature sets
- the 50,000-file explorer scene meets its frame budget
- both apps also run on macOS and Linux with no app changes

## M7. FreeSpeak

**Contents:**
- chat-grade lists (sticky headers, selection across rows)
- `doc-rich`, `media-anim`, `media-video`, `media-isolated`, `viz` (voice ring meters), `services-spell`
- `bridge-android`
- multi-window with moving subtrees
- the FreeSpeak skin mapping
- spaces the user builds (`widgets-board`): FreeSpeak's Home is a house that holds the voice and chat nodes plus updates, information, links and reactions, not a server list. The engine side is a board that its owner edits in place: drag, drop, resize and arrange areas and widgets (each move also doable from the keyboard, each change undoable), a registry of built-in widgets whose settings are typed so the property grid edits them, layouts saved as versioned documents that peers receive and render, reactions as a reusable component, and custom widgets people write in a sandboxed script (Luau, which was made for running untrusted code: no file, network or process access, only the capabilities the board grants, with time and memory checks sized from the device) that build ordinary RemedyUI elements, so they are themed, accessible and fast like built-in ones
- the board's editor and viewer share one renderer, so what the host arranges is exactly what visitors see

**Done when:**
- all of FreeSpeak runs on RemedyUI on Windows, Linux, macOS and Android, meeting the budgets in [design/09](design/09-performance-and-resources.md)
- the share pop-out is a real window
- a host builds a Home from built-in and scripted widgets by dragging and from the keyboard, and a visitor on another platform sees the same Home; a hostile script cannot read files, reach the network, stall the UI thread or exhaust memory
- screen readers operate every screen
- eframe, egui and the `epaint`, `egui_glow` and `ttf-parser` forks are deleted from FreeSpeak

## M8. Breadth: documents, music, media and the web

**Crates:** `doc-pages`, `widgets-music`, `widgets-extra`, `media-playback`, `bridge-web`.

**Also:** progressive zoom in the compositor: old tiles shown scaled while tiles at the new scale raster within a per-frame budget (docs/design/04-rendering.md, Tiles).

**Done when:**
- RemedyPDF, GuitarRemedy, VoiceRemedy, Alex's Craft Calc, Quickcast and Portal run on RemedyUI on their target platforms, including Android and the web where they ship there today
- a 300-page PDF zooms from 1% to 6400% within budget
- the 60 Hz pitch graph runs with no UI-thread work per frame

## M9. Power tools and RemedyOS

**Contents:**
- `doc-code`, `widgets-term`, `embed-webview`, `embed-gpu`
- `bridge-remedyos`
- the agent automation protocol with consent
- `.rui` user overrides and the code-fluid customization editors
- devtools complete: signal graph, Perfetto traces, remote devtools

**Done when:**
- the RemedyAI desktop runs on RemedyUI
- a RemedyOS-native app (Files or Monitor) runs on RemedyOS through the RemedyOS bridge, with the desktop's theme and semantics wires
- TheOven's 3D preview embeds
- an agent operates a FreeSpeak settings screen through commands, under consent, with every action undoable

## M10. 1.0

**Contents:**
- API review across all the apps above, then a freeze
- editions and the deprecation policy in force
- documentation, guides, porting guides and recipes complete
- every reference scene's budget held in CI on every supported platform

`bridge-ios` begins after 1.0, when an app needs it.

**Done when:**
- the apps have run on the frozen API for one release cycle with no breaking change
- the 1.0 crates are published
