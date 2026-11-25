//! WebKitGTK on Linux, under X11.
//!
//! WebKitGTK 4.1 (or 4.0) and GTK 3 are loaded at run time with `dlopen`, so apps build and
//! start without them and only a web view needs them. The page lives in a `WebKitWebView`
//! inside a `GtkFixed` inside a GTK popup window whose X window is reparented into the app's
//! window (GDK's foreign-window support). The popup window is moved to the part of the
//! element that shows; the web view inside it is placed at the element's whole box, so the
//! popup's edges clip the page; overlay boxes are cut out with the X shape extension.
//!
//! GTK runs on the UI thread. Its events (and WebKit's messages from its web and network
//! processes) are serviced by pumping GLib's default main context from a UI timer while a
//! web view exists.
//!
//! Wayland is not supported: a Wayland client cannot place a surface inside another
//! client's surface, and WebKitGTK is another client of the compositor. Under Wayland the
//! element reports [`WebViewStatus::Unsupported`].

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_ulong, c_void};
use std::rc::{Rc, Weak};
use std::sync::OnceLock;
use std::time::Duration;

use remedy_ui_core::host::WindowHandles;
use remedy_ui_core::host::raw_window_handle::RawWindowHandle;
use remedy_ui_engine::view::{Placement, WeakUi};

use super::{Layout, PxRect};
use crate::embed_webview::{BlockReason, Command, Expected, Hub, NativeView, WebViewStatus, json_eval};

/// How often GLib's main context is serviced while a web view exists: twice a 60 Hz frame,
/// so input, painting and WebKit's replies from its processes keep up with the display,
/// while an idle view costs only a non-blocking poll per tick.
const PUMP_INTERVAL: Duration = Duration::from_millis(8);

/// `GTK_WINDOW_POPUP`: no window manager decorations or placement.
const GTK_WINDOW_POPUP: c_int = 1;
/// `WebKitPolicyDecisionType`.
const DECISION_NAVIGATION: c_int = 0;
const DECISION_NEW_WINDOW: c_int = 1;
/// `WebKitLoadEvent`.
const LOAD_STARTED: c_int = 0;
const LOAD_FINISHED: c_int = 3;
/// `WEBKIT_USER_CONTENT_INJECT_TOP_FRAME`, `WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START`.
const INJECT_TOP_FRAME: c_int = 1;
const AT_DOCUMENT_START: c_int = 0;

#[repr(C)]
struct CairoRect {
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
}

type Ptr = *mut c_void;
type GCallback = *const c_void;
type DestroyNotify = unsafe extern "C" fn(Ptr, Ptr);

/// `GError`. The message is a glib string freed with [`Api::g_error_free`].
#[repr(C)]
struct GError {
    _domain: u32,
    _code: i32,
    message: *mut c_char,
}

/// The GTK, GDK, GLib, cairo and WebKitGTK functions used, from the loaded libraries.
struct Api {
    g_main_context_iteration: unsafe extern "C" fn(Ptr, c_int) -> c_int,
    g_main_context_pending: unsafe extern "C" fn(Ptr) -> c_int,
    g_signal_connect_data:
        unsafe extern "C" fn(Ptr, *const c_char, GCallback, Ptr, Option<DestroyNotify>, c_int) -> c_ulong,
    g_object_new: unsafe extern "C" fn(usize, *const c_char, ...) -> Ptr,
    g_object_unref: unsafe extern "C" fn(Ptr),
    g_free: unsafe extern "C" fn(Ptr),
    gdk_set_allowed_backends: unsafe extern "C" fn(*const c_char),
    gtk_init_check: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) -> c_int,
    gtk_window_new: unsafe extern "C" fn(c_int) -> Ptr,
    gtk_fixed_new: unsafe extern "C" fn() -> Ptr,
    gtk_fixed_move: unsafe extern "C" fn(Ptr, Ptr, c_int, c_int),
    gtk_container_add: unsafe extern "C" fn(Ptr, Ptr),
    gtk_widget_set_size_request: unsafe extern "C" fn(Ptr, c_int, c_int),
    gtk_window_resize: unsafe extern "C" fn(Ptr, c_int, c_int),
    gtk_widget_realize: unsafe extern "C" fn(Ptr),
    gtk_widget_show_all: unsafe extern "C" fn(Ptr),
    gtk_widget_hide: unsafe extern "C" fn(Ptr),
    gtk_widget_destroy: unsafe extern "C" fn(Ptr),
    gtk_widget_get_window: unsafe extern "C" fn(Ptr) -> Ptr,
    gtk_widget_grab_focus: unsafe extern "C" fn(Ptr),
    gdk_display_get_default: unsafe extern "C" fn() -> Ptr,
    gdk_x11_window_foreign_new_for_display: unsafe extern "C" fn(Ptr, c_ulong) -> Ptr,
    gdk_window_reparent: unsafe extern "C" fn(Ptr, Ptr, c_int, c_int),
    gdk_window_move_resize: unsafe extern "C" fn(Ptr, c_int, c_int, c_int, c_int),
    gdk_window_shape_combine_region: unsafe extern "C" fn(Ptr, *const c_void, c_int, c_int),
    gdk_window_get_geometry: unsafe extern "C" fn(Ptr, *mut c_int, *mut c_int, *mut c_int, *mut c_int),
    gdk_window_is_visible: unsafe extern "C" fn(Ptr) -> c_int,
    cairo_region_create_rectangle: unsafe extern "C" fn(*const CairoRect) -> Ptr,
    cairo_region_subtract_rectangle: unsafe extern "C" fn(Ptr, *const CairoRect) -> c_int,
    cairo_region_destroy: unsafe extern "C" fn(Ptr),
    webkit_web_view_get_type: unsafe extern "C" fn() -> usize,
    webkit_user_content_manager_new: unsafe extern "C" fn() -> Ptr,
    webkit_user_content_manager_register_script_message_handler:
        unsafe extern "C" fn(Ptr, *const c_char) -> c_int,
    webkit_user_script_new:
        unsafe extern "C" fn(*const c_char, c_int, c_int, *const *const c_char, *const *const c_char) -> Ptr,
    webkit_user_content_manager_add_script: unsafe extern "C" fn(Ptr, Ptr),
    webkit_user_script_unref: unsafe extern "C" fn(Ptr),
    webkit_website_data_manager_new: unsafe extern "C" fn(*const c_char, ...) -> Ptr,
    webkit_web_context_new_with_website_data_manager: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_web_context_new: unsafe extern "C" fn() -> Ptr,
    webkit_web_view_get_settings: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_settings_set_enable_developer_extras: unsafe extern "C" fn(Ptr, c_int),
    webkit_settings_set_javascript_can_open_windows_automatically: unsafe extern "C" fn(Ptr, c_int),
    webkit_web_view_load_uri: unsafe extern "C" fn(Ptr, *const c_char),
    webkit_web_view_load_html: unsafe extern "C" fn(Ptr, *const c_char, *const c_char),
    webkit_web_view_go_back: unsafe extern "C" fn(Ptr),
    webkit_web_view_go_forward: unsafe extern "C" fn(Ptr),
    webkit_web_view_reload: unsafe extern "C" fn(Ptr),
    webkit_web_view_stop_loading: unsafe extern "C" fn(Ptr),
    webkit_web_view_can_go_back: unsafe extern "C" fn(Ptr) -> c_int,
    webkit_web_view_can_go_forward: unsafe extern "C" fn(Ptr) -> c_int,
    webkit_web_view_get_uri: unsafe extern "C" fn(Ptr) -> *const c_char,
    webkit_web_view_get_title: unsafe extern "C" fn(Ptr) -> *const c_char,
    webkit_web_view_run_javascript: unsafe extern "C" fn(Ptr, *const c_char, Ptr, GCallback, Ptr),
    webkit_policy_decision_use: unsafe extern "C" fn(Ptr),
    webkit_policy_decision_ignore: unsafe extern "C" fn(Ptr),
    webkit_navigation_policy_decision_get_navigation_action: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_navigation_action_get_request: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_navigation_action_is_user_gesture: unsafe extern "C" fn(Ptr) -> c_int,
    webkit_navigation_action_is_redirect: unsafe extern "C" fn(Ptr) -> c_int,
    webkit_uri_request_get_uri: unsafe extern "C" fn(Ptr) -> *const c_char,
    webkit_permission_request_deny: unsafe extern "C" fn(Ptr),
    webkit_download_get_request: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_download_cancel: unsafe extern "C" fn(Ptr),
    webkit_javascript_result_get_js_value: unsafe extern "C" fn(Ptr) -> Ptr,
    jsc_value_to_string: unsafe extern "C" fn(Ptr) -> *mut c_char,
    webkit_web_view_run_javascript_finish: unsafe extern "C" fn(Ptr, Ptr, *mut *mut GError) -> Ptr,
    webkit_javascript_result_unref: unsafe extern "C" fn(Ptr),
    g_error_free: unsafe extern "C" fn(*mut GError),
    webkit_settings_set_user_agent: unsafe extern "C" fn(Ptr, *const c_char),
}

/// Looks up `name` in `lib` as a function of type `T`.
///
/// # Safety
/// `T` must be the C function's true signature, as an `unsafe extern "C" fn` pointer type.
unsafe fn sym<T: Copy>(lib: Ptr, name: &str) -> Result<T, String> {
    let c = CString::new(name).map_err(|e| e.to_string())?;
    // SAFETY: `lib` is a live handle from dlopen and `c` a NUL-terminated name.
    let p = unsafe { libc::dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        return Err(format!("{name} is missing from WebKitGTK's libraries"));
    }
    debug_assert_eq!(size_of::<T>(), size_of::<Ptr>());
    // SAFETY: per the caller, `T` is a function pointer type matching the symbol.
    Ok(unsafe { std::mem::transmute_copy::<Ptr, T>(&p) })
}

impl Api {
    /// Loads WebKitGTK (4.1, else 4.0) once per process; everything else comes through its
    /// dependencies.
    fn get() -> Result<&'static Api, String> {
        static API: OnceLock<Result<Api, String>> = OnceLock::new();
        API.get_or_init(Api::load).as_ref().map_err(Clone::clone)
    }

    fn load() -> Result<Api, String> {
        let mut lib = std::ptr::null_mut();
        for name in [c"libwebkit2gtk-4.1.so.0", c"libwebkit2gtk-4.0.so.37"] {
            // SAFETY: dlopen of a system library by name; the handle is kept for the process.
            lib = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !lib.is_null() {
                break;
            }
        }
        if lib.is_null() {
            return Err("WebKitGTK (libwebkit2gtk-4.1 or 4.0) is not installed".into());
        }
        // SAFETY: each type below is the documented C signature of the named function.
        unsafe {
            Ok(Api {
                g_main_context_iteration: sym(lib, "g_main_context_iteration")?,
                g_main_context_pending: sym(lib, "g_main_context_pending")?,
                g_signal_connect_data: sym(lib, "g_signal_connect_data")?,
                g_object_new: sym(lib, "g_object_new")?,
                g_object_unref: sym(lib, "g_object_unref")?,
                g_free: sym(lib, "g_free")?,
                gdk_set_allowed_backends: sym(lib, "gdk_set_allowed_backends")?,
                gtk_init_check: sym(lib, "gtk_init_check")?,
                gtk_window_new: sym(lib, "gtk_window_new")?,
                gtk_fixed_new: sym(lib, "gtk_fixed_new")?,
                gtk_fixed_move: sym(lib, "gtk_fixed_move")?,
                gtk_container_add: sym(lib, "gtk_container_add")?,
                gtk_widget_set_size_request: sym(lib, "gtk_widget_set_size_request")?,
                gtk_window_resize: sym(lib, "gtk_window_resize")?,
                gtk_widget_realize: sym(lib, "gtk_widget_realize")?,
                gtk_widget_show_all: sym(lib, "gtk_widget_show_all")?,
                gtk_widget_hide: sym(lib, "gtk_widget_hide")?,
                gtk_widget_destroy: sym(lib, "gtk_widget_destroy")?,
                gtk_widget_get_window: sym(lib, "gtk_widget_get_window")?,
                gtk_widget_grab_focus: sym(lib, "gtk_widget_grab_focus")?,
                gdk_display_get_default: sym(lib, "gdk_display_get_default")?,
                gdk_x11_window_foreign_new_for_display: sym(lib, "gdk_x11_window_foreign_new_for_display")?,
                gdk_window_reparent: sym(lib, "gdk_window_reparent")?,
                gdk_window_move_resize: sym(lib, "gdk_window_move_resize")?,
                gdk_window_shape_combine_region: sym(lib, "gdk_window_shape_combine_region")?,
                gdk_window_get_geometry: sym(lib, "gdk_window_get_geometry")?,
                gdk_window_is_visible: sym(lib, "gdk_window_is_visible")?,
                cairo_region_create_rectangle: sym(lib, "cairo_region_create_rectangle")?,
                cairo_region_subtract_rectangle: sym(lib, "cairo_region_subtract_rectangle")?,
                cairo_region_destroy: sym(lib, "cairo_region_destroy")?,
                webkit_web_view_get_type: sym(lib, "webkit_web_view_get_type")?,
                webkit_user_content_manager_new: sym(lib, "webkit_user_content_manager_new")?,
                webkit_user_content_manager_register_script_message_handler: sym(
                    lib,
                    "webkit_user_content_manager_register_script_message_handler",
                )?,
                webkit_user_script_new: sym(lib, "webkit_user_script_new")?,
                webkit_user_content_manager_add_script: sym(lib, "webkit_user_content_manager_add_script")?,
                webkit_user_script_unref: sym(lib, "webkit_user_script_unref")?,
                webkit_website_data_manager_new: sym(lib, "webkit_website_data_manager_new")?,
                webkit_web_context_new_with_website_data_manager: sym(
                    lib,
                    "webkit_web_context_new_with_website_data_manager",
                )?,
                webkit_web_context_new: sym(lib, "webkit_web_context_new")?,
                webkit_web_view_get_settings: sym(lib, "webkit_web_view_get_settings")?,
                webkit_settings_set_enable_developer_extras: sym(
                    lib,
                    "webkit_settings_set_enable_developer_extras",
                )?,
                webkit_settings_set_javascript_can_open_windows_automatically: sym(
                    lib,
                    "webkit_settings_set_javascript_can_open_windows_automatically",
                )?,
                webkit_web_view_load_uri: sym(lib, "webkit_web_view_load_uri")?,
                webkit_web_view_load_html: sym(lib, "webkit_web_view_load_html")?,
                webkit_web_view_go_back: sym(lib, "webkit_web_view_go_back")?,
                webkit_web_view_go_forward: sym(lib, "webkit_web_view_go_forward")?,
                webkit_web_view_reload: sym(lib, "webkit_web_view_reload")?,
                webkit_web_view_stop_loading: sym(lib, "webkit_web_view_stop_loading")?,
                webkit_web_view_can_go_back: sym(lib, "webkit_web_view_can_go_back")?,
                webkit_web_view_can_go_forward: sym(lib, "webkit_web_view_can_go_forward")?,
                webkit_web_view_get_uri: sym(lib, "webkit_web_view_get_uri")?,
                webkit_web_view_get_title: sym(lib, "webkit_web_view_get_title")?,
                webkit_web_view_run_javascript: sym(lib, "webkit_web_view_run_javascript")?,
                webkit_policy_decision_use: sym(lib, "webkit_policy_decision_use")?,
                webkit_policy_decision_ignore: sym(lib, "webkit_policy_decision_ignore")?,
                webkit_navigation_policy_decision_get_navigation_action: sym(
                    lib,
                    "webkit_navigation_policy_decision_get_navigation_action",
                )?,
                webkit_navigation_action_get_request: sym(lib, "webkit_navigation_action_get_request")?,
                webkit_navigation_action_is_user_gesture: sym(
                    lib,
                    "webkit_navigation_action_is_user_gesture",
                )?,
                webkit_navigation_action_is_redirect: sym(lib, "webkit_navigation_action_is_redirect")?,
                webkit_uri_request_get_uri: sym(lib, "webkit_uri_request_get_uri")?,
                webkit_permission_request_deny: sym(lib, "webkit_permission_request_deny")?,
                webkit_download_get_request: sym(lib, "webkit_download_get_request")?,
                webkit_download_cancel: sym(lib, "webkit_download_cancel")?,
                webkit_javascript_result_get_js_value: sym(lib, "webkit_javascript_result_get_js_value")?,
                jsc_value_to_string: sym(lib, "jsc_value_to_string")?,
                webkit_web_view_run_javascript_finish: sym(lib, "webkit_web_view_run_javascript_finish")?,
                webkit_javascript_result_unref: sym(lib, "webkit_javascript_result_unref")?,
                g_error_free: sym(lib, "g_error_free")?,
                webkit_settings_set_user_agent: sym(lib, "webkit_settings_set_user_agent")?,
            })
        }
    }
}

thread_local! {
    /// GTK was initialized on this thread (the result, once tried).
    static GTK: RefCell<Option<Result<(), String>>> = const { RefCell::new(None) };
}

/// Initializes GTK on this thread once, on its X11 backend.
fn init_gtk(api: &Api) -> Result<(), String> {
    GTK.with(|g| {
        g.borrow_mut()
            .get_or_insert_with(|| {
                // SAFETY: plain GTK start-up calls on the thread that will run GTK.
                unsafe {
                    (api.gdk_set_allowed_backends)(c"x11".as_ptr());
                    if (api.gtk_init_check)(std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
                        return Err("GTK could not open the X11 display".into());
                    }
                }
                Ok(())
            })
            .clone()
    })
}

/// A copy of a C string GTK owns.
///
/// # Safety
/// `p` is null or a valid NUL-terminated string.
unsafe fn text(p: *const c_char) -> Option<String> {
    // SAFETY: per the caller.
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

fn cstring(s: &str) -> CString {
    // Interior NULs cannot cross into C: cut there, as the platform would read it.
    CString::new(s.split('\0').next().unwrap_or_default()).unwrap_or_default()
}

struct Inner {
    hub: Rc<Hub>,
    api: &'static Api,
    /// The GTK popup window, reparented into the app's window.
    window: Ptr,
    fixed: Ptr,
    view: Ptr,
    layout: RefCell<Option<Layout>>,
    closed: std::cell::Cell<bool>,
}

/// A WebKitGTK web view in the app's X11 window.
pub(crate) struct Native(Rc<Inner>);

/// Connects `handler` to `signal` on `instance`, with a weak reference back as its data.
///
/// # Safety
/// `instance` is a live GObject with that signal, and `handler`'s true signature is the
/// signal's, with the data last.
unsafe fn connect(api: &Api, instance: Ptr, signal: &CStr, handler: GCallback, inner: &Rc<Inner>) {
    unsafe extern "C" fn free(data: Ptr, _closure: Ptr) {
        // SAFETY: `data` came from `Box::into_raw` below and is freed once, by GLib.
        drop(unsafe { Box::from_raw(data.cast::<Weak<Inner>>()) });
    }
    let data = Box::into_raw(Box::new(Rc::downgrade(inner))).cast::<c_void>();
    // SAFETY: per the caller; GLib frees `data` with `free` when the handler goes.
    unsafe { (api.g_signal_connect_data)(instance, signal.as_ptr(), handler, data, Some(free), 0) };
}

/// The view a signal's data points back to, while it lives.
///
/// # Safety
/// `data` is a pointer [`connect`] made.
unsafe fn back(data: Ptr) -> Option<Rc<Inner>> {
    // SAFETY: per the caller, a live boxed weak reference.
    unsafe { &*data.cast::<Weak<Inner>>() }.upgrade().filter(|i| !i.closed.get())
}

unsafe extern "C" fn on_decide(_view: Ptr, decision: Ptr, kind: c_int, data: Ptr) -> c_int {
    // SAFETY: the arguments of WebKit's `decide-policy` signal, on the UI thread.
    unsafe {
        let Some(inner) = back(data) else { return 0 };
        let api = inner.api;
        let action = match kind {
            DECISION_NAVIGATION | DECISION_NEW_WINDOW => {
                (api.webkit_navigation_policy_decision_get_navigation_action)(decision)
            }
            // Responses: the default handling (a download goes to `download-started`).
            _ => return 0,
        };
        let request = (api.webkit_navigation_action_get_request)(action);
        let url = text((api.webkit_uri_request_get_uri)(request)).unwrap_or_default();
        if kind == DECISION_NEW_WINDOW {
            (api.webkit_policy_decision_ignore)(decision);
            inner.hub.block(url, BlockReason::NewWindow);
            return 1;
        }
        let user = (api.webkit_navigation_action_is_user_gesture)(action) != 0;
        let redirect = (api.webkit_navigation_action_is_redirect)(action) != 0;
        if inner.hub.decide(url, user, redirect, false) {
            (api.webkit_policy_decision_use)(decision);
        } else {
            (api.webkit_policy_decision_ignore)(decision);
        }
        1
    }
}

/// `create` (the page's `window.open`): never a new web view; reported. Links that target a
/// new window are refused earlier, in `decide-policy`.
unsafe extern "C" fn on_create(_view: Ptr, action: Ptr, data: Ptr) -> Ptr {
    // SAFETY: the arguments of `create`, on the UI thread.
    unsafe {
        if let Some(inner) = back(data) {
            let api = inner.api;
            let request = (api.webkit_navigation_action_get_request)(action);
            let url = text((api.webkit_uri_request_get_uri)(request)).unwrap_or_default();
            inner.hub.block(url, BlockReason::NewWindow);
        }
    }
    std::ptr::null_mut()
}

unsafe extern "C" fn on_load(view: Ptr, event: c_int, data: Ptr) {
    // SAFETY: the arguments of `load-changed`, on the UI thread.
    unsafe {
        let Some(inner) = back(data) else { return };
        let api = inner.api;
        match event {
            LOAD_STARTED => inner.hub.set_loading(true),
            LOAD_FINISHED => {
                inner.hub.set_loading(false);
                if let Some(url) = text((api.webkit_web_view_get_uri)(view)) {
                    inner.hub.shown(url);
                }
            }
            _ => {}
        }
        inner.hub.set_history(
            (api.webkit_web_view_can_go_back)(view) != 0,
            (api.webkit_web_view_can_go_forward)(view) != 0,
        );
    }
}

unsafe extern "C" fn on_title(view: Ptr, _spec: Ptr, data: Ptr) {
    // SAFETY: the arguments of `notify::title`, on the UI thread.
    unsafe {
        if let Some(inner) = back(data) {
            inner.hub.set_title(text((inner.api.webkit_web_view_get_title)(view)).unwrap_or_default());
        }
    }
}

unsafe extern "C" fn on_uri(view: Ptr, _spec: Ptr, data: Ptr) {
    // SAFETY: the arguments of `notify::uri`, on the UI thread.
    unsafe {
        if let Some(inner) = back(data) {
            inner.hub.set_url(text((inner.api.webkit_web_view_get_uri)(view)).unwrap_or_default());
        }
    }
}

unsafe extern "C" fn on_permission(view: Ptr, request: Ptr, data: Ptr) -> c_int {
    // SAFETY: the arguments of `permission-request`, on the UI thread.
    unsafe {
        let Some(inner) = back(data) else { return 0 };
        (inner.api.webkit_permission_request_deny)(request);
        let url = text((inner.api.webkit_web_view_get_uri)(view)).unwrap_or_default();
        inner.hub.block(url, BlockReason::Permission);
        1
    }
}

unsafe extern "C" fn on_crash(_view: Ptr, reason: c_int, data: Ptr) {
    // SAFETY: the arguments of `web-process-terminated`, on the UI thread.
    unsafe {
        if let Some(inner) = back(data) {
            inner.hub.set_status(WebViewStatus::Failed(format!("the web process stopped (reason {reason})")));
        }
    }
}

unsafe extern "C" fn on_message(_manager: Ptr, result: Ptr, data: Ptr) {
    // SAFETY: the arguments of `script-message-received::remedy`, on the UI thread; the
    // page posts `JSON.stringify(value)`, a string.
    unsafe {
        let Some(inner) = back(data) else { return };
        let api = inner.api;
        let value = (api.webkit_javascript_result_get_js_value)(result);
        let s = (api.jsc_value_to_string)(value);
        if let Some(json) = text(s) {
            inner.hub.message(&json);
        }
        if !s.is_null() {
            (api.g_free)(s.cast());
        }
    }
}

unsafe extern "C" fn on_download(_context: Ptr, download: Ptr, data: Ptr) {
    // SAFETY: the arguments of the context's `download-started`, on the UI thread.
    unsafe {
        let Some(inner) = back(data) else { return };
        let api = inner.api;
        let request = (api.webkit_download_get_request)(download);
        let url = text((api.webkit_uri_request_get_uri)(request)).unwrap_or_default();
        if !inner.hub.download(url, None) {
            (api.webkit_download_cancel)(download);
        }
    }
}

/// Services GTK while `inner` lives.
fn pump(ui: WeakUi, inner: Weak<Inner>) {
    let Some(live) = inner.upgrade().filter(|i| !i.closed.get()) else { return };
    let api = live.api;
    // A busy page keeps the context busy: one interval's worth, then the app's own turn.
    let started = std::time::Instant::now();
    // SAFETY: GLib's default context, on the thread GTK runs on; never blocks.
    unsafe {
        while (api.g_main_context_pending)(std::ptr::null_mut()) != 0 && started.elapsed() < PUMP_INTERVAL {
            (api.g_main_context_iteration)(std::ptr::null_mut(), 0);
        }
    }
    drop(live);
    if let Some(u) = ui.upgrade() {
        let again = ui.clone();
        u.after(PUMP_INTERVAL, move || pump(again, inner));
    }
}

impl Native {
    pub(crate) fn new(handles: &dyn WindowHandles, hub: Rc<Hub>, ui: WeakUi) -> Result<Native, String> {
        let xid: c_ulong = match handles.window_handle().map_err(|e| e.to_string())?.as_raw() {
            RawWindowHandle::Xlib(h) => h.window,
            RawWindowHandle::Xcb(h) => c_ulong::from(h.window.get()),
            RawWindowHandle::Wayland(_) => {
                return Err("web views need X11: under Wayland, WebKitGTK cannot place its view inside \
                            the app's window (run the app under X11 or XWayland)"
                    .into());
            }
            _ => return Err("the window is neither an X11 nor a Wayland window".into()),
        };
        let api = Api::get()?;
        init_gtk(api)?;
        let data_dir = hub.data_dir().map(|d| cstring(&d.to_string_lossy()));
        let devtools = hub.devtools();
        // SAFETY: GTK and WebKit calls on the thread GTK was initialized on; every object
        // made here is owned by the popup window, destroyed in `close`.
        let inner = unsafe {
            let display = (api.gdk_display_get_default)();
            if display.is_null() {
                return Err("GTK has no X11 display".into());
            }
            let parent = (api.gdk_x11_window_foreign_new_for_display)(display, xid);
            if parent.is_null() {
                return Err("GDK could not adopt the app's X11 window".into());
            }
            let context = match &data_dir {
                Some(dir) => {
                    let manager = (api.webkit_website_data_manager_new)(
                        c"base-data-directory".as_ptr(),
                        dir.as_ptr(),
                        c"base-cache-directory".as_ptr(),
                        dir.as_ptr(),
                        std::ptr::null::<c_char>(),
                    );
                    let c = (api.webkit_web_context_new_with_website_data_manager)(manager);
                    (api.g_object_unref)(manager);
                    c
                }
                None => (api.webkit_web_context_new)(),
            };
            let manager = (api.webkit_user_content_manager_new)();
            let view = (api.g_object_new)(
                (api.webkit_web_view_get_type)(),
                c"web-context".as_ptr(),
                context,
                c"user-content-manager".as_ptr(),
                manager,
                std::ptr::null::<c_char>(),
            );
            if view.is_null() {
                (api.g_object_unref)(manager);
                (api.g_object_unref)(context);
                return Err("WebKitGTK could not make a web view".into());
            }
            let settings = (api.webkit_web_view_get_settings)(view);
            (api.webkit_settings_set_enable_developer_extras)(settings, c_int::from(devtools));
            // Let `window.open` reach `create`, which refuses and reports it, rather than
            // WebKit's popup blocker dropping it unseen.
            (api.webkit_settings_set_javascript_can_open_windows_automatically)(settings, 1);
            (api.webkit_user_content_manager_register_script_message_handler)(manager, c"remedy".as_ptr());
            let script = cstring(&crate::embed_webview::bridge_script(
                "window.webkit.messageHandlers.remedy.postMessage(JSON.stringify(value));",
                "Object.defineProperty(window, '__remedyDeliver', { value: deliver, enumerable: false, \
                 configurable: false, writable: false });",
            ));
            let user_script = (api.webkit_user_script_new)(
                script.as_ptr(),
                INJECT_TOP_FRAME,
                AT_DOCUMENT_START,
                std::ptr::null(),
                std::ptr::null(),
            );
            (api.webkit_user_content_manager_add_script)(manager, user_script);
            (api.webkit_user_script_unref)(user_script);
            let window = (api.gtk_window_new)(GTK_WINDOW_POPUP);
            let fixed = (api.gtk_fixed_new)();
            (api.gtk_container_add)(window, fixed);
            (api.gtk_container_add)(fixed, view);
            (api.gtk_widget_realize)(window);
            let gdk = (api.gtk_widget_get_window)(window);
            if gdk.is_null() {
                (api.gtk_widget_destroy)(window);
                (api.g_object_unref)(manager);
                (api.g_object_unref)(context);
                return Err("GTK could not make the web view's window".into());
            }
            (api.gdk_window_reparent)(gdk, parent, 0, 0);
            let inner = Rc::new(Inner {
                hub,
                api,
                window,
                fixed,
                view,
                layout: RefCell::new(None),
                closed: std::cell::Cell::new(false),
            });
            connect(api, view, c"decide-policy", on_decide as GCallback, &inner);
            connect(api, view, c"create", on_create as GCallback, &inner);
            connect(api, view, c"load-changed", on_load as GCallback, &inner);
            connect(api, view, c"notify::title", on_title as GCallback, &inner);
            connect(api, view, c"notify::uri", on_uri as GCallback, &inner);
            connect(api, view, c"permission-request", on_permission as GCallback, &inner);
            connect(api, view, c"web-process-terminated", on_crash as GCallback, &inner);
            connect(api, manager, c"script-message-received::remedy", on_message as GCallback, &inner);
            connect(api, context, c"download-started", on_download as GCallback, &inner);
            // The view holds its context and content manager now.
            (api.g_object_unref)(manager);
            (api.g_object_unref)(context);
            inner
        };
        inner.hub.set_status(WebViewStatus::Ready);
        pump(ui, Rc::downgrade(&inner));
        Ok(Native(inner))
    }

    pub(crate) fn command(&self, c: Command) {
        let i = &self.0;
        if i.closed.get() {
            i.hub.abandon(&c);
            return;
        }
        let api = i.api;
        // SAFETY: calls on the live web view, on its thread.
        unsafe {
            match c {
                Command::Navigate(url) => {
                    i.hub.app_navigates(Expected::Url(url.clone()));
                    (api.webkit_web_view_load_uri)(i.view, cstring(&url).as_ptr());
                }
                Command::Html(html) => {
                    i.hub.app_navigates(Expected::Html);
                    (api.webkit_web_view_load_html)(i.view, cstring(&html).as_ptr(), std::ptr::null());
                }
                Command::Back => {
                    if (api.webkit_web_view_can_go_back)(i.view) != 0 {
                        i.hub.app_navigates(Expected::History);
                        (api.webkit_web_view_go_back)(i.view);
                    }
                }
                Command::Forward => {
                    if (api.webkit_web_view_can_go_forward)(i.view) != 0 {
                        i.hub.app_navigates(Expected::History);
                        (api.webkit_web_view_go_forward)(i.view);
                    }
                }
                Command::Reload => {
                    i.hub.app_navigates(Expected::History);
                    (api.webkit_web_view_reload)(i.view);
                }
                Command::Stop => (api.webkit_web_view_stop_loading)(i.view),
                Command::Post(json) => {
                    // JSON text is a JavaScript expression: nothing but the value crosses.
                    let script =
                        cstring(&format!("window.__remedyDeliver && window.__remedyDeliver({json});"));
                    (api.webkit_web_view_run_javascript)(
                        i.view,
                        script.as_ptr(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                        std::ptr::null_mut(),
                    );
                }
                Command::Focus => (api.gtk_widget_grab_focus)(i.view),
                Command::Eval { id, script } => {
                    let wrapped = cstring(&json_eval(&script));
                    let ctx = Box::into_raw(Box::new(ScriptCtx { hub: Rc::clone(&i.hub), id }));
                    (api.webkit_web_view_run_javascript)(
                        i.view,
                        wrapped.as_ptr(),
                        std::ptr::null_mut(),
                        on_javascript as GCallback,
                        ctx.cast::<c_void>(),
                    );
                }
                Command::UserAgent(ua) => {
                    let settings = (api.webkit_web_view_get_settings)(i.view);
                    if ua.is_empty() {
                        // NULL restores WebKit's own agent.
                        (api.webkit_settings_set_user_agent)(settings, std::ptr::null());
                    } else {
                        let agent = cstring(&ua);
                        (api.webkit_settings_set_user_agent)(settings, agent.as_ptr());
                    }
                }
                Command::Devtools { id, .. } => {
                    i.hub
                        .finish_script(id, Err("the DevTools protocol is not available in WebKitGTK".into()));
                }
            }
        }
    }

    pub(crate) fn place(&self, p: Option<&Placement>) {
        let i = &self.0;
        if i.closed.get() {
            return;
        }
        let layout = Layout::of(p);
        let api = i.api;
        // SAFETY: calls on the live popup window and its children, on their thread.
        unsafe {
            let gdk = (api.gtk_widget_get_window)(i.window);
            match &layout {
                None => (api.gtk_widget_hide)(i.window),
                Some(l) => {
                    let (w, h) = (l.frame.width(), l.frame.height());
                    (api.gtk_window_resize)(i.window, w, h);
                    (api.gtk_fixed_move)(i.fixed, i.view, l.content.left, l.content.top);
                    (api.gtk_widget_set_size_request)(i.view, l.content.width(), l.content.height());
                    (api.gtk_widget_show_all)(i.window);
                    if !gdk.is_null() {
                        (api.gdk_window_move_resize)(gdk, l.frame.left, l.frame.top, w, h);
                        if l.holes.is_empty() {
                            (api.gdk_window_shape_combine_region)(gdk, std::ptr::null(), 0, 0);
                        } else {
                            let region = (api.cairo_region_create_rectangle)(&CairoRect {
                                x: 0,
                                y: 0,
                                width: w,
                                height: h,
                            });
                            for hole in &l.holes {
                                let r = CairoRect {
                                    x: hole.left,
                                    y: hole.top,
                                    width: hole.width(),
                                    height: hole.height(),
                                };
                                (api.cairo_region_subtract_rectangle)(region, &r);
                            }
                            (api.gdk_window_shape_combine_region)(gdk, region, 0, 0);
                            (api.cairo_region_destroy)(region);
                        }
                    }
                }
            }
        }
        *i.layout.borrow_mut() = layout;
    }

    pub(crate) fn native_view(&self) -> Option<NativeView> {
        let i = &self.0;
        if i.closed.get() {
            return None;
        }
        let api = i.api;
        let content = i.layout.borrow().as_ref().map(|l| l.content);
        // SAFETY: calls on the live popup window, on its thread.
        unsafe {
            let gdk = (api.gtk_widget_get_window)(i.window);
            if gdk.is_null() {
                return None;
            }
            let (mut x, mut y, mut w, mut h) = (0, 0, 0, 0);
            (api.gdk_window_get_geometry)(gdk, &mut x, &mut y, &mut w, &mut h);
            let frame = PxRect { left: x, top: y, right: x.saturating_add(w), bottom: y.saturating_add(h) };
            let content = content.map_or(frame, |c| PxRect {
                left: x.saturating_add(c.left),
                top: y.saturating_add(c.top),
                right: x.saturating_add(c.right),
                bottom: y.saturating_add(c.bottom),
            });
            Some(NativeView {
                frame: frame.to_rect(),
                content: content.to_rect(),
                visible: (api.gdk_window_is_visible)(gdk) != 0,
            })
        }
    }

    pub(crate) fn close(self) {
        let i = &self.0;
        if i.closed.replace(true) {
            return;
        }
        i.hub.abandon_pending();
        // SAFETY: destroying our own popup window destroys the view in it; the signal data
        // is freed by GLib as the handlers go.
        unsafe { (i.api.gtk_widget_destroy)(i.window) };
    }
}

/// The hub and id for one `webkit_web_view_run_javascript` completion.
struct ScriptCtx {
    hub: Rc<Hub>,
    id: u64,
}

/// `GAsyncReadyCallback` for [`json_eval`]. Frees `data`.
///
/// # Safety
/// `data` is the `Box<ScriptCtx>` passed to `webkit_web_view_run_javascript`. `source` and
/// `result` are the live web view and its async result, on the GTK thread.
unsafe extern "C" fn on_javascript(source: Ptr, result: Ptr, data: Ptr) {
    // SAFETY: `data` was `Box::into_raw` of a `ScriptCtx` and this is the one callback.
    let ctx = unsafe { Box::from_raw(data.cast::<ScriptCtx>()) };
    let Ok(api) = Api::get() else {
        ctx.hub.finish_script(ctx.id, Err("WebKitGTK is not loaded".into()));
        return;
    };
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: `source` and `result` come from WebKit for this call.
    let js = unsafe { (api.webkit_web_view_run_javascript_finish)(source, result, &mut error) };
    if !error.is_null() {
        // SAFETY: a GError WebKit allocated; its message is a C string.
        let msg = unsafe {
            if (*error).message.is_null() {
                "WebKit script failed".into()
            } else {
                CStr::from_ptr((*error).message).to_string_lossy().into_owned()
            }
        };
        unsafe { (api.g_error_free)(error) };
        ctx.hub.finish_script(ctx.id, Err(msg));
        return;
    }
    if js.is_null() {
        ctx.hub.finish_script(ctx.id, Err("WebKit returned no script result".into()));
        return;
    }
    // SAFETY: `js` is a WebKitJavascriptResult. Its JSC value is borrowed until unref.
    // `jsc_value_to_string` returns a new string.
    let text = unsafe {
        let value = (api.webkit_javascript_result_get_js_value)(js);
        let raw = (api.jsc_value_to_string)(value);
        let text = if raw.is_null() {
            String::new()
        } else {
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            (api.g_free)(raw.cast());
            text
        };
        (api.webkit_javascript_result_unref)(js);
        text
    };
    ctx.hub.finish_script(ctx.id, Ok(text));
}

// The signal handlers' signatures, checked against GLib's untyped callback slot.
const _: [unsafe extern "C" fn(Ptr, Ptr, c_int, Ptr) -> c_int; 1] = [on_decide];
const _: [unsafe extern "C" fn(Ptr, c_int, Ptr); 2] = [on_load, on_crash];
const _: [unsafe extern "C" fn(Ptr, Ptr, Ptr); 4] = [on_title, on_uri, on_message, on_download];
const _: [unsafe extern "C" fn(Ptr, Ptr, Ptr) -> c_int; 1] = [on_permission];
const _: [unsafe extern "C" fn(Ptr, Ptr, Ptr) -> Ptr; 1] = [on_create];
const _: [unsafe extern "C" fn(Ptr, Ptr, Ptr); 1] = [on_javascript];
