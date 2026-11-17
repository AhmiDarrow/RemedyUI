//! The app runner: windows on a host, each with a view, a compositor and a presenter.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_compositor::{Compositor, LayerId};
use remedy_ui_core::{Color, Point, Size, Vec2};
use remedy_ui_host::{
    FileDialog, Host, HostEvent, Pixels, ResizeEdge, SystemSettings, TimerId, TrayEvent, TrayIcon, WindowId,
    WindowOptions,
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
    /// Try the GPU renderer first (with the `gpu` feature).
    pub gpu: bool,
    /// One instance per user session: a second launch hands its arguments to the first
    /// ([`App::on_second_instance`]) and exits.
    pub single_instance: bool,
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppConfig({}, theme {}, gpu {})", self.name, self.theme.name, self.gpu)
    }
}

impl AppConfig {
    /// An app named `name` with the Dark Forest theme, an empty style sheet, the bundled
    /// Remedy fonts, and the GPU renderer.
    pub fn new(name: impl Into<String>) -> Self {
        AppConfig {
            name: name.into(),
            theme: Theme::dark_forest(),
            sheet: Rc::new(|_| StyleSheet::new()),
            fonts: Rc::new(|| {
                let mut f = FontCollection::new();
                for face in remedy_ui_fonts::all() {
                    let _ = f.add_static(face.data);
                }
                f
            }),
            gpu: true,
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
        self.gpu = false;
        self
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
type Setup = Box<dyn FnOnce(&App)>;

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
    Capture(u64, bool),
    Timer(Duration, Box<dyn FnOnce()>),
    Copy(String, bool),
    CopySecret(String, Duration),
    Idle(Option<Duration>, Option<Rc<dyn Fn()>>),
    Forget(String),
    Tray(Option<TrayIcon>),
    Dialog(Option<u64>, FileDialog, Box<dyn FnOnce(Vec<std::path::PathBuf>)>),
    Quit,
}

#[derive(Default)]
struct Shared {
    cmds: VecDeque<Cmd>,
    next: u64,
    keep_running: bool,
    name: String,
    uis: HashMap<u64, Ui>,
    open: HashSet<u64>,
    visible: HashMap<u64, bool>,
    /// Wakes the host's loop for requests made between events.
    waker: Option<remedy_ui_host::Waker>,
    in_event: bool,
    on_tray: Option<Rc<dyn Fn(TrayEvent)>>,
    on_launch: Option<Rc<dyn Fn(Vec<String>)>>,
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

    /// Calls `on_idle` after `timeout` with no real user input (pointer, keys, wheel, input
    /// methods) in any window; `None` turns idle locking off. Timers and programmatic
    /// changes never count as activity.
    pub fn idle_lock(&self, timeout: Option<Duration>, on_idle: impl Fn() + 'static) {
        self.push(Cmd::Idle(timeout, Some(Rc::new(on_idle))));
    }

    /// Changes the idle timeout, keeping the callback.
    pub fn set_idle_timeout(&self, timeout: Option<Duration>) {
        self.push(Cmd::Idle(timeout, None));
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
        self.app.push(Cmd::Title(self.key, title.into()));
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

    /// Keeps the window out of screenshots and screen sharing, or not.
    pub fn set_capture_protected(&self, on: bool) {
        self.app.push(Cmd::Capture(self.key, on));
    }
}

enum Present {
    Cpu(CpuCompositor),
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
    idle: Option<(remedy_ui_secure::IdleTimer, Rc<dyn Fn()>)>,
    idle_timer: Option<TimerId>,
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
        let shared = Shared { name: config.name.clone(), ..Shared::default() };
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
            idle: None,
            idle_timer: None,
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
                let Ok(id) = host.create_window(&spec.options) else {
                    self.app.0.borrow_mut().open.remove(&key);
                    return;
                };
                let ui = Ui::new(
                    self.config.theme.clone(),
                    (self.config.sheet)(&self.config.theme),
                    (self.config.fonts)(),
                );
                ui.set_platform(self.settings.platform);
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
            Cmd::Capture(key, on) => {
                if let Some(h) = self.host_of(key) {
                    let _ = host.set_capture_protected(h, on);
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
        if let Some((t, _)) = self.idle.as_mut() {
            t.input(host.now());
        }
    }

    fn schedule_save(&mut self, host: &mut dyn Host) {
        if let Some(t) = self.save_timer.take() {
            host.cancel_timer(t);
        }
        self.save_timer = Some(host.set_timer(host.now() + GEOMETRY_SAVE_DELAY));
    }

    fn remember_geometry(&mut self, host: &mut dyn Host, key: u64) {
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
        #[cfg(feature = "gpu")]
        let gpu_wanted = self.config.gpu;
        let Some(w) = self.wins.get_mut(&key) else { return };
        let (pw, ph, s) = w.size;
        let f = w.ui.frame(Size::new(pw as f32 / s, ph as f32 / s), s);
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
                w.present = Some(Present::Cpu(CpuCompositor::new()));
            }
        }
        let Some(w) = self.wins.get_mut(&key) else { return };
        let mut again = plan.animating;
        if changed || again {
            match w.present.as_mut() {
                Some(Present::Cpu(cpu)) => {
                    if changed && let Ok(frame) = cpu.present(&plan, pw, ph, background) {
                        let pixels = Pixels { width: pw, height: ph, data: &frame.pixels };
                        let _ = host.present_pixels(w.host, pixels, plan.damage.rects());
                    }
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
                                w.present = Some(Present::Cpu(CpuCompositor::new()));
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
                s.waker = Some(host.waker());
            }
        }
        {
            let mut inbox = self.inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if inbox.waker.is_none() {
                inbox.waker = Some(host.waker());
            }
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
                    w.ui.set_reduced_motion(s.reduced_motion);
                }
                for w in self.wins.values_mut() {
                    if let Some(c) = w.comp.as_mut() {
                        c.set_reduced_motion(s.reduced_motion);
                    }
                }
                self.settings = s;
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
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
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
                    let mut s = self.app.0.borrow_mut();
                    s.open.remove(&key);
                    s.uis.remove(&key);
                    s.visible.remove(&key);
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
