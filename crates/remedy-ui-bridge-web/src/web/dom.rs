//! Small DOM helpers: listeners that remove themselves, elements, styles.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// An event listener that is removed from its target when dropped.
pub(crate) struct Listener {
    target: web_sys::EventTarget,
    kind: &'static str,
    capture: bool,
    cb: Closure<dyn FnMut(web_sys::Event)>,
}

impl Listener {
    /// Listens for `kind` on `target`. Listeners are not passive, so handlers may cancel the
    /// browser's default action (scrolling the page, its context menu).
    pub(crate) fn new(
        target: &web_sys::EventTarget,
        kind: &'static str,
        f: impl FnMut(web_sys::Event) + 'static,
    ) -> Listener {
        Self::with_capture(target, kind, false, f)
    }

    /// As [`Listener::new`], in the capture phase when `capture` is set.
    pub(crate) fn with_capture(
        target: &web_sys::EventTarget,
        kind: &'static str,
        capture: bool,
        f: impl FnMut(web_sys::Event) + 'static,
    ) -> Listener {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(f);
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_passive(false);
        opts.set_capture(capture);
        let _ = target.add_event_listener_with_callback_and_add_event_listener_options(
            kind,
            cb.as_ref().unchecked_ref(),
            &opts,
        );
        Listener { target: target.clone(), kind, capture, cb }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback_and_bool(
            self.kind,
            self.cb.as_ref().unchecked_ref(),
            self.capture,
        );
    }
}

/// The page's document.
pub(crate) fn document() -> Option<web_sys::Document> {
    web_sys::window().and_then(|w| w.document())
}

/// A new element of `tag`.
pub(crate) fn element<T: JsCast>(tag: &str) -> Option<T> {
    document()?.create_element(tag).ok()?.dyn_into::<T>().ok()
}

/// Sets inline style properties.
pub(crate) fn style(el: &web_sys::HtmlElement, props: &[(&str, &str)]) {
    let s = el.style();
    for (k, v) in props {
        let _ = s.set_property(k, v);
    }
}

/// Sets or removes an attribute.
pub(crate) fn attr(el: &web_sys::Element, name: &str, value: Option<&str>) {
    match value {
        Some(v) => {
            if el.get_attribute(name).as_deref() != Some(v) {
                let _ = el.set_attribute(name, v);
            }
        }
        None => {
            if el.has_attribute(name) {
                let _ = el.remove_attribute(name);
            }
        }
    }
}

/// Focuses `el` without scrolling the page to it.
pub(crate) fn focus(el: &web_sys::HtmlElement) {
    let opts = web_sys::FocusOptions::new();
    opts.set_prevent_scroll(true);
    let _ = el.focus_with_options(&opts);
}

/// The element that has keyboard focus.
pub(crate) fn active_element() -> Option<web_sys::Element> {
    document()?.active_element()
}

/// A property of a JavaScript object, where `web-sys` has no binding for it (newer or
/// browser-specific APIs).
pub(crate) fn get(obj: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}
