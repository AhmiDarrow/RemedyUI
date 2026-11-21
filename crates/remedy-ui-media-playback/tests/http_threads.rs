//! A stream opened over HTTP after the thread that opened the last one has ended. On Windows,
//! COM tore its apartment down with that thread and unloaded DLLs Media Foundation's HTTP
//! stack still pointed into, so this open crashed the process. Its own test binary, so no
//! other test's thread keeps the apartment alive.

#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};
use std::net::TcpListener;

use remedy_ui_media_playback::*;

#[test]
fn opens_over_http_after_the_last_opening_thread_ended() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("listening: {e}"));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    // Every request is answered 404, so each open fails at once, after the HTTP stack ran.
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let _ = s.read(&mut [0; 4096]);
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    });
    for _ in 0..3 {
        std::thread::spawn(move || {
            let Ok(host) = platform_host() else { return };
            let url = format!("http://127.0.0.1:{port}/index.m3u8");
            assert!(host.open(&MediaSource::Url(url)).is_err());
        })
        .join()
        .unwrap_or_else(|_| unreachable!("the opening thread panicked"));
    }
}
