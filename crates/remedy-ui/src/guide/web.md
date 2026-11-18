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
  has one, and follows its size. Later windows float over the page as panels the person can
  move, resize, raise and close.
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
  (`remedy_ui::web::files`, on `wasm32` only); saving downloads the file. Dropped files arrive
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

On the web the page loads a WebAssembly module, so the app starts from an exported function
instead of `main`. The `wasm-bindgen` crate marks it, and `web-sys` reaches the browser's
console:

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

`crates/remedy-ui-bridge-web/tools/build-web.sh` builds an example of the web bridge into an
installable page: the WebAssembly module and its `wasm-bindgen` glue, an HTML page, a web app
manifest, an icon, and a service worker that keeps the page working offline and is replaced
by each new build. `serve` serves it on `localhost` (a secure context, which service workers
and the clipboard need).

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130
EXAMPLE=web-demo APP_NAME="RemedyUI demo" crates/remedy-ui-bridge-web/tools/build-web.sh serve
```

The script refuses to run with a `wasm-bindgen` CLI other than the version the workspace
pins, since the glue and the module must match. Its settings (`EXAMPLE`, `APP_NAME`,
`SHORT_NAME`, `THEME`, `PROFILE`, `OUT`, `PORT`) are listed at its top. The `web-demo`
example shows a whole app: a button, a text field, copy and paste, a link, open and save
dialogs with dropped files, and the renderer in use.

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
