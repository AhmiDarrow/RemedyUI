# Running in a browser

A RemedyUI app runs in a browser page, and as an installable web app (a PWA), with the same
engine it runs with on the desktop. The app's code does not change: the facade's `web`
feature adds the web bridge for `wasm32-unknown-unknown`, and [`run`](crate::run) starts the
app in the page. This page covers how an app gets there, what the page gives it, and what a
browser cannot do.

## How it runs

- **One call to start.** In a page, [`run`](crate::run) starts the app and returns at once;
  the browser drives it from then on. Built for `wasm32-unknown-unknown` without the `web`
  feature, `run` returns [`RunError::NeedsFeature`](crate::RunError::NeedsFeature).
- **Windows.** The first window fills the page, or the element with id `remedy` when the page
  has one, and follows its size; [`AppConfig::web`](crate::AppConfig::web) picks another
  place ([`web::Mount`](crate::web::Mount): the whole page, or inside an element of the
  page's) and whether the window's title is the page's. Later windows float over the page as
  panels the person can move, resize, raise and close.
- **No threads needed.** The engine's background work (the compositor, image and page
  workers, spell checking, table sorts, [`App::background`](crate::App::background)) runs on
  threads natively and in short slices between frames in a page. App code calls the same
  functions on both.
- **Rendering.** The GPU renderer draws through WebGPU, or WebGL2 where the browser has no
  WebGPU, and the CPU renderer draws with damage-only presents. Windows start on the CPU and
  move to the GPU as on the desktop; browsers make GPU devices asynchronously, so windows
  draw on the CPU until it is ready.
- **Input.** Mouse, touch and pen, wheels and touchpad pinches, keys by meaning and position,
  and input methods and soft keyboards through a hidden field kept at the caret (a password
  field for secret inputs).
- **Clipboard, files and links.** Copy, cut and paste go through the browser's clipboard
  events. Open dialogs are file pickers whose files the page keeps in an in-page store
  ([`web::files`](crate::web::files)); saving downloads the file. Dropped files arrive
  the same way. Links open in a new tab with no access to the app's page.
- **Screen readers.** The window's semantics tree is mirrored into ARIA elements over the
  canvas, so screen readers read and operate the app as they do on the desktop.
- **Settings**, followed live: dark or light, reduced motion and transparency, more contrast,
  text size, language, the accent color where the browser reveals it, and Apple's shortcut
  conventions on Apple devices.
- **One instance.** [`AppConfig::single_instance`](crate::AppConfig::single_instance) holds
  across tabs: a second tab hands its launch arguments to the first.
- **Media.** [`playback`](crate::playback) plays in the page, decoding with the browser's
  WebCodecs and playing sound through WebAudio, with pictures kept in step with its clock.

## The app's entry point

The app's `main` is its entry point in a page too: `wasm-bindgen` makes the binary's `main`
the module's start function, and [`run`](crate::run) there starts the app in the page and
returns. The `remedy_ui::web` module is there on every target with the `web` feature (its
functions that start an app run only in a page), so the configuration builds everywhere
without `cfg`:

```
use remedy_ui::AppConfig;
use remedy_ui::web::{Mount, WebConfig};

// Shown inside <div id="notes-app"> of a page the app's site lays out, not the whole page.
let config = AppConfig::new("Notes").web(WebConfig::default().mount(Mount::Element("notes-app".into())));
assert_eq!(config.web.mount, Mount::Element("notes-app".into()));
```

A page can also load the app as a library, starting from an exported function. The
`wasm-bindgen` crate marks it, and `web-sys` reaches the browser's console:

```text
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    if let Err(e) = remedy_ui::run(remedy_ui::AppConfig::new("Notes"), notes::app) {
        web_sys::console::error_1(&e.to_string().into());
    }
}
```

The app itself, `notes::app` here, is the same function the desktop's `main` runs and the
tests use. Build the crate as a `cdylib` for the browser, with the facade's `web` feature on:

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
remedy-ui = { version = "0.1", features = ["web"] }
```

## Building a page

`cargo remedy package --target web` builds any app into an installable page in
`target/remedy/web/<name>`, and a zip of it in `target/remedy/dist`: the app built for
`wasm32-unknown-unknown` with remedy-ui's `web` feature, the WebAssembly module and its
`wasm-bindgen` glue, an HTML page with the `#remedy` element the first window mounts in, a web
app manifest, icons, and a service worker that keeps the page working offline and is replaced
by each new build. `--example <name>` packages an example instead of the binary, `--dev` a
debug build, and `--out <dir>` writes the page elsewhere.

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130
cargo remedy package --target web
```

It refuses a `wasm-bindgen` CLI other than the version the app's `Cargo.lock` resolved, since
the glue and the module must match. `remedy.toml` names the page:

```toml
[app]
name = "Notes"
description = "Notes, built with RemedyUI"

[web]
short_name = "Notes"          # under the installed icon
theme_color = "#1e2a24"       # the browser's chrome and the splash screen
icon = "assets/icon.svg"      # SVG or PNG; else [app] icon, else the default
```

Serve the folder over HTTPS, or on `localhost` (a secure context, which service workers and
the clipboard need), to install it. `crates/remedy-ui-bridge-web/tools/build-web.sh [serve]`
packages the bridge's `web-demo` example this way and serves it on `localhost`; that example
shows a whole app: a button, a text field, copy and paste, a link, open and save dialogs with
dropped files, and the renderer in use.

`crates/remedy-ui-bridge-web/tools/web-test.sh` runs the browser tests in headless Firefox or
Chrome.

## What a browser cannot do

A page is sandboxed by the browser, and some things an app does on the desktop are not
allowed there. Each reports [`host::Unsupported`](crate::host::Unsupported) with the reason,
so the app can say so or offer another way:

| Not in a page | Why |
|---|---|
| Tray icons | A page has no place in the system's tray |
| Keeping a window out of screen capture | Browsers give pages no say over capture |
| Dragging out of the page into other apps | Pages can receive drops, but not start drags that leave the browser |
| Moving or sizing the page's own window | The browser owns its window |
| Reading the clipboard outside a paste | Browsers allow it only during a paste, or with the person's permission |
| File pickers and full screen outside a click or key | Browsers allow them only in answer to the person's own input |
| Signed updates ([`update`](crate::update)) | A web app updates through its service worker, which the build sets up |

Test the app itself headless as on the desktop, with
[`testing::TestApp`](crate::testing::TestApp); the browser tests above cover the bridge.
