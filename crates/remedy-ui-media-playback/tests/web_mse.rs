//! Media playback through the browser's media element and Media Source Extensions, in a real
//! browser (run with the web bridge's `tools/web-test.sh`): the HLS test clip remuxed to
//! fragmented MP4, played by a hidden `<video>`, its pictures in the right colors at their
//! times through seeks, and a player that plays, seeks, follows the volume and speed, shows
//! captions, takes a new stream on a timeline and a quality switch, ends, and leaves nothing
//! behind.
//!
//! Built with `REMEDY_MSE_INSECURE=1` the page is served from the machine's LAN address
//! (`http:`, not a secure context, so no WebCodecs): the platform's own host must choose the
//! element by itself there. Otherwise (`localhost`) the element is asked for.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

use remedy_ui_media_playback::{
    Decoder, MediaHost, MediaSource, Picture, PixelFormat, PlaybackState, Player, PlayerOptions, Sample,
    SubtitleFormat, SubtitleTrack, Timeline, WebBackend, parse_subtitles, platform_host_with,
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

#[wasm_bindgen(inline_js = r#"
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function secure() { return !!window.isSecureContext; }
export function probe() {
  return `origin=${location.origin} secure=${isSecureContext} VideoDecoder=${typeof VideoDecoder} ` +
    `MediaSource=${typeof MediaSource} navigator.gpu=${!!navigator.gpu} rVFC=${'requestVideoFrameCallback' in HTMLVideoElement.prototype}`;
}
export function spyFrames() {
  if (window.__vfc) return;
  window.__vfc = 0;
  const f = HTMLVideoElement.prototype.requestVideoFrameCallback;
  window.__vfl = [];
  HTMLVideoElement.prototype.requestVideoFrameCallback = function (cb) { return f.call(this, (n, m) => { window.__vfc++; window.__vfl.push(`${Math.round(performance.now())}:${m.mediaTime.toFixed(2)}`); cb(n, m); }); };
  const raf = window.requestAnimationFrame; window.__raf = 0;
  window.requestAnimationFrame = function (cb) { return raf.call(window, (t) => { window.__raf++; cb(t); }); };
}
export function frameCalls() { return window.__vfc || 0; }
export function frameLog() { return `raf ${window.__raf} now ${Math.round(performance.now())} ` + (window.__vfl || []).join(' '); }
// The app in a small box: the CPU renderer of a debug build draws a whole page slowly.
export function smallMount() {
  if (document.getElementById('remedy')) return;
  const d = document.createElement('div'); d.id = 'remedy'; d.style.cssText = 'width:360px;height:280px';
  document.body.appendChild(d);
}
export function videos() { return document.querySelectorAll('video').length; }
export function videoProp(name) { const all = document.querySelectorAll('video'); const v = all[all.length - 1]; return v ? Number(v[name]) : NaN; }
"#)]
extern "C" {
    fn sleep(ms: u32) -> js_sys::Promise;
    fn secure() -> bool;
    fn probe() -> String;
    fn videos() -> u32;
    #[wasm_bindgen(js_name = smallMount)]
    fn small_mount();
    #[wasm_bindgen(js_name = spyFrames)]
    fn spy_frames();
    #[wasm_bindgen(js_name = frameCalls)]
    fn frame_calls() -> u32;
    #[wasm_bindgen(js_name = frameLog)]
    fn frame_log() -> String;
    #[wasm_bindgen(js_name = videoProp)]
    fn video_prop(name: &str) -> f64;
}

async fn wait_ms(ms: u32) {
    let _ = JsFuture::from(sleep(ms)).await;
}

const INSECURE: bool = option_env!("REMEDY_MSE_INSECURE").is_some();

/// Where `fixtures/hls/` is served (`tools/serve-dir.py`).
const HLS_DIR: Option<&str> = option_env!("REMEDY_HLS_URL");

/// What the page plays with: its own choice on an insecure page, the element asked for on a
/// secure one.
fn backend() -> WebBackend {
    if INSECURE { WebBackend::Auto } else { WebBackend::MediaSource }
}

fn host() -> std::sync::Arc<dyn MediaHost> {
    platform_host_with(backend()).must("the browser's media host")
}

/// Which of red, green and blue an RGBA picture is at its middle (read back from the GPU
/// when it is there).
fn hue(p: &Picture) -> char {
    let (data, plane) = match &p.gpu {
        Some(g) => {
            let px = g.0.read_back().must("the picture reads back");
            (px.data, px.planes[0])
        }
        None => (p.data.clone(), p.planes[0]),
    };
    assert!(p.gpu.is_some() || p.format == PixelFormat::Rgba, "{:?}", p.format);
    let (x, y) = ((p.width / 2) as usize, (p.height / 2) as usize);
    let o = plane.offset + y * plane.stride + x * 4;
    let at = |i: usize| data.get(o + i).copied().unwrap_or(0);
    let (r, g, b) = (at(0), at(1), at(2));
    if r > g && r > b {
        'r'
    } else if g > b {
        'g'
    } else {
        'b'
    }
}

async fn ready(dec: &mut Box<dyn Decoder>) {
    for _ in 0..2000 {
        match dec.poll_ready(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(r) => return r.must("the element takes the stream"),
            Poll::Pending => wait_ms(5).await,
        }
    }
    unreachable!("the stream opened within 10 s");
}

async fn next(dec: &mut Box<dyn Decoder>) -> Option<Sample> {
    for _ in 0..2000 {
        match dec.poll_read(&mut Context::from_waker(std::task::Waker::noop())) {
            Poll::Ready(r) => return r.must("playing"),
            Poll::Pending => wait_ms(5).await,
        }
    }
    unreachable!("the element showed a picture within 10 s");
}

/// The first picture shown at or after `t`.
async fn picture_at(dec: &mut Box<dyn Decoder>, t: Duration) -> (Duration, Picture) {
    loop {
        match next(dec).await {
            Some(Sample::Video { at, picture, .. }) if at + Duration::from_millis(1) >= t => {
                return (at, picture);
            }
            Some(_) => {}
            None => unreachable!("a picture at {t:?} before the end"),
        }
    }
}

#[wasm_bindgen_test]
async fn the_element_plays_hls_remuxed_to_fragmented_mp4() {
    web_sys::console::log_1(&probe().into());
    if INSECURE {
        assert!(!secure(), "served from the LAN address, the page is not a secure context");
    }
    let Some(dir) = HLS_DIR else {
        web_sys::console::log_1(&"REMEDY_HLS_URL not set at build time: skipping".into());
        return;
    };
    let host = host();
    assert!(host.name().contains("media element"), "{}", host.name());
    let before = remedy_ui_media_playback::live_platform_objects();
    for (name, rate, length) in [("aac", Some(48_000), 3u64), ("mp3", Some(44_100), 2), ("video", None, 3)] {
        let mut dec = host.open(&MediaSource::url(format!("{dir}/{name}/index.m3u8"))).must("opening");
        ready(&mut dec).await;
        let info = dec.info().clone();
        web_sys::console::log_1(&format!("{name}: {info:?}").into());
        let video = info.video.clone().must("a picture");
        assert_eq!((video.width, video.height), (176, 100));
        assert_eq!(info.audio.map(|a| a.sample_rate), rate);
        assert_eq!(info.duration, Some(Duration::from_secs(length)));
        let mut playout = dec.playout().must("the element plays it");
        assert!(dec.playout().is_none(), "handed over once");
        // Paused, each seek shows the picture there.
        for (ms, want) in [(500u64, 'r'), (1500, 'g'), (2500, 'b'), (300, 'r')] {
            if ms >= length * 1000 {
                continue;
            }
            let to = Duration::from_millis(ms);
            playout.seek(to);
            dec.seek(to).must("seeking");
            let (at, p) = picture_at(&mut dec, to).await;
            assert!(at < to + Duration::from_millis(100), "{name}: the picture at {ms} ms came at {at:?}");
            assert_eq!(hue(&p), want, "{name}: the picture at {ms} ms");
            assert_eq!((p.width, p.height), (176, 100));
        }
        // Playing: the clock is the element's and moves on; the pictures follow it to the end.
        playout.play();
        let mut last = Duration::ZERO;
        let mut shown = 0;
        while let Some(s) = next(&mut dec).await {
            if let Sample::Video { at, .. } = s {
                assert!(at + Duration::from_millis(100) >= last, "{name}: pictures in order");
                last = last.max(at);
                shown += 1;
            }
        }
        assert!(shown > 10, "{name}: {shown} pictures shown while playing");
        assert!(
            last + Duration::from_millis(150) >= Duration::from_secs(length),
            "{name}: the last at {last:?}"
        );
        assert!(playout.position() + Duration::from_millis(100) >= Duration::from_secs(length));
        drop(playout);
        drop(dec);
        assert_eq!(videos(), 0, "{name}: the element is gone with its decoder");
    }
    assert_eq!(remedy_ui_media_playback::live_platform_objects(), before, "nothing left behind");
}

async fn wait_for(p: &Player, what: &str, ok: impl Fn(&Player) -> bool) {
    for _ in 0..1500 {
        if ok(p) {
            return;
        }
        wait_ms(10).await;
    }
    unreachable!(
        "{what}: state {:?} at {:?}, error {:?}, {:?}, element at {} ready {}",
        p.state(),
        p.position(),
        p.error(),
        p.stats(),
        video_prop("currentTime"),
        video_prop("readyState")
    );
}

#[wasm_bindgen_test]
async fn a_player_on_the_element_plays_seeks_switches_and_ends() {
    let Some(dir) = HLS_DIR else { return };
    let url = |name: &str| MediaSource::url(format!("{dir}/{name}/index.m3u8"));
    let before = remedy_ui_media_playback::live_platform_objects();
    spy_frames();
    small_mount();
    let slot: Rc<RefCell<Option<Player>>> = Rc::default();
    let app: Rc<RefCell<Option<remedy_ui::App>>> = Rc::default();
    let (s2, a2, first) = (slot.clone(), app.clone(), url("aac"));
    remedy_ui::run(
        remedy_ui::AppConfig::new("Element playback test").cpu_only(),
        move |a: &remedy_ui::App| {
            *a2.borrow_mut() = Some(a.clone());
            let s3 = s2.clone();
            a.open(remedy_ui::WindowSpec::new("Element playback test"), move |ui, _| {
                let p = Player::open_platform(ui, first, PlayerOptions::default().web_backend(backend()));
                let view =
                    remedy_ui_media_playback::player(ui, &p, "Element clip").width(352.0).height(260.0);
                let _ = ui.root().child(view);
                *s3.borrow_mut() = Some(p);
            });
        },
    )
    .must("the app starts");
    for _ in 0..600 {
        if slot.borrow().is_some() {
            break;
        }
        wait_ms(10).await;
    }
    let p = slot.borrow().clone().must("the player");
    wait_for(&p, "opened", |p| p.state() == PlaybackState::Paused).await;
    assert_eq!(p.duration(), Some(Duration::from_secs(3)));
    let seen = p.stats().frames_shown;
    p.seek(Duration::from_millis(1200));
    wait_for(&p, "the seek", |p| (p.position().as_secs_f64() - 1.2).abs() < 0.001).await;
    wait_for(&p, "the picture there", |p| p.stats().frames_shown > seen).await;
    // Volume (loudness follows its square) and speed go to the element.
    p.set_volume(0.5);
    wait_for(&p, "the volume", |_| (video_prop("volume") - 0.25).abs() < 1e-6).await;
    p.set_speed(1.5);
    wait_for(&p, "the speed", |_| (video_prop("playbackRate") - 1.5).abs() < 1e-6).await;
    p.set_speed(1.0);
    // Captions follow the element's clock.
    let cues = parse_subtitles("WEBVTT\n\n00:00.000 --> 00:10.000\nHello\n", SubtitleFormat::WebVtt);
    let track = SubtitleTrack { label: "Added".into(), language: None, cues };
    p.set_caption_track(Some(p.add_subtitles(track)));
    p.play();
    wait_for(&p, "playing", |p| p.state() == PlaybackState::Playing).await;
    wait_for(&p, "time moved on", |p| p.position() >= Duration::from_millis(1700)).await;
    assert_eq!(video_prop("paused"), 0.0, "the element plays");
    assert_eq!(p.caption_text().as_deref(), Some("Hello"));
    p.pause();
    wait_for(&p, "paused", |p| p.state() == PlaybackState::Paused).await;
    assert_eq!(video_prop("paused"), 1.0, "the element paused");
    p.play();
    wait_for(&p, "the end", |p| p.state() == PlaybackState::Ended).await;
    let stats = p.stats();
    web_sys::console::log_1(
        &format!("element player: {stats:?}, frame callbacks {}: {}", frame_calls(), frame_log()).into(),
    );
    // A debug build's CPU renderer holds the page's thread a while for each picture it draws,
    // so the browser calls back for a few of the element's frames only.
    assert!(stats.frames_shown >= 2, "{stats:?}");
    assert!((p.position().as_secs_f64() - 3.0).abs() < 0.05, "{:?}", p.position());
    // A stream of the same work from 60 s in (a server's skip), on its timeline.
    p.replace_source(url("video"), Timeline::new(Duration::from_secs(60)).length(Duration::from_secs(63)));
    wait_for(&p, "the new stream plays", |p| {
        p.state() == PlaybackState::Playing && p.position() >= Duration::from_millis(60_300)
    })
    .await;
    assert_eq!(p.duration(), Some(Duration::from_secs(63)), "the work's length");
    assert_eq!(videos(), 1, "the old stream's element is gone");
    p.seek(Duration::from_millis(61_500));
    wait_for(&p, "a seek in the work's time", |p| p.position() >= Duration::from_millis(61_500)).await;
    // A quality or sound switch: a new stream of the same place.
    p.hold_for_source(p.position());
    p.replace_source(url("mp3"), Timeline::new(Duration::from_secs(60)).length(Duration::from_secs(62)));
    wait_for(&p, "the switched stream ends", |p| p.state() == PlaybackState::Ended).await;
    assert!(p.error().is_none(), "{:?}", p.error());
    assert_eq!(videos(), 1);
    // The player goes with its window (the app's quitting would end the page's turns for its
    // workers first).
    slot.borrow_mut().take();
    drop(p);
    for w in app.borrow().as_ref().map(remedy_ui::App::windows).unwrap_or_default() {
        w.close();
    }
    for _ in 0..300 {
        if videos() == 0 && remedy_ui_media_playback::live_platform_objects() == before {
            break;
        }
        wait_ms(10).await;
    }
    assert_eq!(videos(), 0, "no element is left once the player goes");
    assert_eq!(remedy_ui_media_playback::live_platform_objects(), before, "nothing left behind");
    if let Some(a) = app.borrow().as_ref() {
        a.quit();
    }
}
