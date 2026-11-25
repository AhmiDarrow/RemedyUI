//! The native client against a small HTTP server on the loopback interface.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, missing_docs)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use remedy_ui_core::reactive::{Cancel, Resource, Signal, run_posted, untrack};
use remedy_ui_services_net::{
    CookieJar, Http, HttpPolicy, Multipart, NetError, Request, SameSite, finish_keepalive, on_keepalive,
};

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
                let (method, path) =
                    (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
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
                let get = |n: &str| {
                    headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap_or_default()
                };
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
                    "/api/login" => (
                        200,
                        "Set-Cookie: api=1\r\nSet-Cookie: all=1; Path=/; SameSite=Strict\r\n".into(),
                        "null".into(),
                    ),
                    "/api/whoami" => (200, String::new(), serde_json::json!(get("cookie")).to_string()),
                    "/hop" => {
                        (302, "Location: /api/hop2\r\nSet-Cookie: hop=1; Path=/\r\n".into(), "{}".into())
                    }
                    "/api/hop2" => (307, "Location: whoami\r\n".into(), "{}".into()),
                    "/see-other" => (303, "Location: /echo\r\n".into(), "{}".into()),
                    "/off-loopback" => (302, "Location: http://example.com/\r\n".into(), "{}".into()),
                    "/loop" => (302, "Location: /loop\r\n".into(), "{}".into()),
                    "/etag" if get("if-none-match") == "\"v1\"" => {
                        (304, "ETag: \"v2\"\r\nCache-Control: max-age=60\r\n".into(), String::new())
                    }
                    "/big" => (200, "X-Kind: big\r\n".into(), "x".repeat(300_000)),
                    "/drip" => {
                        // Sent in pieces, a while apart, until the client lets go.
                        let mut s = stream;
                        let _ = write!(
                            s,
                            "HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n"
                        );
                        for _ in 0..1000 {
                            if s.write_all(&[b'd'; 100]).and_then(|()| s.flush()).is_err() {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        return;
                    }
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
    // Cancelled rather than the slow answer: the caller returned before the server did,
    // however loaded the machine is.
    let out = wait(|d| http.send(Request::get("slow"), &cancel, d));
    assert_eq!(out, Err(NetError::Cancelled));
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

#[test]
fn cookies_follow_paths_and_redirects() {
    let (addr, _) = server();
    let jar = CookieJar::new();
    let http = Http::new(&addr).cookies(jar.clone());
    http.send_blocking(Request::get("api/login"), &Cancel::new()).unwrap();
    // `api` has the default path `/api`; `all` goes everywhere.
    let me: String = wait(|d| http.send_json(Request::get("api/whoami"), &Cancel::new(), d)).unwrap();
    assert_eq!(me, "api=1; all=1");
    let me: String = wait(|d| http.send_json(Request::get("whoami"), &Cancel::new(), d)).unwrap();
    assert_eq!(me, "all=1");
    let all = jar.cookies().into_iter().find(|c| c.name == "all").unwrap();
    assert_eq!(all.same_site, Some(SameSite::Strict));
    // Redirects are followed hop by hop: the cookie set on the way is sent on the next hop,
    // and each hop gets the cookies of its own path.
    let me: String = wait(|d| http.send_json(Request::get("hop"), &Cancel::new(), d)).unwrap();
    assert_eq!(me, "api=1; all=1; hop=1");
}

#[test]
fn redirects_are_checked() {
    let (addr, _) = server();
    let http = Http::new(&addr);
    // A 303 turns a POST into a GET without its body.
    let v: serde_json::Value =
        wait(|d| http.send_json(Request::post("see-other").json(&1), &Cancel::new(), d)).unwrap();
    assert_eq!((v["method"].as_str(), v["body"].as_str()), (Some("GET"), Some("")));
    // A redirect to plain http off the loopback interface is refused like the address itself.
    let out = http.send_blocking(Request::get("off-loopback"), &Cancel::new());
    assert!(matches!(out, Err(NetError::Refused(_))), "{out:?}");
    let out = http.send_blocking(Request::get("loop"), &Cancel::new());
    assert!(matches!(out, Err(NetError::Network(_))), "{out:?}");
}

#[test]
fn not_modified_carries_its_headers() {
    let (addr, _) = server();
    let out = Http::new(&addr)
        .send_blocking(Request::get("etag").header("If-None-Match", "\"v1\""), &Cancel::new());
    match out {
        Err(NetError::Http(e)) => {
            assert_eq!(e.status, 304);
            assert_eq!(e.header("ETag"), Some("\"v2\""));
            assert_eq!(e.header("cache-control"), Some("max-age=60"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn streams_in_chunks_with_a_cap() {
    let (addr, _) = server();
    let http = Http::new(&addr);
    let got = Arc::new(AtomicUsize::new(0));
    let g = got.clone();
    let out = wait(|d| {
        http.stream(
            Request::get("big"),
            &Cancel::new(),
            move |head, bytes| {
                assert_eq!(head.header("x-kind"), Some("big"));
                g.fetch_add(bytes.len(), Ordering::SeqCst);
            },
            d,
        )
    });
    let r = out.unwrap();
    assert!(r.body.is_empty() && r.status == 200);
    assert_eq!(got.load(Ordering::SeqCst), 300_000);
    // Past the cap.
    let out = wait(|d| http.stream(Request::get("big").max_body(100_000), &Cancel::new(), |_, _| {}, d));
    assert_eq!(out, Err(NetError::TooLarge(100_000)));
    // Cancelled while it drips: the answer comes at once and no chunk follows it.
    let cancel = Cancel::new();
    let answered = Arc::new(AtomicBool::new(false));
    let late = Arc::new(AtomicUsize::new(0));
    let (a, l) = (answered.clone(), late.clone());
    let (tx, rx) = mpsc::channel();
    http.stream(
        Request::get("drip"),
        &cancel,
        move |_, _| {
            if a.load(Ordering::SeqCst) {
                l.fetch_add(1, Ordering::SeqCst);
            }
        },
        move |out| {
            let _ = tx.send(out);
        },
    );
    std::thread::sleep(Duration::from_millis(150));
    cancel.cancel();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Err(NetError::Cancelled));
    answered.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(late.load(Ordering::SeqCst), 0);
    // An error status streams nothing: its body is in the error.
    let out = wait(|d| http.stream(Request::get("error"), &Cancel::new(), |_, _| panic!("no chunk"), d));
    assert_eq!(out.unwrap_err().status(), Some(422));
}

#[test]
fn private_networks_are_opt_in() {
    // An address on no network this machine has: refused before any connection without the
    // policy; with it, sent (and failing to connect, or timing out).
    let lan = "http://10.255.255.1:9/x";
    let out = Http::new("").send_blocking(Request::get(lan), &Cancel::new());
    assert!(matches!(out, Err(NetError::Refused(_))), "{out:?}");
    let out = Http::new("")
        .policy(HttpPolicy::PrivateNetworks)
        .timeout(Duration::from_millis(300))
        .send_blocking(Request::get(lan), &Cancel::new());
    assert!(matches!(out, Err(NetError::Timeout | NetError::Network(_))), "{out:?}");
}

#[test]
fn keepalive_tells_the_platform() {
    static CALLS: std::sync::Mutex<Vec<bool>> = std::sync::Mutex::new(Vec::new());
    on_keepalive(|on| CALLS.lock().unwrap().push(on));
    let (addr, _) = server();
    let http = Http::new(&addr);
    http.send(Request::post("echo").keepalive(), &Cancel::new(), |_| {});
    http.send(Request::post("echo").keepalive(), &Cancel::new(), |_| {});
    assert!(finish_keepalive(Duration::from_secs(10)));
    // Other tests' keepalive requests may overlap; the hook always ends told "none left".
    let calls = CALLS.lock().unwrap().clone();
    assert!(calls.first() == Some(&true) && calls.last() == Some(&false), "{calls:?}");
}

#[test]
fn every_limit_can_be_set() {
    let (addr, _) = server();
    // The body limit: the client's, a request's own, or none.
    let small = Http::new(&addr).max_body(1000);
    assert_eq!(small.send_blocking(Request::get("big"), &Cancel::new()), Err(NetError::TooLarge(1000)));
    let r = small.send_blocking(Request::get("big").max_body(None), &Cancel::new()).unwrap();
    assert_eq!(r.body.len(), 300_000);
    assert!(Http::new(&addr).max_body(None).send_blocking(Request::get("big"), &Cancel::new()).is_ok());
    // Redirects: none followed answers with the redirect itself; too few is an error.
    match Http::new(&addr).max_redirects(0).send_blocking(Request::get("see-other"), &Cancel::new()) {
        Err(NetError::Http(e)) => assert_eq!((e.status, e.header("location")), (303, Some("/echo"))),
        other => panic!("{other:?}"),
    }
    let out = Http::new(&addr).max_redirects(1).send_blocking(Request::get("hop"), &Cancel::new());
    assert!(matches!(out, Err(NetError::Network(_))), "{out:?}");
    assert!(Http::new(&addr).max_redirects(2).send_blocking(Request::get("hop"), &Cancel::new()).is_ok());
    // No timeout at all.
    let none = Http::new(&addr).timeout(None);
    assert!(none.send_blocking(Request::get("json"), &Cancel::new()).is_ok());
    assert!(Http::new(&addr).send_blocking(Request::get("json").timeout(None), &Cancel::new()).is_ok());
}
