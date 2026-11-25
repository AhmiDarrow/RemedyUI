//! Images from addresses through the engine's own fetcher, against a server on this machine:
//! loading, sharing, the memory and disk caches (max-age and ETag), failures shown by the
//! app's fallback, the real size reported, and the fetch cancelled when the element goes.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use remedy_ui_core::Size;
use remedy_ui_media_image::*;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};

fn png(w: u32, h: u32) -> Vec<u8> {
    let raw: Vec<u8> = (0..w * h).flat_map(|i| [(i % 255) as u8, 40, 90, 255]).collect();
    let mut out = Vec::new();
    let _ = PngEncoder::new(&mut out).write_image(&raw, w, h, ExtendedColorType::Rgba8);
    out
}

/// What the server saw: requests per path, the statuses it sent, and whether a slow body
/// was cut off by the client.
#[derive(Default)]
struct Seen {
    hits: HashMap<String, usize>,
    statuses: Vec<(String, u16)>,
    /// The `If-None-Match` each request came with.
    asked: Vec<(String, Option<String>)>,
    cut_off: bool,
}

struct Server {
    base: String,
    seen: Arc<Mutex<Seen>>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e}"));
        let base = format!("http://{}", listener.local_addr().unwrap_or_else(|e| unreachable!("{e}")));
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let seen = s2.clone();
                std::thread::spawn(move || serve(conn, &seen));
            }
        });
        Server { base, seen }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn hits(&self, path: &str) -> usize {
        self.seen.lock().map(|s| s.hits.get(path).copied().unwrap_or(0)).unwrap_or(0)
    }

    fn sent(&self, path: &str, status: u16) -> bool {
        self.seen.lock().is_ok_and(|s| s.statuses.iter().any(|(p, st)| p == path && *st == status))
    }
}

fn serve(conn: std::net::TcpStream, seen: &Mutex<Seen>) {
    let mut reader = BufReader::new(conn.try_clone().unwrap_or_else(|e| unreachable!("{e}")));
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut if_none_match = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.eq_ignore_ascii_case("if-none-match")
        {
            if_none_match = Some(v.trim().to_owned());
        }
    }
    if let Ok(mut s) = seen.lock() {
        *s.hits.entry(path.clone()).or_default() += 1;
        s.asked.push((path.clone(), if_none_match.clone()));
    }
    let mut out = conn;
    let (status, headers, body): (u16, &str, Vec<u8>) = match path.as_str() {
        "/wide.png" | "/shared.png" | "/disk.png" => (200, "Cache-Control: max-age=600\r\n", png(300, 150)),
        "/etag.png" if if_none_match.as_deref() == Some("\"e1\"") => {
            (304, "ETag: \"e1\"\r\nCache-Control: no-cache\r\n", vec![])
        }
        "/etag.png" => (200, "ETag: \"e1\"\r\nCache-Control: no-cache\r\n", png(20, 40)),
        // Revalidating moves it to a new ETag.
        "/rotate.png" if if_none_match.is_some() => {
            (304, "ETag: \"r2\"\r\nCache-Control: no-cache\r\n", vec![])
        }
        "/rotate.png" => (200, "ETag: \"r1\"\r\nCache-Control: no-cache\r\n", png(20, 40)),
        "/slow.png" => {
            let body = png(64, 64);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len() * 100
            );
            let _ = out.write_all(head.as_bytes());
            // A trickle that would take minutes: the client lets go long before.
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(20) {
                if out.write_all(&[0; 64]).and_then(|()| out.flush()).is_err() {
                    if let Ok(mut s) = seen.lock() {
                        s.cut_off = true;
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            return;
        }
        _ => (404, "", b"not here".to_vec()),
    };
    if let Ok(mut s) = seen.lock() {
        s.statuses.push((path, status));
    }
    let reason = match status {
        200 => "OK",
        304 => "Not Modified",
        _ => "Not Found",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n{headers}Content-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = out.write_all(head.as_bytes()).and_then(|()| out.write_all(&body));
}

fn ui() -> Ui {
    // Every test keeps its fetched images in the same folder of its own.
    set_cache_dir(Some(std::env::temp_dir().join(format!("remedy-image-url-{}", std::process::id()))));
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn frame(ui: &Ui) {
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
    let _ = ui.take_requests();
}

/// Frames until `done`, or 10 seconds.
fn until(ui: &Ui, done: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        frame(ui);
        let _ = remedy_ui_core::reactive::run_posted();
        if done() {
            frame(ui);
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn ready(el: &El) -> bool {
    el.info().is_some_and(|i| i.variant.is_empty()) && el.bounds().is_some_and(|b| b.width() > 0.0)
}

#[test]
fn an_address_loads_reports_its_real_size_and_is_fetched_once() {
    let server = Server::start();
    let ui = ui();
    let sizes: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
    let s2 = sizes.clone();
    let a = image_ext(
        &ui,
        Source::Url(server.url("/wide.png")),
        "Wide art",
        Fit::Contain,
        ImageOptions::new().on_state(move |s| {
            if let ImageState::Sized(z) | ImageState::Ready(_, z) = s {
                s2.borrow_mut().push(*z);
            }
        }),
    );
    let b = image(&ui, Source::Url(server.url("/wide.png")), "", Fit::Cover).width(60.0).height(60.0);
    ui.root().child(ui.el("col").column().child(a.clone()).child(b.clone()));
    assert!(until(&ui, || ready(&a) && cached() == 2), "both loaded");
    // Unsized, it takes the image's own size.
    let ab = a.bounds().unwrap_or_default();
    assert_eq!((ab.width(), ab.height()), (300.0, 150.0));
    assert!(sizes.borrow().iter().all(|z| *z == (300, 150)) && !sizes.borrow().is_empty());
    assert_eq!(server.hits("/wide.png"), 1, "two elements and a probe share one fetch");
    // Gone, nothing is held or in flight.
    ui.root().clear();
    frame(&ui);
    frame(&ui);
    assert_eq!(cached(), 0);
    assert_eq!(fetching(), 0);
}

#[test]
fn a_later_element_finds_the_image_in_memory_then_on_disk() {
    let server = Server::start();
    let ui = ui();
    let first =
        image(&ui, Source::Url(server.url("/disk.png")), "Art", Fit::Contain).width(30.0).height(15.0);
    ui.root().child(first.clone());
    assert!(until(&ui, || ready(&first)));
    ui.root().clear();
    frame(&ui);
    let again =
        image(&ui, Source::Url(server.url("/disk.png")), "Art", Fit::Contain).width(30.0).height(15.0);
    ui.root().child(again.clone());
    assert!(until(&ui, || ready(&again)));
    assert_eq!(server.hits("/disk.png"), 1, "from memory");
    // A new run: memory empty, the disk copy still fresh (max-age=600).
    ui.root().clear();
    frame(&ui);
    forget_remembered();
    let third =
        image(&ui, Source::Url(server.url("/disk.png")), "Art", Fit::Contain).width(30.0).height(15.0);
    ui.root().child(third.clone());
    assert!(until(&ui, || ready(&third)));
    assert_eq!(server.hits("/disk.png"), 1, "from disk");
}

#[test]
fn a_no_cache_image_is_revalidated_with_its_etag() {
    let server = Server::start();
    let ui = ui();
    let el = image(&ui, Source::Url(server.url("/etag.png")), "Tall", Fit::Contain).width(20.0).height(40.0);
    ui.root().child(el.clone());
    assert!(until(&ui, || ready(&el)));
    ui.root().clear();
    frame(&ui);
    forget_remembered();
    let state = remedy_ui_core::reactive::Signal::new(ImageState::Loading);
    let _req = load(&Source::Url(server.url("/etag.png")), Some((20, 40)), false, state);
    assert!(until(&ui, || matches!(state.get_untracked().ok(), Some(ImageState::Ready(..)))));
    assert!(server.sent("/etag.png", 304), "asked with If-None-Match, answered Not Modified");
    match state.get_untracked().ok() {
        Some(ImageState::Ready(_, size)) => assert_eq!(size, (20, 40), "the disk copy's pixels"),
        other => unreachable!("{other:?}"),
    }
}

#[test]
fn a_304_brings_new_validators() {
    let server = Server::start();
    let ui = ui();
    let url = server.url("/rotate.png");
    for _ in 0..3 {
        forget_remembered();
        let state = remedy_ui_core::reactive::Signal::new(ImageState::Loading);
        let _req = load(&Source::Url(url.clone()), Some((20, 40)), false, state);
        assert!(until(&ui, || matches!(state.get_untracked().ok(), Some(ImageState::Ready(..)))));
        // Let go of the pixels, so the next load goes back to the server.
        drop(_req);
        state.set(ImageState::Loading);
        frame(&ui);
    }
    let asked: Vec<Option<String>> = server
        .seen
        .lock()
        .map(|s| s.asked.iter().filter(|(p, _)| p == "/rotate.png").map(|(_, v)| v.clone()).collect())
        .unwrap_or_default();
    // The second answer's new ETag is the one the third request asks with.
    assert_eq!(asked, [None, Some("\"r1\"".to_owned()), Some("\"r2\"".to_owned())]);
}

#[test]
fn a_failed_address_shows_the_apps_fallback() {
    let server = Server::start();
    let ui = ui();
    let built = Rc::new(Cell::new(0));
    let failed: Rc<RefCell<Option<String>>> = Rc::default();
    let (b2, f2, ui2) = (built.clone(), failed.clone(), ui.clone());
    let el = image_ext(
        &ui,
        Source::Url(server.url("/missing.png")),
        "Poster",
        Fit::Cover,
        ImageOptions::new()
            .placeholder(ui.el("box"))
            .fallback(move || {
                b2.set(b2.get() + 1);
                ui2.el("box").text("P").width(90.0).height(120.0)
            })
            .on_state(move |s| {
                if let ImageState::Failed(e) = s {
                    *f2.borrow_mut() = Some(e.to_string());
                }
            }),
    );
    // In a row, so its width is its own (the root's column stretches its children).
    ui.root().child(ui.el("row").row().child(el.clone()));
    frame(&ui);
    // While loading the app's placeholder shows, not the theme's.
    assert_eq!(el.info().map(|i| i.variant), Some(""));
    let placeholder = el.child_elements().first().cloned();
    assert!(placeholder.as_ref().and_then(El::info).is_some_and(|i| !i.hidden), "placeholder shown");
    assert!(until(&ui, || built.get() > 0));
    assert_eq!(built.get(), 1, "built once");
    assert!(failed.borrow().as_deref().is_some_and(|e| e.contains("404")), "{:?}", failed.borrow());
    assert!(placeholder.as_ref().and_then(El::info).is_some_and(|i| i.hidden), "placeholder hidden");
    // The fallback sizes the element nothing else sized.
    let b = el.bounds().unwrap_or_default();
    assert_eq!((b.width(), b.height()), (90.0, 120.0));
    assert!(format!("{:?}", el.info().and_then(|i| i.semantics)).contains("Not shown"));
}

#[test]
fn the_fetch_is_cancelled_when_the_element_goes() {
    let server = Server::start();
    let ui = ui();
    let el = image(&ui, Source::Url(server.url("/slow.png")), "Slow", Fit::Contain).width(64.0).height(64.0);
    ui.root().child(el.clone());
    assert!(until(&ui, || server.hits("/slow.png") == 1), "the fetch started");
    assert_eq!(fetching(), 1);
    el.remove();
    frame(&ui);
    assert_eq!(fetching(), 0, "nobody waits for it");
    let start = Instant::now();
    while !server.seen.lock().is_ok_and(|s| s.cut_off) && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(server.seen.lock().is_ok_and(|s| s.cut_off), "the connection was let go");
    assert_eq!(cached(), 0);
}
