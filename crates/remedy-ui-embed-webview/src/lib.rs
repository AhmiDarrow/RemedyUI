//! The platform's web view as an element: the RemedyAI desktop's browser slide, help pages
//! from the web. WebView2 on Windows and WebKitGTK on Linux (X11) draw the page in a native
//! child view that follows its element: placed over the element's box, cut to the part of it
//! that shows (scrolled and clipped as the element is), hidden with it, and kept out from
//! under the window's overlays (menus, dialogs, tooltips, the privacy cover) by cutting their
//! boxes out of the view's region. Elements drawn by the app's own content over the element
//! (not in an overlay) are under the web view, because a native view always draws above its
//! window's own pixels.
//!
//! **Isolation.** The page runs in the platform's own browser processes. Nothing of the app
//! is reachable from it: no signals, no host objects, no files. One JSON message channel
//! crosses, in both directions ([`WebView::post_message`], [`WebView::on_message`]); the page
//! sees it as `window.remedy`. The app can also run a script ([`WebView::eval`]), set the
//! user agent ([`WebView::set_user_agent`]) and, on WebView2, call the DevTools protocol
//! ([`WebView::devtools`]). Those are app calls; the page has no way to invoke them:
//!
//! ```js
//! remedy.onMessage((value) => console.log("from the app", value));
//! remedy.postMessage({ hello: "app" });
//! ```
//!
//! **Navigation.** Every navigation, of the page or any frame in it, is decided by the app's
//! allow-list ([`WebViewOptions::allow`]); with none, only what the app loads itself is
//! allowed. Blocked navigations are reported ([`WebView::on_blocked`]). New windows are
//! never opened, links to other programs (`mailto:`, custom schemes) never launch them,
//! permission requests (camera, microphone, location, notifications) are denied, downloads
//! are denied unless the app's callback allows each one ([`WebViewOptions::downloads`]), and
//! developer tools stay off unless asked for ([`WebViewOptions::devtools`]). Password and
//! form autofill are off.
//!
//! **Platforms.** Windows: WebView2 (the Evergreen runtime the system keeps updated).
//! Linux: WebKitGTK 4.1 (or 4.0) and GTK 3, loaded at run time, under X11 (on Wayland a
//! separate client cannot place its view inside another's surface, so the element reports
//! [`WebViewStatus::Unsupported`]). macOS (WKWebView) and Android (`android.webkit.WebView`)
//! report `Unsupported` with the reason until their bridges host native views.
//!
//! Layer 5. The platform calls live in the `os` module, the crate's only unsafe code.

// Where no platform web view exists yet, the glue the platforms share has no caller.
#![cfg_attr(
    not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))),
    allow(dead_code)
)]

#[allow(unsafe_code)]
mod os;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use remedy_ui_a11y::{Role, Semantics};
use remedy_ui_core::Rect;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Placement, Ui};

pub use serde_json;

/// Who started a navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NavigationSource {
    /// The app: [`WebView::navigate`], [`WebView::load_html`], back, forward or reload.
    App,
    /// The page: a link, a script, a form, a redirect.
    Page,
}

/// A navigation the allow-list decides.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavigationRequest {
    /// Where to.
    pub url: String,
    /// Who started it.
    pub source: NavigationSource,
    /// Started by the person using the page (a click), not by a script.
    pub user_initiated: bool,
    /// A server redirect of an earlier navigation.
    pub redirect: bool,
    /// The navigation of a frame inside the page, not the page itself.
    pub frame: bool,
}

/// Why something the page tried was stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockReason {
    /// The allow-list refused the navigation.
    NotAllowed,
    /// The page asked for a new window.
    NewWindow,
    /// A download the app's callback did not allow.
    Download,
    /// A link to another program (`mailto:`, a custom scheme).
    ExternalScheme,
    /// A permission (camera, microphone, location, notifications, …).
    Permission,
}

/// Something the page tried that was stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Blocked {
    /// What it was for (the URL, or the origin asking for a permission).
    pub url: String,
    /// Why it was stopped.
    pub reason: BlockReason,
}

/// A download the page started.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadRequest {
    /// What is downloaded.
    pub url: String,
    /// Where the platform would save it.
    pub path: Option<PathBuf>,
}

/// How the web view is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WebViewStatus {
    /// The platform's web view is starting.
    Starting,
    /// Running.
    Ready,
    /// Not available here, and why.
    Unsupported(String),
    /// It failed, and how (the browser process stopped, the runtime refused to start).
    Failed(String),
}

/// Where the native view is, as the platform reports it: in the window's device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeView {
    /// The native child view: the part of the element that shows.
    pub frame: Rect,
    /// The page's whole box (the element's), which extends past `frame` where the element is
    /// scrolled or clipped.
    pub content: Rect,
    /// Shown.
    pub visible: bool,
}

type NavigationFilter = Rc<dyn Fn(&NavigationRequest) -> bool>;
type MessageListener = Rc<dyn Fn(&serde_json::Value)>;
type BlockedListener = Rc<dyn Fn(&Blocked)>;
type DownloadFilter = Rc<dyn Fn(&DownloadRequest) -> bool>;
type ScriptCallback = Box<dyn FnOnce(Result<String, String>)>;

/// Why a script or DevTools call will not run.
const VIEW_MISSING: &str = "the web view is missing";

/// How to make a web view.
#[derive(Clone)]
pub struct WebViewOptions {
    label: String,
    allow: Option<NavigationFilter>,
    downloads: Option<DownloadFilter>,
    devtools: bool,
    data_dir: Option<PathBuf>,
}

impl std::fmt::Debug for WebViewOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebViewOptions")
            .field("label", &self.label)
            .field("allow", &self.allow.is_some())
            .field("downloads", &self.downloads.is_some())
            .field("devtools", &self.devtools)
            .field("data_dir", &self.data_dir)
            .finish()
    }
}

impl WebViewOptions {
    /// A web view named `label` for screen readers ("Help"). With nothing else set, only
    /// navigations the app starts are allowed, downloads are denied and developer tools are
    /// off.
    pub fn new(label: impl Into<String>) -> Self {
        WebViewOptions { label: label.into(), allow: None, downloads: None, devtools: false, data_dir: None }
    }

    /// Decides every navigation of the page and its frames: true lets it happen. Blocked
    /// navigations are reported with [`WebView::on_blocked`].
    pub fn allow(mut self, f: impl Fn(&NavigationRequest) -> bool + 'static) -> Self {
        self.allow = Some(Rc::new(f));
        self
    }

    /// Decides each download the page starts: true lets the platform save it (to the
    /// user's downloads folder, with the platform's own progress UI). Without this, every
    /// download is denied.
    pub fn downloads(mut self, f: impl Fn(&DownloadRequest) -> bool + 'static) -> Self {
        self.downloads = Some(Rc::new(f));
        self
    }

    /// Lets the developer tools open (from the context menu, or F12 in the page).
    pub fn devtools(mut self, on: bool) -> Self {
        self.devtools = on;
        self
    }

    /// Where the web view keeps its profile (cookies, cache, storage). Each app should give
    /// its own data folder; without one the platform's default is used (on Windows a folder
    /// next to the executable, which must be writable).
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }
}

/// What the platform's web view reports, on the UI thread. The `os` module calls these.
pub(crate) struct Hub {
    options: WebViewOptions,
    title: Signal<String>,
    url: Signal<String>,
    loading: Signal<bool>,
    back: Signal<bool>,
    forward: Signal<bool>,
    status: Signal<WebViewStatus>,
    /// Navigations the app started that have not reached the platform's check yet, oldest
    /// first, numbered.
    expected: RefCell<Vec<(u64, Expected)>>,
    next_expected: Cell<u64>,
    /// Top-level addresses the view has been allowed to show: where the app's back and
    /// forward may go.
    visited: RefCell<std::collections::HashSet<String>>,
    on_message: RefCell<Vec<MessageListener>>,
    on_blocked: RefCell<Vec<BlockedListener>>,
    blocked: RefCell<Vec<Blocked>>,
    /// Messages from the page that were not JSON (never delivered).
    malformed: Cell<u64>,
    closed: Cell<bool>,
    /// The next id for [`begin_script`](Self::begin_script).
    script_next: Cell<u64>,
    /// Script and DevTools callbacks waiting for the platform, keyed by id.
    script_wait: RefCell<HashMap<u64, ScriptCallback>>,
}

/// A navigation the app started, as the platform will report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Expected {
    /// To this address.
    Url(String),
    /// To the app's own HTML (the platform reports a `data:` or `about:` address).
    Html,
    /// Back, forward or reload: to an address the view has shown.
    History,
}

impl Expected {
    fn matches(&self, url: &str, visited: &std::collections::HashSet<String>) -> bool {
        match self {
            // Platforms add a trailing slash to a bare host.
            Expected::Url(u) => url == u || url.strip_suffix('/') == Some(u.as_str()),
            Expected::Html => url.starts_with("data:") || url.starts_with("about:"),
            Expected::History => visited.contains(url),
        }
    }
}

impl Hub {
    /// The app is about to start a navigation; returns its number, to
    /// [`forget`](Self::forget) it when the platform refuses the call.
    pub(crate) fn app_navigates(&self, e: Expected) -> u64 {
        let n = self.next_expected.get();
        self.next_expected.set(n.wrapping_add(1));
        self.expected.borrow_mut().push((n, e));
        n
    }

    /// The platform refused the app's navigation `n`: it will not be reported. (WebKitGTK
    /// never refuses a load synchronously.)
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn forget(&self, n: u64) {
        self.expected.borrow_mut().retain(|(k, _)| *k != n);
    }

    /// A top-level page finished loading at `url`.
    pub(crate) fn shown(&self, url: String) {
        self.visited.borrow_mut().insert(url);
    }

    /// Who started a top-level navigation to `url`: the app when it matches one the app
    /// started (which, and those before it, are then done), else the page. A page cannot
    /// pass for the app: it would have to go exactly where the app asked.
    fn source(&self, url: &str) -> NavigationSource {
        let visited = self.visited.borrow();
        let mut expected = self.expected.borrow_mut();
        match expected.iter().position(|(_, e)| e.matches(url, &visited)) {
            Some(i) => {
                expected.drain(..=i);
                NavigationSource::App
            }
            None => NavigationSource::Page,
        }
    }

    /// Asks the allow-list about a navigation; reports it when blocked.
    pub(crate) fn decide(&self, url: String, user_initiated: bool, redirect: bool, frame: bool) -> bool {
        if self.closed.get() {
            return false;
        }
        let source = if frame || redirect { NavigationSource::Page } else { self.source(&url) };
        let req = NavigationRequest { url, source, user_initiated, redirect, frame };
        let allowed = match &self.options.allow {
            Some(f) => f(&req),
            None => source == NavigationSource::App,
        };
        if !allowed {
            self.block(req.url, BlockReason::NotAllowed);
        } else if !frame {
            self.loading.set_if_changed(true);
        }
        allowed
    }

    /// Asks the app's download callback; reports a denied download.
    pub(crate) fn download(&self, url: String, path: Option<PathBuf>) -> bool {
        let req = DownloadRequest { url, path };
        let allowed = !self.closed.get() && self.options.downloads.as_ref().is_some_and(|f| f(&req));
        if !allowed {
            self.block(req.url, BlockReason::Download);
        }
        allowed
    }

    pub(crate) fn block(&self, url: String, reason: BlockReason) {
        let b = Blocked { url, reason };
        self.blocked.borrow_mut().push(b.clone());
        let listeners = self.on_blocked.borrow().clone();
        for f in listeners {
            f(&b);
        }
    }

    /// A message from the page, as JSON text.
    pub(crate) fn message(&self, json: &str) {
        if self.closed.get() {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            self.malformed.set(self.malformed.get() + 1);
            return;
        };
        let listeners = self.on_message.borrow().clone();
        for f in listeners {
            f(&value);
        }
    }

    pub(crate) fn set_title(&self, t: String) {
        self.title.set_if_changed(t);
    }

    pub(crate) fn set_url(&self, u: String) {
        self.url.set_if_changed(u);
    }

    pub(crate) fn set_loading(&self, on: bool) {
        self.loading.set_if_changed(on);
    }

    pub(crate) fn set_history(&self, back: bool, forward: bool) {
        self.back.set_if_changed(back);
        self.forward.set_if_changed(forward);
    }

    pub(crate) fn set_status(&self, s: WebViewStatus) {
        if !self.closed.get() {
            self.status.set_if_changed(s);
        }
    }

    pub(crate) fn devtools(&self) -> bool {
        self.options.devtools
    }

    pub(crate) fn data_dir(&self) -> Option<&std::path::Path> {
        self.options.data_dir.as_deref()
    }

    /// Registers `cb` for one script or DevTools result. The id goes on the [`Command`].
    pub(crate) fn begin_script(&self, cb: ScriptCallback) -> u64 {
        let id = self.script_next.get();
        self.script_next.set(id.wrapping_add(1));
        self.script_wait.borrow_mut().insert(id, cb);
        id
    }

    /// Delivers the result of script `id` once. A second call does nothing.
    pub(crate) fn finish_script(&self, id: u64, result: Result<String, String>) {
        let cb = self.script_wait.borrow_mut().remove(&id);
        if let Some(cb) = cb {
            cb(result);
        }
    }

    /// `c` will never run. A script or DevTools command reports that; other commands have
    /// nothing waiting.
    pub(crate) fn abandon(&self, c: &Command) {
        match c {
            Command::Eval { id, .. } | Command::Devtools { id, .. } => {
                self.finish_script(*id, Err(VIEW_MISSING.into()));
            }
            _ => {}
        }
    }

    /// Reports every script still waiting. In-flight platform callbacks then find nothing.
    pub(crate) fn abandon_pending(&self) {
        let pending: Vec<ScriptCallback> = self.script_wait.borrow_mut().drain().map(|(_, cb)| cb).collect();
        for cb in pending {
            cb(Err(VIEW_MISSING.into()));
        }
    }
}

/// Wraps `script` so WebKitGTK's string result matches WebView2's `ExecuteScript`: one JSON
/// value, with `undefined` encoded as `null`. Windows does not use this; WebView2 already
/// JSON-encodes the completion value.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn json_eval(script: &str) -> String {
    let src = serde_json::to_string(script).unwrap_or_else(|_| "\"\"".into());
    format!("(function(){{const __v=eval({src});return JSON.stringify(__v===undefined?null:__v);}})()")
}

/// What the app asks of the platform's view.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    Navigate(String),
    Html(String),
    Back,
    Forward,
    Reload,
    Stop,
    /// A message to the page, as JSON text.
    Post(String),
    Focus,
    /// Runs `script` in the page. The platform calls [`Hub::finish_script`] with JSON text.
    Eval {
        id: u64,
        script: String,
    },
    /// The page's user agent. Empty restores the engine default.
    UserAgent(String),
    /// A Chrome DevTools protocol call. `params` is a JSON object. The platform calls
    /// [`Hub::finish_script`] with the protocol's JSON result.
    Devtools {
        id: u64,
        method: String,
        params: String,
    },
}

struct Shared {
    hub: Rc<Hub>,
    native: RefCell<Option<os::Native>>,
}

/// The app's handle to a web view. Cheap to clone; UI thread only.
#[derive(Clone)]
pub struct WebView(Rc<Shared>);

impl std::fmt::Debug for WebView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = self.0.hub.status.get_untracked().unwrap_or(WebViewStatus::Starting);
        write!(f, "WebView({:?}, {:?})", self.0.hub.options.label, status)
    }
}

impl WebView {
    fn command(&self, c: Command) {
        if self.0.hub.closed.get() {
            self.0.hub.abandon(&c);
            return;
        }
        if let Some(n) = self.0.native.borrow().as_ref() {
            n.command(c);
        } else {
            self.0.hub.abandon(&c);
        }
    }

    /// Goes to `url`. The allow-list decides this too (as an app navigation).
    pub fn navigate(&self, url: impl Into<String>) {
        self.command(Command::Navigate(url.into()));
    }

    /// Shows `html`, a whole page made by the app (help text, a local page).
    pub fn load_html(&self, html: impl Into<String>) {
        self.command(Command::Html(html.into()));
    }

    /// Goes back in the page's history.
    pub fn back(&self) {
        self.command(Command::Back);
    }

    /// Goes forward in the page's history.
    pub fn forward(&self) {
        self.command(Command::Forward);
    }

    /// Loads the page again.
    pub fn reload(&self) {
        self.command(Command::Reload);
    }

    /// Stops loading.
    pub fn stop(&self) {
        self.command(Command::Stop);
    }

    /// Moves keyboard focus into the page.
    pub fn focus(&self) {
        self.command(Command::Focus);
    }

    /// Runs `script` in the top frame and calls `f` once, on the UI thread, with the result
    /// as JSON text. WebView2's `ExecuteScript` already returns that shape (`undefined` is
    /// `null`, a string is a JSON string, an object is JSON). WebKitGTK runs the script
    /// inside a wrapper that produces the same text. `f` is called with an error when the
    /// view is missing, still starting and then fails, or already closed, so a caller never
    /// waits on a callback that cannot run. The page cannot call this.
    pub fn eval(&self, script: impl Into<String>, f: impl FnOnce(Result<String, String>) + 'static) {
        let id = self.0.hub.begin_script(Box::new(f));
        self.command(Command::Eval { id, script: script.into() });
    }

    /// Sets the user agent sent with the page's requests. An empty string restores the
    /// engine's own agent. The change applies to later loads.
    pub fn set_user_agent(&self, ua: impl Into<String>) {
        self.command(Command::UserAgent(ua.into()));
    }

    /// Calls the Chrome DevTools protocol method `method` with `params` (a JSON object,
    /// `"{}"` when the method takes none) and calls `f` once with the protocol's JSON
    /// result. On WebView2 this is `CallDevToolsProtocolMethod`, which does not depend on
    /// the context-menu developer-tools switch, though some input methods are only accepted
    /// when [`WebViewOptions::devtools`] was turned on for this view. WebKitGTK has no
    /// DevTools protocol, so `f` is called with an error there and the app can fall back.
    /// `f` runs on the UI thread. The page cannot call this.
    pub fn devtools(
        &self,
        method: impl Into<String>,
        params: impl Into<String>,
        f: impl FnOnce(Result<String, String>) + 'static,
    ) {
        let id = self.0.hub.begin_script(Box::new(f));
        self.command(Command::Devtools { id, method: method.into(), params: params.into() });
    }

    /// Sends `value` to the page, which receives it in its `remedy.onMessage` listeners.
    pub fn post_message(&self, value: &serde_json::Value) {
        // A `Value` always serializes; the platform gets JSON text and nothing else.
        if let Ok(json) = serde_json::to_string(value) {
            self.command(Command::Post(json));
        }
    }

    /// Calls `f` with each message the page sends with `remedy.postMessage` (anything that
    /// is not valid JSON is dropped).
    pub fn on_message(&self, f: impl Fn(&serde_json::Value) + 'static) {
        self.0.hub.on_message.borrow_mut().push(Rc::new(f));
    }

    /// Calls `f` with each thing the page tried that was stopped.
    pub fn on_blocked(&self, f: impl Fn(&Blocked) + 'static) {
        self.0.hub.on_blocked.borrow_mut().push(Rc::new(f));
    }

    /// Everything stopped so far, oldest first.
    pub fn blocked(&self) -> Vec<Blocked> {
        self.0.hub.blocked.borrow().clone()
    }

    /// Messages from the page dropped because they were not JSON.
    pub fn malformed_messages(&self) -> u64 {
        self.0.hub.malformed.get()
    }

    /// The page's title. Tracked: a binding that reads it runs again when it changes.
    pub fn title(&self) -> String {
        self.0.hub.title.get()
    }

    /// The page's address. Tracked.
    pub fn url(&self) -> String {
        self.0.hub.url.get()
    }

    /// True while a page loads. Tracked.
    pub fn is_loading(&self) -> bool {
        self.0.hub.loading.get()
    }

    /// True when there is a page to go back to. Tracked.
    pub fn can_go_back(&self) -> bool {
        self.0.hub.back.get()
    }

    /// True when there is a page to go forward to. Tracked.
    pub fn can_go_forward(&self) -> bool {
        self.0.hub.forward.get()
    }

    /// How the web view is doing. Tracked.
    pub fn status(&self) -> WebViewStatus {
        self.0.hub.status.get()
    }

    /// Where the platform has the native view now (what the system reports, not what was
    /// asked), or `None` before it exists.
    pub fn native(&self) -> Option<NativeView> {
        self.0.native.borrow().as_ref().and_then(os::Native::native_view)
    }

    /// False once the element is gone.
    pub fn is_open(&self) -> bool {
        !self.0.hub.closed.get()
    }
}

/// An element showing the platform's web view, and the app's handle to it. Nothing is
/// shown until the app loads something ([`WebView::navigate`], [`WebView::load_html`]).
#[track_caller]
pub fn web_view(ui: &Ui, options: WebViewOptions) -> (El, WebView) {
    let label = options.label.clone();
    let hub = Rc::new(Hub {
        options,
        title: Signal::new(String::new()),
        url: Signal::new(String::new()),
        loading: Signal::new(false),
        back: Signal::new(false),
        forward: Signal::new(false),
        status: Signal::new(WebViewStatus::Starting),
        expected: RefCell::new(Vec::new()),
        next_expected: Cell::new(0),
        visited: RefCell::new(std::collections::HashSet::new()),
        on_message: RefCell::new(Vec::new()),
        on_blocked: RefCell::new(Vec::new()),
        blocked: RefCell::new(Vec::new()),
        malformed: Cell::new(0),
        closed: Cell::new(false),
        script_next: Cell::new(0),
        script_wait: RefCell::new(HashMap::new()),
    });
    let native = match ui.window_handles() {
        Some(handles) => match os::Native::new(&*handles, hub.clone(), ui.downgrade()) {
            Ok(n) => Some(n),
            Err(reason) => {
                hub.set_status(WebViewStatus::Unsupported(reason));
                None
            }
        },
        None => {
            hub.set_status(WebViewStatus::Unsupported(
                "this window has no native window to place a web view in (a headless host)".into(),
            ));
            None
        }
    };
    let shared = Rc::new(Shared { hub, native: RefCell::new(native) });
    let placing = shared.clone();
    let focusing = shared.clone();
    let closing = shared.clone();
    let el = ui
        .el("web_view")
        .grow(1.0)
        .semantics(Semantics::new(Role::Group).label(label))
        .on_placed(move |p: Option<&Placement>| {
            if let Some(n) = placing.native.borrow().as_ref() {
                n.place(p);
            }
        })
        .focusable(true)
        .on_focus_change(move |on| {
            if on && let Some(n) = focusing.native.borrow().as_ref() {
                n.command(Command::Focus);
            }
        })
        .on_removed(move || {
            closing.hub.closed.set(true);
            if let Some(n) = closing.native.borrow_mut().take() {
                n.close();
            }
        });
    (el, WebView(shared))
}

/// The script every page gets before its own: `window.remedy`, the one channel to the app.
/// `post` is the platform's way out; the platform's way in calls `window.__remedyDeliver`
/// unless it has an event of its own (`listen`).
pub(crate) fn bridge_script(post: &str, listen: &str) -> String {
    format!(
        r#"(() => {{
  'use strict';
  if (window.remedy) return;
  const listeners = new Set();
  const deliver = (value) => {{
    for (const f of [...listeners]) {{
      try {{ f(value); }} catch (e) {{ console.error(e); }}
    }}
  }};
  {listen}
  Object.defineProperty(window, 'remedy', {{
    value: Object.freeze({{
      postMessage(value) {{ {post} }},
      onMessage(f) {{ listeners.add(f); return () => listeners.delete(f); }},
    }}),
    enumerable: false,
    configurable: false,
    writable: false,
  }});
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub(options: WebViewOptions) -> Hub {
        Hub {
            options,
            title: Signal::new(String::new()),
            url: Signal::new(String::new()),
            loading: Signal::new(false),
            back: Signal::new(false),
            forward: Signal::new(false),
            status: Signal::new(WebViewStatus::Starting),
            expected: RefCell::new(Vec::new()),
            next_expected: Cell::new(0),
            visited: RefCell::new(std::collections::HashSet::new()),
            on_message: RefCell::new(Vec::new()),
            on_blocked: RefCell::new(Vec::new()),
            blocked: RefCell::new(Vec::new()),
            malformed: Cell::new(0),
            closed: Cell::new(false),
            script_next: Cell::new(0),
            script_wait: RefCell::new(HashMap::new()),
        }
    }

    #[test]
    fn without_an_allow_list_only_the_app_navigates() {
        let h = hub(WebViewOptions::new("Help"));
        h.app_navigates(Expected::Url("https://help.example".into()));
        // The page racing the app's navigation cannot pass for it.
        assert!(!h.decide("https://elsewhere.example/".into(), true, false, false), "a link");
        assert!(h.decide("https://help.example/".into(), false, false, false), "the app's own load");
        assert!(!h.decide("https://help.example/".into(), false, false, false), "only once");
        assert_eq!(
            h.blocked.borrow().as_slice(),
            [
                Blocked { url: "https://elsewhere.example/".into(), reason: BlockReason::NotAllowed },
                Blocked { url: "https://help.example/".into(), reason: BlockReason::NotAllowed },
            ]
        );
    }

    #[test]
    fn the_allow_list_decides_every_navigation_with_who_started_it() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s2 = seen.clone();
        let h = hub(WebViewOptions::new("Help").allow(move |r| {
            s2.borrow_mut().push(r.clone());
            r.source == NavigationSource::App || r.url.starts_with("https://help.example/")
        }));
        h.app_navigates(Expected::Html);
        assert!(h.decide("data:text/html,hi".into(), false, false, false));
        h.shown("https://help.example/a".into());
        let n = h.app_navigates(Expected::History);
        h.forget(n);
        assert!(h.decide("https://help.example/a".into(), false, false, false));
        assert!(h.decide("https://help.example/b".into(), true, false, false));
        assert!(!h.decide("https://ads.example/frame".into(), false, false, true));
        let seen = seen.borrow();
        assert_eq!(seen.len(), 4);
        assert_eq!(seen.first().map(|r| r.source), Some(NavigationSource::App));
        assert_eq!(seen.get(1).map(|r| r.source), Some(NavigationSource::Page), "a forgotten call");
        assert_eq!(seen.get(2).map(|r| r.source), Some(NavigationSource::Page));
        assert!(seen.get(3).is_some_and(|r| r.frame));
    }

    #[test]
    fn downloads_are_denied_unless_allowed() {
        let h = hub(WebViewOptions::new("Help"));
        assert!(!h.download("https://x.example/a.zip".into(), None));
        let h = hub(WebViewOptions::new("Help").downloads(|d| d.url.ends_with(".pdf")));
        assert!(h.download("https://x.example/manual.pdf".into(), None));
        assert!(!h.download("https://x.example/tool.exe".into(), None));
        assert_eq!(h.blocked.borrow().len(), 1);
    }

    #[test]
    fn only_json_crosses_from_the_page() {
        let h = hub(WebViewOptions::new("Help"));
        let got = Rc::new(RefCell::new(Vec::new()));
        let g2 = got.clone();
        h.on_message.borrow_mut().push(Rc::new(move |v: &serde_json::Value| g2.borrow_mut().push(v.clone())));
        h.message(r#"{"a":1}"#);
        h.message("not json");
        assert_eq!(*got.borrow(), vec![serde_json::json!({"a": 1})]);
        assert_eq!(h.malformed.get(), 1);
    }

    #[test]
    fn the_bridge_script_defines_one_frozen_channel() {
        let s = bridge_script("window.x.post(value);", "");
        assert!(s.contains("Object.freeze"));
        assert!(s.contains("writable: false"));
        assert!(s.contains("window.x.post(value);"));
    }

    #[test]
    fn a_script_callback_fires_once() {
        let h = hub(WebViewOptions::new("Help"));
        let got = Rc::new(RefCell::new(Vec::new()));
        let g2 = got.clone();
        let id = h.begin_script(Box::new(move |r| g2.borrow_mut().push(r)));
        h.finish_script(id, Ok("\"ok\"".into()));
        h.finish_script(id, Ok("again".into()));
        let got = got.borrow();
        assert_eq!(got.len(), 1);
        assert_eq!(got.first(), Some(&Ok("\"ok\"".into())));
    }

    #[test]
    fn abandoning_eval_or_devtools_reports_the_view_missing() {
        let h = hub(WebViewOptions::new("Help"));
        let eval_got = Rc::new(RefCell::new(None));
        let dev_got = Rc::new(RefCell::new(None));
        let e2 = eval_got.clone();
        let d2 = dev_got.clone();
        let eval_id = h.begin_script(Box::new(move |r| *e2.borrow_mut() = Some(r)));
        let dev_id = h.begin_script(Box::new(move |r| *d2.borrow_mut() = Some(r)));
        h.abandon(&Command::Eval { id: eval_id, script: "1".into() });
        h.abandon(&Command::Devtools {
            id: dev_id,
            method: "Browser.getVersion".into(),
            params: "{}".into(),
        });
        h.abandon(&Command::Navigate("https://help.example".into()));
        assert!(matches!(eval_got.borrow().as_ref(), Some(Err(e)) if e.contains("missing")));
        assert!(matches!(dev_got.borrow().as_ref(), Some(Err(e)) if e.contains("missing")));
        assert!(h.script_wait.borrow().is_empty(), "a navigation has no callback to leak");
    }

    #[test]
    fn webkit_eval_matches_webview2_json() {
        let src = "return \"hi\"";
        let wrapped = json_eval(src);
        let encoded = serde_json::to_string(src).unwrap_or_default();
        assert!(wrapped.contains("JSON.stringify"));
        assert!(wrapped.contains("undefined"));
        assert!(wrapped.contains(&format!("eval({encoded})")));
        assert!(!wrapped.contains("ExecuteScript"));
    }
}
