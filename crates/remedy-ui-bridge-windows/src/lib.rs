//! RemedyUI's Windows bridge.
//!
//! The shared winit core ([`remedy_ui_bridge_winit`]) provides windows, input, frames, input
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
mod hooks {
    use remedy_ui_bridge_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use remedy_ui_bridge_winit::winit::window::Window;
    use remedy_ui_bridge_winit::{PlatformHooks, RunError};
    use remedy_ui_host::{App, SystemSettings, Unsupported};
    use remedy_ui_input::Platform;

    use crate::os;

    /// Windows' side of the host contract.
    #[derive(Debug, Default)]
    pub struct WindowsHooks;

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
    }

    /// Runs `app` on the main thread's event loop until it exits.
    pub fn run(app: impl App) -> Result<(), RunError> {
        remedy_ui_bridge_winit::run(app, WindowsHooks)
    }
}

#[cfg(windows)]
pub use hooks::{WindowsHooks, run};
