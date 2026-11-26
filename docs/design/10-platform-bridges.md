# 10. Platform bridges

RemedyUI has its own model of a host: windows, surfaces, input, text input, clipboard, drag and drop, dialogs, system settings, power and services. Each platform is a **bridge** that translates that model to the platform. Nothing platform-shaped enters the engine. Adding a platform means writing a bridge; nothing else changes.

## The host model

A bridge implements a small set of traits:

| Trait | What the engine asks for |
|---|---|
| `Host` | Event loop integration, a wake handle for other threads, timers, the vsync source, system settings as signals (theme, accent, contrast, text scale, motion, transparency, locale), memory pressure, power state |
| `WindowHost` | Create, show, hide, move, resize, minimize, maximize, fullscreen, always on top, frameless with native hit regions (drag area, caption buttons, resize edges), parent and child windows, per-window scale, safe-area insets, capture protection, cover in the task switcher, the mouse pointer hidden over it (`set_cursor_hidden`: what `El::cursor_hidden` asks, a no-op without a pointer) |
| `SurfaceHost` | A drawable for the GPU renderer (raw window handle) and/or a pixel buffer with damage for the CPU renderer; presentation timing feedback |
| `TextInputHost` | IME sessions with the caret rect, surrounding text, composition, private-input hints, the on-screen keyboard |
| `ClipboardHost` | Read and write typed items, sensitive items, change notifications |
| `DragHost` | Start drags with typed payloads and a preview; receive drops |
| `DialogHost` | Open, save and folder dialogs; message boxes; print |
| `ServiceHost` (optional, per feature) | Tray, notifications, badges, jump lists, single instance, URL activation, autostart, global shortcuts where the platform permits |
| `MediaHost` (optional) | Platform decoders and sound output for `media-playback` (Media Foundation and WASAPI, AVFoundation and Audio Queues, MediaCodec and AAudio, the system GStreamer on Linux). The trait and the platform implementations live in `media-playback` itself (its `os` module), so only apps that play media carry them; an app can supply its own |
| `SandboxHost` (optional) | Starting the sandboxed decoder helper for `media-isolated` |
| `WebViewHost` (optional, later) | The platform web view for `embed-webview`. Until a bridge offers it, `embed-webview` places its own native child view from the window's handles (see below) |
| `A11yHost` | AccessKit adapter for the platform |

A trait method that a platform cannot honor returns `Unsupported` with a reason. It is never faked. The engine adapts: a frameless window falls back to a native frame, and a missing tray hides tray-only actions in the UI.

## Bridges

| Bridge | Windowing and events | Surfaces | Text input | Notes |
|---|---|---|---|---|
| `bridge-windows` | winit, plus Win32 for chrome (`WM_NCHITTEST`, snap layouts); DirectComposition and system backdrops (Mica, acrylic) are planned, not written ([04](04-rendering.md#not-yet-in-code)) | D3D12/Vulkan via wgpu's own swapchain; GDI blit for CPU | TSF (own module) | Tray via `Shell_NotifyIcon`, toasts, jump lists, `SetWindowDisplayAffinity` |
| `bridge-macos` | winit, plus AppKit for chrome, vibrancy and menus | Metal (`CAMetalLayer`); CPU blit via `CALayer` contents | `NSTextInputClient` | Global menu bar from the command table |
| `bridge-linux` | winit, Wayland or X11 chosen at run time | Vulkan/GLES; `wl_shm` or X11 shared memory for CPU | `text-input-v3` on Wayland, IBus/Fcitx via XIM on X11 | One crate for both display servers, since winit picks one per session. Fractional scale, client-side decorations drawn by RemedyUI, XDG portal for settings and dialogs, the tray as a StatusNotifierItem or, on X11 without a StatusNotifier host, in the XEmbed system tray (its menu drawn by RemedyUI, submenus cascading beside it, driven by arrows, Enter, Escape and `&` access keys as Windows menus are), no capture protection (no display server offers it) |
| `bridge-android` | NativeActivity over `bridge-winit` | Vulkan/GLES (windows start on the GPU: a surface keeps its first renderer); `ANativeWindow` buffers for CPU | `InputConnection` from an invisible view over the activity's | Insets, back navigation, lifecycle, `FLAG_SECURE`, trim-memory, Storage Access Framework dialogs, drags out, notifications, foreground services. Android's Java-only listeners come from a small Java half the APK carries (RemedyActivity, the foreground service), or loaded from memory on a plain NativeActivity |
| `bridge-remedyos` | RemedyOS surface and input wires | Shared framebuffer surface for the CPU renderer now; native GPU queue when the RemedyOS GPU backend exists | RemedyOS text-input wire | See below |
| `bridge-web` | DOM canvas and events; later windows float over the page | WebGPU / WebGL2 via wgpu on one canvas, `putImageData` with damage on a 2D canvas beside it (so a window changes renderer without a new surface) | Hidden text field at the caret (a password field for secrets) | For PWAs (GuitarRemedy, Craft Calc today ship as PWAs). ARIA mirror of the AccessKit tree; clipboard through the browser's copy, cut and paste events; file pickers answered later, downloads for saves. Background work runs cooperatively on the page's thread (`core::exec`) |
| `bridge-ios` (later) | UIKit | Metal | `UITextInput` | |
| `bridge-headless` | None | CPU renderer into memory | Scripted | Tests, previews, server-side rendering of images |

winit is used where it serves, for cross-platform windows and events. The desktop bridges share one implementation of the host contract over winit, `bridge-winit`; each adds only its platform's hooks (clipboard, capture protection, system settings, and modules such as TSF). Each bridge adds its own module where winit stops short: TSF, DirectComposition, custom chrome, AppKit menus, Android insets. Moving to winit 0.31 or later, or replacing it on one platform, is a change inside that bridge only.

## RemedyOS

RemedyOS draws its desktop in the kernel today (`kern_surface.c`, `kern_draw.c`, Remedy Sans atlases, `THEME.wire`). RemedyUI gives RemedyOS a native toolkit for its own apps, as a first-class bridge:

- **Surfaces.** A RemedyUI window on RemedyOS is a RemedyOS surface. The CPU renderer draws into a shared buffer and sends damage over the surface wire. The kernel compositor (the desk and Habitat) composites it with its other windows. When RemedyOS gets a native GPU path, `SurfaceHost` adds a GPU queue and the GPU renderer is used. No app code changes.
- **Input** arrives as RemedyOS input records (pointer, key, text input), translated into the engine's events. Shortcuts use the same `Primary` = Ctrl rule.
- **Theme.** `THEME.wire` maps to RemedyUI tokens, so RemedyOS's own theme and a user's edits to it apply to every RemedyUI app on the desktop. This is the code-fluid tenet across the whole OS.
- **Services.** Notifications, the dock badge, file dialogs (the native Files app), capture protection, memory pressure and power come from kernel wires. Each is translated in the bridge; none is a Linux or Windows concept.
- **Semantics.** The accessibility tree goes to RemedyOS over a semantics wire, which also serves the mind (see [11](11-accessibility-and-automation.md)). This gives RemedyOS the accessibility tree its kernel-drawn UI does not have today.

## Embedding and being embedded

- **Embedding other content** (optional packs):
  - **`embed-webview`** hosts the platform web view (WebView2, WKWebView, WebKitGTK, Android WebView) as an element. It is for the RemedyAI desktop's browser slide and help pages from the web. It runs with its own process isolation, no access to the app's signals except through an explicit message channel, and navigation limited by the app's allow-list.
    - The page sits in a native child view of the window, placed over the element's box after each frame: cut to the part that shows (scroll containers and clips), moved as it scrolls, hidden with it, and with the boxes of the window's overlays (menus, dialogs, tooltips, the privacy cover) cut out of its region, since a native view always draws above its window's own pixels. App content drawn over the element outside the overlay layer stays under the page.
    - Windows: WebView2 in a child window (region cut-outs with `SetWindowRgn`). Linux: WebKitGTK 4.1/4.0 with GTK 3, loaded with `dlopen`, in a GTK popup window reparented into the app's X11 window (cut-outs with the X shape extension). Under Wayland one client cannot place a surface inside another's, so the element reports `Unsupported`. macOS (WKWebView as a child `NSView`) and Android (`android.webkit.WebView`) report `Unsupported` until their bridges host native views.
    - Every navigation of the page and its frames goes through the app's allow-list (with none, only what the app loads); new windows, links to other programs and permission requests are refused and reported; downloads are denied unless the app's callback allows each; developer tools are off unless asked for. The page's one way to the app is `window.remedy` (`postMessage`, `onMessage`), carrying JSON values.
  - **`embed-gpu`** hosts the app's own wgpu rendering (3D previews, game views) as an external texture.
    - The app renders on the GPU renderer's own device (shared by every window in the process), given only the device and queue, inside error scopes that report its mistakes instead of failing the window. After a device loss, the replacement is shared by windows and content alike, and the content prepares again.
    - The element owns two textures sized from its box and the window's scale, renders into one while the other is shown, and places the picture with `Item::Texture`, which the GPU renderer samples like any image. On the CPU renderer the picture is read back once per render; the element asks the runner to move an `Auto` window to the GPU at once.
    - Pointer input (in element coordinates), keys and focus reach the content; the element reads as an image named by its label.
- **Being embedded.** RemedyUI can render into a texture or surface the host owns, for a game engine's menus or another toolkit's window. It uses `bridge-headless` input routing and the GPU renderer on the host's device.

## Platform conventions

Bridges carry each platform's conventions into the engine as data, so widgets behave natively without app code:

- the primary modifier and standard shortcuts (copy, paste, undo, redo, find, select all, close, quit, preferences)
- dialog button order (OK/Cancel on Windows, Cancel/OK on macOS)
- scroll physics and scrollbar style
- touch slop, double-click time and long-press delay
- caret blink rate
- the context-menu trigger (right click, two-finger click, long press)
- whether windows have a global menu bar or in-window menus
- the window chrome layout

An app that wants one look everywhere can override any of these.
