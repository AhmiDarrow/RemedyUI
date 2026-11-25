//! Window manager hints winit does not offer on X11, set on a new window before it is first
//! mapped: `_NET_WM_USER_TIME` 0 (do not focus it when it appears) and `_NET_WM_STATE`
//! `_NET_WM_STATE_SKIP_TASKBAR` and `_NET_WM_STATE_SKIP_PAGER` (no taskbar button, not in
//! the window switcher). Window managers that follow EWMH honor them. A connection of its own.

use remedy_ui_core::host::Unsupported;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

fn fail(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("window manager hints", e.to_string())
}

fn intern(c: &RustConnection, name: &str) -> Result<u32, Unsupported> {
    Ok(c.intern_atom(false, name.as_bytes()).map_err(fail)?.reply().map_err(fail)?.atom)
}

/// Asks the window manager not to focus window `id` when it appears (`activate` false) and to
/// keep it off the taskbar and the switcher (`taskbar` false).
pub(crate) fn quiet(id: u32, activate: bool, taskbar: bool) -> Result<(), Unsupported> {
    let (c, _) = RustConnection::connect(None).map_err(fail)?;
    if !activate {
        let user_time = intern(&c, "_NET_WM_USER_TIME")?;
        c.change_property32(PropMode::REPLACE, id, user_time, AtomEnum::CARDINAL, &[0]).map_err(fail)?;
    }
    if !taskbar {
        let state = intern(&c, "_NET_WM_STATE")?;
        let skip = [intern(&c, "_NET_WM_STATE_SKIP_TASKBAR")?, intern(&c, "_NET_WM_STATE_SKIP_PAGER")?];
        c.change_property32(PropMode::APPEND, id, state, AtomEnum::ATOM, &skip).map_err(fail)?;
    }
    c.sync().map_err(fail)?;
    Ok(())
}
