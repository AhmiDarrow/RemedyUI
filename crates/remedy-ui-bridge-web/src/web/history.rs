//! The page's session history: the address's fragment as the app's route.

use std::cell::RefCell;

use remedy_ui_host::HostEvent;
use wasm_bindgen::JsValue;

use super::dom::Listener;

thread_local! {
    static POPSTATE: RefCell<Option<Listener>> = const { RefCell::new(None) };
}

/// The address's fragment, without the `#`.
pub(crate) fn fragment() -> String {
    let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
    hash.strip_prefix('#').unwrap_or(&hash).to_owned()
}

/// The index the current history entry was given, if this app gave it one.
pub(crate) fn index() -> Option<u32> {
    let state = web_sys::window()?.history().ok()?.state().ok()?;
    state.as_f64().filter(|i| *i >= 0.0).map(|i| i as u32)
}

/// Adds (or, with `replace`, rewrites) the current history entry: address `#fragment`.
pub(crate) fn write(fragment: &str, index: u32, replace: bool) {
    let Some(h) = web_sys::window().and_then(|w| w.history().ok()) else { return };
    let url = format!("#{fragment}");
    let state = JsValue::from_f64(f64::from(index));
    let _ = if replace {
        h.replace_state_with_url(&state, "", Some(&url))
    } else {
        h.push_state_with_url(&state, "", Some(&url))
    };
}

/// Moves `delta` entries through the history (the browser's back and forward).
pub(crate) fn go(delta: i32) {
    if let Some(h) = web_sys::window().and_then(|w| w.history().ok()) {
        let _ = h.go_with_delta(delta);
    }
}

/// Calls `f(fragment, index)` whenever the person moves through the history or edits the
/// fragment (popstate fires for both). A second call replaces `f`.
pub(crate) fn listen(f: impl Fn(String, Option<u32>) + 'static) {
    let Some(w) = web_sys::window() else { return };
    let l = Listener::new(&w, "popstate", move |_| {
        f(fragment(), index());
        super::post(HostEvent::Wake);
    });
    POPSTATE.with(|p| *p.borrow_mut() = Some(l));
}

/// Opens `#fragment` of this page in a new tab.
pub(crate) fn open_in_new_tab(fragment: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.open_with_url_and_target(&format!("#{fragment}"), "_blank");
    }
}

/// Reloads the page.
pub(crate) fn reload() {
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}
