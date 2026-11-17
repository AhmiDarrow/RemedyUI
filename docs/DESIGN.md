# RemedyUI design

RemedyUI is a UI engine in Rust. It is built to the standard of modern UI engines (Flutter, SwiftUI, Jetpack Compose, WinUI 3, Qt Quick, the web platform), not to the limits of egui. FreeSpeak is the first app: chat, voice and screen share on Windows, Linux, macOS and Android. RemedyOS and other apps are meant to use it too. [REQUIREMENTS.md](REQUIREMENTS.md) lists the bar it must clear, section by section, and the egui failures it must never repeat.

Five promises shape every decision below, in this order:

1. **Stable.** A frame never panics and never stalls. A lost GPU recovers without a restart, and the app starts on any machine.
2. **Secure.** No unsafe code outside one small, audited boundary. Everything that comes from other people (images, GIFs, fonts, skins, text) is parsed by memory-safe, fuzzed code with limits, off the UI thread.
3. **Fast.** The work done is proportional to what changed. The engine does nothing when nothing changes, and a key press reaches the screen within one frame.
4. **Efficient.** The idle cost is zero: no polling, no 40 ms repaint loop. Memory has budgets, and the GPU keeps what did not change.
5. **Full featured.** Every widget, text feature and platform hook FreeSpeak needs is built in, so the app carries no forks or workarounds.

## 1. Shape of the engine

```
 app code ── builds ──▶ element tree ◀── reactive signals (state)
                           │  dirty flags: style · layout · paint · semantics
                           ▼
 style ─▶ layout ─▶ text shaping ─▶ paint (display list) ─▶ renderer backend ─▶ window
                           │                               (GPU / GL / CPU)
                           └────────────▶ accessibility tree (AccessKit)
```

RemedyUI is **retained**. The app builds a tree of elements once, and **fine-grained signals** update only the parts that depend on changed state. This is the opposite of egui, which rebuilds the whole UI in code every frame. That rebuild is what forces egui's one-frame-late sizes, its hand-made virtualization and its constant repaints.

- A signal change marks only the elements that read it as dirty, and only in the dimension it affects: style, layout, paint or semantics.
- Layout runs only on dirty subtrees. Paint re-records only dirty display-list chunks. The renderer redraws only the damaged rectangles.
- Nothing runs between events. The event loop sleeps until there is input, a signal set from another thread, a timer or animation deadline, or a video frame.

An immediate-mode escape hatch remains. `Canvas` gives a closure a painter and hit regions, for custom drawing like the voice ring. It re-runs only when the signals it reads change.

## 2. Crates

One Cargo workspace. Each crate has one job and a small public API.

| Crate | Job | Unsafe |
|---|---|---|
| `remedy-ui-core` | Geometry (`Point`, `Size`, `Rect`, `Insets`, rounded rects), colors (linear, premultiplied), element ids (generational arena), time, events | forbidden |
| `remedy-ui-reactive` | `Signal`, `Memo`, `Effect`, batching, a per-window scheduler, and the thread-safe `Waker` | forbidden |
| `remedy-ui-tree` | The element tree, the `Element` trait, dirty flags, lifecycle (mount, unmount), focus, hit testing, event dispatch (capture, target, bubble) | forbidden |
| `remedy-ui-style` | Theme tokens, derived tokens, element states (hover, pressed, focused, focus-visible, disabled, selected), transitions, contrast helpers | forbidden |
| `remedy-ui-layout` | Flex, grid and block layout via taffy; the virtual list and virtual grid; pixel snapping | forbidden |
| `remedy-ui-text` | Fonts, fallback, shaping, bidi, line breaking, hinting, the glyph atlas (grayscale and LCD), rich paragraphs, editing, IME, selection | forbidden |
| `remedy-ui-paint` | The display list: rects, rounded rects, borders, shadows, gradients, paths, glyph runs, images, video surfaces, clips, layers, opacity | forbidden |
| `remedy-ui-render` | The renderer trait, the GPU UI renderer and the CPU renderer (§6) | GPU boundary only |
| `remedy-ui-media` | Image decode pool, animated images, SVG icons, vector animations, video surfaces (YUV), external textures, texture budgets | forbidden |
| `remedy-ui-motion` | The frame clock, transitions, keyframes, springs, gesture-driven animation, enter/exit and list transitions | forbidden |
| `remedy-ui-input` | Pointer, touch and pen events, gesture recognizers and their arena, scroll physics, focus, commands and shortcuts, drag and drop | forbidden |
| `remedy-ui-i18n` | Message lookup with plurals and gender, locale formatting of numbers and dates, RTL direction, through ICU4X | forbidden |
| `remedy-ui-devtools` | Inspector, performance overlay, theme hot reload, cache meters | forbidden |
| `remedy-ui-platform` | The shell over winit: windows, custom chrome, system theme, IME, clipboard, drag and drop, file dialogs, DPI, power state, optional tray and notifications, Android | platform boundary only |
| `remedy-ui-a11y` | Builds the AccessKit tree from element semantics | forbidden |
| `remedy-ui-widgets` | The widget set (§8) | forbidden |
| `remedy-ui-test` | Headless runner, scripted input, screenshots, golden images, layout and paint snapshots | forbidden |
| `remedy-ui` | The one crate an app depends on; re-exports the rest | forbidden |

Every crate except the two boundary crates starts with `#![forbid(unsafe_code)]`. The two boundary crates keep their unsafe code in one module each, behind safe types, with a comment for each block saying why it is sound.

## 3. Programming model

Components are plain Rust functions that return elements. State lives in signals. There is no macro language to learn; a `view!` macro may come later as sugar over the same builders.

```rust
use remedy_ui::prelude::*;

fn composer(room: RoomId, send: impl Fn(String) + 'static) -> impl Element {
    let text = Signal::new(String::new());
    let can_send = Memo::new(move || !text.get().trim().is_empty());

    row()
        .gap(Space::S2)
        .child(
            text_input(text)
                .placeholder(t("Message"))
                .on_key(Key::Enter, move |input| {
                    if !input.shift() && can_send.get() {
                        send(text.take());
                        return Handled::Yes;
                    }
                    Handled::No // Shift+Enter falls through and inserts a newline
                })
                .grow(1.0),
        )
        .child(
            button(t("Send"))
                .variant(Variant::Primary)
                .enabled(can_send)
                .on_click(move || send(text.take())),
        )
}
```

- **Reading a signal inside a builder argument** (`enabled(can_send)`) binds that property. When the signal changes, only that property updates. The function does not run again.
- **Lists** use `for_each(items, key, |item| ...)`. Items are diffed by key: inserting a message mounts one row and moves none.
- **Conditional parts** use `show(cond, || ...)` and `switch(value, ...)`, which mount and unmount subtrees.
- **Other threads** (network, voice, decoders) never touch the tree. They hold a `Waker` and post a closure: `waker.post(move |cx| messages.update(|m| m.push(msg)))`. This replaces FreeSpeak's stored egui context and its `request_repaint` calls.
- **Effects** run after the frame's state changes settle and before layout. They are for syncing state, not for drawing.

Signals are owned by the window's runtime (an arena), not by thread-locals. A `Signal` is a `Copy` handle. Reading a disposed signal returns its last value and logs once; it never panics.

## 4. The frame

A frame happens only when something asks for one. In order:

1. **Input and posts.** Platform events and posted closures run. They set signals.
2. **Effects.** Effects whose signals changed run, in dependency order, once each.
3. **Style.** Dirty elements resolve their style from theme tokens and state. Transitions start here.
4. **Layout.** Dirty subtrees are laid out. Text is shaped only when its content, font, size or width changed. Virtual lists lay out only the rows near the view.
5. **Paint.** Dirty elements re-record their chunk of the display list. Clean subtrees keep theirs. Static subtrees can be cached as GPU layers.
6. **Semantics.** Dirty elements update their AccessKit nodes, and a tree update goes to the platform.
7. **Render.** The renderer draws the damaged region only, then presents. A video frame that arrives on its own damages only its surface.

Animations (transitions, spinners, GIFs) register a deadline with the frame clock while they are visible. A hidden, minimized or Reduce Motion window registers none, and an off-screen GIF does not tick.

When a window is hidden to the tray, steps 1 and 2 keep running when events come in, but nothing is painted or presented. Voice and tray code never need to bypass the UI to stay responsive.

## 5. Layout

- **Flex, grid and block** come from taffy, behind RemedyUI's own style types. This lets the engine change or extend the layout code without breaking apps.
- **Pixel snapping.** Every edge is snapped to whole device pixels after layout, so 1 px lines and 28 px controls stay crisp at 125% and 150%.
- **Spacing scale.** Gaps come from theme tokens (`Space::S1` = 4 px … `S8` = 48 px). FreeSpeak's 4-pixel grid becomes the default scale and is not enforced by hand.
- **Virtual list.** A list of any length keeps a height cache keyed by item id. On a width change it re-measures only the rows in view and estimates the rest, correcting as they come into view. Anchoring keeps the view still when items above change size: to the bottom (stick-to-bottom chat), or to the first visible item. This replaces `show_viewport`, the `line_heights` map and its 20,000-entry clear.
- **Virtual grid** for the GIF picker and emoji picker: fixed cells, laid out only where visible.
- **Layers.** Overlays such as menus, tooltips, popovers, modals and toasts live in a per-window layer stack. They are anchored to an element or a point, flip to stay on screen, and trap focus where they should.

## 6. Rendering

The display list is backend-neutral and holds the following:

- rects, per-corner rounded rects, borders and shadows
- paths: fill and stroke, with dashes, joins and caps
- linear, radial and conic gradients
- glyph runs, images, nine-slices, video surfaces and external textures
- clips (rect, rounded rect, path), alpha masks, layers with opacity, blend modes and 2D/3D transforms
- filters (blur, drop shadow, color matrix) and backdrop filters (blur, material)

Two renderers draw it, and both are owned by RemedyUI.

**1. GPU renderer** on wgpu: Direct3D 12, Vulkan and Metal, and OpenGL 3.3 / GLES 3.0 through wgpu's GL backend. It is a 2D UI renderer, tuned for what interfaces draw most:

- Rects, rounded rects, borders and shadows are signed-distance quads in one instanced pass. Shadows are analytic Gaussian, so there is no blur pass.
- Glyphs and small images come from atlases. Text is hinted and rasterized on the CPU at the exact pixel size (§7), then drawn as quads.
- Paths that are not plain rects go through Linebender's `vello_gpu` sparse-strip pipeline. It needs no compute shaders, so it runs on the GL fallback too. It sits behind RemedyUI's own path type, so the engine can swap it for its own tessellator if that churns.
- Backdrop blur uses a downsample and upsample chain, limited to the damaged region.
- Layers render to cached textures. A static subtree, such as a settings page under an open menu, is drawn once and reused.
- Video planes and external textures are sampled directly. YUV is converted in the shader.
- Colors are blended in linear light, and the output follows the surface format: sRGB by default, extended sRGB / P3 / HDR10 where the swapchain offers it.

**2. CPU renderer** on `vello_cpu`, which is multithreaded and SIMD, for machines where no GPU backend starts: no driver, GL below 3.3, WARP or llvmpipe trouble, broken drivers. RemedyUI's own compositor draws glyph and image atlases and LCD text over it with the same blending rules. It presents through a plain pixel blit (softbuffer class). It is also the reference renderer for golden-image tests, so CI needs no GPU. This replaces FreeSpeak's bundled Mesa llvmpipe.

**Choosing and recovering.** At start the engine tries GPU backends in a fixed order with a self-test frame, then the CPU renderer. It records what worked, and the app can override the choice (FreeSpeak's Graphics page). On device loss, the GPU renderer rebuilds every resource from the display list and the atlases' CPU copies, then redraws. The app never sees this. A GPU that keeps failing is demoted to the CPU renderer for the session.

**LCD text.** Where the backend has dual-source blending, LCD text is blended per channel. Over images, video, transparent layers and transforms it falls back to grayscale, as browsers do. The CPU compositor always has per-channel blending.

**Damage and presenting.** Each element's painted bounds are tracked. A frame redraws only the union of what changed. The swapchain presents with damage rectangles where the platform supports it (Wayland, Android, DXGI present1). Frames are paced to the display's refresh: FIFO by default, mailbox when the app asks for the lowest latency, and variable refresh where available. A window with nothing to draw presents nothing.

**Embedding.** An element can host the app's own GPU work: a game view, a 3D preview or a camera. The app renders into a texture RemedyUI shares, and the element composites it like an image.

## 7. Text

- **Fonts.** A `FontCollection` is built from bytes the app gives it: the bundled faces, the user's `.ttf`/`.otf` files and a language font. No system font scan happens unless the app asks. Each family has a fallback chain, so Remedy Sans UI → AhmiPrint → language font is one line.
- **Shaping and layout.** Text is shaped with a HarfBuzz-compatible shaper (OpenType features per span, such as `tnum` and `ss01`). It has full Unicode bidi, line breaking by UAX #14, and grapheme and word segmentation. Arabic joins and Hebrew runs right to left, which closes the gap FreeSpeak's `docs/languages.md` documents.
- **Rich paragraphs.** One paragraph holds spans: weight, style, color, size, strike, underline, code background, link, spoiler. It also holds **inline objects**: emoji images, mention pills and custom-emoji art, which wrap with the text like characters. A FreeSpeak message is one paragraph, not a row of widgets.
- **Variable and color fonts.** Weight, width and optical-size axes are set per span; optical size follows the pixel size automatically. Color glyphs (COLRv0/v1, CBDT, sbix) draw in full color, so system and bundled emoji fonts work beside FreeSpeak's own emoji art.
- **Hinting.** TrueType bytecode where the font has it, the autohinter where it does not, at the exact pixel size. Line metrics are rounded to whole pixels when hinting is on. This is FreeSpeak's "Sharp text" setting, built in.
- **Selection.** Any `Text` can be selectable: drag, double-click a word, triple-click a paragraph, keyboard selection, copy. Selection can span several elements in reading order, so a range of chat messages can be copied.
- **Rich text editing.** `RichEditor` edits attributed text (bold, italic, code, links, inline objects) with the same caret, selection and IME engine, and serializes to the app's own format (Markdown for FreeSpeak).
- **Spell checking** uses the platform checker (Windows Spell Checking API, NSSpellChecker, Hunspell or Enchant on Linux, Android's service) behind a trait. Misspellings get an underline, and suggestions appear in the context menu.
- **Find in page** searches the text model, including rows of a virtual list that are not mounted, and scrolls to the matches.
- **Text scale** from the OS or app is applied on top of UI scale, so text can grow without everything growing.
- **Editing.** `TextInput` handles single line, multi-line and password. It has undo and redo, word motions, key filters, a caret and selection API, and completion popups anchored to the caret. IME preedit is drawn inline, and the candidate window is placed at the caret. Password buffers are zeroed on drop and are never exposed to accessibility.
- **Caches.** Shaped runs are cached by (text, font, size, features); glyph rasters by (glyph, size, subpixel offset, mode). Both have byte budgets with least-recently-used eviction.

## 8. Widgets

Every widget is styled only through theme tokens and states, and exposes accessibility semantics.

- **Layout:** `row`, `column`, `stack`, `grid`, `scroll`, `virtual_list`, `virtual_grid`, `split` (resizable panels), `spacer`, `divider`
- **Text:** `text` (wrap, truncate, max lines, selectable), `rich_text`, `markdown`, `link`, `code`
- **Input:** `button` (primary, quiet, danger, toggle, alarm; icon, icon + text, frameless), `icon_button`, `switch`, `check`, `radio`, `segmented`, `slider` (horizontal, vertical, stepped, `on_commit`), `select`, `text_input`, `search`, `key_capture` (for keybinds), `color_picker` (skin editor), `chip`
- **Feedback:** `progress` (determinate, indeterminate), `spinner`, `lamp` (status dot), `badge`, `toast`, `empty_state`
- **Overlays:** `tooltip`, `menu`, `menu_button`, `context_menu`, `popover`, `modal`, `dialog`
- **Media:** `image` (fit, corner radius, placeholder), `avatar`, `animated_image`, `video_surface`, `icon` (vector)
- **Custom:** `canvas` (paths, hit regions, gestures), `paint_layer` (raw display-list access)
- **Navigation:** `tabs`, `nav_rail` / `nav_item`, `nav_stack` (push, pop, back handling, per-screen state), `breadcrumbs`, `toolbar`, `command_palette`
- **Data:** `table` (sortable, resizable and reorderable columns, virtualized rows, selection), `tree_view` (lazy children, keyboard expand), `list` with selection modes
- **More input:** `number_input`, `date_picker`, `time_picker`, `file_drop_zone`, `rating`, `rich_editor`
- **Surfaces:** `card`, `sheet` and `drawer` (gesture-driven), `bottom_bar`, `dock` (panels that can be torn into windows), `title_bar` (custom window chrome)

Gestures are shared: click, double-click, long press, drag (with threshold), hover, wheel and precise scroll, pinch, drag and drop (internal and file drops).

## 9. Media

- **Decoding** happens on a worker pool owned by the engine. The pool size defaults to two workers and the app can change it. Each decode has limits on byte size, pixel count, frame count and time. A result that breaks a limit becomes an error placeholder, never a crash. Decoders are pure Rust.
- **Images** are uploaded in the frame that needs them and cached by content hash within a texture budget. They are evicted when least recently used, and the CPU copy is dropped once the image is uploaded, unless the CPU renderer needs it.
- **Animated images** (GIF, animated WebP, APNG) decode their frames into a shared atlas. All views of the same GIF share one decode, and each view keeps its own clock. Frames only advance while the image is visible and motion is allowed.
- **Video surfaces** take frames as NV12, I420 or RGBA from any thread through a triple buffer. YUV planes are uploaded as they are and converted in the shader (BT.601/709, limited or full range). A new frame wakes the event loop and damages only the surface. There is no polling, and a 1440p60 share costs one upload per frame.

## 10. Motion

- **One frame clock per window**, driven by the display's vertical blank (DXGI frame statistics, CVDisplayLink, Wayland frame callbacks, Android Choreographer). Every animation reads its time from this clock, never from wall time, so animations stay in step and tests can use a fake clock.
- **Implicit transitions.** Any animatable style property (color, opacity, size, offset, radius, shadow, transform) can declare a transition in the theme. Changing the value eases to it. Hover, press, focus and theme changes animate without app code.
- **Explicit animations**: keyframe tracks, tweens with easing curves, and springs (stiffness, damping, mass). Every animation can be interrupted and retargeted mid-flight without a jump, because springs carry their velocity over.
- **Enter, exit and move.** `show` and `for_each` take transitions for mounting, unmounting and reordering. An exiting element stays in the tree, inert, until its exit finishes. List moves animate from old to new positions (FLIP).
- **Shared-element transitions** between screens: an element marked with the same id on both screens morphs its bounds, radius and opacity.
- **Gesture-driven motion.** A drag can drive an animation value directly. On release, a spring settles it using the finger's velocity: swipe to reply, drawers, sheets and pull to refresh.
- **Vector animations** (Lottie-class) play through `remedy-ui-media` on the same clock.
- **Cost control.** An animation registers deadlines only while its element is visible and motion is allowed. Reduced Motion (from the OS or the app) turns transitions into instant changes or cross-fades, and stops spinners and GIFs on their first frame.

## 11. Input

- **Pointer events** carry pointer type (mouse, touch, pen), buttons (including back and forward), pressure, tilt and twist. Pointer capture keeps a drag going outside the element.
- **Gesture recognizers**: tap, double tap, long press, drag, pan, pinch, rotate, swipe, hover. Nested recognizers compete in a gesture arena, so a horizontal swipe on a chat row and a vertical scroll of the list do not both fire. Slop thresholds and long-press delays come from the platform.
- **Scrolling** has per-platform physics: momentum, Android's overscroll stretch, macOS rubber banding, and Windows precise touchpad deltas. It also supports scroll snapping, nested scroll views that hand off to each other, and programmatic scrolling with animation. Scrollbars appear on hover or always, per platform and theme.
- **Focus.** The tab order follows the tree, can be overridden per element, and has focus scopes. Arrow-key navigation works inside groups (toolbars, grids, radio groups). Modals and menus trap focus and give it back on close. The focus-visible ring is drawn by the theme only after keyboard use.
- **Commands.** A command has an id, a label, a default chord and an enabled state. Commands are registered at window, screen or element scope, and the innermost enabled one wins. Users can rebind any chord. Menus, context menus, tooltips and a command palette all read the same table. FreeSpeak's keybinds by name map onto it.
- **Drag and drop** works inside the app and with other apps. Payloads can be files, text, images or custom data. Drag previews are rendered elements, drop targets get hover feedback, and lists can reorder by drag.
- **Clipboard** handles plain text, rich text (HTML and the platform's own format), images and file lists. Paste is an event with the offered types, and the app picks one.
- **Global hotkeys and push-to-talk** stay outside the engine. They must work with no window and no focus. The engine gives them a `Waker`.

## 12. Windows and platform

- **Multiple windows.** Each window has its own tree root, layer stack, frame clock and DPI. A subtree can move between windows: FreeSpeak's share pop-out becomes a real window and comes back. A window that moves to a monitor with a different scale re-lays out and re-rasterizes text for the new DPI.
- **Window chrome.** Windows can use the OS frame or draw their own title bar. With a custom title bar, the drag region, caption buttons and resize borders stay native. Windows 11 snap layouts appear on hovering the maximize button, macOS keeps its traffic lights, and Linux uses client-side decorations on Wayland. Mica, acrylic and vibrancy backgrounds are available where the OS has them.
- **System theme.** Light or dark, the accent color, high contrast or forced colors, text scale, reduced motion and reduced transparency are read at start and followed live as signals.
- **Platform services**, each optional and behind a feature flag: tray icon with menu, notifications with actions, taskbar or dock badges and progress, jump lists, native file and folder dialogs (with the XDG portal on Linux, over D-Bus without an async runtime), single-instance handling, URL scheme activation, and power state (battery saver, background throttling). FreeSpeak's Windows tray and Linux StatusNotifierItem code can move here so other apps get them too.
- **Hidden windows keep living.** The runtime, timers and posted work run whether a window is visible, minimized or hidden to the tray. Only painting and presenting stop.
- **Android.** A NativeActivity / GameActivity shell gives safe-area and soft-keyboard insets as signals, maps the back button to navigation, and handles lifecycle (pause drops the GPU surface, resume restores it). Text input goes through the system IME, and the Android accessibility bridge is covered (§15). iOS and the web are later targets. Nothing in the core assumes a desktop.

## 13. Localization

- **Messages** are looked up by key with arguments, plural categories (CLDR rules) and gender select, in an ICU MessageFormat-class syntax. FreeSpeak's `.flang` files map onto it.
- **Formatting** of numbers, dates, times, durations, relative times ("5 min ago") and lists uses ICU4X data for the active locale.
- **Direction.** The locale sets the layout direction. Flex and grid mirror for RTL: start and end, not left and right. Icons that point a direction mirror too, unless marked otherwise.
- **Fonts per language.** The font fallback chain can add a font for a language, as FreeSpeak's language files do today.
- **Runtime switching.** The locale is a signal, so changing language re-renders only the text that changed.

## 14. Theming

- A **theme** is a typed set of tokens:
  - colors: `bg`, `panel`, `card`, `text`, `muted`, `accent`, `danger`, `warn`, `ok`, `speaking`, `line`, `rail`, and any custom ones
  - sizes: `small`, `body`, `title`, from which the derived text sizes come
  - spacing: `item`, `button_x`, `button_y`, `window`, `rounding`, `stroke`
  - control height, shadow, motion, and widget style tokens
  - asset handles (background images, nine-slice frames)
- **Derived tokens** are computed once per theme change, not per widget: hover and selected fills, the six text sizes, and contrast-balanced text colors.
- FreeSpeak keeps its `.fskin` parser in `freespeak-core` and maps a skin to a `Theme`. Live reload sets the theme signal, and every widget restyles in the next frame. The global `RwLock` palette goes away.
- Theme tokens are signals, so a single token can change at runtime (the skin editor) and only what reads it updates.

## 15. Accessibility

Each element declares semantics: role, name, description, value, states (checked, selected, expanded, disabled), actions and relations. `remedy-ui-a11y` turns the dirty ones into AccessKit tree updates every frame. AccessKit connects them to UI Automation on Windows, NSAccessibility on macOS, AT-SPI on Linux and the Android accessibility bridge. New chat messages use a polite live region. Focus order follows the tree unless an element sets its own order. Every widget in §8 ships with its semantics, so apps get accessibility without extra work.

## 16. Stability

- **No panics in the frame path.** The engine crates deny `unwrap`, `expect`, indexing that can panic, and integer overflow in release (`clippy::unwrap_used`, `clippy::indexing_slicing`, `overflow-checks = true`). Fallible steps return errors that degrade: a failed glyph draws the font's notdef, and a failed image draws a placeholder.
- **App callbacks are isolated.** Each event handler, effect and canvas closure runs inside `catch_unwind`. A panic there is logged with the element path, the element is marked failed and shows nothing, and the frame goes on.
- **Determinism.** Layout and paint are pure functions of the tree, theme and size. The same input gives the same display list, which the snapshot tests rely on.
- **Renderer recovery** is described in §6. Starting never depends on a single backend.
- **Bounded memory.** Every cache has a byte budget: glyphs, shaped runs, images, animated frames and layers. The budgets are part of the theme or app options, and a debug overlay shows their use.

## 17. Security

- **Memory safety.** `#![forbid(unsafe_code)]` everywhere but the GPU and platform boundary modules, which are small, audited and fuzzed through their safe APIs.
- **Untrusted parsing.** Images, GIFs and fonts from other users go through pure-Rust decoders that are fuzzed upstream and in this repository's `fuzz/` targets, with limits (§9). They run off the UI thread and never block a frame.
- **Data, not code.** Themes and skins are token data. Nothing in a theme runs. Markdown and rich text are parsed into spans and never interpreted as markup the engine executes.
- **Links** show their real destination on hover and in accessibility. The app decides what is risky, as FreeSpeak's registrable-domain check does today, and the engine exposes a hook for it.
- **Secrets.** Password inputs zero their buffers, are not copied to the clipboard by the engine, and are reported to accessibility as protected.
- **Clipboard.** It is read only on a user paste gesture.
- **Dependencies.** Every dependency must pass FreeSpeak's `deny.toml`, and the repository runs `cargo deny` and `cargo audit` in CI. No async runtime and no C libraries are needed, except the platform's own (system GL, Vulkan loader, D3D, Cocoa, Android NDK).

## 18. Performance budgets

These are checked by benchmarks in CI on a fixed reference scene (the FreeSpeak chat screen with 10,000 messages, a member list of 500 and an open voice ring).

| Measure | Budget |
|---|---|
| CPU while idle (window open, nothing changing) | 0 wakeups per second |
| Key press to present | ≤ 1 frame |
| Frame CPU time, one new message arriving | ≤ 0.5 ms |
| Frame CPU time, scrolling the chat | ≤ 2 ms |
| Frame CPU time, window resize (relayout of the view) | ≤ 4 ms |
| Cold start to first frame | ≤ 150 ms (GPU backend) |
| 1440p60 video share | one upload per frame, no CPU color conversion |
| Memory for the reference scene | ≤ 120 MB including atlases |

## 19. Tooling and testing

- **Inspector** (hotkey, debug builds and opt-in release builds): the element tree with layout boxes, resolved style, tokens in use, semantics and the signals each element reads. Clicking an element on screen selects it in the tree.
- **Performance overlay**: frame time split by phase, which elements were dirty and why, damage rectangles, and cache and atlas use against their budgets.
- **Hot reload** of themes, assets and message files while the app runs. Component code reloads by fast incremental rebuild, keeping signal state where the shape of the tree allows.
- **Headless runner.** It runs any component with the CPU renderer, scripted input (click, type, drag, IME, shortcuts) and a fake clock, then asserts on the tree, semantics or pixels.
- **Golden images** of every widget in every state, in a light and a dark theme, at 100%, 150% and 200%.
- **Snapshot tests** of layout and display lists, which do not depend on pixels.
- **Fuzzing** (cargo-fuzz) of text input editing, rich-text building, theme loading, image decoding limits and the display-list renderer.
- **Accessibility tests** check that every widget's semantics pass the AccessKit schema and that focus order matches the visual order.
- **FreeSpeak's shot tour** moves onto the headless runner, so CI no longer needs a GPU or Mesa.

## 20. Building blocks

RemedyUI writes its own engine core: reactivity, tree, styling, motion, input, the GPU UI renderer, compositing, widgets and tooling. For the deep, solved problems it uses maintained, memory-safe crates. Each one is hidden behind RemedyUI's own types, so a dependency can be upgraded or replaced without breaking apps.

Versions are as published on crates.io on 2026-11-17. They are pinned exactly in the workspace, the way FreeSpeak pins its dependencies.

| Area | Crate | Version | Why this one |
|---|---|---|---|
| Rich text layout, editing helpers | `parley` | 0.13.0 | Bidi, line breaking, inline objects, carets and selection. Pre-1.0 with breaking releases, so pinned and wrapped |
| Shaping | `harfrust` | 0.16.0 | Pure-Rust HarfBuzz port, used by parley |
| Font reading, hinting, color glyphs | `skrifa` / `read-fonts` | 0.50.0 / 0.47.0 | TrueType bytecode, CFF and auto hinting with grayscale and LCD targets. COLRv0/v1, CBDT and sbix. Fuzzed on OSS-Fuzz and shipped in Chrome |
| Font collection and fallback | `fontique` | 0.13.0 | Fallback chains and optional system fonts. Android coverage to be checked in M2 |
| Segmentation, bidi, locale data | ICU4X (`icu_segmenter` and others) | 2.4.0 | Unicode-correct segmentation and CLDR data. Replaces the stale `unicode-bidi` |
| Layout algorithms | `taffy` | 0.15.1 | CSS flexbox, grid and block, as faithful to the specs as browsers |
| GPU access | `wgpu` | 30.0.2 | D3D12, Vulkan, Metal and GL 3.3 / GLES 3.0 |
| Path rasterization | `vello_gpu`, `vello_cpu` | 0.4.0 | Sparse-strip paths without compute shaders on the GPU, and a fast multithreaded CPU renderer. Pre-1.0, so behind RemedyUI's path type |
| Windowing and events | `winit` | 0.30.13 (0.31 when stable) | Windows, macOS, Wayland, X11 and Android. 0.31 is in beta (0.31.0-beta.4) |
| Accessibility | `accesskit` + adapters | 0.25.2 | UI Automation, NSAccessibility, AT-SPI and Android |
| Clipboard | `arboard` | 3.7.0 | Text and images on every desktop |
| File dialogs | `rfd` | 0.18.0 | Native dialogs. On Linux it is used through the portal, without an async runtime |
| Images | `image` / `zune-image` | 0.25.10 / 0.5.0 | Pure-Rust decoders. GIF frame handling is checked against RustSec and fuzzed in M5 before one is chosen |
| SVG | `usvg` | pinned at M5 | Parses SVG to a path tree that the renderer draws |

Every crate here is MIT, Apache-2.0 or both, and passes FreeSpeak's `deny.toml`. None needs an async runtime.

Several of these crates are young and change quickly (parley, the vello renderers, winit 0.31, accesskit). For each one, RemedyUI does three things:

- It keeps the crate behind a small internal trait or module.
- It pins the exact version.
- It runs its own tests against the behavior it relies on.

An upgrade is then one module's work. If a crate stalls, it can be forked or replaced without touching the public API.

## 21. Moving FreeSpeak over

The move happens screen by screen, with egui and RemedyUI side by side in the app until the last screen moves:

1. `theme.rs` becomes a `.fskin` → `Theme` mapping. The custom `Plate`, `switch`, `check`, `segmented`, `chip`, `lamp`, `nav_item` and other widgets are replaced by RemedyUI widgets with the same look.
2. Settings and dialogs move first. They are forms and test the widget set without the chat list.
3. The chat view moves next: virtual list, rich paragraphs, selection, hover toolbars, GIFs and the composer.
4. The voice ring (canvas), the screen-share surface and pop-out windows move last.
5. eframe, egui, and the `third_party/epaint` and `third_party/egui_glow` forks are removed, along with the `ttf-parser` fork that only egui needed.

[ROADMAP.md](ROADMAP.md) gives the milestones and what "done" means for each.
