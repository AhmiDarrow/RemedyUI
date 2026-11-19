//! System notifications in a browser page: the Notification API, which shows through the
//! operating system's own notifications. The browser asks the person once
//! (`Notification.requestPermission`, from [`request`]); a press brings the page forward and
//! reports [`NotificationResponse::Activated`]. Buttons on page notifications need a
//! service worker, so a page shows the title and body alone.

use std::cell::RefCell;
use std::collections::HashMap;

use js_sys::{Function, Object, Promise, Reflect};
use remedy_ui_host::{HostEvent, Notification, NotificationResponse, PermissionId, Unsupported};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use super::post;

/// A notification shown, with the callbacks wired to it (kept alive while it shows).
type Shown = (JsValue, Vec<Closure<dyn FnMut(JsValue)>>);

thread_local! {
    /// Each notification shown, by the app's id.
    static SHOWN: RefCell<HashMap<u64, Shown>> = RefCell::new(HashMap::new());
    static NEXT_REQUEST: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

fn class() -> Option<Function> {
    get(&js_sys::global(), "Notification").dyn_into().ok()
}

fn unsupported(reason: impl Into<String>) -> Unsupported {
    Unsupported::new("system notifications", reason.into())
}

/// The page's notification permission: `granted`, `denied` or `default` (not asked yet).
fn permission() -> Option<String> {
    class().and_then(|c| get(&c, "permission").as_string())
}

pub(crate) fn show(n: &Notification) -> Result<(), Unsupported> {
    let Some(class) = class() else {
        return Err(unsupported(
            "this browser has no notifications (or the page is not https: or localhost)",
        ));
    };
    match permission().as_deref() {
        Some("granted") => {}
        Some("denied") => return Err(unsupported("the person blocked notifications for this page")),
        _ => {
            return Err(unsupported("ask first with App::request_permission(Permission::Notifications, ..)"));
        }
    }
    withdraw(n.id);
    let opts = Object::new();
    let set = |k: &str, v: JsValue| {
        let _ = Reflect::set(&opts, &JsValue::from_str(k), &v);
    };
    set("body", JsValue::from_str(&n.body));
    // The same tag replaces an earlier notification instead of stacking another.
    set("tag", JsValue::from_str(&format!("remedy-{}", n.id)));
    set("silent", JsValue::from_bool(n.silent));
    let shown = Reflect::construct(&class, &js_sys::Array::of2(&JsValue::from_str(&n.title), &opts))
        .map_err(|e| unsupported(format!("{e:?}")))?;
    let id = n.id;
    let click = Closure::<dyn FnMut(JsValue)>::new(move |_| {
        if let Some(w) = web_sys::window() {
            let _ = w.focus();
        }
        post(HostEvent::Notification(id, NotificationResponse::Activated));
    });
    let close = Closure::<dyn FnMut(JsValue)>::new(move |_| {
        SHOWN.with(|s| s.borrow_mut().remove(&id));
    });
    let _ = Reflect::set(&shown, &JsValue::from_str("onclick"), click.as_ref());
    let _ = Reflect::set(&shown, &JsValue::from_str("onclose"), close.as_ref());
    SHOWN.with(|s| s.borrow_mut().insert(id, (shown, vec![click, close])));
    Ok(())
}

pub(crate) fn withdraw(id: u64) {
    let shown = SHOWN.with(|s| s.borrow_mut().remove(&id));
    if let Some((n, _callbacks)) = shown
        && let Ok(f) = get(&n, "close").dyn_into::<Function>()
    {
        let _ = Reflect::set(&n, &JsValue::from_str("onclose"), &JsValue::NULL);
        let _ = f.call0(&n);
    }
}

/// Asks the browser for leave to notify: answered at once when the person already decided,
/// otherwise the browser's prompt answers through [`HostEvent::Permission`].
pub(crate) fn request() -> Result<remedy_ui_host::PermissionStarted, Unsupported> {
    use remedy_ui_host::PermissionStarted;
    let Some(class) = class() else {
        return Ok(PermissionStarted::Finished(false));
    };
    match permission().as_deref() {
        Some("granted") => return Ok(PermissionStarted::Finished(true)),
        Some("denied") => return Ok(PermissionStarted::Finished(false)),
        _ => {}
    }
    let ask: Function =
        get(&class, "requestPermission").dyn_into().map_err(|e| unsupported(format!("{e:?}")))?;
    let promise: Promise = ask
        .call0(&class)
        .map_err(|e| unsupported(format!("{e:?}")))?
        .dyn_into()
        .map_err(|e| unsupported(format!("{e:?}")))?;
    let id = NEXT_REQUEST.with(|n| n.replace(n.get() + 1));
    wasm_bindgen_futures::spawn_local(async move {
        let answer = wasm_bindgen_futures::JsFuture::from(promise).await;
        let granted = answer.ok().and_then(|v| v.as_string()).as_deref() == Some("granted");
        post(HostEvent::Permission(PermissionId(id), granted));
    });
    Ok(PermissionStarted::Pending(PermissionId(id)))
}
