# RemedyUI roadmap

Each milestone ends with something that runs and is tested. Nothing is merged as a placeholder: a crate enters the workspace when its first real code does. The bar for each milestone is checked by the headless test runner and the benchmarks in CI.

## M1. Core, reactivity and the tree

- `remedy-ui-core`, `remedy-ui-reactive`, `remedy-ui-tree` and the first cut of `remedy-ui-test`
- Signals, memos, effects, batching, the `Waker`; the element arena; event dispatch; focus; hit testing
- Headless runner with a fake clock and scripted input

**Done when:** a component tree with keyed lists updates exactly the elements whose signals changed (checked by dirty counters in tests); no wakeups happen while idle; posting from another thread wakes the loop; a panicking handler is isolated and the frame goes on.

## M2. Text

- `remedy-ui-text`: font collection and fallback, shaping, bidi, line breaking, hinting at pixel size, the grayscale and LCD glyph atlas, rich paragraphs with inline objects, selection, `TextInput` with IME, undo, password handling

**Done when:** Remedy Sans UI, AhmiPrint and an Arabic language font lay out mixed-direction paragraphs correctly against reference shapes; hinted glyphs match FreeType within tolerance; selection spans several paragraphs; IME preedit works in the headless runner's IME script.

## M3. Rendering

- `remedy-ui-paint` and `remedy-ui-render`: the display list, the wgpu UI renderer (SDF shapes, analytic shadows, atlases, gradients, clips, layers, `vello_gpu` paths) and the `vello_cpu` CPU renderer with RemedyUI's compositor
- Damage tracking, backend ladder, device-loss recovery

**Done when:** every golden image matches between the GPU and CPU renderers within tolerance; a forced device loss recovers with no app involvement; a machine with no usable GPU starts on the CPU renderer; an unchanged frame draws nothing.

## M4. Layout, theming and the first widgets

- `remedy-ui-layout` (taffy behind RemedyUI's style types, pixel snapping), `remedy-ui-style` (tokens, derived tokens, states, contrast helpers), `remedy-ui-input` (gestures, scroll physics, commands), `remedy-ui-platform` (one window on Windows, Linux and macOS)
- Widgets: layout containers, text, buttons, switch, check, radio, segmented, slider, select, text input, tooltip, menu, context menu, modal, progress, spinner, scroll

**Done when:** FreeSpeak's settings screens are rebuilt on RemedyUI from a `.fskin` mapped to a theme, look the same as today in every built-in skin, pass the accessibility checks, and live-reload a skin in one frame.

## M5. Media

- `remedy-ui-media`: decode pool with limits, image cache with budgets, animated images on shared atlases, SVG icons, video surfaces (NV12, I420, RGBA), external textures
- Fuzz targets for every decoder path

**Done when:** the GIF library with 200 GIFs scrolls within the frame budget and stays inside its memory budget; a 1440p60 share runs with one upload per frame and no CPU color conversion; fuzzing finds no crash in 24 hours per target.

## M6. Chat-grade lists, motion and rich content

- Virtual list and grid with height caches and anchoring, sticky headers
- `remedy-ui-motion`: frame clock, transitions, springs, enter/exit/move, gesture-driven motion, Reduced Motion
- Markdown and rich documents; `RichEditor`; spell checking; find in page

**Done when:** FreeSpeak's chat view (10,000-message reference room, hover toolbars, GIFs, spoilers, links, emoji completion, composer) runs on RemedyUI within the budgets in DESIGN.md §18, and the 18 wiki pages render as rich documents.

## M7. Whole platform

- Multi-window with moving subtrees, custom window chrome, system theme following, platform services (tray, notifications, dialogs, single instance, URL activation, power state), Android shell
- `remedy-ui-a11y` complete on all four platforms
- `remedy-ui-i18n`: messages with plurals, locale formatting, RTL mirroring

**Done when:** all of FreeSpeak runs on RemedyUI on Windows, Linux, macOS and Android; the share pop-out is a real window; screen readers on each platform can operate every screen; eframe, egui and the `epaint`, `egui_glow` and `ttf-parser` forks are deleted from FreeSpeak.

## M8. Tooling and 1.0

- `remedy-ui-devtools`: inspector, performance overlay, hot reload
- API review and freeze, documentation and examples for every widget, semantic versioning from 1.0

**Done when:** the public API has been through one full app (FreeSpeak) and one new app (a RemedyOS settings panel) without breaking changes, and the 1.0 crates are published.
