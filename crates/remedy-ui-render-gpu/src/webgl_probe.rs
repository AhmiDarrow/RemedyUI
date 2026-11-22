//! Whether a page can make a WebGL2 context, found out off the page's thread.
//!
//! `getContext("webgl2")` blocks the thread that calls it until the browser's GL is up. A
//! browser starts its GL once, on the first context any page or worker asks for; most take
//! milliseconds, but headless Firefox on Windows took eight minutes on a loaded machine
//! (ANGLE starting over Direct3D 11 in its parent process), and the page's thread, where the
//! WebGL2 rung called it, froze with it: no input, no frames, no timers. So the rung first
//! has a worker make a context on an `OffscreenCanvas` and waits for its answer without
//! blocking, however long it takes: the page keeps drawing on the CPU renderer meanwhile, and
//! once the worker has its context the browser's GL is up and the page's own is made at once
//! (2 ms in that same Firefox).

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// The worker: makes a WebGL2 context on a 1×1 offscreen canvas and says whether it could;
/// `"unknown"` where workers have no WebGL2 to ask (Safari before 17), which says nothing
/// about the page's own.
const WORKER: &str = "let r = 'unknown'; try { if (self.WebGL2RenderingContext && self.OffscreenCanvas) { \
const gl = new OffscreenCanvas(1, 1).getContext('webgl2'); r = !!gl; \
if (gl) { const l = gl.getExtension('WEBGL_lose_context'); if (l) l.loseContext(); } } } catch (e) {} postMessage(r);";

/// What a probe found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    /// The worker made a context: the browser's GL is up, and the page's own is made at once.
    Ready,
    /// The browser makes no WebGL2 context.
    Unusable,
    /// A worker could not tell (no WebGL2 in workers, or the page's content security policy
    /// forbids `blob:` workers): the rung asks on the page's thread, as it always did.
    Unknown,
}

thread_local! {
    /// The answer, once known: the browser's GL does not go away while the page lives.
    static KNOWN: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

/// Whether the page's WebGL2 context can be made without stalling it. The first call waits
/// (without blocking) for the browser to start its GL; later calls answer at once.
pub(crate) async fn webgl2() -> Probe {
    if let Some(p) = KNOWN.with_borrow(|k| *k) {
        return p;
    }
    let p = probe().await;
    KNOWN.with_borrow_mut(|k| *k = Some(p));
    p
}

async fn probe() -> Probe {
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/javascript");
    let parts = js_sys::Array::of1(&JsValue::from_str(WORKER));
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else { return Probe::Unknown };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return Probe::Unknown };
    let Ok(worker) = web_sys::Worker::new(&url) else {
        let _ = web_sys::Url::revoke_object_url(&url);
        return Probe::Unknown;
    };
    // Resolves with the worker's answer, or `null` if it could not start.
    let w = worker.clone();
    let answer = js_sys::Promise::new(&mut |resolve, _reject| {
        let r2 = resolve.clone();
        let on_message = Closure::once_into_js(move |e: web_sys::MessageEvent| {
            let _ = resolve.call1(&JsValue::NULL, &e.data());
        });
        let on_error = Closure::once_into_js(move |_: JsValue| {
            let _ = r2.call1(&JsValue::NULL, &JsValue::NULL);
        });
        w.set_onmessage(Some(on_message.unchecked_ref()));
        w.set_onerror(Some(on_error.unchecked_ref()));
    });
    let got = JsFuture::from(answer).await.unwrap_or(JsValue::NULL);
    worker.terminate();
    let _ = web_sys::Url::revoke_object_url(&url);
    match got.as_bool() {
        Some(true) => Probe::Ready,
        Some(false) => Probe::Unusable,
        None => Probe::Unknown,
    }
}
