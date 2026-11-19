//! RemedyUI's headless bridge.
//!
//! The whole host model in memory: windows are pixel surfaces, the clock moves only when told,
//! and input, input-method sessions, the clipboard, timers and wakes are scripted. Tests drive
//! apps through exactly the contract real bridges implement, on every platform, and the same
//! code renders previews and images on servers. Presents copy only their damage into the
//! window's surface, so a test that checks the surface also checks that damage was complete.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use remedy_ui_core::{Point, Rect, Size, Vec2};
use remedy_ui_host::{
    App, DragData, DragStarted, DropEffect, FileDialog, Host, HostEvent, ImeEvent, Pixels, ResizeEdge,
    SystemSettings, TextInputState, TimerId, TrayIcon, Unsupported, Waker, WindowHandles, WindowId,
    WindowOptions,
};
use remedy_ui_input::{Modifiers, PinchEvent, PinchPhase, Platform, WheelEvent};

/// A window in memory.
#[derive(Clone, Debug)]
pub struct HeadlessWindow {
    options: WindowOptions,
    width: u32,
    height: u32,
    scale: f32,
    frame_requested: bool,
    surface: Vec<u8>,
    presents: usize,
    last_damage: Vec<Rect>,
    text_input: Option<TextInputState>,
    minimized: bool,
    maximized: bool,
    fullscreen: bool,
}

impl HeadlessWindow {
    /// The options it was opened with, as changed since (title, visibility, …).
    pub fn options(&self) -> &WindowOptions {
        &self.options
    }

    /// Size in device pixels and scale.
    pub fn size(&self) -> (u32, u32, f32) {
        (self.width, self.height, self.scale)
    }

    /// What the window shows: premultiplied RGBA8, everything presented so far.
    pub fn surface(&self) -> &[u8] {
        &self.surface
    }

    /// How many presents it received.
    pub fn presents(&self) -> usize {
        self.presents
    }

    /// The damage of the latest present.
    pub fn last_damage(&self) -> &[Rect] {
        &self.last_damage
    }

    /// The text-input session, if a field asked for one.
    pub fn text_input(&self) -> Option<&TextInputState> {
        self.text_input.as_ref()
    }

    /// True while maximized.
    pub fn is_maximized(&self) -> bool {
        self.maximized
    }

    /// True while it fills the screen.
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// True while minimized.
    pub fn is_minimized(&self) -> bool {
        self.minimized
    }

    /// True while a frame is requested and not yet delivered.
    pub fn is_frame_requested(&self) -> bool {
        self.frame_requested
    }
}

/// The headless host and its script controls.
#[derive(Debug)]
pub struct HeadlessHost {
    clock: Duration,
    /// The wall clock when `clock` was zero, and the UTC offset.
    wall: remedy_ui_host::LocalTime,
    refresh: Duration,
    scale: f32,
    settings: SystemSettings,
    windows: BTreeMap<WindowId, HeadlessWindow>,
    next_window: u64,
    next_timer: u64,
    timers: BTreeMap<TimerId, Duration>,
    queue: VecDeque<HostEvent>,
    clipboard: Option<(String, bool)>,
    /// The primary selection (X11 and Wayland have one; the headless host keeps it too).
    primary: Option<String>,
    /// HTML on the clipboard beside its text.
    clipboard_html: Option<String>,
    /// The machine's memory the scripted host reports.
    total_memory: Option<u64>,
    woken: Arc<AtomicBool>,
    exited: bool,
    /// Whether windows can be kept out of screen capture (Wayland and X11 cannot).
    capture_supported: bool,
    focused: Option<WindowId>,
    interactions: Vec<(WindowId, Option<ResizeEdge>)>,
    tray: Option<TrayIcon>,
    a11y: BTreeMap<WindowId, Vec<remedy_ui_host::accesskit::TreeUpdate>>,
    dialogs: Vec<FileDialog>,
    dialog_answers: VecDeque<Vec<std::path::PathBuf>>,
    dialogs_answer_later: bool,
    drags_out: Vec<(WindowId, DragData)>,
    opened: Vec<String>,
    drag_answers: VecDeque<DropEffect>,
    drop_effects: BTreeMap<WindowId, DropEffect>,
    permissions: Vec<remedy_ui_host::Permission>,
    permission_answers: VecDeque<bool>,
    permissions_answer_later: bool,
    notifications: Vec<remedy_ui_host::Notification>,
    withdrawn: Vec<u64>,
    notifications_off: bool,
}

impl Default for HeadlessHost {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessHost {
    /// A host at time zero with a 60 Hz refresh, scale 1, default settings, and
    /// [`HostEvent::Resumed`] queued.
    pub fn new() -> Self {
        HeadlessHost {
            clock: Duration::ZERO,
            // 2026-01-01T00:00:00Z: tests see the same dates on every machine.
            wall: remedy_ui_host::LocalTime { unix_seconds: 1_767_225_600, utc_offset: 0 },
            refresh: Duration::from_nanos(16_666_667),
            scale: 1.0,
            settings: SystemSettings::default(),
            windows: BTreeMap::new(),
            next_window: 1,
            next_timer: 1,
            timers: BTreeMap::new(),
            queue: VecDeque::from([HostEvent::Resumed]),
            clipboard: None,
            primary: None,
            clipboard_html: None,
            total_memory: None,
            woken: Arc::new(AtomicBool::new(false)),
            exited: false,
            capture_supported: true,
            focused: None,
            interactions: Vec::new(),
            tray: None,
            a11y: BTreeMap::new(),
            dialogs: Vec::new(),
            dialog_answers: VecDeque::new(),
            dialogs_answer_later: false,
            drags_out: Vec::new(),
            opened: Vec::new(),
            drag_answers: VecDeque::new(),
            drop_effects: BTreeMap::new(),
            permissions: Vec::new(),
            permission_answers: VecDeque::new(),
            permissions_answer_later: false,
            notifications: Vec::new(),
            withdrawn: Vec::new(),
            notifications_off: false,
        }
    }

    /// Sets the scale new windows get.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = if scale > 0.0 { scale } else { 1.0 };
        self
    }

    /// Sets the display refresh interval.
    pub fn with_refresh(mut self, interval: Duration) -> Self {
        self.refresh = interval;
        self
    }

    /// Changes system settings and tells the app.
    pub fn change_settings(&mut self, settings: SystemSettings) {
        self.settings = settings.clone();
        self.queue.push_back(HostEvent::Settings(settings));
    }

    /// Queues an event as if the platform sent it.
    pub fn send(&mut self, event: HostEvent) {
        self.queue.push_back(event);
    }

    /// Queues a wheel scroll at `position` (logical pixels) by `delta`.
    pub fn wheel(&mut self, window: WindowId, position: Point, delta: Vec2) {
        self.wheel_with(window, position, delta, Modifiers::NONE);
    }

    /// Queues a wheel scroll with modifier keys held (Ctrl and the wheel zoom).
    pub fn wheel_with(&mut self, window: WindowId, position: Point, delta: Vec2, modifiers: Modifiers) {
        let time = self.clock;
        let e = WheelEvent::new(position, delta).with_precise(true).with_time(time).with_modifiers(modifiers);
        self.queue.push_back(HostEvent::Wheel(window, e));
    }

    /// Queues a whole touchpad pinch at `position`: a start, one change by `scale`, an end.
    pub fn pinch(&mut self, window: WindowId, position: Point, scale: f32) {
        let time = self.clock;
        for (phase, scale) in [(PinchPhase::Start, 1.0), (PinchPhase::Change, scale), (PinchPhase::End, 1.0)]
        {
            self.queue
                .push_back(HostEvent::Pinch(window, PinchEvent::new(phase, position, scale).with_time(time)));
        }
    }

    /// Queues an input method composing `text`, then committing it, as a user typing through
    /// an IME does.
    pub fn compose_and_commit(&mut self, window: WindowId, preedit: &str, commit: &str) {
        let len = preedit.len();
        self.queue.extend([
            HostEvent::Ime(window, ImeEvent::Enabled),
            HostEvent::Ime(window, ImeEvent::Preedit { text: preedit.to_owned(), cursor: Some(len..len) }),
            HostEvent::Ime(window, ImeEvent::Preedit { text: String::new(), cursor: None }),
            HostEvent::Ime(window, ImeEvent::Commit(commit.to_owned())),
            HostEvent::Ime(window, ImeEvent::Disabled),
        ]);
    }

    /// Resizes a window as the user dragging its edge would.
    pub fn user_resize(&mut self, window: WindowId, size: Size) {
        if let Some(w) = self.windows.get_mut(&window) {
            Self::apply_size(w, size);
            self.queue.push_back(HostEvent::Resized {
                window,
                width: w.width,
                height: w.height,
                scale: w.scale,
            });
        }
    }

    fn apply_size(w: &mut HeadlessWindow, size: Size) {
        w.options.size = size;
        w.width = (size.width * w.scale).round().max(1.0) as u32;
        w.height = (size.height * w.scale).round().max(1.0) as u32;
        w.surface = vec![0; w.width as usize * w.height as usize * 4];
    }

    /// Acts like a system that cannot keep windows out of screen capture (`false`), as
    /// Wayland and X11 are, or one that can (the default).
    pub fn set_capture_supported(&mut self, on: bool) {
        self.capture_supported = on;
    }

    /// Moves the clock forward.
    pub fn advance(&mut self, by: Duration) {
        self.clock += by;
    }

    /// A window, for inspection.
    pub fn window(&self, id: WindowId) -> Option<&HeadlessWindow> {
        self.windows.get(&id)
    }

    /// Every open window.
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.windows.keys().copied().collect()
    }

    /// Makes the host report `bytes` of physical memory (none by default).
    pub fn set_total_memory(&mut self, bytes: Option<u64>) {
        self.total_memory = bytes;
    }

    /// The HTML on the clipboard, when formatted text was copied.
    pub fn copied_html(&self) -> Option<&str> {
        self.clipboard_html.as_deref()
    }

    /// Puts `text` and its `html` on the clipboard, as another app copying formatted text.
    pub fn put_clipboard_html(&mut self, text: &str, html: &str) {
        self.clipboard = Some((text.to_owned(), false));
        self.clipboard_html = Some(html.to_owned());
    }

    /// The primary selection: what the app last selected.
    pub fn primary_selection(&self) -> Option<&str> {
        self.primary.as_deref()
    }

    /// What is on the clipboard, and whether it was marked sensitive.
    pub fn clipboard(&self) -> Option<(&str, bool)> {
        self.clipboard.as_ref().map(|(t, s)| (t.as_str(), *s))
    }

    /// The window last brought to the front with [`Host::focus_window`].
    pub fn focused_window(&self) -> Option<WindowId> {
        self.focused
    }

    /// User moves (`None`) and resizes (`Some(edge)`) the app started, oldest first.
    pub fn take_interactions(&mut self) -> Vec<(WindowId, Option<ResizeEdge>)> {
        std::mem::take(&mut self.interactions)
    }

    /// Accessibility updates a window sent, oldest first (drained).
    pub fn take_accessibility(&mut self, window: WindowId) -> Vec<remedy_ui_host::accesskit::TreeUpdate> {
        self.a11y.remove(&window).unwrap_or_default()
    }

    /// Sets the wall clock (now) and the UTC offset; it then moves with the host clock.
    pub fn set_local_time(&mut self, unix_seconds: i64, utc_offset: i32) {
        self.wall = remedy_ui_host::LocalTime {
            unix_seconds: unix_seconds - self.clock.as_secs() as i64,
            utc_offset,
        };
    }

    /// The tray icon the app shows, if any.
    pub fn tray(&self) -> Option<&TrayIcon> {
        self.tray.as_ref()
    }

    /// Queues what the other app does with the next drag out of a window (none queued:
    /// the user lets go outside any target, [`DropEffect::None`]).
    pub fn answer_drag(&mut self, effect: DropEffect) {
        self.drag_answers.push_back(effect);
    }

    /// The links the app opened, oldest first.
    pub fn opened_urls(&self) -> &[String] {
        &self.opened
    }

    /// The drags the app started out of its windows, oldest first.
    pub fn drags_out(&self) -> &[(WindowId, DragData)] {
        &self.drags_out
    }

    /// What the app last said a drop on `window` would do.
    pub fn drop_effect(&self, window: WindowId) -> DropEffect {
        self.drop_effects.get(&window).copied().unwrap_or_default()
    }

    /// Queues what the user picks in the next file dialog (empty: cancel).
    pub fn answer_dialog(&mut self, paths: Vec<std::path::PathBuf>) {
        self.dialog_answers.push_back(paths);
    }

    /// Makes file dialogs stay open after [`Host::file_dialog`] returns, as Android's picker
    /// does; answer one with [`HostEvent::FileDialog`] through [`HeadlessHost::send`], naming
    /// it `DialogId(n)` for the n-th dialog opened (counting from 1).
    pub fn dialogs_answer_later(&mut self, on: bool) {
        self.dialogs_answer_later = on;
    }

    /// Answers the next permission request (true grants it). Without a scripted answer, a
    /// request is granted, as on platforms with no run-time permissions.
    pub fn answer_permission(&mut self, granted: bool) {
        self.permission_answers.push_back(granted);
    }

    /// Makes permission requests wait for an answer after [`Host::request_permission`]
    /// returns, as Android's prompt does; answer one with [`HostEvent::Permission`] through
    /// [`HeadlessHost::send`], naming it `PermissionId(n)` for the n-th request (counting
    /// from 1).
    pub fn permissions_answer_later(&mut self, on: bool) {
        self.permissions_answer_later = on;
    }

    /// The permissions the app asked for, oldest first.
    pub fn permission_requests(&self) -> &[remedy_ui_host::Permission] {
        &self.permissions
    }

    /// The notifications the app showed, oldest first (a replacement is listed again).
    /// Answer one with [`HostEvent::Notification`] through [`HeadlessHost::send`].
    pub fn notifications(&self) -> &[remedy_ui_host::Notification] {
        &self.notifications
    }

    /// The ids of the notifications the app took away, oldest first.
    pub fn withdrawn_notifications(&self) -> &[u64] {
        &self.withdrawn
    }

    /// Makes showing notifications fail, as when the person turned them off for the app.
    pub fn notifications_off(&mut self, off: bool) {
        self.notifications_off = off;
    }

    /// The file dialogs the app opened, oldest first.
    pub fn dialogs(&self) -> &[FileDialog] {
        &self.dialogs
    }

    /// True once the app called [`Host::exit`].
    pub fn has_exited(&self) -> bool {
        self.exited
    }

    /// One turn of the event loop: queued events in order, a wake if one came, due timers,
    /// then one frame for each window that requested one. Returns how many events were
    /// delivered.
    pub fn turn(&mut self, app: &mut dyn App) -> usize {
        let mut delivered = 0;
        loop {
            if self.exited {
                return delivered;
            }
            let next = if let Some(e) = self.queue.pop_front() {
                Some(e)
            } else if self.woken.swap(false, Ordering::SeqCst) {
                Some(HostEvent::Wake)
            } else {
                let due = self.timers.iter().find(|(_, at)| **at <= self.clock).map(|(id, _)| *id);
                due.map(|id| {
                    self.timers.remove(&id);
                    HostEvent::Timer(id)
                })
            };
            match next {
                Some(e) => {
                    app.event(self, e);
                    delivered += 1;
                }
                None => break,
            }
        }
        let wanting: Vec<WindowId> =
            self.windows.iter().filter(|(_, w)| w.frame_requested).map(|(id, _)| *id).collect();
        for window in wanting {
            if let Some(w) = self.windows.get_mut(&window) {
                w.frame_requested = false;
            }
            let at = self.clock;
            app.event(self, HostEvent::Frame { window, at });
            delivered += 1;
            // Events the frame queued (a close, a new window) run in this same turn.
            while let Some(e) = self.queue.pop_front() {
                app.event(self, e);
                delivered += 1;
            }
        }
        delivered
    }

    /// Runs `frames` display refreshes: a turn, then the clock moves one refresh.
    pub fn run_frames(&mut self, app: &mut dyn App, frames: usize) {
        for _ in 0..frames {
            self.turn(app);
            self.advance(self.refresh);
        }
    }

    fn window_mut(&mut self, id: WindowId) -> Option<&mut HeadlessWindow> {
        self.windows.get_mut(&id)
    }
}

impl Host for HeadlessHost {
    fn platform(&self) -> Platform {
        self.settings.platform
    }

    fn now(&self) -> Duration {
        self.clock
    }

    fn local_time(&self) -> remedy_ui_host::LocalTime {
        remedy_ui_host::LocalTime {
            unix_seconds: self.wall.unix_seconds + self.clock.as_secs() as i64,
            utc_offset: self.wall.utc_offset,
        }
    }

    fn settings(&self) -> SystemSettings {
        self.settings.clone()
    }

    fn total_memory(&self) -> Option<u64> {
        self.total_memory
    }

    fn create_window(&mut self, options: &WindowOptions) -> Result<WindowId, Unsupported> {
        let id = WindowId(self.next_window);
        self.next_window += 1;
        let mut w = HeadlessWindow {
            options: options.clone(),
            width: 1,
            height: 1,
            scale: self.scale,
            frame_requested: false,
            surface: Vec::new(),
            presents: 0,
            last_damage: Vec::new(),
            text_input: None,
            minimized: false,
            maximized: false,
            fullscreen: false,
        };
        Self::apply_size(&mut w, options.size);
        self.queue.push_back(HostEvent::Resized {
            window: id,
            width: w.width,
            height: w.height,
            scale: w.scale,
        });
        self.windows.insert(id, w);
        Ok(id)
    }

    fn close_window(&mut self, window: WindowId) {
        if self.windows.remove(&window).is_some() {
            self.queue.push_back(HostEvent::Destroyed(window));
        }
    }

    fn window_size(&self, window: WindowId) -> Option<(u32, u32, f32)> {
        self.windows.get(&window).map(HeadlessWindow::size)
    }

    fn set_title(&mut self, window: WindowId, title: &str) {
        if let Some(w) = self.window_mut(window) {
            w.options.title = title.to_owned();
        }
    }

    fn set_size(&mut self, window: WindowId, size: Size) -> Result<(), Unsupported> {
        self.user_resize(window, size);
        Ok(())
    }

    fn set_visible(&mut self, window: WindowId, visible: bool) {
        if let Some(w) = self.window_mut(window) {
            w.options.visible = visible;
        }
    }

    fn window_position(&self, window: WindowId) -> Option<Point> {
        self.windows.get(&window).map(|w| w.options.position.unwrap_or(Point::ZERO))
    }

    fn set_position(&mut self, window: WindowId, position: Point) -> Result<(), Unsupported> {
        if let Some(w) = self.window_mut(window) {
            w.options.position = Some(position);
            self.queue.push_back(HostEvent::Moved(window, position));
        }
        Ok(())
    }

    fn focus_window(&mut self, window: WindowId) {
        if let Some(w) = self.window_mut(window) {
            w.options.visible = true;
        }
        self.focused = Some(window);
    }

    fn set_minimized(&mut self, window: WindowId, minimized: bool) {
        if let Some(w) = self.window_mut(window) {
            w.minimized = minimized;
        }
    }

    fn set_maximized(&mut self, window: WindowId, maximized: bool) {
        if let Some(w) = self.window_mut(window) {
            w.maximized = maximized;
        }
    }

    fn is_maximized(&self, window: WindowId) -> bool {
        self.window(window).is_some_and(|w| w.maximized)
    }

    fn set_fullscreen(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        if let Some(w) = self.window_mut(window) {
            w.fullscreen = on;
        }
        Ok(())
    }

    fn refresh_interval(&self, window: WindowId) -> Option<Duration> {
        self.windows.contains_key(&window).then_some(self.refresh)
    }

    fn is_fullscreen(&self, window: WindowId) -> bool {
        self.window(window).is_some_and(|w| w.fullscreen)
    }

    fn begin_move(&mut self, window: WindowId) -> Result<(), Unsupported> {
        self.interactions.push((window, None));
        Ok(())
    }

    fn begin_resize(&mut self, window: WindowId, edge: ResizeEdge) -> Result<(), Unsupported> {
        self.interactions.push((window, Some(edge)));
        Ok(())
    }

    fn set_always_on_top(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        if let Some(w) = self.window_mut(window) {
            w.options.always_on_top = on;
        }
        Ok(())
    }

    fn set_capture_protected(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        if on && !self.capture_supported {
            return Err(Unsupported::new(
                "capture protection",
                "this system cannot exclude windows from capture",
            ));
        }
        if let Some(w) = self.window_mut(window) {
            w.options.capture_protected = on;
        }
        Ok(())
    }

    fn request_frame(&mut self, window: WindowId) {
        if let Some(w) = self.window_mut(window) {
            w.frame_requested = true;
        }
    }

    fn present_pixels(
        &mut self,
        window: WindowId,
        pixels: Pixels<'_>,
        damage: &[Rect],
    ) -> Result<(), Unsupported> {
        let Some(w) = self.windows.get_mut(&window) else {
            return Err(Unsupported::new("present", "the window is closed"));
        };
        if (pixels.width, pixels.height) != (w.width, w.height)
            || pixels.data.len() != w.width as usize * w.height as usize * 4
        {
            return Err(Unsupported::new("present", "the pixels are not the window's size"));
        }
        let stride = w.width as usize * 4;
        for r in damage {
            let x0 = r.min.x.floor().clamp(0.0, w.width as f32) as usize;
            let x1 = r.max.x.ceil().clamp(0.0, w.width as f32) as usize;
            let y0 = r.min.y.floor().clamp(0.0, w.height as f32) as usize;
            let y1 = r.max.y.ceil().clamp(0.0, w.height as f32) as usize;
            for y in y0..y1 {
                let span = y * stride + x0 * 4..y * stride + x1 * 4;
                if let (Some(dst), Some(src)) = (w.surface.get_mut(span.clone()), pixels.data.get(span)) {
                    dst.copy_from_slice(src);
                }
            }
        }
        w.presents += 1;
        w.last_damage = damage.to_vec();
        Ok(())
    }

    fn window_handles(&self, _window: WindowId) -> Result<Arc<dyn WindowHandles>, Unsupported> {
        Err(Unsupported::new(
            "GPU surfaces",
            "the headless bridge has no windowing system; it presents CPU-rendered pixels",
        ))
    }

    fn set_text_input(&mut self, window: WindowId, state: Option<TextInputState>) {
        if let Some(w) = self.window_mut(window) {
            w.text_input = state;
        }
    }

    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
        Ok(self.clipboard.as_ref().map(|(t, _)| t.clone()))
    }

    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
        self.clipboard = Some((text.to_owned(), sensitive));
        self.clipboard_html = None;
        Ok(())
    }

    fn primary_selection_text(&mut self) -> Result<Option<String>, Unsupported> {
        Ok(self.primary.clone())
    }

    fn set_primary_selection(&mut self, text: &str) -> Result<(), Unsupported> {
        self.primary = Some(text.to_owned());
        Ok(())
    }

    fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
        self.clipboard = None;
        self.clipboard_html = None;
        Ok(())
    }

    fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
        Ok(self.clipboard_html.clone())
    }

    fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
        self.clipboard = Some((text.to_owned(), false));
        self.clipboard_html = Some(html.to_owned());
        Ok(())
    }

    fn update_accessibility(&mut self, window: WindowId, update: remedy_ui_host::accesskit::TreeUpdate) {
        self.a11y.entry(window).or_default().push(update);
    }

    fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
        self.tray = tray.cloned();
        Ok(())
    }

    fn file_dialog(
        &mut self,
        _window: Option<WindowId>,
        dialog: &FileDialog,
    ) -> Result<remedy_ui_host::DialogStarted, Unsupported> {
        self.dialogs.push(dialog.clone());
        if self.dialogs_answer_later {
            // As a mobile picker: the dialog stays open until the test answers it.
            let id = remedy_ui_host::DialogId(u64::try_from(self.dialogs.len()).unwrap_or(u64::MAX));
            return Ok(remedy_ui_host::DialogStarted::Pending(id));
        }
        // With no scripted answer, the user cancels.
        Ok(remedy_ui_host::DialogStarted::Finished(self.dialog_answers.pop_front().unwrap_or_default()))
    }

    fn show_notification(&mut self, notification: &remedy_ui_host::Notification) -> Result<(), Unsupported> {
        if self.notifications_off {
            return Err(Unsupported::new("system notifications", "turned off for this app"));
        }
        self.notifications.push(notification.clone());
        Ok(())
    }

    fn withdraw_notification(&mut self, id: u64) -> Result<(), Unsupported> {
        self.withdrawn.push(id);
        Ok(())
    }

    fn request_permission(
        &mut self,
        permission: remedy_ui_host::Permission,
    ) -> Result<remedy_ui_host::PermissionStarted, Unsupported> {
        self.permissions.push(permission);
        if self.permissions_answer_later {
            let id = remedy_ui_host::PermissionId(u64::try_from(self.permissions.len()).unwrap_or(u64::MAX));
            return Ok(remedy_ui_host::PermissionStarted::Pending(id));
        }
        Ok(remedy_ui_host::PermissionStarted::Finished(self.permission_answers.pop_front().unwrap_or(true)))
    }

    fn start_drag(
        &mut self,
        window: WindowId,
        data: &DragData,
        allowed: &[DropEffect],
    ) -> Result<DragStarted, Unsupported> {
        self.drags_out.push((window, data.clone()));
        let effect = self.drag_answers.pop_front().unwrap_or_default();
        Ok(DragStarted::Finished(if allowed.contains(&effect) { effect } else { DropEffect::None }))
    }

    fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
        if !remedy_ui_host::is_openable_url(url) {
            return Err(Unsupported::new("opening links", "only web and mail links open"));
        }
        self.opened.push(url.to_owned());
        Ok(())
    }

    fn set_drop_effect(&mut self, window: WindowId, effect: DropEffect) {
        self.drop_effects.insert(window, effect);
    }

    fn set_timer(&mut self, at: Duration) -> TimerId {
        let id = TimerId(self.next_timer);
        self.next_timer += 1;
        self.timers.insert(id, at);
        id
    }

    fn cancel_timer(&mut self, timer: TimerId) {
        self.timers.remove(&timer);
    }

    fn waker(&self) -> Waker {
        let flag = self.woken.clone();
        Arc::new(move || flag.store(true, Ordering::SeqCst))
    }

    fn exit(&mut self) {
        self.exited = true;
    }
}
