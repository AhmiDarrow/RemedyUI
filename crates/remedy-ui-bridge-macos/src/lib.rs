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
    use remedy_ui_host::{App, SystemSettings, Unsupported};
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
