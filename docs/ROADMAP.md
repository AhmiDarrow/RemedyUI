# RemedyUI roadmap

Each milestone ends with something real running and tested, and from M4 on, with a Remedy app moved onto the engine. Apps go smallest first, so each one proves a new part of the engine before the large ones depend on it ([APPS.md](APPS.md)).

Nothing is merged as a placeholder. A crate enters the workspace with its first real code and tests. Every milestone is judged by repeat runs of its scenario suites, its benchmarks against their targets, and its fuzzers.

## M1. Foundations

- `core`, `reactive`, `tree`, and the first cut of `test` (the headless runner with a fake clock and scripted input)
- Glitch-free signals, memos, effects, stores, `SignalVec`, the `Waker`, scopes and disposal
- Element arena, dirty flags, event dispatch, focus, error boundaries
- Workspace rules in CI: layer graph check, lints, `cargo deny` / `audit` / `vet`, formatting, docs

**Done when:**
- Dirty counters prove that only dependents of a changed signal update.
- An idle runtime has zero wakeups.
- Posts from other threads are batched into one frame.
- A panicking handler is contained by its boundary.
- The CI rules reject an upward dependency.

## M2. Text

- `text`: font registry and fallback, itemize, shape, break, position; hinting and LCD rasterization; atlases; rich paragraphs with inline objects; selection; the editing core with undo; `secret_input` storage rules
- Windows TSF as the first real IME (headless IME scripts for the rest)

**Done when:**
- Mixed Latin, Arabic, Hebrew and CJK paragraphs lay out to match reference shapes.
- Hinted Remedy Sans UI glyphs match FreeType within tolerance.
- Selection spans paragraphs.
- Zalgo and bidi-spoofing cases stay within frame time and show visibly.
- A secret buffer is verified zeroed after drop.

## M3. Rendering and the compositor

- `paint`, `compositor`, `render-gpu`, `render-cpu`; `bridge-windows` and `bridge-headless`
- Paint chunks, layerization, tiles, commit, compositor-side scrolling and animation, damage-only presenting
- Pipeline cache, backend self-test and ladder, device-loss recovery

**Done when:**
- Golden images match between the GPU and CPU renderers.
- A forced device loss recovers with no app code.
- Scrolling stays at the display's rate while the UI thread is artificially stalled.
- An unchanged frame presents nothing.
- No shader compiles after start-up in a recorded session.

## M4. A first real app: SecretSticky

- `layout` (with virtualization), `style`, `input` (gestures, focus, commands), `motion` (frame clock, transitions, springs)
- `widgets-core`, `widgets-forms`
- `remedy-ui-themes` (the shared Remedy theme set), `remedy-ui-fonts`
- `services-desktop` (tray, single instance, dialogs)
- The `secure` module: idle lock, sensitive clipboard, capture protection
- `cargo remedy new/run`, previews, inspector v1

**Done when:**
- SecretSticky runs on RemedyUI on Windows with all its features: many frameless always-on-top notes with persisted geometry, a tray-hidden manager, the unlock screen, idle lock and clipboard clear.
- It is at or under the `app` footprint targets and passes the accessibility checks.

## M5. Data apps: SecretFolder and Hearth

- `widgets-nav`, `widgets-data`, `media-image`, `media-svg`
- OS drag and drop in and out, enter/exit and list transitions
- `bridge-macos`, `bridge-wayland`, `bridge-x11`

**Done when:**
- SecretFolder and Hearth run on RemedyUI with their full feature sets.
- The 50,000-file explorer scene meets its frame budget.
- Both apps also build and run on macOS and Linux with no app changes.

## M6. FreeSpeak

- Chat-grade virtual lists with anchoring and sticky headers
- `doc-rich` (Markdown, `RichEditor`, find)
- `media-anim`, `media-video`, `media-isolated`
- `services-spell`, `services-i18n`
- `bridge-android`
- Multi-window with moving subtrees
- Accessibility complete on all bridges so far
- The FreeSpeak skin mapping

**Done when:**
- All of FreeSpeak runs on RemedyUI on Windows, Linux, macOS and Android, meeting the budgets in [design/09](design/09-performance-and-resources.md).
- The share pop-out is a real window.
- Screen readers operate every screen.
- eframe, egui and the `epaint`, `egui_glow` and `ttf-parser` forks are deleted from FreeSpeak.

## M7. Breadth: documents, music, media and the web

- `doc-pages`, `viz`, `widgets-music`, `widgets-extra`, `media-playback`, `services-update`
- `bridge-web`

**Done when:**
- RemedyPDF, GuitarRemedy, VoiceRemedy, Alex's Craft Calc, Quickcast and Portal run on RemedyUI on their target platforms, including Android and the web where they ship there today.
- A 300-page PDF zooms from 1% to 6400% within budget.
- The 60 Hz pitch graph runs with no UI-thread work per frame.

## M8. Power tools and RemedyOS

- `doc-code`, `widgets-term`, `embed-webview`, `embed-gpu`
- `bridge-remedyos`
- The agent automation protocol with consent
- `.rui` user overrides and the code-fluid customization editors
- Devtools complete: signal graph, Perfetto traces, remote devtools

**Done when:**
- The RemedyAI desktop runs on RemedyUI.
- A RemedyOS-native app (Files or Monitor) runs on RemedyOS through the RemedyOS bridge, with the desktop's theme and semantics wires.
- TheOven's 3D preview embeds.
- An agent operates a FreeSpeak settings screen through commands, under consent, with every action undoable.

## M9. 1.0

- API review across all the apps above, then freeze
- Editions and the deprecation policy in force
- Documentation, guides, porting guides and recipes complete
- Every reference scene's budget held in CI on every supported platform
- `bridge-ios` begins after 1.0, when an app needs it

**Done when:**
- The apps have run on the frozen API for one release cycle with no breaking change.
- The 1.0 crates are published.
