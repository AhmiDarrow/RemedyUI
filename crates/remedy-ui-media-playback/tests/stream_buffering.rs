//! What the platform's decoder says it has downloaded of a stream: the test clip served over
//! HTTP from this process, slowly (a few kilobytes at a time), and the decoder's
//! `buffered` ranges asked as it arrives. They grow, and stay within the media.
//!
//! Runs where the platform decoder reads HTTP itself (GStreamer here; Media Foundation on
//! Windows); skips, saying why, where there is no platform host.

#![cfg(not(target_arch = "wasm32"))]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use remedy_ui_media_playback::*;

fn clip() -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip.mp4"))
        .unwrap_or_else(|e| unreachable!("the test clip: {e}"))
}

/// How the server trickles the file out: `CHUNK` bytes, then a pause of `PAUSE`.
const CHUNK: usize = 2048;
const PAUSE: Duration = Duration::from_millis(60);

/// Answers one request: the clip (or the range asked of it), a chunk at a time.
fn serve(mut s: TcpStream, data: &[u8]) -> std::io::Result<()> {
    let mut reader = BufReader::new(s.try_clone()?);
    let mut range = None;
    let mut head = false;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if l.starts_with("HEAD ") {
            head = true;
        }
        if let Some(v) = l.to_ascii_lowercase().strip_prefix("range: bytes=") {
            let (a, b) = v.split_once('-').unwrap_or((v, ""));
            let a: usize = a.trim().parse().unwrap_or(0);
            let b: usize = b.trim().parse().map_or(data.len() - 1, |b: usize| b.min(data.len() - 1));
            range = Some((a.min(data.len()), b));
        }
    }
    let (status, body) = match range {
        Some((a, b)) if a <= b => ("206 Partial Content", data.get(a..=b).unwrap_or_default()),
        Some(_) => ("416 Range Not Satisfiable", &[][..]),
        None => ("200 OK", data),
    };
    let mut header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
        body.len()
    );
    if let Some((a, b)) = range.filter(|(a, b)| a <= b) {
        header.push_str(&format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()));
    }
    header.push_str("\r\n");
    s.write_all(header.as_bytes())?;
    if head {
        return Ok(());
    }
    for c in body.chunks(CHUNK) {
        s.write_all(c)?;
        s.flush()?;
        std::thread::sleep(PAUSE);
    }
    Ok(())
}

/// A server on a free local port, serving the clip slowly to each connection; its URL.
fn slow_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("a local port: {e}"));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let data: Arc<[u8]> = clip().into();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let data = data.clone();
            std::thread::spawn(move || {
                let _ = serve(s, &data);
            });
        }
    });
    format!("http://127.0.0.1:{port}/clip.mp4")
}

#[test]
fn a_slow_stream_reports_what_has_arrived_growing_within_the_media() {
    let host = match platform_host() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("no platform media host here, skipping: {e}");
            return;
        }
    };
    let url = slow_server();
    let started = Instant::now();
    let mut dec = match host.open(&MediaSource::url(&url)) {
        Ok(d) => d,
        Err(e @ (MediaError::Missing(_) | MediaError::Unsupported(_))) if !cfg!(windows) => {
            eprintln!("the platform media host cannot stream HTTP here, skipping: {e}");
            return;
        }
        Err(e) => unreachable!("open {url}: {e}"),
    };
    let length = dec.info().duration.unwrap_or_else(|| unreachable!("a length: {:?}", dec.info()));
    // The clip is ~29 KB: at 2 KB every 60 ms it takes about 0.9 s to arrive.
    let mut seen: Vec<(Duration, Vec<std::ops::Range<Duration>>)> = Vec::new();
    let ask = |dec: &mut Box<dyn Decoder>, seen: &mut Vec<_>| {
        let r = dec.buffered();
        if let Some(r) = r
            && seen.last().is_none_or(|(_, last)| *last != r)
        {
            seen.push((started.elapsed(), r));
        }
    };
    ask(&mut dec, &mut seen);
    // Read slowly too, so the download is not over before the first ask.
    let mut samples = 0;
    while dec.read().unwrap_or_else(|e| unreachable!("read: {e}")).is_some() {
        samples += 1;
        ask(&mut dec, &mut seen);
        if samples % 8 == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    // After the end, what arrived is the whole clip; ask a few times as the pipeline settles.
    for _ in 0..20 {
        ask(&mut dec, &mut seen);
        std::thread::sleep(Duration::from_millis(25));
    }
    eprintln!("over {:?}, {samples} samples, the decoder held:", started.elapsed());
    for (at, r) in &seen {
        eprintln!("  at {at:>10.3?}: {r:?}");
    }
    assert!(!seen.is_empty(), "the decoder never said what it holds of {url}");
    let tolerance = Duration::from_millis(100);
    for (at, r) in &seen {
        assert!(
            r.iter().all(|r| r.start <= r.end && r.end <= length + tolerance),
            "at {at:?} a range outside 0..{length:?}: {r:?}"
        );
    }
    let reach = |r: &[std::ops::Range<Duration>]| r.iter().map(|r| r.end).max().unwrap_or_default();
    let first = seen.first().map(|(_, r)| reach(r)).unwrap_or_default();
    let last = seen.last().map(|(_, r)| reach(r)).unwrap_or_default();
    assert!(first < last, "what arrived grew: first {first:?}, last {last:?}");
    assert!(last + tolerance >= length, "all of it arrived by the end: {last:?} of {length:?}");
    // It grows as the bytes come: never more than a little back (a pipeline's estimate
    // may wobble), and through more than two steps.
    let mut most = Duration::ZERO;
    for (at, r) in &seen {
        let end = reach(r);
        assert!(end + Duration::from_millis(300) >= most, "at {at:?} it shrank to {end:?} from {most:?}");
        most = most.max(end);
    }
    let steps = seen.iter().zip(seen.iter().skip(1)).filter(|(a, b)| reach(&b.1) > reach(&a.1)).count();
    // Media Foundation's HTTP byte stream caches in 16 KB blocks: the ~29 KB clip is only two
    // of them, and the first has arrived by the time `open` returns.
    let want = if cfg!(windows) { 1 } else { 2 };
    assert!(steps >= want, "it grew in steps as the bytes came, not all at once: {seen:?}");
}
