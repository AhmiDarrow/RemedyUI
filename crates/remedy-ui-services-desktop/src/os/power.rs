//! Power on Windows: the power status, power requests, and suspend, resume and power-source
//! notifications delivered to a callback (no window needed).

use std::ffi::c_void;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, GetSystemPowerStatus, HPOWERNOTIFY, POWERBROADCAST_SETTING,
    PowerClearRequest, PowerCreateRequest, PowerRegisterSuspendResumeNotification,
    PowerRequestDisplayRequired, PowerRequestSystemRequired, PowerSetRequest,
    PowerSettingRegisterNotification, PowerSettingUnregisterNotification,
    PowerUnregisterSuspendResumeNotification, SYSTEM_POWER_STATUS,
};
use windows_sys::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, POWER_REQUEST_CONTEXT_VERSION};
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
};
use windows_sys::core::GUID;

use crate::power::{Callback, PowerEvent};

/// True on battery (`ACLineStatus` 0), false on mains (1).
pub(crate) fn on_battery() -> Result<bool, String> {
    let mut s = SYSTEM_POWER_STATUS {
        ACLineStatus: 0,
        BatteryFlag: 0,
        BatteryLifePercent: 0,
        SystemStatusFlag: 0,
        BatteryLifeTime: 0,
        BatteryFullLifeTime: 0,
    };
    // SAFETY: fills the structure it is given.
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return Err("Windows did not report the power status".into());
    }
    match s.ACLineStatus {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err("Windows does not know the power source".into()),
    }
}

/// A power request: the system (and the display) stay on until it is dropped.
pub(crate) struct Request {
    handle: HANDLE,
    display: bool,
}

// SAFETY: a power request handle may be cleared and closed from any thread.
unsafe impl Send for Request {}
// SAFETY: the handle is only read.
unsafe impl Sync for Request {}

pub(crate) fn request(display: bool, reason: &str) -> Result<Request, String> {
    let mut text: Vec<u16> = reason.encode_utf16().chain(std::iter::once(0)).collect();
    let context = REASON_CONTEXT {
        Version: POWER_REQUEST_CONTEXT_VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 { SimpleReasonString: text.as_mut_ptr() },
    };
    // SAFETY: the reason string is NUL-terminated and outlives the call (Windows copies it).
    let handle = unsafe { PowerCreateRequest(&context) };
    if handle.is_null() || handle as isize == -1 {
        return Err("Windows refused the power request".into());
    }
    let r = Request { handle, display };
    // SAFETY: a valid power request handle.
    if unsafe { PowerSetRequest(r.handle, PowerRequestSystemRequired) } == 0 {
        return Err("Windows refused to keep the system awake".into());
    }
    // SAFETY: as above.
    if display && unsafe { PowerSetRequest(r.handle, PowerRequestDisplayRequired) } == 0 {
        return Err("Windows refused to keep the screen on".into());
    }
    Ok(r)
}

impl Drop for Request {
    fn drop(&mut self) {
        // SAFETY: the request created above, cleared and closed once.
        unsafe {
            if self.display {
                PowerClearRequest(self.handle, PowerRequestDisplayRequired);
            }
            PowerClearRequest(self.handle, PowerRequestSystemRequired);
            CloseHandle(self.handle);
        }
    }
}

/// What the callbacks reach: the app's callback and the last power source reported.
struct Context {
    f: Callback,
    on_battery: Mutex<Option<bool>>,
}

/// The registrations; dropping it unregisters both before the context is freed.
pub(crate) struct Watch {
    context: *mut Context,
    suspend: *mut c_void,
    source: *mut c_void,
}

// SAFETY: unregistering is allowed from any thread; the context is only read by callbacks.
unsafe impl Send for Watch {}
// SAFETY: as above.
unsafe impl Sync for Watch {}

fn same(a: &GUID, b: &GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}

unsafe extern "system" fn notified(context: *const c_void, kind: u32, setting: *const c_void) -> u32 {
    // SAFETY: the context is the `Context` the watch registered, alive until both
    // registrations are gone.
    let c = unsafe { &*context.cast::<Context>() };
    match kind {
        PBT_APMSUSPEND => (c.f)(PowerEvent::Sleeping),
        PBT_APMRESUMEAUTOMATIC => (c.f)(PowerEvent::Resumed),
        PBT_POWERSETTINGCHANGE if !setting.is_null() => {
            // SAFETY: for a power-setting change, `setting` points to a POWERBROADCAST_SETTING
            // whose data is the setting's DWORD.
            let s = unsafe { &*setting.cast::<POWERBROADCAST_SETTING>() };
            if same(&s.PowerSetting, &GUID_ACDC_POWER_SOURCE) && s.DataLength >= 4 {
                // SAFETY: at least four bytes of data, read unaligned.
                let source = unsafe { std::ptr::read_unaligned(s.Data.as_ptr().cast::<u32>()) };
                // 0 is mains; 1 battery; 2 a short-term source (a UPS), counted as battery.
                let battery = source != 0;
                let mut last = c.on_battery.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if *last != Some(battery) {
                    // Registering reports the current source first: not a change.
                    let first = last.is_none();
                    *last = Some(battery);
                    drop(last);
                    if !first {
                        (c.f)(PowerEvent::OnBattery(battery));
                    }
                }
            }
        }
        _ => {}
    }
    0
}

pub(crate) fn watch(f: Callback) -> Result<Watch, String> {
    let context = Box::into_raw(Box::new(Context { f, on_battery: Mutex::new(on_battery().ok()) }));
    let mut params = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS { Callback: Some(notified), Context: context.cast() };
    let mut w = Watch { context, suspend: std::ptr::null_mut(), source: std::ptr::null_mut() };
    let recipient: HANDLE = (&raw mut params).cast();
    // SAFETY: registers the callback with the parameters above (Windows copies them); the
    // context stays alive until `Watch::drop` unregisters.
    let r =
        unsafe { PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK, recipient, &mut w.suspend) };
    if r != 0 {
        return Err(format!("Windows refused suspend and resume notifications (error {r})"));
    }
    // SAFETY: as above, for the power-source setting.
    let r = unsafe {
        PowerSettingRegisterNotification(
            &GUID_ACDC_POWER_SOURCE,
            DEVICE_NOTIFY_CALLBACK,
            recipient,
            &mut w.source,
        )
    };
    if r != 0 {
        return Err(format!("Windows refused power source notifications (error {r})"));
    }
    Ok(w)
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: the registrations made in `watch`, each undone once; after both, no callback
        // can run, and the context is freed.
        unsafe {
            if !self.suspend.is_null() {
                PowerUnregisterSuspendResumeNotification(self.suspend as HPOWERNOTIFY);
            }
            if !self.source.is_null() {
                PowerSettingUnregisterNotification(self.source as HPOWERNOTIFY);
            }
            drop(Box::from_raw(self.context));
        }
    }
}
