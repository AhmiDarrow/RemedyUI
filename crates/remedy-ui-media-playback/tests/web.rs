//! Media playback in a real browser (run with the web bridge's `tools/web-test.sh`): the test
//! clip decoded by the browser's WebCodecs decoders (pictures in the right colors at their
//! times, sound, seeks), WebAudio's clock advancing, and a player playing and seeking on
//! them.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

use remedy_ui_media_playback::{
    AudioFormat, Decoder, MediaHost, MediaSource, Picture, PixelFormat, PlaybackState, Player, PlayerOptions,
    Sample, platform_host,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

const CLIP: &[u8] = include_bytes!("fixtures/clip.mp4");

#[wasm_bindgen(inline_js = r#"
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function blobUrl(bytes) { return URL.createObjectURL(new Blob([bytes], { type: 'video/mp4' })); }
"#)]
extern "C" {
    fn sleep(ms: u32) -> js_sys::Promise;
    #[wasm_bindgen(js_name = blobUrl)]
    fn blob_url(bytes: &[u8]) -> String;
}

/// Waits (10 ms at a time, up to `limit` times) for `ok`.
async fn wait_for(p: &Player, (what, limit): (&str, u32), ok: impl Fn(&Player) -> bool) {
    for _ in 0..limit {
        if ok(p) {
            return;
        }
        wait_ms(10).await;
    }
    unreachable!("{what}: state {:?} at {:?}, error {:?}", p.state(), p.position(), p.error());
}

async fn wait_ms(ms: u32) {
    let _ = JsFuture::from(sleep(ms)).await;
}

fn host() -> std::sync::Arc<dyn MediaHost> {
    platform_host().must("the browser's media host")
}

/// Which of red, green and blue a BT.709 video-range picture is at its middle.
fn hue(p: &Picture) -> char {
    let (x, y) = ((p.width / 2) as usize, (p.height / 2) as usize);
    let at = |i: usize, off: usize| {
        p.planes.get(i).and_then(|pl| p.data.get(pl.offset + off)).copied().unwrap_or(0)
    };
    let stride = |i: usize| p.planes.get(i).map_or(0, |pl| pl.stride);
    let (r, g, b) = match p.format {
        PixelFormat::Rgba => {
            let o = y * stride(0) + x * 4;
            (f32::from(at(0, o)), f32::from(at(0, o + 1)), f32::from(at(0, o + 2)))
        }
        PixelFormat::Nv12 | PixelFormat::I420 => {
            let yv = f32::from(at(0, y * stride(0) + x));
            let (u, v) = if p.format == PixelFormat::Nv12 {
                let o = (y / 2) * stride(1) + (x / 2) * 2;
                (f32::from(at(1, o)), f32::from(at(1, o + 1)))
            } else {
                (f32::from(at(1, (y / 2) * stride(1) + x / 2)), f32::from(at(2, (y / 2) * stride(2) + x / 2)))
            };
            let (c, d, e) = (yv - 16.0, u - 128.0, v - 128.0);
            (1.164 * c + 1.793 * e, 1.164 * c - 0.213 * d - 0.533 * e, 1.164 * c + 2.112 * d)
        }
    };
    if r > g && r > b {
        'r'
    } else if g > b {
        'g'
    } else {
        'b'
    }
}

/// Polls `dec` (giving the page its turns between polls) for the next sample.
async fn next(dec: &mut Box<dyn Decoder>) -> Option<Sample> {
    for _ in 0..4000 {
        match dec.poll_read(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(r) => return r.must("decoding"),
            Poll::Pending => wait_ms(2).await,
        }
    }
    unreachable!("the decoder answered within 8 s")
}

/// The first picture at or after `t`.
async fn picture_at(dec: &mut Box<dyn Decoder>, t: Duration) -> (Duration, Picture) {
    loop {
        match next(dec).await {
            Some(Sample::Video { at, picture, .. }) if at >= t => return (at, picture),
            Some(_) => {}
            None => unreachable!("a picture at {t:?} before the end"),
        }
    }
}

/// The URL of the test clip served slowly (`tools/slow-serve.py`, started by the web
/// bridge's `tools/web-test.sh`), when the build was given one.
const SLOW_CLIP: Option<&str> = option_env!("REMEDY_SLOW_CLIP_URL");

#[wasm_bindgen_test]
async fn a_slow_download_reports_what_has_arrived_growing_within_the_media() {
    let Some(url) = SLOW_CLIP else {
        web_sys::console::log_1(
            &"REMEDY_SLOW_CLIP_URL not set at build time: skipping the slow download".into(),
        );
        return;
    };
    let started = js_sys::Date::now();
    let mut dec = host().open(&MediaSource::url(url)).must("opening the slow clip");
    let mut seen: Vec<(f64, Vec<std::ops::Range<Duration>>)> = Vec::new();
    let ask = |dec: &mut Box<dyn Decoder>, seen: &mut Vec<(f64, Vec<std::ops::Range<Duration>>)>| {
        if let Some(r) = dec.buffered()
            && seen.last().is_none_or(|(_, last)| *last != r)
        {
            seen.push((js_sys::Date::now() - started, r));
        }
    };
    // It opens once the movie box is in, at the front of the file, long before the rest.
    let mut ready = false;
    for _ in 0..2000 {
        match dec.poll_ready(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(r) => {
                r.must("the decoders take the clip");
                ready = true;
                break;
            }
            Poll::Pending => wait_ms(2).await,
        }
    }
    assert!(ready, "the slow clip opened");
    let opened_at = js_sys::Date::now() - started;
    let length = dec.info().duration.must("a length");
    ask(&mut dec, &mut seen);
    let mut ended = false;
    for _ in 0..3000 {
        match dec.poll_read(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(Ok(Some(_))) => {}
            Poll::Ready(Ok(None)) => {
                ended = true;
                break;
            }
            Poll::Ready(Err(e)) => unreachable!("decoding the slow clip: {e}"),
            Poll::Pending => wait_ms(5).await,
        }
        ask(&mut dec, &mut seen);
    }
    assert!(ended, "the slow clip played to its end");
    ask(&mut dec, &mut seen);
    let log = seen.iter().map(|(at, r)| format!("  at {at:>6.0} ms: {r:?}")).collect::<Vec<_>>().join("\n");
    web_sys::console::log_1(&format!("opened at {opened_at:.0} ms; the page held:\n{log}").into());
    for (at, r) in &seen {
        assert!(
            r.iter().all(|r| r.start <= r.end && r.end <= length),
            "at {at} ms a range outside 0..{length:?}: {r:?}"
        );
    }
    let reach = |r: &[std::ops::Range<Duration>]| r.iter().map(|r| r.end).max().unwrap_or_default();
    let first = seen.first().map(|(_, r)| reach(r)).unwrap_or_default();
    let last = seen.last().map(|(_, r)| reach(r)).unwrap_or_default();
    assert!(first < length, "it opened before all of it arrived: {first:?} of {length:?}");
    assert_eq!(last, length, "all of it arrived by the end");
    let mut most = Duration::ZERO;
    for (at, r) in &seen {
        assert!(reach(r) >= most, "at {at} ms it shrank: {r:?}");
        most = reach(r);
    }
    let steps = seen.iter().zip(seen.iter().skip(1)).filter(|(a, b)| reach(&b.1) > reach(&a.1)).count();
    assert!(steps >= 3, "it grew in steps as the bytes came: {log}");
}

#[wasm_bindgen_test]
async fn the_browser_decodes_the_clip_and_plays_it() {
    let host = host();
    web_sys::console::log_1(&format!("media host: {}", host.name()).into());
    let url = blob_url(CLIP);
    let mut dec = host.open(&MediaSource::url(url.clone())).must("opening the clip");
    // The source is fetched and the decoders configured without blocking.
    let mut ready = false;
    for _ in 0..2000 {
        match dec.poll_ready(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(r) => {
                r.must("the decoders take the clip");
                ready = true;
                break;
            }
            Poll::Pending => wait_ms(2).await,
        }
    }
    assert!(ready, "the clip opened");
    let info = dec.info().clone();
    let video = info.video.clone().must("the clip's picture");
    assert_eq!((video.width, video.height), (176, 100));
    assert_eq!(info.audio, Some(AudioFormat::new(48_000, 2)));
    assert!(info.duration.is_some_and(|d| (d.as_secs_f64() - 3.0).abs() < 0.1), "{:?}", info.duration);

    // Pictures in the right colors at their times, and sound with them.
    let (at, first) = picture_at(&mut dec, Duration::ZERO).await;
    assert!(at < Duration::from_millis(50), "the first picture at the start: {at:?}");
    assert_eq!(hue(&first), 'r', "the first second is red ({:?})", first.format);
    let mut sound = 0usize;
    for _ in 0..40 {
        if let Some(Sample::Audio { samples, .. }) = next(&mut dec).await {
            sound += samples.len();
        }
    }
    assert!(sound > 0, "sound was decoded with the pictures");

    // Seeks land: the picture at the target has the target's color.
    dec.seek(Duration::from_millis(1500)).must("seeking to 1.5 s");
    let (at, green) = picture_at(&mut dec, Duration::from_millis(1500)).await;
    assert!(at < Duration::from_millis(1600), "{at:?}");
    assert_eq!(hue(&green), 'g', "1.5 s is green");
    dec.seek(Duration::from_millis(2500)).must("seeking to 2.5 s");
    let (_, blue) = picture_at(&mut dec, Duration::from_millis(2500)).await;
    assert_eq!(hue(&blue), 'b', "2.5 s is blue");
    // To the end.
    let mut ended = false;
    for _ in 0..400 {
        if next(&mut dec).await.is_none() {
            ended = true;
            break;
        }
    }
    assert!(ended, "the clip ends");
    drop(dec);

    // WebAudio's clock: written sound is heard, and the clock says how much.
    let mut out = host.audio_output(AudioFormat::new(48_000, 2)).must("a sound output");
    let room = out.room().must("room");
    assert!(room > 0, "an empty output has room");
    let frames = room.min(9600);
    let tone: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let s = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.1;
            [s, s]
        })
        .collect();
    out.write(&tone).must("writing sound");
    assert_eq!(out.played().must("the clock"), 0, "nothing is heard while paused");
    out.resume().must("resuming");
    let mut heard = 0;
    for _ in 0..100 {
        wait_ms(10).await;
        heard = out.played().must("the clock");
        if heard > 0 {
            break;
        }
    }
    assert!(heard > 0, "the sound clock advances while playing");
    out.flush().must("flushing");
    assert_eq!(out.played().must("the clock"), 0, "a flush sets the clock back");
    drop(out);

    // A player on the browser's decoders: it opens, plays on the sound clock, and seeks.
    let slot: Rc<RefCell<Option<Player>>> = Rc::default();
    let s2 = slot.clone();
    let config = remedy_ui::AppConfig::new("Playback test").cpu_only();
    remedy_ui::run(config, move |app: &remedy_ui::App| {
        let s3 = s2.clone();
        let url = url.clone();
        app.open(remedy_ui::WindowSpec::new("Playback test"), move |ui, _| {
            let p = Player::open(
                ui,
                platform_host().must("host"),
                MediaSource::url(url),
                PlayerOptions::default(),
            );
            // Seen on screen, so its pictures are shown (an unseen player shows none).
            let view = remedy_ui_media_playback::player(ui, &p, "Test clip").width(352.0).height(260.0);
            let _ = ui.root().child(view);
            *s3.borrow_mut() = Some(p);
        });
    })
    .must("the app starts");
    for _ in 0..600 {
        if slot.borrow().is_some() {
            break;
        }
        wait_ms(10).await;
    }
    let p = slot.borrow().clone().must("the player");
    wait_for(&p, ("opened", 600), |p: &Player| p.state() == PlaybackState::Paused).await;
    p.seek(Duration::from_millis(1200));
    wait_for(&p, ("the seek landed", 300), |p: &Player| (p.position().as_secs_f64() - 1.2).abs() < 0.001)
        .await;
    p.play();
    wait_for(&p, ("playing", 300), |p: &Player| p.state() == PlaybackState::Playing).await;
    wait_for(&p, ("time moved on", 600), |p: &Player| p.position() >= Duration::from_millis(1700)).await;
    wait_for(&p, ("the end", 600), |p: &Player| p.state() == PlaybackState::Ended).await;
    let stats = p.stats();
    web_sys::console::log_1(&format!("player: {stats:?}, sound {:?}", p.audio_error()).into());
    assert!(stats.frames_shown > 0, "pictures were shown: {stats:?}");
    assert!((p.position().as_secs_f64() - 3.0).abs() < 0.05, "{:?}", p.position());
    if p.audio_error().is_none() {
        assert!(p.stats().audio_frames > 0, "sound went to WebAudio");
    }
}
