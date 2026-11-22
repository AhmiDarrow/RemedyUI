# What RemedyUI must provide

RemedyUI is measured against what modern UI engines provide. The benchmarks are Flutter, SwiftUI, Jetpack Compose, WinUI 3, Qt Quick and the web platform. It is not measured against what egui happens to do: egui's limits are the reason RemedyUI exists, and they are listed only as things never to repeat (§13).

RemedyUI is for every Remedy app, not one. A requirement marked **FS** is one FreeSpeak, the first large app, uses today or needs next. §14 lists what the other Remedy apps add, from a survey of all of them ([APPS.md](APPS.md)). Unmarked requirements are part of the bar any full engine has to clear.

The design that meets these is [DESIGN.md](DESIGN.md). The order of work is in [ROADMAP.md](ROADMAP.md).

## 1. Programming model and state

| Requirement | Notes |
|---|---|
| Declarative UI: describe what the screen is for a given state, and the engine works out the changes | As in SwiftUI, Compose, Flutter and React |
| Fine-grained reactivity: a state change updates only what depends on it | No full rebuild per frame, and no view-tree diff of the whole screen |
| Derived state (memos), side effects with clean-up, batching of updates | |
| Keyed lists, where an insert mounts one item and moves none | **FS** chat, member list |
| Updates from any thread, waking the UI without polling | **FS** network, voice, decoders, single-instance wake |
| Components with props, children, slots and context (values provided to a subtree) | Theme, locale and app services passed down without globals |
| Navigation: stacks, tabs and back handling, with state kept per screen | Android back button, Escape |
| Forms: validation, dirty tracking, submit and disabled states | **FS** settings, invites, backup |
| Undo and redo as an engine service, not only inside text fields | |
| Async work tied to a component's lifetime, cancelled when it unmounts | Image loads, searches |

## 2. Layout

| Requirement | Notes |
|---|---|
| Flexbox, grid and block/flow layout | Taffy implements the CSS specs |
| Intrinsic sizing (min-content, max-content), aspect ratio, min and max constraints | |
| Absolute and anchored positioning: overlays anchored to an element, flipping to stay on screen | Menus, tooltips, popovers |
| Virtualized lists and grids of any length, with variable item heights measured in the same frame | **FS** 100,000-message chat, GIF and emoji grids |
| Scroll anchoring: stick to the bottom, or keep the item in view still when content above changes | **FS** chat |
| Sticky headers and sections | Date separators in chat |
| Resizable, collapsible split panes, and docking panels that can be torn into windows | **FS** member list, room side panel, share pop-out |
| Responsive layout: breakpoints by window size, from phone width to ultra-wide | **FS** Android and desktop in one codebase |
| Pixel snapping, so 1 px lines and borders stay crisp at fractional scales | 125% and 150% displays |
| Right-to-left mirroring of the whole layout for RTL languages | |
| Safe areas and insets: notches, system bars, soft keyboard | Android |

## 3. Text and typography

| Requirement | Notes |
|---|---|
| Complex shaping for all scripts: Arabic joining, Indic, Thai, Hebrew, CJK | HarfBuzz-level shaping |
| Bidirectional text (UAX #9), line breaking (UAX #14), grapheme, word and sentence segmentation | |
| Font fallback chains per family and per language, from bundled fonts and optionally system fonts | **FS** Remedy Sans UI first, plus a language font |
| Variable fonts (weight, width and optical-size axes) and OpenType features per span (tabular figures, stylistic sets, small caps) | **FS** tabular clocks |
| Color fonts: COLRv0/v1, CBDT, sbix, SVG-in-OpenType for emoji | Today FreeSpeak's emoji are its own art; system emoji must also work |
| Hinting at the exact pixel size (TrueType bytecode, CFF, autohint), with grayscale, LCD RGB/BGR and the platform's gamma | **FS** sharp text and subpixel are a must |
| Rich paragraphs: spans of style, links, decorations, and inline objects (images, pills, custom widgets) that wrap with the text | **FS** messages, mentions, custom emoji |
| Text selection across elements and paragraphs, with copy as plain and rich text | **FS** copy a range of messages |
| Rich text editing: an editor with spans, not only a plain input | Formatting in the composer |
| Inputs: single line, multi-line, password, numeric, search; masks and filters | **FS** |
| IME for CJK, dead keys, emoji pickers of the OS, dictation; preedit drawn inline, candidates at the caret | |
| Spell checking hooks using the platform checker, with suggestions in the context menu | |
| Truncation with ellipsis, max lines (`El::max_lines`), text transforms (`El::text_transform`), balanced and justified text, hyphenation | |
| Find in page across virtualized content | |
| Text scaling for accessibility (OS text size) separate from UI scale | Android and Windows text-size settings |
| Markdown and rich documents: headings, lists, quotes, code blocks with syntax colors, tables | **FS** wiki pages, bot messages |

## 4. Graphics and compositing

| Requirement | Notes |
|---|---|
| Anti-aliased vector paths: fill and stroke, dashes, joins and caps, even-odd and non-zero rules | **FS** voice ring |
| Rounded rectangles with per-corner radii, borders, inner and outer shadows | |
| Linear, radial and conic gradients | |
| Clipping to rounded rects and paths, alpha masks | |
| Layer opacity, blend modes, transforms (2D affine; 3D perspective for transitions) | |
| Backdrop effects: blur and acrylic-style material, like Windows Mica and macOS vibrancy | |
| Filters: blur, drop shadow, color matrix (grayscale, tint) | Disabled states, muted video |
| SVG icons and vector illustrations at any scale | |
| Images with fit modes, corner radius, placeholders and progressive loading; nine-slice frames | **FS** avatars, skin assets |
| Animated images: GIF, animated WebP, APNG | **FS** GIF library, GIPHY |
| Vector animations (Lottie-class) | Stickers, onboarding |
| Video surfaces fed by the app as YUV or RGBA, with no CPU color conversion | **FS** screen share at 1440p and above |
| External GPU textures: show a game view, 3D preview or camera from the app's own GPU code | |
| Color management: sRGB by default, wide gamut (Display P3), HDR content | |
| Offscreen rendering of any element to an image | **FS** CI shot tour, share previews |
| Printing and export to PDF | |

## 5. Animation and motion

| Requirement | Notes |
|---|---|
| Implicit transitions: a style property eases to its new value | Hover, press, theme changes |
| Explicit animations: keyframes, springs (physics-based), with interruption and retargeting | |
| Enter and exit transitions for mounted and unmounted elements; list item insert, remove and move | **FS** new messages, toasts |
| Shared-element and page transitions | Opening a media viewer |
| Gesture-driven animation that follows the finger and settles with velocity | Swipe to reply, drawer |
| One frame clock for the whole window, synced to the display's refresh (60–240 Hz, VRR) | |
| Reduced motion honored everywhere, from the OS setting or the app | **FS** Reduce Motion |
| Animations stop costing anything when off screen or the window is hidden | **FS** |

## 6. Input and interaction

| Requirement | Notes |
|---|---|
| Pointer events with capture: mouse (all buttons), touch, pen (pressure, tilt, eraser) | **FS** Mouse 4/5 |
| Gesture recognizers: tap, double tap, long press, drag, pan, pinch, rotate, swipe, with conflict resolution between nested recognizers | |
| Scrolling with momentum, overscroll and edge glow or bounce per platform, precise touchpad deltas, scroll snapping | |
| Keyboard focus: tab order, focus groups with arrow keys, focus traps in modals, focus-visible rings, restore focus on close | **FS** keyboard mode |
| Shortcuts and commands: a command table with key chords, scoped to window, screen or element, rebindable by the user | **FS** user keybinds |
| Drag and drop inside the app and with other apps: files, text, images, custom data, with drop previews | **FS** file drops |
| Clipboard: text, rich text, images, files; paste events with type choice | **FS** |
| Context menus on any element, with platform-native look where the platform has one | |
| Hover and focus state on any element, so hover toolbars are plain children | **FS** chat row actions |

## 7. Windows and platform

| Requirement | Notes |
|---|---|
| Multiple windows, each on any monitor with its own DPI; moving between monitors re-renders crisply | **FS** share pop-out |
| Custom title bars and window chrome, keeping Windows 11 snap layouts, macOS traffic lights and Linux client-side decorations working | |
| Transparent and blurred window backgrounds where the OS offers them | |
| System theme: light or dark, accent color and high-contrast mode, followed live | |
| Windows, macOS, Linux (Wayland and X11) and Android from one codebase; iOS and the web as later targets | **FS** Android today |
| Tray icon, notifications, badges and jump lists or dock menus as optional platform services | **FS** tray and notifications, today in app code |
| Native file and folder dialogs on every desktop (portal on Linux) | **FS** only Windows today |
| Single instance and URL or protocol activation | **FS** |
| Keep running while hidden or minimized, without repainting | **FS** hide to tray |
| Power: no work when idle; respect battery saver; throttle in the background | |

## 8. Accessibility

| Requirement | Notes |
|---|---|
| Full screen-reader support on every platform: UI Automation, NSAccessibility, AT-SPI, Android | AccessKit covers all four |
| Every built-in widget has its role, name, value, state and actions | |
| Live regions for changing content | **FS** new messages, call status |
| Keyboard operability of every control | |
| High contrast and forced colors, OS text scaling, reduced motion, reduced transparency | |
| Contrast checking tools for themes | **FS** skins balance to 7:1 and 4.5:1 |

## 9. Theming and styling

| Requirement | Notes |
|---|---|
| Design tokens (colors, type scale, spacing, radii, elevation, motion) with derived tokens | **FS** `.fskin` |
| Light, dark and high-contrast variants; follow the system or not | |
| Per-component style variants and states (hover, pressed, focus, disabled, selected, error) | |
| Live theme changes with no restart, at the cost of only what changed | **FS** live skin reload, skin editor |
| User-loaded themes and assets treated as untrusted data | **FS** imported skins |
| Density settings (compact, comfortable) | **FS** compact chat |

## 10. Localization

| Requirement | Notes |
|---|---|
| Message lookup with plurals and gender (ICU MessageFormat class) | **FS** has `.flang` files with plural rules |
| Locale-aware number, date, time and list formatting | |
| Layout mirroring for RTL and per-language fonts | |
| Switching language at runtime | |

## 11. Tooling

| Requirement | Notes |
|---|---|
| Inspector: element tree, layout boxes, styles, semantics, on a hotkey | |
| Performance overlay: frame time, what was dirty, cache use | |
| Hot reload of themes and assets; fast rebuild of component code | |
| Headless test runner: scripted input, fake clock, tree and semantics assertions, screenshot diffs | **FS** shot tour |
| Golden-image tests across themes and scales | |
| Benchmarks that run in CI with budgets | |
| Clear errors: a failing component shows where it failed, not a crash | |

## 12. Quality bar

| Requirement | Target |
|---|---|
| Idle | Zero CPU wakeups when nothing changes |
| Latency | Input to photon within one frame |
| Frame cost | Proportional to what changed; ≤ 2 ms CPU for a full chat scroll frame on a 2020 mid-range laptop |
| Start | ≤ 150 ms to first frame on the GPU path |
| Memory | Every cache sizes itself from the device and shrinks under OS memory pressure; no fixed caps; a large chat stays proportional to what is on screen |
| Robustness | No panic reaches a frame; a lost GPU recovers; the app starts on any machine through a CPU renderer |
| Safety | No unsafe code outside an audited GPU and platform boundary; untrusted bytes parsed by fuzzed, memory-safe code off the UI thread, bounded by what can be shown rather than by fixed numbers |
| Supply chain | Every dependency passes FreeSpeak's `cargo deny`; no async runtime required; no bundled C libraries (the OS's own frameworks, such as system GStreamer, may be loaded) |
| API | Stable public API with semantic versioning from 1.0; dependencies hidden behind RemedyUI's own types so they can be swapped |
| Modularity | An app links only the packs it uses; unused packs add no code and no dependencies, and linked packs start only when first used |

## 13. egui's failures, never to repeat

These are what FreeSpeak works around today (`crates/freespeak-cli/src/gui/`, about 16,000 lines on egui 0.33.3). Each one is a design rule for RemedyUI.

| egui failure | What FreeSpeak does about it | Rule for RemedyUI |
|---|---|---|
| The whole UI is rebuilt in code every frame | Repaints every 40 ms to stay current | Retained tree, fine-grained updates, nothing runs when nothing changes |
| Sizes are known one frame late | "Their width is measured each frame and used on the next" | Layout always runs before paint in the same frame |
| No real virtualization | A hand-written list with a `line_heights` map cleared at 20,000 entries | Virtual lists with height caches and anchoring are built in |
| Stops when the window is hidden | Tray, hotkeys and voice bypass the UI through raw window handles; 2 s keep-alive repaints | The runtime is independent of window visibility |
| Soft text: no hinting, no subpixel | Forked `epaint` and `egui_glow`; Bold font disabled | Hinting and LCD text are core features |
| Text runs are separate widgets | Lines break between runs, not inside them | One rich paragraph with inline objects |
| No text selection in labels | A Copy chip on every message | Selection is a core text feature |
| No RTL or complex shaping | Documented as unsupported | Full shaping and bidi |
| No accessibility in the build | Nothing for screen readers | AccessKit on every widget |
| One OS window | Pop-outs are fake windows inside the main one | Real multi-window |
| Renderer needs OpenGL 2+, or crashes | `catch_unwind` around startup, and Mesa llvmpipe bundled | A backend ladder with a CPU renderer and device-loss recovery |
| GIFs as one texture per frame | 48-entry cache cleared wholesale | Shared frame atlases with budgets |
| Video re-uploaded as RGBA and polled at 8 ms | ~120 Hz repaint polling | YUV surfaces that wake the loop when a frame arrives |
| Global style plus a global `RwLock` palette | Custom widgets read globals | Tokens provided through context, as signals |

## 14. What the other Remedy apps add

From the survey of every Remedy app ([APPS.md](APPS.md)). Each line names the apps that need it.

| Requirement | Apps |
|---|---|
| Many small frameless windows, always on top per window, geometry persisted per window | SecretSticky |
| Secret input that never leaks (zeroized, hidden from accessibility, logs and IME), idle lock fed only by real input, clipboard auto-clear, capture protection | SecretFolder, SecretSticky, Hearth, FreeSpeak |
| Explorer: file tree, table with sort and resize, breadcrumbs, previews, drag in from and out to the OS | SecretFolder, Hearth, RemedyOS Files |
| Paged documents: deep zoom from 1%, smooth pan, rotation, two-page spreads, annotation that stays exact under rotation, page tiles cached and rendered off the UI thread | RemedyPDF |
| Live audio visuals at display rate fed from an audio thread: pitch graphs, tuner needles, meters, waveforms; automatic stop when hidden | VoiceRemedy, GuitarRemedy, FreeSpeak voice ring |
| Music notation widgets: fretboard, tab staff, note staff, chord diagrams | GuitarRemedy, VoiceRemedy |
| Media transport with buffered ranges, subtitles and next-up, over platform decoders | Portal |
| Terminal emulator element | RemedyAI desktop, RemedyOS |
| Code editor with syntax colors and diffs | RemedyAI desktop, RemedyOS Editor |
| Embedded web view with an explicit message channel | RemedyAI desktop |
| The app's own 3D rendering embedded as an element | Game views, 3D previews (no Remedy app yet) |
| Exact decimal input and live recalculation for safety-critical numbers | Alex's Craft Calc |
| QR codes for pairing | Portal, RemedyAI |
| Signed auto-update from GitHub releases | GuitarRemedy, SecretFolder, SecretSticky, RemedyPDF, RemedyAI, Hearth |
| One theme set across all apps, including the colour-blind palettes | All, from RemedyAI's `themes.ts`, mirrored in Portal and Hearth and copied into FreeSpeak's skins |
| Android and tablets from the same code, with large touch targets | GuitarRemedy, VoiceRemedy, Craft Calc, RemedyPDF, RemedyAI |
| A web target for apps that also live in a browser (PWAs, phone UIs) | GuitarRemedy, Craft Calc, Portal |
| A native bridge for RemedyOS: its surfaces, input, theme wire, services and a semantics wire | RemedyOS apps |
