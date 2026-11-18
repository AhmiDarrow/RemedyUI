//! RemedyUI's shared desktop bridge core over winit.
//!
//! One implementation of the [`Host`] contract for every desktop platform winit serves:
//! windows, pointer, touch, wheel and key input, input-method sessions, frames on the
//! display's refresh, timers, wakes from other threads, CPU presents through softbuffer with
//! damage, and native handles for GPU surfaces. What winit does not cover comes from
//! [`PlatformHooks`], which each platform bridge implements (`bridge-windows`, `bridge-macos`,
//! `bridge-wayland`, `bridge-x11`): the clipboard, capture protection and system settings.

pub mod backend;
mod translate;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use remedy_ui_core::{Point, Rect, Size, Vec2};
use remedy_ui_host::{
    AccessibilityEvent, App, DragData, DragEvent, DragStarted, DropEffect, FileDialog, Host, HostEvent,
    ImeEvent, Pixels, ResizeEdge, SystemSettings, TextInputState, TimerId, TrayIcon, Unsupported, Waker,
    WindowHandles, WindowId, WindowOptions,
};
use remedy_ui_input::{
    Buttons, KeyEvent, Modifiers, Platform, PointerEvent, PointerKind, PointerPhase, WheelEvent,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowLevel};

pub use winit;

/// What a platform bridge adds where winit stops.
pub trait PlatformHooks: 'static {
    /// Shortcut conventions.
    fn platform(&self) -> Platform;
    /// System settings, read from the OS.
    fn settings(&mut self) -> SystemSettings;
    /// Seconds east of UTC at `unix_seconds` in the user's time zone, or `None` when the
    /// platform cannot say (the host then uses UTC).
    fn utc_offset(&self, unix_seconds: i64) -> Option<i32>;
    /// The clipboard's text.
    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported>;
    /// Writes text to the clipboard; `sensitive` keeps it out of history and sync.
    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported>;
    /// Empties the clipboard.
    fn clear_clipboard(&mut self) -> Result<(), Unsupported>;
    /// The machine's physical memory in bytes.
    fn total_memory(&self) -> Option<u64> {
        None
    }
    /// The clipboard's HTML, if it holds formatted text.
    fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
        Err(Unsupported::new("clipboard_html", "this platform reads plain text only"))
    }
    /// Writes formatted text: `text` for every app, `html` for apps that paste formatting.
    fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
        let _ = html;
        self.set_clipboard_text(text, false)
    }
    /// Keeps a window out of screenshots and screen sharing.
    fn set_capture_protected(&mut self, window: &Window, on: bool) -> Result<(), Unsupported>;
    /// Called once per new window, before it is shown, for platform setup.
    fn window_created(&mut self, window: &Window) {
        let _ = window;
    }
    /// Called before a window closes, to release platform state tied to it.
    fn window_closed(&mut self, window: &Window) {
        let _ = window;
    }
    /// Takes over a text-input session with the platform's own text services (TSF on
    /// Windows). Returns false to let winit's input-method support handle it.
    fn set_text_input(&mut self, window: &Window, state: Option<&TextInputState>) -> bool {
        let _ = (window, state);
        false
    }
    /// Shows, updates or removes the tray icon.
    fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported>;
    /// Runs a native file dialog, modal to `window`.
    fn file_dialog(
        &mut self,
        window: Option<&Window>,
        dialog: &FileDialog,
    ) -> Result<Vec<std::path::PathBuf>, Unsupported>;
    /// Opens a web or mail link with the system (checked by the caller).
    fn open_url(&mut self, url: &str) -> Result<(), Unsupported>;
    /// Receives a handle that wakes the event loop, for platform services that report from
    /// other threads (a D-Bus tray).
    fn set_waker(&mut self, waker: Waker) {
        let _ = waker;
    }
    /// Adjusts a new window's attributes before it is created (a platform that handles
    /// drops itself turns winit's file drops off).
    fn window_attributes(&mut self, attrs: WindowAttributes) -> WindowAttributes {
        attrs
    }
    /// True when the platform reports drags over windows itself, through
    /// [`PlatformHooks::take_drag_events`], with positions and every kind of data;
    /// otherwise winit's file drops are used (files only, without positions).
    fn handles_drops(&self) -> bool {
        false
    }
    /// Starts a drag out of `window` to other apps.
    fn start_drag(
        &mut self,
        window: &Window,
        data: &DragData,
        allowed: &[DropEffect],
    ) -> Result<DragStarted, Unsupported> {
        let _ = (window, data, allowed);
        Err(Unsupported::new("dragging out to other apps", "this platform's bridge does not start drags"))
    }
    /// What a drop on `window` would do now (cursor feedback and the source's answer).
    fn set_drop_effect(&mut self, window: &Window, effect: DropEffect) {
        let _ = (window, effect);
    }
    /// App-wide events the platform produced since the last call (tray use).
    fn take_events(&mut self) -> Vec<HostEvent> {
        Vec::new()
    }
    /// Drags over windows the platform reported since the last call.
    fn take_drag_events(&mut self) -> Vec<(winit::window::WindowId, DragEvent)> {
        Vec::new()
    }
    /// Drags out that [`PlatformHooks::start_drag`] left pending and that have ended since
    /// the last call.
    fn take_drag_ends(&mut self) -> Vec<(winit::window::WindowId, DropEffect)> {
        Vec::new()
    }
    /// Input-method events the platform's text services produced since the last call.
    fn take_ime_events(&mut self) -> Vec<(winit::window::WindowId, ImeEvent)> {
        Vec::new()
    }
}

/// Why the event loop could not run.
#[derive(Debug)]
pub struct RunError(pub String);

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the event loop failed: {}", self.0)
    }
}

impl std::error::Error for RunError {}

/// Events from other threads.
#[derive(Debug)]
pub enum UserEvent {
    /// A [`Waker`] was called.
    Wake,
    /// The accessibility adapter of a window reported.
    Accessibility(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        UserEvent::Accessibility(e)
    }
}

type SoftSurface = softbuffer::Surface<Arc<Window>, Arc<Window>>;

struct Win {
    window: Arc<Window>,
    a11y: Option<accesskit_winit::Adapter>,
    surface: Option<SoftSurface>,
    cursor: Point,
    buttons: Buttons,
    composing: bool,
    /// Files winit reported hovering or dropped since the last flush (winit sends one event
    /// per file).
    hovered: Vec<std::path::PathBuf>,
    dropped: Vec<std::path::PathBuf>,
    /// A drag from winit's file drops is over the window.
    dragging: bool,
}

struct State {
    start: Instant,
    windows: HashMap<winit::window::WindowId, (WindowId, Win)>,
    ids: HashMap<WindowId, winit::window::WindowId>,
    next_window: u64,
    next_timer: u64,
    timers: BTreeMap<TimerId, Duration>,
    pending: VecDeque<HostEvent>,
    proxy: Arc<Mutex<EventLoopProxy<UserEvent>>>,
    modifiers: Modifiers,
    softbuffer: Option<softbuffer::Context<Arc<Window>>>,
    settings: SystemSettings,
    exit: bool,
}

/// The host as an app sees it during one callback.
struct Ctx<'a, H: PlatformHooks> {
    el: &'a ActiveEventLoop,
    state: &'a mut State,
    hooks: &'a mut H,
}

fn logical(p: PhysicalPosition<f64>, scale: f64) -> Point {
    let l: LogicalPosition<f64> = p.to_logical(scale);
    Point::new(l.x as f32, l.y as f32)
}

impl<H: PlatformHooks> Ctx<'_, H> {
    fn win(&self, id: WindowId) -> Option<&Win> {
        let wid = self.state.ids.get(&id)?;
        self.state.windows.get(wid).map(|(_, w)| w)
    }
}

impl<H: PlatformHooks> Host for Ctx<'_, H> {
    fn platform(&self) -> Platform {
        self.hooks.platform()
    }

    fn now(&self) -> Duration {
        self.state.start.elapsed()
    }

    fn settings(&self) -> SystemSettings {
        // Read from the platform at start and again on every change notification.
        self.state.settings.clone()
    }

    fn total_memory(&self) -> Option<u64> {
        self.hooks.total_memory()
    }

    fn local_time(&self) -> remedy_ui_host::LocalTime {
        let unix_seconds = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        };
        remedy_ui_host::LocalTime {
            unix_seconds,
            utc_offset: self.hooks.utc_offset(unix_seconds).unwrap_or(0),
        }
    }

    fn create_window(&mut self, options: &WindowOptions) -> Result<WindowId, Unsupported> {
        let mut attrs = WindowAttributes::default()
            .with_title(options.title.clone())
            .with_inner_size(LogicalSize::new(f64::from(options.size.width), f64::from(options.size.height)))
            .with_resizable(options.resizable)
            .with_decorations(options.decorated)
            .with_transparent(options.transparent)
            .with_visible(false)
            .with_window_level(if options.always_on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            });
        if let Some(m) = options.min_size {
            attrs = attrs.with_min_inner_size(LogicalSize::new(f64::from(m.width), f64::from(m.height)));
        }
        if let Some(p) = options.position {
            attrs = attrs.with_position(LogicalPosition::new(f64::from(p.x), f64::from(p.y)));
        }
        let attrs = self.hooks.window_attributes(attrs);
        let window =
            Arc::new(self.el.create_window(attrs).map_err(|e| Unsupported::new("windows", e.to_string()))?);
        self.hooks.window_created(&window);
        // The accessibility adapter must exist before the window is first shown.
        let a11y = self
            .state
            .proxy
            .lock()
            .ok()
            .map(|p| accesskit_winit::Adapter::with_event_loop_proxy(self.el, &window, p.clone()));
        // Best effort: where the system cannot keep windows out of capture, the window still
        // opens (the runner asks again and tells the app whether protection holds).
        if options.capture_protected {
            let _ = self.hooks.set_capture_protected(&window, true);
        }
        let id = WindowId(self.state.next_window);
        self.state.next_window += 1;
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        self.state.ids.insert(id, window.id());
        if options.visible {
            window.set_visible(true);
        }
        self.state.windows.insert(
            window.id(),
            (
                id,
                Win {
                    window,
                    a11y,
                    surface: None,
                    cursor: Point::ZERO,
                    buttons: Buttons::default(),
                    composing: false,
                    hovered: Vec::new(),
                    dropped: Vec::new(),
                    dragging: false,
                },
            ),
        );
        self.state.pending.push_back(HostEvent::Resized {
            window: id,
            width: size.width.max(1),
            height: size.height.max(1),
            scale,
        });
        Ok(id)
    }

    fn close_window(&mut self, window: WindowId) {
        if let Some(wid) = self.state.ids.remove(&window) {
            if let Some((_, w)) = self.state.windows.get(&wid) {
                self.hooks.window_closed(&w.window);
            }
            self.state.windows.remove(&wid);
            self.state.pending.push_back(HostEvent::Destroyed(window));
        }
    }

    fn window_size(&self, window: WindowId) -> Option<(u32, u32, f32)> {
        let w = self.win(window)?;
        let s = w.window.inner_size();
        Some((s.width, s.height, w.window.scale_factor() as f32))
    }

    fn set_title(&mut self, window: WindowId, title: &str) {
        if let Some(w) = self.win(window) {
            w.window.set_title(title);
        }
    }

    fn set_size(&mut self, window: WindowId, size: Size) -> Result<(), Unsupported> {
        if let Some(w) = self.win(window) {
            // The platform may adjust or refuse; a Resized event reports what happened.
            let _ =
                w.window.request_inner_size(LogicalSize::new(f64::from(size.width), f64::from(size.height)));
        }
        Ok(())
    }

    fn set_visible(&mut self, window: WindowId, visible: bool) {
        if let Some(w) = self.win(window) {
            w.window.set_visible(visible);
        }
    }

    fn window_position(&self, window: WindowId) -> Option<Point> {
        let w = self.win(window)?;
        let p = w.window.outer_position().ok()?;
        let l: LogicalPosition<f64> = p.to_logical(w.window.scale_factor());
        Some(Point::new(l.x as f32, l.y as f32))
    }

    fn set_position(&mut self, window: WindowId, position: Point) -> Result<(), Unsupported> {
        if let Some(w) = self.win(window) {
            w.window.set_outer_position(LogicalPosition::new(f64::from(position.x), f64::from(position.y)));
        }
        Ok(())
    }

    fn focus_window(&mut self, window: WindowId) {
        if let Some(w) = self.win(window) {
            w.window.focus_window();
        }
    }

    fn set_minimized(&mut self, window: WindowId, minimized: bool) {
        if let Some(w) = self.win(window) {
            w.window.set_minimized(minimized);
        }
    }

    fn set_maximized(&mut self, window: WindowId, maximized: bool) {
        if let Some(w) = self.win(window) {
            w.window.set_maximized(maximized);
        }
    }

    fn is_maximized(&self, window: WindowId) -> bool {
        let Some(wid) = self.state.ids.get(&window) else { return false };
        self.state.windows.get(wid).is_some_and(|(_, w)| w.window.is_maximized())
    }

    fn begin_move(&mut self, window: WindowId) -> Result<(), Unsupported> {
        match self.win(window) {
            Some(w) => w.window.drag_window().map_err(|e| Unsupported::new("window moves", e.to_string())),
            None => Ok(()),
        }
    }

    fn begin_resize(&mut self, window: WindowId, edge: ResizeEdge) -> Result<(), Unsupported> {
        use winit::window::ResizeDirection as D;
        let d = match edge {
            ResizeEdge::Top => D::North,
            ResizeEdge::Bottom => D::South,
            ResizeEdge::Left => D::West,
            ResizeEdge::Right => D::East,
            ResizeEdge::TopLeft => D::NorthWest,
            ResizeEdge::TopRight => D::NorthEast,
            ResizeEdge::BottomLeft => D::SouthWest,
            ResizeEdge::BottomRight => D::SouthEast,
        };
        match self.win(window) {
            Some(w) => {
                w.window.drag_resize_window(d).map_err(|e| Unsupported::new("window resizes", e.to_string()))
            }
            None => Ok(()),
        }
    }

    fn set_always_on_top(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        if let Some(w) = self.win(window) {
            w.window.set_window_level(if on { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal });
        }
        Ok(())
    }

    fn set_capture_protected(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        let Some(wid) = self.state.ids.get(&window) else { return Ok(()) };
        let Some((_, w)) = self.state.windows.get(wid) else { return Ok(()) };
        self.hooks.set_capture_protected(&w.window, on)
    }

    fn request_frame(&mut self, window: WindowId) {
        if let Some(w) = self.win(window) {
            w.window.request_redraw();
        }
    }

    fn present_pixels(
        &mut self,
        window: WindowId,
        pixels: Pixels<'_>,
        damage: &[Rect],
    ) -> Result<(), Unsupported> {
        let fail = |e: softbuffer::SoftBufferError| Unsupported::new("CPU presents", e.to_string());
        let Some(wid) = self.state.ids.get(&window).copied() else {
            return Err(Unsupported::new("present", "the window is closed"));
        };
        if self.state.softbuffer.is_none() {
            let any = self.state.windows.get(&wid).map(|(_, w)| w.window.clone());
            if let Some(win) = any {
                self.state.softbuffer = Some(softbuffer::Context::new(win).map_err(fail)?);
            }
        }
        let Some(ctx) = self.state.softbuffer.as_ref() else {
            return Err(Unsupported::new("CPU presents", "no display connection"));
        };
        let Some((_, w)) = self.state.windows.get_mut(&wid) else {
            return Err(Unsupported::new("present", "the window is closed"));
        };
        let (Some(width), Some(height)) = (NonZeroU32::new(pixels.width), NonZeroU32::new(pixels.height))
        else {
            return Err(Unsupported::new("present", "empty pixels"));
        };
        if pixels.data.len() != pixels.width as usize * pixels.height as usize * 4 {
            return Err(Unsupported::new("present", "the pixel data does not match its size"));
        }
        if w.surface.is_none() {
            w.surface = Some(softbuffer::Surface::new(ctx, w.window.clone()).map_err(fail)?);
        }
        let Some(surface) = w.surface.as_mut() else { return Err(Unsupported::new("present", "no surface")) };
        surface.resize(width, height).map_err(fail)?;
        let mut buffer = surface.buffer_mut().map_err(fail)?;
        let stride = pixels.width as usize;
        let src = pixels.data.as_chunks::<4>().0;
        // A buffer that held the previous frame needs only the damage; any other (new, or
        // older in a swap chain) is written whole.
        let rects: Vec<(usize, usize, usize, usize)> = if buffer.age() == 1 {
            damage
                .iter()
                .map(|r| {
                    let x0 = r.min.x.floor().clamp(0.0, pixels.width as f32) as usize;
                    let x1 = r.max.x.ceil().clamp(0.0, pixels.width as f32) as usize;
                    let y0 = r.min.y.floor().clamp(0.0, pixels.height as f32) as usize;
                    let y1 = r.max.y.ceil().clamp(0.0, pixels.height as f32) as usize;
                    (x0, y0, x1, y1)
                })
                .collect()
        } else {
            vec![(0, 0, stride, pixels.height as usize)]
        };
        for &(x0, y0, x1, y1) in &rects {
            for y in y0..y1 {
                let span = y * stride + x0..y * stride + x1;
                if let (Some(dst), Some(px)) = (buffer.get_mut(span.clone()), src.get(span)) {
                    for (d, p) in dst.iter_mut().zip(px) {
                        *d = translate::pack(p);
                    }
                }
            }
        }
        let sb: Vec<softbuffer::Rect> = rects
            .iter()
            .filter_map(|&(x0, y0, x1, y1)| {
                Some(softbuffer::Rect {
                    x: u32::try_from(x0).ok()?,
                    y: u32::try_from(y0).ok()?,
                    width: NonZeroU32::new(u32::try_from(x1 - x0).ok()?)?,
                    height: NonZeroU32::new(u32::try_from(y1 - y0).ok()?)?,
                })
            })
            .collect();
        w.window.pre_present_notify();
        if sb.is_empty() { buffer.present() } else { buffer.present_with_damage(&sb) }.map_err(fail)
    }

    fn window_handles(&self, window: WindowId) -> Result<Arc<dyn WindowHandles>, Unsupported> {
        match self.win(window) {
            Some(w) => Ok(w.window.clone()),
            None => Err(Unsupported::new("window handles", "the window is closed")),
        }
    }

    fn set_text_input(&mut self, window: WindowId, state: Option<TextInputState>) {
        let Some(wid) = self.state.ids.get(&window).copied() else { return };
        let Some((_, w)) = self.state.windows.get(&wid) else { return };
        if self.hooks.set_text_input(&w.window, state.as_ref()) {
            // The platform's text services own the session; winit's must stay out of it.
            w.window.set_ime_allowed(false);
            return;
        }
        match state {
            Some(s) => {
                w.window.set_ime_allowed(true);
                w.window.set_ime_purpose(if s.private {
                    winit::window::ImePurpose::Password
                } else {
                    winit::window::ImePurpose::Normal
                });
                w.window.set_ime_cursor_area(
                    LogicalPosition::new(f64::from(s.caret.min.x), f64::from(s.caret.min.y)),
                    LogicalSize::new(f64::from(s.caret.width().max(1.0)), f64::from(s.caret.height())),
                );
            }
            None => w.window.set_ime_allowed(false),
        }
    }

    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
        self.hooks.clipboard_text()
    }

    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
        self.hooks.set_clipboard_text(text, sensitive)
    }

    fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
        self.hooks.clear_clipboard()
    }

    fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
        self.hooks.clipboard_html()
    }

    fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
        self.hooks.set_clipboard_html(text, html)
    }

    fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
        self.hooks.set_tray(tray)
    }

    fn update_accessibility(&mut self, window: WindowId, update: remedy_ui_host::accesskit::TreeUpdate) {
        let Some(wid) = self.state.ids.get(&window).copied() else { return };
        if let Some((_, w)) = self.state.windows.get_mut(&wid)
            && let Some(a) = w.a11y.as_mut()
        {
            a.update_if_active(|| update);
        }
    }

    fn file_dialog(
        &mut self,
        window: Option<WindowId>,
        dialog: &FileDialog,
    ) -> Result<Vec<std::path::PathBuf>, Unsupported> {
        let w = window.and_then(|id| self.win(id)).map(|w| w.window.clone());
        self.hooks.file_dialog(w.as_deref(), dialog)
    }

    fn start_drag(
        &mut self,
        window: WindowId,
        data: &DragData,
        allowed: &[DropEffect],
    ) -> Result<DragStarted, Unsupported> {
        let Some(w) = self.win(window).map(|w| w.window.clone()) else {
            return Err(Unsupported::new("dragging out", "the window is gone"));
        };
        self.hooks.start_drag(&w, data, allowed)
    }

    fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
        if !remedy_ui_host::is_openable_url(url) {
            return Err(Unsupported::new("opening links", "only web and mail links open"));
        }
        self.hooks.open_url(url)
    }

    fn set_drop_effect(&mut self, window: WindowId, effect: DropEffect) {
        if let Some(w) = self.win(window).map(|w| w.window.clone()) {
            self.hooks.set_drop_effect(&w, effect);
        }
    }

    fn set_timer(&mut self, at: Duration) -> TimerId {
        let id = TimerId(self.state.next_timer);
        self.state.next_timer += 1;
        self.state.timers.insert(id, at);
        id
    }

    fn cancel_timer(&mut self, timer: TimerId) {
        self.state.timers.remove(&timer);
    }

    fn waker(&self) -> Waker {
        let proxy = self.state.proxy.clone();
        Arc::new(move || {
            if let Ok(p) = proxy.lock() {
                let _ = p.send_event(UserEvent::Wake);
            }
        })
    }

    fn exit(&mut self) {
        self.state.exit = true;
        self.el.exit();
    }
}

struct Runner<A: App, H: PlatformHooks> {
    app: A,
    hooks: H,
    state: State,
}

impl<A: App, H: PlatformHooks> Runner<A, H> {
    /// Hands over the files winit reported since the last flush, as one drag event each.
    fn flush_file_drops(&mut self, el: &ActiveEventLoop) {
        let mut out = Vec::new();
        for (id, w) in self.state.windows.values_mut() {
            if !w.hovered.is_empty() && !w.dragging {
                w.dragging = true;
                out.push(HostEvent::Drag(
                    *id,
                    DragEvent::Enter {
                        position: None,
                        data: DragData::files(std::mem::take(&mut w.hovered)),
                    },
                ));
            }
            w.hovered.clear();
            if !w.dropped.is_empty() {
                w.dragging = false;
                out.push(HostEvent::Drag(
                    *id,
                    DragEvent::Drop { position: None, data: DragData::files(std::mem::take(&mut w.dropped)) },
                ));
            }
        }
        for e in out {
            self.deliver(el, e);
        }
    }

    fn deliver(&mut self, el: &ActiveEventLoop, event: HostEvent) {
        let mut queue = VecDeque::from([event]);
        while let Some(e) = queue.pop_front() {
            let mut ctx = Ctx { el, state: &mut self.state, hooks: &mut self.hooks };
            self.app.event(&mut ctx, e);
            queue.extend(self.state.pending.drain(..));
            queue.extend(self.platform_ime());
            if self.state.exit {
                break;
            }
        }
    }

    /// Input-method events from the platform's text services, for open windows.
    fn platform_ime(&mut self) -> Vec<HostEvent> {
        let mut out = Vec::new();
        for (wid, e) in self.hooks.take_ime_events() {
            let Some((id, w)) = self.state.windows.get_mut(&wid) else { continue };
            match &e {
                ImeEvent::Preedit { text, .. } => w.composing = !text.is_empty(),
                ImeEvent::Commit(_) | ImeEvent::Disabled => w.composing = false,
                _ => {}
            }
            out.push(HostEvent::Ime(*id, e));
        }
        out
    }

    fn fire_timers(&mut self, el: &ActiveEventLoop) {
        let now = self.state.start.elapsed();
        let due: Vec<TimerId> =
            self.state.timers.iter().filter(|(_, at)| **at <= now).map(|(id, _)| *id).collect();
        for id in due {
            self.state.timers.remove(&id);
            self.deliver(el, HostEvent::Timer(id));
        }
    }

    fn pointer(&mut self, el: &ActiveEventLoop, wid: winit::window::WindowId, phase: PointerPhase) {
        let Some((id, w)) = self.state.windows.get(&wid) else { return };
        let e = PointerEvent {
            id: 0,
            kind: PointerKind::Mouse,
            phase,
            position: w.cursor,
            buttons: w.buttons,
            pressure: if w.buttons.0 == 0 { 0.0 } else { 0.5 },
            time: self.state.start.elapsed(),
            coalesced: Vec::new(),
            modifiers: self.state.modifiers,
        };
        let id = *id;
        self.deliver(el, HostEvent::Pointer(id, e));
    }
}

impl<A: App, H: PlatformHooks> ApplicationHandler<UserEvent> for Runner<A, H> {
    fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.fire_timers(el);
        }
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        let settings = self.hooks.settings();
        self.state.settings = settings.clone();
        self.deliver(el, HostEvent::Settings(settings));
        self.deliver(el, HostEvent::Resumed);
    }

    fn suspended(&mut self, el: &ActiveEventLoop) {
        self.deliver(el, HostEvent::Suspended);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Wake => self.deliver(el, HostEvent::Wake),
            UserEvent::Accessibility(e) => {
                let Some((id, _)) = self.state.windows.get(&e.window_id) else { return };
                let id = *id;
                let ev = match e.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => AccessibilityEvent::Requested,
                    accesskit_winit::WindowEvent::ActionRequested(r) => AccessibilityEvent::Action(r),
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => AccessibilityEvent::Deactivated,
                };
                self.deliver(el, HostEvent::Accessibility(id, ev));
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, wid: winit::window::WindowId, event: WindowEvent) {
        let Some((id, w)) = self.state.windows.get_mut(&wid) else { return };
        let id = *id;
        if let Some(a) = w.a11y.as_mut() {
            a.process_event(&w.window, &event);
        }
        let scale = w.window.scale_factor();
        let now = self.state.start.elapsed();
        match event {
            WindowEvent::Resized(size) => {
                let e = HostEvent::Resized {
                    window: id,
                    width: size.width.max(1),
                    height: size.height.max(1),
                    scale: scale as f32,
                };
                self.deliver(el, e);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = w.window.inner_size();
                let e = HostEvent::Resized {
                    window: id,
                    width: size.width.max(1),
                    height: size.height.max(1),
                    scale: scale_factor as f32,
                };
                self.deliver(el, e);
            }
            WindowEvent::CloseRequested => self.deliver(el, HostEvent::CloseRequested(id)),
            WindowEvent::Moved(p) => {
                let l: LogicalPosition<f64> = p.to_logical(scale);
                self.deliver(el, HostEvent::Moved(id, Point::new(l.x as f32, l.y as f32)));
            }
            WindowEvent::Destroyed => {}
            // winit's file drops, where the platform does not report drags itself: one event
            // per file, gathered and handed over before the loop waits.
            WindowEvent::HoveredFile(path) if !self.hooks.handles_drops() => w.hovered.push(path),
            WindowEvent::DroppedFile(path) if !self.hooks.handles_drops() => w.dropped.push(path),
            WindowEvent::HoveredFileCancelled if !self.hooks.handles_drops() => {
                w.hovered.clear();
                if std::mem::take(&mut w.dragging) {
                    self.deliver(el, HostEvent::Drag(id, DragEvent::Leave));
                }
            }
            WindowEvent::Focused(f) => self.deliver(el, HostEvent::Focused(id, f)),
            WindowEvent::Occluded(o) => self.deliver(el, HostEvent::Occluded(id, o)),
            WindowEvent::ThemeChanged(theme) => {
                let mut s = self.hooks.settings();
                s.dark = Some(theme == winit::window::Theme::Dark);
                self.state.settings = s.clone();
                self.deliver(el, HostEvent::Settings(s));
            }
            WindowEvent::RedrawRequested => self.deliver(el, HostEvent::Frame { window: id, at: now }),
            WindowEvent::CursorMoved { position, .. } => {
                w.cursor = logical(position, scale);
                self.pointer(el, wid, PointerPhase::Move);
            }
            WindowEvent::CursorLeft { .. } => {
                if w.buttons.0 == 0 {
                    self.pointer(el, wid, PointerPhase::Cancel);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => Buttons::PRIMARY,
                    MouseButton::Right => Buttons::SECONDARY,
                    MouseButton::Middle => Buttons::MIDDLE,
                    MouseButton::Back => Buttons::BACK,
                    MouseButton::Forward => Buttons::FORWARD,
                    MouseButton::Other(_) => return,
                };
                let phase = if state == ElementState::Pressed {
                    w.buttons = Buttons(w.buttons.0 | b.0);
                    PointerPhase::Down
                } else {
                    w.buttons = Buttons(w.buttons.0 & !b.0);
                    PointerPhase::Up
                };
                self.pointer(el, wid, phase);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (d, precise) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        (Vec2::new(-x * translate::LINE_PIXELS, -y * translate::LINE_PIXELS), false)
                    }
                    MouseScrollDelta::PixelDelta(p) => {
                        let l: LogicalPosition<f64> = p.to_logical(scale);
                        (Vec2::new(-l.x as f32, -l.y as f32), true)
                    }
                };
                let e = WheelEvent { position: w.cursor, delta: d, precise, time: now };
                self.deliver(el, HostEvent::Wheel(id, e));
            }
            WindowEvent::Touch(t) => {
                let phase = match t.phase {
                    TouchPhase::Started => PointerPhase::Down,
                    TouchPhase::Moved => PointerPhase::Move,
                    TouchPhase::Ended => PointerPhase::Up,
                    TouchPhase::Cancelled => PointerPhase::Cancel,
                };
                let pressure = t.force.map_or(0.5, |f| f.normalized() as f32);
                let e = PointerEvent {
                    id: u32::try_from(t.id % u64::from(u32::MAX)).unwrap_or(0).saturating_add(1),
                    kind: PointerKind::Touch,
                    phase,
                    position: logical(t.location, scale),
                    buttons: if matches!(phase, PointerPhase::Up | PointerPhase::Cancel) {
                        Buttons::default()
                    } else {
                        Buttons::PRIMARY
                    },
                    pressure,
                    time: now,
                    coalesced: Vec::new(),
                    modifiers: self.state.modifiers,
                };
                self.deliver(el, HostEvent::Pointer(id, e));
            }
            WindowEvent::ModifiersChanged(m) => self.state.modifiers = translate::modifiers(m.state()),
            WindowEvent::KeyboardInput { event, .. } => {
                let e = KeyEvent {
                    key: translate::key(&event.logical_key),
                    physical: translate::physical(&event.physical_key),
                    modifiers: self.state.modifiers,
                    pressed: event.state == ElementState::Pressed,
                    repeat: event.repeat,
                    composing: w.composing,
                    text: event
                        .text
                        .as_ref()
                        .map(|t| t.to_string())
                        .filter(|t| !t.chars().all(char::is_control)),
                };
                self.deliver(el, HostEvent::Key(id, e));
            }
            WindowEvent::Ime(ime) => {
                let e = match ime {
                    winit::event::Ime::Enabled => ImeEvent::Enabled,
                    winit::event::Ime::Preedit(text, cursor) => {
                        w.composing = !text.is_empty();
                        let cursor = translate::preedit_cursor(&text, cursor);
                        ImeEvent::Preedit { text, cursor }
                    }
                    winit::event::Ime::Commit(text) => {
                        w.composing = false;
                        ImeEvent::Commit(text)
                    }
                    winit::event::Ime::Disabled => {
                        w.composing = false;
                        ImeEvent::Disabled
                    }
                };
                self.deliver(el, HostEvent::Ime(id, e));
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.flush_file_drops(el);
        for (wid, e) in self.hooks.take_drag_events() {
            if let Some((id, _)) = self.state.windows.get(&wid) {
                let id = *id;
                self.deliver(el, HostEvent::Drag(id, e));
            }
        }
        for (wid, e) in self.hooks.take_drag_ends() {
            if let Some((id, _)) = self.state.windows.get(&wid) {
                let id = *id;
                self.deliver(el, HostEvent::DragEnded(id, e));
            }
        }
        // Text services and the tray act inside message dispatch; hand their events over now.
        for e in self.platform_ime() {
            self.deliver(el, e);
        }
        for e in self.hooks.take_events() {
            self.deliver(el, e);
        }
        self.fire_timers(el);
        let next = self.state.timers.values().min().copied();
        el.set_control_flow(match next {
            Some(at) => ControlFlow::WaitUntil(self.state.start + at),
            None => ControlFlow::Wait,
        });
    }
}

/// Runs `app` on `event_loop` until it exits.
pub fn run_on<A: App, H: PlatformHooks>(
    event_loop: EventLoop<UserEvent>,
    app: A,
    mut hooks: H,
) -> Result<(), RunError> {
    let proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
    let settings = hooks.settings();
    let state = State {
        start: Instant::now(),
        windows: HashMap::new(),
        ids: HashMap::new(),
        next_window: 1,
        next_timer: 1,
        timers: BTreeMap::new(),
        pending: VecDeque::new(),
        proxy,
        modifiers: Modifiers::NONE,
        softbuffer: None,
        settings,
        exit: false,
    };
    let mut hooks = hooks;
    let wake_proxy = state.proxy.clone();
    hooks.set_waker(Arc::new(move || {
        if let Ok(p) = wake_proxy.lock() {
            let _ = p.send_event(UserEvent::Wake);
        }
    }));
    let mut runner = Runner { app, hooks, state };
    event_loop.run_app(&mut runner).map_err(|e| RunError(e.to_string()))
}

/// Runs `app` on a new event loop (the main thread's) until it exits.
pub fn run<A: App, H: PlatformHooks>(app: A, hooks: H) -> Result<(), RunError> {
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
    {
        // One event loop per process: the display server is chosen before it is built.
        match backend::choose(&backend::Env::current(), backend::socket_answers) {
            backend::Backend::X11 => {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            }
            backend::Backend::Wayland => {
                use winit::platform::wayland::EventLoopBuilderExtWayland;
                builder.with_wayland();
            }
            backend::Backend::Auto => {}
        }
    }
    let event_loop = builder.build().map_err(|e| RunError(e.to_string()))?;
    run_on(event_loop, app, hooks)
}
