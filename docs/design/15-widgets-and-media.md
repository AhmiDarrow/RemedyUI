# 15. Widgets and media

Every widget is built only from public engine APIs: the same ones an app uses for its own components. It takes its look from theme tokens and states, declares complete semantics, works with keyboard, mouse, touch and pen, and mirrors for RTL. Widgets are grouped into feature packs so an app links only what it uses ([00](00-principles-and-modularity.md)).

## `widgets-core` (every app)

- **Containers:** `row`, `column`, `wrap`, `grid`, `stack`, `flow`, `scroll`, `split`, `spacer`, `divider`, `card`, `section`
- **Lists:** `virtual_list`, `virtual_grid`, `for_each` (keyed), `list` with single, multi and range selection
- **Text:** `text`, `rich_text`, `link`, `label`
- **Actions:** `button` (primary, secondary, quiet, danger; icon slot; toggle), `icon_button` (label required), `menu_button`, `split_button`
- **Choices:** `switch`, `check`, `radio_group`, `segmented`, `select` (type-ahead), `chip` / `chip_group`
- **Values:** `slider` (horizontal, vertical, range, stepped, `on_commit`), `stepper`
- **Input:** `text_input` (single and multi-line, filters, completion popup), `search`, `secret_input` ([08](08-security-and-privacy.md))
- **Feedback:** `progress` (bar and ring; determinate and indeterminate), `spinner`, `lamp`, `badge`, `toast` (with action), `empty_state`, `skeleton`
- **Overlays:** `tooltip`, `menu`, `context_menu`, `popover`, `modal`, `dialog`, `trusted_prompt`, `lock_screen`
- **Media basics:** `image` (fit, radius, placeholder, error state), `avatar`, `icon` (vector)
- **Custom:** `canvas` (paths, text, images, hit regions, gestures), `paint_layer`

## Optional packs

| Pack | Widgets | Driving apps |
|---|---|---|
| `widgets-nav` | `tabs`, `nav_rail`, `nav_item`, `nav_stack`, `breadcrumbs`, `toolbar` (user-customizable slots), `command_palette`, `sheet`, `drawer`, `bottom_bar`, `title_bar` (custom chrome) | FreeSpeak, RemedyAI, SecretFolder, RemedyOS apps |
| `widgets-data` | `table` (sort, resize, reorder, freeze columns, virtual rows, inline edit), `tree_view` (lazy children, drag to move), `property_grid` | SecretFolder, Hearth, RemedyOS Files and Monitor |
| `widgets-forms` | `number_input` (units, precision, locale-aware), `color_picker` (with contrast readout), `key_capture`, `date_picker` / `time_picker`, `form` (validation, dirty tracking, submit state) | Craft Calc (lye math needs exact decimal input), settings everywhere |
| `doc-rich` | `markdown` (headings, lists, quotes, tables, code blocks), `rich_editor`, `find_bar` | FreeSpeak, GuitarRemedy and Craft Calc wikis, RemedyAI |
| `doc-code` | `code_view` and `code_editor`: line numbers, syntax colors (tree-sitter grammars per language, each its own feature), folding, diff view, multiple cursors | RemedyAI, RemedyOS Editor |
| `doc-pages` | `page_view`: paged documents with tiled deep zoom (1%–6400%), rotation, single and two-page spreads, thumbnails, text selection over positioned runs, an annotation layer that stays exact under rotation and zoom. The app supplies pages through a trait; parsing (MuPDF, an EPUB engine) stays in the app | RemedyPDF |
| `widgets-term` | `terminal`: VT and xterm emulation, scrollback on a virtual list, selection, links, IME, true color. The PTY stays in the app | RemedyAI, RemedyOS Terminal |
| `media-image` | Image decoding per format feature (`png`, `jpeg`, `webp`, `avif`, `gif`, `ico`) | Most apps |
| `media-anim` | `animated_image` (GIF, WebP, APNG), `vector_animation` (Lottie-class) | FreeSpeak, onboarding screens |
| `media-svg` | SVG icons and illustrations; icon sets as SVG | Most apps |
| `media-video` | `video_surface` (NV12, I420, RGBA; zero-copy GPU import), `external_texture` | FreeSpeak screen share, Portal, Quickcast |
| `media-playback` | `player`: a transport bar (play, pause, seek with buffered ranges, volume, speed), subtitles track rendering (WebVTT, SRT, ASS basics) and next-up. Decoding goes through platform decoders behind a trait (Media Foundation, AVFoundation, MediaCodec, GStreamer); an app can plug in its own | Portal, Quickcast |
| `viz` | `meter`, `pitch_graph`, `waveform`, `spectrum`, `needle_gauge`, `sparkline`, `line_chart`, `bar_chart`, `timeline`; streaming ring-buffer inputs ([09](09-performance-and-resources.md)) | VoiceRemedy, GuitarRemedy tuner, RemedyAI usage dashboard, RemedyOS Monitor |
| `widgets-music` | `fretboard`, `tab_staff`, `chord_diagram`, `note_staff` (on `canvas` + `viz`) | GuitarRemedy, VoiceRemedy |
| `widgets-extra` | `qr_code` (generate), `rating`, `carousel`, `stepper_flow` (setup wizards), `color_swatch_grid` | Portal and RemedyAI pairing, onboarding |
| `embed-webview` | `web_view` ([10](10-platform-bridges.md)) | RemedyAI browser slide |
| `embed-gpu` | `gpu_view` for the app's own wgpu rendering | TheOven 3D preview, game views |
| `services-update` | Update check, download, verify (signature and hash), apply on restart, from GitHub releases or a custom feed; `update_banner` widget | GuitarRemedy, SecretFolder/Sticky, RemedyPDF, RemedyAI, Hearth |

`widgets-music` shows the pattern for domain widgets. Each sits in its own pack and is built on `canvas` and `viz`. A domain that needs something these primitives cannot express is a reason to improve the primitives, not to special-case the engine.

## Windows as widgets

Window features apps need are window options, not separate widgets:

- frameless
- always on top
- per-window persisted geometry
- tool and utility windows
- a parent window
- capture protection
- hide to tray

Every SecretSticky note is an ordinary `Window` with `frameless()`, `always_on_top(pinned)`, `persist_geometry(note_id)` and a custom `title_bar`. The manager window is `hide_on_close()` with a tray icon.

## Media pipeline

- **Decoding** runs on the worker pool, or in the isolated helper ([08](08-security-and-privacy.md)), at the size each image is shown. The same source shown at two sizes decodes twice, at both sizes, and shares the parsed headers.
- **Caching.** Decoded images are cached by (source hash, decoded size). They are uploaded to the GPU when first drawn, and the CPU copy is released after upload unless the CPU renderer or a snapshot needs it. Caches size from the device and shrink under pressure ([09](09-performance-and-resources.md)).
- **Animated images.** All views of the same source share one decoder. Frames are decoded ahead by a small window and kept in an atlas. Each view has its own clock. Off-screen, unfocused (by app setting) and reduced-motion views do not advance.
- **Video** frames come from any thread through a triple buffer. Each new frame wakes the compositor, not the UI thread, and is composited in the next vsync. Zero-copy import is used where the platform decoder gives GPU textures.
- **SVG** is parsed once into a path tree. It is drawn as paths, or rasterized into the atlas at icon sizes.
- **Placeholders.** Every media element has loading, error and empty states. A failed decode shows the error state and logs the reason, and never takes down the frame.
