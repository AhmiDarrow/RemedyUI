//! Requests in a browser page: `fetch` with an `AbortController`, never blocking. A
//! watchdog on the page's timer aborts the request when it is cancelled or its time runs out.
//! A streamed body is read from the response's `ReadableStream`, chunk by chunk.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use js_sys::{Array, Promise, Uint8Array};
use remedy_ui_reactive::Cancel;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::{HttpError, HttpPolicy, NetError, Parts, Prepared, Response};

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

/// Calls `obj.name(args…)`.
fn call(obj: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let f: js_sys::Function = js_sys::Reflect::get(obj, &name.into())?.dyn_into()?;
    f.apply(obj, &args.iter().collect::<Array>())
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
    // `fetch` is called here, before returning: a page that is closing (`pagehide`) only gets
    // the requests it starts inside that event, and a task's first poll comes after it.
    match start(p, cancel) {
        Ok(s) => wasm_bindgen_futures::spawn_local(async move { done(finish(s).await) }),
        Err(e) => done(Err(e)),
    }
}

/// A request whose `fetch` has been called.
struct Started {
    p: Prepared,
    fetch: Promise,
    why: Rc<Cell<Option<NetError>>>,
    finished: Rc<Cell<bool>>,
}

fn start(p: Prepared, cancel: Cancel) -> Result<Started, NetError> {
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
    // Plain http to the local network: Chrome's local network access lets an https page
    // reach it (after asking the person) when the request says so; other browsers ignore it.
    if p.policy == HttpPolicy::PrivateNetworks && Parts::of(&p.url).is_some_and(|a| a.scheme == "http") {
        js_sys::Reflect::set(&init, &"targetAddressSpace".into(), &"local".into()).map_err(net)?;
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
        let (started, limit) =
            (js_sys::Date::now(), p.timeout.map_or(f64::INFINITY, |t| t.as_secs_f64() * 1000.0));
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
    let fetch = win.fetch_with_str_and_init(&p.url, &init);
    Ok(Started { p, fetch, why, finished })
}

async fn finish(s: Started) -> Result<Response, NetError> {
    let net = |e: JsValue| NetError::Network(message(&e));
    let Started { p, fetch, why, finished } = s;
    let got = async {
        let resp: web_sys::Response = JsFuture::from(fetch).await.map_err(net)?.unchecked_into();
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
        let ok = (200..300).contains(&status);
        let head = Response { status, headers: out_headers, body: Vec::new() };
        let mut body = Vec::new();
        let reader = js_sys::Reflect::get(&resp, &"body".into())
            .ok()
            .filter(|b| !b.is_null() && !b.is_undefined())
            .map(|b| call(&b, "getReader", &[]))
            .transpose()
            .map_err(net)?;
        let mut total = 0u64;
        // A body-less answer (a 204, a 304) has no stream.
        while let Some(reader) = &reader {
            let got =
                JsFuture::from(Promise::from(call(reader, "read", &[]).map_err(net)?)).await.map_err(net)?;
            if js_sys::Reflect::get(&got, &"done".into()).is_ok_and(|d| d.is_truthy()) {
                break;
            }
            let bytes = js_sys::Reflect::get(&got, &"value".into())
                .map(|v| Uint8Array::new(&v).to_vec())
                .map_err(net)?;
            total += bytes.len() as u64;
            // The watchdog has answered (cancelled, timed out): no chunk after that.
            let stopped = {
                let w = why.take();
                let s = w.is_some();
                why.set(w);
                s
            };
            if stopped {
                return Err(NetError::Cancelled);
            }
            if let Some(limit) = p.limit.filter(|l| total > *l) {
                let _ = call(reader, "cancel", &[]);
                return Err(NetError::TooLarge(limit));
            }
            match p.chunks.as_ref().filter(|_| ok) {
                Some(c) => match c.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
                    Some(f) => f(&head, &bytes),
                    None => return Err(NetError::Cancelled),
                },
                None => body.extend_from_slice(&bytes),
            }
        }
        if ok {
            Ok(Response { body, ..head })
        } else {
            Err(NetError::Http(HttpError::new(status, head.headers, &body)))
        }
    }
    .await;
    finished.set(true);
    match why.take() {
        Some(reason) if got.is_err() => Err(reason),
        _ => got,
    }
}
