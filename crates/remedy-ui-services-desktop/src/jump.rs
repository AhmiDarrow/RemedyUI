//! What the app's taskbar button, Dock icon or launcher offers beyond starting it: tasks
//! (Windows' jump list, a Linux desktop entry's actions) and recent files.
//!
//! - Windows: tasks are the jump list's Tasks (`ICustomDestinationList`), each a shortcut to
//!   the program with its arguments; recent files go through `SHAddToRecentDocs`, which feeds
//!   the jump list's Recent category and Explorer's Quick access
//! - Linux and other freedesktops: tasks are `[Desktop Action]`s in the app's desktop entry,
//!   which GNOME, KDE, Cinnamon and the docks offer on right-click; recent files are added to
//!   `recently-used.xbel`, which GTK and KDE file dialogs and the Activities search read
//! - macOS: recent files are the app's Open Recent list (`NSDocumentController`), which the
//!   Dock icon's menu also shows; tasks come from the app delegate's Dock menu, which the
//!   bridge's windowing library owns, so [`set_tasks`] says [`DesktopError::Unsupported`]

use std::path::{Path, PathBuf};

use crate::DesktopError;
use crate::autostart::Entry;

/// Something the app's jump list or launcher menu starts it to do.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Task {
    /// What the menu says ("New note").
    pub label: String,
    /// The program's arguments for it (`--new`). With single instance, a running app gets
    /// them like any later launch's.
    pub args: Vec<String>,
    /// A longer description, where the platform shows one (Windows' tooltip).
    pub description: String,
    /// An icon file (`.ico` on Windows), where the platform shows one; the program's own
    /// icon when unset.
    pub icon: Option<PathBuf>,
}

impl Task {
    /// A task labeled `label` that starts the program with `args`.
    pub fn new(label: impl Into<String>, args: &[&str]) -> Task {
        Task {
            label: label.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            description: String::new(),
            icon: None,
        }
    }

    /// Sets the description.
    #[must_use]
    pub fn description(mut self, text: impl Into<String>) -> Task {
        self.description = text.into();
        self
    }

    /// Sets the icon file.
    #[must_use]
    pub fn icon(mut self, file: impl Into<PathBuf>) -> Task {
        self.icon = Some(file.into());
        self
    }
}

/// Replaces the app's tasks with `tasks` (empty takes them all away).
pub fn set_tasks(app: &Entry, tasks: &[Task]) -> Result<(), DesktopError> {
    platform::set_tasks(app, tasks)
}

/// Adds `file` to the system's recent files as opened by the app.
pub fn add_recent(app: &Entry, file: &Path) -> Result<(), DesktopError> {
    if !file.exists() {
        return Err(DesktopError::NotFound { path: file.to_path_buf() });
    }
    let file = std::fs::canonicalize(file).map_err(|e| DesktopError::io(file, &e))?;
    platform::add_recent(app, &file)
}

/// Takes the app's files out of the recent files (other apps' stay).
pub fn clear_recent(app: &Entry) -> Result<(), DesktopError> {
    platform::clear_recent(app)
}

#[cfg(windows)]
mod platform {
    use super::{DesktopError, Entry, Path, Task};

    pub(super) fn set_tasks(app: &Entry, tasks: &[Task]) -> Result<(), DesktopError> {
        crate::os::set_jump_tasks(&app.program, tasks).map_err(DesktopError::platform)
    }

    pub(super) fn add_recent(_app: &Entry, file: &Path) -> Result<(), DesktopError> {
        crate::os::add_recent(&crate::reveal::strip_verbatim(file));
        Ok(())
    }

    pub(super) fn clear_recent(_app: &Entry) -> Result<(), DesktopError> {
        crate::os::clear_recent().map_err(DesktopError::platform)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{DesktopError, Entry, Path, Task};

    pub(super) fn set_tasks(_app: &Entry, _tasks: &[Task]) -> Result<(), DesktopError> {
        Err(DesktopError::unsupported(
            "Dock menu tasks",
            "a Dock menu comes from the app delegate, which the windowing library owns on macOS",
        ))
    }

    pub(super) fn add_recent(_app: &Entry, file: &Path) -> Result<(), DesktopError> {
        crate::os::macos::add_recent(file).map_err(|reason| DesktopError::unsupported("recent files", reason))
    }

    pub(super) fn clear_recent(_app: &Entry) -> Result<(), DesktopError> {
        crate::os::macos::clear_recent().map_err(|reason| DesktopError::unsupported("recent files", reason))
    }
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod platform {
    use super::{DesktopError, Entry, Path, Task};
    use crate::freedesktop::{self as fd, Xdg};

    pub(super) fn set_tasks(app: &Entry, tasks: &[Task]) -> Result<(), DesktopError> {
        fd::set_tasks(&Xdg::current()?, app, tasks)
    }

    pub(super) fn add_recent(app: &Entry, file: &Path) -> Result<(), DesktopError> {
        fd::add_recent(&Xdg::current()?, app, file, std::time::SystemTime::now())
    }

    pub(super) fn clear_recent(app: &Entry) -> Result<(), DesktopError> {
        fd::clear_recent(&Xdg::current()?, app)
    }
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    all(unix, not(any(target_os = "ios", target_os = "android")))
)))]
mod platform {
    use super::{DesktopError, Entry, Path, Task};

    fn no() -> DesktopError {
        DesktopError::unsupported("jump lists and recent files", "this platform has no app launcher menu")
    }

    pub(super) fn set_tasks(_: &Entry, _: &[Task]) -> Result<(), DesktopError> {
        Err(no())
    }

    pub(super) fn add_recent(_: &Entry, _: &Path) -> Result<(), DesktopError> {
        Err(no())
    }

    pub(super) fn clear_recent(_: &Entry) -> Result<(), DesktopError> {
        Err(no())
    }
}
