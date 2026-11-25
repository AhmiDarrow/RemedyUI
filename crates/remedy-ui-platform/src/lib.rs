//! RemedyUI on each platform: the window bridges (Windows, macOS, Linux, Android, headless), desktop services, networking, localization data, web storage and embedded web views and GPU surfaces.
//!
//! Each part is a module behind a Cargo feature of the same name.

#[cfg(feature = "bridge-android")]
pub mod bridge_android;
#[cfg(feature = "bridge-headless")]
pub mod bridge_headless;
#[cfg(feature = "bridge-linux")]
pub mod bridge_linux;
#[cfg(feature = "bridge-macos")]
pub mod bridge_macos;
#[cfg(feature = "bridge-windows")]
pub mod bridge_windows;
// winit windows are for the desktops and Android; a browser page has its own bridge.
#[cfg(all(feature = "bridge-winit", not(all(target_arch = "wasm32", target_os = "unknown"))))]
pub mod bridge_winit;
#[cfg(feature = "embed-gpu")]
pub mod embed_gpu;
#[cfg(feature = "embed-webview")]
pub mod embed_webview;
#[cfg(feature = "services-desktop")]
pub mod services_desktop;
#[cfg(feature = "services-i18n")]
pub mod services_i18n;
#[cfg(feature = "services-net")]
pub mod services_net;
#[cfg(feature = "services-webstorage")]
pub mod services_webstorage;
