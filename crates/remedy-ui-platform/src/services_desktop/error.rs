//! What can go wrong when an app asks the system for something.

use std::path::PathBuf;

/// Why a desktop service could not do what was asked.
///
/// ```
/// use remedy_ui_platform::services_desktop::{DesktopError, reveal};
///
/// let missing = std::path::Path::new("/no/such/place/at-all.txt");
/// assert!(matches!(reveal(missing), Err(DesktopError::NotFound { .. })));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DesktopError {
    /// The file or folder is not there (any more).
    NotFound {
        /// What was asked for.
        path: PathBuf,
    },
    /// This system has no place for apps that start with the session.
    NoAutostartFolder,
    /// Reading or writing a file failed.
    Io {
        /// The file or folder.
        path: PathBuf,
        /// The system's own words.
        reason: String,
    },
    /// The platform's shell or file manager refused or failed.
    Platform {
        /// The platform's own words.
        reason: String,
    },
    /// This platform (or this system) cannot do it at all; never faked.
    Unsupported {
        /// What was asked for ("URL schemes").
        what: &'static str,
        /// Why not, and what to do instead where there is something.
        reason: String,
    },
    /// The name is not one the system accepts (a URL scheme with a space in it).
    Invalid {
        /// What was given.
        value: String,
        /// What it must be.
        reason: &'static str,
    },
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesktopError::NotFound { .. } => f.write_str("That file is no longer available."),
            DesktopError::NoAutostartFolder => {
                f.write_str("This system has no place for apps that start with the session.")
            }
            DesktopError::Io { path, reason } => {
                write!(f, "Could not read or write {}: {reason}", path.display())
            }
            DesktopError::Platform { reason } => write!(f, "The system could not do it: {reason}"),
            DesktopError::Unsupported { what, reason } => {
                write!(f, "Not available here: {what} ({reason})")
            }
            DesktopError::Invalid { value, reason } => write!(f, "\"{value}\" cannot be used: {reason}"),
        }
    }
}

impl std::error::Error for DesktopError {}

impl DesktopError {
    pub(crate) fn unsupported(what: &'static str, reason: impl Into<String>) -> DesktopError {
        DesktopError::Unsupported { what, reason: reason.into() }
    }

    pub(crate) fn io(path: &std::path::Path, err: &std::io::Error) -> DesktopError {
        DesktopError::Io { path: path.to_path_buf(), reason: err.to_string() }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn platform(reason: impl std::fmt::Display) -> DesktopError {
        DesktopError::Platform { reason: reason.to_string() }
    }
}
