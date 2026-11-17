//! RemedyUI's Windows bridge.
//!
//! The shared winit core (`remedy-ui-bridge-winit`) provides windows, input, frames, input
//! methods, CPU presents and GPU surface handles; this crate adds what Windows does beyond it:
//! the Win32 clipboard (sensitive copies kept out of clipboard history and cloud sync),
//! capture protection (`SetWindowDisplayAffinity`), and system settings read from Windows
//! (dark mode, accent, high contrast, text scale, animations, transparency, locale, double-click
//! time and caret blink). All Win32 calls are in the `os` module. On other platforms this crate
//! is empty.

#[cfg(windows)]
#[allow(unsafe_code)]
mod os;

#[cfg(windows)]
mod text_store;

#[cfg(windows)]
mod hooks {
    use remedy_ui_bridge_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use remedy_ui_bridge_winit::winit::window::{Window, WindowId};
    use remedy_ui_bridge_winit::{PlatformHooks, RunError};
    use remedy_ui_host::{App, ImeEvent, SystemSettings, TextInputState, Unsupported};
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
            SystemSettings {
                platform: Platform::Standard,
                dark: s.dark,
                accent: s.accent,
                high_contrast: s.high_contrast,
                text_scale: s.text_scale.unwrap_or(base.text_scale),
                reduced_motion: s.animations.is_some_and(|on| !on),
                reduced_transparency: s.transparency.is_some_and(|on| !on),
                locale: s.locale.unwrap_or(base.locale),
                double_click: s.double_click,
                caret_blink: s.caret_blink,
            }
        }

        fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
            os::clipboard_text()
        }

        fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
            os::set_clipboard_text(text, sensitive)
        }

        fn set_capture_protected(&mut self, window: &Window, on: bool) -> Result<(), Unsupported> {
            let handle =
                window.window_handle().map_err(|e| Unsupported::new("capture protection", e.to_string()))?;
            match handle.as_raw() {
                RawWindowHandle::Win32(h) => os::set_capture_protected(h.hwnd.get(), on),
                _ => Err(Unsupported::new("capture protection", "not a Win32 window")),
            }
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

        fn window_closed(&mut self, window: &Window) {
            let Some(h) = hwnd(window) else { return };
            if let Some(Ok(tsf)) = self.tsf.as_mut() {
                tsf.close(h);
            }
            self.windows.remove(&h);
        }
    }

    /// Runs `app` on the main thread's event loop until it exits.
    pub fn run(app: impl App) -> Result<(), RunError> {
        remedy_ui_bridge_winit::run(app, WindowsHooks::default())
    }
}

#[cfg(windows)]
pub use hooks::{WindowsHooks, run};
