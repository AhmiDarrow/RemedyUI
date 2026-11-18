//! What can go wrong when an app asks the system for something.

use std::path::PathBuf;

/// Why a desktop service could not do what was asked.
///
/// ```
/// use remedy_ui_services_desktop::{DesktopError, reveal};
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
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesktopError::NotFound { .. } => f.write_str("That file is no longer available."),
            DesktopError::NoAutostartFolder => {
                f.write_str("This system has no place for apps that start with the session.")
            }
            DesktopError::Io { path, reason } => write!(f, "Could not write {}: {reason}", path.display()),
            DesktopError::Platform { reason } => write!(f, "The system could not do it: {reason}"),
        }
    }
}

impl std::error::Error for DesktopError {}
