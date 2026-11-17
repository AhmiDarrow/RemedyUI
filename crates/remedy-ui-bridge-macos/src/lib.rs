//! RemedyUI's macOS bridge.
//!
//! The shared winit core (`remedy-ui-bridge-winit`) provides windows, input, frames, input
//! methods (`NSTextInputClient` through winit), CPU presents and Metal surface handles. This
//! crate adds the general pasteboard (sensitive copies marked concealed and transient, so
//! clipboard managers skip them), window sharing protection (`NSWindowSharingNone`), and
//! system settings: appearance, accent color, increased contrast, reduced motion and
//! transparency, locale and double-click interval. All AppKit calls are in the `os` module.
//! On other platforms this crate is empty.

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod os;

#[cfg(target_os = "macos")]
mod hooks {
    use remedy_ui_bridge_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use remedy_ui_bridge_winit::winit::window::Window;
    use remedy_ui_bridge_winit::{PlatformHooks, RunError};
    use remedy_ui_host::{App, FileDialog, HostEvent, SystemSettings, TrayIcon, Unsupported, Waker};
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
            let base = SystemSettings::default();
            SystemSettings {
                platform: Platform::Apple,
                dark: s.dark,
                accent: s.accent,
                high_contrast: s.high_contrast,
                // macOS has no system-wide text scale; apps follow their own Dynamic Type-like
                // settings, which RemedyUI exposes in its customization layer.
                text_scale: base.text_scale,
                reduced_motion: s.reduced_motion,
                reduced_transparency: s.reduced_transparency,
                locale: s.locale.unwrap_or(base.locale),
                double_click: s.double_click,
                caret_blink: base.caret_blink,
            }
        }

        fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
            Ok(os::clipboard_text())
        }

        fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
            os::set_clipboard_text(text, sensitive)
        }

        fn set_tray(&mut self, tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
            os::tray::set(tray)
        }

        fn set_waker(&mut self, waker: Waker) {
            os::tray::set_waker(waker);
        }

        fn take_events(&mut self) -> Vec<HostEvent> {
            os::tray::take_events().into_iter().map(HostEvent::Tray).collect()
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
        remedy_ui_bridge_winit::run(app, MacHooks)
    }
}

#[cfg(target_os = "macos")]
pub use hooks::{MacHooks, run};
