//! Throughput: 100 MB of colored output fed from a thread while the UI thread keeps drawing
//! frames. Run in release:
//!
//! ```text
//! cargo test --release -p remedy-ui-widgets-term --test throughput -- --ignored --nocapture
//! ```
//!
//! It prints the parse rate alone, the rate with the UI running, and the UI thread's frame
//! times; it fails if a frame takes long enough to be felt.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use remedy_ui_core::Size;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{Request, Ui};
use remedy_ui_widgets_term::*;

const TOTAL: usize = 100 << 20;

/// About 100 MB of the output a build or a log tail makes: numbered lines in eight colors,
/// some bold, some with true color.
fn payload() -> Vec<u8> {
    let mut out = Vec::with_capacity(TOTAL + 4096);
    let mut n = 0u64;
    while out.len() < TOTAL {
        let line = match n % 4 {
            0 => format!(
                "\x1b[3{}m{n:09}\x1b[0m compiling crate number {n} of the workspace, all well\r\n",
                n % 8
            ),
            1 => format!(
                "\x1b[1;38;2;{};{};200m{n:09}\x1b[0m warning: something minor happened here\r\n",
                n % 256,
                (n * 7) % 256
            ),
            2 => format!("{n:09} plain text line with nothing special in it, just words and more words\r\n"),
            _ => format!("\x1b[4m{n:09}\x1b[24m \x1b[7mreverse\x1b[27m \x1b[2mdim\x1b[22m and done\r\n"),
        };
        out.extend_from_slice(line.as_bytes());
        n += 1;
    }
    out.extend_from_slice(b"\x1b[0mTHE END");
    out
}

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

/// The parse rate for kinds of output, to see where time goes.
#[test]
#[ignore = "a measurement: run in release with --ignored --nocapture"]
fn parse_rates_by_kind_of_output() {
    let line = b"abcdefghijklmnopqrstuvwxyz0123456789 abcdefghijklmnopqrstuvwxyz0123456789\r\n";
    let cases: Vec<(&str, Vec<u8>, bool)> = vec![
        ("ascii no newline", b"abcdefghij".repeat(2_000_000), true),
        ("ascii lines", line.repeat(270_000), true),
        ("sgr", b"\x1b[31mab\x1b[0m".repeat(1_500_000), true),
        ("lines no scrollback", line.repeat(270_000), false),
    ];
    for (name, data, keep) in cases {
        let mut e = Emulator::new(120, 40);
        if !keep {
            e.set_scrollback(Scrollback::Off);
        }
        let t = Instant::now();
        for c in data.chunks(65536) {
            e.feed(c);
        }
        let s = t.elapsed().as_secs_f64();
        println!("{name}: {:.0} MB/s", data.len() as f64 / (1 << 20) as f64 / s);
    }
}

#[test]
#[ignore = "a measurement: run in release with --ignored --nocapture"]
fn a_hundred_megabytes_keep_the_ui_responsive() {
    let data = Arc::new(payload());
    let mb = data.len() as f64 / (1 << 20) as f64;

    // The parser and screen alone, on one thread.
    let mut e = Emulator::new(120, 40);
    let t = Instant::now();
    for chunk in data.chunks(64 * 1024) {
        e.feed(chunk);
    }
    let alone = t.elapsed();
    assert!(e.screen_text().ends_with("THE END"));
    println!(
        "parse alone: {mb:.0} MB in {:.2} s = {:.0} MB/s",
        alone.as_secs_f64(),
        mb / alone.as_secs_f64()
    );
    drop(e);

    // Fed from a thread while the UI thread draws frames.
    let ui = ui();
    let term = terminal(&ui, TerminalOptions::default());
    ui.root().child(ui.el("col").column().height(900.0).fill_width().child(term.element()));
    let size = Size::new(1400.0, 900.0);
    let frame = |ui: &Ui| {
        let _ = remedy_ui_reactive::run_posted();
        let _ = ui.frame(size, 1.0);
        for r in ui.take_requests() {
            if let Request::ScrollTo { scroller, offset } = r {
                ui.set_scroll_offset(scroller, offset);
            }
        }
    };
    for _ in 0..4 {
        frame(&ui);
    }
    let g = term.size();
    let done = Arc::new(AtomicBool::new(false));
    let feeder = term.feeder();
    let (d2, data2) = (done.clone(), data.clone());
    let start = Instant::now();
    let worker = std::thread::spawn(move || {
        for chunk in data2.chunks(64 * 1024) {
            feeder.feed(chunk);
        }
        d2.store(true, Ordering::Release);
        start.elapsed()
    });
    let mut frames: Vec<Duration> = Vec::new();
    let mut next = Instant::now();
    loop {
        let finished = done.load(Ordering::Acquire);
        let t = Instant::now();
        frame(&ui);
        frames.push(t.elapsed());
        if finished {
            break;
        }
        // A 60 Hz display: the next frame is due 16.7 ms after the last.
        next += Duration::from_micros(16_667);
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        } else {
            next = Instant::now();
        }
    }
    let fed = worker.join().unwrap_or_default();
    frame(&ui);
    let total = start.elapsed();
    assert!(term.with_emulator(|e| e.screen_text().ends_with("THE END")));
    frames.sort();
    let pct = |p: f64| frames.get(((frames.len() - 1) as f64 * p) as usize).copied().unwrap_or_default();
    let max = frames.last().copied().unwrap_or_default();
    println!(
        "with the UI: {mb:.0} MB parsed in {:.2} s = {:.0} MB/s ({} x {} cells); all shown after {:.2} s",
        fed.as_secs_f64(),
        mb / fed.as_secs_f64(),
        g.cols,
        g.rows,
        total.as_secs_f64()
    );
    println!(
        "UI thread: {} frames, median {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        frames.len(),
        pct(0.5).as_secs_f64() * 1e3,
        pct(0.99).as_secs_f64() * 1e3,
        max.as_secs_f64() * 1e3
    );
    println!(
        "scrollback: {} lines in {:.1} MB",
        term.with_emulator(|e| e.history_len()),
        term.with_emulator(|e| e.history_bytes()) as f64 / (1 << 20) as f64
    );
    // Responsive: no frame long enough to be felt as a stall (100 ms is where people notice).
    assert!(max < Duration::from_millis(100), "a frame took {max:?}");
}
