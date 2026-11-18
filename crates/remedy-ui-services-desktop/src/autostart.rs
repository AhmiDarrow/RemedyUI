//! Starting with the user's session. Each platform's own place, never a registry Run key:
//!
//! - Windows: a shortcut in the Startup folder (`<name>.lnk`)
//! - macOS: a LaunchAgent (`~/Library/LaunchAgents/<id>.plist`, RunAtLoad)
//! - Linux and other freedesktops: an XDG autostart entry (`~/.config/autostart/<id>.desktop`)
//!
//! "Enabled" means the entry is there, so what the user changes in the system's own settings
//! shows here too.

use std::path::{Path, PathBuf};

use crate::DesktopError;

/// What starts, and how it is named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The app's name as people see it ("Hearth"); the Windows shortcut's file name.
    pub name: String,
    /// A reverse-domain id ("com.ahmi.hearth"): the macOS label and the Linux file name.
    pub id: String,
    /// The program.
    pub program: PathBuf,
    /// Its arguments (`--background`).
    pub args: Vec<String>,
}

impl Entry {
    /// This program, with `args`.
    pub fn current(name: &str, id: &str, args: &[&str]) -> std::io::Result<Entry> {
        Ok(Entry {
            name: name.into(),
            id: id.into(),
            program: std::env::current_exe()?,
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        })
    }
}

/// The folder this platform's entries live in.
pub fn folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(|a| PathBuf::from(a).join("Microsoft/Windows/Start Menu/Programs/Startup"))
    } else if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/LaunchAgents"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
            .map(|c| c.join("autostart"))
    }
}

fn file_in(dir: &Path, e: &Entry) -> PathBuf {
    if cfg!(windows) {
        dir.join(format!("{}.lnk", e.name))
    } else if cfg!(target_os = "macos") {
        dir.join(format!("{}.plist", e.id))
    } else {
        dir.join(format!("{}.desktop", e.id))
    }
}

/// True when `e` starts with the session.
pub fn enabled(e: &Entry) -> bool {
    folder().is_some_and(|d| enabled_in(&d, e))
}

/// Starts `e` with the session (`on`), or stops doing so.
pub fn set(e: &Entry, on: bool) -> Result<(), DesktopError> {
    let dir = folder().ok_or(DesktopError::NoAutostartFolder)?;
    set_in(&dir, e, on)
}

/// [`enabled`] for entries in `dir` (tests point it elsewhere).
pub fn enabled_in(dir: &Path, e: &Entry) -> bool {
    file_in(dir, e).exists()
}

/// [`set`] for entries in `dir`.
pub fn set_in(dir: &Path, e: &Entry, on: bool) -> Result<(), DesktopError> {
    let file = file_in(dir, e);
    if !on {
        return match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(DesktopError::Io { path: file, reason: err.to_string() }),
        };
    }
    std::fs::create_dir_all(dir)
        .map_err(|err| DesktopError::Io { path: dir.to_path_buf(), reason: err.to_string() })?;
    let io = |err: std::io::Error| DesktopError::Io { path: file.clone(), reason: err.to_string() };
    if cfg!(windows) {
        crate::os::shortcut(&file, &e.program, &e.args).map_err(|reason| DesktopError::Platform { reason })
    } else if cfg!(target_os = "macos") {
        std::fs::write(&file, launch_agent(e)).map_err(io)
    } else {
        std::fs::write(&file, desktop_entry(e)).map_err(io)
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A LaunchAgent property list that runs `e` at login.
pub fn launch_agent(e: &Entry) -> String {
    let mut args = format!("    <string>{}</string>\n", xml(&e.program.to_string_lossy()));
    for a in &e.args {
        args.push_str(&format!("    <string>{}</string>\n", xml(a)));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{}</string>\n  <key>ProgramArguments</key>\n  <array>\n{args}  </array>\n  <key>RunAtLoad</key>\n  <true/>\n</dict>\n</plist>\n",
        xml(&e.id)
    )
}

/// Quotes an argument for a desktop entry's `Exec` key.
fn exec_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c)) {
        return s.to_owned();
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// An XDG desktop entry that runs `e` when the session starts.
pub fn desktop_entry(e: &Entry) -> String {
    let mut exec = exec_quote(&e.program.to_string_lossy());
    for a in &e.args {
        exec.push(' ');
        exec.push_str(&exec_quote(a));
    }
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={exec}\nX-GNOME-Autostart-enabled=true\nTerminal=false\n",
        e.name
    )
}
