//! The app runner: windows on a host, each with a view, a compositor and a presenter.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remedy_ui_core::host::{
    AccessibilityEvent, FileDialog, Host, HostEvent, Pixels, ResizeEdge, SystemSettings, TimerId, TrayEvent,
    TrayIcon, Unsupported, WindowIcon, WindowId, WindowOptions,
};
use remedy_ui_core::input::{
    Buttons, GestureSettings, PointerEvent, PointerKind, PointerPhase, ScrollPhysics, VelocityTracker,
};
use remedy_ui_core::style::{StyleSheet, Theme};
use remedy_ui_core::tree::ElementId;
use remedy_ui_core::{Color, Edition, Point, Size, Vec2};
use remedy_ui_engine::compositor::{Compositor, LayerId};
use remedy_ui_engine::render_cpu::CpuCompositor;
use remedy_ui_engine::view::{Request, Ui};
use remedy_ui_text::{FontCollection, FontFile};

use crate::geometry::{GeometryStore, data_dir};

/// How long window moves and resizes settle before geometry is written.
const GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(400);

/// The bundled faces, read once per process and shared by every window.
fn bundled_faces() -> &'static [FontFile] {
    static FACES: std::sync::OnceLock<Vec<FontFile>> = std::sync::OnceLock::new();
    FACES.get_or_init(|| {
        remedy_ui_fonts::all().into_iter().filter_map(|f| FontFile::new(f.data).ok()).collect()
    })
}

/// The Chinese and Japanese faces, unpacked from WOFF2 once per process (a quarter of a
/// second): on demand, when a window first shows such text, or ahead of time on a
/// background thread when the app starts in Chinese or Japanese ([`warm_fonts_for`]).
#[cfg(all(feature = "cjk-fonts", not(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))))]
fn cjk_faces() -> &'static [FontFile] {
    static FACES: std::sync::OnceLock<Vec<FontFile>> = std::sync::OnceLock::new();
    FACES.get_or_init(|| {
        remedy_ui_fonts_cjk::all().into_iter().filter_map(|f| FontFile::new(f.data).ok()).collect()
    })
}

/// In a browser page, the Chinese and Japanese faces are fetched from beside it the first
/// time such text shows (none until they arrive).
#[cfg(all(feature = "cjk-fonts", feature = "web", target_arch = "wasm32", target_os = "unknown"))]
fn cjk_faces() -> Vec<FontFile> {
    crate::web_fonts::faces(&remedy_ui_fonts_cjk::all())
}

/// Starts unpacking the faces `locale` needs on a background thread, so its first window
/// finds them ready.
fn warm_fonts_for(locale: &str) {
    #[cfg(feature = "cjk-fonts")]
    {
        let lang = locale.split(['-', '_']).next().unwrap_or_default();
        // A page starts fetching them; elsewhere they unpack on a background thread.
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        if matches!(lang, "zh" | "ja") {
            let _ = cjk_faces();
        }
        #[cfg(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")))]
        if matches!(lang, "zh" | "ja") && remedy_ui_core::exec::threads() {
            let _ = std::thread::Builder::new().name("remedy-fonts".into()).spawn(|| {
                let _ = cjk_faces();
            });
        }
    }
    #[cfg(not(feature = "cjk-fonts"))]
    let _ = locale;
}

/// The fonts every window starts with ([`AppConfig::fonts`] by default): the bundled faces,
/// with the script faces as fallbacks for their scripts (Arabic, Devanagari and Bengali;
/// Chinese and Japanese by language, loaded when text first needs them), and the system's
/// fonts after them with `system-fonts`. In a browser page the script faces are files beside
/// the page, each fetched the first time text needs it (`remedy_ui_fonts::Face::is_remote`);
/// the text that waited is drawn again once it arrives.
pub fn bundled_fonts() -> FontCollection {
    #[cfg(feature = "system-fonts")]
    let mut f = FontCollection::with_system_fonts();
    #[cfg(not(feature = "system-fonts"))]
    let mut f = FontCollection::new();
    for file in bundled_faces() {
        let _ = f.add_file(file);
    }
    let fallbacks = remedy_ui_fonts::script_fallbacks();
    for fb in &fallbacks {
        let _ = f.add_script_fallback(fb.script, fb.locale, fb.family);
    }
    // The script faces not built in (a browser page's), each fetched when its script shows.
    let remote: Vec<&'static remedy_ui_fonts::Face> =
        remedy_ui_fonts::all().into_iter().filter(|face| face.is_remote()).collect();
    let mut families: Vec<&'static str> = remote.iter().map(|face| face.family).collect();
    families.dedup();
    for family in families {
        let Some(needs) = remedy_ui_fonts::needs(family) else { continue };
        let group: Vec<&'static remedy_ui_fonts::Face> =
            remote.iter().copied().filter(|face| face.family == family).collect();
        f.add_on_demand(
            needs,
            move || on_demand_faces(&group),
            fallbacks.iter().filter(|fb| fb.family == family).map(|fb| (fb.script, fb.locale, fb.family)),
        );
    }
    #[cfg(feature = "cjk-fonts")]
    {
        let cjk = remedy_ui_fonts_cjk::script_fallbacks();
        f.add_on_demand(
            remedy_ui_fonts_cjk::needs,
            || cjk_faces().to_vec(),
            cjk.iter().map(|fb| (fb.script, fb.locale, fb.family)),
        );
    }
    f
}

/// Adds the device's interface font to `fonts` and makes `system-ui` draw in it.
fn use_system_ui(fonts: &mut FontCollection, families: &[String], files: &[FontFile]) {
    for file in files {
        let _ = fonts.add_file(file);
    }
    let names: Vec<&str> = families.iter().map(String::as_str).collect();
    let _ = fonts.set_generic_family("system-ui", &names);
}

/// Faces loaded on demand: in a browser page, fetched from beside it (none until they
/// arrive); elsewhere built in.
#[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
fn on_demand_faces(faces: &[&'static remedy_ui_fonts::Face]) -> Vec<FontFile> {
    crate::web_fonts::faces(faces)
}

#[cfg(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")))]
fn on_demand_faces(faces: &[&'static remedy_ui_fonts::Face]) -> Vec<FontFile> {
    faces.iter().filter_map(|face| FontFile::new(face.data).ok()).collect()
}

/// How long a later launch waits for the running instance's reply when
/// [`AppConfig::single_instance_await_exit`] turns replies on without a wait of its own.
pub const SINGLE_INSTANCE_REPLY: std::time::Duration = std::time::Duration::from_secs(30);

/// App-wide settings.
#[derive(Clone)]
#[non_exhaustive]
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
    /// GPU windows present in step with the display ([`AppConfig::vsync`]); on by default.
    pub vsync: bool,
    /// One instance per user session: a second launch hands its arguments to the first
    /// ([`App::on_second_instance`]) and exits.
    pub single_instance: bool,
    /// Where the app keeps its data, instead of the platform's place for it ([`App::data_dir`]):
    /// a portable app's folder beside its executable, or a test's temporary folder.
    pub data_dir: Option<PathBuf>,
    /// Which edition's default behaviors the app's windows follow ([`AppConfig::edition`]).
    pub edition: Edition,
    /// The app's reverse-domain id (`com.example.hearth`): its desktop entry's name on Linux,
    /// where docks look up its badge ([`AppConfig::app_id`]).
    pub app_id: Option<String>,
    /// In a browser page: where the first window mounts and whether its title is the page's
    /// ([`AppConfig::web`]). Not used on other platforms.
    #[cfg(feature = "web")]
    pub web: remedy_ui_bridge_web::WebConfig,
    /// How long quitting waits for keepalive requests still under way, an app's last saves
    /// (feature `net`; [`AppConfig::keepalive_grace`]). Default 10 s.
    pub keepalive_grace: std::time::Duration,
    /// How long a second launch keeps trying to reach the first (Windows;
    /// [`AppConfig::single_instance_wait`]). Default 5 s.
    pub single_instance_wait: std::time::Duration,
    /// A second launch waits this long for the running instance's reply and exits with it
    /// ([`AppConfig::single_instance_reply`]); `None` (the default) hands over and exits at once.
    pub single_instance_reply: Option<std::time::Duration>,
    /// A second launch that waits for a reply then also waits this long for the running
    /// instance's process to exit ([`AppConfig::single_instance_await_exit`]).
    pub single_instance_exit: Option<std::time::Duration>,
    /// How long a drop from another app may take to read, and a drag out of the app serves
    /// its data once released (Linux; [`AppConfig::drop_waits`]). Default 2 s and 10 s.
    pub drop_waits: (std::time::Duration, std::time::Duration),
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AppConfig({}, theme {}, {:?}, edition {})",
            self.name, self.theme.name, self.renderer, self.edition
        )
    }
}

impl AppConfig {
    /// An app named `name` with the bundled Remedy fonts and [`RendererChoice::Auto`],
    /// styled by the first Remedy palette and the widget sheet (feature `widgets`), or the
    /// Dark Forest theme with an empty sheet without it.
    pub fn new(name: impl Into<String>) -> Self {
        #[cfg(feature = "widgets")]
        let (theme, sheet): (Theme, SheetBuilder) = (
            remedy_ui_widgets::themes::PALETTES
                .first()
                .map_or_else(Theme::dark_forest, remedy_ui_widgets::themes::theme),
            Rc::new(remedy_ui_widgets::themes::sheet),
        );
        #[cfg(not(feature = "widgets"))]
        let (theme, sheet): (Theme, SheetBuilder) = (Theme::dark_forest(), Rc::new(|_| StyleSheet::new()));
        AppConfig {
            name: name.into(),
            theme,
            sheet,
            fonts: Rc::new(bundled_fonts),
            renderer: RendererChoice::Auto,
            vsync: true,
            single_instance: false,
            data_dir: None,
            edition: Edition::FIRST,
            app_id: None,
            #[cfg(feature = "web")]
            web: remedy_ui_bridge_web::WebConfig::default(),
            keepalive_grace: std::time::Duration::from_secs(10),
            single_instance_wait: std::time::Duration::from_secs(5),
            single_instance_reply: None,
            single_instance_exit: None,
            drop_waits: (std::time::Duration::from_secs(2), std::time::Duration::from_secs(10)),
        }
    }

    /// Sets how long quitting waits for keepalive requests still under way (an app's last
    /// saves, `net::Request::keepalive`) before the app ends anyway: 10 s by default, as a
    /// closing browser page lets them finish. On Android it is also how long the way out
    /// waits for them. Longer keeps a quitting app's process around that much longer when a
    /// server is slow; zero ends at once and may lose the saves.
    pub fn keepalive_grace(mut self, grace: std::time::Duration) -> Self {
        self.keepalive_grace = grace;
        self
    }

    /// Sets how long a second launch of a [single instance](Self::single_instance) app keeps
    /// trying to hand its arguments to the first, which may be starting up, before `run`
    /// fails with `RunError::SingleInstance` (Windows; 5 s by default). Elsewhere the hand-off
    /// does not wait.
    pub fn single_instance_wait(mut self, wait: std::time::Duration) -> Self {
        self.single_instance_wait = wait;
        self
    }

    /// Runs as a [single instance](Self::single_instance) whose later launches wait up to
    /// `timeout` for the running instance's reply (what its
    /// [`App::on_second_instance`] returns), then exit the process with that code, as a
    /// command-line tool asking the running app to do something would (an installer's
    /// `--stop`). A reply that does not come in time, or a running instance that ends
    /// without one, makes `run` fail with `RunError::SingleInstance` instead.
    ///
    /// ```
    /// use std::time::Duration;
    /// use remedy_ui::AppConfig;
    ///
    /// let config = AppConfig::new("Portal")
    ///     .single_instance_reply(Duration::from_secs(30))
    ///     .single_instance_await_exit(Duration::from_secs(30));
    /// assert!(config.single_instance);
    /// assert_eq!(config.single_instance_reply, Some(Duration::from_secs(30)));
    /// ```
    pub fn single_instance_reply(mut self, timeout: std::time::Duration) -> Self {
        self.single_instance = true;
        self.single_instance_reply = Some(timeout);
        self
    }

    /// After the reply ([`AppConfig::single_instance_reply`], which this turns on with a 30 s
    /// wait when it is not set), a later launch also waits up to `timeout` for the running
    /// instance's process to exit, so whoever started it knows the files are free (an
    /// installer replacing them). Past `timeout`, `run` fails with `RunError::SingleInstance`.
    pub fn single_instance_await_exit(mut self, timeout: std::time::Duration) -> Self {
        if self.single_instance_reply.is_none() {
            self = self.single_instance_reply(SINGLE_INSTANCE_REPLY);
        }
        self.single_instance_exit = Some(timeout);
        self
    }

    /// Sets how long a drop from another app may take to read (Wayland; 2 s by default) and
    /// how long a drag out of the app serves its data once released (X11; 10 s by default),
    /// on Linux. Each guards against an app that never answers: longer lets a slow app hand
    /// over a large drop, at a drop that waits that long when the other app hangs.
    pub fn drop_waits(mut self, read: std::time::Duration, finish: std::time::Duration) -> Self {
        self.drop_waits = (read, finish);
        self
    }

    /// Names the app by its reverse-domain id (`com.example.hearth`), the one its package
    /// and `remedy.toml` use. On Linux it is the desktop entry (`<id>.desktop`) docks show the
    /// app's badge on; without it the bridge uses the entry the desktop launched the app from.
    pub fn app_id(mut self, id: impl Into<String>) -> Self {
        self.app_id = Some(id.into());
        self
    }

    /// Follows `edition`'s default behaviors. An app that never calls this keeps
    /// [`Edition::FIRST`]'s, so a newer RemedyUI does not change how it behaves; moving to a
    /// newer edition is the app's choice, made after reading what the edition changes
    /// (`docs/STABILITY.md`). `cargo remedy new` writes the latest edition into new apps.
    ///
    /// ```
    /// use remedy_ui::{AppConfig, Edition};
    ///
    /// let config = AppConfig::new("Notes").edition(Edition::E2026);
    /// assert_eq!(config.edition, Edition::E2026);
    /// ```
    pub fn edition(mut self, edition: Edition) -> Self {
        self.edition = edition;
        self
    }

    /// In a browser page, shows the first window where `web` says (the whole page, or inside
    /// an element of the page's) and makes its title the page's or not. Other platforms
    /// ignore it, so the same configuration builds everywhere.
    ///
    /// ```
    /// use remedy_ui::AppConfig;
    /// use remedy_ui::web::{Mount, WebConfig};
    ///
    /// let config = AppConfig::new("Notes")
    ///     .web(WebConfig::default().mount(Mount::Element("notes-app".into())).title_is_page_title(false));
    /// assert_eq!(config.web.mount, Mount::Element("notes-app".into()));
    /// ```
    #[cfg(feature = "web")]
    pub fn web(mut self, web: remedy_ui_bridge_web::WebConfig) -> Self {
        self.web = web;
        self
    }

    /// Adds a font the app ships (its brand's typeface; TrueType, OpenType, WOFF or WOFF2) to
    /// every window's fonts, after the bundled Remedy families. Name its family first in the
    /// theme's [`font_families`](remedy_ui_core::style::Theme::font_families) to draw text in it.
    /// Several files of one family (weights, styles, or a web font's subsets per script) are
    /// added one call each and join as one family.
    ///
    /// ```
    /// use remedy_ui::AppConfig;
    /// use remedy_ui::text::FontFile;
    ///
    /// # fn brand() -> Result<AppConfig, remedy_ui::text::FontError> {
    /// # let font_bytes: &'static [u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../remedy-ui-text/tests/fixtures/inter/inter-latin-400-normal.woff2"));
    /// // let font_bytes = include_bytes!("../assets/brand-400.woff2");
    /// let regular = FontFile::new(font_bytes)?;
    /// let family = regular.families()[0].clone();
    /// let mut config = AppConfig::new("Notes").font(regular);
    /// config.theme.font_families.insert(0, family);
    /// # Ok(config)
    /// # }
    /// # assert!(brand().is_ok());
    /// ```
    pub fn font(mut self, file: FontFile) -> Self {
        let before = self.fonts.clone();
        self.fonts = Rc::new(move || {
            let mut fonts = before();
            // Checked when the file was read; adding it again cannot fail.
            let _ = fonts.add_file(&file);
            fonts
        });
        self
    }

    /// Uses `theme` styled by `sheet`.
    pub fn theme(mut self, theme: Theme, sheet: impl Fn(&Theme) -> StyleSheet + 'static) -> Self {
        self.theme = theme;
        self.sheet = Rc::new(sheet);
        self
    }

    /// Keeps the app's data in `dir` instead of the platform's place for it.
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
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

    /// Starts every window on the CPU renderer, even where the platform would start on the
    /// GPU (see [`RendererChoice::CpuFirst`]).
    pub fn cpu_first(mut self) -> Self {
        self.renderer = RendererChoice::CpuFirst;
        self
    }

    /// Starts every window on the GPU (games, canvases, video: apps that always need it).
    pub fn gpu_first(mut self) -> Self {
        self.renderer = RendererChoice::Gpu;
        self
    }

    /// Whether GPU windows wait for the display to show each frame (vertical sync, the
    /// default: no tearing) or show it as soon as it is drawn (lower latency).
    pub fn vsync(mut self, on: bool) -> Self {
        self.vsync = on;
        self
    }
}

/// A graphics adapter as people read it ([`App::graphics_adapter`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GraphicsAdapter {
    /// The device's name ("NVIDIA GeForce RTX 3080").
    pub name: String,
    /// The maker's PCI vendor id (0x10DE NVIDIA, 0x1002 AMD, 0x8086 Intel), 0 when unknown.
    pub vendor: u32,
    /// The driver's name and version, where the platform says.
    pub driver: String,
    /// The graphics API in use ("Vulkan", "Dx12", "Metal", "Gl").
    pub api: String,
    /// A software rasterizer rather than a graphics card.
    pub software: bool,
}

/// Which renderer draws an app's windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RendererChoice {
    /// Windows start on the CPU renderer, which costs no GPU driver memory, and move to the
    /// GPU (with the `gpu` feature) once their frames take longer than
    /// [`GPU_PROMOTE_FRAME_COST`] on average: large windows, heavy scenes, constant motion.
    /// Where a surface keeps its first renderer (Android), windows start on the GPU instead,
    /// since moving later needs a new surface.
    #[default]
    Auto,
    /// As [`RendererChoice::Auto`], but windows start on the CPU everywhere; where a surface
    /// keeps its first renderer, a window that needs the GPU moves with its next surface.
    CpuFirst,
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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct WindowSpec {
    /// Platform window options.
    pub options: WindowOptions,
    /// Closing hides the window instead (a manager that lives in the tray); where windows
    /// cannot hide ([`Window::can_hide`]: Wayland) it minimizes.
    pub hide_on_close: bool,
    /// Closing minimizes the window instead (an app that keeps running while its tray icon
    /// cannot be shown); wins over `hide_on_close`.
    pub minimize_on_close: bool,
    /// Saves and restores position and size under this key.
    pub persist: Option<String>,
    /// The color behind everything (the theme's background when `None`).
    pub background: Option<Color>,
    /// Covers the content while the window is not focused, so the task switcher and
    /// screen glances show nothing.
    pub privacy_cover: bool,
    /// Opened hidden, it stays unminimized where windows cannot hide (Wayland, where a
    /// window shows only once it first draws): the confirm dialog sizes itself first.
    pub(crate) hidden_until_drawn: bool,
}

impl WindowSpec {
    /// A window titled `title`.
    pub fn new(title: impl Into<String>) -> Self {
        WindowSpec {
            options: {
                let mut v_ = WindowOptions::default();
                v_.title = title.into();
                v_
            },
            hide_on_close: false,
            minimize_on_close: false,
            persist: None,
            background: None,
            privacy_cover: false,
            hidden_until_drawn: false,
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

    /// Whether showing the window gives it focus and brings it forward (the default); `false`
    /// shows it behind the active window without taking focus, as for a window opened by an
    /// automated test that must not interrupt the person at the machine.
    pub fn activate(mut self, on: bool) -> Self {
        self.options.activate = on;
        self
    }

    /// Whether the window has a taskbar button (the default); `false` keeps it off the
    /// taskbar and the window switcher.
    pub fn in_taskbar(mut self, on: bool) -> Self {
        self.options.taskbar = on;
        self
    }

    /// Closing hides the window (minimizes it where windows cannot hide: Wayland).
    pub fn hide_on_close(mut self) -> Self {
        self.hide_on_close = true;
        self
    }

    /// Closing minimizes the window (it keeps running in the taskbar).
    pub fn minimize_on_close(mut self) -> Self {
        self.minimize_on_close = true;
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

    /// Gives the window its own icon: what the taskbar, its title bar and window switchers
    /// show for it, instead of the app's (from the executable or the desktop entry). Shown on
    /// Windows, X11, Wayland compositors with the `xdg-toplevel-icon` protocol, and as the
    /// page's icon (or in a floating window's title bar) in a browser; macOS and Android show
    /// the app's icon for every window. Artwork drawn at several sizes gives them all
    /// ([`WindowIcon::with_size`]), and each platform shows the drawing of the size it needs.
    pub fn icon(mut self, icon: WindowIcon) -> Self {
        self.options.icon = Some(icon);
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

/// A signal the app or a window holds for as long as it lives (whether a tray icon is up,
/// whether a window is maximized), made in a scope of its own: whatever reactive owner is
/// current where it is first asked for (a window's build, a binding) must not dispose it
/// when that owner runs again or ends.
fn app_signal<T: 'static>(value: T) -> remedy_ui_core::reactive::Signal<T> {
    remedy_ui_core::reactive::Scope::detached(|| remedy_ui_core::reactive::Signal::new(value)).0
}
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
    waker: Option<remedy_ui_core::host::Waker>,
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

/// Work [`App::background`] started: cancel it when its result is no longer wanted.
#[derive(Debug)]
pub struct BackgroundJob {
    id: u64,
    app: std::rc::Weak<RefCell<Shared>>,
    cancelled: Cancelled,
}

impl BackgroundJob {
    /// Stops waiting for the job: its `done` never runs, and work started with
    /// [`App::background_cancellable`] sees [`Cancelled::is_cancelled`] turn true.
    pub fn cancel(&self) {
        self.cancelled.0.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(app) = self.app.upgrade() {
            app.borrow_mut().jobs.remove(&self.id);
        }
    }

    /// True once [`BackgroundJob::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.is_cancelled()
    }
}

/// Whether a background job was cancelled, for its work to check between steps.
#[derive(Clone, Debug)]
pub struct Cancelled(Arc<std::sync::atomic::AtomicBool>);

impl Cancelled {
    /// True once the job's [`BackgroundJob::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

enum Cmd {
    Open(u64, WindowSpec, Build),
    Close(u64),
    QuitNow,
    Visible(u64, bool),
    Focus(u64),
    OnTop(u64, bool),
    Title(u64, String),
    Size(u64, Size),
    Center(u64),
    BeginMove(u64),
    BeginResize(u64, ResizeEdge),
    Minimize(u64, bool),
    Maximize(u64, bool),
    Fullscreen(u64, bool),
    Zoom(u64, f32),
    HideOnClose(u64, bool),
    ReduceMotion,
    MinimizeOnClose(u64, bool),
    Capture(u64, bool),
    Icon(u64, Option<WindowIcon>),
    Timer(Duration, Box<dyn FnOnce()>),
    Copy(String, bool),
    CopySecret(String, Duration),
    ClearSecret,
    Theme(Box<Theme>),
    Locale(Option<String>),
    Direction(remedy_ui_engine::view::WindowDirection),
    Idle(Option<Duration>, Option<Rc<dyn Fn()>>),
    Forget(String),
    Tray(Option<TrayIcon>),
    Dialog(Option<u64>, FileDialog, DialogDone),
    OpenUrl(String, Box<dyn FnOnce(Result<(), Unsupported>)>),
    Permission(remedy_ui_core::host::Permission, PermissionDone),
    Notify(Box<remedy_ui_core::host::Notification>, Box<dyn FnOnce(Result<(), Unsupported>)>),
    Withdraw(u64),
    Badge(Option<remedy_ui_core::host::Badge>),
    Gamepad(remedy_ui_core::input::GamepadTuning),
    Foreground(Box<remedy_ui_core::host::ForegroundTask>, Box<dyn FnOnce(Result<(), Unsupported>)>),
    StopForeground,
    Share(Option<u64>, remedy_ui_core::host::ShareOptions, ShareDone),
    Print(Option<u64>, String, String, Box<dyn FnOnce(Result<(), Unsupported>)>),
    Quit,
}

/// Builds a style sheet for a theme.
type SheetBuilder = Rc<dyn Fn(&Theme) -> StyleSheet>;
/// What a [`Window::snapshot`] calls with its picture.
type SnapshotDone = Box<dyn FnOnce(Option<remedy_ui_engine::paint::Image>)>;
/// Input handed to a live window ([`Window::send_key`], [`Window::send_pointer`]), made into
/// the host's event once the window's host id is known.
type Injected = Box<dyn FnOnce(WindowId) -> HostEvent>;

#[derive(Default)]
struct Shared {
    cmds: VecDeque<Cmd>,
    /// The device's interface font, once read ([`App::load_system_ui_font`]): its family
    /// names and files, given to every window opened after.
    system_ui: Option<(Vec<String>, Vec<FontFile>)>,
    /// The app's style sheet builder (the inspector restyles edited themes with it).
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    sheet: Option<SheetBuilder>,
    /// Open inspector windows by the window they inspect.
    #[cfg(feature = "devtools")]
    inspectors: HashMap<u64, Window>,
    next: u64,
    keep_running: bool,
    /// At most this many frames a second per window ([`App::set_frame_rate_limit`]).
    frame_limit: Option<u32>,
    /// The GPU adapter, once made ([`App::graphics_adapter`]).
    adapter: Option<GraphicsAdapter>,
    /// The app reduces motion whatever the system says ([`App::set_reduce_motion`]).
    reduce_motion: bool,
    /// Every window's ambient-motion budget ([`App::set_ambient_policy`]); `None`: the default.
    ambient: Option<remedy_ui_engine::view::AmbientPolicy>,
    /// A tray icon is shown now: one was asked for and the platform took it.
    /// (Made when first asked for.)
    tray_up: Option<remedy_ui_core::reactive::Signal<bool>>,
    name: String,
    /// The data folder the app config chose, if it chose one.
    data_dir: Option<PathBuf>,
    /// Persisted signals' folder (`<data>/state`), opened when first needed.
    persist: Option<remedy_ui_core::reactive::PersistStore>,
    uis: HashMap<u64, Ui>,
    open: HashSet<u64>,
    /// Windows kept out of screen capture (where the system could do it).
    protected: HashSet<u64>,
    /// Why a window could not be opened.
    open_errors: HashMap<u64, String>,
    visible: HashMap<u64, bool>,
    /// Whether each window has keyboard focus, as the host last said.
    focused: HashMap<u64, bool>,
    /// Told when a window gains or loses focus ([`Window::on_focus`]).
    on_focus: HashMap<u64, Rc<dyn Fn(bool)>>,
    /// Windows the platform cannot hide while they are open (Wayland's): hiding minimizes.
    unhideable: HashSet<u64>,
    /// Wakes the host's loop for requests made between events.
    waker: Option<remedy_ui_core::host::Waker>,
    in_event: bool,
    on_tray: Option<Rc<dyn Fn(TrayEvent)>>,
    on_notification: Option<Rc<dyn Fn(u64, remedy_ui_core::host::NotificationResponse)>>,
    on_foreground_stopped: Option<Rc<dyn Fn(String)>>,
    /// Notification answers that came before [`App::on_notification`] was set: a press that
    /// started the app arrives before its setup.
    unheard_notifications: Vec<(u64, remedy_ui_core::host::NotificationResponse)>,
    next_job: u64,
    jobs: HashMap<u64, Job>,
    done: Arc<Mutex<Vec<(u64, JobResult)>>>,
    on_launch: Option<Rc<dyn Fn(Vec<String>) -> crate::instance::InstanceReply>>,
    /// The app's id, for its badge ([`AppConfig::app_id`]).
    app_id: String,
    /// Links in these schemes reach [`App::on_url`].
    #[cfg(feature = "desktop")]
    url_schemes: Vec<String>,
    #[cfg(feature = "desktop")]
    on_url: Option<Rc<dyn Fn(String)>>,
    /// Reports power changes to [`App::on_power`] while the app runs.
    #[cfg(feature = "desktop")]
    power: Option<remedy_ui_platform::services_desktop::power::PowerWatch>,
    on_activity: Option<Rc<dyn Fn()>>,
    /// Told when the app may be ended without another word ([`App::on_leaving`]).
    on_leaving: Option<Rc<dyn Fn(Leaving)>>,
    /// Asked before a window closes ([`Window::on_close_request`]), by window key.
    close_asks: HashMap<u64, CloseAsk>,
    /// Told when the app is back after [`Leaving::Hidden`] or [`Leaving::Frozen`].
    on_return: Option<Rc<dyn Fn()>>,
    /// Told the system's light/dark preference and high contrast, now and when they change.
    on_appearance: Option<AppearanceFn>,
    /// Hears the app's language (its own, or the system's) when it changes.
    on_locale: Option<LocaleFn>,
    /// The language the app shows now, once known.
    locale: Option<String>,
    /// The user's preferred languages, as the system last reported them.
    preferred_locales: Vec<String>,
    /// The last appearance the system reported.
    appearance: Option<(Option<bool>, bool)>,
    renderers: HashMap<u64, RendererKind>,
    titles: HashMap<u64, String>,
    /// Whether each window is maximized, kept current from the host.
    maximized: HashMap<u64, remedy_ui_core::reactive::Signal<bool>>,
    /// Whether each window fills the screen, kept current from the host.
    fullscreen: HashMap<u64, remedy_ui_core::reactive::Signal<bool>>,
    /// Each window's content zoom ([`Window::set_zoom`]).
    zoom: HashMap<u64, f32>,
    on_geometry: HashMap<u64, Rc<dyn Fn(Point, Size)>>,
    /// Told when a window is shown or hidden ([`Window::on_visibility`]).
    on_visibility: HashMap<u64, Rc<dyn Fn(bool)>>,
    on_closed: HashMap<u64, Vec<Box<dyn FnOnce()>>>,
    /// Waiting for a picture of the window's next frame ([`Window::snapshot`]).
    snapshots: HashMap<u64, Vec<SnapshotDone>>,
    /// Keys and pointer events sent to a window by the app ([`Window::send_key`]), in order.
    injected: Vec<(u64, Injected)>,
    /// The effect keeping the tray in step with [`App::tray_dyn`], while it does.
    tray_effect: Option<remedy_ui_core::reactive::Effect>,
    /// Agents' access to each window that took them ([`Window::attach_agents`]).
    #[cfg(feature = "agents")]
    agents: HashMap<u64, remedy_ui_widgets::automation::Agents>,
}

/// Asked before a window closes; false keeps it open.
type CloseAsk = Rc<dyn Fn(CloseReason) -> bool>;

/// Why a window is asked to close ([`Window::on_close_request`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CloseReason {
    /// The person asked to close this window: its close button, Alt+F4 or the platform's
    /// own shortcut, or a back action nothing took.
    Window,
    /// The app was asked to quit ([`App::quit`]).
    Quit,
}

/// Why the app may be ended ([`App::on_leave`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Leaving {
    /// Out of sight: its page's tab hidden, every window minimized. It may come back.
    Hidden,
    /// Frozen or suspended: a page put into the back-forward cache or frozen in the
    /// background, a phone app sent to the background. It may come back.
    Frozen,
    /// Its page closed, reloaded or navigated away from for good (not kept in the cache).
    Closing,
    /// The app quits, or its last window closed.
    Quitting,
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

    /// Records whether window `key` has keyboard focus, and tells its [`Window::on_focus`]
    /// when that changed.
    fn note_focus(&self, key: u64, on: bool) {
        let cb = {
            let mut s = self.0.borrow_mut();
            let was = s.focused.insert(key, on);
            (was != Some(on)).then(|| s.on_focus.get(&key).cloned()).flatten()
        };
        if let Some(cb) = cb {
            cb(on);
        }
    }

    /// Records whether window `key` is shown, and tells its [`Window::on_visibility`] when
    /// that changed.
    fn note_visible(&self, key: u64, on: bool) {
        let cb = {
            let mut s = self.0.borrow_mut();
            let was = s.visible.insert(key, on);
            (was != Some(on)).then(|| s.on_visibility.get(&key).cloned()).flatten()
        };
        if let Some(cb) = cb {
            cb(on);
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
        self.0.borrow_mut().maximized.insert(key, app_signal(false));
        self.0.borrow_mut().fullscreen.insert(key, app_signal(false));
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

    /// Calls `f` whenever the app may be ended without another word: its page hidden or
    /// closed (a browser tab put away, closed or reloaded), every window out of sight (all
    /// minimized), the app suspended (a phone app sent to the background), quitting, or its
    /// last window closing. The last moment for an app to stop its work and save: `f` runs
    /// at once, inside the browser's own `visibilitychange` or `pagehide`, so a request it
    /// starts with `net::Request::keepalive` (feature `net`) still goes out as the page closes (natively, quitting waits for it,
    /// [`AppConfig::keepalive_grace`]). It may run more than once on the way out (hidden,
    /// then closed); the app may also come back (shown again). A second call replaces `f`,
    /// and so does [`on_leave`](Self::on_leave), which also says why.
    pub fn on_leaving(&self, f: impl Fn() + 'static) {
        self.on_leave(move |_| f());
    }

    /// [`on_leaving`](Self::on_leaving), told why: hidden, frozen, closing or quitting. Work
    /// that should survive a tab switch stops on [`Leaving::Closing`] and
    /// [`Leaving::Quitting`] only. A second call replaces `f`.
    pub fn on_leave(&self, f: impl Fn(Leaving) + 'static) {
        self.0.borrow_mut().on_leaving = Some(Rc::new(f));
    }

    /// Calls `f` when the app is back in front of the person after [`on_leave`](Self::on_leave)
    /// reported [`Leaving::Hidden`] or [`Leaving::Frozen`]: a window is in sight again, the
    /// page's tab is shown again, a phone app returns to the foreground. Work stopped on
    /// leaving (playback, a microphone, polling) starts again here. Not called at launch.
    /// A second call replaces `f`.
    pub fn on_return(&self, f: impl Fn() + 'static) {
        self.0.borrow_mut().on_return = Some(Rc::new(f));
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

    /// Lays every window (and windows opened later) out the way `direction` says: from each
    /// window's locale ([`WindowDirection::Locale`](remedy_ui_engine::view::WindowDirection), the
    /// default), or always left to right or right to left, for an app setting that lets
    /// people choose. A window can still choose its own with [`Ui::set_direction`].
    pub fn set_direction(&self, direction: remedy_ui_engine::view::WindowDirection) {
        self.push(Cmd::Direction(direction));
    }

    /// The language the app shows (BCP 47): its own ([`App::set_locale`]) or the system's;
    /// `None` until the system has said. For parts with no window, such as a tray menu.
    pub fn locale(&self) -> Option<String> {
        self.0.borrow().locale.clone()
    }

    /// The user's preferred languages (BCP 47), most preferred first, as the system last
    /// reported them ([`SystemSettings::locales`]); empty until it has. For choosing among
    /// content languages, such as subtitles or audio tracks, whatever the app's own language.
    pub fn preferred_locales(&self) -> Vec<String> {
        self.0.borrow().preferred_locales.clone()
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

    /// Makes text styled in the CSS generic family `system-ui` draw in the device's own
    /// interface font, in every window and every window opened later, then calls `done` with
    /// that font's family names, or why it could not.
    ///
    /// On the desktop with `system-fonts` (on by default), `system-ui` already is the
    /// system's font (Segoe UI on Windows, San Francisco on macOS, the desktop's font on
    /// Linux) and `done` gets its names. In a browser page the engine draws text itself and
    /// cannot use a font it has no file of, so this reads the font the browser draws
    /// `system-ui` text in through the Local Font Access API (Chrome and Edge on desktops:
    /// the exact face the browser shows, told by measuring text), which asks the person once
    /// whether the page may use their fonts. Call it in answer to the click or key press that
    /// chooses the system font, the first time; once they allowed it, at any time (when the
    /// page loads with that choice saved). Where a browser cannot (Firefox, Safari, phones) or
    /// the person declines, `done` gets the reason and `system-ui` falls through to the next
    /// family the style names, so name the app's own face after it:
    /// `["system-ui", "Remedy Sans UI"]`.
    pub fn load_system_ui_font(&self, done: impl FnOnce(Result<Vec<String>, String>) + 'static) {
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        {
            let app = self.clone();
            remedy_ui_bridge_web::fonts::system_ui_faces(move |got| {
                let read = got.and_then(|(chosen, bytes)| {
                    let files: Vec<FontFile> =
                        bytes.into_iter().filter_map(|b| FontFile::from_vec(b).ok()).collect();
                    let mut families: Vec<String> =
                        files.iter().flat_map(|f| f.families().to_vec()).collect();
                    families.sort_by_key(|f| *f != chosen);
                    families.dedup();
                    if files.is_empty() {
                        Err(format!("the files of {chosen} could not be read"))
                    } else {
                        Ok((families, files))
                    }
                });
                match read {
                    Ok((families, files)) => {
                        let uis: Vec<Ui> = {
                            let mut s = app.0.borrow_mut();
                            s.system_ui = Some((families.clone(), files.clone()));
                            s.uis.values().cloned().collect()
                        };
                        for ui in uis {
                            ui.update_fonts(|f| use_system_ui(f, &families, &files));
                        }
                        done(Ok(families));
                    }
                    Err(e) => done(Err(e)),
                }
            });
        }
        #[cfg(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")))]
        {
            let mut fonts = bundled_fonts();
            let names = fonts.generic_family("system-ui");
            if names.is_empty() {
                done(Err("this build has no system fonts (remedy-ui's `system-fonts` feature)".into()));
            } else {
                done(Ok(names));
            }
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
    /// reads), then `done` with its result back on the UI thread. In a browser page, which
    /// has one thread, `work` runs whole as a task between frames.
    pub fn background<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) -> BackgroundJob {
        self.background_cancellable(move |_| work(), done)
    }

    /// Like [`App::background`], with work that can stop early: it gets a [`Cancelled`] to
    /// check between steps, which turns true once the job is cancelled. Cancelling also
    /// forgets `done`, so a result that arrives after it never reaches the UI.
    ///
    /// ```no_run
    /// # fn search(app: &remedy_ui::App) {
    /// let job = app.background_cancellable(
    ///     |cancelled| {
    ///         let mut found = Vec::new();
    ///         for n in 0..1_000_000u32 {
    ///             if cancelled.is_cancelled() {
    ///                 break;
    ///             }
    ///             if n % 97 == 0 {
    ///                 found.push(n);
    ///             }
    ///         }
    ///         found
    ///     },
    ///     |found| println!("{} matches", found.len()),
    /// );
    /// // The person typed again: the old search is not wanted any more.
    /// job.cancel();
    /// # }
    /// ```
    pub fn background_cancellable<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Cancelled) -> T + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) -> BackgroundJob {
        let c = self.completion(done);
        let job = BackgroundJob {
            id: c.id,
            app: Rc::downgrade(&self.0),
            cancelled: Cancelled(Arc::new(std::sync::atomic::AtomicBool::new(false))),
        };
        let cancelled = job.cancelled.clone();
        // A thread that cannot start drops the completion, which forgets `done`.
        let _ = remedy_ui_core::exec::spawn("remedy-ui background", move || {
            let out = {
                let _span = remedy_ui_core::phase!("background").entered();
                work(&cancelled)
            };
            if !cancelled.is_cancelled() {
                c.complete(out);
            }
        });
        job
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

    /// Quits, after asking every window that has a close-request handler
    /// ([`Window::on_close_request`], with [`CloseReason::Quit`]). The first that refuses
    /// stops the quit; the windows after it are not asked.
    pub fn quit(&self) {
        self.push(Cmd::Quit);
    }

    /// Quits without asking any window: for after a Save or Discard answer to a prompt a
    /// close-request handler opened.
    pub fn quit_now(&self) {
        self.push(Cmd::QuitNow);
    }

    /// True while a tray icon asked for with [`App::set_tray`] or [`App::tray_dyn`] is shown;
    /// false before, after `None`, and where the platform or desktop has no tray (a Linux
    /// desktop without a status notifier host, a browser). Tracked: a binding that reads it
    /// runs again when it changes. A tray app checks it before hiding a window it could not
    /// bring back otherwise.
    pub fn tray_shown(&self) -> bool {
        self.tray_signal().get()
    }

    fn tray_signal(&self) -> remedy_ui_core::reactive::Signal<bool> {
        *self.0.borrow_mut().tray_up.get_or_insert_with(|| app_signal(false))
    }

    /// Reduces motion in every window even where the system does not ask for it (an app's
    /// own "reduce motion" setting): animations jump to their end and spinners hold still,
    /// as with the system setting. Off follows the system again.
    pub fn set_reduce_motion(&self, on: bool) {
        let was = std::mem::replace(&mut self.0.borrow_mut().reduce_motion, on);
        if was != on {
            self.push(Cmd::ReduceMotion);
        }
    }

    /// Sets how much ambient motion (a theme's animated background, decorative loops) every
    /// window may have: full, light or off, frames a second with focus and in the
    /// background, and on battery. Windows minimized, hidden or covered never draw it, and
    /// reduced motion stops it unless the policy says otherwise. See
    /// [`AmbientPolicy`](remedy_ui_engine::view::AmbientPolicy).
    ///
    /// ```no_run
    /// # fn setup(app: &remedy_ui::App) {
    /// use remedy_ui::view::{AmbientMode, AmbientPolicy};
    /// let mut p = AmbientPolicy::default();
    /// p.mode = AmbientMode::Light;
    /// app.set_ambient_policy(p);
    /// # }
    /// ```
    pub fn set_ambient_policy(&self, policy: remedy_ui_engine::view::AmbientPolicy) {
        let uis: Vec<Ui> = {
            let mut s = self.0.borrow_mut();
            s.ambient = Some(policy);
            s.uis.values().cloned().collect()
        };
        for ui in uis {
            ui.set_ambient_policy(policy);
        }
    }

    /// The ambient-motion budget windows follow.
    pub fn ambient_policy(&self) -> remedy_ui_engine::view::AmbientPolicy {
        self.0.borrow().ambient.unwrap_or_default()
    }

    /// Draws each window at most `fps` times a second (`None`: as often as it changes, up
    /// to the display's rate), to save power or leave a game the processor and graphics card.
    pub fn set_frame_rate_limit(&self, fps: Option<u32>) {
        self.0.borrow_mut().frame_limit = fps.filter(|f| *f > 0);
    }

    /// The graphics adapter windows draw with on the GPU, once one has been made (`None`
    /// before, without the `gpu` feature, or where only the CPU draws).
    pub fn graphics_adapter(&self) -> Option<GraphicsAdapter> {
        self.0.borrow().adapter.clone()
    }

    /// Keeps the app running when its last window closes (tray apps).
    pub fn keep_running(&self, on: bool) {
        self.0.borrow_mut().keep_running = on;
    }

    /// Shows (or updates) the tray icon; `None` removes it. On Linux it is a
    /// StatusNotifierItem, or on X11 desktops without a StatusNotifier host an icon docked in
    /// the XEmbed system tray.
    pub fn set_tray(&self, tray: Option<TrayIcon>) {
        if let Some(e) = self.0.borrow_mut().tray_effect.take() {
            e.dispose();
        }
        self.push(Cmd::Tray(tray));
    }

    /// Keeps the tray icon in step with `tray`, which reads signals (the window's language,
    /// a count of unread messages): it shows what `tray` returns now and again whenever what it
    /// read changes. A later [`App::set_tray`] or `tray_dyn` replaces it.
    ///
    /// ```no_run
    /// # fn setup(app: &remedy_ui::App, icon: remedy_ui::host::TrayImage) {
    /// use remedy_ui::host::TrayIcon;
    /// use remedy_ui::reactive::Signal;
    ///
    /// let unread = Signal::new(0u32);
    /// app.tray_dyn(move || Some(TrayIcon::new(format!("{} unread", unread.get()), icon.clone(), Vec::new())));
    /// # }
    /// ```
    pub fn tray_dyn(&self, tray: impl Fn() -> Option<TrayIcon> + 'static) {
        if let Some(e) = self.0.borrow_mut().tray_effect.take() {
            e.dispose();
        }
        let app = self.clone();
        let effect = remedy_ui_core::reactive::Effect::new(move || {
            let t = tray();
            app.push(Cmd::Tray(t));
        });
        self.0.borrow_mut().tray_effect = Some(effect);
    }

    /// Shows a system notification (Windows' notification center, the Linux desktop's
    /// notification server, Android's shade, the browser's), or replaces the one with the
    /// same id, then `done` with the outcome: [`Unsupported`] where the platform has none or
    /// the person turned the app's notifications off. What the person does with it reaches
    /// [`App::on_notification`].
    ///
    /// On Android 13 and later and in a browser, ask first with
    /// [`App::request_permission`]`(Permission::Notifications, ..)` (and list `notifications`
    /// in `remedy.toml`'s `[android] permissions`). Keep private text out of the body unless
    /// the person chose to show it: notifications can show on a locked screen.
    pub fn notify(
        &self,
        notification: remedy_ui_core::host::Notification,
        done: impl FnOnce(Result<(), Unsupported>) + 'static,
    ) {
        self.push(Cmd::Notify(Box::new(notification), Box::new(done)));
    }

    /// Takes away a notification the app showed, if it is still shown (what it said no
    /// longer applies: the message was read in the app).
    pub fn withdraw_notification(&self, id: u64) {
        self.push(Cmd::Withdraw(id));
    }

    /// Shows `badge` on the app's taskbar button (Windows' overlay), Dock tile (macOS),
    /// dock launcher (Linux docks that read the launcher entry API; name the app with
    /// [`AppConfig::app_id`]) or installed web app's icon, or takes it away with `None`.
    /// Platforms without badges (Android, whose launchers count notifications) ignore it.
    ///
    /// ```no_run
    /// # fn unread(app: &remedy_ui::App, n: u32) {
    /// use remedy_ui::host::Badge;
    ///
    /// app.set_badge(Badge::count(n));
    /// # }
    /// ```
    pub fn set_badge(&self, badge: Option<remedy_ui_core::host::Badge>) {
        self.push(Cmd::Badge(badge));
    }

    /// How gamepads move focus: a held d-pad or stick repeats as a held arrow key does, after
    /// the system's keyboard repeat delay and at its rate unless `tuning` sets its own, and the
    /// stick counts once it leans past `tuning.dead_zone`. Gamepads' d-pad and left stick
    /// arrive as arrow keys, A as Enter and B as `Key::Back` (Windows' XInput pads, Linux's
    /// evdev pads, the browser's Gamepad API; Android's arrive as its own keys).
    ///
    /// ```no_run
    /// # fn setup(app: &remedy_ui::App) {
    /// use std::time::Duration;
    /// use remedy_ui::input::GamepadTuning;
    ///
    /// let tuning = GamepadTuning::default().with_repeat_delay(Duration::from_millis(300)).with_dead_zone(0.3);
    /// app.set_gamepad_tuning(tuning);
    /// # }
    /// ```
    pub fn set_gamepad_tuning(&self, tuning: remedy_ui_core::input::GamepadTuning) {
        self.push(Cmd::Gamepad(tuning));
    }

    /// Called with each link in one of `schemes` (`hearth://note/42`) the app is opened with:
    /// the one it was started with (right after this call), and with
    /// [`AppConfig::single_instance`] every link opened while it runs. Register the scheme
    /// with [`desktop::scheme::register`](crate::desktop::scheme::register) (on macOS, list
    /// it in `remedy.toml`'s `url_schemes`). Treat a link as untrusted input: anything on the
    /// system can open one.
    #[cfg(feature = "desktop")]
    pub fn on_url(&self, schemes: &[&str], f: impl Fn(String) + 'static) {
        let f: Rc<dyn Fn(String)> = Rc::new(f);
        let first = {
            let mut s = self.0.borrow_mut();
            let first = s.on_url.is_none();
            s.url_schemes = schemes.iter().map(|s| (*s).to_owned()).collect();
            s.on_url = Some(f);
            first
        };
        if first {
            let args: Vec<String> = std::env::args().skip(1).collect();
            self.links_in(&args);
        }
    }

    /// Hands the links in `args` to [`App::on_url`].
    #[cfg(feature = "desktop")]
    fn links_in(&self, args: &[String]) {
        let (schemes, f) = {
            let s = self.0.borrow();
            (s.url_schemes.clone(), s.on_url.clone())
        };
        let Some(f) = f else { return };
        let schemes: Vec<&str> = schemes.iter().map(String::as_str).collect();
        for url in remedy_ui_platform::services_desktop::scheme::urls(args, &schemes) {
            f(url);
        }
    }

    /// Called on the UI thread whenever the machine is about to sleep, wakes, or changes
    /// between battery and mains power, while the app runs. A second call replaces `f`.
    /// [`DesktopError::Unsupported`](crate::desktop::DesktopError::Unsupported) where the
    /// system reports none of it. To keep the machine awake, hold a
    /// [`desktop::power::inhibit`](crate::desktop::power::inhibit) guard.
    #[cfg(feature = "desktop")]
    pub fn on_power(
        &self,
        f: impl Fn(remedy_ui_platform::services_desktop::power::PowerEvent) + 'static,
    ) -> Result<(), remedy_ui_platform::services_desktop::DesktopError> {
        use remedy_ui_platform::services_desktop::power::PowerEvent;
        type Handler = Rc<dyn Fn(PowerEvent)>;
        thread_local! {
            static POWER: RefCell<Option<Handler>> = const { RefCell::new(None) };
        }
        POWER.with(|p| *p.borrow_mut() = Some(Rc::new(f)));
        if self.0.borrow().power.is_some() {
            return Ok(());
        }
        let ui = remedy_ui_core::reactive::waker().map_err(|e| {
            remedy_ui_platform::services_desktop::DesktopError::Platform { reason: e.to_string() }
        })?;
        let watch = remedy_ui_platform::services_desktop::power::watch(move |e| {
            let _ = ui.post(move || {
                let f = POWER.with(|p| p.borrow().clone());
                if let Some(f) = f {
                    f(e);
                }
            });
        })?;
        self.0.borrow_mut().power = Some(watch);
        Ok(())
    }

    /// Called when the person uses a notification the app showed: its id and what they did.
    /// Answers that came before it was set (pressing a notification that started the app,
    /// on Android) reach `f` right after the call that sets it.
    pub fn on_notification(&self, f: impl Fn(u64, remedy_ui_core::host::NotificationResponse) + 'static) {
        let f: Rc<dyn Fn(u64, remedy_ui_core::host::NotificationResponse)> = Rc::new(f);
        let unheard = {
            let mut s = self.0.borrow_mut();
            s.on_notification = Some(f.clone());
            std::mem::take(&mut s.unheard_notifications)
        };
        if !unheard.is_empty() {
            self.after(Duration::ZERO, move || {
                for (id, response) in unheard {
                    f(id, response);
                }
            });
        }
    }

    /// Keeps the app running while it is out of sight for ongoing work the person started (a
    /// voice call, music, a recording, an upload), showing the task's notification while it
    /// runs; then `done` with the outcome. Starting again while it runs changes the work and
    /// its notification. The notification's buttons, pressing it and dismissing it reach
    /// [`App::on_notification`] under its id.
    ///
    /// This is Android's foreground service: list each [`ForegroundKind`] the app uses in
    /// `remedy.toml`'s `[android] foreground` (`cargo remedy package` declares the service
    /// and its permissions), ask for [`Permission::Microphone`] or the camera first, and
    /// start microphone or camera work while the app is on screen: Android refuses it from
    /// the background ([`Unsupported`] says why). Desktop and web apps keep running out of
    /// sight anyway, so there it answers `Ok` and shows nothing; show a notification with
    /// [`App::notify`] where one is wanted there too.
    ///
    /// [`ForegroundKind`]: remedy_ui_core::host::ForegroundKind
    /// [`Permission::Microphone`]: remedy_ui_core::host::Permission::Microphone
    pub fn start_foreground(
        &self,
        task: remedy_ui_core::host::ForegroundTask,
        done: impl FnOnce(Result<(), Unsupported>) + 'static,
    ) {
        self.push(Cmd::Foreground(Box::new(task), Box::new(done)));
    }

    /// Ends the work [`App::start_foreground`] started and takes its notification away.
    pub fn stop_foreground(&self) {
        self.push(Cmd::StopForeground);
    }

    /// Called when the platform ends the work [`App::start_foreground`] started, or could not
    /// start it after all (the system's time limit for the kind ran out), with why. The app
    /// should stop that work, or start it again once it can.
    pub fn on_foreground_stopped(&self, f: impl Fn(String) + 'static) {
        self.0.borrow_mut().on_foreground_stopped = Some(Rc::new(f));
    }

    /// Asks the person a question in a small window of the app's own, drawn in the app's
    /// theme, then calls `done` with the answer: true for the yes button, false for the no
    /// button, Escape, or closing the window. It works with no other window open (a tray
    /// app asking "Quit?"): the window opens centered, above other windows, in front and
    /// focused, as a message box that sets itself in front does. Enter presses the focused
    /// button (the no button when [`ConfirmSpec::danger`](crate::ConfirmSpec::danger)), Tab
    /// moves between the buttons, and screen readers meet it as a dialog with its title and
    /// text. Its window is one of the app's, so an app without
    /// [`App::keep_running`] quits when it closes and no other window is open.
    ///
    /// ```no_run
    /// # fn setup(app: &remedy_ui::App) {
    /// use remedy_ui::ConfirmSpec;
    ///
    /// let a = app.clone();
    /// let ask = ConfirmSpec::new("Quit Portal?", "Anything playing will stop.").ok("Yes").cancel("No").danger(true);
    /// app.confirm(ask, move |yes| if yes { a.quit() });
    /// # }
    /// ```
    #[cfg(feature = "widgets")]
    pub fn confirm(&self, spec: crate::ConfirmSpec, done: impl FnOnce(bool) + 'static) -> Window {
        crate::confirm::open(self, spec, Box::new(done))
    }

    /// Called when the tray icon is used.
    pub fn on_tray(&self, f: impl Fn(TrayEvent) + 'static) {
        self.0.borrow_mut().on_tray = Some(Rc::new(f));
    }

    /// Called when the app is launched again while running (with
    /// [`AppConfig::single_instance`]), with that launch's arguments. What it returns is the
    /// reply a launch waiting for one gets ([`AppConfig::single_instance_reply`]): nothing
    /// (`()`, which answers 0), an exit code, or [`InstanceReply::later`](crate::InstanceReply::later)
    /// for a code sent once slower work is done.
    ///
    /// ```no_run
    /// # fn setup(app: &remedy_ui::App) {
    /// use remedy_ui::InstanceReply;
    ///
    /// let a = app.clone();
    /// app.on_second_instance(move |args| {
    ///     if args.iter().any(|x| x == "--stop") {
    ///         let (reply, later) = InstanceReply::later();
    ///         let a2 = a.clone();
    ///         a.background(|| 0, move |code| {
    ///             later.send(code);
    ///             a2.quit();
    ///         });
    ///         return reply;
    ///     }
    ///     InstanceReply::code(0)
    /// });
    /// # }
    /// ```
    pub fn on_second_instance<R: Into<crate::instance::InstanceReply>>(
        &self,
        f: impl Fn(Vec<String>) -> R + 'static,
    ) {
        self.0.borrow_mut().on_launch = Some(Rc::new(move |args| f(args).into()));
    }

    /// Runs a native open or save dialog (modal to `window`), then `done` with the chosen
    /// paths (empty when cancelled).
    pub fn file_dialog(
        &self,
        window: Option<&Window>,
        dialog: FileDialog,
        done: impl FnOnce(Vec<std::path::PathBuf>) + 'static,
    ) {
        self.try_file_dialog(window, dialog, move |r| done(r.unwrap_or_default()));
    }

    /// [`App::file_dialog`], telling a cancelled dialog (`Ok` with no paths) from a system
    /// that has none to show ([`Unsupported`]: a Linux desktop without the file-chooser
    /// portal, say), so the app can offer its own way to name a file (a field for a path)
    /// instead of doing nothing.
    pub fn try_file_dialog(
        &self,
        window: Option<&Window>,
        dialog: FileDialog,
        done: impl FnOnce(Result<Vec<std::path::PathBuf>, Unsupported>) + 'static,
    ) {
        self.push(Cmd::Dialog(window.map(|w| w.key), dialog, Box::new(done)));
    }

    /// Opens `url` in the user's browser or mail app, then `done` with the outcome. Only
    /// `http:`, `https:` and `mailto:` links open; anything else could start a program.
    pub fn open_url(&self, url: impl Into<String>, done: impl FnOnce(Result<(), Unsupported>) + 'static) {
        self.push(Cmd::OpenUrl(url.into(), Box::new(done)));
    }

    /// Starts the UI over, after an update or a change that needs a fresh start: in a
    /// browser page, reloads the page (at the same address, so the same route); elsewhere,
    /// rebuilds every [`Router::view`](crate::router::Router::view) at its current route.
    pub fn reload(&self) {
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        remedy_ui_bridge_web::history::reload();
        #[cfg(all(
            feature = "widgets",
            not(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))
        ))]
        crate::router::reload_views();
    }

    /// Asks for the person's leave to use `permission`, then `done` with the answer (true when
    /// granted). On Android this shows the system's prompt unless the permission was already
    /// granted, and the app's manifest must declare it (`cargo remedy package` writes it from
    /// `[android] permissions` in `remedy.toml`). Platforms without run-time permissions answer
    /// true at once; their privacy settings may still refuse the device, which then says so
    /// when it is opened.
    ///
    /// Ask when the person reaches for the feature (pressing a tuner's start), not at launch,
    /// so the prompt explains itself.
    pub fn request_permission(
        &self,
        permission: remedy_ui_core::host::Permission,
        done: impl FnOnce(bool) + 'static,
    ) {
        self.push(Cmd::Permission(permission, Box::new(done)));
    }

    /// Hands a title, a text and files to the platform's share sheet (over `window`), then
    /// `done` with how it ended: [`ShareOutcome::Shared`] once the person chose where it goes,
    /// [`ShareOutcome::Cancelled`] when they closed the sheet, and
    /// [`ShareOutcome::Unsupported`] with the reason where the platform cannot share it, so
    /// the app can fall back to a save dialog or the clipboard.
    ///
    /// Android shows its chooser (the files served to the chosen app from the app's cache,
    /// through the provider `cargo remedy package` declares), Windows its share window, and a
    /// browser page `navigator.share` (call it from a click or key press: browsers share only
    /// in answer to one). Linux and macOS have no share sheet the bridges show.
    ///
    /// ```no_run
    /// # use remedy_ui::App;
    /// use remedy_ui::host::{ShareFile, ShareOptions, ShareOutcome};
    /// # fn f(app: &App, json: Vec<u8>) {
    /// let share = ShareOptions::new()
    ///     .title("Lavender soap")
    ///     .text("Lavender soap: 500 g oils, 5% superfat")
    ///     .file(ShareFile::new("Lavender.alex-soap.json", "application/json", json));
    /// app.share(None, share, |outcome| match outcome {
    ///     ShareOutcome::Shared | ShareOutcome::Cancelled => {}
    ///     ShareOutcome::Unsupported(why) => eprintln!("{why}; offering a save dialog instead"),
    ///     _ => {}
    /// });
    /// # }
    /// ```
    ///
    /// [`ShareOutcome::Shared`]: remedy_ui_core::host::ShareOutcome::Shared
    /// [`ShareOutcome::Cancelled`]: remedy_ui_core::host::ShareOutcome::Cancelled
    /// [`ShareOutcome::Unsupported`]: remedy_ui_core::host::ShareOutcome::Unsupported
    pub fn share(
        &self,
        window: Option<&Window>,
        share: remedy_ui_core::host::ShareOptions,
        done: impl FnOnce(remedy_ui_core::host::ShareOutcome) + 'static,
    ) {
        self.push(Cmd::Share(window.map(|w| w.key), share, Box::new(done)));
    }

    /// Prints `html` (a whole page, UTF-8) through the platform's print dialog, over `window`,
    /// as a job named `title`: Windows' print dialog, Android's print manager (printers and
    /// Save as PDF), the browser's print dialog in a page. `done` says whether the dialog
    /// opened; what the person does there is the platform's.
    pub fn print_html(
        &self,
        window: Option<&Window>,
        title: impl Into<String>,
        html: impl Into<String>,
        done: impl FnOnce(Result<(), Unsupported>) + 'static,
    ) {
        self.push(Cmd::Print(window.map(|w| w.key), title.into(), html.into(), Box::new(done)));
    }

    /// Forgets saved geometry for `key`.
    pub fn forget_geometry(&self, key: impl Into<String>) {
        self.push(Cmd::Forget(key.into()));
    }

    /// Where the app keeps its data (created on first write): the folder
    /// [`AppConfig::data_dir`] names, else the platform's place for it: `%APPDATA%\<name>` on
    /// Windows, `~/Library/Application Support/<name>` on macOS, `$XDG_CONFIG_HOME/<name>`
    /// (or `~/.config/<name>`) on Linux, and the app's private internal storage on Android
    /// (`/data/user/<user>/<package>/files`, the app's own already, so without the name).
    /// `None` in a browser page, which has no files: keep data there with
    /// [`App::persist_store`], which works on every platform.
    pub fn data_dir(&self) -> Option<PathBuf> {
        let s = self.0.borrow();
        s.data_dir.clone().or_else(|| data_dir(&s.name))
    }

    /// Where persisted signals ([`remedy_ui_core::reactive::Signal::persist`]) and the app's own
    /// documents ([`PersistStore::save`](remedy_ui_core::reactive::PersistStore::save) and
    /// [`load`](remedy_ui_core::reactive::PersistStore::load)) are kept: the `state` folder of the
    /// app's data folder ([`App::data_dir`]), and in a browser page the page's `localStorage`
    /// under `remedy:<name>:` ([`PersistStore::local_storage`](remedy_ui_core::reactive::PersistStore)).
    /// Every window's content is built with it provided as context, so
    /// `Signal::new(v).persist("key")` there saves and restores itself. It is flushed when the
    /// app quits, is suspended, or goes out of sight (every window hidden or covered, a page's
    /// tab hidden). `None` when the platform gives the app no place for data (no data folder,
    /// or a page whose browser blocks its storage).
    pub fn persist_store(&self) -> Option<remedy_ui_core::reactive::PersistStore> {
        if let Some(s) = self.0.borrow().persist.clone() {
            return Some(s);
        }
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        let store = {
            let name = self.0.borrow().name.clone();
            remedy_ui_core::reactive::PersistStore::local_storage(&name).ok()?
        };
        #[cfg(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")))]
        let store = remedy_ui_core::reactive::PersistStore::open(self.data_dir()?.join("state")).ok()?;
        self.0.borrow_mut().persist = Some(store.clone());
        Some(store)
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

    /// Closes the window, without asking its close-request handler: the app has decided.
    pub fn close(&self) {
        self.app.push(Cmd::Close(self.key));
    }

    /// Asks `f` before this window closes because the person asked (its close button,
    /// Alt+F4, a back action nothing took) or because the app was told to quit
    /// ([`App::quit`]). `f` returns true to let it close and false to keep it open: an
    /// editor with unsaved work returns false, opens its Save / Discard / Cancel prompt, and
    /// on Save or Discard calls [`Window::close`] (or [`App::quit_now`] when the reason was
    /// [`CloseReason::Quit`]), neither of which asks again. A window that hides or minimizes
    /// on close is not closing and is not asked. A second call replaces `f`.
    pub fn on_close_request(&self, f: impl Fn(CloseReason) -> bool + 'static) {
        self.app.0.borrow_mut().close_asks.insert(self.key, Rc::new(f));
    }

    /// Removes the close-request handler: the window closes when asked to.
    pub fn clear_close_request(&self) {
        self.app.0.borrow_mut().close_asks.remove(&self.key);
    }

    /// Shows or hides. Where the platform cannot hide an open window ([`Window::can_hide`]
    /// is false: Wayland), hiding minimizes it instead and the window stays visible.
    pub fn set_visible(&self, on: bool) {
        if !on && !self.can_hide() {
            self.app.push(Cmd::Minimize(self.key, true));
            return;
        }
        self.app.note_visible(self.key, on);
        self.app.push(Cmd::Visible(self.key, on));
    }

    /// Whether the platform can take this window off the screen while it stays open. Wayland
    /// cannot (a compositor offers no request for it), so there [`Window::set_visible`]`(false)`,
    /// [`WindowSpec::hidden`] and closing a [`WindowSpec::hide_on_close`] window minimize it
    /// instead, and it stays reported as shown. True until the window exists (a hide asked for
    /// before then is handled the same way when it arrives).
    pub fn can_hide(&self) -> bool {
        !self.app.0.borrow().unhideable.contains(&self.key)
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

    /// Resizes the content to `width` × `height` logical pixels (the platform may adjust it;
    /// [`Window::on_geometry`] reports what it chose).
    pub fn set_size(&self, width: f32, height: f32) {
        self.app.push(Cmd::Size(self.key, Size::new(width, height)));
    }

    /// Moves the window to the middle of its display, as a message box opens. Where the
    /// platform places windows itself (Wayland, a browser, Android) it stays where it is.
    pub fn center(&self) {
        self.app.push(Cmd::Center(self.key));
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

    /// Makes the window fill its display with no frame, or brings it back (a player's full
    /// screen: wire it to `Player::on_fullscreen` from `remedy_ui::playback`
    /// with the `playback` feature). Where the platform has no full-screen windows the
    /// window stays as it is and [`Window::is_fullscreen`] stays false.
    pub fn set_fullscreen(&self, on: bool) {
        self.app.push(Cmd::Fullscreen(self.key, on));
    }

    /// Draws the window's content `zoom` times larger (an app's interface size, Ctrl and +):
    /// text, spacing, images and hit testing all follow, and the content lays out in the
    /// smaller logical size that leaves. Clamped to 0.25 to 5; 1 is as designed.
    pub fn set_zoom(&self, zoom: f32) {
        let zoom = if zoom.is_finite() { zoom.clamp(0.25, 5.0) } else { 1.0 };
        self.app.0.borrow_mut().zoom.insert(self.key, zoom);
        self.app.push(Cmd::Zoom(self.key, zoom));
    }

    /// The window's content zoom ([`Window::set_zoom`]).
    pub fn zoom(&self) -> f32 {
        self.app.0.borrow().zoom.get(&self.key).copied().unwrap_or(1.0)
    }

    /// Whether closing the window hides it (as [`WindowSpec::hide_on_close`]) or closes it,
    /// changed while it is open: an app's "keep running in the tray when closed" setting.
    pub fn set_hide_on_close(&self, on: bool) {
        self.app.push(Cmd::HideOnClose(self.key, on));
    }

    /// Whether closing the window minimizes it (as [`WindowSpec::minimize_on_close`]),
    /// changed while it is open.
    pub fn set_minimize_on_close(&self, on: bool) {
        self.app.push(Cmd::MinimizeOnClose(self.key, on));
    }

    /// True while the window fills its display, by the app or the user (tracked).
    pub fn is_fullscreen(&self) -> bool {
        let s = self.app.0.borrow().fullscreen.get(&self.key).copied();
        s.is_some_and(|s| s.get())
    }

    /// True while the window is maximized, by the app or the user (tracked).
    pub fn is_maximized(&self) -> bool {
        self.maximized().get()
    }

    /// Tracks whether the window is maximized: button, Aero Snap, dragging off a
    /// maximized window, Win+Up/Down, double-click on the title bar. Bind a custom
    /// title-bar icon to this so it follows the real window state.
    pub fn maximized(&self) -> remedy_ui_core::reactive::Signal<bool> {
        self.app
            .0
            .borrow()
            .maximized
            .get(&self.key)
            .cloned()
            .unwrap_or_else(|| remedy_ui_core::reactive::Signal::new(false))
    }

    /// Calls `f` with the window's desktop position and content size (logical pixels) after
    /// the user moves or resizes it.
    pub fn on_geometry(&self, f: impl Fn(Point, Size) + 'static) {
        self.app.0.borrow_mut().on_geometry.insert(self.key, Rc::new(f));
    }

    /// Calls `f` with `true` when the window is shown and `false` when it is hidden: by
    /// [`Window::set_visible`] or [`Window::focus`], or by the user closing a
    /// [`WindowSpec::hide_on_close`] window (a tray app remembering whether its window was
    /// open). Only changes are reported; a later call replaces `f`.
    pub fn on_visibility(&self, f: impl Fn(bool) + 'static) {
        self.app.0.borrow_mut().on_visibility.insert(self.key, Rc::new(f));
    }

    /// True while the window has keyboard focus (the person is using it): false when another
    /// app's window is in front, and while it is minimized or hidden. False until the
    /// platform first says it is focused.
    pub fn is_focused(&self) -> bool {
        self.app.0.borrow().focused.get(&self.key).copied().unwrap_or(false)
    }

    /// Calls `f` with `true` when the window gains keyboard focus and `false` when it loses
    /// it (another window comes in front, it is minimized or hidden): an app that alerts only
    /// when the person is elsewhere. Only changes are reported; a later call replaces `f`.
    pub fn on_focus(&self, f: impl Fn(bool) + 'static) {
        self.app.0.borrow_mut().on_focus.insert(self.key, Rc::new(f));
    }

    /// Calls `done` with a picture of the window's next frame: the whole content at its size
    /// in device pixels, premultiplied RGBA (a picture tour, a bug report, a test on a real
    /// desktop). A window the GPU draws reads back the frame it shows, so the picture holds
    /// what the screen does: Prism effects, video, `gpu_view` textures, blurs and shadows. A
    /// window the CPU draws draws its frame once more. `None` when the window is closed or the
    /// frame could not be drawn. A hidden window draws no frames, so its picture comes once it
    /// is shown.
    pub fn snapshot(&self, done: impl FnOnce(Option<remedy_ui_engine::paint::Image>) + 'static) {
        let Some(ui) = self.ui() else {
            done(None);
            return;
        };
        self.app.0.borrow_mut().snapshots.entry(self.key).or_default().push(Box::new(done));
        ui.request_frame();
    }

    /// [`snapshot`](Self::snapshot) as PNG bytes (straight alpha, as PNG stores it): a picture
    /// to save or attach. `None` as for `snapshot`, or when it cannot be encoded.
    #[cfg(feature = "devtools")]
    pub fn snapshot_png(&self, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
        self.snapshot(move |img| {
            done(img.and_then(|i| {
                let snap = remedy_ui_widgets::devtools::Snapshot {
                    width: i.width,
                    height: i.height,
                    rgba: i.pixels.to_vec(),
                };
                snap.to_png().ok()
            }));
        });
    }

    /// Hands `event` to the window as if the platform sent it: a key press or release goes
    /// where a keyboard's would (the focused element, then the window's shortcuts), for
    /// automation and end-to-end tests of a running app. It arrives with the next event the
    /// app handles (the window is asked for a frame). [`press_key`](Self::press_key) sends a
    /// press and its release.
    pub fn send_key(&self, event: remedy_ui_core::input::KeyEvent) {
        self.inject(Box::new(move |w| HostEvent::Key(w, event)));
    }

    /// Presses and releases `event`'s key ([`send_key`](Self::send_key) twice).
    pub fn press_key(&self, event: remedy_ui_core::input::KeyEvent) {
        let release = remedy_ui_core::input::KeyEvent::release(event.key.clone())
            .with_modifiers(event.modifiers)
            .with_physical(event.physical.clone());
        let mut press = event;
        press.pressed = true;
        self.send_key(press);
        self.send_key(release);
    }

    /// Hands a pointer event to the window as if the platform sent it, at a position in the
    /// window's logical pixels (before the window's [zoom](Self::set_zoom)): hover, press,
    /// drag and release reach elements as the mouse's or a finger's would, gestures included.
    /// [`click_at`](Self::click_at) sends a press and its release.
    pub fn send_pointer(&self, event: PointerEvent) {
        self.inject(Box::new(move |w| HostEvent::Pointer(w, event)));
    }

    /// Clicks the primary mouse button at `at` (window logical pixels): a move there, a press
    /// and a release.
    pub fn click_at(&self, at: Point) {
        for phase in [PointerPhase::Move, PointerPhase::Down, PointerPhase::Up] {
            self.send_pointer(PointerEvent::new(phase, at));
        }
    }

    fn inject(&self, event: Injected) {
        self.app.0.borrow_mut().injected.push((self.key, event));
        if let Some(ui) = self.ui() {
            ui.request_frame();
        }
    }

    /// Runs `f` once when the window is gone, however it closed.
    pub fn on_closed(&self, f: impl FnOnce() + 'static) {
        self.app.0.borrow_mut().on_closed.entry(self.key).or_default().push(Box::new(f));
    }

    /// The renderer drawing the window now (`None` before its first frame).
    pub fn renderer(&self) -> Option<RendererKind> {
        self.app.0.borrow().renderers.get(&self.key).copied()
    }

    /// Lets agents (an assistant, the RemedyOS mind) work with this window, through its
    /// semantics and the app's `commands`, once the person consents: ask them with
    /// [`Agents::request_access`](remedy_ui_widgets::automation::Agents::request_access), then
    /// [`Agents::listen`](remedy_ui_widgets::automation::Agents::listen) for the agent. The same
    /// access comes back on every call; it ends when the window closes. `None` once the window
    /// is closed.
    ///
    /// ```no_run
    /// # fn setup(window: &remedy_ui::Window) {
    /// use remedy_ui::agents::Scope;
    ///
    /// if let Some(agents) = window.attach_agents(None) {
    ///     let a2 = agents.clone();
    ///     agents.request_access("Assistant", Scope::Act, None, move |token| {
    ///         if let (Some(token), Ok(listener)) = (token, a2.listen()) {
    ///             println!("agent: connect to {} with {token}", listener.addr());
    ///         }
    ///     });
    /// }
    /// # }
    /// ```
    #[cfg(feature = "agents")]
    pub fn attach_agents(
        &self,
        commands: Option<&remedy_ui_widgets::nav::Commands>,
    ) -> Option<remedy_ui_widgets::automation::Agents> {
        if let Some(a) = self.agents() {
            return Some(a);
        }
        let ui = self.ui()?;
        let agents = remedy_ui_widgets::automation::Agents::new(&ui, commands);
        self.app.0.borrow_mut().agents.insert(self.key, agents.clone());
        Some(agents)
    }

    /// The agents' access [`Window::attach_agents`] gave this window, if it did.
    #[cfg(feature = "agents")]
    pub fn agents(&self) -> Option<remedy_ui_widgets::automation::Agents> {
        self.app.0.borrow().agents.get(&self.key).cloned()
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
        let insp = remedy_ui_widgets::devtools::Inspector::attach(&target, move |t| sheet(t));
        let i2 = insp.clone();
        let title = match self.title() {
            t if t.is_empty() => "Inspector".to_owned(),
            t => format!("Inspector · {t}"),
        };
        let win = self.app.open(WindowSpec::new(title).size(480.0, 760.0), move |ui, _| {
            let (theme, sheet) = remedy_ui_widgets::devtools::tool_style();
            ui.set_theme(theme, sheet);
            ui.root().child(remedy_ui_widgets::devtools::inspector_panel(ui, &i2));
        });
        let (app, key) = (self.app.clone(), self.key);
        win.on_closed(move || {
            insp.detach();
            app.0.borrow_mut().inspectors.remove(&key);
        });
        self.app.0.borrow_mut().inspectors.insert(self.key, win.clone());
        Some(win)
    }

    /// Serves this window to remote devtools (its element tree, element details, semantics,
    /// signal graph, update log and frame timings) on a loopback port, for a devtools window
    /// elsewhere to connect with the server's address and token. Debug builds only unless
    /// `options.allow_release` is set; serving stops when the returned server is dropped. On
    /// Android, forward the port to the desktop with `adb forward tcp:PORT tcp:PORT`.
    #[cfg(feature = "devtools")]
    pub fn serve_devtools(
        &self,
        options: remedy_ui_widgets::devtools::RemoteOptions,
    ) -> std::io::Result<remedy_ui_widgets::devtools::RemoteServer> {
        let ui = self
            .ui()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "the window is not open"))?;
        remedy_ui_widgets::devtools::RemoteServer::listen(&ui, options)
    }

    /// Keeps the window out of screenshots and screen sharing, or not.
    pub fn set_capture_protected(&self, on: bool) {
        self.app.push(Cmd::Capture(self.key, on));
    }

    /// Changes the window's own icon ([`WindowSpec::icon`]), or gives it back the app's with
    /// `None`: a note window in its note's color, a document's file type.
    pub fn set_icon(&self, icon: Option<WindowIcon>) {
        self.app.push(Cmd::Icon(self.key, icon));
    }
}

/// What runs with a file dialog's choice.
type DialogDone = Box<dyn FnOnce(Result<Vec<std::path::PathBuf>, Unsupported>)>;
type PermissionDone = Box<dyn FnOnce(bool)>;
type ShareDone = Box<dyn FnOnce(remedy_ui_core::host::ShareOutcome)>;

enum Present {
    Cpu(Box<CpuCompositor>),
    #[cfg(feature = "gpu")]
    Gpu(Box<remedy_ui_engine::render_gpu::GpuSurface>),
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
    /// It wanted the GPU while the device was still being made (browsers make it
    /// asynchronously) and draws on the CPU until the device is ready.
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    awaiting_gpu: bool,
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    cost: FrameCost,
    scrollers: Vec<(LayerId, ElementId)>,
    /// Scrolls asked for before their container had a layer (content built this frame, such
    /// as a page a router brings back), applied once it has one.
    pending_scrolls: Vec<(ElementId, Vec2, ScrollKind)>,
    cover: Option<remedy_ui_engine::view::El>,
    /// The window's content zoom (1 = as designed): everything is drawn this much larger,
    /// and input positions are mapped back.
    zoom: f32,
    /// When it was last drawn, and a frame held back by the app's frame-rate limit.
    last_frame: Option<Duration>,
    held: Option<TimerId>,
    /// A finger that is not yet a tap or a scroll, or that is scrolling.
    pan: Option<TouchPan>,
    /// What the host was last told about the mouse pointer ([`Ui::cursor_hidden`]).
    cursor_hidden: bool,
    /// The pointer shape the host was last told ([`Ui::cursor`]).
    cursor: remedy_ui_engine::view::Cursor,
    /// Middle-button autoscroll in progress ([`Autoscroll`]).
    autoscroll: Option<Autoscroll>,
}

/// Middle-button autoscroll, as Windows browsers and editors have it: a middle press over
/// something that scrolls marks a spot, and the content then scrolls toward the pointer,
/// faster the further it is, every frame until any button is pressed, Escape is pressed,
/// or (when the middle button was held and moved) it is released.
struct Autoscroll {
    origin: Point,
    at: Point,
    /// The middle button is still down from the press that started it.
    held: bool,
    /// The pointer moved past the dead zone while held: releasing ends it.
    dragged: bool,
    last: Option<Duration>,
}

impl Autoscroll {
    /// No scrolling this close to the spot.
    const DEAD_ZONE: f32 = 12.0;
    /// Logical pixels per second for each pixel past the dead zone.
    const SPEED: f32 = 8.0;

    /// The scroll for `dt` seconds.
    fn delta(&self, dt: f32) -> Vec2 {
        let axis = |d: f32| {
            let past = d.abs() - Self::DEAD_ZONE;
            if past > 0.0 { past * Self::SPEED * dt * d.signum() } else { 0.0 }
        };
        Vec2::new(axis(self.at.x - self.origin.x), axis(self.at.y - self.origin.y))
    }
}

/// How long a finger rests before a press is a long press (Android's default).
const LONG_PRESS: Duration = Duration::from_millis(500);

/// A finger pressed on a window: held back from the view until it lifts (a tap) or moves
/// past the slop (a scroll), so a swipe over a field neither focuses it nor clicks.
struct TouchPan {
    down: PointerEvent,
    last: Point,
    scrolling: bool,
    layer: Option<LayerId>,
    /// The view scrolls under the finger itself (an element that takes the wheel: a page
    /// view, a text area), so the drag reaches it as wheel movement.
    by_view: bool,
    velocity: VelocityTracker,
    /// Fires when the finger has rested long enough to be a long press.
    hold: Option<TimerId>,
}

impl Win {
    /// Whether something at `p` (the content's logical pixels) can scroll: a field or other
    /// element that takes the wheel, or a scroll container. Probes one pixel each way and
    /// puts back what moved.
    fn can_scroll_at(&mut self, p: Point) -> bool {
        for dir in [Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0), Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)] {
            if self.ui.wheel(&remedy_ui_core::input::WheelEvent::new(p, dir).with_precise(true)) {
                self.ui.wheel(&remedy_ui_core::input::WheelEvent::new(p, -dir).with_precise(true));
                return true;
            }
            let s = self.scale();
            if let Some(c) = self.comp.as_mut() {
                let at = Point::new(p.x * s, p.y * s);
                if c.scroll_at(at, dir * s).is_some() {
                    c.scroll_at(at, -dir * s);
                    return true;
                }
            }
        }
        false
    }

    /// Pixels per logical pixel of the window's content: the display's scale times the zoom.
    fn scale(&self) -> f32 {
        self.size.2 * self.zoom
    }

    /// Turns a touch into scrolling on the compositor, as a wheel is (the platforms give a
    /// canvas no scrolling of its own). Returns the events the view gets instead.
    fn touch(&mut self, e: PointerEvent) -> Vec<PointerEvent> {
        let slop = GestureSettings::default().touch_slop;
        let s = self.scale();
        let Some(p) = self.pan.as_mut().filter(|p| p.down.id == e.id) else {
            if e.phase == PointerPhase::Down && self.pan.is_none() {
                let mut velocity = VelocityTracker::new();
                velocity.add_event(&e);
                self.pan = Some(TouchPan {
                    last: e.position,
                    down: e,
                    scrolling: false,
                    layer: None,
                    by_view: false,
                    velocity,
                    hold: None,
                });
                return Vec::new();
            }
            // A second finger: the view has both from here on (a pinch, a two-finger pan),
            // the first from where it is now. Held back, it would leave the view one finger.
            if e.phase == PointerPhase::Down
                && let Some(first) = self.pan.take()
            {
                let mut down = first.down;
                down.position = first.last;
                return vec![down, e];
            }
            return vec![e];
        };
        p.velocity.add_event(&e);
        match e.phase {
            PointerPhase::Move => {
                let (dx, dy) = (e.position.x - p.down.position.x, e.position.y - p.down.position.y);
                if !p.scrolling && dx * dx + dy * dy <= slop * slop {
                    return Vec::new();
                }
                p.scrolling = true;
                let moved = Vec2::new(p.last.x - e.position.x, p.last.y - e.position.y);
                let d = Vec2::new(moved.x * s, moved.y * s);
                p.last = e.position;
                let at = Point::new(e.position.x * s, e.position.y * s);
                match self.comp.as_mut().and_then(|c| c.scroll_at(at, d)) {
                    Some(l) => p.layer = Some(l),
                    None if p.layer.is_none() => {
                        // No scroll container under the finger. An element that scrolls
                        // itself takes the drag as it takes the wheel.
                        let wheel =
                            remedy_ui_core::input::WheelEvent::new(e.position, moved).with_precise(true);
                        if self.ui.wheel(&wheel) {
                            p.by_view = true;
                        } else if !p.by_view && dx.abs() > dy.abs() {
                            // Nothing scrolls sideways here: a sideways drag is the view's
                            // (a slider, a drag and drop).
                            let down = self.pan.take().map(|p| p.down);
                            return down.into_iter().chain([e]).collect();
                        }
                    }
                    None => {}
                }
                Vec::new()
            }
            PointerPhase::Up => {
                let p = self.pan.take();
                match p {
                    Some(TouchPan { scrolling: true, layer, velocity, .. }) => {
                        let v = velocity.velocity();
                        if let (Some(l), Some(c)) = (layer, self.comp.as_mut()) {
                            c.fling(l, Vec2::new(-v.x, -v.y));
                        }
                        Vec::new()
                    }
                    Some(p) => vec![p.down, e],
                    None => vec![e],
                }
            }
            // The view never saw this finger go down.
            _ => {
                self.pan = None;
                Vec::new()
            }
        }
    }

    /// A finger rested past [`LONG_PRESS`] without moving: the press becomes the touch
    /// screen's right-click (a context menu opens under it), and the rest of the finger's
    /// movement goes to the view.
    fn long_press(&mut self, timer: TimerId) -> Option<PointerEvent> {
        if self.pan.as_ref().is_none_or(|p| p.scrolling || p.hold != Some(timer)) {
            return None;
        }
        self.pan.take().map(|p| p.down.with_buttons(Buttons::SECONDARY))
    }
}

/// `p`, a position the host reports in window logical pixels, in the content's logical
/// pixels under `zoom`.
fn unzoom(p: Point, zoom: f32) -> Point {
    Point::new(p.x / zoom, p.y / zoom)
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
    /// Frames held back by the frame-rate limit, by the window they are for.
    frame_timers: HashMap<TimerId, u64>,
    /// Host timers standing for view timers: (window, view timer id).
    view_timers: HashMap<TimerId, (u64, u64)>,
    geometry: GeometryStore,
    save_timer: Option<TimerId>,
    settings: SystemSettings,
    /// The app's own locale, in place of the system's.
    locale: Option<String>,
    /// Where every window's layout direction comes from ([`App::set_direction`]).
    direction: remedy_ui_engine::view::WindowDirection,
    idle: Option<(crate::secure::IdleTimer, Rc<dyn Fn()>)>,
    activity_reported: Option<Duration>,
    idle_timer: Option<TimerId>,
    /// Fires at the next local midnight, so dates roll over in idle windows.
    midnight_timer: Option<TimerId>,
    /// Between `Suspended` and the next `Resumed`: the windows have no surfaces to draw in.
    suspended: bool,
    /// The app was told it is hidden or frozen and has not been told it is back.
    away: bool,
    /// File dialogs the platform answers later, and what to call with the choice.
    dialogs: HashMap<remedy_ui_core::host::DialogId, DialogDone>,
    /// Permission requests the platform answers later, and what to call with the answer.
    permissions: HashMap<remedy_ui_core::host::PermissionId, PermissionDone>,
    /// Share sheets the platform closes later, and what to call with how they ended.
    shares: HashMap<remedy_ui_core::host::ShareId, ShareDone>,
    secret: Option<(crate::secure::SensitiveCopy, TimerId)>,
    inbox: crate::instance::SharedInbox,
    #[cfg(feature = "gpu")]
    gpu: Option<Option<remedy_ui_engine::render_gpu::GpuCompositor>>,
    /// The GPU device is being made without blocking (in a browser); windows draw on the CPU
    /// until the host's waker says it is ready.
    #[cfg(feature = "gpu")]
    gpu_waiting: bool,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Runner({:?}, {} windows)", self.config, self.wins.len())
    }
}

/// How a view's scroll request moves a scroll container.
#[derive(Clone, Copy, Debug)]
enum ScrollKind {
    /// To an offset; a snapping container to the snap point nearest it.
    To,
    /// To exactly the offset.
    Exact,
    /// A key step or focus move: a snapping container glides to the next snap point that way.
    Step,
}

impl ScrollKind {
    fn apply(self, c: &mut Compositor, layer: remedy_ui_engine::compositor::LayerId, offset: Vec2) -> bool {
        match self {
            ScrollKind::To => c.scroll_to_snapped(layer, offset),
            ScrollKind::Exact => c.set_scroll_offset(layer, offset),
            ScrollKind::Step => c.scroll_step(layer, offset),
        }
    }
}

impl Runner {
    /// A runner for `config`; `setup` runs when the host is ready.
    pub fn new(config: AppConfig, setup: impl FnOnce(&App) + 'static) -> Self {
        // The engine's own messages write numbers the locale's way (Arabic-Indic digits in
        // Arabic, a comma for decimals in German), from the same data the app's do.
        #[cfg(feature = "i18n")]
        remedy_ui_engine::view::strings::set_number_format(Some(std::rc::Rc::new(
            |locale, value, places| {
                let max = places.map_or(remedy_ui_platform::services_i18n::MAX_SCALE, |p| p.min(28) as u8);
                let d = remedy_ui_platform::services_i18n::Decimal::from_f64(value, max)?;
                Some(
                    remedy_ui_platform::services_i18n::NumberSymbols::for_locale(locale).format(d, d.scale()),
                )
            },
        )));
        let dir = config.data_dir.clone().or_else(|| data_dir(&config.name));
        // Images fetched from addresses are kept across runs in the app's cache folder (its
        // own data folder's `cache` when the app names one).
        #[cfg(feature = "image")]
        remedy_ui_media::media_image::set_cache_dir(
            config
                .data_dir
                .as_ref()
                .map(|d| d.join("cache"))
                .or_else(|| crate::geometry::cache_dir(&config.name))
                .map(|d| d.join("images")),
        );
        // What this machine's graphics backends did (passed their self-test, drew wrong,
        // hung presenting) is remembered with the app's data: tested once per driver, and a
        // backend that hung is skipped from the next start.
        #[cfg(feature = "gpu")]
        if let Some(d) = &dir {
            remedy_ui_engine::render_gpu::health::remember_in(d.join("gpu"));
        }
        let geometry =
            dir.map_or_else(GeometryStore::in_memory, |d| GeometryStore::open(d.join("windows.txt")));
        let runner = Self::with_geometry(config, geometry, setup);
        // Fallback faces a page fetched: every window's text is shaped again with them.
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        {
            let app = Rc::downgrade(&runner.app.0);
            crate::web_fonts::on_arrival(move || {
                let uis: Vec<Ui> =
                    app.upgrade().map(|s| s.borrow().uis.values().cloned().collect()).unwrap_or_default();
                for ui in uis {
                    ui.fonts_changed();
                }
            });
        }
        // A page has no files: its windows' geometry is kept with its persisted state.
        #[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
        let runner = {
            let mut runner = runner;
            if let Some(store) = runner.app.persist_store() {
                runner.geometry = GeometryStore::persisted_in(&store);
            }
            runner
        };
        runner
    }

    /// A runner keeping geometry in `geometry` (tests use an in-memory store).
    pub fn with_geometry(
        config: AppConfig,
        geometry: GeometryStore,
        setup: impl FnOnce(&App) + 'static,
    ) -> Self {
        let shared = Shared {
            name: config.name.clone(),
            app_id: config.app_id.clone().unwrap_or_default(),
            data_dir: config.data_dir.clone(),
            sheet: Some(config.sheet.clone()),
            ..Shared::default()
        };
        Runner {
            app: App(Rc::new(RefCell::new(shared))),
            config,
            setup: Some(Box::new(setup)),
            wins: HashMap::new(),
            by_host: HashMap::new(),
            timers: HashMap::new(),
            frame_timers: HashMap::new(),
            view_timers: HashMap::new(),
            geometry,
            save_timer: None,
            settings: SystemSettings::default(),
            locale: None,
            direction: remedy_ui_engine::view::WindowDirection::Locale,
            idle: None,
            activity_reported: None,
            idle_timer: None,
            midnight_timer: None,
            suspended: false,
            away: false,
            dialogs: HashMap::new(),
            permissions: HashMap::new(),
            shares: HashMap::new(),
            secret: None,
            inbox: Default::default(),
            #[cfg(feature = "gpu")]
            gpu: None,
            #[cfg(feature = "gpu")]
            gpu_waiting: false,
        }
    }

    pub(crate) fn with_inbox(mut self, inbox: crate::instance::SharedInbox) -> Self {
        self.inbox = inbox;
        self
    }

    fn launched(&mut self, args: Vec<String>, reply: Option<std::sync::mpsc::Sender<i32>>) {
        #[cfg(feature = "desktop")]
        self.app.links_in(&args);
        let cb = self.app.0.borrow().on_launch.clone();
        let answer = match cb {
            Some(cb) => cb(args),
            None => crate::instance::InstanceReply::code(0),
        };
        if let Some(to) = reply {
            answer.deliver(to);
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
                // A window in Chinese or Japanese needs its faces from its first frame.
                warm_fonts_for(self.locale.as_deref().unwrap_or(&self.settings.locale));
                let mut fonts = (self.config.fonts)();
                if let Some((families, files)) = &self.app.0.borrow().system_ui {
                    use_system_ui(&mut fonts, families, files);
                }
                let ui = Ui::new(self.config.theme.clone(), (self.config.sheet)(&self.config.theme), fonts);
                ui.set_edition(self.config.edition);
                ui.set_platform(self.settings.platform);
                ui.set_direction(self.direction);
                ui.set_locale(self.locale.clone().unwrap_or_else(|| self.settings.locale.clone()));
                ui.set_double_click_time(self.settings.double_click);
                ui.set_local_time(host.local_time());
                ui.set_time_zone(host.time_zone());
                let app = self.app.clone();
                ui.set_wake(Rc::new(move || app.wake_if_idle()));
                ui.set_reduced_motion(self.reduced_motion());
                ui.set_power_saving(self.settings.power_saving);
                ui.set_system_spatial_navigation(self.settings.spatial_navigation);
                ui.set_pointer_caps(self.settings.pointer);
                if let Some(hw) = self.settings.pointer_devices {
                    ui.set_pointer_hardware(hw);
                }
                if let Some(p) = self.app.0.borrow().ambient {
                    ui.set_ambient_policy(p);
                }
                ui.set_system_subpixel(self.settings.subpixel_text);
                // Content the platform draws itself (an embedded web view) parents to it.
                ui.set_window_handles(host.window_handles(id).ok());
                ui.set_refresh_interval(host.refresh_interval(id));
                let unhideable = !host.can_hide(id);
                {
                    let mut s = self.app.0.borrow_mut();
                    s.uis.insert(key, ui.clone());
                    if unhideable {
                        s.unhideable.insert(key);
                    }
                }
                // A window asked to open hidden where windows cannot hide opens minimized:
                // the platform shows it regardless.
                if unhideable && !spec.options.visible {
                    if !spec.hidden_until_drawn {
                        host.set_minimized(id, true);
                    }
                    self.app.note_visible(key, true);
                }
                self.by_host.insert(id, key);
                self.wins.insert(
                    key,
                    Win {
                        zoom: 1.0,
                        last_frame: None,
                        held: None,
                        pan: None,
                        cursor_hidden: false,
                        cursor: remedy_ui_engine::view::Cursor::Default,
                        autoscroll: None,
                        host: id,
                        ui: ui.clone(),
                        comp: None,
                        size: (1, 1, 1.0),
                        spec,
                        present: None,
                        wants_gpu: false,
                        awaiting_gpu: false,
                        cost: FrameCost::default(),
                        scrollers: Vec::new(),
                        pending_scrolls: Vec::new(),
                        cover: None,
                    },
                );
                let window = Window { key, app: self.app.clone() };
                // The window's content is built in its root's scope, with the app's persisted
                // state folder provided, so what it builds reads it as context.
                let content = ui.root();
                match content.scope() {
                    Some(scope) => {
                        if let Some(store) = self.app.persist_store() {
                            scope.provide_context(store);
                        }
                        scope.provide_context(self.app.clone());
                        let _ = scope.run(|| build(&ui, &window));
                    }
                    None => build(&ui, &window),
                }
            }
            Cmd::Close(key) => {
                if let Some(h) = self.host_of(key) {
                    host.close_window(h);
                }
            }
            Cmd::Visible(key, on) => {
                if let Some(h) = self.host_of(key) {
                    if !on && !host.can_hide(h) {
                        // Asked before the window existed (Window::can_hide could not tell
                        // yet): minimize it, and it stays shown.
                        host.set_minimized(h, true);
                        self.app.note_visible(key, true);
                    } else {
                        host.set_visible(h, on);
                        if on {
                            // What it showed was not kept while it was hidden: all of it again.
                            if let Some(c) = self.wins.get_mut(&key).and_then(|w| w.comp.as_mut()) {
                                c.damage_all();
                            }
                            host.request_frame(h);
                        }
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
            Cmd::Size(key, size) => {
                if let Some(h) = self.host_of(key) {
                    let _ = host.set_size(h, size);
                }
            }
            Cmd::Center(key) => {
                if let Some(h) = self.host_of(key) {
                    // Where the platform places windows itself, it already chose.
                    let _ = host.center_window(h);
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
            Cmd::Zoom(key, zoom) => {
                if let Some(w) = self.wins.get_mut(&key)
                    && (w.zoom - zoom).abs() > f32::EPSILON
                {
                    w.zoom = zoom;
                    let (pw, ph, s) = (w.size.0, w.size.1, w.scale());
                    if let Some(c) = w.comp.as_mut() {
                        c.resize(pw, ph, s);
                    }
                    let h = w.host;
                    host.request_frame(h);
                }
            }
            Cmd::ReduceMotion => self.apply_reduced_motion(),
            Cmd::HideOnClose(key, on) => {
                if let Some(w) = self.wins.get_mut(&key) {
                    w.spec.hide_on_close = on;
                }
            }
            Cmd::MinimizeOnClose(key, on) => {
                if let Some(w) = self.wins.get_mut(&key) {
                    w.spec.minimize_on_close = on;
                }
            }
            Cmd::Fullscreen(key, on) => {
                if let Some(h) = self.host_of(key) {
                    // A host without full-screen windows leaves it as it was; the window's
                    // `is_fullscreen` says what happened.
                    let _ = host.set_fullscreen(h, on);
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
            Cmd::Icon(key, icon) => {
                if let Some(h) = self.host_of(key) {
                    // Where windows have no icons of their own, the app's stays: nothing to do.
                    let _ = host.set_window_icon(h, icon.as_ref());
                }
            }
            Cmd::Timer(delay, f) => {
                let id = host.set_timer(host.now() + delay);
                self.timers.insert(id, f);
            }
            Cmd::Copy(text, sensitive) => {
                let _ = host.set_clipboard_text(&text, sensitive);
            }
            Cmd::CopySecret(text, keep) => self.copy_secret(host, &text, keep),
            Cmd::Locale(locale) => {
                self.locale = locale;
                let tag = self.locale.clone().unwrap_or_else(|| self.settings.locale.clone());
                for w in self.wins.values() {
                    w.ui.set_locale(tag.clone());
                    host.request_frame(w.host);
                }
                self.publish_locale();
            }
            Cmd::Direction(direction) => {
                self.direction = direction;
                for w in self.wins.values() {
                    w.ui.set_direction(direction);
                    host.request_frame(w.host);
                }
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
                    (slot, Some(cb)) => *slot = Some((crate::secure::IdleTimer::new(timeout, now), cb)),
                    (None, None) => {}
                }
                self.schedule_idle(host);
            }
            Cmd::Tray(t) => {
                // Whether an icon is up now ([`App::tray_shown`]).
                // The host is always told, `None` included: that is what removes the icon.
                let shown = host.set_tray(t.as_ref());
                let up = t.is_some() && shown.is_ok();
                self.app.tray_signal().set_if_changed(up);
            }
            Cmd::Dialog(key, dialog, done) => {
                let w = key.and_then(|k| self.host_of(k));
                match host.file_dialog(w, &dialog) {
                    Ok(remedy_ui_core::host::DialogStarted::Finished(chosen)) => done(Ok(chosen)),
                    // The platform's picker answers later, with `HostEvent::FileDialog`.
                    Ok(remedy_ui_core::host::DialogStarted::Pending(id)) => {
                        self.dialogs.insert(id, done);
                    }
                    Err(e) => done(Err(e)),
                }
            }
            Cmd::OpenUrl(url, done) => done(host.open_url(&url)),
            Cmd::Notify(mut notification, done) => {
                if notification.app.is_empty() {
                    notification.app = self.app.0.borrow().name.clone();
                }
                done(host.show_notification(&notification));
            }
            Cmd::Withdraw(id) => {
                let _ = host.withdraw_notification(id);
            }
            Cmd::Badge(badge) => {
                let id = self.app.0.borrow().app_id.clone();
                let _ = host.set_badge(&id, badge);
            }
            Cmd::Gamepad(tuning) => host.set_gamepad_tuning(tuning),
            Cmd::Foreground(mut task, done) => {
                if task.notification.app.is_empty() {
                    task.notification.app = self.app.0.borrow().name.clone();
                }
                done(host.start_foreground(&task));
            }
            Cmd::StopForeground => {
                let _ = host.stop_foreground();
            }
            Cmd::Permission(permission, done) => match host.request_permission(permission) {
                Ok(remedy_ui_core::host::PermissionStarted::Finished(granted)) => done(granted),
                // The platform's prompt answers later, with `HostEvent::Permission`.
                Ok(remedy_ui_core::host::PermissionStarted::Pending(id)) => {
                    self.permissions.insert(id, done);
                }
                Err(_) => done(false),
            },
            Cmd::Print(key, title, html, done) => {
                let w = key.and_then(|k| self.host_of(k));
                done(host.print_html(w, &title, &html));
            }
            Cmd::Share(key, share, done) => {
                if share.is_empty() {
                    done(remedy_ui_core::host::ShareOutcome::Unsupported(Unsupported::new(
                        "sharing",
                        "there is nothing to share: no title, text or files",
                    )));
                    return;
                }
                let w = key.and_then(|k| self.host_of(k));
                match host.share(w, &share) {
                    Ok(remedy_ui_core::host::ShareStarted::Finished(outcome)) => done(outcome),
                    // The platform's sheet closes later, with `HostEvent::Share`.
                    Ok(remedy_ui_core::host::ShareStarted::Pending(id)) => {
                        self.shares.insert(id, done);
                    }
                    Err(why) => done(remedy_ui_core::host::ShareOutcome::Unsupported(why)),
                }
            }
            Cmd::Forget(key) => {
                self.geometry.remove(&key);
                self.schedule_save(host);
            }
            Cmd::Quit => {
                // In the order the windows were opened; the first refusal stops the quit.
                let mut keys: Vec<u64> = self.wins.keys().copied().collect();
                keys.sort_unstable();
                if keys.into_iter().all(|key| self.may_close(key, CloseReason::Quit)) {
                    self.save_now(host, Leaving::Quitting);
                    host.exit();
                }
            }
            Cmd::QuitNow => {
                self.save_now(host, Leaving::Quitting);
                host.exit();
            }
        }
    }

    /// Puts a secret on the clipboard (out of history and sync) and clears it after `keep`
    /// if the clipboard still holds it; a newer secret replaces the older one's timer.
    fn copy_secret(&mut self, host: &mut dyn Host, text: &str, keep: Duration) {
        if host.set_clipboard_text(text, true).is_ok() {
            let copy = crate::secure::SensitiveCopy::new(text, host.now(), keep);
            if let Some((_, old)) = self.secret.take() {
                host.cancel_timer(old);
            }
            let t = host.set_timer(copy.clear_at());
            self.secret = Some((copy, t));
        }
    }

    fn requests(&mut self, host: &mut dyn Host, key: u64) {
        let Some(w) = self.wins.get(&key) else { return };
        let (h, ui) = (w.host, w.ui.clone());
        let hide = ui.cursor_hidden();
        if hide != w.cursor_hidden {
            host.set_cursor_hidden(h, hide);
            if let Some(w) = self.wins.get_mut(&key) {
                w.cursor_hidden = hide;
            }
        }
        let shape = ui.cursor();
        if self.wins.get(&key).is_some_and(|w| w.cursor != shape) {
            host.set_cursor(h, shape);
            if let Some(w) = self.wins.get_mut(&key) {
                w.cursor = shape;
            }
        }
        for r in ui.take_requests() {
            match r {
                Request::Frame => host.request_frame(h),
                Request::TextInput(mut s) => {
                    // The input method places its window in the host's pixels.
                    if let (Some(st), Some(z)) =
                        (s.as_mut(), self.wins.get(&key).map(|w| w.zoom).filter(|z| *z != 1.0))
                    {
                        let c = st.caret;
                        st.caret =
                            remedy_ui_core::Rect::new(c.min.x * z, c.min.y * z, c.max.x * z, c.max.y * z);
                    }
                    host.set_text_input(h, s)
                }
                Request::PreferGpu => {
                    #[cfg(feature = "gpu")]
                    if self.config.renderer == RendererChoice::Auto
                        && let Some(w) = self.wins.get_mut(&key)
                        && !w.wants_gpu
                    {
                        // The next frame starts this window on the GPU (as a slow window does).
                        w.wants_gpu = true;
                        w.present = None;
                        if let Some(c) = w.comp.as_mut() {
                            c.invalidate_tiles();
                        }
                        host.request_frame(h);
                    }
                }
                Request::ScrollTo { scroller, offset }
                | Request::ScrollToExact { scroller, offset }
                | Request::ScrollStep { scroller, offset } => {
                    let kind = match r {
                        Request::ScrollToExact { .. } => ScrollKind::Exact,
                        Request::ScrollStep { .. } => ScrollKind::Step,
                        _ => ScrollKind::To,
                    };
                    if let Some(w) = self.wins.get_mut(&key) {
                        let layer = w.scrollers.iter().find(|(_, e)| *e == scroller).map(|(l, _)| *l);
                        match (layer, w.comp.as_mut()) {
                            (Some(l), Some(c)) => {
                                if kind.apply(c, l, offset) {
                                    host.request_frame(h);
                                }
                            }
                            // Not drawn yet: the next frame that gives it a layer scrolls it.
                            _ => {
                                w.pending_scrolls.retain(|(e, _, _)| *e != scroller);
                                w.pending_scrolls.push((scroller, offset, kind));
                                host.request_frame(h);
                            }
                        }
                    }
                }
                Request::StartDrag { data, allowed } => {
                    match host.start_drag(h, &data, &allowed) {
                        Ok(remedy_ui_core::host::DragStarted::Finished(effect)) => ui.drag_finished(effect),
                        // The host reports the end with `HostEvent::DragEnded`.
                        Ok(remedy_ui_core::host::DragStarted::Pending) => {}
                        Err(_) => ui.drag_finished(remedy_ui_core::host::DropEffect::None),
                    }
                    host.request_frame(h);
                }
                Request::Copy { text, sensitive, html } => {
                    let _ = match html {
                        Some(html) if !sensitive => host.set_clipboard_html(&text, &html),
                        _ => host.set_clipboard_text(&text, sensitive),
                    };
                }
                Request::CopySecret { text, keep } => self.copy_secret(host, &text, keep),
                Request::Paste => {
                    if let Ok(Some(t)) = host.clipboard_text() {
                        let html = host.clipboard_html().ok().flatten();
                        ui.paste_html(&t, html.as_deref());
                        host.request_frame(h);
                    }
                }
                Request::SetPrimary(text) => {
                    // Platforms without a primary selection have nothing to set.
                    let _ = host.set_primary_selection(&text);
                }
                Request::PastePrimary { fallback } => {
                    let text = match host.primary_selection_text() {
                        Ok(t) => t,
                        Err(_) => fallback,
                    };
                    if let Some(t) = text {
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

    /// Motion is reduced: the system asks for it, or the app does ([`App::set_reduce_motion`]).
    fn reduced_motion(&self) -> bool {
        self.settings.reduced_motion || self.app.0.borrow().reduce_motion
    }

    /// Tells every window and its compositor whether motion is reduced now.
    fn apply_reduced_motion(&mut self) {
        let on = self.reduced_motion();
        for w in self.wins.values_mut() {
            w.ui.set_reduced_motion(on);
            if let Some(c) = w.comp.as_mut() {
                c.set_reduced_motion(on);
            }
        }
    }

    /// The user asked to leave a window (its close button, or a back action nothing took):
    /// minimize or hide it when the app keeps it, else close it.
    fn close_requested(&mut self, host: &mut dyn Host, window: WindowId) {
        let Some(&key) = self.by_host.get(&window) else { return };
        let (hide, minimize) =
            self.wins.get(&key).map_or((false, false), |w| (w.spec.hide_on_close, w.spec.minimize_on_close));
        // Where a window cannot leave the screen while open (Wayland), keeping it means
        // minimizing it: a hide the platform ignores would leave it up but reported hidden.
        if minimize || (hide && !host.can_hide(window)) {
            host.set_minimized(window, true);
        } else if hide {
            host.set_visible(window, false);
            self.app.note_visible(key, false);
        } else if self.may_close(key, CloseReason::Window) {
            host.close_window(window);
        }
    }

    /// Asks the window's close-request handler, if it has one. The handler is called with no
    /// borrow held, so it may open a prompt, close windows or quit.
    fn may_close(&self, key: u64, why: CloseReason) -> bool {
        let ask = self.app.0.borrow().close_asks.get(&key).cloned();
        ask.is_none_or(|f| f(why))
    }

    /// Writes window geometry and persisted state now: before the app quits, and when it may
    /// be stopped without warning (suspended, or out of sight: a mobile system or a browser
    /// can end a background app or a hidden tab at any time).
    /// Geometry is written when a write is due (or `always`, when the app quits).
    fn save_now(&mut self, host: &mut dyn Host, why: Leaving) {
        let always = why == Leaving::Quitting;
        self.away |= matches!(why, Leaving::Hidden | Leaving::Frozen);
        self.leaving(why);
        let due = self.save_timer.take().inspect(|t| host.cancel_timer(*t)).is_some();
        if due || always {
            let _ = self.geometry.save();
        }
        let store = self.app.0.borrow().persist.clone();
        if let Some(s) = store {
            let _ = s.flush();
        }
    }

    /// Tells the app it is back, once, after it was told it was hidden or frozen.
    fn returned(&mut self) {
        if std::mem::take(&mut self.away) {
            let f = self.app.0.borrow().on_return.clone();
            if let Some(f) = f {
                f();
            }
        }
    }

    /// Tells the app it may be ended ([`App::on_leaving`]).
    fn leaving(&self, why: Leaving) {
        let f = self.app.0.borrow().on_leaving.clone();
        if let Some(f) = f {
            f(why);
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

    /// Reads whether the window is maximized or fills the screen from the host into its
    /// signals.
    fn sync_maximized(&mut self, host: &dyn Host, key: u64) {
        let Some(h) = self.host_of(key) else { return };
        let (max, full) = (host.is_maximized(h), host.is_fullscreen(h));
        let (s, f) = {
            let shared = self.app.0.borrow();
            (shared.maximized.get(&key).copied(), shared.fullscreen.get(&key).copied())
        };
        if let Some(s) = s {
            s.set_if_changed(max);
        }
        if let Some(f) = f {
            f.set_if_changed(full);
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

    /// Once the GPU device being made is ready (or failed), windows that waited for it on the
    /// CPU choose their renderer again.
    #[cfg(feature = "gpu")]
    fn gpu_resolved(&mut self, host: &mut dyn Host) {
        let ladder = remedy_ui_engine::render_gpu::LadderOptions::default();
        match remedy_ui_engine::render_gpu::GpuContext::shared(&ladder) {
            Err(remedy_ui_engine::render_gpu::GpuError::Pending) => return,
            Ok(ctx) => {
                let renderer = remedy_ui_engine::render_gpu::GpuRenderer::new(ctx, None);
                self.gpu = Some(Some(remedy_ui_engine::render_gpu::GpuCompositor::new(renderer, ladder)));
            }
            // It could not be made: the CPU renderer from now on.
            Err(_) => self.gpu = Some(None),
        }
        self.gpu_waiting = false;
        for w in self.wins.values_mut().filter(|w| w.awaiting_gpu) {
            w.awaiting_gpu = false;
            w.present = None;
            if let Some(c) = w.comp.as_mut() {
                c.invalidate_tiles();
            }
            host.request_frame(w.host);
        }
    }

    /// Routes a wheel scroll (in the content's logical pixels) in `window`.
    fn wheel(&mut self, host: &mut dyn Host, window: WindowId, e: &remedy_ui_core::input::WheelEvent) {
        // Elements that take the wheel themselves (zoomable documents, Ctrl+wheel)
        // come first; otherwise it scrolls on the compositor.
        let taken = self
            .by_host
            .get(&window)
            .and_then(|k| self.wins.get(k))
            .map(|w| w.ui.clone())
            .is_some_and(|ui| ui.wheel(e));
        if taken {
            if let Some(&key) = self.by_host.get(&window) {
                self.requests(host, key);
            }
            host.request_frame(window);
        } else if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get_mut(k)) {
            let s = w.scale();
            if let Some(c) = w.comp.as_mut() {
                c.wheel_at(
                    Point::new(e.position.x * s, e.position.y * s),
                    Vec2::new(e.delta.x * s, e.delta.y * s),
                );
            }
            host.request_frame(window);
        }
    }

    fn frame(&mut self, host: &mut dyn Host, key: u64, at: Duration) {
        if self.suspended {
            return;
        }
        // Middle-button autoscroll moves on every frame while it lasts.
        if let Some(w) = self.wins.get_mut(&key)
            && let Some(a) = w.autoscroll.as_mut()
        {
            let dt = a.last.map_or(0.0, |l| at.saturating_sub(l).as_secs_f32().min(0.1));
            a.last = Some(at);
            let (delta, origin, window) = (a.delta(dt), a.origin, w.host);
            if delta != Vec2::ZERO {
                self.wheel(
                    host,
                    window,
                    &remedy_ui_core::input::WheelEvent::new(origin, delta).with_precise(true),
                );
            }
            host.request_frame(window);
        }
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
        #[cfg(feature = "gpu")]
        let vsync = self.config.vsync;
        // The app's frame-rate limit: a frame asked for sooner waits for its turn.
        let limit = self.app.0.borrow().frame_limit;
        let now = host.now();
        if let (Some(fps), Some(w)) = (limit, self.wins.get_mut(&key)) {
            let gap = Duration::from_secs(1) / fps;
            if let Some(last) = w.last_frame
                && now < last + gap
            {
                if w.held.is_none() {
                    let t = host.set_timer(last + gap);
                    w.held = Some(t);
                    self.frame_timers.insert(t, key);
                }
                return;
            }
        }
        let Some(w) = self.wins.get_mut(&key) else { return };
        w.last_frame = Some(now);
        let _frame = remedy_ui_core::phase!("frame", ui = w.ui.id()).entered();
        let (pw, ph, s) = (w.size.0, w.size.1, w.scale());
        // Animations and painters sample the frame's presentation time.
        w.ui.set_frame_time(at);
        let f = w.ui.frame(Size::new(pw as f32 / s, ph as f32 / s), s);
        if let Some(update) = w.ui.take_accessibility_update() {
            host.update_accessibility(w.host, update);
        }
        let Some(comp) = w.comp.as_mut() else { return };
        if let Some(l) = f.layers {
            comp.commit(l);
        }
        comp.set_ambient(w.ui.tick_interval(remedy_ui_engine::view::TickKind::Ambient));
        w.scrollers = f.scrollers;
        if !w.pending_scrolls.is_empty() {
            let tree = w.ui.tree();
            let scrollers = &w.scrollers;
            let mut moved = false;
            w.pending_scrolls.retain(|(el, offset, kind)| {
                match scrollers.iter().find(|(_, e)| e == el) {
                    Some((layer, _)) => {
                        moved |= kind.apply(comp, *layer, *offset);
                        false
                    }
                    // Kept while the container lives: it may overflow (and get its layer) later.
                    None => tree.contains(*el),
                }
            });
            if moved {
                host.request_frame(w.host);
            }
        }
        // A picture asked for. A window the GPU draws reads back the frame it keeps after this
        // frame's present: what the screen shows, effects, video and app textures included.
        // Otherwise this frame draws everything, and a CPU renderer of its own draws it once
        // more into the picture.
        let mut snaps = self.app.0.borrow_mut().snapshots.remove(&key).unwrap_or_default();
        #[cfg(feature = "gpu")]
        let on_gpu = matches!(w.present, Some(Present::Gpu(_)));
        #[cfg(not(feature = "gpu"))]
        let on_gpu = false;
        if !snaps.is_empty() && !on_gpu {
            comp.invalidate_tiles();
        }
        let plan = comp.frame(at);
        if !on_gpu {
            let picture = (!snaps.is_empty()).then(|| {
                let background = if w.spec.options.transparent {
                    Color::TRANSPARENT
                } else {
                    w.spec.background.unwrap_or(bg)
                };
                CpuCompositor::new()
                    .present(&plan, pw, ph, background)
                    .ok()
                    .map(|f| remedy_ui_engine::paint::Image::new(f.width, f.height, Arc::from(f.pixels)))
            });
            // Answered at once: the callbacks reach the app only through its handles, never
            // this frame's state, and a later early return must not lose them.
            for done in snaps.drain(..) {
                done(picture.clone().flatten());
            }
        }
        // Pictures still waiting are the GPU's, answered after the present below (or with
        // nothing, if the window cannot draw).
        #[cfg_attr(not(feature = "gpu"), allow(unused_mut))]
        let mut gpu_snaps = snaps;
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
                // Where the first present fixes a surface's renderer, start on the GPU.
                RendererChoice::Auto => w.wants_gpu || host.renderer_fixed_per_surface(),
                RendererChoice::CpuFirst => w.wants_gpu,
                RendererChoice::Cpu => false,
            };
            #[cfg(feature = "gpu")]
            if gpu_wanted {
                if let Some(gpu) = gpu_device(&mut self.gpu, &mut self.gpu_waiting, host, w.host) {
                    // A page's canvas can show through: its elements (a playing `<video>`) show in the
                    // holes the window leaves clear (`El::set_see_through`), the background stays opaque.
                    let see_through = w.spec.options.transparent || cfg!(target_arch = "wasm32");
                    let surface = host
                        .window_handles(w.host)
                        .ok()
                        .and_then(|hd| gpu.create_surface_paced(hd, pw, ph, see_through, vsync).ok());
                    if self.app.0.borrow().adapter.is_none() {
                        let a = gpu.adapter();
                        self.app.0.borrow_mut().adapter = Some(GraphicsAdapter {
                            name: a.name,
                            vendor: a.vendor,
                            driver: a.driver,
                            api: a.api,
                            software: a.software,
                        });
                    }
                    if let Some(sf) = surface {
                        w.present = Some(Present::Gpu(Box::new(sf)));
                    }
                }
                w.awaiting_gpu = w.present.is_none() && self.gpu_waiting;
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
            w.ui.set_gpu_rendering(kind == RendererKind::Gpu);
            host.renderer_changed(w.host, kind == RendererKind::Gpu);
        }
        let Some(w) = self.wins.get_mut(&key) else { return };
        #[cfg_attr(not(feature = "gpu"), allow(unused_mut))]
        let mut again = plan.animating;
        if changed || again {
            match w.present.as_mut() {
                Some(Present::Cpu(cpu)) => {
                    let started = remedy_ui_core::time::Instant::now();
                    if changed && let Ok(frame) = cpu.present_frame(&plan, pw, ph, background) {
                        let pixels = Pixels { width: pw, height: ph, data: &frame.pixels };
                        let present = remedy_ui_core::phase!("present").entered();
                        let _ = host.present_pixels(w.host, pixels, plan.damage.rects());
                        drop(present);
                        // What rastering cost here sizes the next zoom's per-frame budget.
                        let (px, took) = cpu.last_raster();
                        if let Some(c) = w.comp.as_mut() {
                            c.record_raster(px, took);
                        }
                    }
                    // Too slow on the CPU: the next frame starts this window on the GPU.
                    #[cfg(feature = "gpu")]
                    if changed
                        && matches!(self.config.renderer, RendererChoice::Auto | RendererChoice::CpuFirst)
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
                            let recovered = gpu.recover_device().is_ok();
                            if !recovered {
                                // No replacement now (a browser makes one asynchronously):
                                // every window chooses its renderer again, drawing on the CPU
                                // until a new device is ready.
                                self.gpu = None;
                                self.gpu_waiting = false;
                            }
                            for other in self.wins.values_mut() {
                                if !recovered && matches!(other.present, Some(Present::Gpu(_))) {
                                    other.present = None;
                                }
                                if let Some(c) = other.comp.as_mut() {
                                    c.invalidate_tiles();
                                }
                                host.request_frame(other.host);
                            }
                            // The kept frame went with the device: ask again next frame.
                            if !gpu_snaps.is_empty() {
                                self.app
                                    .0
                                    .borrow_mut()
                                    .snapshots
                                    .entry(key)
                                    .or_default()
                                    .append(&mut gpu_snaps);
                            }
                            return;
                        }
                        let started = remedy_ui_core::time::Instant::now();
                        match gpu.present_to_surface(&plan, surface, background) {
                            Ok(remedy_ui_engine::render_gpu::Presented::Retry) => again = true,
                            Ok(_) => {
                                // The GPU's whole present time (recording, submitting, waiting
                                // for the swapchain) bounds what its rastering cost.
                                let px: u64 = plan.raster.iter().map(|r| r.pixels()).sum();
                                if let Some(c) = w.comp.as_mut() {
                                    c.record_raster(px, started.elapsed());
                                }
                            }
                            Err(_) => {
                                // The GPU failed this window: fall back to the CPU renderer.
                                w.present = Some(Present::Cpu(Box::new(CpuCompositor::new())));
                                self.app.0.borrow_mut().renderers.insert(key, RendererKind::Cpu);
                                w.ui.set_gpu_rendering(false);
                                host.renderer_changed(w.host, false);
                                if let Some(c) = w.comp.as_mut() {
                                    c.invalidate_tiles();
                                }
                                again = true;
                                // Pictures asked for come from the CPU's next frame.
                                if !gpu_snaps.is_empty() {
                                    self.app
                                        .0
                                        .borrow_mut()
                                        .snapshots
                                        .entry(key)
                                        .or_default()
                                        .append(&mut gpu_snaps);
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }
        if !gpu_snaps.is_empty() {
            #[cfg(feature = "gpu")]
            let picture = match (self.gpu.as_ref(), self.wins.get(&key).and_then(|w| w.present.as_ref())) {
                (Some(Some(gpu)), Some(Present::Gpu(surface))) => gpu
                    .read_surface(surface)
                    .and_then(Result::ok)
                    .map(|f| remedy_ui_engine::paint::Image::new(f.width, f.height, Arc::from(f.pixels))),
                _ => None,
            };
            #[cfg(not(feature = "gpu"))]
            let picture = None;
            for done in gpu_snaps {
                done(picture.clone());
            }
        }
        if again && let Some(w) = self.wins.get(&key) {
            host.request_frame(w.host);
        } else if let (Some(wait), Some(w)) = (plan.wake_in, self.wins.get_mut(&key))
            && w.held.is_none()
        {
            // Ambient loops on the compositor: the next frame at their rate, not the next vsync.
            let t = host.set_timer(now + wait);
            w.held = Some(t);
            self.frame_timers.insert(t, key);
        }
    }
}

/// AccessKit's action as the engine's, for the actions elements offer.
fn action(a: remedy_ui_core::host::accesskit::Action) -> Option<remedy_ui_engine::view::A11yAction> {
    use remedy_ui_core::host::accesskit::Action as K;
    use remedy_ui_engine::view::A11yAction as A;
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

/// The shared GPU compositor, made on first use. Natively the device is made here. A browser
/// makes it asynchronously: the first call asks for it and returns `None` (the window draws on
/// the CPU meanwhile, `waiting` set), and the host's waker runs once it is ready or has
/// failed ([`Runner::gpu_resolved`] then moves the waiting windows).
#[cfg(feature = "gpu")]
fn gpu_device<'a>(
    slot: &'a mut Option<Option<remedy_ui_engine::render_gpu::GpuCompositor>>,
    waiting: &mut bool,
    host: &mut dyn Host,
    window: WindowId,
) -> Option<&'a remedy_ui_engine::render_gpu::GpuCompositor> {
    if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
        if slot.is_none() {
            let ladder = remedy_ui_engine::render_gpu::LadderOptions::default();
            match remedy_ui_engine::render_gpu::GpuContext::shared(&ladder) {
                Ok(ctx) => {
                    *waiting = false;
                    let renderer = remedy_ui_engine::render_gpu::GpuRenderer::new(ctx, None);
                    *slot = Some(Some(remedy_ui_engine::render_gpu::GpuCompositor::new(renderer, ladder)));
                }
                Err(remedy_ui_engine::render_gpu::GpuError::Pending) => *waiting = true,
                Err(_) if !*waiting => match host.window_handles(window) {
                    Ok(handles) => {
                        *waiting = true;
                        let wake = host.waker();
                        remedy_ui_engine::render_gpu::GpuContext::prepare_shared(
                            &ladder,
                            handles,
                            move |_| wake(),
                        );
                    }
                    Err(_) => *slot = Some(None),
                },
                // It was asked for and could not be made: the CPU renderer from now on.
                Err(_) => {
                    *waiting = false;
                    *slot = Some(None);
                }
            }
        }
        slot.as_ref().and_then(Option::as_ref)
    } else {
        slot.get_or_insert_with(new_gpu).as_ref()
    }
}

#[cfg(feature = "gpu")]
fn new_gpu() -> Option<remedy_ui_engine::render_gpu::GpuCompositor> {
    let ladder = remedy_ui_engine::render_gpu::LadderOptions::default();
    // The process's shared device: GPU content the app embeds renders on the same one.
    let ctx = remedy_ui_engine::render_gpu::GpuContext::shared(&ladder).ok()?;
    Some(remedy_ui_engine::render_gpu::GpuCompositor::new(
        remedy_ui_engine::render_gpu::GpuRenderer::new(ctx, None),
        ladder,
    ))
}

impl remedy_ui_core::host::App for Runner {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        // Edits group into undo steps by when they happen: every window reads the host's time.
        let now = host.now();
        for w in self.wins.values() {
            w.ui.set_clock(now);
        }
        {
            let mut s = self.app.0.borrow_mut();
            s.in_event = true;
            if s.waker.is_none() {
                let w = host.waker();
                s.waker = Some(w.clone());
                // Work other threads post to the UI thread (decoded images, finished sorts,
                // reloaded messages) wakes the loop, and runs below.
                let _ = remedy_ui_core::reactive::set_wake_handler(move || w());
            }
        }
        {
            let mut inbox = self.inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if inbox.waker.is_none() {
                inbox.waker = Some(host.waker());
            }
        }
        // A GPU device made asynchronously (in a browser) is ready, or could not be made.
        #[cfg(feature = "gpu")]
        if self.gpu_waiting {
            self.gpu_resolved(host);
        }
        // Background work that finished.
        self.app.finish_background();
        // Closures other threads posted through the reactive waker.
        if remedy_ui_core::reactive::has_posted() {
            let _ = remedy_ui_core::reactive::run_posted();
        }
        // Launches handed over by later instances.
        let launches = std::mem::take(
            &mut self.inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).launches,
        );
        for l in launches {
            self.launched(l.args, l.reply);
        }
        match event {
            HostEvent::Resumed => {
                self.suspended = false;
                self.returned();
                remedy_ui_engine::view::device::set_total_memory(host.total_memory());
                if let Some(setup) = self.setup.take() {
                    #[cfg(all(
                        feature = "playback",
                        not(any(target_arch = "wasm32", target_os = "android", target_os = "ios"))
                    ))]
                    crate::pip::install(&self.app);
                    setup(&self.app);
                } else {
                    // Back from a suspension: the windows are seen again, on new surfaces.
                    for w in self.wins.values() {
                        w.ui.set_occluded(false);
                        host.request_frame(w.host);
                    }
                }
            }
            HostEvent::Settings(s) => {
                for w in self.wins.values() {
                    w.ui.set_platform(s.platform);
                    w.ui.set_locale(self.locale.clone().unwrap_or_else(|| s.locale.clone()));
                    w.ui.set_double_click_time(s.double_click);
                    w.ui.set_system_subpixel(s.subpixel_text);
                    w.ui.set_power_saving(s.power_saving);
                    w.ui.set_system_spatial_navigation(s.spatial_navigation);
                    w.ui.set_pointer_caps(s.pointer);
                    // Only when the hardware changed: other settings leave the last pointer's say.
                    if s.pointer_devices != self.settings.pointer_devices
                        && let Some(hw) = s.pointer_devices
                    {
                        w.ui.set_pointer_hardware(hw);
                    }
                }
                let appearance = (s.dark, s.high_contrast);
                self.app.0.borrow_mut().preferred_locales.clone_from(&s.locales);
                self.settings = s;
                self.apply_reduced_motion();
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
                let reduced = self.reduced_motion();
                let Some(w) = self.wins.get_mut(&key) else { return };
                // A window with no area (minimized on Windows) shows nothing; it shows again
                // when it has area. Platforms that say so (`Occluded`) are told separately.
                let was_flat = w.size.0 == 0 || w.size.1 == 0;
                let flat = width == 0 || height == 0;
                if flat != was_flat {
                    w.ui.set_occluded(flat);
                }
                w.size = (width, height, scale);
                let zoomed = w.scale();
                match w.comp.as_mut() {
                    Some(c) => c.resize(width, height, zoomed),
                    None => {
                        let mut c = Compositor::new(width, height, zoomed, ScrollPhysics::CLAMPED);
                        c.set_reduced_motion(reduced);
                        w.comp = Some(c);
                    }
                }
                #[cfg(feature = "gpu")]
                if let (Some(Present::Gpu(sf)), Some(Some(gpu))) = (w.present.as_mut(), self.gpu.as_ref()) {
                    gpu.resize_surface(sf, width, height);
                }
                w.ui.set_refresh_interval(host.refresh_interval(window));
                host.request_frame(window);
                self.sync_maximized(host, key);
                self.remember_geometry(host, key);
            }
            HostEvent::Occluded(window, covered) => {
                if let Some(&key) = self.by_host.get(&window) {
                    self.sync_maximized(host, key);
                }
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get_mut(k)) {
                    w.ui.set_occluded(covered);
                    // Back in sight: the platform may not have kept what the window showed
                    // (X11 without a compositor), so the next frame presents all of it.
                    if !covered {
                        if let Some(c) = w.comp.as_mut() {
                            c.damage_all();
                        }
                        host.request_frame(window);
                    }
                }
                // Out of sight altogether (a page's tab hidden, every window minimized): the
                // system may end the app from here on, so what it keeps is written now.
                if covered && self.wins.values().all(|w| w.ui.is_occluded()) {
                    self.save_now(host, Leaving::Hidden);
                } else if !covered {
                    self.returned();
                }
            }
            HostEvent::Moved(window, _) => {
                if let Some(&key) = self.by_host.get(&window) {
                    if let Some(w) = self.wins.get(&key) {
                        w.ui.set_refresh_interval(host.refresh_interval(window));
                    }
                    // Drag-unmaximize and some snap restores fire a move (sometimes
                    // before, or without, a resize). The title-bar icon must follow.
                    self.sync_maximized(host, key);
                    self.remember_geometry(host, key);
                }
            }
            HostEvent::Frame { window, at } => {
                if let Some(&key) = self.by_host.get(&window) {
                    self.frame(host, key, at);
                }
            }
            HostEvent::Pointer(window, mut e) => {
                self.activity(host);
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get_mut(k)) {
                    if w.zoom != 1.0 {
                        e.position = unzoom(e.position, w.zoom);
                        for (p, _) in &mut e.coalesced {
                            *p = unzoom(*p, w.zoom);
                        }
                    }
                    if let Some(a) = w.autoscroll.as_mut() {
                        // While autoscrolling the pointer steers it and nothing else.
                        let middle = e.buttons.contains(Buttons::MIDDLE);
                        a.at = e.position;
                        let far = (a.at.x - a.origin.x).abs().max((a.at.y - a.origin.y).abs())
                            > Autoscroll::DEAD_ZONE;
                        let end = match e.phase {
                            PointerPhase::Down => !(a.held && middle && e.buttons == Buttons::MIDDLE),
                            PointerPhase::Up | PointerPhase::Cancel => {
                                let released = a.held && !middle;
                                if released {
                                    a.held = false;
                                }
                                (released && a.dragged) || e.phase == PointerPhase::Cancel
                            }
                            _ => {
                                if a.held && far {
                                    a.dragged = true;
                                }
                                false
                            }
                        };
                        if end {
                            w.autoscroll = None;
                        }
                        host.pointer_consumed(window, true);
                        host.request_frame(window);
                        return;
                    }
                    let events = if e.kind == PointerKind::Touch { w.touch(e) } else { vec![e] };
                    if let Some(p) = w.pan.as_mut().filter(|p| p.hold.is_none()) {
                        p.hold = Some(host.set_timer(host.now() + LONG_PRESS));
                    }
                    let mut start = None;
                    for e in &events {
                        w.ui.pointer(e);
                        let consumed = w.ui.pointer_consumed();
                        host.pointer_consumed(window, consumed);
                        if !consumed
                            && e.kind == PointerKind::Mouse
                            && e.phase == PointerPhase::Down
                            && e.buttons == Buttons::MIDDLE
                        {
                            start = Some(e.position);
                        }
                    }
                    if let Some(p) = start
                        && w.can_scroll_at(p)
                    {
                        w.autoscroll =
                            Some(Autoscroll { origin: p, at: p, held: true, dragged: false, last: None });
                    }
                    // A held finger (no events yet), a scroll, and a tap (down + up together)
                    // all need a frame: a tap focuses a field and opens the keyboard.
                    host.request_frame(window);
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
            HostEvent::Drag(window, mut e) => {
                self.activity(host);
                if let Some(key) = self.by_host.get(&window).copied()
                    && let Some((ui, zoom)) = self.wins.get(&key).map(|w| (w.ui.clone(), w.zoom))
                {
                    match &mut e {
                        remedy_ui_core::host::DragEvent::Enter { position: Some(p), .. }
                        | remedy_ui_core::host::DragEvent::Drop { position: Some(p), .. }
                        | remedy_ui_core::host::DragEvent::Over { position: p } => *p = unzoom(*p, zoom),
                        _ => {}
                    }
                    let effect = ui.drag_event(&e);
                    host.set_drop_effect(window, effect);
                    self.requests(host, key);
                    host.request_frame(window);
                }
            }
            HostEvent::Wheel(window, e) => {
                self.activity(host);
                let mut e = e;
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    // The element under the pointer is found in the content's pixels; the
                    // compositor below scrolls in the display's.
                    e.position = unzoom(e.position, w.zoom);
                }
                self.wheel(host, window, &e);
            }
            HostEvent::Pinch(window, mut e) => {
                self.activity(host);
                if let Some(&key) = self.by_host.get(&window)
                    && let Some((ui, zoom)) = self.wins.get(&key).map(|w| (w.ui.clone(), w.zoom))
                {
                    e.position = unzoom(e.position, zoom);
                    ui.pinch(&e);
                    self.requests(host, key);
                    host.request_frame(window);
                }
            }
            HostEvent::Key(window, e) => {
                self.activity(host);
                // Any key ends autoscroll; Escape does nothing else.
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get_mut(k))
                    && w.autoscroll.take().is_some()
                    && e.key == remedy_ui_core::input::Key::Escape
                {
                    host.request_frame(window);
                    return;
                }
                // F12 opens the inspector (devtools) instead of reaching the app.
                #[cfg(feature = "devtools")]
                let inspector = e.pressed
                    && e.key == remedy_ui_core::input::Key::F(12)
                    && e.modifiers == remedy_ui_core::input::Modifiers::NONE;
                #[cfg(not(feature = "devtools"))]
                let inspector = false;
                if inspector {
                    #[cfg(feature = "devtools")]
                    if let Some(&key) = self.by_host.get(&window) {
                        let _ = Window { key, app: self.app.clone() }.open_inspector();
                    }
                } else if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    let taken = w.ui.key(&e);
                    // A back action nothing in the window took leaves the window, as the
                    // platform's back does.
                    if !taken && e.pressed && e.key == remedy_ui_core::input::Key::Back {
                        self.close_requested(host, window);
                    }
                }
            }
            HostEvent::Ime(window, e) => {
                self.activity(host);
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    w.ui.ime(&e);
                }
            }
            HostEvent::Focused(window, focused) => {
                if let Some(&key) = self.by_host.get(&window) {
                    self.app.note_focus(key, focused);
                    self.sync_maximized(host, key);
                }
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get_mut(k)) {
                    w.ui.set_window_focused(focused);
                    if w.spec.privacy_cover {
                        match (focused, w.cover.take()) {
                            (true, Some(c)) => c.remove(),
                            (false, None) => {
                                let ui = w.ui.clone();
                                let cover = ui.el("privacy_cover").layout(|l| {
                                    l.position = remedy_ui_core::layout::Position::Absolute;
                                    l.inset = remedy_ui_core::Insets::ZERO;
                                    l.width = remedy_ui_core::layout::Length::Percent(1.0);
                                    l.height = remedy_ui_core::layout::Length::Percent(1.0);
                                });
                                let bg = w.spec.background.unwrap_or(self.config.theme.colors.bg);
                                let cover =
                                    cover.style(remedy_ui_core::style::StylePatch::default().background(bg));
                                w.cover = Some(ui.trusted_overlay(cover));
                            }
                            (_, keep) => w.cover = keep,
                        }
                    }
                    w.ui.request_frame();
                }
            }
            HostEvent::CloseRequested(window) => self.close_requested(host, window),
            HostEvent::FileDialog(id, chosen) => {
                if let Some(done) = self.dialogs.remove(&id) {
                    done(Ok(chosen));
                }
            }
            HostEvent::Permission(id, granted) => {
                if let Some(done) = self.permissions.remove(&id) {
                    done(granted);
                }
            }
            HostEvent::Share(id, outcome) => {
                if let Some(done) = self.shares.remove(&id) {
                    done(outcome);
                }
            }
            HostEvent::SafeArea(window, area) => {
                if let Some(w) = self.by_host.get(&window).and_then(|k| self.wins.get(k)) {
                    w.ui.set_safe_area(area);
                }
            }
            HostEvent::Suspended | HostEvent::Closing => {
                // The platform takes the windows' surfaces away (Android destroys the native
                // window when this returns): let go of every presenter now, and draw nothing
                // until the windows come back.
                self.suspended = true;
                for w in self.wins.values_mut() {
                    w.present = None;
                    if let Some(c) = w.comp.as_mut() {
                        c.invalidate_tiles();
                    }
                    w.ui.set_occluded(true);
                }
                // A suspended app may be ended without another word.
                let why =
                    if matches!(event, HostEvent::Closing) { Leaving::Closing } else { Leaving::Frozen };
                self.save_now(host, why);
            }
            HostEvent::Destroyed(window) => {
                if let Some(key) = self.by_host.remove(&window) {
                    self.wins.remove(&key);
                    let closed = {
                        let mut s = self.app.0.borrow_mut();
                        s.open.remove(&key);
                        let ui = s.uis.remove(&key);
                        s.visible.remove(&key);
                        s.close_asks.remove(&key);
                        s.unhideable.remove(&key);
                        s.on_geometry.remove(&key);
                        s.on_visibility.remove(&key);
                        let snapshots = s.snapshots.remove(&key);
                        s.focused.remove(&key);
                        s.on_focus.remove(&key);
                        s.renderers.remove(&key);
                        s.titles.remove(&key);
                        s.maximized.remove(&key);
                        s.fullscreen.remove(&key);
                        // An agent's access ends with its window.
                        #[cfg(feature = "agents")]
                        s.agents.remove(&key);
                        (s.on_closed.remove(&key).unwrap_or_default(), ui, snapshots)
                    };
                    let (closed, ui, snapshots) = closed;
                    // Pictures asked for and never drawn: the window is gone.
                    for done in snapshots.unwrap_or_default() {
                        done(None);
                    }
                    for f in closed {
                        f();
                    }
                    // The window's elements go with it, and with them everything they held.
                    if let Some(ui) = ui {
                        ui.close();
                    }
                }
                let (none_left, keep) = {
                    let s = self.app.0.borrow();
                    (s.open.is_empty(), s.keep_running)
                };
                if none_left && !keep {
                    self.leaving(Leaving::Quitting);
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
                } else if let Some(key) = self.frame_timers.remove(&id) {
                    if let Some(w) = self.wins.get_mut(&key) {
                        w.held = None;
                        host.request_frame(w.host);
                    }
                } else if let Some((w, down)) =
                    self.wins.values_mut().find_map(|w| w.long_press(id).map(|d| (w, d)))
                {
                    w.ui.pointer(&down);
                    host.pointer_consumed(w.host, w.ui.pointer_consumed());
                    host.request_frame(w.host);
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
                            if r.action == remedy_ui_core::host::accesskit::Action::SetValue {
                                if let Some(remedy_ui_core::host::accesskit::ActionData::Value(v)) = &r.data {
                                    w.ui.accessibility_set_value(r.target_node.0, v);
                                }
                            } else if let Some(a) = action(r.action) {
                                w.ui.accessibility_action(r.target_node.0, a);
                            }
                        }
                    }
                    host.request_frame(window);
                }
            }
            HostEvent::Notification(id, response) => {
                let cb = self.app.0.borrow().on_notification.clone();
                match cb {
                    Some(cb) => cb(id, response),
                    None => self.app.0.borrow_mut().unheard_notifications.push((id, response)),
                }
            }
            HostEvent::ForegroundStopped(why) => {
                let cb = self.app.0.borrow().on_foreground_stopped.clone();
                if let Some(cb) = cb {
                    cb(why);
                }
            }
            HostEvent::Tray(e) => {
                let cb = self.app.0.borrow().on_tray.clone();
                if let Some(cb) = cb {
                    cb(e);
                }
            }
            HostEvent::SecondInstance(args) => self.launched(args, None),
            HostEvent::MemoryPressure(level) => {
                for w in self.wins.values_mut() {
                    if let Some(c) = w.comp.as_mut() {
                        c.trim_memory();
                    }
                }
                #[cfg(feature = "gpu")]
                if level != remedy_ui_core::host::MemoryPressure::Normal
                    && let Some(Some(gpu)) = self.gpu.as_mut()
                {
                    gpu.trim_memory();
                }
                remedy_ui_engine::view::device::memory_pressure(level);
            }
            _ => {}
        }
        // Input the app sent its windows: handled as the platform's, in order.
        let injected = std::mem::take(&mut self.app.0.borrow_mut().injected);
        for (key, make) in injected {
            if let Some(window) = self.wins.get(&key).map(|w| w.host) {
                self.event(host, make(window));
            }
        }
        self.drain(host);
        self.app.0.borrow_mut().in_event = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoscroll_rests_near_its_spot_and_speeds_up_with_distance() {
        let o = Point::new(100.0, 100.0);
        let at = |x: f32, y: f32| Autoscroll {
            origin: o,
            at: Point::new(x, y),
            held: false,
            dragged: false,
            last: None,
        };
        assert_eq!(at(108.0, 92.0).delta(0.016), Vec2::ZERO, "inside the dead zone");
        let near = at(100.0, 132.0).delta(0.5).y;
        let far = at(100.0, 212.0).delta(0.5).y;
        assert!(near > 0.0 && far > near * 4.0, "{near} then {far}");
        assert!(at(100.0, 40.0).delta(0.5).y < 0.0, "above the spot scrolls up");
        assert_eq!(at(100.0, 132.0).delta(0.0), Vec2::ZERO, "nothing before time passes");
    }

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
