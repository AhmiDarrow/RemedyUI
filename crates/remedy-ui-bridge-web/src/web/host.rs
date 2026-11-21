//! The host contract in a browser page.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_core::{Point, Rect, Size};
use remedy_ui_host::{
    AccessibilityEvent, DialogStarted, DragData, DragStarted, DropEffect, FileDialog, Host, HostEvent,
    LocalTime, Pixels, ResizeEdge, SafeArea, ShareOptions, ShareStarted, SystemSettings, TextInputState,
    TimerId, TrayIcon, Unsupported, Waker, WindowHandles, WindowIcon, WindowId, WindowOptions,
};
use remedy_ui_input::Platform;

use super::window::{Kind, Win};
use super::{Driver, clipboard, dialog, driver, notify, now, post, print, settings, share};
use crate::Mount;

/// The host the app acts through while it handles an event.
pub(crate) struct WebHost<'a> {
    pub(crate) d: &'a Rc<Driver>,
}

impl WebHost<'_> {
    fn win(&self, id: WindowId) -> Option<Rc<Win>> {
        self.d.window(id)
    }

    /// Where a new root window goes: the configured element, `#remedy`, or the body.
    fn mount(&self) -> Option<web_sys::Element> {
        let doc = super::dom::document()?;
        let mount = self.d.state.borrow().config.mount.clone();
        match mount {
            Mount::Element(id) => doc.get_element_by_id(&id),
            Mount::Auto => doc.get_element_by_id("remedy").or_else(|| doc.body().map(Into::into)),
            Mount::Body => doc.body().map(Into::into),
        }
    }
}

impl Host for WebHost<'_> {
    fn platform(&self) -> Platform {
        self.d.state.borrow().settings.platform
    }

    fn now(&self) -> Duration {
        now()
    }

    fn local_time(&self) -> LocalTime {
        let date = js_sys::Date::new_0();
        let ms = date.get_time();
        LocalTime {
            unix_seconds: (ms / 1000.0).floor() as i64,
            // `getTimezoneOffset` is minutes west of UTC.
            utc_offset: (-date.get_timezone_offset() * 60.0) as i32,
        }
    }

    fn settings(&self) -> SystemSettings {
        self.d.state.borrow().settings.clone()
    }

    fn total_memory(&self) -> Option<u64> {
        web_sys::window().and_then(|w| settings::device_memory(&w))
    }

    fn create_window(&mut self, options: &WindowOptions) -> Result<WindowId, Unsupported> {
        let has_root = self.d.state.borrow().windows.values().any(|w| w.kind == Kind::Root);
        let mount = if has_root { None } else { self.mount() };
        let (id, z) = {
            let mut s = self.d.state.borrow_mut();
            let id = WindowId(s.next_window);
            s.next_window += 1;
            s.next_z += 1;
            (id, s.next_z)
        };
        let z = if options.always_on_top { z.saturating_add(1_000_000) } else { z };
        let win = Win::create(self.d, id, options, mount, z)?;
        let page_title = self.d.state.borrow().config.title_is_page_title;
        win.set_title(&options.title, page_title);
        if let Some(icon) = &options.icon {
            // Best effort, as on the desktop: a page whose icon is its own keeps it.
            let _ = win.set_icon(Some(icon), page_title);
        }
        self.d.state.borrow_mut().windows.insert(id, win.clone());
        // Screen readers cannot announce their presence to a page, so the tree is always
        // kept for them.
        post(HostEvent::Accessibility(id, AccessibilityEvent::Requested));
        // The window's size arrives from its resize observer; until layout runs, measure now
        // so the app learns it with the window.
        win.measure();
        Ok(id)
    }

    fn close_window(&mut self, window: WindowId) {
        let gone = self.d.state.borrow_mut().windows.remove(&window);
        if let Some(w) = gone {
            w.aria.clear();
            w.remove();
            post(HostEvent::Destroyed(window));
        }
    }

    fn window_size(&self, window: WindowId) -> Option<(u32, u32, f32)> {
        self.win(window).map(|w| w.size.get())
    }

    fn safe_area(&self, window: WindowId) -> SafeArea {
        self.win(window).map(|w| w.safe_area.get()).unwrap_or_default()
    }

    fn set_title(&mut self, window: WindowId, title: &str) {
        let page_title = self.d.state.borrow().config.title_is_page_title;
        if let Some(w) = self.win(window) {
            w.set_title(title, page_title);
        }
    }

    fn set_size(&mut self, window: WindowId, size: Size) -> Result<(), Unsupported> {
        self.win(window).ok_or_else(closed)?.set_size(size)
    }

    fn set_visible(&mut self, window: WindowId, visible: bool) {
        if let Some(w) = self.win(window) {
            w.set_visible(visible);
        }
    }

    fn window_position(&self, window: WindowId) -> Option<Point> {
        self.win(window).map(|w| w.position())
    }

    fn set_position(&mut self, window: WindowId, position: Point) -> Result<(), Unsupported> {
        self.win(window).ok_or_else(closed)?.set_position(position)
    }

    fn focus_window(&mut self, window: WindowId) {
        if let Some(w) = self.win(window) {
            w.raise();
            let target = w.aria.focused_element().unwrap_or_else(|| w.surface.clone());
            super::dom::focus(&target);
        }
    }

    fn set_minimized(&mut self, window: WindowId, minimized: bool) {
        if let Some(w) = self.win(window) {
            w.set_minimized(minimized);
        }
    }

    fn set_maximized(&mut self, window: WindowId, maximized: bool) {
        if let Some(w) = self.win(window) {
            w.set_maximized(maximized);
        }
    }

    fn is_maximized(&self, window: WindowId) -> bool {
        self.win(window).is_some_and(|w| w.maximized.get())
    }

    fn set_fullscreen(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        self.win(window).ok_or_else(closed)?.set_fullscreen(on)
    }

    fn is_fullscreen(&self, window: WindowId) -> bool {
        self.win(window).is_some_and(|w| w.is_fullscreen())
    }

    fn begin_move(&mut self, window: WindowId) -> Result<(), Unsupported> {
        self.win(window).ok_or_else(closed)?.begin_move()
    }

    fn begin_resize(&mut self, window: WindowId, edge: ResizeEdge) -> Result<(), Unsupported> {
        self.win(window).ok_or_else(closed)?.begin_resize(edge)
    }

    fn set_always_on_top(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        let w = self.win(window).ok_or_else(closed)?;
        if w.kind == Kind::Root {
            return Err(Unsupported::new(
                "windows on top",
                "the page's own window cannot rise above the browser",
            ));
        }
        w.on_top.set(on);
        w.raise();
        Ok(())
    }

    fn set_window_icon(&mut self, window: WindowId, icon: Option<&WindowIcon>) -> Result<(), Unsupported> {
        let page_title = self.d.state.borrow().config.title_is_page_title;
        self.win(window).ok_or_else(closed)?.set_icon(icon, page_title)
    }

    fn set_capture_protected(&mut self, window: WindowId, on: bool) -> Result<(), Unsupported> {
        let _ = window;
        if on {
            Err(Unsupported::new(
                "capture protection",
                "a web page cannot keep itself out of screenshots, recordings or screen sharing",
            ))
        } else {
            Ok(())
        }
    }

    fn request_frame(&mut self, window: WindowId) {
        if let Some(w) = self.win(window) {
            w.request_frame();
        }
    }

    fn present_pixels(
        &mut self,
        window: WindowId,
        pixels: Pixels<'_>,
        damage: &[Rect],
    ) -> Result<(), Unsupported> {
        self.win(window).ok_or_else(closed)?.present(pixels, damage)
    }

    fn window_handles(&self, window: WindowId) -> Result<Arc<dyn WindowHandles>, Unsupported> {
        let w = self.win(window).ok_or_else(closed)?;
        Ok(Arc::new(w.handles()))
    }

    fn pointer_consumed(&mut self, window: WindowId, consumed: bool) {
        if let Some(w) = self.win(window) {
            w.consumed.set(consumed);
        }
    }

    fn renderer_changed(&mut self, window: WindowId, gpu: bool) {
        if let Some(w) = self.win(window) {
            w.show_renderer(gpu);
        }
    }

    fn set_text_input(&mut self, window: WindowId, state: Option<TextInputState>) {
        let Some(w) = self.win(window) else { return };
        match state {
            Some(s) => {
                let label = w.aria.focused_label();
                w.text.update(s, label);
            }
            None => w.text.end(),
        }
    }

    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
        clipboard::text(self.d)
    }

    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
        clipboard::set_text(self.d, text, sensitive);
        Ok(())
    }

    fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
        clipboard::clear(self.d);
        Ok(())
    }

    fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
        clipboard::html(self.d)
    }

    fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
        clipboard::set_html(self.d, text, html);
        Ok(())
    }

    fn update_accessibility(&mut self, window: WindowId, update: remedy_ui_host::accesskit::TreeUpdate) {
        if let Some(w) = self.win(window) {
            w.aria.update(update);
        }
    }

    fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
        match tray {
            None => Ok(()),
            Some(_) => Err(Unsupported::new("tray icons", "a web page has no place in the system tray")),
        }
    }

    fn file_dialog(
        &mut self,
        window: Option<WindowId>,
        dialog: &FileDialog,
    ) -> Result<DialogStarted, Unsupported> {
        let _ = window;
        dialog::file_dialog(self.d, dialog)
    }

    fn share(
        &mut self,
        window: Option<WindowId>,
        options: &ShareOptions,
    ) -> Result<ShareStarted, Unsupported> {
        let _ = window;
        share::share(self.d, options)
    }

    fn print_html(&mut self, window: Option<WindowId>, title: &str, html: &str) -> Result<(), Unsupported> {
        let _ = (window, title);
        print::print_html(html)
    }

    fn start_drag(
        &mut self,
        window: WindowId,
        data: &DragData,
        allowed: &[DropEffect],
    ) -> Result<DragStarted, Unsupported> {
        let _ = (window, data, allowed);
        Err(Unsupported::new(
            "dragging out of the page",
            "a browser starts a drag only from its own dragstart on a draggable element, never from a page's code",
        ))
    }

    fn set_drop_effect(&mut self, window: WindowId, effect: DropEffect) {
        if let Some(w) = self.win(window) {
            w.drop_effect.set(effect);
        }
    }

    fn show_notification(&mut self, notification: &remedy_ui_host::Notification) -> Result<(), Unsupported> {
        notify::show(notification)
    }

    fn withdraw_notification(&mut self, id: u64) -> Result<(), Unsupported> {
        notify::withdraw(id);
        Ok(())
    }

    fn set_badge(&mut self, _app_id: &str, badge: Option<remedy_ui_host::Badge>) -> Result<(), Unsupported> {
        // The Badging API: on the icon of an installed web app (Chromium browsers, Safari).
        let Some(window) = web_sys::window() else {
            return Err(Unsupported::new("app badges", "no browser window"));
        };
        let navigator: wasm_bindgen::JsValue = window.navigator().into();
        let (name, arg) = match badge {
            Some(remedy_ui_host::Badge::Count(n)) => ("setAppBadge", Some(wasm_bindgen::JsValue::from(n))),
            Some(_) => ("setAppBadge", None),
            None => ("clearAppBadge", None),
        };
        let f = super::dom::get(&navigator, name);
        let Some(f) = wasm_bindgen::JsCast::dyn_ref::<js_sys::Function>(&f) else {
            return Err(Unsupported::new("app badges", "this browser has no Badging API"));
        };
        // The promise rejects only where the page may not badge (not installed); nothing to do then.
        let called = match arg {
            Some(a) => f.call1(&navigator, &a),
            None => f.call0(&navigator),
        };
        called.map(|_| ()).map_err(|_| Unsupported::new("app badges", "the browser refused the badge"))
    }

    fn request_permission(
        &mut self,
        permission: remedy_ui_host::Permission,
    ) -> Result<remedy_ui_host::PermissionStarted, Unsupported> {
        match permission {
            remedy_ui_host::Permission::Notifications => notify::request(),
            // The microphone's own `getUserMedia` call asks the person.
            _ => Ok(remedy_ui_host::PermissionStarted::Finished(true)),
        }
    }

    fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
        if !remedy_ui_host::is_openable_url(url) {
            return Err(Unsupported::new("opening links", "only web and mail links open"));
        }
        let win = web_sys::window()
            .ok_or_else(|| Unsupported::new("opening links", "there is no browser window"))?;
        // A new tab that cannot reach back into the app's page.
        win.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer")
            .map(drop)
            .map_err(|e| Unsupported::new("opening links", format!("{e:?}")))
    }

    fn set_timer(&mut self, at: Duration) -> TimerId {
        let id = {
            let mut s = self.d.state.borrow_mut();
            let id = TimerId(s.next_timer);
            s.next_timer += 1;
            id
        };
        if let Some(h) = self.d.arm_timer(id, at) {
            self.d.state.borrow_mut().timers.insert(id, (h, at));
        }
        id
    }

    fn cancel_timer(&mut self, timer: TimerId) {
        let handle = self.d.state.borrow_mut().timers.remove(&timer);
        if let (Some((h, _)), Some(w)) = (handle, web_sys::window()) {
            w.clear_timeout_with_handle(h);
        }
    }

    fn waker(&self) -> Waker {
        Arc::new(|| {
            if let Some(d) = driver() {
                d.wake();
            }
        })
    }

    fn exit(&mut self) {
        self.d.state.borrow_mut().exiting = true;
    }
}

fn closed() -> Unsupported {
    Unsupported::new("windows", "the window is closed")
}
