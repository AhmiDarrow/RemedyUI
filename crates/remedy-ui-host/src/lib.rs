//! RemedyUI's host model.
//!
//! The engine's own idea of what a platform provides: windows, a frame clock, input, text
//! input, the clipboard, timers, a wake handle for other threads, and system settings. Every
//! platform is a bridge that implements [`Host`] and feeds [`HostEvent`]s to an [`App`]:
//! Windows, macOS, Wayland and X11 (sharing a winit core), Android, RemedyOS, the web, iOS, and
//! the headless bridge used by tests. Nothing platform-shaped appears here, so adding a
//! platform means writing a bridge and nothing else. A capability a platform cannot provide
//! returns [`Unsupported`] with the reason; it is never faked. See
//! `docs/design/10-platform-bridges.md`.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::{Color, Point, Rect, Size};
use remedy_ui_input::{KeyEvent, PinchEvent, Platform, PointerEvent, WheelEvent};

pub use accesskit;
pub use raw_window_handle;

/// The wall clock and the local time zone at one instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LocalTime {
    /// Seconds since 1970-01-01T00:00:00Z.
    pub unix_seconds: i64,
    /// Seconds east of UTC at that instant (daylight saving included). 0 where the platform
    /// cannot say.
    pub utc_offset: i32,
}

impl LocalTime {
    /// Seconds since 1970-01-01T00:00:00 on the local wall clock.
    pub fn local_seconds(self) -> i64 {
        self.unix_seconds + i64::from(self.utc_offset)
    }

    /// Days since 1970-01-01 on the local calendar.
    pub fn local_days(self) -> i64 {
        self.local_seconds().div_euclid(86_400)
    }

    /// Seconds until the next local midnight (at least 1).
    pub fn until_midnight(self) -> u64 {
        (86_400 - self.local_seconds().rem_euclid(86_400)).max(1) as u64
    }
}

/// A screen reader or other assistive technology talking to a window.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum AccessibilityEvent {
    /// It connected and wants the whole tree; send it with [`Host::update_accessibility`].
    Requested,
    /// It asks an element to do something (click, focus, increment, …).
    Action(accesskit::ActionRequest),
    /// It went away; stop sending updates.
    Deactivated,
}

/// An app's icon in the system tray (notification area, menu bar extras, status notifier).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TrayIcon {
    /// What hovering shows.
    pub tooltip: String,
    /// The icon: premultiplied RGBA8, square (the platform scales it).
    pub icon: TrayImage,
    /// The menu, top to bottom.
    pub menu: Vec<TrayItem>,
}

impl TrayIcon {
    /// `icon` with `tooltip` and `menu`.
    pub fn new(tooltip: impl Into<String>, icon: TrayImage, menu: Vec<TrayItem>) -> Self {
        TrayIcon { tooltip: tooltip.into(), icon, menu }
    }
}

/// Tray icon pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayImage {
    /// Width and height in pixels.
    pub size: u32,
    /// Premultiplied RGBA8, row by row.
    pub rgba: Vec<u8>,
}

/// One tray menu entry.
#[derive(Clone, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge must show completely, so a new kind is a major change.
pub enum TrayItem {
    /// A command; its id comes back in [`TrayEvent::Menu`].
    Item {
        /// The app's id for it.
        id: u32,
        /// What it says.
        label: String,
        /// Whether it can be chosen.
        enabled: bool,
    },
    /// A separator line.
    Separator,
}

/// What the user did with the tray icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrayEvent {
    /// The icon was activated (clicked, or the platform's primary action).
    Activate,
    /// A menu entry was chosen.
    Menu(u32),
}

/// A native open or save dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileDialog {
    /// Title.
    pub title: String,
    /// Save rather than open.
    pub save: bool,
    /// Choose folders rather than files.
    pub folders: bool,
    /// Allow choosing several.
    pub multiple: bool,
    /// Start here.
    pub directory: Option<std::path::PathBuf>,
    /// Suggested file name (save).
    pub file_name: Option<String>,
    /// Filters: a name and the extensions it allows (without dots).
    pub filters: Vec<(String, Vec<String>)>,
}

/// A file dialog's identity while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DialogId(pub u64);

/// Where a file dialog stands once [`Host::file_dialog`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum DialogStarted {
    /// It already closed (platforms whose dialogs run inside the call): the chosen paths,
    /// empty when cancelled.
    Finished(Vec<std::path::PathBuf>),
    /// It is open; [`HostEvent::FileDialog`] reports the choice (platforms whose pickers are
    /// separate screens, such as Android's).
    Pending(DialogId),
}

/// Something the app may need the person's leave for, on platforms that ask at run time
/// (Android). Desktop platforms grant at install or in their privacy settings instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Permission {
    /// Hearing the microphone (`RECORD_AUDIO` on Android, which the app's manifest must also
    /// declare).
    Microphone,
}

/// A permission request's identity while the platform asks the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PermissionId(pub u64);

/// Where a permission request stands once [`Host::request_permission`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum PermissionStarted {
    /// Already decided: true when the app may go ahead. Platforms with no run-time
    /// permissions answer true; their own privacy settings may still refuse the device,
    /// which then reports why when it is opened.
    Finished(bool),
    /// The platform is asking the person; [`HostEvent::Permission`] reports the answer.
    Pending(PermissionId),
}

/// The edge or corner a resize drags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Exhaustive on purpose: a window has four edges and four corners.
pub enum ResizeEdge {
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
    /// Right edge.
    Right,
    /// Top-left corner.
    TopLeft,
    /// Top-right corner.
    TopRight,
    /// Bottom-left corner.
    BottomLeft,
    /// Bottom-right corner.
    BottomRight,
}

/// A window's identity for the life of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

/// A timer's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub u64);

/// A capability this platform does not have, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unsupported {
    /// What was asked for.
    pub what: &'static str,
    /// Why this platform cannot do it.
    pub reason: String,
}

impl Unsupported {
    /// `what` is not available because of `reason`.
    pub fn new(what: &'static str, reason: impl Into<String>) -> Self {
        Unsupported { what, reason: reason.into() }
    }
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not available here: {}", self.what, self.reason)
    }
}

impl std::error::Error for Unsupported {}

/// How to open a window. Sizes are logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WindowOptions {
    /// Title (also what assistive technology and the task switcher show).
    pub title: String,
    /// Content size.
    pub size: Size,
    /// Smallest content size the user may resize to.
    pub min_size: Option<Size>,
    /// Position on the desktop; `None` lets the platform place it.
    pub position: Option<Point>,
    /// The user may resize it.
    pub resizable: bool,
    /// The platform draws a frame and title bar; false for frameless windows that draw their
    /// own chrome.
    pub decorated: bool,
    /// Stays above other windows.
    pub always_on_top: bool,
    /// Has a transparent background (for rounded or shaped windows).
    pub transparent: bool,
    /// Shown at creation.
    pub visible: bool,
    /// Kept out of screenshots, screen sharing and recordings where the platform allows.
    pub capture_protected: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: String::new(),
            size: Size::new(800.0, 600.0),
            min_size: None,
            position: None,
            resizable: true,
            decorated: true,
            always_on_top: false,
            transparent: false,
            visible: true,
            capture_protected: false,
        }
    }
}

/// System settings the engine follows, delivered again whenever one changes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SystemSettings {
    /// Shortcut conventions.
    pub platform: Platform,
    /// The system prefers dark; `None` when the platform does not say.
    pub dark: Option<bool>,
    /// The user's accent color, where the platform has one.
    pub accent: Option<Color>,
    /// High contrast is on.
    pub high_contrast: bool,
    /// Text scale chosen by the user (1 is the default size).
    pub text_scale: f32,
    /// Reduce motion.
    pub reduced_motion: bool,
    /// Reduce transparency.
    pub reduced_transparency: bool,
    /// BCP 47 locale, such as `en-US`.
    pub locale: String,
    /// Longest gap between the clicks of a double click.
    pub double_click: Duration,
    /// Caret blink interval; `None` when the user turned blinking off.
    pub caret_blink: Option<Duration>,
}

impl Default for SystemSettings {
    fn default() -> Self {
        SystemSettings {
            platform: Platform::Standard,
            dark: None,
            accent: None,
            high_contrast: false,
            text_scale: 1.0,
            reduced_motion: false,
            reduced_transparency: false,
            locale: String::from("en-US"),
            double_click: Duration::from_millis(500),
            caret_blink: Some(Duration::from_millis(530)),
        }
    }
}

/// The edges of a window that system interface covers or may cover: status and navigation
/// bars, display cutouts and the on-screen keyboard, in logical pixels. Content belongs inside
/// them; the window's background may run under them. Zero on every side where nothing covers
/// the window (desktop windows).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SafeArea {
    /// Covered at the top.
    pub top: f32,
    /// Covered on the right.
    pub right: f32,
    /// Covered at the bottom.
    pub bottom: f32,
    /// Covered on the left.
    pub left: f32,
}

impl SafeArea {
    /// Device pixels to logical pixels at `scale` device pixels per logical pixel.
    pub fn from_device(top: f32, right: f32, bottom: f32, left: f32, scale: f32) -> SafeArea {
        let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let px = |v: f32| if v.is_finite() { v.max(0.0) / s } else { 0.0 };
        SafeArea { top: px(top), right: px(right), bottom: px(bottom), left: px(left) }
    }
}

/// How hard the system is asking processes to give memory back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum MemoryPressure {
    /// Plenty.
    Normal,
    /// Drop caches that are cheap to rebuild.
    Moderate,
    /// Drop everything not on screen.
    Critical,
}

/// What a focused text field tells the input method.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TextInputState {
    /// Text around the caret, for reconversion and prediction (empty for private fields).
    pub surrounding: String,
    /// Selection within `surrounding`, in bytes.
    pub selection: Range<usize>,
    /// Caret rectangle in window logical pixels, where the candidate window goes.
    pub caret: Rect,
    /// A password or other secret: no learning, no prediction, no cloud input.
    pub private: bool,
    /// Several lines (Enter inserts a line break instead of submitting).
    pub multiline: bool,
}

/// What an input method did.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum ImeEvent {
    /// Composition started.
    Enabled,
    /// The text being composed, with the input method's caret or selection inside it.
    Preedit {
        /// The composition.
        text: String,
        /// Caret or selection within `text`, in bytes.
        cursor: Option<Range<usize>>,
    },
    /// Final text to insert.
    Commit(String),
    /// Delete text around the caret (bytes before and after), as some input methods ask.
    DeleteSurrounding {
        /// Bytes before the caret.
        before: usize,
        /// Bytes after.
        after: usize,
    },
    /// Composition ended.
    Disabled,
}

/// Everything a host tells the app.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum HostEvent {
    /// The app may create windows (at start, and again after a mobile resume).
    Resumed,
    /// The app should release surfaces (mobile background).
    Suspended,
    /// A window's surface size or scale changed (device pixels and device pixels per logical
    /// pixel). Also sent once when a window is created.
    Resized {
        /// The window.
        window: WindowId,
        /// Width in device pixels.
        width: u32,
        /// Height in device pixels.
        height: u32,
        /// Device pixels per logical pixel.
        scale: f32,
    },
    /// The user asked to close a window. The app decides.
    CloseRequested(WindowId),
    /// A window moved; its top-left is now at this desktop position (logical pixels).
    Moved(WindowId, Point),
    /// A window is gone.
    Destroyed(WindowId),
    /// Keyboard focus came or went.
    Focused(WindowId, bool),
    /// The part of a window that system interface leaves clear changed (also sent once when
    /// it is first known).
    SafeArea(WindowId, SafeArea),
    /// A window became fully covered or minimized (true), or visible again (false).
    Occluded(WindowId, bool),
    /// Pointer input.
    Pointer(WindowId, PointerEvent),
    /// Wheel or touchpad scroll.
    Wheel(WindowId, WheelEvent),
    /// A pinch to zoom the platform recognized (touchpad magnify gestures).
    Pinch(WindowId, PinchEvent),
    /// Key input.
    Key(WindowId, KeyEvent),
    /// Input method events for the focused text field.
    Ime(WindowId, ImeEvent),
    /// Time to draw a requested frame, for presentation at `at`.
    Frame {
        /// The window.
        window: WindowId,
        /// Predicted presentation time on the host clock.
        at: Duration,
    },
    /// System settings changed.
    Settings(SystemSettings),
    /// The system wants memory back.
    MemoryPressure(MemoryPressure),
    /// A timer is due.
    Timer(TimerId),
    /// Another thread called the [`Waker`].
    Wake,
    /// The tray icon was used.
    Tray(TrayEvent),
    /// Assistive technology on a window.
    Accessibility(WindowId, AccessibilityEvent),
    /// Something is dragged over a window, or dropped on it.
    Drag(WindowId, DragEvent),
    /// A drag out of a window that [`Host::start_drag`] left pending ended.
    DragEnded(WindowId, DropEffect),
    /// A file dialog [`Host::file_dialog`] left pending closed: the chosen paths, empty when
    /// cancelled.
    FileDialog(DialogId, Vec<std::path::PathBuf>),
    /// A permission request [`Host::request_permission`] left pending was answered: true when
    /// granted.
    Permission(PermissionId, bool),
    /// The app was launched again while running (single instance); these are the new
    /// launch's arguments.
    SecondInstance(Vec<String>),
}

/// What a drag carries: files, text and links, in the forms other apps understand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DragData {
    /// Files and folders.
    pub files: Vec<std::path::PathBuf>,
    /// Plain text.
    pub text: Option<String>,
    /// Links (`https:` and other URIs).
    pub uris: Vec<String>,
}

impl DragData {
    /// Nothing.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.text.is_none() && self.uris.is_empty()
    }

    /// Some files.
    pub fn files(files: Vec<std::path::PathBuf>) -> Self {
        DragData { files, ..DragData::default() }
    }

    /// Some text.
    pub fn text(text: impl Into<String>) -> Self {
        DragData { text: Some(text.into()), ..DragData::default() }
    }
}

/// What a drop does to what was dragged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// Exhaustive on purpose: the four outcomes every platform's drag and drop knows.
pub enum DropEffect {
    /// Refused: nothing happens.
    #[default]
    None,
    /// Copied; the source keeps it.
    Copy,
    /// Moved; the source lets it go.
    Move,
    /// Linked to where it is.
    Link,
}

/// Where a drag out of a window stands once [`Host::start_drag`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum DragStarted {
    /// It already ended (platforms whose drags run to the end inside the call): what the
    /// drop did.
    Finished(DropEffect),
    /// It is under way; [`HostEvent::DragEnded`] reports how it ended.
    Pending,
}

/// A drag from another app (or from this one, outside its own windows) over a window.
#[derive(Clone, Debug, PartialEq)]
// Exhaustive on purpose: part of the host contract every bridge and runner must handle completely, so a new kind is a major change.
pub enum DragEvent {
    /// It came in. Platforms that do not say where report `None` until it moves.
    Enter {
        /// Where, in logical pixels.
        position: Option<Point>,
        /// What it carries.
        data: DragData,
    },
    /// It moved.
    Over {
        /// Where, in logical pixels.
        position: Point,
    },
    /// It left, or was cancelled.
    Leave,
    /// It was dropped.
    Drop {
        /// Where, in logical pixels (`None` when the platform does not say).
        position: Option<Point>,
        /// What it carries.
        data: DragData,
    },
}

/// True for links an app may hand to the system to open: web pages and mail. Anything else
/// (`file:`, custom protocols, programs) could start software the user did not mean to run.
pub fn is_openable_url(url: &str) -> bool {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    let scheme_ok = ["http://", "https://", "mailto:"].iter().any(|s| lower.starts_with(s));
    // No whitespace or control characters: one link, nothing smuggled after it.
    scheme_ok && u.len() == url.len() && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Premultiplied RGBA8 pixels to present.
#[derive(Clone, Copy, Debug)]
pub struct Pixels<'a> {
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// Row-major premultiplied RGBA8.
    pub data: &'a [u8],
}

/// Wakes the host's event loop from any thread; the app then receives [`HostEvent::Wake`].
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// A window's native handles, for creating GPU surfaces.
pub trait WindowHandles:
    raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync
{
}

impl<T: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync> WindowHandles
    for T
{
}

/// What the app may ask of its host. Every bridge implements it.
pub trait Host {
    /// Shortcut conventions of this platform.
    fn platform(&self) -> Platform;
    /// The host clock (monotonic, the clock frame times use).
    fn now(&self) -> Duration;
    /// The wall clock and the local UTC offset now (for dates and times people read).
    fn local_time(&self) -> LocalTime;
    /// Current system settings.
    fn settings(&self) -> SystemSettings;
    /// The machine's physical memory in bytes, when the platform says (packs size caches
    /// from it).
    fn total_memory(&self) -> Option<u64> {
        None
    }
    /// Opens a window. A [`HostEvent::Resized`] follows with its real size and scale.
    fn create_window(&mut self, options: &WindowOptions) -> Result<WindowId, Unsupported>;
    /// Closes a window. [`HostEvent::Destroyed`] follows.
    fn close_window(&mut self, window: WindowId);
    /// A window's size in device pixels and its scale.
    fn window_size(&self, window: WindowId) -> Option<(u32, u32, f32)>;
    /// The edges of a window that system interface covers (status and navigation bars,
    /// cutouts, the on-screen keyboard). [`HostEvent::SafeArea`] reports changes.
    fn safe_area(&self, window: WindowId) -> SafeArea {
        let _ = window;
        SafeArea::default()
    }
    /// How often the display the window is on refreshes (16.7 ms at 60 Hz), where the
    /// platform says. Frames that take longer miss a refresh.
    fn refresh_interval(&self, window: WindowId) -> Option<std::time::Duration> {
        let _ = window;
        None
    }
    /// Changes the title.
    fn set_title(&mut self, window: WindowId, title: &str);
    /// Resizes the content to `size` logical pixels.
    fn set_size(&mut self, window: WindowId, size: Size) -> Result<(), Unsupported>;
    /// Shows or hides.
    fn set_visible(&mut self, window: WindowId, visible: bool);
    /// The window's top-left on the desktop in logical pixels, where the platform reports it
    /// (Wayland does not).
    fn window_position(&self, window: WindowId) -> Option<Point>;
    /// Moves the window's top-left to a desktop position in logical pixels.
    fn set_position(&mut self, window: WindowId, position: Point) -> Result<(), Unsupported>;
    /// Brings the window to the front and gives it keyboard focus.
    fn focus_window(&mut self, window: WindowId);
    /// Minimizes or restores.
    fn set_minimized(&mut self, window: WindowId, minimized: bool);
    /// Maximizes or restores.
    fn set_maximized(&mut self, window: WindowId, maximized: bool);
    /// True while the window is maximized (by the app or the user).
    fn is_maximized(&self, window: WindowId) -> bool;
    /// Makes the window fill its display with no frame (a player's full screen, a slide
    /// show), or brings it back.
    fn set_fullscreen(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        let _ = (window, on);
        Err(Unsupported::new("full-screen windows", "this host has no full-screen mode"))
    }
    /// True while the window fills its display (by the app or the user).
    fn is_fullscreen(&self, window: WindowId) -> bool {
        let _ = window;
        false
    }
    /// Starts a user move of the window, as dragging a title bar does (call on a pointer
    /// press in custom chrome).
    fn begin_move(&mut self, window: WindowId) -> Result<(), Unsupported>;
    /// Starts a user resize from `edge`, as dragging a window border does.
    fn begin_resize(&mut self, window: WindowId, edge: ResizeEdge) -> Result<(), Unsupported>;
    /// Keeps above other windows, or not.
    fn set_always_on_top(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported>;
    /// Keeps the window out of screenshots and screen sharing, or not.
    fn set_capture_protected(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported>;
    /// Asks for a [`HostEvent::Frame`] at the next display refresh. Several requests before
    /// it arrives merge into one.
    fn request_frame(&mut self, window: WindowId);
    /// Shows CPU-rendered pixels. Only `damage` (device pixels) changed since the last
    /// present; hosts that can, copy and flush only that.
    fn present_pixels(
        &mut self,
        window: WindowId,
        pixels: Pixels<'_>,
        damage: &[Rect],
    ) -> Result<(), Unsupported>;
    /// The window's native handles, for a GPU surface.
    fn window_handles(&self, window: WindowId) -> Result<Arc<dyn WindowHandles>, Unsupported>;
    /// True when a window's surface keeps the renderer it was first presented with: once
    /// CPU pixels reached it, a GPU swapchain cannot take it over until the platform gives
    /// the window a new surface (Android). The engine then picks the renderer before a
    /// window's first present.
    fn renderer_fixed_per_surface(&self) -> bool {
        false
    }
    /// Which renderer draws a window now: the GPU (on a surface from
    /// [`Host::window_handles`]) or the CPU (through [`Host::present_pixels`]). Called before
    /// a window's first frame and whenever its renderer changes; a host that shows the two
    /// differently (a browser's GPU and 2D canvases) switches here.
    fn renderer_changed(&mut self, window: WindowId, gpu: bool) {
        let _ = (window, gpu);
    }
    /// Whether the app consumed the pointer event it was just given (an element's handler took
    /// it, such as a context menu opening on a secondary press). Called after every pointer
    /// event; a host whose platform acts on presses itself (a browser's own context menu)
    /// cancels that action only when the app took the press.
    fn pointer_consumed(&mut self, window: WindowId, consumed: bool) {
        let _ = (window, consumed);
    }
    /// Starts, updates (`Some`) or ends (`None`) text input on a window.
    fn set_text_input(&mut self, window: WindowId, state: Option<TextInputState>);
    /// The clipboard's text.
    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported>;
    /// Puts text on the clipboard; `sensitive` keeps it out of clipboard history and cloud
    /// sync where the platform allows.
    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported>;
    /// Empties the clipboard (a copied secret's time is up).
    fn clear_clipboard(&mut self) -> Result<(), Unsupported>;
    /// The primary selection's text: what was last selected anywhere, pasted with the middle
    /// button (X11 and Wayland). Platforms without one say `Unsupported`.
    fn primary_selection_text(&mut self) -> Result<Option<String>, Unsupported> {
        Err(Unsupported::new("primary_selection_text", "this platform has no primary selection"))
    }
    /// Makes `text` the primary selection (what was just selected), where the platform has
    /// one.
    fn set_primary_selection(&mut self, text: &str) -> Result<(), Unsupported> {
        let _ = text;
        Err(Unsupported::new("set_primary_selection", "this platform has no primary selection"))
    }
    /// The clipboard's HTML (formatted text another app copied), if it holds any.
    fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
        Err(Unsupported::new("clipboard_html", "this host reads plain text only"))
    }
    /// Puts formatted text on the clipboard: `html` for apps that paste formatting, `text` for
    /// the rest. A host without HTML support puts the text alone.
    fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
        let _ = html;
        self.set_clipboard_text(text, false)
    }
    /// Sends a window's accessibility tree, or what changed in it, to assistive technology.
    fn update_accessibility(&mut self, window: WindowId, update: accesskit::TreeUpdate);
    /// Shows, updates (`Some`) or removes (`None`) the app's tray icon.
    fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported>;
    /// Runs a native open or save dialog, modal to `window`. Desktop dialogs finish inside the
    /// call with the chosen paths (empty when cancelled); others return at once and report
    /// the choice with [`HostEvent::FileDialog`].
    fn file_dialog(
        &mut self,
        window: Option<WindowId>,
        dialog: &FileDialog,
    ) -> Result<DialogStarted, Unsupported>;
    /// Asks for the person's leave to use `permission`. Platforms without run-time
    /// permissions answer [`PermissionStarted::Finished`] with true at once; Android shows its
    /// system prompt (when the permission is not already granted) and reports the answer
    /// with [`HostEvent::Permission`].
    fn request_permission(&mut self, permission: Permission) -> Result<PermissionStarted, Unsupported> {
        let _ = permission;
        Ok(PermissionStarted::Finished(true))
    }
    /// Starts dragging `data` out of `window` to other apps (call while the pointer button
    /// that began the drag is held). Some platforms run the drag to its end inside the call;
    /// others return at once and report the end with [`HostEvent::DragEnded`].
    fn start_drag(
        &mut self,
        window: WindowId,
        data: &DragData,
        allowed: &[DropEffect],
    ) -> Result<DragStarted, Unsupported>;
    /// What a drop on `window` would do now, as the app decided for the last
    /// [`DragEvent`]: platforms show it with the cursor and tell the source.
    fn set_drop_effect(&mut self, window: WindowId, effect: DropEffect);
    /// Opens `url` (an `http:`, `https:` or `mailto:` link) in the user's browser or mail app.
    fn open_url(&mut self, url: &str) -> Result<(), Unsupported>;
    /// A one-shot timer at host time `at`.
    fn set_timer(&mut self, at: Duration) -> TimerId;
    /// Cancels a timer that has not fired.
    fn cancel_timer(&mut self, timer: TimerId);
    /// A handle other threads use to wake the loop.
    fn waker(&self) -> Waker;
    /// Ends the event loop after the current event.
    fn exit(&mut self);
}

/// An application driven by a host.
pub trait App {
    /// Handles one event. `host` is how the app acts on the platform.
    fn event(&mut self, host: &mut dyn Host, event: HostEvent);
}

impl<T: App + ?Sized> App for &mut T {
    fn event(&mut self, host: &mut dyn Host, event: HostEvent) {
        (**self).event(host, event);
    }
}

#[cfg(test)]
mod safe_area_tests {
    use super::SafeArea;

    #[test]
    fn device_pixels_become_logical_and_never_negative() {
        let a = SafeArea::from_device(96.0, 0.0, 144.0, -3.0, 2.0);
        assert_eq!(a, SafeArea { top: 48.0, right: 0.0, bottom: 72.0, left: 0.0 });
        let bad = SafeArea::from_device(f32::NAN, 10.0, 0.0, 0.0, 0.0);
        assert_eq!(
            bad,
            SafeArea { top: 0.0, right: 10.0, bottom: 0.0, left: 0.0 },
            "a bad scale counts as 1"
        );
    }
}

#[cfg(test)]
mod local_time_tests {
    use super::LocalTime;

    #[test]
    fn local_days_and_midnight_follow_the_offset() {
        // 2026-11-17T23:30:00Z.
        let t = LocalTime { unix_seconds: 1_794_958_200, utc_offset: 0 };
        assert_eq!(t.until_midnight(), 1800);
        let east = LocalTime { utc_offset: 3600, ..t };
        assert_eq!(east.local_days(), t.local_days() + 1, "already tomorrow in UTC+1");
        assert_eq!(east.until_midnight(), 86_400 - 1800);
        let west = LocalTime { utc_offset: -5 * 3600, ..t };
        assert_eq!(west.local_days(), t.local_days());
        let before_1970 = LocalTime { unix_seconds: -1, utc_offset: 0 };
        assert_eq!(before_1970.local_days(), -1);
    }
}
