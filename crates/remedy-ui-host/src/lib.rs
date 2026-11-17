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
use remedy_ui_input::{KeyEvent, Platform, PointerEvent, WheelEvent};

pub use raw_window_handle;

/// A window's identity for the life of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

/// A timer's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub u64);

/// A capability this platform does not have, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// How hard the system is asking processes to give memory back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryPressure {
    /// Plenty.
    Normal,
    /// Drop caches that are cheap to rebuild.
    Moderate,
    /// Drop everything not on screen.
    Critical,
}

/// What a focused text field tells the input method.
#[derive(Clone, Debug, PartialEq)]
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
    /// A window is gone.
    Destroyed(WindowId),
    /// Keyboard focus came or went.
    Focused(WindowId, bool),
    /// A window became fully covered or minimized (true), or visible again (false).
    Occluded(WindowId, bool),
    /// Pointer input.
    Pointer(WindowId, PointerEvent),
    /// Wheel or touchpad scroll.
    Wheel(WindowId, WheelEvent),
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
    /// Current system settings.
    fn settings(&self) -> SystemSettings;
    /// Opens a window. A [`HostEvent::Resized`] follows with its real size and scale.
    fn create_window(&mut self, options: &WindowOptions) -> Result<WindowId, Unsupported>;
    /// Closes a window. [`HostEvent::Destroyed`] follows.
    fn close_window(&mut self, window: WindowId);
    /// A window's size in device pixels and its scale.
    fn window_size(&self, window: WindowId) -> Option<(u32, u32, f32)>;
    /// Changes the title.
    fn set_title(&mut self, window: WindowId, title: &str);
    /// Resizes the content to `size` logical pixels.
    fn set_size(&mut self, window: WindowId, size: Size) -> Result<(), Unsupported>;
    /// Shows or hides.
    fn set_visible(&mut self, window: WindowId, visible: bool);
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
    /// Starts, updates (`Some`) or ends (`None`) text input on a window.
    fn set_text_input(&mut self, window: WindowId, state: Option<TextInputState>);
    /// The clipboard's text.
    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported>;
    /// Puts text on the clipboard; `sensitive` keeps it out of clipboard history and cloud
    /// sync where the platform allows.
    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported>;
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
