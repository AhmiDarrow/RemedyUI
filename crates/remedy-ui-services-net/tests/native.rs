//! The native client against a small HTTP server on the loopback interface.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, missing_docs)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use remedy_ui_reactive::{Cancel, Resource, Signal, run_posted, untrack};
use remedy_ui_services_net::{CookieJar, Http, Multipart, NetError, Request, finish_keepalive};

/// Starts the server; returns its address and a count of `/slow` requests it finished.
fn server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let slow_done = Arc::new(AtomicUsize::new(0));
    let count = slow_done.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let count = count.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
                let mut headers = Vec::new();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
                    }
                }
                let get = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap_or_default();
                let len: usize = get("content-length").parse().unwrap_or(0);
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let (status, extra, out): (u16, String, String) = match path.as_str() {
                    "/json" => (200, String::new(), r#"{"id":7,"name":"seven"}"#.into()),
                    "/error" => (422, String::new(), r#"{"detail":"the name is taken"}"#.into()),
                    "/plain-error" => (500, String::new(), "it broke".into()),
                    "/echo" => (
                        200,
                        String::new(),
                        serde_json::json!({
                            "method": method,
                            "type": get("content-type"),
                            "body": String::from_utf8_lossy(&body),
                            "token": get("x-token"),
                        })
                        .to_string(),
                    ),
                    "/login" => (200, "Set-Cookie: sid=abc; Path=/; HttpOnly\r\n".into(), "null".into()),
                    "/logout" => (200, "Set-Cookie: sid=; Max-Age=0\r\n".into(), "null".into()),
                    "/whoami" => (200, String::new(), serde_json::json!(get("cookie")).to_string()),
                    p if p.starts_with("/slow") => {
                        std::thread::sleep(Duration::from_millis(1500));
                        count.fetch_add(1, Ordering::SeqCst);
                        (200, String::new(), r#"{"id":1,"name":"slow"}"#.into())
                    }
                    _ => (404, String::new(), "{}".into()),
                };
                let mut s = stream;
                let _ = write!(
                    s,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{out}",
                    out.len()
                );
            });
        }
    });
    (addr, slow_done)
}

#[derive(serde::Deserialize, Debug, PartialEq, Clone)]
struct Item {
    id: u32,
    name: String,
}

fn wait<T: Send + 'static>(f: impl FnOnce(Box<dyn FnOnce(T) + Send>)) -> T {
    let (tx, rx) = mpsc::channel();
    f(Box::new(move |v| {
        let _ = tx.send(v);
    }));
    rx.recv_timeout(Duration::from_secs(10)).unwrap()
}

#[test]
fn json_get_post_put_delete() {
    let (addr, _) = server();
    let http = Http::new(&addr).header("X-Token", "t1");
    let item: Item = wait(|d| http.send_json(Request::get("/json"), &Cancel::new(), d)).unwrap();
    assert_eq!(item, Item { id: 7, name: "seven".into() });
    for req in [Request::post("echo"), Request::put("echo"), Request::delete("echo")] {
        let v: serde_json::Value =
            wait(|d| http.send_json(req.json(&serde_json::json!({"a": 1})), &Cancel::new(), d)).unwrap();
        assert_eq!(v["type"], "application/json");
        assert_eq!(v["body"], r#"{"a":1}"#);
        assert_eq!(v["token"], "t1");
    }
    // A request's own header wins over the client's.
    let r = http.send_blocking(Request::post("echo").header("x-token", "t2"), &Cancel::new()).unwrap();
    assert_eq!(r.json::<serde_json::Value>().unwrap()["token"], "t2");
    assert_eq!(r.header("Content-Type"), Some("application/json"));
}

#[test]
fn errors_carry_status_body_and_detail() {
    let (addr, _) = server();
    let http = Http::new(&addr);
    match http.send_blocking(Request::get("error"), &Cancel::new()) {
        Err(NetError::Http(e)) => {
            assert_eq!(e.status, 422);
            assert_eq!(e.detail.as_deref(), Some("the name is taken"));
            assert!(e.body.contains("taken"));
        }
        other => panic!("{other:?}"),
    }
    match http.send_blocking(Request::get("plain-error"), &Cancel::new()) {
        Err(NetError::Http(e)) => assert_eq!((e.status, e.body.as_str(), e.detail), (500, "it broke", None)),
        other => panic!("{other:?}"),
    }
    // Decoding the wrong shape.
    let out: Result<Vec<u32>, _> = wait(|d| http.send_json(Request::get("json"), &Cancel::new(), d));
    assert!(matches!(out, Err(NetError::Decode(_))));
    // Plain http off the loopback interface is never sent.
    let out = Http::new("http://example.com").send_blocking(Request::get("x"), &Cancel::new());
    assert!(matches!(out, Err(NetError::Refused(_))));
}

#[test]
fn multipart_upload() {
    let (addr, _) = server();
    let form = Multipart::new().text("title", "notes").file("file", "a.txt", "text/plain", b"hello".to_vec());
    let r = Http::new(&addr).send_blocking(Request::post("echo").multipart(&form), &Cancel::new()).unwrap();
    let v: serde_json::Value = r.json().unwrap();
    let ty = v["type"].as_str().unwrap();
    assert!(ty.starts_with("multipart/form-data; boundary="));
    let body = v["body"].as_str().unwrap();
    assert!(body.contains("name=\"title\"\r\n\r\nnotes\r\n"));
    assert!(body.contains("filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n"));
}

#[test]
fn timeouts() {
    let (addr, _) = server();
    let http = Http::new(&addr).timeout(Duration::from_millis(300));
    let t = Instant::now();
    assert_eq!(http.send_blocking(Request::get("slow"), &Cancel::new()), Err(NetError::Timeout));
    assert!(t.elapsed() < Duration::from_millis(1200));
    // A request's own timeout wins.
    assert!(http.send_blocking(Request::get("slow").timeout(Duration::from_secs(5)), &Cancel::new()).is_ok());
}

#[test]
fn cancellation_answers_at_once() {
    let (addr, _) = server();
    let http = Http::new(&addr);
    let cancel = Cancel::new();
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        c2.cancel();
    });
    let t = Instant::now();
    let out = wait(|d| http.send(Request::get("slow"), &cancel, d));
    assert_eq!(out, Err(NetError::Cancelled));
    assert!(t.elapsed() < Duration::from_millis(1000));
}

#[test]
fn resource_cancels_the_old_key() {
    let (addr, _) = server();
    let http = Http::new(&addr);
    let path = Signal::new("slow".to_owned());
    let items: Resource<Item, NetError> = http.resource(move || path.get(), |p| Request::get(p.clone()));
    assert!(untrack(|| items.loading()));
    path.set("json".to_owned());
    let t = Instant::now();
    while untrack(|| items.loading()) && t.elapsed() < Duration::from_secs(10) {
        run_posted();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(untrack(|| items.get()), Some(Item { id: 7, name: "seven".into() }));
    // The slow answer for the old key never replaces it.
    std::thread::sleep(Duration::from_millis(1800));
    run_posted();
    assert_eq!(untrack(|| items.get()).map(|i| i.id), Some(7));
}

#[test]
fn keepalive_finishes_on_the_way_out() {
    let (addr, slow_done) = server();
    let cancel = Cancel::new();
    Http::new(&addr).send(Request::post("slow").keepalive(), &cancel, |_| {});
    // Cancelling does not stop a keepalive request; quitting waits for it.
    cancel.cancel();
    std::thread::sleep(Duration::from_millis(50));
    assert!(finish_keepalive(Duration::from_secs(10)));
    assert_eq!(slow_done.load(Ordering::SeqCst), 1);
}

#[test]
fn cookie_jar() {
    let (addr, _) = server();
    let jar = CookieJar::new();
    let http = Http::new(&addr).cookies(jar.clone());
    let none: String = wait(|d| http.send_json(Request::get("whoami"), &Cancel::new(), d)).unwrap();
    assert_eq!(none, "");
    http.send_blocking(Request::get("login"), &Cancel::new()).unwrap();
    assert_eq!(jar.get(&addr, "sid").as_deref(), Some("abc"));
    let me: String = wait(|d| http.send_json(Request::get("whoami"), &Cancel::new(), d)).unwrap();
    assert_eq!(me, "sid=abc");
    http.send_blocking(Request::get("logout"), &Cancel::new()).unwrap();
    assert_eq!(jar.get(&addr, "sid"), None);
    // Without a jar nothing is kept.
    let plain = Http::new(&addr);
    plain.send_blocking(Request::get("login"), &Cancel::new()).unwrap();
    let none: String = wait(|d| plain.send_json(Request::get("whoami"), &Cancel::new(), d)).unwrap();
    assert_eq!(none, "");
}
