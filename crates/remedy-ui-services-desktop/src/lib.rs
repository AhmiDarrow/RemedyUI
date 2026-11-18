//! RemedyUI desktop services: what an app does with the system beyond its windows.
//!
//! - [`autostart`]: start with the user's session, the platform's way (a Startup-folder
//!   shortcut on Windows, a LaunchAgent on macOS, an XDG autostart entry on Linux)
//! - [`reveal`]: show a file in the file manager, selected
//! - [`known_folder`]: the user's Documents, Pictures, Downloads and other usual folders

pub mod autostart;
mod error;
pub use error::DesktopError;
mod known;
pub use known::{KnownFolder, known_folder, xdg_user_dir};
mod reveal;
pub use reveal::{file_uri, reveal};
#[allow(unsafe_code)]
mod os;
