//! One instance across the browser's tabs: the first tab of an app holds a Web Lock named
//! after it and runs; a later tab finds the lock taken, hands its launch (its address) to the
//! first over a `BroadcastChannel`, and does not start the app.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::RunError;

fn channel_name(app: &str) -> String {
    let id: String = app.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("remedyui-instance-{id}")
}

/// Claims the instance and then starts the app with `start`, or hands `args` to the tab that
/// holds it. `deliver` receives later tabs' launches. Returns at once; the browser answers the
/// lock request later.
pub(crate) fn claim(
    app: &str,
    args: Vec<String>,
    start: Box<dyn FnOnce()>,
    deliver: Box<dyn Fn(Vec<String>)>,
) -> Result<(), RunError> {
    let win = super::web_window()?;
    let locks = super::dom::get(win.navigator().unchecked_ref(), "locks");
    let request = super::dom::get(&locks, "request").dyn_into::<js_sys::Function>().map_err(|_| {
        RunError("this browser has no Web Locks, which a single instance across tabs needs".into())
    })?;
    let name = channel_name(app);
    let channel = web_sys::BroadcastChannel::new(&name)
        .map_err(|e| RunError(format!("no broadcast channel: {e:?}")))?;
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"ifAvailable".into(), &JsValue::TRUE);
    let mut start = Some(start);
    let granted = Closure::once_into_js(move |lock: JsValue| -> JsValue {
        if lock.is_null() || lock.is_undefined() {
            // Another tab runs the app: it gets this launch.
            let list = js_sys::Array::new();
            for a in &args {
                list.push(&JsValue::from_str(a));
            }
            let _ = channel.post_message(&list);
            channel.close();
            return JsValue::UNDEFINED;
        }
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            let launch: Vec<String> =
                js_sys::Array::from(&e.data()).iter().filter_map(|v| v.as_string()).collect();
            deliver(launch);
        });
        channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        // The listener and the channel live as long as the page.
        on_message.forget();
        std::mem::forget(channel);
        if let Some(s) = start.take() {
            s();
        }
        // Held until the page closes: a promise that never settles.
        js_sys::Promise::new(&mut |_, _| {}).into()
    });
    request
        .call3(&locks, &JsValue::from_str(&name), &opts, &granted)
        .map(drop)
        .map_err(|e| RunError(format!("the lock request failed: {e:?}")))
}

/// The page's launch: its address.
pub(crate) fn launch_args() -> Vec<String> {
    web_sys::window().and_then(|w| w.location().href().ok()).into_iter().collect()
}
