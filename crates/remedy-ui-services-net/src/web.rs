//! Requests in a browser page: `fetch` with an `AbortController`, never blocking. A
//! watchdog on the page's timer aborts the request when it is cancelled or its time runs out.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use js_sys::{Array, Promise, Uint8Array};
use remedy_ui_reactive::Cancel;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::{HttpError, NetError, Prepared, Response};

/// How often the watchdog looks at the cancel token and the clock.
const WATCH: Duration = Duration::from_millis(25);

async fn sleep(d: Duration) {
    let ms = d.as_secs_f64() * 1000.0;
    let p = Promise::new(&mut |resolve, _| {
        let set = js_sys::Reflect::get(&js_sys::global(), &"setTimeout".into())
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        if let Some(set) = set {
            let _ = set.call2(&JsValue::NULL, &resolve, &JsValue::from_f64(ms));
        }
    });
    let _ = JsFuture::from(p).await;
}

fn message(e: &JsValue) -> String {
    js_sys::Reflect::get(e, &"message".into())
        .ok()
        .and_then(|m| m.as_string())
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

pub(super) fn send(
    p: Prepared,
    cancel: Cancel,
    done: impl FnOnce(Result<Response, NetError>) + Send + 'static,
) {
    wasm_bindgen_futures::spawn_local(async move {
        done(run(p, cancel).await);
    });
}

async fn run(p: Prepared, cancel: Cancel) -> Result<Response, NetError> {
    let net = |e: JsValue| NetError::Network(message(&e));
    if !p.keepalive && cancel.is_cancelled() {
        return Err(NetError::Cancelled);
    }
    let win = web_sys::window().ok_or_else(|| NetError::Network("no browser window".into()))?;
    let abort = web_sys::AbortController::new().map_err(net)?;
    let init = web_sys::RequestInit::new();
    init.set_method(p.method);
    init.set_signal(Some(&abort.signal()));
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    // web-sys has no setter for `keepalive`; it is a plain field of the init dictionary.
    if p.keepalive {
        js_sys::Reflect::set(&init, &"keepalive".into(), &JsValue::TRUE).map_err(net)?;
    }
    let headers = web_sys::Headers::new().map_err(net)?;
    for (k, v) in &p.headers {
        headers.set(k, v).map_err(|e| NetError::Refused(message(&e)))?;
    }
    init.set_headers(&headers);
    if let Some(body) = &p.body {
        init.set_body(&Uint8Array::from(body.as_slice()));
    }
    // Set by the watchdog before it aborts: why.
    let why = Rc::new(Cell::new(None::<NetError>));
    let finished = Rc::new(Cell::new(false));
    {
        let (why, finished, abort, keepalive) = (why.clone(), finished.clone(), abort.clone(), p.keepalive);
        let (started, limit) = (js_sys::Date::now(), p.timeout.as_secs_f64() * 1000.0);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                sleep(WATCH).await;
                if finished.get() {
                    return;
                }
                let reason = if !keepalive && cancel.is_cancelled() {
                    NetError::Cancelled
                } else if js_sys::Date::now() - started > limit {
                    NetError::Timeout
                } else {
                    continue;
                };
                why.set(Some(reason));
                abort.abort();
                return;
            }
        });
    }
    let got = async {
        let resp: web_sys::Response =
            JsFuture::from(win.fetch_with_str_and_init(&p.url, &init)).await.map_err(net)?.unchecked_into();
        let status = resp.status();
        let mut out_headers = Vec::new();
        if let Ok(Some(entries)) = js_sys::try_iter(&resp.headers()) {
            for pair in entries.flatten() {
                let pair: Array = pair.unchecked_into();
                if let (Some(k), Some(v)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                    out_headers.push((k.to_ascii_lowercase(), v));
                }
            }
        }
        let buf = JsFuture::from(resp.array_buffer().map_err(net)?).await.map_err(net)?;
        let body = Uint8Array::new(&buf).to_vec();
        if (200..300).contains(&status) {
            Ok(Response { status, headers: out_headers, body })
        } else {
            Err(NetError::Http(HttpError::new(status, &body)))
        }
    }
    .await;
    finished.set(true);
    match why.take() {
        Some(reason) if got.is_err() => Err(reason),
        _ => got,
    }
}
