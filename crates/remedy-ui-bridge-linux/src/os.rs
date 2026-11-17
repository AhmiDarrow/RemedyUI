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
