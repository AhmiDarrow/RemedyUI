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

**Status:** in progress. `remedy-ui-layout` is built: flex, grid and block through taffy behind RemedyUI's own styles; a viewport root, so an auto-width root fills the window like a page body; inherited direction with logical insets mirrored for right to left; measured leaves; absolute and pixel-snapped rects; and the virtual list (estimates, Fenwick offsets, anchoring through inserts, removals and re-measures, stick-to-end, jump to any row).
`remedy-ui-motion` is built: cubic-Bézier easing with the standard curves, tweens, closed-form springs (stiffness and damping, or duration and bounce), retargeting that keeps position and velocity, animatable geometry and Oklab colors, the frame clock with presentation-time prediction and deadlines, and reduced motion. Still to come in M3: `style`, `input` and `a11y`.

**Crates:** `layout` (taffy behind RemedyUI types, pixel snapping, virtualization with anchoring), `style` (tokens, derived tokens, states, contrast helpers), `input` (event model, hit testing, gesture arena, scroll physics, focus, commands), `a11y` (semantics tree and incremental AccessKit updates), and `motion` (frame clock, transitions, springs). All of it is tested headless.

**Done when:**
- layout snapshots match references
- a virtual list keeps its anchor through inserts above it
- the gesture arena resolves nested pans
- the semantics tree passes AccessKit's schema checks for test scenes

## M4. Rendering, the compositor and Windows

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

**Crates:** `widgets-core`, `widgets-forms`, `themes`, `secure` (idle lock, sensitive clipboard, capture protection), `services-desktop` (tray, single instance, dialogs), `services-update`, `devtools` v1 (inspector, previews), and `cargo-remedy` (`new`, `run`, `package` for Windows).

**Done when:**
- SecretSticky runs on RemedyUI on Windows with all its features: many frameless always-on-top notes with persisted geometry, a manager hidden to the tray, the unlock screen, idle lock, clipboard clear and auto-update
- it is within the `app` footprint targets and passes the accessibility checks

## M6. Data apps: SecretFolder and Hearth

**Crates:** `widgets-nav`, `widgets-data`, `media-image`, `media-svg`, `services-i18n`, plus OS drag and drop in and out. Also `bridge-macos`, `bridge-wayland` and `bridge-x11`.

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

**Done when:**
- all of FreeSpeak runs on RemedyUI on Windows, Linux, macOS and Android, meeting the budgets in [design/09](design/09-performance-and-resources.md)
- the share pop-out is a real window
- screen readers operate every screen
- eframe, egui and the `epaint`, `egui_glow` and `ttf-parser` forks are deleted from FreeSpeak

## M8. Breadth: documents, music, media and the web

**Crates:** `doc-pages`, `widgets-music`, `widgets-extra`, `media-playback`, `bridge-web`.

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
