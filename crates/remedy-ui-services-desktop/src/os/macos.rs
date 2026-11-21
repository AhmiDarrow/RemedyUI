//! macOS: IOKit power (sources, assertions, system sleep and wake), the Open Recent list, and
//! the bundle's URL schemes through Launch Services.

use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSDocumentController;
use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSString, NSURL};

use crate::power::{Callback, PowerEvent};

type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: CFTypeRef);
    fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, return_after_source_handled: u8) -> i32;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSGetProvidingPowerSourceType(blob: CFTypeRef) -> CFTypeRef;
    fn IOPSNotificationCreateRunLoopSource(
        callback: extern "C" fn(*mut c_void),
        context: *mut c_void,
    ) -> *mut c_void;
    fn IOPMAssertionCreateWithName(kind: CFTypeRef, level: u32, name: CFTypeRef, id: *mut u32) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
    fn IORegisterForSystemPower(
        refcon: *mut c_void,
        port: *mut *mut c_void,
        callback: extern "C" fn(*mut c_void, u32, u32, *mut c_void),
        notifier: *mut u32,
    ) -> u32;
    fn IODeregisterForSystemPower(notifier: *mut u32) -> i32;
    fn IOServiceClose(connect: u32) -> i32;
    fn IONotificationPortGetRunLoopSource(port: *mut c_void) -> *mut c_void;
    fn IONotificationPortDestroy(port: *mut c_void);
    fn IOAllowPowerChange(kernel_port: u32, notification_id: isize) -> i32;
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSSetDefaultHandlerForURLScheme(scheme: CFTypeRef, bundle_id: CFTypeRef) -> i32;
    fn LSCopyDefaultHandlerForURLScheme(scheme: CFTypeRef) -> CFTypeRef;
}

/// `NSString` is toll-free bridged to `CFString`.
fn cf(s: &NSString) -> CFTypeRef {
    std::ptr::from_ref(s).cast()
}

// --------------------------------------------------------------------------------------------
// Power.

/// The power source the machine runs on now.
pub(crate) fn on_battery() -> Result<bool, String> {
    // SAFETY: the blob is ours (a Copy function) and released; the type string belongs to it
    // and is read while it lives.
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return Err("IOKit reported no power sources".into());
        }
        let kind = IOPSGetProvidingPowerSourceType(blob);
        let out = if kind.is_null() {
            Err("IOKit did not say what powers the machine".into())
        } else {
            let s = &*kind.cast::<NSString>();
            // "AC Power", "Battery Power" or "UPS Power"; a UPS counts as battery.
            Ok(s.to_string() != "AC Power")
        };
        CFRelease(blob);
        out
    }
}

/// A power assertion, released when dropped.
pub(crate) struct Assertion(u32);

pub(crate) fn assert_awake(display: bool, name: &str) -> Result<Assertion, String> {
    let kind = NSString::from_str(if display {
        "PreventUserIdleDisplaySleep"
    } else {
        "PreventUserIdleSystemSleep"
    });
    let name = NSString::from_str(name);
    let mut id = 0u32;
    // SAFETY: two CFStrings that outlive the call; level 255 is kIOPMAssertionLevelOn.
    let r = unsafe { IOPMAssertionCreateWithName(cf(&kind), 255, cf(&name), &mut id) };
    if r == 0 { Ok(Assertion(id)) } else { Err(format!("IOKit refused the power assertion ({r:#x})")) }
}

impl Drop for Assertion {
    fn drop(&mut self) {
        // SAFETY: the assertion made above, released once.
        unsafe { IOPMAssertionRelease(self.0) };
    }
}

const CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
const SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
const SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;

struct Context {
    f: Callback,
    root: AtomicU32,
    on_battery: Mutex<Option<bool>>,
}

extern "C" fn system_power(refcon: *mut c_void, _service: u32, message: u32, argument: *mut c_void) {
    // SAFETY: the context the watch registered, alive until its thread has deregistered.
    let c = unsafe { &*refcon.cast::<Context>() };
    let root = c.root.load(Ordering::SeqCst);
    match message {
        // Idle sleep may go ahead: the app has no say here (that is what assertions are for).
        // SAFETY: the root port this registration returned, and the message's own id.
        CAN_SYSTEM_SLEEP => unsafe {
            IOAllowPowerChange(root, argument as isize);
        },
        SYSTEM_WILL_SLEEP => {
            (c.f)(PowerEvent::Sleeping);
            // SAFETY: as above; the system waits for this (up to 30 seconds).
            unsafe { IOAllowPowerChange(root, argument as isize) };
        }
        SYSTEM_HAS_POWERED_ON => (c.f)(PowerEvent::Resumed),
        _ => {}
    }
}

extern "C" fn power_source(context: *mut c_void) {
    // SAFETY: as in `system_power`.
    let c = unsafe { &*context.cast::<Context>() };
    let Ok(now) = on_battery() else { return };
    let mut last = c.on_battery.lock().unwrap_or_else(PoisonError::into_inner);
    if *last != Some(now) {
        *last = Some(now);
        drop(last);
        (c.f)(PowerEvent::OnBattery(now));
    }
}

/// A thread with its own run loop hearing IOKit; stopped and joined when dropped.
pub(crate) struct PowerWatch {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) fn watch_power(f: Callback) -> Result<PowerWatch, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let s = stop.clone();
    let thread = std::thread::Builder::new()
        .name("remedy power".into())
        .spawn(move || {
            let context = Box::into_raw(Box::new(Context {
                f,
                root: AtomicU32::new(0),
                on_battery: Mutex::new(on_battery().ok()),
            }));
            let mut port: *mut c_void = std::ptr::null_mut();
            let mut notifier = 0u32;
            // SAFETY: registers for system power messages with the context above; everything
            // registered here is undone below on this same thread, before the context is freed.
            unsafe {
                let root = IORegisterForSystemPower(context.cast(), &mut port, system_power, &mut notifier);
                if root == 0 {
                    let _ = ready_tx.send(Err("IOKit refused system power notifications".into()));
                    drop(Box::from_raw(context));
                    return;
                }
                (*context).root.store(root, Ordering::SeqCst);
                let rl = CFRunLoopGetCurrent();
                CFRunLoopAddSource(rl, IONotificationPortGetRunLoopSource(port), kCFRunLoopDefaultMode);
                let source = IOPSNotificationCreateRunLoopSource(power_source, context.cast());
                if !source.is_null() {
                    CFRunLoopAddSource(rl, source, kCFRunLoopDefaultMode);
                }
                let _ = ready_tx.send(Ok(()));
                while !s.load(Ordering::SeqCst) {
                    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, 0);
                }
                if !source.is_null() {
                    CFRelease(source.cast_const());
                }
                IODeregisterForSystemPower(&mut notifier);
                IOServiceClose(root);
                IONotificationPortDestroy(port);
                drop(Box::from_raw(context));
            }
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(PowerWatch { stop, thread: Some(thread) }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err("the power thread stopped".into()),
    }
}

impl Drop for PowerWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// --------------------------------------------------------------------------------------------
// Recent files.

fn documents() -> Result<Retained<NSDocumentController>, String> {
    let mtm = MainThreadMarker::new().ok_or("the Open Recent list is kept on the main thread")?;
    Ok(NSDocumentController::sharedDocumentController(mtm))
}

pub(crate) fn add_recent(file: &Path) -> Result<(), String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    documents()?.noteNewRecentDocumentURL(&url);
    Ok(())
}

pub(crate) fn clear_recent() -> Result<(), String> {
    let d = documents()?;
    // SAFETY: the sender may be nil (it is only passed to the action's target).
    unsafe { d.clearRecentDocuments(None) };
    Ok(())
}

// --------------------------------------------------------------------------------------------
// URL schemes.

/// The schemes the bundle's `Info.plist` declares (`CFBundleURLTypes` → `CFBundleURLSchemes`).
fn declared() -> Vec<String> {
    let bundle = NSBundle::mainBundle();
    let Some(types) = bundle.objectForInfoDictionaryKey(&NSString::from_str("CFBundleURLTypes")) else {
        return Vec::new();
    };
    let Ok(types) = types.downcast::<NSArray>() else { return Vec::new() };
    let key = NSString::from_str("CFBundleURLSchemes");
    let mut out = Vec::new();
    for t in types.iter() {
        let Ok(t) = t.downcast::<NSDictionary>() else { continue };
        let key: &AnyObject = &key;
        let Some(schemes) = t.objectForKey(key) else { continue };
        let Ok(schemes) = schemes.downcast::<NSArray>() else { continue };
        for s in schemes.iter() {
            if let Ok(s) = s.downcast::<NSString>() {
                out.push(s.to_string().to_ascii_lowercase());
            }
        }
    }
    out
}

fn bundle_id() -> Result<Retained<NSString>, String> {
    NSBundle::mainBundle().bundleIdentifier().ok_or_else(|| {
        "the app runs outside an app bundle; package it with `cargo remedy package`".to_owned()
    })
}

pub(crate) fn claim_scheme(scheme: &str) -> Result<(), String> {
    let id = bundle_id()?;
    if !declared().contains(&scheme.to_ascii_lowercase()) {
        return Err(format!(
            "the app bundle does not declare \"{scheme}\" (add it to remedy.toml's url_schemes and package again)"
        ));
    }
    let s = NSString::from_str(scheme);
    // SAFETY: two CFStrings that outlive the call.
    let r = unsafe { LSSetDefaultHandlerForURLScheme(cf(&s), cf(&id)) };
    if r == 0 { Ok(()) } else { Err(format!("Launch Services refused ({r})")) }
}

pub(crate) fn handles_scheme(scheme: &str) -> bool {
    let Ok(id) = bundle_id() else { return false };
    let s = NSString::from_str(scheme);
    // SAFETY: a Copy function's result, read and released.
    unsafe {
        let handler = LSCopyDefaultHandlerForURLScheme(cf(&s));
        if handler.is_null() {
            return false;
        }
        let same = (*handler.cast::<NSString>()).to_string().eq_ignore_ascii_case(&id.to_string());
        CFRelease(handler);
        same
    }
}
