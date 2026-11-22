//! evdev's ioctls, inotify and poll, for reading gamepads under /dev/input.

use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Duration;

/// `_IOC(_IOC_READ, 'E', nr, size)`: the request number of an evdev ioctl that reads `size`
/// bytes. The direction bits sit higher on the architectures with a 13-bit size field.
fn eviocg(nr: u32, size: usize) -> u64 {
    let read: u64 = if cfg!(any(
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "sparc",
        target_arch = "sparc64"
    )) {
        2 << 29
    } else {
        2 << 30
    };
    read | ((size as u64) << 16) | (u64::from(b'E') << 8) | u64::from(nr)
}

/// Fills `buf` with an evdev ioctl's answer; false when the device refused.
fn ioctl_read(dev: &File, nr: u32, buf: &mut [u8]) -> bool {
    // SAFETY: the request reads at most `buf.len()` bytes (its size field says so) into `buf`,
    // which is that long, on an open descriptor.
    #[allow(clippy::useless_conversion, reason = "the request is a c_int on musl, a c_ulong on glibc")]
    let r = unsafe { libc::ioctl(dev.as_raw_fd(), eviocg(nr, buf.len()) as _, buf.as_mut_ptr()) };
    r >= 0
}

/// The device's key bitmask (`EVIOCGBIT(EV_KEY)`), or its keys held now (`EVIOCGKEY`).
pub(crate) fn key_bits(dev: &File, held: bool) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; crate::evdev::KEY_BYTES];
    let nr = if held { 0x18 } else { 0x20 + u32::from(crate::evdev::EV_KEY) };
    ioctl_read(dev, nr, &mut buf).then_some(buf)
}

/// An axis's value, minimum and maximum (`EVIOCGABS`), when the device has it.
pub(crate) fn abs_info(dev: &File, axis: u16) -> Option<(i32, i32, i32)> {
    let mut info = [0u8; std::mem::size_of::<libc::input_absinfo>()];
    if !ioctl_read(dev, 0x40 + u32::from(axis), &mut info) {
        return None;
    }
    let int = |i: usize| info.get(i * 4..i * 4 + 4).and_then(|b| b.try_into().ok()).map(i32::from_ne_bytes);
    Some((int(0)?, int(1)?, int(2)?))
}

/// The size of one `input_event` as this target lays it out (its time is 8 or 16 bytes).
pub(crate) const EVENT_SIZE: usize = std::mem::size_of::<libc::input_event>();

/// An inotify descriptor watching `dir` for device nodes coming, going and changing
/// permission (udev grants access just after it makes a node).
pub(crate) fn watch(dir: &std::ffi::CStr) -> Option<File> {
    // SAFETY: a new descriptor, owned by the File below.
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` is ours and open; nothing else owns it.
    let file = unsafe { File::from_raw_fd(fd) };
    let mask = libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB;
    // SAFETY: `dir` is NUL-terminated, and the watch lives as long as the descriptor.
    let w = unsafe { libc::inotify_add_watch(file.as_raw_fd(), dir.as_ptr(), mask) };
    (w >= 0).then_some(file)
}

/// Waits until one of `files` can be read or `timeout` passes (`None`: as long as it takes),
/// and says which can. Each comes back false when the wait failed.
pub(crate) fn poll(files: &[&File], timeout: Option<Duration>) -> Vec<bool> {
    let mut fds: Vec<libc::pollfd> =
        files.iter().map(|f| libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
    let ms = timeout.map_or(-1, |t| i32::try_from(t.as_millis().max(1)).unwrap_or(i32::MAX));
    // SAFETY: `fds` holds `fds.len()` valid pollfd entries for poll to fill in.
    let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
    // A device that went away reads as an error, which the read then reports.
    fds.iter().map(|p| r > 0 && p.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0).collect()
}
