//! RemedyUI's macOS bridge.
//!
//! The shared winit core (`remedy-ui-bridge-winit`) provides windows, input, frames, input
//! methods (`NSTextInputClient` through winit), CPU presents and Metal surface handles. This
//! crate adds the general pasteboard (sensitive copies marked concealed and transient, so
//! clipboard managers skip them), window sharing protection (`NSWindowSharingNone`), and
//! system settings: appearance, accent color, increased contrast, reduced motion and
//! transparency, locale and double-click interval; notifications (User Notifications), the
//! Dock tile's badge, and links in the app's URL schemes (the "get URL" Apple event), which
//! reach the app as a later launch's arguments. All AppKit calls are in the `os` module.
//! On other platforms this crate is empty.

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod os;

#[cfg(target_os = "macos")]
mod hooks {
    use remedy_ui_bridge_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use remedy_ui_bridge_winit::winit::window::{Window, WindowId};
    use remedy_ui_bridge_winit::{PlatformHooks, RunError};
    use remedy_ui_host::{
        App, DragData, DragStarted, DropEffect, FileDialog, HostEvent, SystemSettings, TrayIcon, Unsupported,
        Waker,
    };
    use remedy_ui_input::Platform;

    use crate::os;

    /// macOS's side of the host contract.
    #[derive(Debug, Default)]
    pub struct MacHooks;

    impl PlatformHooks for MacHooks {
        fn platform(&self) -> Platform {
            Platform::Apple
        }

        fn settings(&mut self) -> SystemSettings {
            let s = os::settings();
            // macOS has no system-wide text scale (apps follow their own Dynamic Type-like
            // settings, which RemedyUI exposes in its customization layer) and the caret blink
            // keeps RemedyUI's default.
            let mut out = SystemSettings::default();
            out.platform = Platform::Apple;
            out.dark = s.dark;
            out.accent = s.accent;
            out.high_contrast = s.high_contrast;
            out.reduced_motion = s.reduced_motion;
            out.reduced_transparency = s.reduced_transparency;
            if let Some(locale) = s.locale {
                out.locale = locale;
            }
            out.locales = if s.languages.is_empty() { vec![out.locale.clone()] } else { s.languages };
            out.double_click = s.double_click;
            out
        }

        fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
            Ok(os::clipboard_text())
        }

        fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
            os::set_clipboard_text(text, sensitive)
        }

        fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
            Ok(os::clipboard_html())
        }

        fn total_memory(&self) -> Option<u64> {
            Some(objc2_foundation::NSProcessInfo::processInfo().physicalMemory()).filter(|n| *n > 0)
        }

        fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
            os::set_clipboard_html(text, html)
        }

        fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
            os::tray::set(tray)
        }

        fn set_waker(&mut self, waker: Waker) {
            os::tray::set_waker(waker.clone());
            os::notify::set_waker(waker.clone());
            os::url::set_waker(waker.clone());
            os::dnd::set_waker(waker);
            // Again now the app has finished launching, in case AppKit installed its own.
            os::url::listen();
        }

        fn show_notification(
            &mut self,
            notification: &remedy_ui_host::Notification,
        ) -> Result<(), Unsupported> {
            os::notify::show(notification)
        }

        fn withdraw_notification(&mut self, id: u64) -> Result<(), Unsupported> {
            os::notify::withdraw(id)
        }

        fn set_badge(
            &mut self,
            _windows: &[&Window],
            _app_id: &str,
            badge: Option<remedy_ui_host::Badge>,
        ) -> Result<(), Unsupported> {
            os::set_badge(badge.map(remedy_ui_host::Badge::label).as_deref())
        }

        fn start_drag(
            &mut self,
            window: &Window,
            data: &DragData,
            allowed: &[DropEffect],
        ) -> Result<DragStarted, Unsupported> {
            let handle =
                window.window_handle().map_err(|e| Unsupported::new("dragging out", e.to_string()))?;
            match handle.as_raw() {
                RawWindowHandle::AppKit(h) => {
                    // AppKit runs the session on its own and reports its end.
                    os::dnd::drag_out(h.ns_view, window.id(), data, allowed).map(|()| DragStarted::Pending)
                }
                _ => Err(Unsupported::new("dragging out", "not an AppKit window")),
            }
        }

        fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
            os::open_url(url)
        }

        fn share(
            &mut self,
            window: Option<&Window>,
            share: &remedy_ui_host::ShareOptions,
        ) -> Result<remedy_ui_host::ShareStarted, Unsupported> {
            let _ = (window, share);
            Err(Unsupported::new(
                "sharing",
                "the macOS bridge does not show the system's share picker (NSSharingServicePicker)",
            ))
        }

        fn take_drag_ends(&mut self) -> Vec<(WindowId, DropEffect)> {
            os::dnd::take_ended()
        }

        fn take_events(&mut self) -> Vec<HostEvent> {
            let mut events: Vec<HostEvent> =
                os::tray::take_events().into_iter().map(HostEvent::Tray).collect();
            events.extend(os::notify::take_events());
            // A link in one of the app's schemes reaches it like a later launch's arguments.
            events.extend(os::url::take().into_iter().map(|url| HostEvent::SecondInstance(vec![url])));
            events
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

        fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
            os::clear_clipboard();
            Ok(())
        }

        fn set_window_icon(
            &mut self,
            _window: &Window,
            _icon: Option<&remedy_ui_host::WindowIcon>,
        ) -> Result<(), Unsupported> {
            Err(Unsupported::new(
                "window icons",
                "macOS windows have no icons of their own; the Dock and the app switcher show the app's",
            ))
        }

        fn set_capture_protected(&mut self, window: &Window, on: bool) -> Result<(), Unsupported> {
            let handle =
                window.window_handle().map_err(|e| Unsupported::new("capture protection", e.to_string()))?;
            match handle.as_raw() {
                RawWindowHandle::AppKit(h) => os::set_capture_protected(h.ns_view, on),
                _ => Err(Unsupported::new("capture protection", "not an AppKit window")),
            }
        }
    }

    /// Runs `app` on the main thread's event loop until it exits.
    pub fn run(app: impl App) -> Result<(), RunError> {
        // Before the app finishes launching, so the link that launched it is heard.
        os::url::listen();
        remedy_ui_bridge_winit::run(app, MacHooks)
    }
}

#[cfg(target_os = "macos")]
pub use hooks::{MacHooks, run};
