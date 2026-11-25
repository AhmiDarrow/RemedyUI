//! Android's side of the host contract, over the shared winit core.

use std::sync::{Mutex, OnceLock, PoisonError};

use crate::bridge_winit::winit::event_loop::EventLoop;
use crate::bridge_winit::winit::platform::android::EventLoopBuilderExtAndroid;
use crate::bridge_winit::winit::platform::android::activity::AndroidApp;
use crate::bridge_winit::winit::window::{Window, WindowId as WinitId};
use crate::bridge_winit::{PlatformHooks, RunError, UserEvent, WindowNotice};
use jni::objects::GlobalRef;
use remedy_ui_core::host::accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use remedy_ui_core::host::{
    AccessibilityEvent, App, DialogStarted, DragData, DragStarted, DropEffect, FileDialog, ForegroundTask,
    HostEvent, ImeEvent, Notification, NotificationResponse, Permission, PermissionStarted, SafeArea,
    SystemSettings, TrayIcon, Unsupported, Waker,
};
use remedy_ui_core::input::Platform;

use crate::bridge_android::convert;
use crate::bridge_android::os::{self, Java};

/// What the platform reported from its own threads, waiting for the event loop.
pub(crate) enum Notice {
    /// `onTrimMemory` at this level.
    Trim(i32),
    /// The configuration or a followed setting changed: read the settings again.
    Settings,
    /// The drawing view's safe area changed: top, right, bottom, left in device pixels.
    SafeArea([i32; 4]),
    /// Assistive technology asked something of the window.
    Accessibility(AccessibilityEvent),
    /// The input method edited the focused field.
    Ime(ImeEvent),
    /// The document picker for this request closed with these files (empty: cancelled).
    Dialog(u64, Vec<std::path::PathBuf>),
    /// The person answered the permission prompt for this request: true when granted.
    Permission(u64, bool),
    /// The share chooser for this request closed.
    Share(u64, remedy_ui_core::host::ShareOutcome),
    /// The drag out ended: 1 dropped and taken, 0 dropped nowhere or refused, -1 never started.
    DragEnded(i32),
    /// A screen reader set a text field's text; the field is the target of the focus request
    /// just before.
    SetText(String),
    /// The person used one of the app's notifications.
    Notification(u64, NotificationResponse),
    /// Android ended the app's foreground work, or refused it once it ran: why.
    ForegroundStopped(String),
}

/// State the Java half and the accessibility adapter reach from the UI thread.
#[derive(Default)]
struct Shared {
    notices: Mutex<Vec<Notice>>,
    waker: Mutex<Option<Waker>>,
    /// The activity is paused (out of sight), when winit ignores wake-ups.
    paused: std::sync::atomic::AtomicBool,
    bridge: Mutex<Option<GlobalRef>>,
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(Shared::default)
}

/// Queues a notice and wakes the event loop to take it.
pub(crate) fn notify(n: Notice) {
    let s = shared();
    s.notices.lock().unwrap_or_else(PoisonError::into_inner).push(n);
    let waker = s.waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(w) = waker {
        w();
    }
}

/// Records whether the activity is paused (from its lifecycle, on the UI thread).
pub(crate) fn set_paused(paused: bool) {
    shared().paused.store(paused, std::sync::atomic::Ordering::Relaxed);
}

/// How often the loop looks for reports while the activity is paused: winit ignores wake-ups
/// then. Short enough that a notification's button answers at once to the person; the
/// process is frozen by Android anyway once it is cached and holds no foreground work.
const PAUSED_WAIT_SLICE: std::time::Duration = std::time::Duration::from_millis(100);

pub(crate) fn set_bridge(bridge: GlobalRef) {
    *shared().bridge.lock().unwrap_or_else(PoisonError::into_inner) = Some(bridge);
}

/// The Java half, once it has started.
pub(crate) fn bridge() -> Option<GlobalRef> {
    shared().bridge.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The activity android-activity started, until [`run`] takes it.
static ACTIVITY: Mutex<Option<AndroidApp>> = Mutex::new(None);

/// Hands the activity to the bridge. The [`main!`](crate::main) entry point calls this before
/// the app's own entry; call it yourself only when writing `android_main` by hand.
pub fn attach(app: AndroidApp) {
    if let Some(dir) = app.internal_data_path() {
        let _ = DATA_DIR.set(dir);
    }
    *ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner) = Some(app);
}

/// The app's private internal storage, kept from [`attach`] for as long as the process runs.
static DATA_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();

/// The app's private data folder: the internal storage Android gives each app
/// (`Context.getFilesDir()`, `/data/user/<user>/<package>/files`). Only the app reads and
/// writes it, it needs no permission, it is kept across updates and removed when the app is
/// uninstalled (or its data cleared). `None` before [`attach`], or when the activity reports
/// none.
pub fn data_dir() -> Option<std::path::PathBuf> {
    DATA_DIR.get().cloned()
}

/// A string extra of the intent that launched the app (`am start --es name value`, or
/// another app's intent), for launch options. `None` when absent, or before [`attach`].
pub fn launch_extra(name: &str) -> Option<String> {
    let app = ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner).clone()?;
    Java::new(&app).ok()?.intent_extra(name).ok().flatten()
}

/// The link that opened the app, when it was started by one in a scheme the app takes
/// (`[app] url_schemes` in `remedy.toml`): a link pressed in a browser or a scanned code.
/// `None` when the app was started another way, or before [`attach`]. It is the link the
/// app was started with: one pressed while the app is already running is not reported here.
pub fn launch_url() -> Option<String> {
    let app = ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner).clone()?;
    Java::new(&app).ok()?.intent_data().ok().flatten()
}

/// Whether [`hold_process`] holds the process: the app's way out waits for it to let go.
static HELD: (Mutex<bool>, std::sync::Condvar) = (Mutex::new(false), std::sync::Condvar::new());

/// How long the way out waits for held work (keepalive requests) unless the app says
/// otherwise ([`set_held_grace`]), as `remedy_ui::run` waits for them on the desktop.
pub const HELD_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

static GRACE: Mutex<std::time::Duration> = Mutex::new(HELD_GRACE);

/// Sets how long the app's way out waits for work that holds the process ([`hold_process`]:
/// keepalive requests) before it ends anyway (default [`HELD_GRACE`], 10 s).
/// `remedy_ui::run` sets it from `AppConfig::keepalive_grace`. Android may end a process
/// that takes long to go away itself, so a much longer wait may not be granted.
pub fn set_held_grace(grace: std::time::Duration) {
    *GRACE.lock().unwrap_or_else(PoisonError::into_inner) = grace;
}

/// The activity [`run`] is running, for calls from any thread.
static RUNNING: OnceLock<AndroidApp> = OnceLock::new();

/// Keeps the app's process running while it is out of sight (`true`), until told otherwise
/// (`false`): a started service of the bridge's (`dev.remedyui.android.Keepalive`, declared
/// by `cargo remedy package`), which keeps Android from freezing the cached process; Android
/// still ends it about a minute after the app leaves the screen. `remedy_ui::run` hands this
/// to the HTTP client, so keepalive requests (an app's last saves) finish when the app goes
/// to the background, as a browser lets a closing page's keepalive `fetch` finish. Any
/// thread; what fails (no service in the manifest, Android refusing) goes to the log.
pub fn hold_process(on: bool) {
    let (held, cv) = &HELD;
    *held.lock().unwrap_or_else(PoisonError::into_inner) = on;
    cv.notify_all();
    let Some(app) = RUNNING.get() else { return };
    if let Err(e) = Java::new(app).and_then(|j| j.hold_process(on)) {
        os::log(&format!("RemedyUI: the process could not be kept running out of sight: {e}"));
    }
}

/// Writes a line to the system log (logcat), tagged `RemedyUI`, at info priority. Android
/// discards a native app's standard output, so this is where an app's diagnostics go.
pub fn log(message: &str) {
    os::log(message);
}

/// Whether this app may offer an APK to the system installer (the "install unknown apps"
/// setting on Android 8 and later). `false` before [`attach`], or if the check fails.
pub fn can_request_package_installs() -> bool {
    let Some(app) = ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner).clone() else {
        return false;
    };
    Java::new(&app).ok().and_then(|j| j.can_request_package_installs().ok()).unwrap_or(false)
}

/// What [`install_apk`] did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApkInstall {
    /// The system installer was started; the person still has to tap Install.
    Started,
    /// The install-permission settings page was opened instead; call again after the person
    /// allows installs from this app.
    NeedPermission,
}

/// Opens the system package installer for an APK the app has already written under
/// `cache/remedy-share/<request>/<index>/<name>` (the same folder [`share`](crate) uses,
/// served by the APK's `SharedFiles` provider). The person taps Install; existing app data
/// is kept. When the "install unknown apps" setting is off, the settings page for this app
/// is opened instead ([`ApkInstall::NeedPermission`]).
///
/// `mime` is the content URI path segment, the same encoding shares use (for an APK,
/// `application/vnd.android.package-archive`).
pub fn install_apk(request: &str, index: u32, mime: &str, name: &str) -> Result<ApkInstall, String> {
    let app = ACTIVITY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| "the activity has not attached".to_owned())?;
    let java = Java::new(&app)?;
    let index = i32::try_from(index).map_err(|_| "share index is too large".to_owned())?;
    match java.install_apk(&app, request, index, mime, name)?.as_str() {
        "started" => Ok(ApkInstall::Started),
        "need_permission" => Ok(ApkInstall::NeedPermission),
        other => Err(format!("unexpected installer result: {other}")),
    }
}

/// Screen readers asking for the tree: the app sends it on its next frame.
struct Activation;

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        notify(Notice::Accessibility(AccessibilityEvent::Requested));
        None
    }
}

/// Screen readers acting on an element: the app does it on the event loop.
struct Actions;

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        notify(Notice::Accessibility(AccessibilityEvent::Action(request)));
    }
}

/// Android's side of the host contract.
pub struct AndroidHooks {
    app: AndroidApp,
    /// The activity's Java side, or why it could not be reached.
    java: Result<Java, String>,
    /// The one window a native activity shows.
    window: Option<WinitId>,
    a11y: Option<accesskit_android::InjectingAdapter>,
    /// The drawing view's safe area in device pixels: top, right, bottom, left.
    safe_px: [i32; 4],
    /// Events taken from the notices, each kind waiting for its own call.
    app_events: Vec<HostEvent>,
    window_events: Vec<(WinitId, WindowNotice)>,
    ime_events: Vec<(WinitId, ImeEvent)>,
    drag_ends: Vec<(WinitId, DropEffect)>,
    /// The settings last reported, to report only changes.
    settings: Option<SystemSettings>,
    /// The next document picker's id.
    next_dialog: u64,
    /// The next permission request's number.
    next_permission: u64,
    /// The next share's number.
    next_share: u64,
    /// A text-input session is open.
    text_session: bool,
    /// The element a screen reader last asked to focus (where its "set text" goes).
    a11y_focus: Option<(remedy_ui_core::host::accesskit::TreeId, remedy_ui_core::host::accesskit::NodeId)>,
}

impl std::fmt::Debug for AndroidHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AndroidHooks").field("window", &self.window).field("safe_px", &self.safe_px).finish()
    }
}

impl AndroidHooks {
    /// Hooks for the activity `app`; starts the bridge's Java half.
    pub fn new(app: AndroidApp) -> AndroidHooks {
        let java = Java::new(&app);
        match &java {
            Ok(j) => {
                if let Err(e) = j.start_bridge(&app) {
                    os::log(&format!(
                        "RemedyUI: memory trims, setting changes and safe areas are unavailable: {e}"
                    ));
                }
            }
            Err(e) => os::log(&format!("RemedyUI: the activity's Java side is unreachable: {e}")),
        }
        AndroidHooks {
            app,
            java,
            window: None,
            a11y: None,
            safe_px: [0; 4],
            app_events: Vec::new(),
            window_events: Vec::new(),
            ime_events: Vec::new(),
            drag_ends: Vec::new(),
            settings: None,
            next_dialog: 1,
            next_permission: 1,
            next_share: 1,
            text_session: false,
            a11y_focus: None,
        }
    }

    /// Sorts what the platform reported into the event lists the core takes.
    fn drain(&mut self) {
        let notices = std::mem::take(&mut *shared().notices.lock().unwrap_or_else(PoisonError::into_inner));
        let mut settings_changed = false;
        for n in notices {
            match n {
                Notice::Trim(level) => {
                    if let Some(p) = convert::trim_pressure(level) {
                        self.app_events.push(HostEvent::MemoryPressure(p));
                    }
                }
                Notice::Settings => settings_changed = true,
                Notice::SafeArea(px) => {
                    if px != self.safe_px {
                        self.safe_px = px;
                        if let Some(w) = self.window {
                            // The scale is the display's density, which the area is read with.
                            let scale = self.app.config().density().map_or(1.0, |d| f64::from(d) / 160.0);
                            self.window_events.push((w, WindowNotice::SafeArea(self.safe_area_at(scale))));
                        }
                    }
                }
                Notice::Accessibility(e) => {
                    if let AccessibilityEvent::Action(r) = &e
                        && r.action == remedy_ui_core::host::accesskit::Action::Focus
                    {
                        self.a11y_focus = Some((r.target_tree, r.target_node));
                    }
                    if let Some(w) = self.window {
                        self.window_events.push((w, WindowNotice::Accessibility(e)));
                    }
                }
                Notice::SetText(text) => {
                    if let (Some(w), Some((target_tree, target_node))) = (self.window, self.a11y_focus) {
                        let request = ActionRequest {
                            action: remedy_ui_core::host::accesskit::Action::SetValue,
                            target_tree,
                            target_node,
                            data: Some(remedy_ui_core::host::accesskit::ActionData::Value(text.into())),
                        };
                        self.window_events
                            .push((w, WindowNotice::Accessibility(AccessibilityEvent::Action(request))));
                    }
                }
                Notice::Ime(e) => {
                    if let Some(w) = self.window
                        && self.text_session
                    {
                        self.ime_events.push((w, e));
                    }
                }
                Notice::Dialog(id, paths) => {
                    self.app_events.push(HostEvent::FileDialog(remedy_ui_core::host::DialogId(id), paths));
                }
                Notice::Permission(id, granted) => {
                    self.app_events
                        .push(HostEvent::Permission(remedy_ui_core::host::PermissionId(id), granted));
                }
                Notice::Notification(id, response) => {
                    self.app_events.push(HostEvent::Notification(id, response));
                }
                Notice::ForegroundStopped(why) => {
                    self.app_events.push(HostEvent::ForegroundStopped(why));
                }
                Notice::Share(id, outcome) => {
                    self.app_events.push(HostEvent::Share(remedy_ui_core::host::ShareId(id), outcome));
                }
                Notice::DragEnded(result) => {
                    if let Some(w) = self.window {
                        self.drag_ends
                            .push((w, if result > 0 { DropEffect::Copy } else { DropEffect::None }));
                    }
                }
            }
        }
        if settings_changed {
            let s = self.read_settings();
            if self.settings.as_ref() != Some(&s) {
                self.settings = Some(s.clone());
                self.app_events.push(HostEvent::Settings(s));
            }
        }
    }

    fn java(&self, what: &'static str) -> Result<&Java, Unsupported> {
        self.java
            .as_ref()
            .map_err(|e| Unsupported::new(what, format!("the activity's Java side is unreachable: {e}")))
    }

    fn safe_area_at(&self, scale: f64) -> SafeArea {
        let [t, r, b, l] = self.safe_px.map(|v| v as f32);
        SafeArea::from_device(t, r, b, l, scale as f32)
    }

    fn read_settings(&self) -> SystemSettings {
        match &self.java {
            Ok(j) => j.settings().to_settings(),
            Err(_) => SystemSettings::default(),
        }
    }
}

impl Drop for AndroidHooks {
    fn drop(&mut self) {
        self.a11y = None;
        *shared().waker.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let bridge = shared().bridge.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let (Ok(j), Some(b)) = (&self.java, bridge) {
            j.stop_bridge(&self.app, b);
        }
    }
}

impl PlatformHooks for AndroidHooks {
    fn platform(&self) -> Platform {
        Platform::Standard
    }

    fn settings(&mut self) -> SystemSettings {
        let s = self.read_settings();
        self.settings = Some(s.clone());
        s
    }

    fn utc_offsets(&self) -> fn(i64) -> Option<i32> {
        os::utc_offset
    }

    fn total_memory(&self) -> Option<u64> {
        // "MemTotal:  3995260 kB" in /proc/meminfo, which apps may read.
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        kb.checked_mul(1024)
    }

    fn clipboard_text(&mut self) -> Result<Option<String>, Unsupported> {
        self.java("the clipboard")?.clipboard_read(false).map_err(|e| Unsupported::new("the clipboard", e))
    }

    fn set_clipboard_text(&mut self, text: &str, sensitive: bool) -> Result<(), Unsupported> {
        self.java("the clipboard")?
            .clipboard_write(text, None, sensitive)
            .map_err(|e| Unsupported::new("the clipboard", e))
    }

    fn clear_clipboard(&mut self) -> Result<(), Unsupported> {
        if AndroidApp::sdk_version() < 28 {
            return Err(Unsupported::new(
                "emptying the clipboard",
                "Android before 9 (API 28) has no way to empty the clipboard",
            ));
        }
        self.java("the clipboard")?.clipboard_clear().map_err(|e| Unsupported::new("the clipboard", e))
    }

    fn clipboard_html(&mut self) -> Result<Option<String>, Unsupported> {
        self.java("the clipboard")?.clipboard_read(true).map_err(|e| Unsupported::new("the clipboard", e))
    }

    fn set_clipboard_html(&mut self, text: &str, html: &str) -> Result<(), Unsupported> {
        self.java("the clipboard")?
            .clipboard_write(text, Some(html), false)
            .map_err(|e| Unsupported::new("the clipboard", e))
    }

    fn set_capture_protected(&mut self, _window: &Window, on: bool) -> Result<(), Unsupported> {
        // FLAG_SECURE, set on the UI thread. (android-activity's own window-flag call holds
        // its state lock while the flag change recreates the surface, whose callbacks need
        // that lock: the activity stops answering.)
        self.java("capture protection")?.set_secure(&self.app, on);
        Ok(())
    }

    fn cpu_presents_hold_surface(&self) -> bool {
        // `ANativeWindow_lock` connects the window to CPU rendering until it is destroyed;
        // Vulkan and EGL then fail on it (and wgpu cannot recover from that).
        true
    }

    fn window_created(&mut self, window: &Window) {
        self.window = Some(window.id());
    }

    fn set_window_icon(
        &mut self,
        _window: &Window,
        _icon: Option<&remedy_ui_core::host::WindowIcon>,
    ) -> Result<(), Unsupported> {
        Err(Unsupported::new("window icons", "Android shows the app's launcher icon for its window"))
    }

    fn set_tray(&mut self, _tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
        Err(Unsupported::new(
            "the tray",
            "Android has no system tray; apps reach people through notifications",
        ))
    }

    fn file_dialog(
        &mut self,
        _window: Option<&Window>,
        _dialog: &FileDialog,
    ) -> Result<Vec<std::path::PathBuf>, Unsupported> {
        // Reached only by calling the hook directly: the host starts dialogs with
        // `start_file_dialog`.
        Err(Unsupported::new(
            "a blocking file dialog",
            "Android's document picker is another screen that answers after this call returns",
        ))
    }

    fn start_file_dialog(
        &mut self,
        _window: Option<&Window>,
        dialog: &FileDialog,
    ) -> Result<DialogStarted, Unsupported> {
        if dialog.folders {
            // ACTION_OPEN_DOCUMENT_TREE grants a document tree reached only through
            // DocumentsContract URIs: no path names it, and the engine's dialogs return paths.
            return Err(Unsupported::new(
                "choosing folders",
                "a folder chosen on Android is a document tree reachable only through content URIs, \
                 not a path the app can read with file calls",
            ));
        }
        let id = self.next_dialog;
        self.next_dialog += 1;
        let extensions: Vec<String> =
            dialog.filters.iter().flat_map(|(_, exts)| exts.iter().cloned()).collect();
        self.java("file dialogs")?
            .pick(
                &self.app,
                id,
                dialog.save,
                dialog.multiple,
                extensions,
                dialog.file_name.clone(),
                dialog.title.clone(),
            )
            .map_err(|e| Unsupported::new("file dialogs", e))?;
        Ok(DialogStarted::Pending(remedy_ui_core::host::DialogId(id)))
    }

    fn request_permission(&mut self, permission: Permission) -> Result<PermissionStarted, Unsupported> {
        let name = match permission {
            Permission::Microphone => "android.permission.RECORD_AUDIO",
            // Granted at install before Android 13: nothing to ask, but the person may have
            // turned the app's notifications off, which is the answer then.
            Permission::Notifications if AndroidApp::sdk_version() < 33 => {
                let on = self
                    .java("notifications")?
                    .notifications_enabled()
                    .map_err(|e| Unsupported::new("notifications", e))?;
                return Ok(PermissionStarted::Finished(on));
            }
            Permission::Notifications => "android.permission.POST_NOTIFICATIONS",
            _ => {
                return Err(Unsupported::new(
                    "this permission",
                    "Android has no prompt the bridge knows for it",
                ));
            }
        };
        let id = self.next_permission;
        self.next_permission += 1;
        self.java("permission requests")?
            .request_permission(&self.app, id, name)
            .map_err(|e| Unsupported::new("permission requests", e))?;
        Ok(PermissionStarted::Pending(remedy_ui_core::host::PermissionId(id)))
    }

    fn show_notification(&mut self, notification: &Notification) -> Result<(), Unsupported> {
        self.java("system notifications")?
            .show_notification(notification)
            .map_err(|e| Unsupported::new("system notifications", e))
    }

    fn withdraw_notification(&mut self, id: u64) -> Result<(), Unsupported> {
        self.java("system notifications")?
            .withdraw_notification(id)
            .map_err(|e| Unsupported::new("system notifications", e))
    }

    fn start_foreground(&mut self, task: &ForegroundTask) -> Result<(), Unsupported> {
        let types = convert::foreground_types(&task.kinds).ok_or_else(|| {
            Unsupported::new(
                "foreground work",
                "a kind of work this release of the Android bridge does not know",
            )
        })?;
        if types == 0 {
            return Err(Unsupported::new("foreground work", "name at least one kind of work"));
        }
        self.java("foreground work")?
            .start_foreground(&task.notification, types)
            .map_err(|e| Unsupported::new("foreground work", e))
    }

    fn stop_foreground(&mut self) -> Result<(), Unsupported> {
        self.java("foreground work")?.stop_foreground().map_err(|e| Unsupported::new("foreground work", e))
    }

    fn share(
        &mut self,
        window: Option<&Window>,
        share: &remedy_ui_core::host::ShareOptions,
    ) -> Result<remedy_ui_core::host::ShareStarted, Unsupported> {
        let _ = window;
        let id = self.next_share;
        self.next_share += 1;
        self.java("sharing")?.share(&self.app, id, share).map_err(|e| Unsupported::new("sharing", e))?;
        Ok(remedy_ui_core::host::ShareStarted::Pending(remedy_ui_core::host::ShareId(id)))
    }

    fn print_html(&mut self, window: Option<&Window>, title: &str, html: &str) -> Result<(), Unsupported> {
        let _ = window;
        self.java("printing")?
            .print_html(&self.app, title.to_owned(), html.to_owned())
            .map_err(|e| Unsupported::new("printing", e))
    }

    fn start_drag(
        &mut self,
        window: &Window,
        data: &DragData,
        _allowed: &[DropEffect],
    ) -> Result<DragStarted, Unsupported> {
        if !data.files.is_empty() {
            // Another app reads a dragged file only through a content URI the app serves
            // (a FileProvider in its manifest), never through a path.
            return Err(Unsupported::new(
                "dragging files out",
                "Android apps share files as content URIs from a provider the app declares, not as paths",
            ));
        }
        if data.text.is_none() && data.uris.is_empty() {
            return Err(Unsupported::new("dragging out", "nothing to drag"));
        }
        // A finger-sized square under the touch (48 dp, Android's touch target).
        let shadow = (48.0 * window.scale_factor()).round() as i32;
        self.java("dragging out")?
            .start_drag(&self.app, data.text.clone(), data.uris.clone(), shadow)
            .map_err(|e| Unsupported::new("dragging out", e))?;
        Ok(DragStarted::Pending)
    }

    fn take_drag_ends(&mut self) -> Vec<(WinitId, DropEffect)> {
        self.drain();
        std::mem::take(&mut self.drag_ends)
    }

    fn set_text_input(
        &mut self,
        window: &Window,
        state: Option<&remedy_ui_core::host::TextInputState>,
    ) -> bool {
        let Ok(j) = &self.java else { return false };
        let r = match state {
            Some(s) => {
                let (text, selection) = convert::utf16_selection(&s.surrounding, s.selection.clone());
                let scale = window.scale_factor() as f32;
                let caret = (s.caret.min.x * scale, s.caret.min.y * scale, s.caret.max.y * scale);
                self.text_session = true;
                j.text_input(
                    &self.app,
                    text,
                    selection,
                    caret,
                    s.private,
                    s.multiline,
                    s.purpose,
                    s.enter_key(),
                )
            }
            None => {
                self.text_session = false;
                j.end_text_input(&self.app)
            }
        };
        match r {
            Ok(()) => true,
            Err(e) => {
                os::log(&format!("RemedyUI: the input method view is unavailable, keys only: {e}"));
                false
            }
        }
    }

    fn keys_bypass_ime(&self) -> bool {
        // The native activity's input queue takes keys before the input method sees them.
        self.text_session
    }

    fn end_composition(&mut self, _window: &Window) {
        if let Ok(j) = &self.java
            && let Err(e) = j.finish_composition(&self.app)
        {
            os::log(&format!("RemedyUI: the input method was not told its composition ended: {e}"));
        }
    }

    fn take_ime_events(&mut self) -> Vec<(WinitId, ImeEvent)> {
        self.drain();
        std::mem::take(&mut self.ime_events)
    }

    fn open_url(&mut self, url: &str) -> Result<(), Unsupported> {
        self.java("opening links")?.open_url(url).map_err(|e| Unsupported::new("opening links", e))
    }

    fn set_waker(&mut self, waker: Waker) {
        *shared().waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker.clone());
        // Anything reported before the loop could be woken is waiting.
        if !shared().notices.lock().unwrap_or_else(PoisonError::into_inner).is_empty() {
            waker();
        }
    }

    fn start_accessibility(&mut self, _window: &Window) -> bool {
        let Ok(j) = &self.java else { return false };
        match j.accessibility(Activation, Actions) {
            Ok(a) => {
                self.a11y = Some(a);
                if let Err(e) = j.extend_accessibility(&self.app) {
                    os::log(&format!("RemedyUI: screen readers cannot set text here: {e}"));
                }
                true
            }
            Err(e) => {
                os::log(&format!("RemedyUI: screen readers cannot reach this window: {e}"));
                false
            }
        }
    }

    fn update_accessibility(&mut self, _window: &Window, update: TreeUpdate) {
        if let Some(a) = self.a11y.as_mut() {
            a.update_if_active(|| update);
        }
    }

    fn safe_area(&self, window: &Window) -> SafeArea {
        self.safe_area_at(window.scale_factor())
    }

    fn wait_slice(&self) -> Option<std::time::Duration> {
        shared().paused.load(std::sync::atomic::Ordering::Relaxed).then_some(PAUSED_WAIT_SLICE)
    }

    fn take_events(&mut self) -> Vec<HostEvent> {
        self.drain();
        std::mem::take(&mut self.app_events)
    }

    fn take_window_events(&mut self) -> Vec<(WinitId, WindowNotice)> {
        self.drain();
        std::mem::take(&mut self.window_events)
    }
}

/// Runs `app` on the activity's event loop until it exits. The activity comes from
/// [`attach`], which the [`main!`](crate::main) entry point calls.
///
/// When the loop ends the activity finishes and, once Android has destroyed it, the process
/// exits: winit makes one event loop per process, so a later launch into the same (cached)
/// process could not start. `run` therefore returns only when the loop could not start.
pub fn run(app: impl App) -> Result<(), RunError> {
    // Keep the activity in ACTIVITY so `launch_extra`, `install_apk` and the rest can still
    // reach it after the event loop has started (window code, background work).
    let activity =
        ACTIVITY.lock().unwrap_or_else(PoisonError::into_inner).clone().ok_or_else(|| {
            RunError::new("no Android activity: start the app with remedy_ui_platform::main!")
        })?;
    let _ = RUNNING.set(activity.clone());
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .with_android_app(activity.clone())
        .build()
        .map_err(|e| RunError::new(e.to_string()))?;
    let result = crate::bridge_winit::run_on(event_loop, app, AndroidHooks::new(activity.clone()));
    if let Err(e) = &result {
        os::log(&format!("RemedyUI: the event loop ended with an error: {e}"));
    }
    finish_and_exit(&activity, result.is_ok())
}

/// Finishes the activity, waits (a few seconds at most) for Android to destroy it, and ends
/// the process.
fn finish_and_exit(activity: &AndroidApp, ok: bool) -> ! {
    use crate::bridge_winit::winit::platform::android::activity::{MainEvent, PollEvent};
    match Java::new(activity) {
        Ok(j) => {
            if let Err(e) = j.finish() {
                os::log(&format!("RemedyUI: the activity could not finish: {e}"));
            }
        }
        Err(e) => os::log(&format!("RemedyUI: the activity could not finish: {e}")),
    }
    // Android's own grace period for an activity to go away; a tuning constant, not a cap.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut destroyed = false;
    while !destroyed {
        let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) else { break };
        activity.poll_events(Some(left), |e| {
            if matches!(e, PollEvent::Main(MainEvent::Destroy)) {
                destroyed = true;
            }
        });
    }
    // The exit below runs no destructors: wait here for the work that holds the process.
    let (held, cv) = &HELD;
    let held = held.lock().unwrap_or_else(PoisonError::into_inner);
    let grace = *GRACE.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = cv.wait_timeout_while(held, grace, |h| *h);
    std::process::exit(if ok { 0 } else { 1 })
}
