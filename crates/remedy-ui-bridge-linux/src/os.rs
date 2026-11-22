//! The C library calls the Linux bridge needs.

pub(crate) mod gamepad;

/// Seconds east of UTC at `unix_seconds` in the user's time zone (`TZ`, else
/// /etc/localtime), with that instant's daylight-saving rule.
pub(crate) fn utc_offset(unix_seconds: i64) -> Option<i32> {
    // `time_t` is 32 bits on some Linux targets: dates past 2038 do not fit there.
    #[allow(clippy::useless_conversion, reason = "time_t is i32 on 32-bit targets")]
    let t: libc::time_t = unix_seconds.try_into().ok()?;
    // SAFETY: localtime_r reads `t` and writes only the `tm` it is given; it is the
    // thread-safe form, and calls tzset itself.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        tm
    };
    i32::try_from(tm.tm_gmtoff).ok()
}

/// winit's Wayland display as a connection of the bridge's own (its own event queues on the
/// same socket).
pub(crate) fn wayland_connection(display: std::ptr::NonNull<std::ffi::c_void>) -> wayland_client::Connection {
    // SAFETY: `display` is winit's live `wl_display`; winit keeps it open for the life of the
    // event loop, which outlives the hooks that hold this connection.
    let backend = unsafe { wayland_backend::client::Backend::from_foreign_display(display.as_ptr().cast()) };
    wayland_client::Connection::from_backend(backend)
}

/// A surface winit made, as an object on `conn`.
pub(crate) fn wayland_surface(
    conn: &wayland_client::Connection,
    surface: std::ptr::NonNull<std::ffi::c_void>,
) -> Option<wayland_client::protocol::wl_surface::WlSurface> {
    wayland_object(conn, surface)
}

/// An object winit made (its surface, its `xdg_toplevel`), as an object of type `I` on `conn`.
pub(crate) fn wayland_object<I: wayland_client::Proxy>(
    conn: &wayland_client::Connection,
    object: std::ptr::NonNull<std::ffi::c_void>,
) -> Option<I> {
    // SAFETY: `object` is a live object of interface `I` on the same display (winit's window
    // keeps its surface and toplevel alive while the window exists, and the window is alive
    // during this call); `from_ptr` checks the interface.
    let id = unsafe { wayland_backend::client::ObjectId::from_ptr(I::interface(), object.as_ptr().cast()) }
        .ok()?;
    I::from_id(conn, id).ok()
}

/// Anonymous shared memory holding `data`, sealed against any later change (what a
/// compositor maps for a buffer whose contents must not change).
pub(crate) fn sealed_memory(name: &str, data: &[u8]) -> std::io::Result<std::fs::File> {
    use std::io::Write;
    use std::os::fd::FromRawFd;
    let name = std::ffi::CString::new(name).map_err(std::io::Error::other)?;
    // SAFETY: a NUL-terminated name; the result is a new descriptor this function owns.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was just created and is owned by nothing else.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(data)?;
    let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE | libc::F_SEAL_SEAL;
    // SAFETY: fcntl on a descriptor this function owns.
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, seals) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}
