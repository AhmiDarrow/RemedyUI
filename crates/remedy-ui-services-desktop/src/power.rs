//! Power: whether the machine runs on battery, when it goes to sleep and wakes, and keeping
//! it (or its screen) awake while the app works.
//!
//! - Windows: `GetSystemPowerStatus`; suspend, resume and power-source notifications
//!   registered with a callback (`PowerRegisterSuspendResumeNotification`,
//!   `PowerSettingRegisterNotification`), so no window is needed; power requests
//!   (`PowerCreateRequest`) to stay awake, which `powercfg /requests` lists with the reason
//! - macOS: IOKit's power sources (`IOPSCopyPowerSourcesInfo`) and system power
//!   notifications (`IORegisterForSystemPower`) on a run loop of their own; power assertions
//!   (`IOPMAssertionCreateWithName`) to stay awake, which Activity Monitor shows
//! - Linux: UPower's `OnBattery` and logind's `PrepareForSleep` on the system bus; a logind
//!   inhibitor lock to keep the system awake, and the session's screen saver
//!   (`org.freedesktop.ScreenSaver`, which GNOME, KDE and the others provide) to keep the screen
//!   on
//!
//! - Android: the activity's `FLAG_KEEP_SCREEN_ON` to keep the screen on (through the Android
//!   bridge's Java, which the APK carries); no power state
//! - A browser: the Screen Wake Lock (`navigator.wakeLock`) to keep the screen on, while the
//!   page shows; no power state
//!
//! Where a system has none of these (a container without UPower or logind), the calls say
//! [`DesktopError::Unsupported`].

use crate::DesktopError;

/// A change in the machine's power.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PowerEvent {
    /// The machine is about to sleep: save what must not be lost, close connections.
    Sleeping,
    /// The machine woke: reconnect, refresh what went stale.
    Resumed,
    /// It changed to running on battery (true) or on mains power (false): an app may do less
    /// background work on battery.
    OnBattery(bool),
}

/// What [`inhibit`] keeps awake.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Inhibit {
    /// The system does not sleep while the app works (a download, a backup, a call); the
    /// screen may still turn off.
    Sleep,
    /// The screen stays on too (a video, a presentation, a recipe being followed).
    Display,
}

/// True when the machine runs on battery now; false on mains power (or a desktop with no
/// battery).
pub fn on_battery() -> Result<bool, DesktopError> {
    platform::on_battery()
}

/// Calls `f` from a thread of its own whenever the machine sleeps, wakes, or changes power
/// source, until the returned watch is dropped. (The `remedy-ui` facade's `App::on_power`
/// brings these to the UI thread.)
pub fn watch(f: impl Fn(PowerEvent) + Send + Sync + 'static) -> Result<PowerWatch, DesktopError> {
    platform::watch(Box::new(f)).map(|inner| PowerWatch { _inner: inner })
}

/// Keeps the system (and for [`Inhibit::Display`], the screen) awake until the returned
/// guard is dropped. `app` is the app's name and `reason` says why in words the person
/// understands ("Downloading 3 files"); the system shows both where it lists what keeps it
/// awake. The system can still sleep when the person asks it to, or the battery runs out.
pub fn inhibit(kind: Inhibit, app: &str, reason: &str) -> Result<Inhibitor, DesktopError> {
    platform::inhibit(kind, app, reason).map(|inner| Inhibitor { kind, _inner: inner })
}

pub(crate) type Callback = Box<dyn Fn(PowerEvent) + Send + Sync>;

/// Reports power changes while it lives (see [`watch`]).
#[must_use = "the watch stops when dropped"]
pub struct PowerWatch {
    _inner: platform::Watch,
}

impl std::fmt::Debug for PowerWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PowerWatch")
    }
}

/// Keeps the machine awake while it lives (see [`inhibit`]).
#[must_use = "the machine may sleep again once this is dropped"]
pub struct Inhibitor {
    kind: Inhibit,
    _inner: platform::Hold,
}

impl Inhibitor {
    /// What it keeps awake.
    pub fn kind(&self) -> Inhibit {
        self.kind
    }
}

impl std::fmt::Debug for Inhibitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Inhibitor({:?})", self.kind)
    }
}

#[cfg(windows)]
mod platform {
    use super::{Callback, DesktopError, Inhibit};

    pub(super) type Watch = crate::os::power::Watch;
    pub(super) type Hold = crate::os::power::Request;

    pub(super) fn on_battery() -> Result<bool, DesktopError> {
        crate::os::power::on_battery().map_err(|r| DesktopError::unsupported("power status", r))
    }

    pub(super) fn watch(f: Callback) -> Result<Watch, DesktopError> {
        crate::os::power::watch(f).map_err(DesktopError::platform)
    }

    pub(super) fn inhibit(kind: Inhibit, _app: &str, reason: &str) -> Result<Hold, DesktopError> {
        crate::os::power::request(kind == Inhibit::Display, reason).map_err(DesktopError::platform)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Callback, DesktopError, Inhibit};

    pub(super) type Watch = crate::os::macos::PowerWatch;
    pub(super) type Hold = crate::os::macos::Assertion;

    pub(super) fn on_battery() -> Result<bool, DesktopError> {
        crate::os::macos::on_battery().map_err(|r| DesktopError::unsupported("power status", r))
    }

    pub(super) fn watch(f: Callback) -> Result<Watch, DesktopError> {
        crate::os::macos::watch_power(f).map_err(DesktopError::platform)
    }

    pub(super) fn inhibit(kind: Inhibit, app: &str, reason: &str) -> Result<Hold, DesktopError> {
        let what = if reason.is_empty() { app.to_owned() } else { format!("{app}: {reason}") };
        crate::os::macos::assert_awake(kind == Inhibit::Display, &what).map_err(DesktopError::platform)
    }
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod platform {
    use super::{Callback, DesktopError, Inhibit};

    pub(super) use super::linux::{Hold, Watch};

    fn system() -> Result<zbus::Connection, DesktopError> {
        zbus::block_on(zbus::Connection::system())
            .map_err(|e| DesktopError::unsupported("power status", format!("the system bus: {e}")))
    }

    pub(super) fn on_battery() -> Result<bool, DesktopError> {
        super::linux::on_battery(&system()?)
    }

    pub(super) fn watch(f: Callback) -> Result<Watch, DesktopError> {
        super::linux::watch(system()?, f)
    }

    pub(super) fn inhibit(kind: Inhibit, app: &str, reason: &str) -> Result<Hold, DesktopError> {
        let session = if kind == Inhibit::Display {
            Some(zbus::block_on(zbus::Connection::session()).map_err(|e| {
                DesktopError::unsupported("keeping the screen on", format!("the session bus: {e}"))
            })?)
        } else {
            None
        };
        super::linux::inhibit(system().ok(), session, app, reason)
    }
}

#[cfg(not(any(windows, unix, all(target_arch = "wasm32", target_os = "unknown"))))]
mod platform {
    use super::{Callback, DesktopError, Inhibit};

    pub(super) type Watch = ();
    pub(super) type Hold = ();

    fn no() -> DesktopError {
        DesktopError::unsupported("power status", "this platform reports no power state")
    }

    pub(super) fn on_battery() -> Result<bool, DesktopError> {
        Err(no())
    }

    pub(super) fn watch(_: Callback) -> Result<Watch, DesktopError> {
        Err(no())
    }

    pub(super) fn inhibit(_: Inhibit, _: &str, _: &str) -> Result<Hold, DesktopError> {
        Err(no())
    }
}

/// A browser page: no power state, and only the screen kept on, with the Screen Wake Lock
/// (`navigator.wakeLock`, reached by name). The browser lets go of the lock while the page is
/// hidden; a hold asks for it again each time the page shows.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod platform {
    use std::cell::RefCell;
    use std::rc::Rc;

    use js_sys::{Function, Reflect};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    use super::{Callback, DesktopError, Inhibit};

    pub(super) type Watch = ();

    /// The sentinel the browser hands back once it holds the lock, whether a request is in
    /// flight, and whether the hold went (a sentinel that comes after is let go at once).
    #[derive(Default)]
    struct Lock {
        sentinel: Option<JsValue>,
        asking: bool,
        dropped: bool,
    }

    pub(crate) struct Hold {
        lock: Rc<RefCell<Lock>>,
        /// Asks again when the page shows; removed with the hold.
        on_show: Closure<dyn FnMut()>,
    }

    fn get(o: &JsValue, k: &str) -> JsValue {
        Reflect::get(o, &k.into()).unwrap_or(JsValue::UNDEFINED)
    }

    fn method(o: &JsValue, k: &str) -> Option<Function> {
        get(o, k).dyn_into().ok()
    }

    fn release(sentinel: &JsValue) {
        if let Some(f) = method(sentinel, "release") {
            let _ = f.call0(sentinel);
        }
    }

    fn document() -> JsValue {
        get(&js_sys::global(), "document")
    }

    fn unsupported() -> DesktopError {
        DesktopError::unsupported("keeping the screen on", "this browser has no Screen Wake Lock")
    }

    /// Asks the browser for the lock, unless it is held or asked for already.
    fn request(lock: &Rc<RefCell<Lock>>) -> Result<(), DesktopError> {
        {
            let l = lock.borrow();
            let live = l.sentinel.as_ref().is_some_and(|s| get(s, "released") != JsValue::TRUE);
            if l.dropped || l.asking || live {
                return Ok(());
            }
        }
        let wake = get(&get(&js_sys::global(), "navigator"), "wakeLock");
        let request = wake.is_object().then(|| method(&wake, "request")).flatten().ok_or_else(unsupported)?;
        let promise = request
            .call1(&wake, &"screen".into())
            .map_err(|e| DesktopError::platform(format!("navigator.wakeLock.request: {e:?}")))?;
        let then = method(&promise, "then")
            .ok_or_else(|| DesktopError::platform("navigator.wakeLock.request returned no promise"))?;
        lock.borrow_mut().asking = true;
        let l = lock.clone();
        let held = Closure::once_into_js(move |sentinel: JsValue| {
            let mut l = l.borrow_mut();
            l.asking = false;
            match l.dropped {
                true => release(&sentinel),
                false => l.sentinel = Some(sentinel),
            }
        });
        // Refused (the page hidden, a permissions policy): nothing is held until it shows.
        let l = lock.clone();
        let refused = Closure::once_into_js(move |_: JsValue| l.borrow_mut().asking = false);
        then.call2(&promise, &held, &refused)
            .map_err(|e| DesktopError::platform(format!("navigator.wakeLock.request: {e:?}")))?;
        Ok(())
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            let doc = document();
            if let Some(f) = method(&doc, "removeEventListener") {
                let _ = f.call2(&doc, &"visibilitychange".into(), self.on_show.as_ref().unchecked_ref());
            }
            let mut l = self.lock.borrow_mut();
            l.dropped = true;
            if let Some(s) = l.sentinel.take() {
                release(&s);
            }
        }
    }

    fn no() -> DesktopError {
        DesktopError::unsupported("power status", "a page reports no power state")
    }

    pub(super) fn on_battery() -> Result<bool, DesktopError> {
        Err(no())
    }

    pub(super) fn watch(_: Callback) -> Result<Watch, DesktopError> {
        Err(no())
    }

    pub(super) fn inhibit(kind: Inhibit, _: &str, _: &str) -> Result<Hold, DesktopError> {
        if kind != Inhibit::Display {
            return Err(DesktopError::unsupported(
                "keeping the system awake",
                "a page can keep only the screen on",
            ));
        }
        let lock = Rc::new(RefCell::new(Lock::default()));
        request(&lock)?;
        let l = lock.clone();
        let on_show = Closure::<dyn FnMut()>::new(move || {
            if get(&document(), "visibilityState").as_string().as_deref() == Some("visible") {
                let _ = request(&l);
            }
        });
        let doc = document();
        if let Some(f) = method(&doc, "addEventListener") {
            let _ = f.call2(&doc, &"visibilitychange".into(), on_show.as_ref().unchecked_ref());
        }
        Ok(Hold { lock, on_show })
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use super::{Callback, DesktopError, Inhibit};

    pub(super) type Watch = ();
    pub(super) type Hold = ();

    fn no() -> DesktopError {
        DesktopError::unsupported(
            "power status",
            "the mobile bridges report power through the app's lifecycle",
        )
    }

    pub(super) fn on_battery() -> Result<bool, DesktopError> {
        Err(no())
    }

    pub(super) fn watch(_: Callback) -> Result<Watch, DesktopError> {
        Err(no())
    }

    pub(super) fn inhibit(_: Inhibit, _: &str, _: &str) -> Result<Hold, DesktopError> {
        Err(no())
    }
}

/// Android: power comes through the app's lifecycle; the screen is kept on with the
/// activity's `FLAG_KEEP_SCREEN_ON` (no permission needed), through the bridge's Java.
#[cfg(target_os = "android")]
mod platform {
    use super::{Callback, DesktopError, Inhibit};

    pub(super) type Watch = ();
    pub(super) type Hold = crate::os::android_media::ScreenOn;

    fn no() -> DesktopError {
        DesktopError::unsupported(
            "power status",
            "the mobile bridges report power through the app's lifecycle",
        )
    }

    pub(super) fn on_battery() -> Result<bool, DesktopError> {
        Err(no())
    }

    pub(super) fn watch(_: Callback) -> Result<Watch, DesktopError> {
        Err(no())
    }

    pub(super) fn inhibit(kind: Inhibit, _: &str, _: &str) -> Result<Hold, DesktopError> {
        match kind {
            Inhibit::Display => Hold::hold(),
            Inhibit::Sleep => Err(DesktopError::unsupported(
                "keeping the system awake",
                "an Android app keeps working while its screen is on; for work out of sight, use a foreground service",
            )),
        }
    }
}

/// UPower, logind and the screen saver over D-Bus. Every function takes its connections, so
/// the tests can hand it a private bus.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod linux {
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::task::Poll;

    use futures_core::Stream;
    use zbus::{Connection, OwnedVariant, Proxy};

    use super::{Callback, DesktopError, PowerEvent};

    pub(super) const UPOWER: &str = "org.freedesktop.UPower";
    pub(super) const UPOWER_PATH: &str = "/org/freedesktop/UPower";
    pub(super) const LOGIND: &str = "org.freedesktop.login1";
    pub(super) const LOGIND_PATH: &str = "/org/freedesktop/login1";
    pub(super) const LOGIND_IFACE: &str = "org.freedesktop.login1.Manager";
    pub(super) const SCREENSAVER: &str = "org.freedesktop.ScreenSaver";
    pub(super) const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
    const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

    fn has_owner(conn: &Connection, name: &str) -> bool {
        zbus::block_on(async {
            let dbus =
                Proxy::new(conn, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus")
                    .await?;
            dbus.call::<_, _, bool>("NameHasOwner", &(name,)).await
        })
        .unwrap_or(false)
    }

    pub(super) fn on_battery(conn: &Connection) -> Result<bool, DesktopError> {
        if !has_owner(conn, UPOWER) {
            return Err(DesktopError::unsupported("power status", "UPower is not running on this system"));
        }
        zbus::block_on(async {
            let p = Proxy::new(conn, UPOWER, UPOWER_PATH, UPOWER).await?;
            p.get_property::<bool>("OnBattery").await
        })
        .map_err(|e| DesktopError::platform(format!("UPower: {e}")))
    }

    /// Wakes the watch thread to stop.
    #[derive(Default)]
    struct Stop {
        stopped: AtomicBool,
        waker: Mutex<Option<std::task::Waker>>,
    }

    pub(crate) struct Watch {
        stop: Arc<Stop>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            self.stop.stopped.store(true, Ordering::SeqCst);
            let w = self.stop.waker.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some(w) = w {
                w.wake();
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    pub(super) fn watch(conn: Connection, f: Callback) -> Result<Watch, DesktopError> {
        let (upower, logind) = (has_owner(&conn, UPOWER), has_owner(&conn, LOGIND));
        if !upower && !logind {
            return Err(DesktopError::unsupported(
                "power notifications",
                "neither UPower nor logind is running on this system",
            ));
        }
        let stop = Arc::new(Stop::default());
        // Subscribed before the thread starts, so nothing sent after `watch` returns is missed.
        let streams = zbus::block_on(async {
            let sleep = Proxy::new(&conn, LOGIND, LOGIND_PATH, LOGIND_IFACE)
                .await?
                .receive_signal("PrepareForSleep")
                .await?;
            let props = Proxy::new(&conn, UPOWER, UPOWER_PATH, PROPERTIES)
                .await?
                .receive_signal("PropertiesChanged")
                .await?;
            Ok::<_, zbus::Error>((sleep, props))
        })
        .map_err(|e| DesktopError::platform(format!("power notifications: {e}")))?;
        let s = stop.clone();
        let thread = std::thread::Builder::new()
            .name("remedy power".into())
            .spawn(move || {
                let _conn = conn;
                let (mut sleep, mut props) = streams;
                zbus::block_on(std::future::poll_fn(|cx| {
                    *s.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(cx.waker().clone());
                    loop {
                        if s.stopped.load(Ordering::SeqCst) {
                            return Poll::Ready(());
                        }
                        let mut progressed = false;
                        if let Poll::Ready(Some(msg)) = Pin::new(&mut sleep).poll_next(cx) {
                            progressed = true;
                            if let Ok(start) = msg.body().deserialize::<bool>() {
                                f(if start { PowerEvent::Sleeping } else { PowerEvent::Resumed });
                            }
                        }
                        if let Poll::Ready(Some(msg)) = Pin::new(&mut props).poll_next(cx) {
                            progressed = true;
                            let body = msg.body();
                            if let Ok((iface, changed, _)) =
                                body.deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
                                && iface == UPOWER
                                && let Some(v) = changed.get("OnBattery")
                                && let Ok(on) = bool::try_from(&**v)
                            {
                                f(PowerEvent::OnBattery(on));
                            }
                        }
                        if !progressed {
                            return Poll::Pending;
                        }
                    }
                }));
            })
            .map_err(|e| DesktopError::platform(e.to_string()))?;
        Ok(Watch { stop, thread: Some(thread) })
    }

    /// A logind lock (released when its descriptor closes) and a screen saver cookie (released
    /// with `UnInhibit`, or when the connection closes).
    pub(crate) struct Hold {
        _lock: Option<zbus::OwnedFd>,
        screen: Option<(Connection, u32)>,
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            if let Some((conn, cookie)) = self.screen.take() {
                let _ = zbus::block_on(async {
                    let p = Proxy::new(&conn, SCREENSAVER, SCREENSAVER_PATH, SCREENSAVER).await?;
                    p.call::<_, _, ()>("UnInhibit", &(cookie,)).await
                });
            }
        }
    }

    fn logind_lock(
        conn: &Connection,
        what: &str,
        app: &str,
        reason: &str,
    ) -> Result<zbus::OwnedFd, zbus::Error> {
        zbus::block_on(async {
            let p = Proxy::new(conn, LOGIND, LOGIND_PATH, LOGIND_IFACE).await?;
            p.call::<_, _, zbus::OwnedFd>("Inhibit", &(what, app, reason, "block")).await
        })
    }

    /// Keeps the system awake with a logind lock; with `session`, keeps the screen on with the
    /// screen saver too (a lock is then a bonus where logind allows it).
    pub(super) fn inhibit(
        system: Option<Connection>,
        session: Option<Connection>,
        app: &str,
        reason: &str,
    ) -> Result<Hold, DesktopError> {
        let why = if reason.is_empty() { app } else { reason };
        match session {
            None => {
                let conn = system.ok_or_else(|| {
                    DesktopError::unsupported("keeping the system awake", "there is no system bus")
                })?;
                if !has_owner(&conn, LOGIND) {
                    return Err(DesktopError::unsupported(
                        "keeping the system awake",
                        "logind is not running",
                    ));
                }
                let fd = logind_lock(&conn, "sleep:idle", app, why)
                    .map_err(|e| DesktopError::platform(format!("logind refused the lock: {e}")))?;
                Ok(Hold { _lock: Some(fd), screen: None })
            }
            Some(conn) => {
                let lock = system.as_ref().and_then(|s| logind_lock(s, "sleep:idle", app, why).ok());
                if !has_owner(&conn, SCREENSAVER) {
                    // logind's idle lock is what a desktop without the interface honors.
                    return match lock {
                        Some(fd) => Ok(Hold { _lock: Some(fd), screen: None }),
                        None => Err(DesktopError::unsupported(
                            "keeping the screen on",
                            "the desktop has no screen saver service (org.freedesktop.ScreenSaver) and logind gave no lock",
                        )),
                    };
                }
                let cookie = zbus::block_on(async {
                    let p = Proxy::new(&conn, SCREENSAVER, SCREENSAVER_PATH, SCREENSAVER).await?;
                    p.call::<_, _, u32>("Inhibit", &(app, why)).await
                })
                .map_err(|e| DesktopError::platform(format!("the screen saver refused: {e}")))?;
                Ok(Hold { _lock: lock, screen: Some((conn, cookie)) })
            }
        }
    }
}

#[cfg(all(test, unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod tests {
    use std::collections::HashMap;
    use std::io::BufRead;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use zbus::{Connection, interface, object_server::SignalEmitter};

    use super::linux::{self, LOGIND, LOGIND_PATH, SCREENSAVER, SCREENSAVER_PATH, UPOWER, UPOWER_PATH};
    use super::{DesktopError, PowerEvent};

    /// A private bus for one test, gone when dropped.
    struct Bus {
        daemon: std::process::Child,
        address: String,
    }

    impl Bus {
        fn start() -> Option<Bus> {
            let mut daemon = std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()?;
            let mut line = String::new();
            std::io::BufReader::new(daemon.stdout.as_mut()?).read_line(&mut line).ok()?;
            Some(Bus { daemon, address: line.trim().to_owned() })
        }

        fn connect(&self) -> Connection {
            match zbus::block_on(async { zbus::conn::Builder::address(self.address.as_str()).build().await })
            {
                Ok(c) => c,
                Err(e) => unreachable!("the private bus at {} refused: {e}", self.address),
            }
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    /// The tests need `dbus-daemon`; without it they say so and pass.
    fn bus() -> Option<Bus> {
        let b = Bus::start();
        if b.is_none() {
            eprintln!("skipped: dbus-daemon is not installed");
        }
        b
    }

    struct UPower {
        on_battery: bool,
    }

    #[interface(name = "org.freedesktop.UPower")]
    impl UPower {
        #[zbus(property)]
        fn on_battery(&self) -> bool {
            self.on_battery
        }
    }

    /// What, who, why and mode, and the service's end of the lock.
    type Lock = (String, String, String, String, std::os::fd::OwnedFd);

    #[derive(Clone, Default)]
    struct Logind {
        locks: Arc<Mutex<Vec<Lock>>>,
    }

    #[interface(name = "org.freedesktop.login1.Manager")]
    impl Logind {
        fn inhibit(
            &self,
            what: String,
            who: String,
            why: String,
            mode: String,
        ) -> zbus::fdo::Result<zbus::OwnedFd> {
            let (ours, theirs) = std::os::unix::net::UnixStream::pair()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            self.locks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((
                what,
                who,
                why,
                mode,
                std::os::fd::OwnedFd::from(ours),
            ));
            Ok(zbus::OwnedFd::from(std::os::fd::OwnedFd::from(theirs)))
        }

        #[zbus(signal)]
        async fn prepare_for_sleep(emitter: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
    }

    #[derive(Clone, Default)]
    struct ScreenSaver {
        held: Arc<Mutex<Vec<u32>>>,
    }

    #[interface(name = "org.freedesktop.ScreenSaver")]
    impl ScreenSaver {
        fn inhibit(&self, _app: String, _reason: String) -> u32 {
            let mut h = self.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            h.push(7);
            7
        }

        fn un_inhibit(&self, cookie: u32) {
            self.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|c| *c != cookie);
        }
    }

    fn serve<I: zbus::object_server::Interface>(bus: &Bus, name: &str, path: &str, iface: I) -> Connection {
        match zbus::block_on(async {
            zbus::conn::Builder::address(bus.address.as_str()).name(name).serve_at(path, iface).build().await
        }) {
            Ok(c) => c,
            Err(e) => unreachable!("serving {name}: {e}"),
        }
    }

    #[test]
    fn battery_state_comes_from_upower() {
        let Some(bus) = bus() else { return };
        let app = bus.connect();
        assert!(
            matches!(linux::on_battery(&app), Err(DesktopError::Unsupported { .. })),
            "without UPower there is nothing to ask"
        );
        let _upower = serve(&bus, UPOWER, UPOWER_PATH, UPower { on_battery: true });
        assert_eq!(linux::on_battery(&app), Ok(true));
    }

    #[test]
    fn sleep_resume_and_power_source_changes_are_reported() {
        let Some(bus) = bus() else { return };
        let app = bus.connect();
        assert!(matches!(linux::watch(app.clone(), Box::new(|_| {})), Err(DesktopError::Unsupported { .. })));
        let logind = serve(&bus, LOGIND, LOGIND_PATH, Logind::default());
        let upower = serve(&bus, UPOWER, UPOWER_PATH, UPower { on_battery: false });
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let watch = match linux::watch(
            app,
            Box::new(move |e| {
                let _ = tx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).send(e);
            }),
        ) {
            Ok(w) => w,
            Err(e) => unreachable!("watching: {e}"),
        };
        let emit = |start: bool| {
            zbus::block_on(async {
                let i = logind.object_server().interface::<_, Logind>(LOGIND_PATH).await?;
                Logind::prepare_for_sleep(i.signal_emitter(), start).await
            })
        };
        assert!(emit(true).is_ok());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(PowerEvent::Sleeping));
        assert!(emit(false).is_ok());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(PowerEvent::Resumed));
        let changed: HashMap<&str, zbus::Variant<'_>> =
            HashMap::from([("OnBattery", zbus::Variant::from(true))]);
        let sent = zbus::block_on(async {
            let i = upower.object_server().interface::<_, UPower>(UPOWER_PATH).await?;
            i.get_mut().await.on_battery = true;
            zbus::fdo::Properties::properties_changed(
                i.signal_emitter(),
                zbus::names::InterfaceName::from_static_str_unchecked(UPOWER),
                changed,
                std::borrow::Cow::Borrowed(&[]),
            )
            .await
        });
        assert!(sent.is_ok(), "{sent:?}");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(PowerEvent::OnBattery(true)));
        drop(watch);
        assert!(emit(true).is_ok());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "a dropped watch reports nothing");
    }

    #[test]
    fn keeping_awake_holds_a_logind_lock_and_a_screen_saver_cookie() {
        let Some(bus) = bus() else { return };
        let app = bus.connect();
        let state = Logind::default();
        let _logind = serve(&bus, LOGIND, LOGIND_PATH, state.clone());
        let hold = match linux::inhibit(Some(app.clone()), None, "Hearth", "Downloading 3 files") {
            Ok(h) => h,
            Err(e) => unreachable!("{e}"),
        };
        {
            let locks = state.locks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(locks.len(), 1);
            let Some((what, who, why, mode, _)) = locks.first() else { return };
            assert_eq!(
                (what.as_str(), who.as_str(), why.as_str(), mode.as_str()),
                ("sleep:idle", "Hearth", "Downloading 3 files", "block")
            );
        }
        drop(hold);
        // logind releases a lock when every copy of its descriptor is closed: ours is.
        let mut buf = [0u8; 1];
        let closed = {
            let mut locks = state.locks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let (.., fd) = locks.remove(0);
            let mut s = std::os::unix::net::UnixStream::from(fd);
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            std::io::Read::read(&mut s, &mut buf)
        };
        assert!(matches!(closed, Ok(0)), "the app's end of the lock is closed: {closed:?}");

        let saver = ScreenSaver::default();
        let _screen = serve(&bus, SCREENSAVER, SCREENSAVER_PATH, saver.clone());
        let hold = match linux::inhibit(Some(app.clone()), Some(app.clone()), "Hearth", "Playing a video") {
            Ok(h) => h,
            Err(e) => unreachable!("{e}"),
        };
        assert_eq!(*saver.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner), vec![7]);
        drop(hold);
        assert!(
            saver.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty(),
            "UnInhibit ran"
        );
    }

    #[test]
    fn nothing_to_ask_is_unsupported() {
        let Some(bus) = bus() else { return };
        let app = bus.connect();
        assert!(matches!(
            linux::inhibit(Some(app.clone()), None, "Hearth", ""),
            Err(DesktopError::Unsupported { .. })
        ));
        assert!(matches!(
            linux::inhibit(Some(app.clone()), Some(app), "Hearth", ""),
            Err(DesktopError::Unsupported { .. })
        ));
    }
}
