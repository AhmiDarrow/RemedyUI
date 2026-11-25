//! The pointing hardware attached (`GetSystemMetrics`: a mouse, touch and pen digitizers,
//! tablet posture), and a watch on devices coming and going (`CM_Register_Notification` for
//! HID and mouse interfaces) that wakes the event loop to read it again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use remedy_ui_core::host::{PointerCaps, Waker};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL, CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL,
    CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0,
    CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification, CR_SUCCESS,
    HCMNOTIFICATION,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{GUID_DEVINTERFACE_HID, GUID_DEVINTERFACE_MOUSE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, NID_EXTERNAL_PEN, NID_EXTERNAL_TOUCH, NID_INTEGRATED_PEN, NID_INTEGRATED_TOUCH,
    NID_READY, SM_CONVERTIBLESLATEMODE, SM_DIGITIZER, SM_MAXIMUMTOUCHES, SM_MOUSEPRESENT,
};

/// The caps from the system metrics: `mouse` is `SM_MOUSEPRESENT`, `digitizer` the
/// `SM_DIGITIZER` bits, `touches` `SM_MAXIMUMTOUCHES` and `slate` whether a convertible is in
/// tablet posture (`SM_CONVERTIBLESLATEMODE` is 0), its keyboard and touchpad folded away.
pub(crate) fn from_metrics(mouse: bool, digitizer: u32, touches: i32, slate: bool) -> Option<PointerCaps> {
    let ready = digitizer & NID_READY != 0;
    let touch = ready && touches > 0 && digitizer & (NID_INTEGRATED_TOUCH | NID_EXTERNAL_TOUCH) != 0;
    let pen = ready && digitizer & (NID_INTEGRATED_PEN | NID_EXTERNAL_PEN) != 0;
    PointerCaps::of_devices(mouse && !(slate && touch), touch, pen)
}

/// What the pointing hardware attached can do now.
pub(crate) fn caps() -> Option<PointerCaps> {
    // SAFETY: GetSystemMetrics takes only the metric's index.
    let m = |i| unsafe { GetSystemMetrics(i) };
    from_metrics(
        m(SM_MOUSEPRESENT) != 0,
        m(SM_DIGITIZER) as u32,
        m(SM_MAXIMUMTOUCHES),
        m(SM_CONVERTIBLESLATEMODE) == 0,
    )
}

#[derive(Default)]
struct Shared {
    changed: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// Devices arriving and leaving, noted for the event loop.
pub(crate) struct DeviceWatch {
    shared: Arc<Shared>,
    handles: Vec<HCMNOTIFICATION>,
}

impl Default for DeviceWatch {
    fn default() -> Self {
        DeviceWatch::start()
    }
}

unsafe extern "system" fn on_device(
    _: HCMNOTIFICATION,
    context: *const core::ffi::c_void,
    action: CM_NOTIFY_ACTION,
    _: *const CM_NOTIFY_EVENT_DATA,
    _: u32,
) -> u32 {
    if matches!(action, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL | CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL) {
        // SAFETY: the context is the watch's `Shared`, which outlives its registrations
        // (unregistering waits for callbacks under way).
        let shared = unsafe { &*context.cast::<Shared>() };
        shared.changed.store(true, Ordering::Release);
        let waker = shared.waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(w) = waker {
            w();
        }
    }
    0
}

impl DeviceWatch {
    /// Watches HID and mouse interfaces; without the notifications (they need Windows 8),
    /// devices are read again when the app comes back to the front.
    pub(crate) fn start() -> DeviceWatch {
        let shared = Arc::new(Shared::default());
        let mut handles = Vec::new();
        for guid in [GUID_DEVINTERFACE_HID, GUID_DEVINTERFACE_MOUSE] {
            let filter = CM_NOTIFY_FILTER {
                cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
                Flags: 0,
                FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
                Reserved: 0,
                u: CM_NOTIFY_FILTER_0 { DeviceInterface: CM_NOTIFY_FILTER_0_0 { ClassGuid: guid } },
            };
            let mut handle: HCMNOTIFICATION = std::ptr::null_mut();
            // SAFETY: a filter of the size given; the context lives in `shared`, kept until
            // the registration is undone in `drop`; `handle` is filled in.
            let r = unsafe {
                CM_Register_Notification(&filter, Arc::as_ptr(&shared).cast(), Some(on_device), &mut handle)
            };
            if r == CR_SUCCESS {
                handles.push(handle);
            }
        }
        DeviceWatch { shared, handles }
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

impl Drop for DeviceWatch {
    fn drop(&mut self) {
        for h in self.handles.drain(..) {
            // SAFETY: a registration of ours, undone once; this waits for its callbacks.
            let _ = unsafe { CM_Unregister_Notification(h) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_say_what_the_pointer_can_do() {
        let touch = NID_READY | NID_INTEGRATED_TOUCH;
        assert_eq!(from_metrics(true, 0, 0, false), Some(PointerCaps::MOUSE), "a desktop");
        assert_eq!(from_metrics(true, touch, 10, false), Some(PointerCaps::MOUSE), "a touch laptop");
        assert_eq!(from_metrics(true, touch, 10, true), Some(PointerCaps::TOUCH), "folded into a tablet");
        assert_eq!(from_metrics(false, touch | NID_INTEGRATED_PEN, 10, false), Some(PointerCaps::TOUCH));
        assert_eq!(from_metrics(false, NID_READY | NID_EXTERNAL_PEN, 0, false), Some(PointerCaps::PEN));
        assert_eq!(from_metrics(false, NID_INTEGRATED_TOUCH, 10, false), None, "a digitizer not ready");
        assert_eq!(
            from_metrics(true, 0, 0, true),
            Some(PointerCaps::MOUSE),
            "no touch: slate mode means nothing"
        );
        let _ = caps();
    }

    #[test]
    fn the_watch_starts_and_stops() {
        let w = DeviceWatch::start();
        assert!(!w.take_changed());
        drop(w);
    }
}
