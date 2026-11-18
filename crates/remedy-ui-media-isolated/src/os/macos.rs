//! macOS: unlinked POSIX shared memory objects, the helper's memory watched by the app (the
//! system does not enforce address-space limits), and a deny-by-default Seatbelt profile,
//! the sandbox the App Sandbox is built on.

use std::ffi::{CStr, c_char, c_int};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};

use super::unix::{common_limits, owned_fd};
use crate::Layer;

/// macOS does not enforce RLIMIT_AS; the watchdog watches the helper's memory instead.
pub(crate) const LIMITS_MEMORY: bool = false;

pub(super) fn shared_fd(len: usize) -> io::Result<OwnedFd> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // A name nobody else uses, unlinked at once: only the descriptor names the object.
    // SAFETY: a pure query.
    let pid = unsafe { libc::getpid() };
    let name = format!("/rui.{pid}.{}\0", NEXT.fetch_add(1, Ordering::Relaxed));
    let name = CStr::from_bytes_with_nul(name.as_bytes()).map_err(io::Error::other)?;
    // SAFETY: a NUL-terminated name; O_EXCL makes it ours alone.
    let fd = owned_fd(unsafe {
        libc::shm_open(name.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, 0o600 as libc::c_uint)
    })?;
    // SAFETY: removes the name of the object just made.
    unsafe { libc::shm_unlink(name.as_ptr()) };
    let len = libc::off_t::try_from(len).map_err(|_| io::Error::other("region too large"))?;
    // SAFETY: sizes our own object; macOS allows this once, so it cannot be resized later.
    if unsafe { libc::ftruncate(fd.as_raw_fd(), len) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a descriptor flag on our own descriptor.
    unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    Ok(fd)
}

/// Its resident memory, in bytes.
pub(super) fn memory_of(pid: libc::pid_t) -> Option<u64> {
    // SAFETY: an all-zero proc_taskinfo is valid output space.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_taskinfo>() as c_int;
    // SAFETY: `info` has room for `size` bytes.
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&raw mut info).cast(), size) };
    (n == size).then_some(info.pti_resident_size)
}

/// Its user and system time (counted in Mach time units, converted with the timebase).
pub(super) fn cpu_time_of(pid: libc::pid_t) -> Option<std::time::Duration> {
    // SAFETY: an all-zero proc_taskinfo is valid output space.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_taskinfo>() as c_int;
    // SAFETY: `info` has room for `size` bytes.
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&raw mut info).cast(), size) };
    if n != size {
        return None;
    }
    let mut tb = Timebase { numer: 0, denom: 0 };
    // SAFETY: fills the timebase struct.
    let ok = unsafe { mach_timebase_info(&mut tb) } == 0 && tb.denom != 0;
    if !ok {
        return None;
    }
    let units = u128::from(info.pti_total_user) + u128::from(info.pti_total_system);
    let nanos = units * u128::from(tb.numer) / u128::from(tb.denom);
    Some(std::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)))
}

/// Nothing to set: the watchdog compares [`memory_of`] with the limit.
pub(super) fn limit_memory(_pid: libc::pid_t, _bytes: u64) -> io::Result<()> {
    Ok(())
}

/// `mach_timebase_info_data_t`: Mach time units are `numer / denom` nanoseconds.
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> c_int;
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// Everything denied: no files, network, processes, IPC or services. Reading system values
/// (`sysctl`) is what the C library needs to allocate.
const PROFILE: &CStr = c"(version 1)
(deny default)
(allow sysctl-read)
(allow signal (target self))
";

pub(super) fn lock_down() -> Result<Vec<Layer>, String> {
    common_limits()?;
    let mut err: *mut c_char = std::ptr::null_mut();
    // SAFETY: a NUL-terminated profile; flags 0 means the profile is SBPL text. On failure
    // `err` is a message freed with sandbox_free_error.
    let r = unsafe { sandbox_init(PROFILE.as_ptr(), 0, &mut err) };
    if r != 0 {
        let why = if err.is_null() {
            "unknown error".to_owned()
        } else {
            // SAFETY: a NUL-terminated message from sandbox_init, freed right after.
            let s = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
            // SAFETY: as above.
            unsafe { sandbox_free_error(err) };
            s
        };
        return Err(format!("the sandbox profile was refused: {why}"));
    }
    Ok(vec![Layer::AppSandbox, Layer::ResourceLimits])
}
