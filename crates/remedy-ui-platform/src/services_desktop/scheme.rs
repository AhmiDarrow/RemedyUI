//! URL schemes: links like `hearth://note/42` that open the app.
//!
//! - Windows: a URL protocol under the user's classes
//!   (`HKCU\Software\Classes\<scheme>`, `shell\open\command` running the program with the URL)
//! - Linux and other freedesktops: `x-scheme-handler/<scheme>` in the app's desktop entry
//!   (`~/.local/share/applications/<id>.desktop`, which passes the URL with `%u`) and the
//!   user's default for it in `~/.config/mimeapps.list`
//! - macOS: schemes are part of the app bundle (`CFBundleURLTypes` in its `Info.plist`, which
//!   `cargo remedy package` writes from `remedy.toml`'s `url_schemes`); [`register`] makes the
//!   bundle the default handler of a scheme it declares, and says
//!   [`DesktopError::Unsupported`] for one it does not, or when the app runs outside a bundle
//!
//! Opening a link starts the app with the URL as an argument (Windows, Linux); with
//! single instance, a running app gets it through the same hand-off as any later launch's
//! arguments. macOS hands URLs to the running app itself, and the bridge passes them on the
//! same way. [`urls`] picks them out of the arguments.

use crate::services_desktop::DesktopError;
use crate::services_desktop::autostart::Entry;

/// Schemes the system or the browser own, which an app must not take over.
const RESERVED: &[&str] = &[
    "http",
    "https",
    "file",
    "ftp",
    "data",
    "javascript",
    "about",
    "blob",
    "ws",
    "wss",
    "chrome",
    "ms-settings",
];

/// True for a scheme an app may register: a letter, then letters, digits, `+`, `-` or `.`
/// (RFC 3986), not one the system owns (`http`, `file`).
///
/// ```
/// use remedy_ui_platform::services_desktop::scheme::is_valid;
///
/// assert!(is_valid("hearth"));
/// assert!(is_valid("com.example.hearth"));
/// assert!(!is_valid("https"));
/// assert!(!is_valid("2fa"));
/// assert!(!is_valid("my scheme"));
/// ```
pub fn is_valid(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        && !RESERVED.contains(&scheme.to_ascii_lowercase().as_str())
}

fn check(scheme: &str) -> Result<(), DesktopError> {
    if is_valid(scheme) {
        Ok(())
    } else {
        Err(DesktopError::Invalid {
            value: scheme.to_owned(),
            reason: "a URL scheme is a letter followed by letters, digits, '+', '-' or '.', and not one the system owns",
        })
    }
}

/// Opens links in `scheme` with `app` from now on.
pub fn register(app: &Entry, scheme: &str) -> Result<(), DesktopError> {
    check(scheme)?;
    platform::register(app, scheme)
}

/// Stops opening links in `scheme` with `app` (another app the person chose since is left
/// alone).
pub fn unregister(app: &Entry, scheme: &str) -> Result<(), DesktopError> {
    check(scheme)?;
    platform::unregister(app, scheme)
}

/// True when links in `scheme` open `app`.
pub fn is_registered(app: &Entry, scheme: &str) -> bool {
    is_valid(scheme) && platform::is_registered(app, scheme)
}

/// The launch arguments that are links in one of `schemes` (compared without regard to
/// case), in order.
///
/// ```
/// use remedy_ui_platform::services_desktop::scheme::urls;
///
/// let args = ["--quiet".to_owned(), "Hearth://note/42".to_owned(), "https://example.com".to_owned()];
/// assert_eq!(urls(&args, &["hearth"]), vec!["Hearth://note/42".to_owned()]);
/// ```
pub fn urls(args: &[String], schemes: &[&str]) -> Vec<String> {
    args.iter()
        .filter(|a| {
            a.split_once(':').is_some_and(|(s, _)| schemes.iter().any(|want| want.eq_ignore_ascii_case(s)))
        })
        .cloned()
        .collect()
}

#[cfg(windows)]
mod platform {
    use super::{DesktopError, Entry};

    pub(super) fn register(app: &Entry, scheme: &str) -> Result<(), DesktopError> {
        crate::services_desktop::os::register_scheme(scheme, &app.name, &app.program, &app.args).map_err(DesktopError::platform)
    }

    pub(super) fn unregister(app: &Entry, scheme: &str) -> Result<(), DesktopError> {
        if is_registered(app, scheme) {
            crate::services_desktop::os::unregister_scheme(scheme).map_err(DesktopError::platform)
        } else {
            Ok(())
        }
    }

    pub(super) fn is_registered(app: &Entry, scheme: &str) -> bool {
        crate::services_desktop::os::scheme_command(scheme).is_some_and(|cmd| {
            let program = app.program.to_string_lossy().to_lowercase();
            cmd.to_lowercase().contains(&program)
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{DesktopError, Entry};

    pub(super) fn register(_app: &Entry, scheme: &str) -> Result<(), DesktopError> {
        crate::services_desktop::os::macos::claim_scheme(scheme)
            .map_err(|reason| DesktopError::unsupported("URL schemes", reason))
    }

    pub(super) fn unregister(_app: &Entry, _scheme: &str) -> Result<(), DesktopError> {
        Err(DesktopError::unsupported(
            "URL schemes",
            "a scheme belongs to the app bundle on macOS; take it out of remedy.toml's url_schemes",
        ))
    }

    pub(super) fn is_registered(_app: &Entry, scheme: &str) -> bool {
        crate::services_desktop::os::macos::handles_scheme(scheme)
    }
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod platform {
    use super::{DesktopError, Entry};
    use crate::services_desktop::freedesktop::{self as fd, Xdg};

    pub(super) fn register(app: &Entry, scheme: &str) -> Result<(), DesktopError> {
        let x = Xdg::current()?;
        fd::register_scheme(&x, app, scheme)?;
        refresh(&x);
        Ok(())
    }

    pub(super) fn unregister(app: &Entry, scheme: &str) -> Result<(), DesktopError> {
        let x = Xdg::current()?;
        fd::unregister_scheme(&x, app, scheme)?;
        refresh(&x);
        Ok(())
    }

    pub(super) fn is_registered(app: &Entry, scheme: &str) -> bool {
        Xdg::current().is_ok_and(|x| fd::scheme_registered(&x, app, scheme))
    }

    /// Rebuilds the folder's handler cache where the tool is installed; desktops that read
    /// `mimeapps.list` directly need nothing more.
    fn refresh(x: &Xdg) {
        let _ = std::process::Command::new("update-desktop-database")
            .arg(x.applications())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use super::{DesktopError, Entry};

    pub(super) fn register(_: &Entry, _: &str) -> Result<(), DesktopError> {
        Err(DesktopError::unsupported("URL schemes", "this platform has no URL scheme registry"))
    }

    pub(super) fn unregister(_: &Entry, _: &str) -> Result<(), DesktopError> {
        Err(DesktopError::unsupported("URL schemes", "this platform has no URL scheme registry"))
    }

    pub(super) fn is_registered(_: &Entry, _: &str) -> bool {
        false
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
mod platform {
    use super::{DesktopError, Entry};

    fn no() -> DesktopError {
        DesktopError::unsupported(
            "URL schemes",
            "a mobile app declares its links in its manifest (intent filters, CFBundleURLTypes)",
        )
    }

    pub(super) fn register(_: &Entry, _: &str) -> Result<(), DesktopError> {
        Err(no())
    }

    pub(super) fn unregister(_: &Entry, _: &str) -> Result<(), DesktopError> {
        Err(no())
    }

    pub(super) fn is_registered(_: &Entry, _: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_and_reserved_schemes_are_refused() {
        let app = Entry {
            name: "A".into(),
            id: "a".into(),
            program: "/bin/a".into(),
            args: vec![],
            run_key: false,
        };
        for bad in ["", "http", "HTTPS", "file", "a b", "1up", "x/y", "é"] {
            assert!(matches!(register(&app, bad), Err(DesktopError::Invalid { .. })), "{bad:?}");
            assert!(!is_registered(&app, bad));
        }
        assert!(is_valid("web+hearth"), "browsers' own prefix for app schemes is fine");
    }

    #[test]
    fn only_links_in_the_schemes_are_picked() {
        let args: Vec<String> = ["hearth:", "hearthy://x", "C:\\notes.txt", "hearth://a?b=c", "--hearth"]
            .map(String::from)
            .to_vec();
        assert_eq!(urls(&args, &["hearth"]), vec!["hearth:".to_owned(), "hearth://a?b=c".to_owned()]);
        assert!(urls(&args, &[]).is_empty());
    }
}
