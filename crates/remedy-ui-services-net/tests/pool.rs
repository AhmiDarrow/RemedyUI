//! [`set_pool_limits`] decides whether connections are kept for reuse. A binary of its own,
//! so no other test sees the pool changed.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::indexing_slicing, missing_docs)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use remedy_ui_core::reactive::Cancel;
use remedy_ui_services_net::{Http, PoolLimits, Request, pool_limits, set_pool_limits};

/// A server answering every request on a connection with "ok"; returns its address and how
/// many connections it accepted.
fn server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = accepted.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line.trim_end().is_empty() {
                        let ok = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                        if ok.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    (addr, accepted)
}

#[test]
fn the_app_sets_how_connections_are_kept() {
    assert_eq!(pool_limits(), PoolLimits::DEFAULT);
    let (addr, accepted) = server();
    let http = Http::new(&addr);
    for _ in 0..3 {
        assert!(http.send_blocking(Request::get("/"), &Cancel::new()).is_ok());
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 1, "one connection, kept and reused");
    set_pool_limits(PoolLimits::default().idle_connections(0).idle_per_host(0));
    for _ in 0..3 {
        assert!(http.send_blocking(Request::get("/"), &Cancel::new()).is_ok());
    }
    set_pool_limits(PoolLimits::default());
    assert_eq!(accepted.load(Ordering::SeqCst), 4, "none kept: a connection a request");
}
