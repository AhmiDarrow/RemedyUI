//! Which display server a Linux desktop app talks to. winit makes one event loop per
//! process, so the choice is made before it is built: Wayland when its compositor answers,
//! X11 when it does not and an X server is set, and whatever the user names in
//! `REMEDY_UI_DISPLAY_BACKEND` (`wayland` or `x11`) above both.

use std::path::{Path, PathBuf};

/// A display server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Let winit choose (Wayland first, then X11).
    Auto,
    /// X11 (or XWayland).
    X11,
    /// Wayland.
    Wayland,
}

/// What the choice depends on, read from the environment (or given, in tests).
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// `REMEDY_UI_DISPLAY_BACKEND`.
    pub forced: Option<String>,
    /// `WAYLAND_DISPLAY`.
    pub wayland_display: Option<String>,
    /// `XDG_RUNTIME_DIR`.
    pub runtime_dir: Option<PathBuf>,
    /// `DISPLAY`.
    pub x_display: Option<String>,
}

impl Env {
    /// The process environment.
    pub fn current() -> Env {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Env {
            forced: var("REMEDY_UI_DISPLAY_BACKEND"),
            wayland_display: var("WAYLAND_DISPLAY"),
            runtime_dir: var("XDG_RUNTIME_DIR").map(PathBuf::from),
            x_display: var("DISPLAY"),
        }
    }

    /// The Wayland socket a client would connect to (`WAYLAND_DISPLAY`, `wayland-0` by
    /// default, under `XDG_RUNTIME_DIR` unless absolute).
    pub fn wayland_socket(&self) -> Option<PathBuf> {
        let name = self.wayland_display.clone().unwrap_or_else(|| "wayland-0".into());
        let p = Path::new(&name);
        if p.is_absolute() { Some(p.to_path_buf()) } else { self.runtime_dir.as_ref().map(|d| d.join(p)) }
    }
}

/// The backend for `env`, given whether a Wayland compositor answers on its socket.
pub fn choose(env: &Env, wayland_answers: impl Fn(&Path) -> bool) -> Backend {
    match env.forced.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("x11") => return Backend::X11,
        Some("wayland") => return Backend::Wayland,
        _ => {}
    }
    let wayland = env.wayland_socket().is_some_and(|s| wayland_answers(&s));
    if wayland {
        Backend::Auto
    } else if env.x_display.is_some() {
        Backend::X11
    } else {
        // Neither: winit says why.
        Backend::Auto
    }
}

/// Whether something accepts connections on the Unix socket at `path` (a socket file left by a
/// compositor that is gone does not).
#[cfg(unix)]
pub fn socket_answers(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// Unix sockets are a Unix idea.
#[cfg(not(unix))]
pub fn socket_answers(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Backend, Env, choose};

    fn env(wayland: Option<&str>, x: Option<&str>) -> Env {
        Env {
            forced: None,
            wayland_display: wayland.map(str::to_owned),
            runtime_dir: Some(PathBuf::from("/run/user/1000")),
            x_display: x.map(str::to_owned),
        }
    }

    #[test]
    fn wayland_when_it_answers_x11_when_it_does_not() {
        let up = |p: &Path| p == Path::new("/run/user/1000/wayland-0");
        assert_eq!(choose(&env(Some("wayland-0"), Some(":0")), up), Backend::Auto);
        assert_eq!(choose(&env(None, Some(":0")), up), Backend::Auto, "wayland-0 is the default socket");
        assert_eq!(choose(&env(Some("wayland-1"), Some(":0")), up), Backend::X11, "a dead socket falls back");
        assert_eq!(choose(&env(Some("wayland-1"), None), up), Backend::Auto, "no X either: winit reports it");
        let abs = env(Some("/tmp/wl"), Some(":0"));
        assert_eq!(abs.wayland_socket(), Some(PathBuf::from("/tmp/wl")));
    }

    #[test]
    fn the_user_can_name_the_backend() {
        let mut e = env(Some("wayland-0"), Some(":0"));
        e.forced = Some("X11".into());
        assert_eq!(choose(&e, |_| true), Backend::X11);
        e.forced = Some("wayland".into());
        assert_eq!(choose(&e, |_| false), Backend::Wayland);
        e.forced = Some("something else".into());
        assert_eq!(choose(&e, |_| true), Backend::Auto);
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_file_with_no_listener_does_not_answer() {
        let dir = std::env::temp_dir().join(format!("remedy-ui-wl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("wayland-test");
        let listener = std::os::unix::net::UnixListener::bind(&path);
        assert!(listener.is_ok());
        assert!(super::socket_answers(&path));
        drop(listener);
        assert!(!super::socket_answers(&path), "the file is still there, nobody listens");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
