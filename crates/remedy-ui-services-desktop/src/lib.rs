//! RemedyUI desktop services: what an app does with the system beyond its windows.
//!
//! - [`autostart`]: start with the user's session, the platform's way (a Startup-folder
//!   shortcut on Windows, a LaunchAgent on macOS, an XDG autostart entry on Linux)
//! - [`scheme`]: URL schemes (`hearth://note/42`) that open the app, handed to a running
//!   instance like any later launch's arguments
//! - [`jump`]: tasks on the app's taskbar or launcher menu (Windows' jump list, a desktop
//!   entry's actions) and the system's recent files
//! - [`power`]: battery or mains, sleep and resume, and keeping the system or the screen awake
//! - [`reveal`]: show a file in the file manager, selected
//! - [`known_folder`]: the user's Documents, Pictures, Downloads and other usual folders
//! - [`media_session`]: what plays now in the system's media controls, and their buttons and
//!   the media keys passed back (SMTC on Windows, MPRIS on Linux, `MediaSession` on Android,
//!   `navigator.mediaSession` in a browser)
//!
//! Notifications, the tray and badges need the event loop, so they are part of the host
//! contract (`remedy_ui_host::Host::show_notification`, `set_tray`, `set_badge`) and every
//! bridge, not this crate. What a platform cannot do says [`DesktopError::Unsupported`].

pub mod autostart;
pub use autostart::Entry;
mod error;
pub use error::DesktopError;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod freedesktop;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod ini;
pub mod jump;
mod known;
pub mod media_session;
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod mpris;
pub use known::{KnownFolder, known_folder, xdg_user_dir};
pub mod power;
mod reveal;
pub use reveal::{file_uri, reveal};
#[allow(unsafe_code)]
mod os;
pub mod scheme;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web_media;
