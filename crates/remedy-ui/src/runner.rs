//! The app runner: windows on a host, each with a view, a compositor and a presenter.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_compositor::{Compositor, LayerId};
use remedy_ui_core::{Color, Point, Size, Vec2};
use remedy_ui_host::{
    AccessibilityEvent, FileDialog, Host, HostEvent, Pixels, ResizeEdge, SystemSettings, TimerId, TrayEvent,
    TrayIcon, WindowId, WindowOptions,
};
use remedy_ui_input::ScrollPhysics;
use remedy_ui_render_cpu::CpuCompositor;
use remedy_ui_style::{StyleSheet, Theme};
use remedy_ui_text::FontCollection;
use remedy_ui_tree::ElementId;
use remedy_ui_view::{Request, Ui};

use crate::geometry::{GeometryStore, data_dir};

/// How long window moves and resizes settle before geometry is written.
const GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(400);

/// App-wide settings.
#[derive(Clone)]
pub struct AppConfig {
    /// The app's name: its data directory and the default window title.
    pub name: String,
    /// The theme windows start with.
    pub theme: Theme,
    /// Builds the style sheet for a theme (the theme pack provides one for the widgets).
    pub sheet: Rc<dyn Fn(&Theme) -> StyleSheet>,
    /// Builds each window's fonts.
    pub fonts: Rc<dyn Fn() -> FontCollection>,
    /// Which renderer windows draw with.
    pub renderer: RendererChoice,
    /// One instance per user session: a second launch hands its arguments to the first
    /// ([`App::on_second_instance`]) and exits.
    pub single_instance: bool,
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppConfig({}, theme {}, {:?})", self.name, self.theme.name, self.renderer)
    }
}

impl AppConfig {
    /// An app named `name` with the bundled Remedy fonts and [`RendererChoice::Auto`],
    /// styled by the first Remedy palette and the widget sheet (feature `widgets`), or the
    /// Dark Forest theme with an empty sheet without it.
    pub fn new(name: impl Into<String>) -> Self {
        #[cfg(feature = "widgets")]
        let (theme, sheet): (Theme, SheetBuilder) = (
            remedy_ui_themes::PALETTES.first().map_or_else(Theme::dark_forest, remedy_ui_themes::theme),
            Rc::new(remedy_ui_themes::sheet),
        );
        #[cfg(not(feature = "widgets"))]
        let (theme, sheet): (Theme, SheetBuilder) = (Theme::dark_forest(), Rc::new(|_| StyleSheet::new()));
        AppConfig {
            name: name.into(),
            theme,
            sheet,
            fonts: Rc::new(|| {
                #[cfg(feature = "system-fonts")]
                let mut f = FontCollection::with_system_fonts();
                #[cfg(not(feature = "system-fonts"))]
                let mut f = FontCollection::new();
                for face in remedy_ui_fonts::all() {
                    let _ = f.add_static(face.data);
                }
                f
            }),
            renderer: RendererChoice::Auto,
            single_instance: false,
        }
    }

    /// Uses `theme` styled by `sheet`.
    pub fn theme(mut self, theme: Theme, sheet: impl Fn(&Theme) -> StyleSheet + 'static) -> Self {
        self.theme = theme;
        self.sheet = Rc::new(sheet);
        self
    }

    /// Runs as a single instance.
    pub fn single_instance(mut self) -> Self {
        self.single_instance = true;
        self
    }

    /// Draws with the CPU renderer only.
    pub fn cpu_only(mut self) -> Self {
        self.renderer = RendererChoice::Cpu;
        self
    }

    /// Starts every window on the GPU (games, canvases, video: apps that always need it).
    pub fn gpu_first(mut self) -> Self {
        self.renderer = RendererChoice::Gpu;
        self
    }
}

/// Which renderer draws an app's windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RendererChoice {
    /// Windows start on the CPU renderer, which costs no GPU driver memory, and move to the
    /// GPU (with the `gpu` feature) once their frames take longer than
    /// [`GPU_PROMOTE_FRAME_COST`] on average: large windows, heavy scenes, constant motion.
    #[default]
    Auto,
    /// Every window starts on the GPU, falling back to the CPU when it fails.
    Gpu,
    /// The CPU renderer only.
    Cpu,
}

/// The average CPU frame cost past which an [`RendererChoice::Auto`] window moves to the GPU.
pub const GPU_PROMOTE_FRAME_COST: Duration = Duration::from_millis(6);

/// A single CPU frame this slow moves an [`RendererChoice::Auto`] window to the GPU at once.
pub const GPU_PROMOTE_FRAME_STALL: Duration = Duration::from_millis(50);

/// The renderer drawing a window now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererKind {
    /// The CPU renderer.
    Cpu,
    /// The GPU renderer.
    Gpu,
}

/// Rolling cost of a window's recent CPU frames.
#[derive(Debug, Default)]
#[cfg_attr(not(feature = "gpu"), allow(dead_code))]
struct FrameCost {
    recent: [Duration; 16],
    filled: usize,
    next: usize,
}

#[cfg_attr(not(feature = "gpu"), allow(dead_code))]
impl FrameCost {
    /// Records a frame; true once a full window of frames averages over `limit`, or at once
    /// for a frame so slow ([`GPU_PROMOTE_FRAME_STALL`]) that the window visibly freezes.
    fn record(&mut self, cost: Duration, limit: Duration) -> bool {
        if cost >= GPU_PROMOTE_FRAME_STALL {
            return true;
        }
        if let Some(slot) = self.recent.get_mut(self.next) {
            *slot = cost;
        }
        self.next = (self.next + 1) % self.recent.len();
        self.filled = (self.filled + 1).min(self.recent.len());
        self.filled == self.recent.len()
            && self.recent.iter().sum::<Duration>() / self.recent.len() as u32 > limit
    }
}

/// How to open a window.
#[derive(Clone, Debug)]
pub struct WindowSpec {
    /// Platform window options.
    pub options: WindowOptions,
    /// Closing hides the window instead (a manager that lives in the tray).
    pub hide_on_close: bool,
    /// Saves and restores position and size under this key.
    pub persist: Option<String>,
    /// The color behind everything (the theme's background when `None`).
    pub background: Option<Color>,
    /// Covers the content while the window is not focused, so the task switcher and
    /// screen glances show nothing.
    pub privacy_cover: bool,
}

impl WindowSpec {
    /// A window titled `title`.
    pub fn new(title: impl Into<String>) -> Self {
        WindowSpec {
            options: WindowOptions { title: title.into(), ..WindowOptions::default() },
            hide_on_close: false,
            persist: None,
            background: None,
            privacy_cover: false,
        }
    }

    /// Content size in logical pixels.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.options.size = Size::new(width, height);
        self
    }

    /// Smallest size the user may resize to.
    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.options.min_size = Some(Size::new(width, height));
        self
    }

    /// Desktop position of the top-left corner.
    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.options.position = Some(Point::new(x, y));
        self
    }

    /// No platform frame: the window draws its own chrome.
    pub fn frameless(mut self) -> Self {
        self.options.decorated = false;
        self
    }

    /// Stays above other windows.
    pub fn always_on_top(mut self, on: bool) -> Self {
        self.options.always_on_top = on;
        self
    }

    /// Closing hides the window.
    pub fn hide_on_close(mut self) -> Self {
        self.hide_on_close = true;
        self
    }

    /// Saves and restores geometry under `key`.
    pub fn persist_geometry(mut self, key: impl Into<String>) -> Self {
        self.persist = Some(key.into());
        self
    }

    /// Kept out of screenshots and screen sharing.
    pub fn capture_protected(mut self, on: bool) -> Self {
        self.options.capture_protected = on;
        self
    }

    /// Covers the content while the window is not focused.
    pub fn privacy_cover(mut self, on: bool) -> Self {
        self.privacy_cover = on;
        self
    }

    /// Opens hidden.
    pub fn hidden(mut self) -> Self {
        self.options.visible = false;
        self
    }
}

type Build = Box<dyn FnOnce(&Ui, &Window)>;
/// A background job's result, typed back by its [`Job`].
type JobResult = Box<dyn std::any::Any + Send>;
/// What runs on the UI thread with a background job's result.
type Job = Box<dyn FnOnce(JobResult)>;
type Setup = Box<dyn FnOnce(&App)>;
/// Hears the system's appearance: `(prefers_dark, high_contrast)`.
type AppearanceFn = Rc<dyn Fn(Option<bool>, bool)>;
/// Hears the app's language.
type LocaleFn = Rc<dyn Fn(&str)>;

/// A result for the UI thread, completed from any thread: see [`App::completion`].
#[must_use = "a completion that is dropped forgets its callback"]
pub struct Completion<T: Send + 'static> {
    id: u64,
    results: Arc<Mutex<Vec<(u64, JobResult)>>>,
    waker: Option<remedy_ui_host::Waker>,
    done: bool,
    _t: std::marker::PhantomData<fn(T)>,
}

/// What a completion dropped unfinished sends: its callback is forgotten.
struct Abandoned;

impl<T: Send + 'static> Completion<T> {
    /// Hands `value` to the callback, which runs on the UI thread next turn.
    pub fn complete(mut self, value: T) {
        self.send(Box::new(value));
    }

    fn send(&mut self, value: JobResult) {
        self.done = true;
        self.results.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((self.id, value));
        if let Some(w) = &self.waker {
            w();
        }
    }
}

impl<T: Send + 'static> Drop for Completion<T> {
    fn drop(&mut self) {
        if !self.done {
            self.send(Box::new(Abandoned));
        }
    }
}

impl<T: Send + 'static> std::fmt::Debug for Completion<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Completion({})", self.id)
    }
}

enum Cmd {
    Open(u64, WindowSpec, Build),
    Close(u64),
    Visible(u64, bool),
    Focus(u64),
    OnTop(u64, bool),
    Title(u64, String),
    BeginMove(u64),
    BeginResize(u64, ResizeEdge),
    Minimize(u64, bool),
    Maximize(u64, bool),
    Capture(u64, bool),
    Timer(Duration, Box<dyn FnOnce()>),
    Copy(String, bool),
    CopySecret(String, Duration),
    ClearSecret,
    Theme(Box<Theme>),
    Locale(Option<String>),
    Idle(Option<Duration>, Option<Rc<dyn Fn()>>),
    Forget(String),
    Tray(Option<TrayIcon>),
    Dialog(Option<u64>, FileDialog, Box<dyn FnOnce(Vec<std::path::PathBuf>)>),
    OpenUrl(String, Box<dyn FnOnce(Result<(), String>)>),
    Quit,
}

/// Builds a style sheet for a theme.
type SheetBuilder = Rc<dyn Fn(&Theme) -> StyleSheet>;

#[derive(Default)]
struct Shared {
    cmds: VecDeque<Cmd>,
    /// The app's style sheet builder (the inspector restyles edited themes with it).
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    sheet: Option<SheetBuilder>,
    /// Open inspector windows by the window they inspect.
    #[cfg(feature = "devtools")]
    inspectors: HashMap<u64, Window>,
    next: u64,
    keep_running: bool,
    name: String,
    uis: HashMap<u64, Ui>,
    open: HashSet<u64>,
    /// Windows kept out of screen capture (where the system could do it).
    protected: HashSet<u64>,
    /// Why a window could not be opened.
    open_errors: HashMap<u64, String>,
    visible: HashMap<u64, bool>,
    /// Wakes the host's loop for requests made between events.
    waker: Option<remedy_ui_host::Waker>,
    in_event: bool,
    on_tray: Option<Rc<dyn Fn(TrayEvent)>>,
    next_job: u64,
    jobs: HashMap<u64, Job>,
    done: Arc<Mutex<Vec<(u64, JobResult)>>>,
    on_launch: Option<Rc<dyn Fn(Vec<String>)>>,
    on_activity: Option<Rc<dyn Fn()>>,
    /// Told the system's light/dark preference and high contrast, now and when they change.
    on_appearance: Option<AppearanceFn>,
    /// Hears the app's language (its own, or the system's) when it changes.
    on_locale: Option<LocaleFn>,
    /// The language the app shows now, once known.
    locale: Option<String>,
    /// The last appearance the system reported.
    appearance: Option<(Option<bool>, bool)>,
    renderers: HashMap<u64, RendererKind>,
    titles: HashMap<u64, String>,
    /// Whether each window is maximized, kept current from the host.
    maximized: HashMap<u64, remedy_ui_reactive::Signal<bool>>,
    on_geometry: HashMap<u64, Rc<dyn Fn(Point, Size)>>,
    on_closed: HashMap<u64, Vec<Box<dyn FnOnce()>>>,
}

/// The app, as its code sees it. Requests take effect right after the current event.
#[derive(Clone)]
pub struct App(Rc<RefCell<Shared>>);

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "App({} windows)", self.0.borrow().open.len())
    }
}

/// A window, as app code sees it.
#[derive(Clone, Debug)]
pub struct Window {
    key: u64,
    app: App,
}

impl PartialEq for Window {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key
    }
}

impl App {
    /// Wakes the host's loop unless an event is being handled (it drains at the end).
    fn wake_if_idle(&self) {
        let w = {
            let s = self.0.borrow();
            (!s.in_event).then(|| s.waker.clone()).flatten()
        };
        if let Some(w) = w {
            w();
        }
    }

    fn push(&self, c: Cmd) {
        let wake = {
            let mut s = self.0.borrow_mut();
            s.cmds.push_back(c);
            (!s.in_event).then(|| s.waker.clone()).flatten()
        };
        if let Some(w) = wake {
            w();
        }
    }

    /// Opens a window; `build` fills its UI once it exists.
    pub fn open(&self, spec: WindowSpec, build: impl FnOnce(&Ui, &Window) + 'static) -> Window {
        let key = {
            let mut s = self.0.borrow_mut();
            s.next += 1;
            let key = s.next;
            s.open.insert(key);
            key
        };
        let visible = spec.options.visible;
        self.0.borrow_mut().visible.insert(key, visible);
        self.0.borrow_mut().titles.insert(key, spec.options.title.clone());
        self.0.borrow_mut().maximized.insert(key, remedy_ui_reactive::Signal::new(false));
        self.push(Cmd::Open(key, spec, Box::new(build)));
        Window { key, app: self.clone() }
    }

    /// Runs `f` after `delay`.
    pub fn after(&self, delay: Duration, f: impl FnOnce() + 'static) {
        self.push(Cmd::Timer(delay, Box::new(f)));
    }

    /// Puts text on the clipboard; `sensitive` keeps it out of history and sync.
    pub fn copy(&self, text: impl Into<String>, sensitive: bool) {
        self.push(Cmd::Copy(text.into(), sensitive));
    }

    /// Copies a secret: kept out of clipboard history and sync, and cleared after `keep` if
    /// the clipboard still holds it.
    pub fn copy_secret(&self, text: impl Into<String>, keep: Duration) {
        self.push(Cmd::CopySecret(text.into(), keep));
    }

    /// Clears a secret copied with [`App::copy_secret`] now, if the clipboard still holds it
    /// (anything the user copied since is left alone). Apps call it when they lock.
    pub fn clear_secret(&self) {
        self.push(Cmd::ClearSecret);
    }

    /// Calls `on_idle` after `timeout` with no real user input (pointer, keys, wheel, input
    /// methods) in any window; `None` turns idle locking off. Timers and programmatic
    /// changes never count as activity.
    pub fn idle_lock(&self, timeout: Option<Duration>, on_idle: impl Fn() + 'static) {
        self.push(Cmd::Idle(timeout, Some(Rc::new(on_idle))));
    }

    /// Calls `f` when the user uses the app (pointer, keys, text), at most once a second.
    pub fn on_activity(&self, f: impl Fn() + 'static) {
        self.0.borrow_mut().on_activity = Some(Rc::new(f));
    }

    /// Calls `f(prefers_dark, high_contrast)` with the system's appearance now (once the
    /// system has said) and whenever it changes: apps whose theme follows the system restyle.
    pub fn on_appearance(&self, f: impl Fn(Option<bool>, bool) + 'static) {
        let f: AppearanceFn = Rc::new(f);
        let now = {
            let mut s = self.0.borrow_mut();
            s.on_appearance = Some(f.clone());
            s.appearance
        };
        if let Some((dark, hc)) = now {
            f(dark, hc);
        }
    }

    /// The system's appearance as last reported, `(prefers_dark, high_contrast)`; `None`
    /// until the system has said (an app choosing its theme again reads it).
    pub fn appearance(&self) -> Option<(Option<bool>, bool)> {
        self.0.borrow().appearance
    }

    /// Shows every window (and windows opened later) in `locale` (a BCP 47 tag) instead of
    /// the system's; `None` follows the system again. For apps with their own language
    /// setting: the choice outlasts the system's settings changes.
    pub fn set_locale(&self, locale: Option<String>) {
        self.push(Cmd::Locale(locale));
    }

    /// The language the app shows (BCP 47): its own ([`App::set_locale`]) or the system's;
    /// `None` until the system has said. For parts with no window, such as a tray menu.
    pub fn locale(&self) -> Option<String> {
        self.0.borrow().locale.clone()
    }

    /// Calls `f` with the app's language now (once known) and whenever it changes: the
    /// system's changes, or [`App::set_locale`]. Windows follow on their own; this is for
    /// what has no window (a tray menu, notifications).
    pub fn on_locale(&self, f: impl Fn(&str) + 'static) {
        let f: LocaleFn = Rc::new(f);
        let now = {
            let mut s = self.0.borrow_mut();
            s.on_locale = Some(f.clone());
            s.locale.clone()
        };
        if let Some(l) = now {
            f(&l);
        }
    }

    /// Restyles every window (and windows opened later) with `theme`.
    pub fn set_theme(&self, theme: Theme) {
        self.push(Cmd::Theme(Box::new(theme)));
    }

    /// Changes the idle timeout, keeping the callback.
    pub fn set_idle_timeout(&self, timeout: Option<Duration>) {
        self.push(Cmd::Idle(timeout, None));
    }

    /// Runs `work` on a background thread (slow work: key derivation, downloads, file
    /// reads), then `done` with its result back on the UI thread.
    pub fn background<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) {
        let c = self.completion(done);
        // A thread that cannot start drops the completion, which forgets `done`.
        let _ =
            std::thread::Builder::new().name("remedy-ui background".into()).spawn(move || c.complete(work()));
    }

    /// A result some other thread will produce (an async runtime's task, a callback from a
    /// library): `done` runs with it on the UI thread once [`Completion::complete`] is called
    /// from anywhere. Dropping the completion without completing it forgets `done`.
    ///
    /// Unlike a signal set from a posted closure, nothing here belongs to a reactive scope, so
    /// work started inside `done` (or inside an effect) is never lost when that scope re-runs.
    pub fn completion<T: Send + 'static>(&self, done: impl FnOnce(T) + 'static) -> Completion<T> {
        let mut s = self.0.borrow_mut();
        s.next_job += 1;
        let id = s.next_job;
        s.jobs.insert(
            id,
            Box::new(move |any: JobResult| {
                if let Ok(v) = any.downcast::<T>() {
                    done(*v);
                }
            }),
        );
        Completion {
            id,
            results: s.done.clone(),
            waker: s.waker.clone(),
            done: false,
            _t: std::marker::PhantomData,
        }
    }

    fn finish_background(&self) {
        let finished = {
            let s = self.0.borrow();
            std::mem::take(&mut *s.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
        };
        for (id, value) in finished {
            let job = self.0.borrow_mut().jobs.remove(&id);
            if let Some(job) = job {
                job(value);
            }
        }
    }

    /// Quits.
    pub fn quit(&self) {
        self.push(Cmd::Quit);
    }

    /// Keeps the app running when its last window closes (tray apps).
    pub fn keep_running(&self, on: bool) {
        self.0.borrow_mut().keep_running = on;
    }

    /// Shows (or updates) the tray icon; `None` removes it.
    pub fn set_tray(&self, tray: Option<TrayIcon>) {
        self.push(Cmd::Tray(tray));
    }

    /// Called when the tray icon is used.
    pub fn on_tray(&self, f: impl Fn(TrayEvent) + 'static) {
        self.0.borrow_mut().on_tray = Some(Rc::new(f));
    }

    /// Called when the app is launched again while running (with
    /// [`AppConfig::single_instance`]), with that launch's arguments.
    pub fn on_second_instance(&self, f: impl Fn(Vec<String>) + 'static) {
        self.0.borrow_mut().on_launch = Some(Rc::new(f));
    }

    /// Runs a native open or save dialog (modal to `window`), then `done` with the chosen
    /// paths (empty when cancelled).
    pub fn file_dialog(
        &self,
        window: Option<&Window>,
        dialog: FileDialog,
        done: impl FnOnce(Vec<std::path::PathBuf>) + 'static,
    ) {
        self.push(Cmd::Dialog(window.map(|w| w.key), dialog, Box::new(done)));
    }

    /// Opens `url` in the user's browser or mail app, then `done` with the outcome. Only
    /// `http:`, `https:` and `mailto:` links open; anything else could start a program.
    pub fn open_url(&self, url: impl Into<String>, done: impl FnOnce(Result<(), String>) + 'static) {
        self.push(Cmd::OpenUrl(url.into(), Box::new(done)));
    }

    /// Forgets saved geometry for `key`.
    pub fn forget_geometry(&self, key: impl Into<String>) {
        self.push(Cmd::Forget(key.into()));
    }

    /// Where the app keeps its data (created on first write).
    pub fn data_dir(&self) -> Option<PathBuf> {
        data_dir(&self.0.borrow().name)
    }

    /// The open windows.
    pub fn windows(&self) -> Vec<Window> {
        let mut keys: Vec<u64> = self.0.borrow().open.iter().copied().collect();
        keys.sort_unstable();
        keys.into_iter().map(|key| Window { key, app: self.clone() }).collect()
    }
}

impl Window {
    /// The window's UI, once it exists.
    pub fn ui(&self) -> Option<Ui> {
        self.app.0.borrow().uis.get(&self.key).cloned()
    }

    /// The app.
    pub fn app(&self) -> &App {
        &self.app
    }

    /// True until the window closes.
    pub fn is_open(&self) -> bool {
        self.app.0.borrow().open.contains(&self.key)
    }

    /// Why the window could not be opened (the system refused it), once it tried.
    pub fn open_error(&self) -> Option<String> {
        self.app.0.borrow().open_errors.get(&self.key).cloned()
    }

    /// True while the system keeps the window out of screenshots and screen sharing. Asking
    /// for it is a request: where the system has no such protection (Wayland, X11), the window
    /// opens anyway and this stays false, so the app can say so.
    pub fn is_capture_protected(&self) -> bool {
        self.app.0.borrow().protected.contains(&self.key)
    }

    /// True while shown.
    pub fn is_visible(&self) -> bool {
        self.app.0.borrow().visible.get(&self.key).copied().unwrap_or(false)
    }

    /// Closes the window.
    pub fn close(&self) {
        self.app.push(Cmd::Close(self.key));
    }

    /// Shows or hides.
    pub fn set_visible(&self, on: bool) {
        self.app.0.borrow_mut().visible.insert(self.key, on);
        self.app.push(Cmd::Visible(self.key, on));
    }

    /// Shows the window, brings it to the front and focuses it.
    pub fn focus(&self) {
        self.set_visible(true);
        self.app.push(Cmd::Focus(self.key));
    }

    /// Keeps above other windows, or not.
    pub fn set_always_on_top(&self, on: bool) {
        self.app.push(Cmd::OnTop(self.key, on));
    }

    /// Changes the title.
    pub fn set_title(&self, title: impl Into<String>) {
        let title = title.into();
        self.app.0.borrow_mut().titles.insert(self.key, title.clone());
        self.app.push(Cmd::Title(self.key, title));
    }

    /// The window's title (empty once it closed).
    pub fn title(&self) -> String {
        self.app.0.borrow().titles.get(&self.key).cloned().unwrap_or_default()
    }

    /// Starts a user move (from custom chrome, on a pointer press).
    pub fn begin_move(&self) {
        self.app.push(Cmd::BeginMove(self.key));
    }

    /// Starts a user resize from `edge`.
    pub fn begin_resize(&self, edge: ResizeEdge) {
        self.app.push(Cmd::BeginResize(self.key, edge));
    }

    /// Minimizes or restores.
    pub fn set_minimized(&self, on: bool) {
        self.app.push(Cmd::Minimize(self.key, on));
    }

    /// Maximizes or restores.
    pub fn set_maximized(&self, on: bool) {
        self.app.push(Cmd::Maximize(self.key, on));
    }

    /// True while the window is maximized, by the app or the user (tracked).
    pub fn is_maximized(&self) -> bool {
        let s = self.app.0.borrow().maximized.get(&self.key).copied();
        s.is_some_and(|s| s.get())
    }

    /// Calls `f` with the window's desktop position and content size (logical pixels) after
    /// the user moves or resizes it.
    pub fn on_geometry(&self, f: impl Fn(Point, Size) + 'static) {
        self.app.0.borrow_mut().on_geometry.insert(self.key, Rc::new(f));
    }

    /// Runs `f` once when the window is gone, however it closed.
    pub fn on_closed(&self, f: impl FnOnce() + 'static) {
        self.app.0.borrow_mut().on_closed.entry(self.key).or_default().push(Box::new(f));
    }

    /// The renderer drawing the window now (`None` before its first frame).
    pub fn renderer(&self) -> Option<RendererKind> {
        self.app.0.borrow().renderers.get(&self.key).copied()
    }

    /// Opens the devtools inspector for this window (F12 does the same), or focuses it.
    #[cfg(feature = "devtools")]
    pub fn open_inspector(&self) -> Option<Window> {
        let existing = self.app.0.borrow().inspectors.get(&self.key).cloned();
        if let Some(w) = existing.filter(Window::is_open) {
            w.focus();
            return Some(w);
        }
        let target = self.ui()?;
        let sheet = self.app.0.borrow().sheet.clone()?;
        let insp = remedy_ui_devtools::Inspector::attach(&target, move |t| sheet(t));
        let i2 = insp.clone();
        let title = match self.title() {
            t if t.is_empty() => "Inspector".to_owned(),
            t => format!("Inspector · {t}"),
        };
        let win = self.app.open(WindowSpec::new(title).size(480.0, 760.0), move |ui, _| {
            let (theme, sheet) = remedy_ui_devtools::tool_style();
            ui.set_theme(theme, sheet);
            ui.root().child(remedy_ui_devtools::inspector_panel(ui, &i2));
        });
        let (app, key) = (self.app.clone(), self.key);
        win.on_closed(move || {
            insp.detach();
            app.0.borrow_mut().inspectors.remove(&key);
        });
        self.app.0.borrow_mut().inspectors.insert(self.key, win.clone());
        Some(win)
    }

    /// Keeps the window out of screenshots and screen sharing, or not.
    pub fn set_capture_protected(&self, on: bool) {
        self.app.push(Cmd::Capture(self.key, on));
    }
}

enum Present {
    Cpu(Box<CpuCompositor>),
    #[cfg(feature = "gpu")]
    Gpu(Box<remedy_ui_render_gpu::GpuSurface>),
}

struct Win {
    host: WindowId,
    ui: Ui,
    comp: Option<Compositor>,
    size: (u32, u32, f32),
    spec: WindowSpec,
    present: Option<Present>,
    /// The CPU renderer has been too slow for this window: use the GPU from now on.
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    wants_gpu: bool,
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    cost: FrameCost,
    scrollers: Vec<(LayerId, ElementId)>,
    cover: Option<remedy_ui_view::El>,
}

/// Drives an app on a host. [`run`](crate::run) uses it with the platform bridge; tests use
/// it with the headless bridge.
pub struct Runner {
    app: App,
    config: AppConfig,
    setup: Option<Setup>,
    wins: HashMap<u64, Win>,
    by_host: HashMap<WindowId, u64>,
    timers: HashMap<TimerId, Box<dyn FnOnce()>>,
    /// Host timers standing for view timers: (window, view timer id).
    view_timers: HashMap<TimerId, (u64, u64)>,
    geometry: GeometryStore,
    save_timer: Option<TimerId>,
    settings: SystemSettings,
    /// The app's own locale, in place of the system's.
    locale: Option<String>,
    idle: Option<(remedy_ui_secure::IdleTimer, Rc<dyn Fn()>)>,
    activity_reported: Option<Duration>,
    idle_timer: Option<TimerId>,
    /// Fires at the next local midnight, so dates roll over in idle windows.
    midnight_timer: Option<TimerId>,
    secret: Option<(remedy_ui_secure::SensitiveCopy, TimerId)>,
    inbox: crate::instance::SharedInbox,
    #[cfg(feature = "gpu")]
    gpu: Option<Option<remedy_ui_render_gpu::GpuCompositor>>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Runner({:?}, {} windows)", self.config, self.wins.len())
    }
}

impl Runner {
    /// A runner for `config`; `setup` runs when the host is ready.
    pub fn new(config: AppConfig, setup: impl FnOnce(&App) + 'static) -> Self {
        let geometry = data_dir(&config.name)
            .map_or_else(GeometryStore::in_memory, |d| GeometryStore::open(d.join("windows.txt")));
        Self::with_geometry(config, geometry, setup)
    }

    /// A runner keeping geometry in `geometry` (tests use an in-memory store).
    pub fn with_geometry(
        config: AppConfig,
        geometry: GeometryStore,
        setup: impl FnOnce(&App) + 'static,
    ) -> Self {
        let shared =
            Shared { name: config.name.clone(), sheet: Some(config.sheet.clone()), ..Shared::default() };
        Runner {
            app: App(Rc::new(RefCell::new(shared))),
            config,
            setup: Some(Box::new(setup)),
            wins: HashMap::new(),
            by_host: HashMap::new(),
            timers: HashMap::new(),
            view_timers: HashMap::new(),
            geometry,
            save_timer: None,
            settings: SystemSettings::default(),
            locale: None,
            idle: None,
            activity_reported: None,
            idle_timer: None,
            midnight_timer: None,
            secret: None,
            inbox: Default::default(),
            #[cfg(feature = "gpu")]
            gpu: None,
        }
    }

    pub(crate) fn with_inbox(mut self, inbox: crate::instance::SharedInbox) -> Self {
        self.inbox = inbox;
        self
    }

    fn launched(&mut self, args: Vec<String>) {
        let cb = self.app.0.borrow().on_launch.clone();
        if let Some(cb) = cb {
            cb(args);
        }
    }

    /// The app handle (tests inspect windows through it).
    pub fn app(&self) -> &App {
        &self.app
    }

    /// The saved geometry.
    pub fn geometry(&self) -> &GeometryStore {
        &self.geometry
    }

    fn drain(&mut self, host: &mut dyn Host) {
        loop {
            let Some(cmd) = self.app.0.borrow_mut().cmds.pop_front() else { break };
            self.command(host, cmd);
        }
        let keys: Vec<u64> = self.wins.keys().copied().collect();
        for k in keys {
            self.requests(host, k);
        }
    }

    fn host_of(&self, key: u64) -> Option<WindowId> {
        self.wins.get(&key).map(|w| w.host)
    }

    fn command(&mut self, host: &mut dyn Host, cmd: Cmd) {
        match cmd {
            Cmd::Open(key, mut spec, build) => {
                if let Some((p, s)) = spec.persist.as_deref().and_then(|k| self.geometry.get(k)) {
                    spec.options.position = Some(p);
                    spec.options.size = s;
                }
                let id = match host.create_window(&spec.options) {
                    Ok(id) => id,
                    Err(e) => {
                        let mut s = self.app.0.borrow_mut();
                        s.open.remove(&key);
                        s.open_errors.insert(key, e.to_string());
                        return;
                    }
                };
                // Capture protection is a request: ask again to learn whether it holds here.
                if spec.options.capture_protected && host.set_capture_protected(id, true).is_ok() {
                    self.app.0.borrow_mut().protected.insert(key);
                }
                let ui = Ui::new(
                    self.config.theme.clone(),
                    (self.config.sheet)(&self.config.theme),
                    (self.config.fonts)(),
                );
                ui.set_platform(self.settings.platform);
                ui.set_locale(self.locale.clone().unwrap_or_else(|| self.settings.locale.clone()));
                ui.set_double_click_time(self.settings.double_click);
                ui.set_local_time(host.local_time());
                let app = self.app.clone();
                ui.set_wake(Rc::new(move || app.wake_if_idle()));
                ui.set_reduced_motion(self.settings.reduced_motion);
                self.app.0.borrow_mut().uis.insert(key, ui.clone());
                self.by_host.insert(id, key);
                self.wins.insert(
                    key,
                    Win {
                        host: id,
                        ui: ui.clone(),
                        comp: None,
                        size: (1, 1, 1.0),
                        spec,
                        present: None,
                        wants_gpu: false,
                        cost: FrameCost::default(),
                        scrollers: Vec::new(),
                        cover: None,
                    },
                );
                build(&ui, &Window { key, app: self.app.clone() });
            }
            Cmd::Close(key) => {
                if let Some(h) = self.host_of(key) {
                    host.close_window(h);
                }
            }
            Cmd::Visible(key, on) => {
                if let Some(h) = self.host_of(key) {
                    host.set_visible(h, on);
                    if on {
                        host.request_frame(h);
                    }
                }
            }
            Cmd::Focus(key) => {
                if let Some(h) = self.host_of(key) {
                    host.set_minimized(h, false);
                    host.focus_window(h);
                }
            }
            Cmd::OnTop(key, on) => {
                if let Some(h) = self.host_of(key) {
                    let _ = host.set_always_on_top(h, on);
                }
            }
            Cmd::Title(key, t) => {
                if let Some(h) = self.host_of(key) {
                    host.set_title(h, &t);
                }
            }
            Cmd::BeginMove(key) => {
                if let Some(h) = self.host_of(key) {
                    let _ = host.begin_move(h);
                }
            }
            Cmd::BeginResize(key, e) => {
                if let Some(h) = self.host_of(key) {
                    let _ = host.begin_resize(h, e);
                }
            }
            Cmd::Minimize(key, on) => {
                if let Some(h) = self.host_of(key) {
                    host.set_minimized(h, on);
                }
            }
            Cmd::Maximize(key, on) => {
                if let Some(h) = self.host_of(key) {
                    host.set_maximized(h, on);
                    self.sync_maximized(host, key);
                }
            }
            Cmd::Capture(key, on) => {
                if let Some(h) = self.host_of(key) {
                    let holds = host.set_capture_protected(h, on).is_ok() && on;
                    let mut s = self.app.0.borrow_mut();
                    if holds {
                        s.protected.insert(key);
                    } else {
                        s.protected.remove(&key);
                    }
                }
            }
            Cmd::Timer(delay, f) => {
                let id = host.set_timer(host.now() + delay);
                self.timers.insert(id, f);
            }
            Cmd::Copy(text, sensitive) => {
                let _ = host.set_clipboard_text(&text, sensitive);
            }
            Cmd::CopySecret(text, keep) => {
                if host.set_clipboard_text(&text, true).is_ok() {
                    let copy = remedy_ui_secure::SensitiveCopy::new(&text, host.now(), keep);
                    if let Some((_, old)) = self.secret.take() {
                        host.cancel_timer(old);
                    }
                    let t = host.set_timer(copy.clear_at());
                    self.secret = Some((copy, t));
                }
            }
            Cmd::Locale(locale) => {
                self.locale = locale;
                let tag = self.locale.clone().unwrap_or_else(|| self.settings.locale.clone());
                for w in self.wins.values() {
                    w.ui.set_locale(tag.clone());
                    host.request_frame(w.host);
                }
                self.publish_locale();
            }
            Cmd::Theme(theme) => {
                let theme = *theme;
                let sheet = (self.config.sheet)(&theme);
                for w in self.wins.values() {
                    w.ui.set_theme(theme.clone(), sheet.clone());
                    host.request_frame(w.host);
                }
                self.config.theme = theme;
            }
            Cmd::ClearSecret => {
                if let Some((copy, t)) = self.secret.take() {
                    host.cancel_timer(t);
                    if let Ok(Some(current)) = host.clipboard_text()
                        && copy.still_there(current)
                    {
                        let _ = host.clear_clipboard();
                    }
                }
            }
            Cmd::Idle(timeout, cb) => {
                let now = host.now();
                match (&mut self.idle, cb) {
                    (Some((timer, _)), None) => timer.set_timeout(timeout),
                    (slot, Some(cb)) => *slot = Some((remedy_ui_secure::IdleTimer::new(timeout, now), cb)),
                    (None, None) => {}
                }
                self.schedule_idle(host);
            }
            Cmd::Tray(t) => {
                let _ = host.set_tray(t.as_ref());
            }
            Cmd::Dialog(key, dialog, done) => {
                let w = key.and_then(|k| self.host_of(k));
                let chosen = host.file_dialog(w, &dialog).unwrap_or_default();
                done(chosen);
            }
            Cmd::OpenUrl(url, done) => done(host.open_url(&url).map_err(|e| e.to_string())),
            Cmd::Forget(key) => {
                self.geometry.remove(&key);
                self.schedule_save(host);
            }
            Cmd::Quit => {
                let _ = self.geometry.save();
                host.exit();
            }
        }
    }

    fn requests(&mut self, host: &mut dyn Host, key: u64) {
        let Some(w) = self.wins.get(&key) else { return };
        let (h, ui) = (w.host, w.ui.clone());
        for r in ui.take_requests() {
            match r {
                Request::Frame => host.request_frame(h),
                Request::TextInput(s) => host.set_text_input(h, s),
                Request::ScrollTo { scroller, offset } => {
                    if let Some(w) = self.wins.get_mut(&key) {
                        let layer = w.scrollers.iter().find(|(_, e)| *e == scroller).map(|(l, _)| *l);
                        if let (Some(l), Some(c)) = (layer, w.comp.as_mut())
                            && c.set_scroll_offset(l, offset)
                        {
                            host.request_frame(h);
                        }
                    }
                }
                Request::StartDrag { data, allowed } => {
                    match host.start_drag(h, &data, &allowed) {
                        Ok(remedy_ui_host::DragStarted::Finished(effect)) => ui.drag_finished(effect),
                        // The host reports the end with `HostEvent::DragEnded`.
                        Ok(remedy_ui_host::DragStarted::Pending) => {}
                        Err(_) => ui.drag_finished(remedy_ui_host::DropEffect::None),
                    }
                    host.request_frame(h);
                }
                Request::Copy { text, sensitive } => {
                    let _ = host.set_clipboard_text(&text, sensitive);
                }
                Request::Paste => {
                    if let Ok(Some(t)) = host.clipboard_text() {
                        ui.paste(&t);
                        host.request_frame(h);
                    }
                }
                Request::Timer { id, delay } => {
                    let t = host.set_timer(host.now() + delay);
                    self.view_timers.insert(t, (key, id));
                }
                Request::CancelTimer(id) => {
                    let found = self.view_timers.iter().find(|(_, v)| **v == (key, id)).map(|(t, _)| *t);
                    if let Some(t) = found {
                        host.cancel_timer(t);
                        self.view_timers.remove(&t);
                    }
                }
            }
        }
    }

    fn schedule_idle(&mut self, host: &mut dyn Host) {
        if let Some(t) = self.idle_timer.take() {
            host.cancel_timer(t);
        }
        if let Some(d) = self.idle.as_ref().and_then(|(t, _)| t.deadline()) {
            self.idle_timer = Some(host.set_timer(d));
        }
    }

    /// Real user input: the only thing that keeps an idle lock away.
    fn activity(&mut self, host: &dyn Host) {
        let now = host.now();
        if let Some((t, _)) = self.idle.as_mut() {
            t.input(now);
        }
        // At most once a second: apps mirror activity into their own clocks with it.
        if self.activity_reported.is_none_or(|t| now.saturating_sub(t) >= Duration::from_secs(1)) {
            self.activity_reported = Some(now);
            let cb = self.app.0.borrow().on_activity.clone();
            if let Some(cb) = cb {
                cb();
            }
        }
    }

    fn schedule_save(&mut self, host: &mut dyn Host) {
        if let Some(t) = self.save_timer.take() {
            host.cancel_timer(t);
        }
        self.save_timer = Some(host.set_timer(host.now() + GEOMETRY_SAVE_DELAY));
    }

    /// Reads whether the window is maximized from the host into its signal.
    /// Tells the app its language when it changed (its own, or the system's).
    fn publish_locale(&self) {
        let tag = self.locale.clone().unwrap_or_else(|| self.settings.locale.clone());
        let notify = {
            let mut shared = self.app.0.borrow_mut();
            if shared.locale.as_deref() == Some(tag.as_str()) {
                None
            } else {
                shared.locale = Some(tag.clone());
                shared.on_locale.clone()
            }
        };
        if let Some(f) = notify {
            f(&tag);
        }
    }

    fn sync_maximized(&mut self, host: &dyn Host, key: u64) {
        let Some(h) = self.host_of(key) else { return };
        let on = host.is_maximized(h);
        let s = self.app.0.borrow().maximized.get(&key).copied();
        if let Some(s) = s {
            s.set_if_changed(on);
        }
    }

    fn remember_geometry(&mut self, host: &mut dyn Host, key: u64) {
        let Some(w) = self.wins.get(&key) else { return };
        let cb = self.app.0.borrow().on_geometry.get(&key).cloned();
        if let Some(cb) = cb {
            let (pw, ph, s) = w.size;
            let pos = host.window_position(w.host).unwrap_or(Point::ZERO);
            cb(pos, Size::new(pw as f32 / s, ph as f32 / s));
        }
        let Some(w) = self.wins.get(&key) else { return };
        let Some(k) = w.spec.persist.clone() else { return };
        let (pw, ph, s) = w.size;
        let size = Size::new(pw as f32 / s, ph as f32 / s);
        let pos = host.window_position(w.host).unwrap_or(Point::ZERO);
        self.geometry.set(&k, pos, size);
        self.schedule_save(host);
    }

    fn frame(&mut self, host: &mut dyn Host, key: u64, at: Duration) {
        let bg = self.config.theme.colors.bg;
        let local = host.local_time();
        if let Some(w) = self.wins.get(&key) {
            w.ui.set_local_time(local);
        }
        if self.midnight_timer.is_none() {
            self.midnight_timer =
                Some(host.set_timer(host.now() + Duration::from_secs(local.until_midnight())));
        }
        #[cfg(feature = "gpu")]
        let renderer = self.config.renderer;
        let Some(w) = self.wins.get_mut(&key) else { return };
        let (pw, ph, s) = w.size;
        let f = w.ui.frame(Size::new(pw as f32 / s, ph as f32 / s), s);
        if let Some(update) = w.ui.take_accessibility_update() {
            host.update_accessibility(w.host, update);
        }
        let Some(comp) = w.comp.as_mut() else { return };
        if let Some(l) = f.layers {
            comp.commit(l);
        }
        w.scrollers = f.scrollers;
        let plan = comp.frame(at);
        for (layer, el) in &w.scrollers {
            if let Some(o) = comp.scroll_offset(*layer) {
                w.ui.set_scroll_offset(*el, o);
            }
        }
        let background =
            if w.spec.options.transparent { Color::TRANSPARENT } else { w.spec.background.unwrap_or(bg) };
        let changed = !plan.damage.is_empty() || !plan.raster.is_empty() || !plan.released.is_empty();
        if w.present.is_none() {
            #[cfg(feature = "gpu")]
            let gpu_wanted = match renderer {
                RendererChoice::Gpu => true,
                RendererChoice::Auto => w.wants_gpu,
                RendererChoice::Cpu => false,
            };
            #[cfg(feature = "gpu")]
            if gpu_wanted && let Some(gpu) = self.gpu.get_or_insert_with(new_gpu).as_ref() {
                let surface = host
                    .window_handles(w.host)
                    .ok()
                    .and_then(|hd| gpu.create_surface(hd, pw, ph, w.spec.options.transparent).ok());
                if let Some(sf) = surface {
                    w.present = Some(Present::Gpu(Box::new(sf)));
                }
            }
            if w.present.is_none() {
                w.present = Some(Present::Cpu(Box::new(CpuCompositor::new())));
            }
            let kind = match w.present.as_ref() {
                #[cfg(feature = "gpu")]
                Some(Present::Gpu(_)) => RendererKind::Gpu,
                _ => RendererKind::Cpu,
            };
            self.app.0.borrow_mut().renderers.insert(key, kind);
        }
        let Some(w) = self.wins.get_mut(&key) else { return };
        #[cfg_attr(not(feature = "gpu"), allow(unused_mut))]
        let mut again = plan.animating;
        if changed || again {
            match w.present.as_mut() {
                Some(Present::Cpu(cpu)) => {
                    let started = std::time::Instant::now();
                    if changed && let Ok(frame) = cpu.present(&plan, pw, ph, background) {
                        let pixels = Pixels { width: pw, height: ph, data: &frame.pixels };
                        let _ = host.present_pixels(w.host, pixels, plan.damage.rects());
                    }
                    // Too slow on the CPU: the next frame starts this window on the GPU.
                    #[cfg(feature = "gpu")]
                    if changed
                        && self.config.renderer == RendererChoice::Auto
                        && !w.wants_gpu
                        && w.cost.record(started.elapsed(), GPU_PROMOTE_FRAME_COST)
                    {
                        w.wants_gpu = true;
                        w.present = None;
                        if let Some(c) = w.comp.as_mut() {
                            c.invalidate_tiles();
                        }
                        again = true;
                    }
                    #[cfg(not(feature = "gpu"))]
                    let _ = started;
                }
                #[cfg(feature = "gpu")]
                Some(Present::Gpu(surface)) => {
                    if let Some(Some(gpu)) = self.gpu.as_mut() {
                        if gpu.is_lost() {
                            // A lost device: rebuild it, and every window re-rasters.
                            if gpu.recover_device().is_ok() {
                                for other in self.wins.values_mut() {
                                    if let Some(c) = other.comp.as_mut() {
                                        c.invalidate_tiles();
                                    }
                                    host.request_frame(other.host);
                                }
                            }
                            return;
                        }
                        match gpu.present_to_surface(&plan, surface, background) {
                            Ok(remedy_ui_render_gpu::Presented::Retry) => again = true,
                            Ok(_) => {}
                            Err(_) => {
                                // The GPU failed this window: fall back to the CPU renderer.
                                w.present = Some(Present::Cpu(Box::new(CpuCompositor::new())));
                                self.app.0.borrow_mut().renderers.insert(key, RendererKind::Cpu);
                                if let Some(c) = w.comp.as_mut() {
                                    c.invalidate_tiles();
                                }
                                again = true;
                            }
                        }
                    }
                }
                None => {}
            }
        }
        if again && let Some(w) = self.wins.get(&key) {
            host.request_frame(w.host);
        }
    }
}

/// AccessKit's action as the engine's, for the actions elements offer.
fn action(a: remedy_ui_host::accesskit::Action) -> Option<remedy_ui_view::A11yAction> {
    use remedy_ui_host::accesskit::Action as K;
    use remedy_ui_view::A11yAction as A;
    Some(match a {
        K::Click => A::Click,
        K::Focus => A::Focus,
        K::Increment => A::Increment,
        K::Decrement => A::Decrement,
        K::Expand => A::Expand,
        K::Collapse => A::Collapse,
        K::ShowContextMenu => A::ShowContextMenu,
        K::ScrollIntoView => A::ScrollIntoView,
        _ => return None,
    })
}

#[cfg(feature = "gpu")]
fn new_gpu() -> Option<remedy_ui_render_gpu::GpuCompositor> {
    let ladder = remedy_ui_render_gpu::LadderOptions::default();
    let ctx = remedy_ui_render_gpu::GpuContext::new(&ladder).ok()?;
    Some(remedy_ui_render_gpu::GpuCompositor::new(remedy_ui_render_gpu::GpuRenderer::new(ctx, None), ladder))
}

impl remedy_ui_host::App for Runner {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        {
            let mut s = self.app.0.borrow_mut();
            s.in_event = true;
            if s.waker.is_none() {
                let w = host.waker();
                s.waker = Some(w.clone());
                // Work other threads post to the UI thread (decoded images, finished sorts,
                // reloaded messages) wakes the loop, and runs below.
                let _ = remedy_ui_reactive::set_wake_handler(move || w());
            }
        }
        {
            let mut inbox = self.inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if inbox.waker.is_none() {
                inbox.waker = Some(host.waker());
            }
        }
        // Background work that finished.
        self.app.finish_background();
        // Closures other threads posted through the reactive waker.
        if remedy_ui_reactive::has_posted() {
            let _ = remedy_ui_reactive::run_posted();
        }
        // Launches handed over by later instances.
        let launches = std::mem::take(
            &mut self.inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).launches,
        );
        for args in launches {
            self.launched(args);
        }
        match event {
            HostEvent::Resumed => {
                if let Some(setup) = self.setup.take() {
                    setup(&self.app);
                }
            }
            HostEvent::Settings(s) => {
                for w in self.wins.values() {
                    w.ui.set_platform(s.platform);
                    w.ui.set_locale(self.locale.clone().unwrap_or_else(|| s.locale.clone()));
                    w.ui.set_double_click_time(s.double_click);
                    w.ui.set_reduced_motion(s.reduced_motion);
                }
                for w in self.wins.values_mut() {
                    if let Some(c) = w.comp.as_mut() {
                        c.set_reduced_motion(s.reduced_motion);
                    }
                }
                let appearance = (s.dark, s.high_contrast);
                self.settings = s;
                let notify = {
                    let mut shared = self.app.0.borrow_mut();
                    let changed = shared.appearance != Some(appearance);
                    shared.appearance = Some(appearance);
                    changed.then(|| shared.on_appearance.clone()).flatten()
                };
                if let Some(f) = notify {
                    f(appearance.0, appearance.1);
                }
                self.publish_locale();
            }
            HostEvent::Resized { window, width, height, scale } => {
                let Some(&key) = self.by_host.get(&window) else { return };
                let reduced = self.settings.reduced_motion;
                let Some(w) = self.wins.get_mut(&key) else { return };
                w.size = (width, height, scale);
                match w.comp.as_mut() {
                    Some(c) => c.resize(width, height, scale),
                    None => {
                        let mut c = Compositor::new(width, height, scale, ScrollPhysics::CLAMPED);
                        c.set_reduced_motion(reduced);
                        w.comp = Some(c);
                    }
                }
                #[cfg(feature = "gpu")]
                if let (Some(Present::Gpu(sf)), Some(Some(gpu))) = (w.present.as_mut(), self.gpu.as_ref()) {
                    gpu.resize_surface(sf, width, height);
                }
                host.request_frame(window);
                self.sync_maximized(host, key);
                self.remember_geometry(host, key);
            }
            HostEvent::Moved(window, _) => {
                if let Some(&key) = self.by_host.get(&window) {
                    self.remember_geometry(host, key);
                }
            }
            HostEvent::Frame { window, at } => {
                if let Some(&key) = self.by_host.get(&window) {
                    self.frame(host, key, at);
                }
            }
            HostEvent::Pointer(window, e) => {
                self.activity(host);
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    w.ui.pointer(&e);
                }
            }
            HostEvent::DragEnded(window, effect) => {
                if let Some(ui) =
                    self.by_host.get(&window).and_then(|k| self.wins.get(k)).map(|w| w.ui.clone())
                {
                    ui.drag_finished(effect);
                    host.request_frame(window);
                }
            }
            HostEvent::Drag(window, e) => {
                self.activity(host);
                if let Some(key) = self.by_host.get(&window).copied()
                    && let Some(ui) = self.wins.get(&key).map(|w| w.ui.clone())
                {
                    let effect = ui.drag_event(&e);
                    host.set_drop_effect(window, effect);
                    self.requests(host, key);
                    host.request_frame(window);
                }
            }
            HostEvent::Wheel(window, e) => {
                self.activity(host);
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get_mut(k)) {
                    let s = w.size.2;
                    if let Some(c) = w.comp.as_mut() {
                        c.scroll_at(
                            Point::new(e.position.x * s, e.position.y * s),
                            Vec2::new(e.delta.x * s, e.delta.y * s),
                        );
                    }
                    host.request_frame(window);
                }
            }
            HostEvent::Key(window, e) => {
                self.activity(host);
                // F12 opens the inspector (devtools) instead of reaching the app.
                #[cfg(feature = "devtools")]
                let inspector = e.pressed
                    && e.key == remedy_ui_input::Key::F(12)
                    && e.modifiers == remedy_ui_input::Modifiers::NONE;
                #[cfg(not(feature = "devtools"))]
                let inspector = false;
                if inspector {
                    #[cfg(feature = "devtools")]
                    if let Some(&key) = self.by_host.get(&window) {
                        let _ = Window { key, app: self.app.clone() }.open_inspector();
                    }
                } else if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    w.ui.key(&e);
                }
            }
            HostEvent::Ime(window, e) => {
                self.activity(host);
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    w.ui.ime(&e);
                }
            }
            HostEvent::Focused(window, focused) => {
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get_mut(k)) {
                    if w.spec.privacy_cover {
                        match (focused, w.cover.take()) {
                            (true, Some(c)) => c.remove(),
                            (false, None) => {
                                let ui = w.ui.clone();
                                let cover = ui.el("privacy_cover").layout(|l| {
                                    l.position = remedy_ui_layout::Position::Absolute;
                                    l.inset = remedy_ui_core::Insets::ZERO;
                                    l.width = remedy_ui_layout::Length::Percent(1.0);
                                    l.height = remedy_ui_layout::Length::Percent(1.0);
                                });
                                let bg = w.spec.background.unwrap_or(self.config.theme.colors.bg);
                                let cover = cover.style(remedy_ui_style::StylePatch {
                                    background: Some(bg),
                                    ..Default::default()
                                });
                                w.cover = Some(ui.trusted_overlay(cover));
                            }
                            (_, keep) => w.cover = keep,
                        }
                    }
                    w.ui.request_frame();
                }
            }
            HostEvent::CloseRequested(window) => {
                let Some(&key) = self.by_host.get(&window) else { return };
                let hide = self.wins.get(&key).is_some_and(|w| w.spec.hide_on_close);
                if hide {
                    host.set_visible(window, false);
                    self.app.0.borrow_mut().visible.insert(key, false);
                } else {
                    host.close_window(window);
                }
            }
            HostEvent::Destroyed(window) => {
                if let Some(key) = self.by_host.remove(&window) {
                    self.wins.remove(&key);
                    let closed = {
                        let mut s = self.app.0.borrow_mut();
                        s.open.remove(&key);
                        s.uis.remove(&key);
                        s.visible.remove(&key);
                        s.on_geometry.remove(&key);
                        s.renderers.remove(&key);
                        s.titles.remove(&key);
                        s.maximized.remove(&key);
                        s.on_closed.remove(&key).unwrap_or_default()
                    };
                    for f in closed {
                        f();
                    }
                }
                let (none_left, keep) = {
                    let s = self.app.0.borrow();
                    (s.open.is_empty(), s.keep_running)
                };
                if none_left && !keep {
                    let _ = self.geometry.save();
                    host.exit();
                }
            }
            HostEvent::Timer(id) => {
                if self.save_timer == Some(id) {
                    self.save_timer = None;
                    let _ = self.geometry.save();
                } else if self.midnight_timer == Some(id) {
                    // A new day: every window's dates move on, and the next midnight is set.
                    let local = host.local_time();
                    for w in self.wins.values() {
                        w.ui.set_local_time(local);
                        host.request_frame(w.host);
                    }
                    self.midnight_timer =
                        Some(host.set_timer(host.now() + Duration::from_secs(local.until_midnight())));
                } else if self.idle_timer == Some(id) {
                    self.idle_timer = None;
                    let now = host.now();
                    let fire = self
                        .idle
                        .as_ref()
                        .is_some_and(|(t, _)| t.is_idle(now))
                        .then(|| self.idle.as_ref().map(|(_, cb)| cb.clone()))
                        .flatten();
                    if let Some(cb) = fire {
                        cb();
                        // Count the lock as a fresh start, so it fires once per idle stretch.
                        if let Some((t, _)) = self.idle.as_mut() {
                            t.input(now);
                        }
                    }
                    self.schedule_idle(host);
                } else if self.secret.as_ref().is_some_and(|(_, t)| *t == id) {
                    if let Some((copy, _)) = self.secret.take()
                        && let Ok(Some(current)) = host.clipboard_text()
                        && copy.still_there(current)
                    {
                        let _ = host.clear_clipboard();
                    }
                } else if let Some(f) = self.timers.remove(&id) {
                    f();
                } else if let Some((key, vid)) = self.view_timers.remove(&id)
                    && let Some(w) = self.wins.get(&key)
                {
                    w.ui.fire_timer(vid);
                }
            }
            HostEvent::Accessibility(window, ev) => {
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    match ev {
                        AccessibilityEvent::Requested => w.ui.connect_accessibility(),
                        AccessibilityEvent::Deactivated => w.ui.disconnect_accessibility(),
                        AccessibilityEvent::Action(r) => {
                            if let Some(a) = action(r.action) {
                                w.ui.accessibility_action(r.target_node.0, a);
                            }
                        }
                    }
                    host.request_frame(window);
                }
            }
            HostEvent::Tray(e) => {
                let cb = self.app.0.borrow().on_tray.clone();
                if let Some(cb) = cb {
                    cb(e);
                }
            }
            HostEvent::SecondInstance(args) => self.launched(args),
            HostEvent::MemoryPressure(_) => {
                for w in self.wins.values_mut() {
                    if let Some(c) = w.comp.as_mut() {
                        c.trim_memory();
                    }
                }
            }
            _ => {}
        }
        self.drain(host);
        self.app.0.borrow_mut().in_event = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_sustained_slow_average_asks_for_the_gpu() {
        let limit = Duration::from_millis(6);
        let mut c = FrameCost::default();
        // One slow frame among fast ones is not enough, nor is a short burst.
        assert!(!c.record(Duration::from_millis(40), limit));
        assert!(FrameCost::default().record(Duration::from_millis(60), limit), "a stall moves at once");
        for _ in 0..14 {
            assert!(!c.record(Duration::from_millis(1), limit));
        }
        assert!(!c.record(Duration::from_millis(1), limit), "16 frames averaging 3.4 ms");
        let mut slow = FrameCost::default();
        let asked: Vec<bool> = (0..16).map(|_| slow.record(Duration::from_millis(9), limit)).collect();
        assert_eq!(asked.iter().position(|a| *a), Some(15), "asks once 16 frames average over the limit");
    }
}
