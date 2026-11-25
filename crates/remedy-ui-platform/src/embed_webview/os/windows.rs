//! WebView2 on Windows.
//!
//! The page lives in a WebView2 controller hosted in a child window of the app's window (the
//! container). The container is moved to the part of the element that shows; the controller
//! inside it is placed at the element's whole box, so the container's edges clip the page as
//! the element is clipped. Overlay boxes are cut out of the container's window region. The
//! WebView2 loader is linked statically (`WebView2LoaderStatic.lib` from `webview2-com-sys`),
//! so apps ship no loader DLL; it finds the Evergreen runtime the system keeps updated.
//!
//! Everything here runs on the UI thread, which must pump window messages (the winit loop
//! does): WebView2 delivers its callbacks through them.

use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::{Rc, Weak};
use std::sync::OnceLock;

use remedy_ui_core::host::WindowHandles;
use remedy_ui_core::host::raw_window_handle::RawWindowHandle;
use remedy_ui_engine::view::{Placement, WeakUi};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CallDevToolsProtocolMethodCompletedHandler,
    CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DocumentTitleChangedEventHandler,
    DownloadStartingEventHandler, ExecuteScriptCompletedHandler, HistoryChangedEventHandler,
    LaunchingExternalUriSchemeEventHandler, NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, PermissionRequestedEventHandler, ProcessFailedEventHandler,
    SourceChangedEventHandler, WebMessageReceivedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, MapWindowPoints, RGN_DIFF, SetWindowRgn,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowRect, IsWindowVisible, RegisterClassExW,
    SW_HIDE, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE,
    WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
};
use windows::core::{BOOL, HRESULT, HSTRING, Interface, PCWSTR, PWSTR, w};

use super::{Layout, PxRect};
use crate::embed_webview::{BlockReason, Command, Expected, Hub, NativeView, WebViewStatus};

#[link(name = "WebView2LoaderStatic", kind = "static")]
unsafe extern "system" {
    /// The WebView2 loader's entry point, from the statically linked loader.
    fn CreateCoreWebView2EnvironmentWithOptions(
        browser_folder: PCWSTR,
        user_data_folder: PCWSTR,
        options: *mut c_void,
        handler: *mut c_void,
    ) -> HRESULT;
}

/// The loader's answer when no WebView2 runtime is installed
/// (`HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`).
const NO_RUNTIME: HRESULT = HRESULT(0x8007_0002_u32 as i32);

enum Phase {
    /// Being made; what the app asked meanwhile, in order.
    Starting(Vec<Command>),
    Ready {
        controller: ICoreWebView2Controller,
        webview: ICoreWebView2,
    },
    /// Failed to start, or closed.
    Gone,
}

struct Inner {
    hub: Rc<Hub>,
    /// The child window the controller lives in.
    container: HWND,
    parent: HWND,
    phase: RefCell<Phase>,
    layout: RefCell<Option<Layout>>,
}

/// A WebView2 web view in a child window of the app's window.
pub(crate) struct Native(Rc<Inner>);

unsafe extern "system" fn container_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the arguments come from the system for this window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Registers the container's window class once per process.
fn container_class() -> Result<PCWSTR, String> {
    static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    let name = w!("RemedyUIWebViewHost");
    CLASS
        .get_or_init(|| {
            // SAFETY: plain Win32 calls with a class description that outlives them.
            unsafe {
                let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
                let class = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(container_proc),
                    hInstance: instance.into(),
                    lpszClassName: name,
                    ..Default::default()
                };
                if RegisterClassExW(&class) == 0 {
                    return Err(windows::core::Error::from_thread().to_string());
                }
            }
            Ok(())
        })
        .clone()
        .map(|()| name)
}

impl Native {
    /// `_ui` is unused here: WebView2 runs on the window messages the app's loop pumps.
    pub(crate) fn new(handles: &dyn WindowHandles, hub: Rc<Hub>, _ui: WeakUi) -> Result<Native, String> {
        let raw = handles.window_handle().map_err(|e| e.to_string())?.as_raw();
        let RawWindowHandle::Win32(h) = raw else {
            return Err("the window is not a Win32 window".into());
        };
        let parent = HWND(h.hwnd.get() as *mut c_void);
        let class = container_class()?;
        // SAFETY: COM on this (UI) thread; already initialized is fine (S_FALSE), and a
        // different apartment already chosen by the app is reported by the calls below.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // SAFETY: a child window of the app's live window, in a class registered above.
        let container = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                0,
                0,
                1,
                1,
                Some(parent),
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            )
        }
        .map_err(|e| format!("the web view's window could not be made: {e}"))?;
        let inner = Rc::new(Inner {
            hub,
            container,
            parent,
            phase: RefCell::new(Phase::Starting(Vec::new())),
            layout: RefCell::new(None),
        });
        if let Err(e) = start(&inner) {
            inner.fail(e);
        }
        Ok(Native(inner))
    }

    pub(crate) fn command(&self, c: Command) {
        // Starting still queues, so a script callback stays until the view is ready.
        // Gone reports the callback now; dropping the command would leak it.
        let ready = {
            let mut phase = self.0.phase.borrow_mut();
            match &mut *phase {
                Phase::Starting(queue) => {
                    queue.push(c);
                    return;
                }
                Phase::Ready { controller, webview } => Some((controller.clone(), webview.clone())),
                Phase::Gone => None,
            }
        };
        if let Some((controller, webview)) = ready {
            run(&self.0.hub, &controller, &webview, c);
        } else {
            self.0.hub.abandon(&c);
        }
    }

    pub(crate) fn place(&self, p: Option<&Placement>) {
        let layout = Layout::of(p);
        *self.0.layout.borrow_mut() = layout;
        self.0.apply();
    }

    pub(crate) fn native_view(&self) -> Option<NativeView> {
        let (controller, frame, visible) = {
            let phase = self.0.phase.borrow();
            let Phase::Ready { controller, .. } = &*phase else { return None };
            let mut r = RECT::default();
            // SAFETY: our own live container and its parent.
            unsafe {
                GetWindowRect(self.0.container, &mut r).ok()?;
                let mut pts = [POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom }];
                MapWindowPoints(None, Some(self.0.parent), &mut pts);
                let [a, b] = pts;
                (
                    controller.clone(),
                    PxRect { left: a.x, top: a.y, right: b.x, bottom: b.y },
                    IsWindowVisible(self.0.container).as_bool(),
                )
            }
        };
        let mut c = RECT::default();
        // SAFETY: a live controller.
        unsafe { controller.Bounds(&mut c) }.ok()?;
        let content = PxRect {
            left: frame.left.saturating_add(c.left),
            top: frame.top.saturating_add(c.top),
            right: frame.left.saturating_add(c.right),
            bottom: frame.top.saturating_add(c.bottom),
        };
        Some(NativeView { frame: frame.to_rect(), content: content.to_rect(), visible })
    }

    pub(crate) fn close(self) {
        let old = self.0.gone();
        if let Phase::Ready { controller, .. } = old {
            // SAFETY: a live controller; closing ends its browser processes' part for us.
            let _ = unsafe { controller.Close() };
        }
        // SAFETY: our own container window.
        let _ = unsafe { DestroyWindow(self.0.container) };
    }
}

impl Inner {
    /// Marks the view gone and reports every script still waiting, including commands that
    /// were queued while it started.
    fn gone(&self) -> Phase {
        let old = std::mem::replace(&mut *self.phase.borrow_mut(), Phase::Gone);
        if let Phase::Starting(ref queue) = old {
            for c in queue {
                self.hub.abandon(c);
            }
        }
        self.hub.abandon_pending();
        old
    }

    fn fail(&self, e: String) {
        let _ = self.gone();
        self.hub.set_status(WebViewStatus::Failed(e));
    }

    /// Moves, cuts and shows (or hides) the container for the last placement.
    fn apply(&self) {
        // Copied out: the calls below send window messages, and what they set off must find
        // neither cell borrowed.
        let controller = {
            let phase = self.phase.borrow();
            let Phase::Ready { controller, .. } = &*phase else { return };
            controller.clone()
        };
        let layout = self.layout.borrow().clone();
        // SAFETY: our own container window and a live controller.
        unsafe {
            match &layout {
                None => {
                    let _ = controller.SetIsVisible(false);
                    let _ = ShowWindow(self.container, SW_HIDE);
                }
                Some(l) => {
                    let (w, h) = (l.frame.width(), l.frame.height());
                    let _ = SetWindowPos(
                        self.container,
                        None,
                        l.frame.left,
                        l.frame.top,
                        w,
                        h,
                        SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
                    );
                    let c = l.content;
                    let _ = controller.SetBounds(RECT {
                        left: c.left,
                        top: c.top,
                        right: c.right,
                        bottom: c.bottom,
                    });
                    if l.holes.is_empty() {
                        let _ = SetWindowRgn(self.container, None, true);
                    } else {
                        // The container's box with the overlays cut out; the system owns
                        // the region once it is set.
                        let region = CreateRectRgn(0, 0, w, h);
                        for hole in &l.holes {
                            let cut = CreateRectRgn(hole.left, hole.top, hole.right, hole.bottom);
                            CombineRgn(Some(region), Some(region), Some(cut), RGN_DIFF);
                            let _ = DeleteObject(HGDIOBJ(cut.0));
                        }
                        if SetWindowRgn(self.container, Some(region), true) == 0 {
                            let _ = DeleteObject(HGDIOBJ(region.0));
                        }
                    }
                    let _ = controller.SetIsVisible(true);
                    let _ = controller.NotifyParentWindowPositionChanged();
                }
            }
        }
    }
}

/// A string out of a WebView2 getter.
fn get(f: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> Option<String> {
    let mut p = PWSTR::null();
    f(&mut p).ok()?;
    Some(take_pwstr(p))
}

fn flag(f: impl FnOnce(*mut BOOL) -> windows::core::Result<()>) -> bool {
    let mut b = BOOL::default();
    f(&mut b).is_ok() && b.as_bool()
}

/// Runs one of the app's commands on a ready view.
fn run(hub: &Rc<Hub>, controller: &ICoreWebView2Controller, webview: &ICoreWebView2, c: Command) {
    // SAFETY: calls on live WebView2 objects, on their thread.
    unsafe {
        let navigation = |e: Expected, call: &dyn Fn() -> windows::core::Result<()>| {
            let n = hub.app_navigates(e);
            if call().is_err() {
                hub.forget(n);
            }
        };
        match c {
            Command::Navigate(url) => {
                navigation(Expected::Url(url.clone()), &|| webview.Navigate(&HSTRING::from(url.as_str())))
            }
            Command::Html(html) => {
                navigation(Expected::Html, &|| webview.NavigateToString(&HSTRING::from(html.as_str())))
            }
            Command::Back => {
                if flag(|b| webview.CanGoBack(b)) {
                    navigation(Expected::History, &|| webview.GoBack());
                }
            }
            Command::Forward => {
                if flag(|b| webview.CanGoForward(b)) {
                    navigation(Expected::History, &|| webview.GoForward());
                }
            }
            Command::Reload => navigation(Expected::History, &|| webview.Reload()),
            Command::Stop => {
                let _ = webview.Stop();
            }
            Command::Post(json) => {
                let _ = webview.PostWebMessageAsJson(&HSTRING::from(json.as_str()));
            }
            Command::Focus => {
                let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
            Command::Eval { id, script } => {
                let waiting = Rc::clone(hub);
                let handler = ExecuteScriptCompletedHandler::create(Box::new(move |result, json| {
                    let out = match result {
                        Ok(()) => Ok(json),
                        Err(e) => Err(e.to_string()),
                    };
                    waiting.finish_script(id, out);
                    Ok(())
                }));
                if let Err(e) = webview.ExecuteScript(&HSTRING::from(script.as_str()), &handler) {
                    hub.finish_script(id, Err(e.to_string()));
                }
            }
            Command::UserAgent(ua) => {
                // Settings2 is on every Evergreen runtime this loader talks to. An empty
                // agent restores Edge's own, per the WebView2 contract.
                if let Ok(settings) = webview.Settings()
                    && let Ok(settings) = settings.cast::<ICoreWebView2Settings2>()
                {
                    let _ = settings.SetUserAgent(&HSTRING::from(ua.as_str()));
                }
            }
            Command::Devtools { id, method, params } => {
                // The host protocol, not the inspector. AreDevToolsEnabled only gates the
                // context menu; some Input.* methods still want it on, which the app sets
                // with WebViewOptions::devtools.
                let waiting = Rc::clone(hub);
                let handler =
                    CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json| {
                        let out = match result {
                            Ok(()) => Ok(json),
                            Err(e) => Err(e.to_string()),
                        };
                        waiting.finish_script(id, out);
                        Ok(())
                    }));
                if let Err(e) = webview.CallDevToolsProtocolMethod(
                    &HSTRING::from(method.as_str()),
                    &HSTRING::from(params.as_str()),
                    &handler,
                ) {
                    hub.finish_script(id, Err(e.to_string()));
                }
            }
        }
    }
}

/// Starts making the environment, then the controller, asynchronously.
fn start(inner: &Rc<Inner>) -> Result<(), String> {
    let weak = Rc::downgrade(inner);
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
        let Some(inner) = weak.upgrade() else { return Ok(()) };
        match (result, env) {
            (Ok(()), Some(env)) => {
                if let Err(e) = make_controller(&inner, &env) {
                    inner.fail(e);
                }
            }
            (Err(e), _) if e.code() == NO_RUNTIME => {
                let _ = inner.gone();
                inner.hub.set_status(WebViewStatus::Unsupported(
                    "the Microsoft Edge WebView2 runtime is not installed".into(),
                ));
            }
            (Err(e), _) => inner.fail(format!("WebView2 could not start: {e}")),
            (Ok(()), None) => inner.fail("WebView2 gave no environment".into()),
        }
        Ok(())
    }));
    let options: ICoreWebView2EnvironmentOptions = CoreWebView2EnvironmentOptions::default().into();
    let data = inner.hub.data_dir().map(|d| HSTRING::from(d.as_os_str()));
    let data_ptr = data.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr()));
    // SAFETY: the loader's documented entry point; it holds its own references to the
    // options and the handler, and the strings outlive the call.
    let hr = unsafe {
        CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), data_ptr, options.as_raw(), handler.as_raw())
    };
    if hr == NO_RUNTIME {
        let _ = inner.gone();
        inner.hub.set_status(WebViewStatus::Unsupported(
            "the Microsoft Edge WebView2 runtime is not installed".into(),
        ));
        return Ok(());
    }
    hr.ok().map_err(|e| format!("WebView2 could not start: {e}"))
}

fn make_controller(inner: &Rc<Inner>, env: &ICoreWebView2Environment) -> Result<(), String> {
    let weak = Rc::downgrade(inner);
    let handler =
        CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
            let Some(inner) = weak.upgrade() else {
                // The element went away while the controller was being made.
                if let Some(c) = controller {
                    // SAFETY: a controller nobody else holds.
                    let _ = unsafe { c.Close() };
                }
                return Ok(());
            };
            if matches!(&*inner.phase.borrow(), Phase::Gone) {
                if let Some(c) = controller {
                    // SAFETY: as above.
                    let _ = unsafe { c.Close() };
                }
                return Ok(());
            }
            match (result, controller) {
                (Ok(()), Some(c)) => {
                    if let Err(e) = configure(&inner, c) {
                        inner.fail(e);
                    }
                }
                (Err(e), _) => inner.fail(format!("WebView2 could not make its view: {e}")),
                (Ok(()), None) => inner.fail("WebView2 gave no view".into()),
            }
            Ok(())
        }));
    // SAFETY: a live environment and our own container window.
    unsafe { env.CreateCoreWebView2Controller(inner.container, &handler) }
        .map_err(|e| format!("WebView2 could not make its view: {e}"))
}

/// Locks the view down, hooks its events, and installs the message bridge; the view is
/// ready (and what the app asked meanwhile runs) once the bridge is in.
fn configure(inner: &Rc<Inner>, controller: ICoreWebView2Controller) -> Result<(), String> {
    let err = |what: &'static str| move |e: windows::core::Error| format!("WebView2 {what}: {e}");
    // SAFETY: calls on a live controller and its view, on their thread; every handler holds
    // only a weak reference back, so the view never keeps itself alive.
    unsafe {
        let webview = controller.CoreWebView2().map_err(err("gave no view"))?;
        let settings = webview.Settings().map_err(err("settings"))?;
        let devtools = inner.hub.devtools();
        settings.SetAreDevToolsEnabled(devtools).map_err(err("settings"))?;
        settings.SetAreHostObjectsAllowed(false).map_err(err("settings"))?;
        settings.SetIsWebMessageEnabled(true).map_err(err("settings"))?;
        settings.SetIsStatusBarEnabled(false).map_err(err("settings"))?;
        settings.SetIsScriptEnabled(true).map_err(err("settings"))?;
        if let Ok(s4) = settings.cast::<ICoreWebView2Settings4>() {
            let _ = s4.SetIsPasswordAutosaveEnabled(false);
            let _ = s4.SetIsGeneralAutofillEnabled(false);
        }
        hook(inner, &webview).map_err(err("events"))?;

        let script = crate::embed_webview::bridge_script(
            "window.chrome.webview.postMessage(value);",
            "if (!(window.chrome && window.chrome.webview)) return;\n  \
             window.chrome.webview.addEventListener('message', (e) => deliver(e.data));",
        );
        let weak = Rc::downgrade(inner);
        let (c2, w2) = (controller.clone(), webview.clone());
        let done =
            AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |result, _id| {
                let Some(inner) = weak.upgrade() else { return Ok(()) };
                if let Err(e) = result {
                    inner.fail(format!("WebView2 refused the message bridge: {e}"));
                    return Ok(());
                }
                let queued = {
                    let mut phase = inner.phase.borrow_mut();
                    match std::mem::replace(
                        &mut *phase,
                        Phase::Ready { controller: c2.clone(), webview: w2.clone() },
                    ) {
                        Phase::Starting(q) => q,
                        // Closed meanwhile: stay closed.
                        other => {
                            *phase = other;
                            return Ok(());
                        }
                    }
                };
                inner.hub.set_status(WebViewStatus::Ready);
                inner.apply();
                for c in queued {
                    run(&inner.hub, &c2, &w2, c);
                }
                Ok(())
            }));
        webview.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(script), &done).map_err(err("bridge"))?;
    }
    Ok(())
}

/// Hooks the view's events to the hub.
///
/// # Safety
/// `webview` is live and this is its thread.
unsafe fn hook(inner: &Rc<Inner>, webview: &ICoreWebView2) -> windows::core::Result<()> {
    let mut token = 0i64;
    let hub = |w: &Weak<Inner>| w.upgrade().map(|i| i.hub.clone());
    // SAFETY (for every call below): live arguments from WebView2, on its thread.
    unsafe {
        let w = Rc::downgrade(inner);
        webview.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let (Some(hub), Some(args)) = (hub(&w), args) else { return Ok(()) };
                let url = get(|p| args.Uri(p)).unwrap_or_default();
                let user = flag(|b| args.IsUserInitiated(b));
                let redirect = flag(|b| args.IsRedirected(b));
                // Runtimes before 1.0.2045 do not say the kind.
                let history = args.cast::<ICoreWebView2NavigationStartingEventArgs3>().ok().and_then(|a| {
                    let mut kind = COREWEBVIEW2_NAVIGATION_KIND::default();
                    a.NavigationKind(&mut kind).ok()?;
                    Some(kind != COREWEBVIEW2_NAVIGATION_KIND_NEW_DOCUMENT)
                });
                if !hub.decide(url, user, redirect, false, history) {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_FrameNavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let (Some(hub), Some(args)) = (hub(&w), args) else { return Ok(()) };
                let url = get(|p| args.Uri(p)).unwrap_or_default();
                let user = flag(|b| args.IsUserInitiated(b));
                let redirect = flag(|b| args.IsRedirected(b));
                if !hub.decide(url, user, redirect, true, None) {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
                let Some(hub) = hub(&w) else { return Ok(()) };
                hub.set_loading(false);
                if let (Some(view), Some(args)) = (sender, args)
                    && flag(|b| args.IsSuccess(b))
                    && let Some(url) = get(|p| view.Source(p))
                {
                    hub.shown(url);
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_SourceChanged(
            &SourceChangedEventHandler::create(Box::new(move |sender, _| {
                if let (Some(hub), Some(view)) = (hub(&w), sender)
                    && let Some(url) = get(|p| view.Source(p))
                {
                    hub.set_url(url);
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_DocumentTitleChanged(
            &DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
                if let (Some(hub), Some(view)) = (hub(&w), sender)
                    && let Some(t) = get(|p| view.DocumentTitle(p))
                {
                    hub.set_title(t);
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new(move |sender, _| {
                if let (Some(hub), Some(view)) = (hub(&w), sender) {
                    hub.set_history(flag(|b| view.CanGoBack(b)), flag(|b| view.CanGoForward(b)));
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                if let (Some(hub), Some(args)) = (hub(&w), args)
                    && let Some(json) = get(|p| args.WebMessageAsJson(p))
                {
                    hub.message(&json);
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                // Handled with no new window: the page's `window.open` gets nothing.
                args.SetHandled(true)?;
                if let Some(hub) = hub(&w) {
                    hub.block(get(|p| args.Uri(p)).unwrap_or_default(), BlockReason::NewWindow);
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                if let Some(hub) = hub(&w) {
                    hub.block(get(|p| args.Uri(p)).unwrap_or_default(), BlockReason::Permission);
                }
                Ok(())
            })),
            &mut token,
        )?;
        let w = Rc::downgrade(inner);
        webview.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                if let Some(hub) = hub(&w) {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    if let Some(a) = args {
                        let _ = a.ProcessFailedKind(&mut kind);
                    }
                    hub.set_status(WebViewStatus::Failed(format!(
                        "a WebView2 process stopped (kind {})",
                        kind.0
                    )));
                }
                Ok(())
            })),
            &mut token,
        )?;
        // Downloads (WebView2 1.0.902 and later): denied unless the app allows each one.
        if let Ok(v4) = webview.cast::<ICoreWebView2_4>() {
            let w = Rc::downgrade(inner);
            v4.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let url =
                        args.DownloadOperation().ok().and_then(|d| get(|p| d.Uri(p))).unwrap_or_default();
                    let path = get(|p| args.ResultFilePath(p)).map(std::path::PathBuf::from);
                    let allowed = hub(&w).is_some_and(|hub| hub.download(url, path));
                    if !allowed {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        } else {
            // A runtime this old cannot be told to deny downloads: refuse to run.
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_NOINTERFACE,
                "this WebView2 runtime is too old to deny downloads",
            ));
        }
        // Links to other programs (WebView2 1.0.1518 and later; older runtimes do not
        // launch them without asking the user).
        if let Ok(v18) = webview.cast::<ICoreWebView2_18>() {
            let w = Rc::downgrade(inner);
            v18.add_LaunchingExternalUriScheme(
                &LaunchingExternalUriSchemeEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    args.SetCancel(true)?;
                    if let Some(hub) = hub(&w) {
                        hub.block(get(|p| args.Uri(p)).unwrap_or_default(), BlockReason::ExternalScheme);
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
    }
    Ok(())
}
