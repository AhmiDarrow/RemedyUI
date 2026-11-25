# The Remedy apps on RemedyUI

RemedyUI is for every Remedy app. This map, first drawn from a survey of the apps on 2026-11-17, shows what each one needs from the engine, which packs and bridges it links, in what order the apps move, and how far each has got. Before RemedyUI the apps used React with Tauri or Capacitor, egui, PyQt with Kivy, pywebview with HTML, or kernel-drawn C. Each gains native speed, one shared look, and the engine's accessibility and security.

## Where each app stands (2026-11-21)

Every pack the apps need is built (see [ROADMAP.md](ROADMAP.md)), so what is left is app work. Each app moves on a `remedyui` branch, or a `remedyui/` folder, in its own repository, ticking a `PORT-CHECKLIST.md` of everything its old interface does.

| App | Port | Released on RemedyUI | What is left |
|---|---|---|---|
| **Alex's Craft Calc** | Done, merged (`remedyui/`) | Yes: `remedyui-v2.1.1` (Windows, Android, web) | — |
| **SecretSticky** | Done on `remedyui` (checklist 47/47) | No; v0.2.0 is still Tauri | Merge, release; macOS run |
| **SecretFolder** | Done on `remedyui` (checklist 41/41) | No; v0.2.0 is still Tauri | Merge, release; macOS run |
| **Hearth** | Done on `remedyui` (checklist 55/55) | No; v0.4.0 is still pywebview | Merge, release; macOS run |
| **FreeSpeak** | Done on `remedyui`, egui removed (checklist 429/430) | No; `main` is still egui | macOS (tray, hotkeys, signing); merge, release |
| **GuitarRemedy** | Started on `remedyui`: workspace, core crate, content as data | No | The screens |
| **Portal** | Not started; the engine has what it needs (HLS, casting controls, film timeline, Portal's keys, tested live against Portal's server and a Cast TV) | No | The app |
| **RemedyPDF, VoiceRemedy** | Not started | No | All |
| **RemedyAI desktop, RemedyOS apps** | Not started | No | All |

None of these waits on RemedyLua. It only adds Lua bindings for effects; FreeSpeak's own Lua 5.4 engine plugs in through `ScriptEngine` today.

## Coverage

| App | What it is | Before | Packs it uses | Bridges |
|---|---|---|---|---|
| **FreeSpeak** | Chat, voice, screen share | Rust, egui/eframe | core, nav, data, forms, doc-rich, media-image/anim/svg/video/isolated, viz (voice ring meters), services-desktop, spell, services-i18n, update | Windows, macOS, Linux, Android |
| **SecretSticky** | Encrypted sticky notes | Tauri + React | core, forms (colors), services-desktop (tray, single instance), update; frameless always-on-top windows; `secure` | Windows (macOS, Linux free) |
| **SecretFolder** | Encrypted folder vault | Tauri + React | core, nav, data (explorer tree and table), media-image (preview), services-desktop, update; `secure` (idle lock, clipboard clear, capture protection) | Windows |
| **Alex's Craft Calc** | Soap and candle calculators, wiki | React PWA, Tauri, Capacitor | core, forms (exact decimal inputs), data (tables), doc-rich (wiki, F1 help), services-i18n, update | Windows, Android, web |
| **Hearth** | LAN folder sharing and chat | Python, pywebview | core, nav, data, media-image, media-isolated, services-desktop (tray, folder picker), services-i18n, update; drag in and out | Windows |
| **Portal** | Media dashboard with HLS player | Rust service, TypeScript web UI, tray launcher | core, nav, data (shelves as virtual grids), media-image, media-video, media-playback, extra (QR pairing), services-desktop | Windows, web |
| **RemedyPDF** | Multi-format document viewer and annotator | Python, PyQt5 and Kivy | core, nav, doc-pages, forms, media-isolated, services-desktop, update; the document engine (MuPDF) stays in the app | Windows, Android |
| **GuitarRemedy** | Guitar school, tuner, tabs | React, Tauri, Capacitor, PWA | core, nav, doc-rich (wiki), viz (tuner), music (fretboard, tab staff), services-i18n, update; audio stays in the app | Windows, Android, web |
| **VoiceRemedy** | Offline singing coach | React, Tauri, Capacitor | core, nav, viz (pitch graph at 60 fps), music (note staff), services-i18n; mic capture stays in the app | Windows, Android (tablets) |
| **RemedyAI desktop** | AI partner app | Tauri + React | core, nav, data, forms, doc-rich, doc-code (editor, diffs), widgets-term, embed-webview, viz (usage), media-image, extra (QR), services-desktop, services-i18n, update | Windows, macOS, Linux, Android |
| **RemedyOS apps** | Files, Editor, Settings, Monitor, Ask, panels | Kernel-drawn C | core, nav, data, forms, doc-code, widgets-term, viz (Monitor), services via kernel wires | RemedyOS (developed on all) |
| **FallenEarth** | RPG | Godot 4 | Out of scope. A game engine draws its own UI. RemedyUI could render menus into a Godot texture through the embedding path if wanted | — |

Six of these apps (SecretSticky, SecretFolder, Alex's Craft Calc, GuitarRemedy, VoiceRemedy, RemedyAI desktop) kept their users' data in a web view's `localStorage`. Each one's first RemedyUI run imports it with `services-webstorage` (read-only, lossless), so no update loses what people saved.

The survey found no app that needs maps or calendars. Date and time pickers are in `forms` because settings and forms need them. Maps stay out of scope until an app needs them.

## What only one app needs

These are optional packs, so no other app carries them:

- `doc-pages` (RemedyPDF)
- `widgets-term` and `doc-code` (RemedyAI, RemedyOS)
- `music` (GuitarRemedy, VoiceRemedy)
- `media-playback` (Portal)
- `embed-webview` (RemedyAI)
- the RemedyOS bridge

Secret handling is needed by SecretFolder, SecretSticky, Hearth and FreeSpeak. It is small and must be right, so it sits in core.

## One look

Most of these apps already share the Dark Forest look and the theme set defined in RemedyAI's `desktop/src/themes.ts`, mirrored in Portal and Hearth and copied into FreeSpeak's skins. That set, including the deutan, protan and tritan palettes, is now `remedy-ui-widgets::themes`: one source for every app and the RemedyOS desktop ([design/13](design/13-design-language.md)).

## Moving order

The engine is proven on small apps first and on the largest last. Each move exercises a new part of the engine. Milestone numbers are from [ROADMAP.md](ROADMAP.md), which is the source of truth.

| Milestone | Apps | What they prove |
|---|---|---|
| M5 | SecretSticky | Many frameless always-on-top windows, persisted geometry, tray, single instance, `secret_input`, idle lock, clipboard clear, auto-update |
| M6 | SecretFolder, Hearth | Explorer tree and table, drag in and out, previews, capture protection, localization, macOS and Linux |
| M7 | FreeSpeak | Chat-grade lists, rich text, media, video, voice ring, multi-window, Android |
| M8 | RemedyPDF, GuitarRemedy, VoiceRemedy, Alex's Craft Calc, Portal | Document pages, music and viz widgets, playback, the web bridge for apps that also live in a browser |
| M9 | RemedyAI desktop, RemedyOS apps | Terminal, code editor, web view, the RemedyOS bridge, agent automation |

## FreeSpeak in detail

FreeSpeak moved screen by screen, with egui and RemedyUI side by side until the last screen moved. All five steps are done on its `remedyui` branch (egui removed in `ea9f0b5c`); only the macOS run is open:

1. `theme.rs` becomes a `.fskin` → token mapping. The hand-drawn `Plate`, `switch`, `check`, `segmented`, `chip`, `lamp` and `nav_item` are replaced by RemedyUI widgets with the same look.
2. Settings and dialogs move first. They are forms and test the widget set without the chat list.
3. The chat view: virtual list with anchoring, rich paragraphs with inline emoji, selection across messages, hover toolbars as plain children, GIFs, the composer with completion.
4. The voice ring (`canvas` + `viz`), the screen-share `video_surface` and the pop-out as a real window.
5. eframe and egui are removed, along with the `epaint`, `egui_glow` and `ttf-parser` forks. Tray and hotkey code can move to `services-desktop`, or stay in the app.
