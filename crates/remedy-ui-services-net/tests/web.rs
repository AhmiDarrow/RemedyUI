//! The client in a browser page, against `tools/test-serve.py` (started by the web bridge's
//! `tools/web-test.sh`, which passes its address as `REMEDY_NET_URL` at build time).
#![cfg(target_arch = "wasm32")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, missing_docs)]

use std::sync::mpsc;
use std::time::Duration;

use remedy_ui_core::reactive::{Cancel, Resource, Signal, run_posted, untrack};
use remedy_ui_services_net::{Http, Multipart, NetError, Request};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SERVER: Option<&str> = option_env!("REMEDY_NET_URL");

async fn sleep(ms: f64) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let set: js_sys::Function =
            js_sys::Reflect::get(&js_sys::global(), &"setTimeout".into()).unwrap().into();
        set.call2(&JsValue::NULL, &resolve, &JsValue::from_f64(ms)).unwrap();
    });
    JsFuture::from(p).await.unwrap();
}

/// Waits (yielding to the page) for what `f`'s callback sends.
async fn answer<T: Send + 'static>(f: impl FnOnce(Box<dyn FnOnce(T) + Send>)) -> T {
    let (tx, rx) = mpsc::channel();
    f(Box::new(move |v| {
        let _ = tx.send(v);
    }));
    for _ in 0..1000 {
        if let Ok(v) = rx.try_recv() {
            return v;
        }
        sleep(10.0).await;
    }
    panic!("no answer in 10 s");
}

fn now() -> f64 {
    js_sys::Date::now()
}

#[derive(serde::Deserialize, Debug, PartialEq, Clone)]
struct Item {
    id: u32,
    name: String,
}

#[wasm_bindgen_test]
async fn requests_in_a_page() {
    let Some(base) = SERVER else {
        console_log!("REMEDY_NET_URL not set at build time: skipping");
        return;
    };
    let http = Http::new(base);

    // Typed JSON.
    let item: Result<Item, NetError> =
        answer(|d| http.send_json(Request::get("json"), &Cancel::new(), d)).await;
    assert_eq!(item, Ok(Item { id: 7, name: "seven".into() }));

    // Every method, with a JSON body.
    for (req, method) in
        [(Request::post("echo"), "POST"), (Request::put("echo"), "PUT"), (Request::delete("echo"), "DELETE")]
    {
        let v: serde_json::Value =
            answer(|d| http.send_json(req.json(&serde_json::json!({"a": 1})), &Cancel::new(), d))
                .await
                .unwrap();
        assert_eq!(v["method"], method);
        assert_eq!(v["type"], "application/json");
        assert_eq!(v["body"], r#"{"a":1}"#);
    }

    // An error's status and detail.
    match answer(|d| http.send(Request::get("error"), &Cancel::new(), d)).await {
        Err(NetError::Http(e)) => {
            assert_eq!((e.status, e.detail.as_deref()), (422, Some("the name is taken")))
        }
        other => panic!("{other:?}"),
    }

    // A multipart upload.
    let form = Multipart::new().text("title", "notes").file("file", "a.txt", "text/plain", b"hello".to_vec());
    let v: serde_json::Value =
        answer(|d| http.send_json(Request::post("echo").multipart(&form), &Cancel::new(), d)).await.unwrap();
    assert!(v["type"].as_str().unwrap().starts_with("multipart/form-data; boundary="));
    assert!(
        v["body"]
            .as_str()
            .unwrap()
            .contains("filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n")
    );

    // A keepalive request answers like any other.
    let v: serde_json::Value = answer(|d| {
        http.send_json(
            Request::post("echo").body(b"last save".to_vec(), "text/plain").keepalive(),
            &Cancel::new(),
            d,
        )
    })
    .await
    .unwrap();
    assert_eq!(v["body"], "last save");

    // A 304 answering a conditional request carries its new validators.
    match answer(|d| http.send(Request::get("etag").header("If-None-Match", "\"v1\""), &Cancel::new(), d))
        .await
    {
        Err(NetError::Http(e)) => {
            assert_eq!(
                (e.status, e.header("etag"), e.header("cache-control")),
                (304, Some("\"v2\""), Some("max-age=60"))
            )
        }
        other => panic!("{other:?}"),
    }

    // A streamed body arrives in chunks from the response's ReadableStream, within its cap.
    let got = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let g = got.clone();
    let out = answer(|d| {
        http.stream(
            Request::get("big"),
            &Cancel::new(),
            move |head, bytes| {
                assert_eq!(head.header("x-kind"), Some("big"));
                g.fetch_add(bytes.len(), std::sync::atomic::Ordering::SeqCst);
            },
            d,
        )
    })
    .await;
    assert!(out.is_ok_and(|r| r.body.is_empty()));
    assert_eq!(got.load(std::sync::atomic::Ordering::SeqCst), 300_000);
    let out = answer(|d| http.stream(Request::get("big").max_body(1000), &Cancel::new(), |_, _| {}, d)).await;
    assert_eq!(out, Err(NetError::TooLarge(1000)));

    // A timeout aborts the fetch.
    let t = now();
    let out =
        answer(|d| http.send(Request::get("slow").timeout(Duration::from_millis(300)), &Cancel::new(), d))
            .await;
    assert_eq!(out, Err(NetError::Timeout));
    assert!(now() - t < 1200.0, "took {} ms", now() - t);

    // Cancelling aborts it too.
    let cancel = Cancel::new();
    let t = now();
    let (tx, rx) = mpsc::channel();
    http.send(Request::get("slow"), &cancel, move |out| {
        let _ = tx.send(out);
    });
    sleep(100.0).await;
    cancel.cancel();
    let out = loop {
        if let Ok(v) = rx.try_recv() {
            break v;
        }
        sleep(10.0).await;
    };
    assert_eq!(out, Err(NetError::Cancelled));
    assert!(now() - t < 1000.0, "took {} ms", now() - t);

    // A resource: a new key aborts the old request, whose answer never lands.
    let path = Signal::new("slow".to_owned());
    let items: Resource<Item, NetError> = http.resource(move || path.get(), |p| Request::get(p.clone()));
    path.set("json".to_owned());
    for _ in 0..500 {
        run_posted();
        if !untrack(|| items.loading()) {
            break;
        }
        sleep(10.0).await;
    }
    assert_eq!(untrack(|| items.get()).map(|i| i.id), Some(7));
    sleep(1800.0).await;
    run_posted();
    assert_eq!(untrack(|| items.get()).map(|i| i.id), Some(7));
}
