//! The pointing hardware attached, read from `/proc/bus/input/devices` (readable by every
//! user, under X11 and Wayland alike), and a watch on `/dev/input` (inotify) that wakes the
//! event loop when a device comes or goes.

use std::io::Read as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use remedy_ui_core::host::{PointerCaps, Waker};

/// What the pointing hardware attached can do, or `None` where the kernel lists none (or
/// cannot be read: a sandbox).
pub(crate) fn caps() -> Option<PointerCaps> {
    let text = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    let (mouse, touch, pen) = crate::bridge_linux::pointers::kinds(&text, usize::BITS);
    PointerCaps::of_devices(mouse, touch, pen)
}

#[derive(Default)]
struct Shared {
    changed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// Devices arriving and leaving, noted for the event loop.
pub(crate) struct DeviceWatch {
    shared: Arc<Shared>,
}

impl Default for DeviceWatch {
    fn default() -> Self {
        DeviceWatch::start()
    }
}

impl DeviceWatch {
    /// Watches `/dev/input` on a thread of its own; without it (no such directory, no
    /// inotify) devices are read again when the app comes back to the front.
    pub(crate) fn start() -> DeviceWatch {
        let shared = Arc::new(Shared::default());
        if let Some(mut watch) = crate::bridge_linux::os::gamepad::watch(c"/dev/input") {
            let weak: Weak<Shared> = Arc::downgrade(&shared);
            let _ = std::thread::Builder::new().name("remedy-input-devices".into()).spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    // shortcut: the thread waits in poll until the next device event after the
                    // hooks are gone, then ends; a process quitting does not wait for it.
                    let ready = crate::bridge_linux::os::gamepad::poll(&[&watch], None);
                    if !ready.first().copied().unwrap_or(false) {
                        // A signal cut the wait short: wait again. Anything else ends the watch.
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        return;
                    }
                    while matches!(watch.read(&mut buf), Ok(n) if n > 0) {}
                    let Some(shared) = weak.upgrade() else { return };
                    shared.changed.store(true, Ordering::Release);
                    let waker = shared.waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
                    if let Some(w) = waker {
                        w();
                    }
                }
            });
        }
        DeviceWatch { shared }
    }

    /// Wakes the event loop when a device comes or goes.
    pub(crate) fn set_waker(&self, waker: Waker) {
        *self.shared.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// Whether a device came or went since the last call.
    pub(crate) fn take_changed(&self) -> bool {
        self.shared.changed.swap(false, Ordering::AcqRel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hardware_reads_and_the_watch_starts() {
        // Whatever this machine has (a container may list nothing).
        let _ = caps();
        let w = DeviceWatch::start();
        assert!(!w.take_changed());
    }
}
