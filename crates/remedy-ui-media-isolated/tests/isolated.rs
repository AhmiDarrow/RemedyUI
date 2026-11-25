//! Isolated decoding under attack. This binary is its own helper, as an app is: `main` calls
//! `helper_main` first, after registering a hostile test format that panics, loops forever,
//! allocates without end, or tries to open files and sockets, depending on its bytes.
//!
//! Each test is run in order (they share the one enabled engine); a failure is reported and
//! the run carries on. Arguments that are not flags filter tests by name.
//!
//! On Android the same tests run inside an app (`examples/android-isolated-check/`,
//! `tools/android-check.sh`), whose helpers are isolated service processes.

use std::hint::black_box;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use remedy_ui_core::Size;
use remedy_ui_media_image::{
    DecodeError, Decoded, Fit, Format, Source, add_format, decode, decode_for_box, dimensions, image, router,
};
use remedy_ui_media_isolated::{Layer, enable, isolation, stats};
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::{El, Ui};

// ---------- the hostile format ----------

/// `HOSTILE:` then what to do. Its header always says 16×16, so its budgets are a tiny
/// image's.
struct Hostile;

const MAGIC: &[u8] = b"HOSTILE:";

fn hostile(what: &str) -> Vec<u8> {
    [MAGIC, what.as_bytes()].concat()
}

fn attempt<T>(r: std::io::Result<T>) -> String {
    match r {
        Ok(_) => "ALLOWED".to_owned(),
        Err(e) => format!("blocked ({:?}: {e})", e.kind()),
    }
}

/// Tries to read `path`, write a file beside it, and list its folder.
fn probe_files(path: &str) -> String {
    let p = std::path::Path::new(path);
    let read = attempt(std::fs::File::open(p).and_then(|mut f| {
        let mut s = String::new();
        f.read_to_string(&mut s).map(|_| s)
    }));
    let dir = p.parent().unwrap_or(p);
    let made = dir.join("written-by-the-helper.txt");
    let write = attempt(std::fs::File::create(&made).and_then(|mut f| f.write_all(b"escaped")));
    let list = attempt(std::fs::read_dir(dir));
    format!("read: {read}; write: {write}; list: {list}")
}

/// Tries to connect over TCP to `tcp,udp` (the test's listeners) and to send a UDP
/// datagram. A connection to the machine itself is answered at once when allowed, so a short
/// wait tells; where the sandbox drops packets instead of refusing them, the wait running out
/// is the block. Whether the datagram arrived is the listener's to say.
fn probe_network(addrs: &str) -> String {
    let (tcp, udp) = addrs.split_once(',').unwrap_or((addrs, addrs));
    // Rust's standard library panics when Windows Sockets cannot start at all: that is a
    // block too, reported with its message.
    let guarded = |f: &dyn Fn() -> String| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            format!("blocked (sockets cannot start: {msg})")
        })
    };
    let tcp = guarded(&|| match tcp.parse() {
        Ok(a) => attempt(TcpStream::connect_timeout(&a, Duration::from_millis(200))),
        Err(e) => format!("bad address {e}"),
    });
    let udp = guarded(&|| attempt(UdpSocket::bind("127.0.0.1:0").and_then(|s| s.send_to(b"escaped", udp))));
    // Code that skips Windows Sockets opens the socket driver itself: below the device, as
    // Windows Sockets does (an open the device leaves to the driver), and the device itself.
    let afd = if cfg!(windows) {
        let open = |p: &str| attempt(std::fs::File::open(p));
        format!(
            "; socket driver: {}; socket device: {}",
            open(r"\\?\GLOBALROOT\Device\Afd\Endpoint"),
            open(r"\\?\GLOBALROOT\Device\Afd")
        )
    } else {
        String::new()
    };
    format!("tcp: {tcp}; udp send: {udp}{afd}")
}

impl Format for Hostile {
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(MAGIC)
    }

    fn dimensions(&self, _bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
        Ok((16, 16))
    }

    #[allow(clippy::panic)]
    fn decode(&self, bytes: &[u8], _show_at: Option<(u32, u32)>) -> Result<Decoded, DecodeError> {
        let what = String::from_utf8_lossy(bytes.get(MAGIC.len()..).unwrap_or_default()).into_owned();
        match what.as_str() {
            "panic" => panic!("a hostile decoder panics"),
            "loop" => loop {
                black_box(&what);
            },
            "alloc" => {
                let mut hoard: Vec<Vec<u8>> = Vec::new();
                loop {
                    // Filled, so the memory is really used.
                    hoard.push(vec![1u8; 16 << 20]);
                    black_box(&hoard);
                }
            }
            _ => {}
        }
        if let Some(path) = what.strip_prefix("file:") {
            return Err(DecodeError::Malformed(probe_files(path)));
        }
        if let Some(addr) = what.strip_prefix("net:") {
            return Err(DecodeError::Malformed(probe_network(addr)));
        }
        if let Some(spawn) = what.strip_prefix("spawn:") {
            // `program`, or on Android the shell, which leaves the marker itself.
            let (program, marker) = spawn.split_once('\n').unwrap_or((spawn, ""));
            let mut c = std::process::Command::new(program);
            if cfg!(target_os = "android") {
                c.arg("-c").arg(format!("echo started > '{marker}'"));
            } else {
                c.arg(MARKER);
            }
            let r = c.spawn().and_then(|mut c| c.wait());
            return Err(DecodeError::Malformed(format!("spawn: {}", attempt(r))));
        }
        Err(DecodeError::Unrecognized)
    }
}

// ---------- pictures ----------

fn png(w: u32, h: u32) -> Vec<u8> {
    let raw: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, (i / w % 256) as u8, 90, 200]).collect();
    let mut out = Vec::new();
    let _ = PngEncoder::new(&mut out).write_image(&raw, w, h, ExtendedColorType::Rgba8);
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let raw: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 90, 200]).collect();
    let mut out = Vec::new();
    let _ = JpegEncoder::new_with_quality(&mut out, 90).write_image(&raw, w, h, ExtendedColorType::Rgb8);
    out
}

fn stopped(r: Result<Decoded, DecodeError>) -> String {
    match r {
        Err(DecodeError::Stopped(why)) => why,
        other => unreachable!("expected the helper to be stopped, got {:?}", other.map(|d| d.source_size)),
    }
}

/// A real image decodes in a helper, the same as in the app: proof a fresh helper took over.
fn still_decodes() {
    let bytes = png(40, 30);
    let d = decode_for_box(&bytes, Some((20, 20)), false).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((d.image.width, d.image.height, d.source_size), (20, 15, (40, 30)));
}

// ---------- tests ----------

fn helpers_start_sandboxed() {
    let iso = enable().unwrap_or_else(|e| unreachable!("{e}"));
    println!("    layers: {:?}", iso.layers);
    assert!(router().is_some(), "the image loader decodes through the helper");
    assert_eq!(isolation(), Some(iso.clone()));
    if cfg!(windows) {
        for l in [Layer::AppContainer, Layer::RestrictedToken, Layer::JobObject] {
            assert!(iso.layers.contains(&l), "{l:?} missing: {:?}", iso.layers);
        }
    }
    if cfg!(target_os = "linux") {
        assert!(iso.layers.contains(&Layer::Seccomp), "{:?}", iso.layers);
        assert!(iso.layers.contains(&Layer::ResourceLimits), "{:?}", iso.layers);
        let landlock =
            std::fs::read_to_string("/sys/kernel/security/lsm").is_ok_and(|l| l.contains("landlock"));
        if landlock {
            assert!(iso.layers.iter().any(|l| matches!(l, Layer::Landlock { .. })), "{:?}", iso.layers);
        }
    }
    if cfg!(target_os = "android") {
        assert!(iso.layers.contains(&Layer::IsolatedProcess), "{:?}", iso.layers);
        assert!(iso.layers.contains(&Layer::ResourceLimits), "{:?}", iso.layers);
    }
    // Enabling again answers the same.
    assert_eq!(enable().ok(), Some(iso));
}

fn images_decode_in_the_helper_as_in_the_app() {
    for bytes in [png(300, 200), jpeg(64, 48)] {
        let before = stats().requests;
        let here =
            remedy_ui_media_image::decode(&bytes, Some((150, 100))).unwrap_or_else(|e| unreachable!("{e}"));
        let there = decode_for_box(&bytes, Some((150, 150)), false).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(stats().requests >= before + 2, "probe and decode went to the helper");
        assert_eq!((there.image.width, there.image.height), (here.image.width, here.image.height));
        assert_eq!(there.source_size, here.source_size);
        assert_eq!(there.image.pixels, here.image.pixels, "the same pixels");
    }
    // At its own size, and covering a box.
    let bytes = png(30, 10);
    let d = decode_for_box(&bytes, None, false).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((d.image.width, d.image.height), (30, 10));
    let d = decode_for_box(&bytes, Some((6, 6)), true).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!((d.image.width, d.image.height), (18, 6));
    // Damaged and unknown bytes are the usual errors, and leave the helper running.
    let crashed = stats().crashed;
    assert_eq!(decode_for_box(b"not an image", None, false).err(), Some(DecodeError::Unrecognized));
    let mut cut = png(50, 50);
    cut.truncate(cut.len() / 2);
    assert!(matches!(decode_for_box(&cut, None, false), Err(DecodeError::Malformed(_) | DecodeError::Io(_))));
    assert_eq!(stats().crashed, crashed);
}

fn a_panicking_decoder_takes_down_only_the_helper() {
    let s = stats();
    let why = stopped(decode_for_box(&hostile("panic"), Some((16, 16)), false));
    println!("    {why}");
    assert!(why.contains("crashed") && why.contains("panicked"), "{why}");
    assert_eq!(stats().crashed, s.crashed + 1);
    still_decodes();
    assert!(stats().started > s.started, "a new helper was started");
}

fn a_looping_decoder_is_stopped_after_its_images_time() {
    let s = stats();
    let t = Instant::now();
    let why = stopped(decode_for_box(&hostile("loop"), Some((16, 16)), false));
    println!("    {why} (after {:.2?})", t.elapsed());
    assert!(why.contains("worked past"), "{why}");
    assert_eq!(stats().timed_out, s.timed_out + 1);
    still_decodes();
}

fn a_decoder_allocating_without_end_is_stopped_at_its_images_memory() {
    let s = stats();
    let t = Instant::now();
    let why = stopped(decode_for_box(&hostile("alloc"), Some((16, 16)), false));
    println!("    {why} (after {:.2?})", t.elapsed());
    assert!(why.contains("memory"), "{why}");
    assert_eq!(stats().over_memory, s.over_memory + 1);
    still_decodes();
}

fn the_helper_cannot_touch_files() {
    let dir = std::env::temp_dir().join(format!("remedy-isolated-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let secret = dir.join("secret.txt");
    let _ = std::fs::write(&secret, "the user's private file");
    let bytes = hostile(&format!("file:{}", secret.display()));
    let escaped = dir.join("written-by-the-helper.txt");

    // In the app (no sandbox) the probe works: the test is real.
    let here = match decode(&bytes, None) {
        Err(DecodeError::Malformed(r)) => r,
        other => unreachable!("{:?}", other.map(|d| d.source_size)),
    };
    println!("    in the app:    {here}");
    assert!(!here.contains("blocked"), "{here}");
    let _ = std::fs::remove_file(&escaped);

    let there = match decode_for_box(&bytes, None, false) {
        Err(DecodeError::Malformed(r)) => r,
        other => unreachable!("{:?}", other.map(|d| d.source_size)),
    };
    println!("    in the helper: {there}");
    assert!(!there.contains("ALLOWED"), "{there}");
    assert!(!escaped.exists(), "the helper wrote a file");
    let _ = std::fs::remove_dir_all(&dir);
}

fn the_helper_cannot_reach_the_network() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e}"));
    let _ = listener.set_nonblocking(true);
    let addr = listener.local_addr().unwrap_or_else(|e| unreachable!("{e}"));
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e}"));
    let _ = udp.set_read_timeout(Some(Duration::from_millis(300)));
    let udp_addr = udp.local_addr().unwrap_or_else(|e| unreachable!("{e}"));
    let bytes = hostile(&format!("net:{addr},{udp_addr}"));
    let accepted = |l: &TcpListener| {
        let until = Instant::now() + Duration::from_millis(300);
        loop {
            if l.accept().is_ok() {
                return true;
            }
            if Instant::now() > until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let received = |s: &UdpSocket| s.recv_from(&mut [0u8; 16]).is_ok();

    // In the app (no sandbox) both arrive: the test is real.
    let here = match decode(&bytes, None) {
        Err(DecodeError::Malformed(r)) => r,
        other => unreachable!("{:?}", other.map(|d| d.source_size)),
    };
    println!("    in the app:    {here}");
    assert!(!here.contains("blocked"), "{here}");
    assert!(accepted(&listener), "the app's own connection arrived");
    assert!(received(&udp), "the app's own datagram arrived");

    let there = match decode_for_box(&bytes, None, false) {
        Err(DecodeError::Malformed(r)) => r,
        other => unreachable!("{:?}", other.map(|d| d.source_size)),
    };
    println!("    in the helper: {there}");
    // No socket can be made: seccomp on Linux; on Windows the less privileged AppContainer
    // cannot start Windows Sockets at all (and the socket driver is closed to it, see below).
    assert!(there.contains("tcp: blocked") && there.contains("udp send: blocked"), "{there}");
    assert!(!accepted(&listener), "a connection from the helper arrived");
    assert!(!received(&udp), "a datagram from the helper arrived");
}

/// Windows: a decoder that skips Windows Sockets and opens the socket driver (`\Device\Afd`)
/// itself, to make a socket and connect, is refused the driver.
fn the_helper_cannot_open_the_socket_driver() {
    if !cfg!(windows) {
        println!("    skipped: Windows only");
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("{e}"));
    let addr = listener.local_addr().unwrap_or_else(|e| unreachable!("{e}"));
    let bytes = hostile(&format!("net:{addr},{addr}"));
    let probe = |r: Result<Decoded, DecodeError>| match r {
        Err(DecodeError::Malformed(r)) => r,
        other => unreachable!("{:?}", other.map(|d| d.source_size)),
    };
    // In the app the driver opens below the device, as Windows Sockets opens it: the test is real.
    let here = probe(decode(&bytes, None));
    println!("    in the app:    {here}");
    assert!(here.contains("socket driver: ALLOWED"), "{here}");
    let there = probe(decode_for_box(&bytes, None, false));
    println!("    in the helper: {there}");
    for what in ["tcp: blocked", "socket driver: blocked", "socket device: blocked"] {
        assert!(there.contains(what), "{what}: {there}");
    }
}

/// Passed to a program the helper tries to start: started, this binary leaves a file named
/// after it in the temporary folder.
const MARKER: &str = "--remedy-isolated-spawned";

fn spawn_marker() -> std::path::PathBuf {
    std::env::temp_dir().join("remedy-isolated-spawned.txt")
}

fn the_helper_cannot_start_programs() {
    let marker = spawn_marker();
    let _ = std::fs::remove_file(&marker);
    // An app's own executable on Android is the system's app_process: the shell stands in.
    let exe = if cfg!(target_os = "android") {
        std::path::PathBuf::from("/system/bin/sh")
    } else {
        std::env::current_exe().unwrap_or_else(|e| unreachable!("{e}"))
    };
    let bytes = hostile(&format!("spawn:{}\n{}", exe.display(), marker.display()));

    let here = match decode(&bytes, None) {
        Err(DecodeError::Malformed(r)) => r,
        other => unreachable!("{:?}", other.map(|d| d.source_size)),
    };
    println!("    in the app:    {here}");
    assert!(here.contains("ALLOWED") && marker.exists(), "{here}");
    let _ = std::fs::remove_file(&marker);

    // Refused (Windows: the job and child-process policy), or the helper killed on the
    // spot for trying (Linux: seccomp).
    let there = match decode_for_box(&bytes, None, false) {
        Err(DecodeError::Malformed(r) | DecodeError::Stopped(r)) => r,
        other => unreachable!("{:?}", other.map(|d| d.source_size)),
    };
    println!("    in the helper: {there}");
    assert!(!there.contains("ALLOWED"), "{there}");
    assert!(!marker.exists(), "a program started from the helper ran");
    still_decodes();
}

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

fn settle(ui: &Ui, until: impl Fn() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
        let _ = ui.take_requests();
        let _ = remedy_ui_core::reactive::run_posted();
        if until() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = ui.frame(Size::new(800.0, 600.0), 1.0);
}

fn semantics(el: &El) -> String {
    format!("{:?}", el.info().and_then(|i| i.semantics))
}

fn an_image_element_shows_its_error_state_and_the_app_carries_on() {
    let ui = ui();
    let bad = image(&ui, Source::Bytes(Arc::from(hostile("panic"))), "A trap", Fit::Contain)
        .width(64.0)
        .height(64.0);
    let good = image(&ui, Source::Bytes(Arc::from(png(120, 60))), "A ramp", Fit::Contain);
    ui.root().child(ui.el("col").column().child(bad.clone()).child(good.clone()));
    settle(&ui, || {
        semantics(&bad).contains("Not shown") && good.info().is_some_and(|i| i.variant.is_empty())
    });
    let s = semantics(&bad);
    println!("    {s}");
    assert!(s.contains("Not shown") && s.contains("decoder was stopped"), "{s}");
    let gb = good.bounds().unwrap_or_default();
    let gs = semantics(&good);
    assert_eq!((gb.width(), gb.height()), (120.0, 60.0), "the good image loaded beside it: {gs}");
    assert_eq!(good.info().map(|i| i.variant), Some(""), "{gs}");
}

fn disabling_decodes_in_the_app_again() {
    // An animation playing when isolation goes off carries on, decoded in the app.
    let mut playing = anim::playing();
    let first = anim::next_pixels(&mut playing);
    remedy_ui_media_isolated::disable();
    assert!(router().is_none());
    assert!(remedy_ui_media_anim::route::opener().is_none());
    assert_eq!(isolation(), None);
    let requests = stats().requests;
    let second = anim::next_pixels(&mut playing);
    assert!(first.is_some() && second.is_some() && first != second, "it went on to its second frame");
    assert_eq!(stats().requests, requests, "decoded in the app");
    drop(playing);
    assert_eq!(dimensions(&png(5, 7)).ok(), Some((5, 7)));
    let d = decode_for_box(&png(5, 7), None, false).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(d.source_size, (5, 7));
    assert_eq!(stats().requests, requests, "nothing went to a helper");
    // On again (the Windows profile is registered again), and off for good.
    let iso = enable().unwrap_or_else(|e| unreachable!("{e}"));
    assert!(!iso.layers.is_empty());
    still_decodes();
    assert!(stats().requests > requests);
    remedy_ui_media_isolated::disable();
}

#[path = "isolated/anim.rs"]
mod anim;

/// The hostile formats, in the app and (registered before `helper_main`, or on Android from
/// `remedy_ui_media_isolated_formats`) in the helper.
pub(crate) fn register_formats() {
    add_format(Arc::new(Hostile));
    remedy_ui_media_anim::route::add_format(Arc::new(anim::HostileAnimation));
}

pub(crate) fn main() -> ExitCode {
    register_formats();
    remedy_ui_media_isolated::helper_main();
    if std::env::args().any(|a| a == MARKER) {
        let _ = std::fs::write(spawn_marker(), "started");
        return ExitCode::SUCCESS;
    }

    // An Android app's arguments are the system's: its filter comes in the environment.
    let filter: Vec<String> = if cfg!(target_os = "android") {
        std::env::var("REMEDY_TEST_FILTER")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    } else {
        std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect()
    };
    let tests: &[(&str, fn())] = &[
        ("helpers_start_sandboxed", helpers_start_sandboxed),
        ("images_decode_in_the_helper_as_in_the_app", images_decode_in_the_helper_as_in_the_app),
        ("a_panicking_decoder_takes_down_only_the_helper", a_panicking_decoder_takes_down_only_the_helper),
        (
            "a_looping_decoder_is_stopped_after_its_images_time",
            a_looping_decoder_is_stopped_after_its_images_time,
        ),
        (
            "a_decoder_allocating_without_end_is_stopped_at_its_images_memory",
            a_decoder_allocating_without_end_is_stopped_at_its_images_memory,
        ),
        ("the_helper_cannot_touch_files", the_helper_cannot_touch_files),
        ("the_helper_cannot_reach_the_network", the_helper_cannot_reach_the_network),
        ("the_helper_cannot_open_the_socket_driver", the_helper_cannot_open_the_socket_driver),
        ("the_helper_cannot_start_programs", the_helper_cannot_start_programs),
        (
            "an_image_element_shows_its_error_state_and_the_app_carries_on",
            an_image_element_shows_its_error_state_and_the_app_carries_on,
        ),
        (
            "animations_decode_in_the_helper_as_in_the_app",
            anim::animations_decode_in_the_helper_as_in_the_app,
        ),
        (
            "a_panicking_animation_fails_alone_and_its_neighbours_reopen",
            anim::a_panicking_animation_fails_alone_and_its_neighbours_reopen,
        ),
        (
            "a_looping_animation_is_stopped_after_its_frames_time",
            anim::a_looping_animation_is_stopped_after_its_frames_time,
        ),
        (
            "an_animation_allocating_without_end_is_stopped_at_its_frames_memory",
            anim::an_animation_allocating_without_end_is_stopped_at_its_frames_memory,
        ),
        (
            "a_hostile_animation_shows_its_error_state_while_a_good_one_plays",
            anim::a_hostile_animation_shows_its_error_state_while_a_good_one_plays,
        ),
        ("disabling_decodes_in_the_app_again", disabling_decodes_in_the_app_again),
    ];
    let mut failed = Vec::new();
    let mut ran = 0;
    for (name, test) in tests {
        // The first test enables isolation, which the others need.
        if !filter.is_empty()
            && *name != "helpers_start_sandboxed"
            && !filter.iter().any(|f| name.contains(f.as_str()))
        {
            continue;
        }
        ran += 1;
        println!("test {name} ...");
        let t = Instant::now();
        match std::panic::catch_unwind(test) {
            Ok(()) => println!("test {name} ... ok ({:.2?})", t.elapsed()),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
                // Without a helper the hostile decoders would run in this process.
                if remedy_ui_media_isolated::isolation().is_none()
                    && *name != "disabling_decodes_in_the_app_again"
                {
                    println!("isolation is off: the remaining tests are not run");
                    break;
                }
            }
        }
    }
    println!("\n{ran} tests, {} failed; {:?}", failed.len(), stats());
    if failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
