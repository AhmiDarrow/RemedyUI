//! What can go wrong reading web storage.

use std::fmt;
use std::path::PathBuf;

/// Why web storage could not be read. Nothing is returned in part: an import either has every
/// item or says why not, so an app never starts over a store it only half read.
#[derive(Debug)]
#[non_exhaustive]
pub enum WebStorageError {
    /// The store's folder does not exist: the old app never ran on this machine with this
    /// profile, or the path is not where its web view keeps its data.
    NotFound {
        /// The folder.
        path: PathBuf,
    },
    /// A file or folder of the store could not be read.
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What the system reported.
        source: std::io::Error,
    },
    /// A file of the store is not what LevelDB writes there (a damaged table, a manifest
    /// naming a table that is gone, an item Chromium could not have written).
    Corrupt {
        /// The file, by name within the store.
        file: String,
        /// What is wrong with it.
        reason: String,
    },
    /// A table block is compressed with a method this reader does not decode. Chromium
    /// writes Snappy or no compression; a store written with anything else is reported
    /// rather than read in part.
    UnsupportedCompression {
        /// The file, by name within the store.
        file: String,
        /// LevelDB's compression type byte.
        kind: u8,
    },
    /// The page has no `localStorage` (a browser with storage turned off, or a context that
    /// has none), or the browser refused access to it.
    Unavailable {
        /// What the browser reported.
        reason: String,
    },
}

impl WebStorageError {
    pub(crate) fn corrupt(file: &str, reason: impl Into<String>) -> Self {
        Self::Corrupt { file: file.to_owned(), reason: reason.into() }
    }
}

impl fmt::Display for WebStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "no web storage at {}", path.display()),
            Self::Io { path, source } => write!(f, "could not read {}: {source}", path.display()),
            Self::Corrupt { file, reason } => write!(f, "web storage file {file} is damaged: {reason}"),
            Self::UnsupportedCompression { file, kind } => {
                write!(f, "web storage file {file} uses compression type {kind}, which is not supported")
            }
            Self::Unavailable { reason } => write!(f, "localStorage is not available: {reason}"),
        }
    }
}

impl std::error::Error for WebStorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
