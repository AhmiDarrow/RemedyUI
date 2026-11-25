//! The page's session history: the address's fragment (or, when the app asks, its path) as
//! the app's route.
//!
//! Each history entry the app writes carries `{ i, e: [[route, x, y, state], …] }` in its
//! `history.state`: its index and the app's whole list of entries with their scroll
//! positions, so a reload (which keeps the history and the current entry's state) can bring
//! the list back.

use std::cell::RefCell;

use js_sys::{Array, Reflect};
use remedy_ui_core::host::HostEvent;
use wasm_bindgen::{JsCast, JsValue};

use super::dom::Listener;

/// One entry: its route, its scroll position and the app's own state for it.
pub(crate) type Entry = (String, f32, f32, String);

thread_local! {
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    /// The path the app's routes live under, when they are in the address's path and not
    /// its fragment ([`use_paths`]).
    static BASE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Keeps routes in the address's path under `base` (`/app` makes `/app/settings/2`; an
/// empty base, `/settings/2`), or with `None` in its fragment again.
pub(crate) fn use_paths(base: Option<&str>) {
    BASE.with(|b| *b.borrow_mut() = base.map(|b| b.trim_end_matches('/').to_owned()));
}

fn base() -> Option<String> {
    BASE.with(|b| b.borrow().clone())
}

/// The address of `route`: `#route`, or its path under the base.
fn address(route: &str) -> String {
    match base() {
        Some(base) => format!("{base}/{}", route.trim_start_matches('/')),
        None => format!("#{route}"),
    }
}

/// The route in the address: its fragment without the `#`, or with [`use_paths`] its path
/// under the base, with the query.
pub(crate) fn fragment() -> String {
    let Some(location) = web_sys::window().map(|w| w.location()) else { return String::new() };
    match base() {
        Some(base) => {
            let path = location.pathname().unwrap_or_default();
            let rest = path.strip_prefix(base.as_str()).unwrap_or(&path);
            format!("/{}{}", rest.trim_start_matches('/'), location.search().unwrap_or_default())
        }
        None => {
            let hash = location.hash().unwrap_or_default();
            hash.strip_prefix('#').unwrap_or(&hash).to_owned()
        }
    }
}

fn state() -> Option<JsValue> {
    web_sys::window()?.history().ok()?.state().ok().filter(JsValue::is_object)
}

/// The index the current history entry was given, if this app gave it one.
pub(crate) fn index() -> Option<u32> {
    let i = Reflect::get(&state()?, &"i".into()).ok()?.as_f64()?;
    (i >= 0.0 && i.fract() == 0.0).then_some(i as u32)
}

/// The entries the current history entry carries, if this app wrote them.
pub(crate) fn entries() -> Option<Vec<Entry>> {
    let list = Reflect::get(&state()?, &"e".into()).ok()?.dyn_into::<Array>().ok()?;
    list.iter()
        .map(|e| {
            let e = e.dyn_into::<Array>().ok()?;
            let route = e.get(0).as_string()?;
            let x = e.get(1).as_f64()? as f32;
            let y = e.get(2).as_f64()? as f32;
            // Entries written before states were kept have none.
            Some((route, x, y, e.get(3).as_string().unwrap_or_default()))
        })
        .collect()
}

/// Adds (or, with `replace`, rewrites) the current history entry: address `#fragment`,
/// numbered `index`, carrying `entries`.
pub(crate) fn write(fragment: &str, index: u32, entries: &[Entry], replace: bool) {
    let Some(h) = web_sys::window().and_then(|w| w.history().ok()) else { return };
    let list = Array::new();
    for (route, x, y, state) in entries {
        let e = Array::new();
        e.push(&JsValue::from_str(route));
        e.push(&JsValue::from_f64(f64::from(*x)));
        e.push(&JsValue::from_f64(f64::from(*y)));
        e.push(&JsValue::from_str(state));
        list.push(&e);
    }
    let state = js_sys::Object::new();
    let _ = Reflect::set(&state, &"i".into(), &JsValue::from_f64(f64::from(index)));
    let _ = Reflect::set(&state, &"e".into(), &list);
    let url = address(fragment);
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

/// Calls `moved(fragment, index)` whenever the person moves through the history or edits
/// the fragment (popstate fires for both), and `hiding()` when the page is put away or
/// reloaded. A second call replaces both.
pub(crate) fn listen(moved: impl Fn(String, Option<u32>) + 'static, hiding: impl Fn() + 'static) {
    let Some(w) = web_sys::window() else { return };
    let pop = Listener::new(&w, "popstate", move |_| {
        moved(fragment(), index());
        super::post(HostEvent::Wake);
    });
    let hide = Listener::new(&w, "pagehide", move |_| hiding());
    LISTENERS.with(|l| *l.borrow_mut() = vec![pop, hide]);
}

/// Opens `#fragment` of this page in a new tab.
pub(crate) fn open_in_new_tab(fragment: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.open_with_url_and_target(&address(fragment), "_blank");
    }
}

/// Reloads the page.
pub(crate) fn reload() {
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}
