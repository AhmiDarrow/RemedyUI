//! The C library calls the Linux bridge needs.

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
    use wayland_client::Proxy;
    use wayland_client::protocol::wl_surface::WlSurface;
    // SAFETY: `surface` is a live `wl_surface` of the same display (winit's window keeps it
    // alive while the window exists, and the window is alive during this call).
    let id = unsafe {
        wayland_backend::client::ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast())
    }
    .ok()?;
    WlSurface::from_id(conn, id).ok()
}
