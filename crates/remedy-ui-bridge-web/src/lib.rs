//! RemedyUI's web bridge: apps in a browser page, such as PWAs.
//!
//! An app built for `wasm32-unknown-unknown` runs in a page with the same engine as on the
//! desktop. `run` (in a page) starts it and returns; the browser drives it from then on.
//!
//! - **Windows.** The first window fills the page (or the element the page names, see
//!   [`Mount`]) and follows its size with a `ResizeObserver`, in device pixels at the
//!   page's device pixel ratio; zooming the page or moving to another screen resizes it.
//!   Later windows float over the page as panels the user can move, resize, raise and close,
//!   framed by the bridge when decorated; frameless ones move and resize from the app's own
//!   chrome. Frames run on `requestAnimationFrame`; a hidden tab occludes every window, and a
//!   page the browser freezes or puts in its back-forward cache suspends the app.
//! - **Rendering.** Each window holds two canvases: the GPU renderer draws on one through
//!   wgpu (WebGPU, or WebGL2 where the browser has no WebGPU; a WebGL2 device belongs to one
//!   canvas, so other windows then use the CPU), the CPU renderer on the other with
//!   `putImageData`, damage only. The runner's renderer choice holds as on the desktop: an
//!   `Auto` window starts on the CPU and moves to the GPU when its frames are slow, without a
//!   new surface. Browsers make GPU devices asynchronously; windows draw on the CPU until it
//!   is ready.
//! - **Input.** Pointer events (mouse, touch and pen, captured while pressed, with coalesced
//!   moves), wheels in pixels, lines or pages, touchpad pinches (Ctrl+wheel, Safari's gesture
//!   events), keys by meaning and position with modifiers and repeat, and focus. The
//!   browser's own context menu opens where the app did not take the press, and stays away
//!   where it did (its own context menu, a handler using the secondary button).
//! - **Text input.** A hidden text field (a password field for secrets) sits at the caret
//!   while a field edits: input methods compose in it (composition events become the
//!   engine's preedit and commit), soft keyboards open for it, and what they type, delete and
//!   autocorrect reaches the field.
//! - **Clipboard.** Copy, cut and paste go through the browser's clipboard events, the only
//!   time a page may use the clipboard without asking; the async Clipboard API writes
//!   otherwise, and reads where the user granted it. A sensitive copy is never kept past the
//!   page losing focus.
//! - **Files.** Open dialogs are `<input type=file>` pickers whose files are read into the
//!   page's [`files`] store and answered later with their paths; save dialogs answer at once
//!   and [`files::save`] downloads the bytes. Files dropped on a window arrive the same way.
//!   Links open in a new tab without access to the app's page.
//! - **Accessibility.** The engine's AccessKit tree is mirrored into ARIA elements laid over
//!   the canvas (roles, names, values, states, live regions, focus), so screen readers read
//!   and operate the app; their clicks and focus moves come back as actions. AccessKit has no
//!   web adapter; this mirror is the bridge's own.
//! - **Settings**, followed live: dark or light, reduced motion and transparency, more
//!   contrast or forced colors, the user's text size, the language, the accent color where
//!   the browser reveals it, Apple's shortcut conventions on Apple devices, and the device's
//!   memory where the browser says.
//!
//! What a browser cannot do reports [`remedy_ui_host::Unsupported`] with the reason: tray
//! icons, keeping windows out of screen capture, dragging out of the page, moving or sizing
//! the page itself, reading the clipboard outside a paste without permission, and opening a
//! file picker except in answer to a click or key.
//!
//! On other platforms this crate holds its platform-neutral parts ([`convert`], the
//! [`files`] store, [`WebConfig`] and [`Mount`], so app code can set where it mounts on every
//! target); the functions that start an app ([`run`], [`run_with`], [`run_single_instance`])
//! exist only in a page, and are documented for every target.

pub mod convert;
pub mod files;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(unsafe_code)]
mod os;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;

/// Where the app's first window shows in the page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mount {
    /// The element with id `remedy` when the page has one, else the whole page.
    #[default]
    Auto,
    /// The whole page: the window fills the viewport.
    Body,
    /// Inside the element with this id, filling it (the page sizes it with CSS).
    Element(String),
}

/// How the bridge sits in the page.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebConfig {
    /// Where the first window shows.
    pub mount: Mount,
    /// The first window's title is also the page's (`document.title`).
    pub title_is_page_title: bool,
}

impl WebConfig {
    /// Shows the first window at `mount`.
    pub fn mount(mut self, mount: Mount) -> Self {
        self.mount = mount;
        self
    }

    /// Whether the first window's title is also the page's.
    pub fn title_is_page_title(mut self, on: bool) -> Self {
        self.title_is_page_title = on;
        self
    }
}

impl Default for WebConfig {
    fn default() -> Self {
        WebConfig { mount: Mount::Auto, title_is_page_title: true }
    }
}

/// Why the app could not start in the page.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunError {
    reason: String,
}

impl RunError {
    /// The app could not start because of `reason`.
    pub fn new(reason: impl Into<String>) -> Self {
        RunError { reason: reason.into() }
    }

    /// Why, in the browser's words.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the app could not start in this page: {}", self.reason)
    }
}

impl std::error::Error for RunError {}

/// Starts `app` in the page with the default [`WebConfig`] and returns at once; the browser
/// runs the app from then on. One app runs per page.
///
/// Only in a browser page (`wasm32-unknown-unknown`); apps start through `remedy_ui::run`,
/// which calls this with `AppConfig::web`'s settings.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), doc))]
pub fn run(app: impl remedy_ui_host::App + 'static) -> Result<(), RunError> {
    run_with(WebConfig::default(), app)
}

/// Starts `app` in the page as `config` says (where the first window mounts, whether its
/// title is the page's), and returns at once.
///
/// Only in a browser page (`wasm32-unknown-unknown`).
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), doc))]
pub fn run_with(config: WebConfig, app: impl remedy_ui_host::App + 'static) -> Result<(), RunError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        web::start(config, Box::new(app))
    }
    // Documentation built for another target shows this function; it runs only in a page.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        drop((config, app));
        Err(RunError::new(NOT_A_PAGE))
    }
}

/// Runs `app` as the only instance of `name` across the browser's tabs: the first tab starts
/// it (as [`run_with`] with `config`); a later tab hands `args` to that one, where `deliver`
/// receives them, and does not start. Returns at once; the browser settles which tab is
/// first afterwards.
///
/// Only in a browser page (`wasm32-unknown-unknown`).
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), doc))]
pub fn run_single_instance(
    name: &str,
    args: Vec<String>,
    config: WebConfig,
    app: impl remedy_ui_host::App + 'static,
    deliver: impl Fn(Vec<String>) + 'static,
) -> Result<(), RunError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let start = Box::new(move || {
            if let Err(e) = run_with(config, app) {
                web_sys::console::error_1(&e.to_string().into());
            }
        });
        web::instance::claim(name, args, start, Box::new(deliver))
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        drop((name, args, config, app, deliver));
        Err(RunError::new(NOT_A_PAGE))
    }
}

/// A page's launch arguments: its address.
///
/// Only in a browser page (`wasm32-unknown-unknown`).
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), doc))]
pub fn launch_args() -> Vec<String> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        web::instance::launch_args()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        Vec::new()
    }
}

/// True while an app runs in this page.
///
/// Only in a browser page (`wasm32-unknown-unknown`).
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), doc))]
pub fn is_running() -> bool {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        web::driver().is_some()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        false
    }
}

/// What the page-only functions answer in documentation built for another target.
#[cfg(all(doc, not(all(target_arch = "wasm32", target_os = "unknown"))))]
const NOT_A_PAGE: &str = "this runs only in a browser page (wasm32-unknown-unknown)";
