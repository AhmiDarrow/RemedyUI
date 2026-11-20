//! RemedyUI's Windows bridge.
//!
//! The shared winit core (`remedy-ui-bridge-winit`) provides windows, input, frames, input
//! methods, CPU presents and GPU surface handles; this crate adds what Windows does beyond it:
//! the Win32 clipboard (sensitive copies kept out of clipboard history and cloud sync),
//! capture protection (`SetWindowDisplayAffinity`), the share window (`DataTransferManager`),
//! and system settings read from Windows
//! (dark mode, accent, high contrast, text scale, animations, transparency, locale, double-click
//! time and caret blink). All Win32 calls are in the `os` module. On other platforms this crate
//! is empty.

#[cfg_attr(not(windows), allow(dead_code))]
mod cf_html;
#[cfg(windows)]
#[allow(unsafe_code)]
mod os;

#[cfg(windows)]
mod text_store;

#[cfg(windows)]
mod hooks {
    use remedy_ui_bridge_winit::winit::platform::windows::WindowAttributesExtWindows;
    use remedy_ui_bridge_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use remedy_ui_bridge_winit::winit::window::{Window, WindowAttributes, WindowId};
    use remedy_ui_bridge_winit::{PlatformHooks, RunError};
    use remedy_ui_host::{
        App, DragData, DragEvent, DragStarted, DropEffect, FileDialog, HostEvent, ImeEvent, ShareOptions,
        ShareStarted, SystemSettings, TextInputState, TrayIcon, Unsupported, WindowIcon,
    };
    use remedy_ui_input::Platform;

    use crate::os;

    /// Windows' side of the host contract.
    #[derive(Default)]
    pub struct WindowsHooks {
        /// TSF, started with the first text field. `Err` when it could not start; winit's
        /// IMM path then serves input methods.
        tsf: Option<Result<os::tsf::Tsf, String>>,
        /// Which winit window each HWND with a TSF document belongs to.
        windows: std::collections::HashMap<isize, WindowId>,
        /// Each window's drop target, and the winit window it belongs to.
        drops: std::collections::HashMap<isize, (WindowId, os::dnd::Registered)>,
        drag_queue: os::dnd::Queue,
        /// Why drops cannot be taken, if registering failed.
        drop_error: Option<String>,
        /// The app's toast notifications.
        toasts: os::toast::Toasts,
        /// The icons each window shows of its own, kept until replaced or the window closes.
        icons: std::collections::HashMap<isize, os::icon::WindowIcons>,
        /// The share window's shares.
        sharing: os::share::Sharing,
    }

    impl std::fmt::Debug for WindowsHooks {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let state = match &self.tsf {
                None => "not started",
                Some(Ok(_)) => "active",
                Some(Err(_)) => "unavailable",
            };
            write!(f, "WindowsHooks(TSF {state}, {} documents)", self.windows.len())
        }
    }

    fn hwnd(window: &Window) -> Option<isize> {
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }

    impl WindowsHooks {
        fn tsf(&mut self) -> Option<&mut os::tsf::Tsf> {
            self.tsf.get_or_insert_with(|| os::tsf::Tsf::new().map_err(|e| e.to_string())).as_mut().ok()
        }

        /// Why TSF is not in use, if it failed to start.
        pub fn tsf_error(&self) -> Option<&str> {
            match &self.tsf {
                Some(Err(e)) => Some(e),
                _ => None,
            }
        }
    }

    impl PlatformHooks for WindowsHooks {
        fn platform(&self) -> Platform {
            Platform::Standard
        }

        fn settings(&mut self) -> SystemSettings {
            let s = os::settings();
            let base = SystemSettings::default();
            {
                let mut v_ = SystemSettings::default();
                v_.platform = Platform::Standard;
                v_.dark = s.dark;
                v_.accent = s.accent;
                v_.high_contrast = s.high_contrast;
                v_.text_scale = s.text_scale.unwrap_or(base.text_scale);
                v_.reduced_motion = s.animations.is_some_and(|on| !on);
                v_.reduced_transparency = s.transparency.is_some_and(|on| !on);
                v_.locale = s.locale.unwrap_or(base.locale);
                v_.double_click = s.double_click;
                v_.caret_blink = s.caret_blink;
                v_.subpixel_text = s.subpixel_text;
                v_
            }
        }

        fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
            os::clipboard_text()
        }

        fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
            os::set_clipboard_text(text, sensitive)
        }

        fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
            os::clear_clipboard()
        }

        fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
            os::clipboard_html()
        }

        fn total_memory(&self) -> Option<u64> {
            os::total_memory()
        }

        fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
            os::set_clipboard_html(text, html)
        }

        fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
            os::tray::set(tray)
        }

        fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
            os::open_url(url)
        }

        fn take_events(&mut self) -> Vec<HostEvent> {
            let mut events: Vec<HostEvent> =
                os::tray::take_events().into_iter().map(HostEvent::Tray).collect();
            events.extend(os::toast::take_events());
            events.extend(self.sharing.take_events());
            events
        }

        fn set_waker(&mut self, waker: remedy_ui_host::Waker) {
            os::toast::set_waker(waker.clone());
            self.sharing.set_waker(waker);
        }

        fn show_notification(
            &mut self,
            notification: &remedy_ui_host::Notification,
        ) -> Result<(), Unsupported> {
            self.toasts.show(notification)
        }

        fn withdraw_notification(&mut self, id: u64) -> Result<(), Unsupported> {
            self.toasts.withdraw(id)
        }

        fn share(
            &mut self,
            window: Option<&Window>,
            share: &ShareOptions,
        ) -> Result<ShareStarted, Unsupported> {
            let w =
                window.ok_or_else(|| Unsupported::new("sharing", "the share window needs an open window"))?;
            let h = hwnd(w).ok_or_else(|| Unsupported::new("sharing", "not a Win32 window"))?;
            self.sharing.share(h, share)
        }

        fn utc_offset(&self, unix_seconds: i64) -> Option<i32> {
            crate::os::utc_offset(unix_seconds)
        }

        fn file_dialog(
            &mut self,
            window: Option<&Window>,
            dialog: &FileDialog,
        ) -> Result<Vec<std::path::PathBuf>, Unsupported> {
            let mut d = rfd::FileDialog::new();
            if !dialog.title.is_empty() {
                d = d.set_title(&dialog.title);
            }
            if let Some(dir) = &dialog.directory {
                d = d.set_directory(dir);
            }
            if let Some(name) = &dialog.file_name {
                d = d.set_file_name(name);
            }
            for (name, exts) in &dialog.filters {
                d = d.add_filter(name, exts);
            }
            if let Some(w) = window {
                d = d.set_parent(w);
            }
            Ok(match (dialog.save, dialog.folders, dialog.multiple) {
                (true, _, _) => d.save_file().into_iter().collect(),
                (false, true, true) => d.pick_folders().unwrap_or_default(),
                (false, true, false) => d.pick_folder().into_iter().collect(),
                (false, false, true) => d.pick_files().unwrap_or_default(),
                (false, false, false) => d.pick_file().into_iter().collect(),
            })
        }

        fn set_capture_protected(&mut self, window: &Window, on: bool) -> Result<(), Unsupported> {
            let handle =
                window.window_handle().map_err(|e| Unsupported::new("capture protection", e.to_string()))?;
            match handle.as_raw() {
                RawWindowHandle::Win32(h) => os::set_capture_protected(h.hwnd.get(), on),
                _ => Err(Unsupported::new("capture protection", "not a Win32 window")),
            }
        }

        fn set_window_icon(&mut self, window: &Window, icon: Option<&WindowIcon>) -> Result<(), Unsupported> {
            let h = hwnd(window).ok_or_else(|| Unsupported::new("window icons", "not a Win32 window"))?;
            // The old icons go only after the window has switched to the new ones.
            match os::icon::set_window_icon(h, icon)? {
                Some(icons) => self.icons.insert(h, icons),
                None => self.icons.remove(&h),
            };
            Ok(())
        }

        fn set_text_input(&mut self, window: &Window, state: Option<&TextInputState>) -> bool {
            let Some(h) = hwnd(window) else { return false };
            let scale = window.scale_factor() as f32;
            let id = window.id();
            let Some(tsf) = self.tsf() else { return false };
            match state {
                // Secrets get no input method, as Windows does for its own password boxes:
                // nothing is composed, learned or predicted from them.
                Some(s) if s.private => tsf.blur(h),
                Some(s) => {
                    if tsf.focus(h, scale, s).is_err() {
                        return false;
                    }
                    self.windows.insert(h, id);
                }
                None => tsf.blur(h),
            }
            true
        }

        fn take_ime_events(&mut self) -> Vec<(WindowId, ImeEvent)> {
            let Some(Ok(tsf)) = self.tsf.as_mut() else { return Vec::new() };
            let windows = &self.windows;
            tsf.take_events().into_iter().filter_map(|(h, e)| windows.get(&h).map(|id| (*id, e))).collect()
        }

        fn window_attributes(&mut self, attrs: WindowAttributes) -> WindowAttributes {
            // Our own OLE drop target replaces winit's, which reports only files.
            attrs.with_drag_and_drop(false)
        }

        fn handles_drops(&self) -> bool {
            self.drop_error.is_none()
        }

        fn window_created(&mut self, window: &Window) {
            let Some(h) = hwnd(window) else { return };
            match os::dnd::register(h, window.scale_factor() as f32, self.drag_queue.clone()) {
                Ok(r) => {
                    self.drops.insert(h, (window.id(), r));
                }
                Err(e) => self.drop_error = Some(e.to_string()),
            }
        }

        fn take_drag_events(&mut self) -> Vec<(WindowId, DragEvent)> {
            let events = std::mem::take(&mut *self.drag_queue.borrow_mut());
            events.into_iter().filter_map(|(h, e)| self.drops.get(&h).map(|(id, _)| (*id, e))).collect()
        }

        fn set_drop_effect(&mut self, window: &Window, effect: DropEffect) {
            if let Some(h) = hwnd(window)
                && let Some((_, r)) = self.drops.get(&h)
            {
                r.effect.set(effect);
                r.scale.set(window.scale_factor() as f32);
            }
        }

        fn start_drag(
            &mut self,
            window: &Window,
            data: &DragData,
            allowed: &[DropEffect],
        ) -> Result<DragStarted, Unsupported> {
            let h = hwnd(window).ok_or_else(|| Unsupported::new("dragging out", "not a Win32 window"))?;
            // OLE drags are modal: the call returns when the drop is done.
            os::dnd::drag_out(h, data, allowed).map(DragStarted::Finished)
        }

        fn window_closed(&mut self, window: &Window) {
            let Some(h) = hwnd(window) else { return };
            if self.icons.contains_key(&h) {
                let _ = os::icon::set_window_icon(h, None);
                self.icons.remove(&h);
            }
            self.drops.remove(&h);
            self.sharing.window_closed(h);
            if let Some(Ok(tsf)) = self.tsf.as_mut() {
                tsf.close(h);
            }
            self.windows.remove(&h);
        }
    }

    /// Claims the single instance of `app_id` in this user session. `true`: this is the first
    /// instance, and `on_launch` gets each later launch's arguments (on a background thread).
    /// `false`: another instance runs and has been handed `args`; this one should exit.
    pub fn claim_single_instance(
        app_id: &str,
        args: &[String],
        on_launch: impl Fn(Vec<String>) + Send + 'static,
    ) -> Result<bool, Unsupported> {
        os::instance::claim(app_id, args, on_launch)
    }

    /// Runs `app` on the main thread's event loop until it exits.
    pub fn run(app: impl App) -> Result<(), RunError> {
        remedy_ui_bridge_winit::run(app, WindowsHooks::default())
    }
}

#[cfg(windows)]
pub use hooks::{WindowsHooks, claim_single_instance, run};
