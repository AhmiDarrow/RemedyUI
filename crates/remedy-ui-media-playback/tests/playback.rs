//! Playback through the platform's decoders (Media Foundation on Windows, GStreamer on
//! Linux) of a small generated clip (tests/fixtures), and the player's clock, controls and
//! widget on a real view. The engine's sync is also checked against a scripted host whose
//! sound clock the test moves by hand.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use remedy_ui_compositor::Layer;
use remedy_ui_core::Size;
use remedy_ui_input::{Key, KeyEvent};
use remedy_ui_media_playback::*;
use remedy_ui_paint::{Item, VideoFrame};
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};

const SIZE: Size = Size { width: 480.0, height: 320.0 };

fn clip() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip.mp4")
}

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

/// The platform's host, or `None` (with the reason printed) where this machine has none:
/// Linux without GStreamer. On Windows it must be there.
fn platform() -> Option<Arc<dyn MediaHost>> {
    let host = match platform_host() {
        Ok(h) => h,
        Err(e) if cfg!(windows) => unreachable!("Media Foundation must be available on Windows: {e}"),
        Err(e) => {
            eprintln!("no platform media host here, skipping: {e}");
            return None;
        }
    };
    // GStreamer's core can be installed without the plugins that decode (its base plugins
    // hold `playbin`): the host opens, then every file says what is missing.
    match host.open(&MediaSource::file(clip())) {
        Err(MediaError::Missing(d)) if !cfg!(windows) => {
            eprintln!("the platform media host cannot decode here, skipping: {d}");
            None
        }
        _ => Some(host),
    }
}

/// Which of red, green and blue a BT.709 video-range NV12 or I420 picture is at its middle.
fn hue(
    width: u32,
    height: u32,
    planes: &[remedy_ui_paint::Plane; 3],
    data: &[u8],
    format: PixelFormat,
) -> char {
    let (x, y) = ((width / 2) as usize, (height / 2) as usize);
    let at =
        |p: usize, off: usize| planes.get(p).and_then(|pl| data.get(pl.offset + off)).copied().unwrap_or(0);
    let stride = |p: usize| planes.get(p).map_or(0, |pl| pl.stride);
    let yv = f32::from(at(0, y * stride(0) + x));
    let (u, v) = match format {
        PixelFormat::Nv12 => {
            let o = (y / 2) * stride(1) + (x / 2) * 2;
            (f32::from(at(1, o)), f32::from(at(1, o + 1)))
        }
        PixelFormat::I420 => {
            (f32::from(at(1, (y / 2) * stride(1) + x / 2)), f32::from(at(2, (y / 2) * stride(2) + x / 2)))
        }
        PixelFormat::Rgba => return '?',
    };
    let (c, d, e) = (yv - 16.0, u - 128.0, v - 128.0);
    let r = 1.164 * c + 1.793 * e;
    let g = 1.164 * c - 0.213 * d - 0.533 * e;
    let b = 1.164 * c + 2.112 * d;
    if r > g && r > b {
        'r'
    } else if g > b {
        'g'
    } else {
        'b'
    }
}

fn picture_hue(p: &Picture) -> char {
    hue(p.width, p.height, &p.planes, &p.data, p.format)
}

fn frame_hue(f: &VideoFrame) -> char {
    hue(f.width, f.height, &f.planes, &f.data, f.format)
}

fn near(a: Duration, secs: f64, tol: f64) -> bool {
    (a.as_secs_f64() - secs).abs() <= tol
}

#[test]
fn the_platform_decoder_reads_pictures_and_sound_in_time() {
    let Some(host) = platform() else { return };
    let mut dec = host.open(&MediaSource::file(clip())).unwrap_or_else(|e| unreachable!("open: {e}"));
    let info = dec.info().clone();
    // The pipeline's buffering answer, when it gives one, lies within the media.
    if let Some(ranges) = dec.buffered() {
        eprintln!("the decoder holds {ranges:?}");
        let end = info.duration.unwrap_or(Duration::MAX);
        assert!(
            ranges.iter().all(|r| r.start <= r.end && r.end <= end + Duration::from_millis(100)),
            "{ranges:?}"
        );
    }
    assert!(info.duration.is_some_and(|d| near(d, 3.0, 0.1)), "{info:?}");
    assert!(info.seekable);
    let v = info.video.clone().unwrap_or_else(|| unreachable!("a picture: {info:?}"));
    // The coded picture is 112 rows; the display aperture is what shows.
    assert_eq!((v.width, v.height), (176, 100));
    assert!(v.frame_rate.is_some_and(|r| (r - 25.0).abs() < 0.01), "{v:?}");
    assert_eq!(info.audio, Some(AudioFormat::new(48_000, 2)));

    let (mut frames, mut sound, mut last_video) = (Vec::new(), 0u64, None);
    while let Some(s) = dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) {
        match s {
            Sample::Video { at, picture, .. } => {
                assert!(last_video.is_none_or(|l| at > l), "pictures in time order");
                last_video = Some(at);
                assert_eq!((picture.width, picture.height), (176, 100));
                // MediaCodec gives I420 on some devices; the others give NV12.
                assert!(
                    matches!(picture.format, PixelFormat::Nv12 | PixelFormat::I420),
                    "{:?}",
                    picture.format
                );
                frames.push((at, picture_hue(&picture)));
            }
            Sample::Audio { samples, .. } => sound += samples.len() as u64 / 2,
            Sample::Text { .. } => {}
            other => unreachable!("this clip has no other samples: {other:?}"),
        }
    }
    assert_eq!(frames.len(), 75, "every picture of 3 s at 25 fps");
    let hue_at = |t: f64| frames.iter().find(|(at, _)| near(*at, t, 0.021)).map(|f| f.1);
    assert_eq!(
        (hue_at(0.0), hue_at(1.0), hue_at(2.0), hue_at(2.96)),
        (Some('r'), Some('g'), Some('b'), Some('b'))
    );
    // 3 s at 48 kHz, give or take the encoder's priming.
    assert!((140_000..=150_000).contains(&sound), "{sound} sound frames");

    // A seek in a file with one key frame decodes from it; what comes is at or before the
    // target, and pictures after the target are those of the target's second.
    dec.seek(Duration::from_millis(1500)).unwrap_or_else(|e| unreachable!("seek: {e}"));
    let mut after = None;
    while let Some(s) = dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) {
        if let Sample::Video { at, picture, .. } = s
            && at >= Duration::from_millis(1500)
        {
            after = Some((at, picture_hue(&picture)));
            break;
        }
    }
    let (at, h) = after.unwrap_or_else(|| unreachable!("a picture after the seek"));
    assert!(near(at, 1.5, 0.041), "{at:?}");
    assert_eq!(h, 'g');
}

/// Captions carried inside an MP4 (timed text, the cues of `clip.vtt`) reach the player on
/// every platform decoder: they are read from the file on all of them (GStreamer hands its
/// cues out seconds late), and shown paused right after a seek.
#[test]
fn the_player_shows_captions_embedded_in_an_mp4() {
    let Some(host) = platform() else { return };
    let mut s = Screen::new();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip-subs.mp4");
    let p = Player::open(&s.ui, host, MediaSource::file(path), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Test clip"));
    s.wait("opened with its captions", |_| {
        p.state() == PlaybackState::Paused && !p.subtitle_tracks().is_empty()
    });
    let tracks = p.subtitle_tracks();
    assert_eq!(tracks.len(), 1, "one embedded track, not two: {tracks:?}");
    let first = tracks.first().map(|t| (t.label.as_str(), t.language.as_deref()));
    assert_eq!(first, Some(("eng", Some("eng"))), "the file's own track, as it names itself");
    p.set_caption_track(Some(0));
    p.seek(Duration::from_millis(1200));
    s.wait("the cue at 1.2 s", |_| p.caption_text().as_deref() == Some("One"));
    p.seek(Duration::from_millis(2000));
    s.wait("the cue at 2 s", |_| p.caption_text().as_deref() == Some("Two & more"));
}

/// Text subtitles carried inside Matroska and WebM (SRT, ASS and WebVTT tracks made from
/// `clip.vtt` and `clip.fr.srt`) reach the player on every platform decoder, read from the
/// file: Media Foundation, AVFoundation and MediaCodec do not hand them out.
#[test]
fn the_player_shows_captions_embedded_in_matroska_and_webm() {
    let Some(host) = platform() else { return };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    // GStreamer may lack the WebM decoders (VP8, Opus) where it has H.264 and AAC.
    let webm = host.open(&MediaSource::file(dir.join("clip-subs.webm"))).is_ok();
    // Each file, and the label and language of each of its tracks.
    type Tracks = &'static [(&'static str, Option<&'static str>)];
    let files: &[(&str, Tracks)] = &[
        ("clip-subs.mkv", &[("eng", Some("eng")), ("fre", Some("fre")), ("Subtitles", None)]),
        ("clip-subs.webm", &[("eng", Some("eng"))]),
    ];
    for (name, want) in files {
        if *name == "clip-subs.webm" && !webm {
            eprintln!("the platform media host cannot decode WebM here, skipping it");
            continue;
        }
        let mut s = Screen::new();
        let p =
            Player::open(&s.ui, host.clone(), MediaSource::file(dir.join(name)), PlayerOptions::default());
        s.ui.root().child(player(&s.ui, &p, "Test clip"));
        s.wait("opened with its captions", |_| {
            p.state() == PlaybackState::Paused && !p.subtitle_tracks().is_empty()
        });
        let tracks = p.subtitle_tracks();
        let got: Vec<_> = tracks.iter().map(|t| (t.label.as_str(), t.language.as_deref())).collect();
        assert_eq!(&got, want, "{name}: the file's own tracks, not the decoder's too");
        p.set_caption_track(Some(0));
        p.seek(Duration::from_millis(1200));
        s.wait("the cue at 1.2 s", |_| p.caption_text().as_deref() == Some("One"));
        p.seek(Duration::from_millis(2000));
        s.wait("the cue at 2 s", |_| p.caption_text().as_deref() == Some("Two & more"));
        if name.ends_with(".mkv") {
            // The ASS track.
            p.set_caption_track(Some(1));
            s.wait("the French cue", |_| p.caption_text().as_deref() == Some("Un"));
        }
    }
}

/// The captions come from the file, whatever decoder plays it: a scripted one that hands out
/// no text at all shows the Matroska file's tracks too.
#[test]
fn embedded_matroska_captions_do_not_depend_on_the_decoder() {
    let host = Arc::new(Scripted {
        heard: Arc::new(AtomicU64::new(0)),
        buffer: Duration::from_millis(200),
        device: false,
        seconds: 3,
    });
    let mut s = Screen::new();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip-subs-streamed.mkv");
    let p = Player::open(&s.ui, host, MediaSource::file(path), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Scripted"));
    s.wait("opened with its captions", |_| {
        p.state() == PlaybackState::Paused && p.subtitle_tracks().len() == 3
    });
    p.set_caption_track(Some(0));
    p.seek(Duration::from_millis(1200));
    s.wait("the cue at 1.2 s", |_| p.caption_text().as_deref() == Some("One"));
    p.seek(Duration::from_millis(2000));
    s.wait("the cue at 2 s", |_| p.caption_text().as_deref() == Some("Two & more"));
}

#[test]
fn a_missing_file_says_so() {
    let Some(host) = platform() else { return };
    match host.open(&MediaSource::file("no/such/clip.mp4")) {
        Err(MediaError::NotFound(_)) => {}
        other => unreachable!("{:?}", other.map(|d| d.info().clone())),
    }
}

/// A window the test drives with real time.
struct Screen {
    ui: Ui,
    frame: Option<VideoFrame>,
    /// The layers of the latest frame that drew any.
    layers: Vec<Arc<Layer>>,
    /// Timers the window asked for, and when each is due.
    timers: Vec<(u64, Instant)>,
}

fn videos(layer: &Layer, out: &mut Option<VideoFrame>) {
    for item in layer.content.items() {
        if let Item::Video { frame, .. } = item {
            *out = Some(frame.clone());
        }
    }
    for c in &layer.children {
        videos(c, out);
    }
}

impl Screen {
    fn new() -> Screen {
        Screen { ui: ui(), frame: None, layers: Vec::new(), timers: Vec::new() }
    }

    fn pump(&mut self) {
        let _ = remedy_ui_reactive::run_posted();
        let f = self.ui.frame(SIZE, 1.0);
        if let Some(tree) = &f.layers {
            let mut v = None;
            for r in &tree.roots {
                videos(r, &mut v);
            }
            if v.is_some() {
                self.frame = v;
            }
            self.layers = tree.roots.clone();
        }
        for r in self.ui.take_requests() {
            match r {
                remedy_ui_view::Request::Timer { id, delay } => {
                    self.timers.push((id, Instant::now() + delay))
                }
                remedy_ui_view::Request::CancelTimer(id) => self.timers.retain(|(t, _)| *t != id),
                _ => {}
            }
        }
        let now = Instant::now();
        let due: Vec<u64> = self.timers.iter().filter(|(_, at)| *at <= now).map(|(id, _)| *id).collect();
        self.timers.retain(|(_, at)| *at > now);
        for id in due {
            self.ui.fire_timer(id);
        }
    }

    fn wait(&mut self, what: &str, ok: impl Fn(&Screen) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if ok(self) {
                return;
            }
            // Generous: a system's GStreamer can spend seconds probing hardware decoders
            // (CUDA under WSL) the first time a pipeline is built.
            assert!(start.elapsed() < Duration::from_secs(60), "waited for {what}");
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn idle(&mut self, ms: u64) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(ms) {
            self.pump();
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn hue(&self) -> char {
        self.frame.as_ref().map_or('-', frame_hue)
    }
}

fn press(ui: &Ui, key: Key) -> bool {
    ui.key(&KeyEvent::press(key).with_text(None))
}

fn find(el: &El, widget: &str) -> Vec<El> {
    let mut out = Vec::new();
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().is_some_and(|i| i.widget == widget) {
            out.push(e.clone());
        }
        let mut kids = e.child_elements();
        kids.reverse();
        stack.extend(kids);
    }
    out
}

/// Text without the Unicode isolation marks the string catalogs put around placeholders.
fn plain(s: &str) -> String {
    s.replace(['\u{2068}', '\u{2069}'], "")
}

fn named(el: &El, name: &str) -> Option<El> {
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().and_then(|i| i.semantics).is_some_and(|s| s.name().map(plain).as_deref() == Some(name)) {
            return Some(e);
        }
        stack.extend(e.child_elements());
    }
    None
}

#[test]
fn the_player_plays_seeks_and_ends_on_the_platform_decoders() {
    let Some(host) = platform() else { return };
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::file(clip()), PlayerOptions::default());
    let view = player(&s.ui, &p, "Test clip");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    assert!(p.duration().is_some_and(|d| near(d, 3.0, 0.1)));
    // A file on this machine is there to play whole.
    let len = p.duration().unwrap_or_default();
    s.wait("buffered", |_| !p.buffered().is_empty());
    assert_eq!(p.buffered(), vec![Duration::ZERO..len], "a local file is held whole");
    // Paused at the start, the first picture shows.
    s.wait("the first picture", |s| s.hue() == 'r');
    assert!(near(p.position(), 0.0, 0.001));

    // The sidecar captions were found: clip.fr.srt and clip.vtt.
    let tracks = p.subtitle_tracks();
    let labels: Vec<&str> = tracks.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, ["fr", "Subtitles"], "{tracks:?}");
    assert_eq!(tracks.first().and_then(|t| t.language.as_deref()), Some("fr"));

    // A seek while paused lands, and shows its picture.
    p.seek(Duration::from_millis(1200));
    s.wait("the picture at 1.2 s", |s| s.hue() == 'g');
    assert!(near(p.position(), 1.2, 0.001), "{:?}", p.position());

    // Captions: the WebVTT track shows its cue at the position.
    p.set_caption_track(Some(1));
    s.pump();
    assert_eq!(p.caption_text().as_deref(), Some("One"));
    let caption =
        find(&view, "player_caption").into_iter().next().unwrap_or_else(|| unreachable!("captions"));
    assert_eq!(caption.info().map(|i| (i.hidden, i.text)), Some((false, Some("One".into()))));

    // Play: time advances with the clock and pictures follow.
    let started = Instant::now();
    p.play();
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    s.wait("the next cue", |_| p.caption_text().as_deref() == Some("Two & more"));
    s.wait("the third second", |s| p.position() >= Duration::from_millis(2100) && s.hue() == 'b');
    let elapsed = started.elapsed().as_secs_f64();
    let moved = p.position().as_secs_f64() - 1.2;
    // The clock runs at the media's rate (sound device or wall clock), not as fast as
    // decoding: within scheduling, and within what a sound server may skip after a hiccup.
    assert!(moved <= elapsed + 0.3, "position ran ahead of time: {moved} in {elapsed}");
    s.wait("the end", |_| p.state() == PlaybackState::Ended);
    assert!(near(p.position(), 3.0, 0.05), "{:?}", p.position());
    let stats = p.stats();
    assert!(stats.frames_shown >= 30, "{stats:?}");
    match p.audio_error() {
        None => {
            assert!(stats.audio_frames > 0, "sound went to the device: {stats:?}");
            eprintln!("played on the sound device's clock: {stats:?}");
        }
        Some(e) => eprintln!("no sound device here ({e}); played by the wall clock"),
    }

    // Playing again after the end starts over.
    p.play();
    s.wait("playing from the start", |_| {
        p.state() == PlaybackState::Playing && p.position() < Duration::from_secs(1)
    });
    p.pause();
    s.wait("paused", |_| p.state() == PlaybackState::Paused);
    let held = p.position();
    s.idle(150);
    assert_eq!(p.position(), held, "a paused player holds still");
}

#[test]
fn the_player_widget_is_operable_by_keyboard_and_passes_the_audit() {
    let Some(host) = platform() else { return };
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::file(clip()), {
        let mut v_ = PlayerOptions::default();
        v_.volume = 0.5;
        v_
    });
    let asked = Arc::new(Mutex::new(Vec::new()));
    let a = asked.clone();
    p.on_fullscreen(move |on| a.lock().unwrap_or_else(|e| e.into_inner()).push(on));
    let view = player(&s.ui, &p, "Test clip");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused && p.subtitle_tracks().len() == 2);
    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");

    // The picture fills the stage above the transport bar.
    let surface =
        find(&view, "video_surface").into_iter().next().unwrap_or_else(|| unreachable!("a surface"));
    let b = surface.bounds().unwrap_or_else(|| unreachable!("laid out"));
    assert!(b.width() >= SIZE.width - 1.0 && b.height() > SIZE.height / 2.0, "{b:?}");

    // Every control is named; the seek bar is a slider with the position in words.
    for name in ["Play", "Mute", "Volume", "Captions: off", "Full screen", "Seek"] {
        assert!(named(&view, name).is_some(), "no control named {name}");
    }
    let seek = named(&view, "Seek").unwrap_or_else(|| unreachable!());
    let sem = seek.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!());
    assert_eq!(sem.role(), remedy_ui_a11y::Role::Slider);
    assert_eq!(sem.value_text().map(plain).as_deref(), Some("0:00 of 0:03"));

    // The seek bar takes the arrows (5 s, clamped to the end) and Home.
    seek.focus();
    assert!(press(&s.ui, Key::ArrowRight));
    s.wait("moved to the end", |_| near(p.position(), 3.0, 0.05));
    assert!(press(&s.ui, Key::Home));
    s.wait("back at the start", |_| near(p.position(), 0.0, 0.001));

    // Space on the seek bar plays and pauses; anywhere in the player M mutes, C turns
    // captions on, F asks for full screen.
    assert!(press(&s.ui, Key::Space));
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    assert!(named(&view, "Pause").is_some(), "the button now pauses");
    assert!(press(&s.ui, Key::Space));
    s.wait("paused", |_| p.state() == PlaybackState::Paused);
    assert!(press(&s.ui, Key::Character("m".into())));
    s.pump();
    assert!(p.is_muted() && named(&view, "Unmute").is_some());
    assert!(press(&s.ui, Key::Character("c".into())));
    s.pump();
    assert_eq!(p.caption_track(), Some(0));
    assert!(named(&view, "Captions: fr").is_some());
    assert!(press(&s.ui, Key::Character("f".into())));
    s.pump();
    assert!(p.is_fullscreen() && named(&view, "Exit full screen").is_some());
    assert!(press(&s.ui, Key::Escape));
    assert_eq!(*asked.lock().unwrap_or_else(|e| e.into_inner()), [true, false]);
    assert!((p.volume() - 0.5).abs() < 1e-6);

    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_player_that_cannot_open_shows_why() {
    let Some(host) = platform() else { return };
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::file("missing.mp4"), PlayerOptions::default());
    let view = player(&s.ui, &p, "Missing");
    s.ui.root().child(view.clone());
    s.wait("failed", |_| p.state() == PlaybackState::Failed);
    assert!(matches!(p.error(), Some(MediaError::NotFound(_))), "{:?}", p.error());
    let shown: Vec<String> = find(&view, "player_caption")
        .iter()
        .filter_map(|l| l.info().filter(|i| !i.hidden).and_then(|i| i.text))
        .collect();
    assert!(shown.iter().any(|t| t.starts_with("This can't be played")), "{shown:?}");
    s.ui.set_locale("de");
    s.pump();
    let shown: Vec<String> = find(&view, "player_caption")
        .iter()
        .filter_map(|l| l.info().filter(|i| !i.hidden).and_then(|i| i.text))
        .collect();
    assert!(shown.iter().any(|t| t.starts_with("Dies kann nicht wiedergegeben werden")), "{shown:?}");
    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
}

// A scripted host: sound and pictures from a pattern, and a sound device whose clock the
// test moves by hand, to show pictures follow the sound clock and not the wall clock.

/// Frames per second of the scripted pictures.
const FPS: u64 = 10;
const RATE: u32 = 1000;

struct Scripted {
    /// Sound frames the scripted device has played.
    heard: Arc<AtomicU64>,
    /// The device's buffer (a device that stops for twice this long is stalled).
    buffer: Duration,
    /// Whether a device can be opened.
    device: bool,
    seconds: u64,
}

struct ScriptedDecoder {
    info: MediaInfo,
    next: u64,
    seconds: u64,
}

impl Decoder for ScriptedDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        // Alternating sound (a tenth of a second) and the picture of the same tenth.
        let tenth = self.next / 2;
        if tenth >= self.seconds * FPS {
            return Ok(None);
        }
        let at = Duration::from_millis(tenth * 100);
        let s = if self.next.is_multiple_of(2) {
            Sample::Audio { at, samples: vec![0.0; (RATE / 10) as usize] }
        } else {
            // Luma tells the tenth: 20 + 2 per tenth.
            let y = u8::try_from(20 + 2 * tenth).unwrap_or(255);
            let mut data = vec![y; 8 * 8];
            data.extend(std::iter::repeat_n(128u8, 8 * 4));
            Sample::Video {
                at,
                duration: Some(Duration::from_millis(100)),
                picture: Picture::packed(8, 8, PixelFormat::Nv12, data),
            }
        };
        self.next += 1;
        Ok(Some(s))
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.next = (to.as_millis() as u64 / 100) * 2;
        Ok(())
    }
}

struct ScriptedOut {
    heard: Arc<AtomicU64>,
    buffer: Duration,
    written: u64,
    playing: Arc<AtomicBool>,
}

impl AudioOutput for ScriptedOut {
    fn format(&self) -> AudioFormat {
        AudioFormat::new(RATE, 1)
    }
    fn buffer(&self) -> Duration {
        self.buffer
    }
    fn room(&mut self) -> Result<usize, MediaError> {
        // The device holds 200 ms beyond what was heard.
        let heard = self.heard.load(Ordering::SeqCst);
        Ok((heard + 200).saturating_sub(self.written) as usize)
    }
    fn write(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        self.written += samples.len() as u64;
        Ok(())
    }
    fn played(&mut self) -> Result<u64, MediaError> {
        Ok(self.heard.load(Ordering::SeqCst).min(self.written))
    }
    fn pause(&mut self) -> Result<(), MediaError> {
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn resume(&mut self) -> Result<(), MediaError> {
        self.playing.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), MediaError> {
        self.heard.store(0, Ordering::SeqCst);
        self.written = 0;
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }
}

impl MediaHost for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        Ok(Box::new(ScriptedDecoder {
            info: {
                let mut v_ = MediaInfo::default();
                v_.duration = Some(Duration::from_secs(self.seconds));
                v_.seekable = true;
                v_.video = Some(VideoInfo::new(8, 8, Some(FPS as f64)));
                v_.audio = Some(AudioFormat::new(RATE, 1));
                v_.text_tracks = Vec::new();
                v_
            },
            next: 0,
            seconds: self.seconds,
        }))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        if !self.device {
            return Err(MediaError::Device("no device in this script".into()));
        }
        Ok(Box::new(ScriptedOut {
            heard: self.heard.clone(),
            buffer: self.buffer,
            written: 0,
            playing: Arc::new(AtomicBool::new(false)),
        }))
    }
}

fn luma(f: &Option<VideoFrame>) -> u8 {
    f.as_ref().and_then(|f| f.data.first().copied()).unwrap_or(0)
}

#[test]
fn pictures_follow_the_sound_clock_not_the_wall_clock() {
    let heard = Arc::new(AtomicU64::new(0));
    let host =
        Arc::new(Scripted { heard: heard.clone(), buffer: Duration::from_secs(2), device: true, seconds: 3 });
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::url("script:"), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Scripted"));
    s.wait("opened", |s| p.state() == PlaybackState::Paused && luma(&s.frame) == 20);
    p.play();
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    // Nothing heard yet: however long the wall clock runs, the picture and position wait.
    s.idle(400);
    assert_eq!(luma(&s.frame), 20, "the first picture still shows");
    assert!(near(p.position(), 0.0, 0.001), "{:?}", p.position());
    // The device has played 1.05 s: the picture of the tenth at 1.0 s shows.
    heard.store(1050, Ordering::SeqCst);
    s.wait("one second in", |s| luma(&s.frame) == 40);
    assert!(near(p.position(), 1.05, 0.001), "{:?}", p.position());
    // A seek resets the device clock to the target.
    p.seek(Duration::from_millis(2000));
    s.wait("at two seconds", |s| near(p.position(), 2.0, 0.001) && luma(&s.frame) == 60);
    heard.store(500, Ordering::SeqCst);
    // The picture and the position arrive by separate posts: wait for both, as above.
    s.wait("two and a half seconds", |s| luma(&s.frame) == 70 && near(p.position(), 2.5, 0.001));
    // Hidden, a player with pictures pauses; seen again, it plays on.
    let shown = s.ui.root().child_elements();
    let view = shown.first().cloned().unwrap_or_else(|| unreachable!());
    let _ = view.clone().hidden(true);
    s.wait("paused while unseen", |_| p.state() == PlaybackState::Paused);
    let _ = view.hidden(false);
    s.wait("playing again", |_| p.state() == PlaybackState::Playing);
    // The sound runs out before the end: the rest plays by the wall clock to the end.
    heard.store(10_000, Ordering::SeqCst);
    s.wait("ended", |_| p.state() == PlaybackState::Ended);
    assert!(near(p.position(), 3.0, 0.001));
}

#[test]
fn with_no_sound_device_it_plays_silently_by_the_wall_clock() {
    let host = Arc::new(Scripted {
        heard: Arc::new(AtomicU64::new(0)),
        buffer: Duration::from_millis(200),
        device: false,
        seconds: 1,
    });
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::url("script:"), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Scripted"));
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    let started = Instant::now();
    p.play();
    s.wait("ended", |_| p.state() == PlaybackState::Ended);
    let took = started.elapsed().as_secs_f64();
    assert!((0.95..1.6).contains(&took), "a second of media took {took} s");
    assert!(matches!(p.audio_error(), Some(MediaError::Device(_))), "{:?}", p.audio_error());
}

#[test]
fn autoplay_waits_for_the_person_under_reduced_motion() {
    for reduced in [false, true] {
        let host = Arc::new(Scripted {
            heard: Arc::new(AtomicU64::new(0)),
            buffer: Duration::from_millis(200),
            device: false,
            seconds: 2,
        });
        let mut s = Screen::new();
        s.ui.set_reduced_motion(reduced);
        let p = Player::open(&s.ui, host, MediaSource::url("script:"), {
            let mut v_ = PlayerOptions::default();
            v_.autoplay = true;
            v_
        });
        s.ui.root().child(player(&s.ui, &p, "Scripted"));
        s.wait("opened", |_| p.info().is_some());
        s.idle(150);
        let want = if reduced { PlaybackState::Paused } else { PlaybackState::Playing };
        assert_eq!(p.state(), want, "reduced motion {reduced}");
    }
}

#[test]
fn a_device_that_stops_playing_hands_the_clock_to_the_wall() {
    let heard = Arc::new(AtomicU64::new(0));
    let host = Arc::new(Scripted {
        heard: heard.clone(),
        buffer: Duration::from_millis(100),
        device: true,
        seconds: 2,
    });
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::url("script:"), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Scripted"));
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.play();
    // The device takes sound but never plays it: after twice its buffer the pictures go on.
    s.wait("moving without the device", |_| p.position() >= Duration::from_millis(300));
    // It plays again: its clock leads once more.
    heard.store(1500, Ordering::SeqCst);
    s.wait("ended", |_| p.state() == PlaybackState::Ended);
    assert!(near(p.position(), 2.0, 0.001));
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_view::strings::check(&remedy_ui_media_playback::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn ranges_merge_in_order_within_the_media() {
    let ms = Duration::from_millis;
    assert_eq!(merge_ranges(vec![], None), vec![]);
    assert_eq!(
        merge_ranges(vec![ms(500)..ms(900), ms(0)..ms(200), ms(150)..ms(300), ms(900)..ms(1000)], None),
        vec![ms(0)..ms(300), ms(500)..ms(1000)],
        "sorted; overlapping and touching ones joined"
    );
    assert_eq!(merge_ranges(vec![ms(100)..ms(100), ms(300)..ms(200)], None), vec![], "empty ones dropped");
    assert_eq!(
        merge_ranges(vec![ms(0)..ms(5000), ms(6000)..ms(7000)], Some(ms(3000))),
        vec![ms(0)..ms(3000)],
        "cut to the media's length"
    );
}

/// A scripted host whose source says how much of it has arrived, like a stream downloading.
struct Downloading {
    inner: Scripted,
    arrived: Arc<Mutex<Option<Vec<std::ops::Range<Duration>>>>>,
}

struct DownloadingDecoder {
    inner: Box<dyn Decoder>,
    arrived: Arc<Mutex<Option<Vec<std::ops::Range<Duration>>>>>,
}

impl Decoder for DownloadingDecoder {
    fn info(&self) -> &MediaInfo {
        self.inner.info()
    }
    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        self.inner.read()
    }
    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.inner.seek(to)
    }
    fn buffered(&mut self) -> Option<Vec<std::ops::Range<Duration>>> {
        self.arrived.lock().ok().and_then(|a| a.clone())
    }
}

impl MediaHost for Downloading {
    fn name(&self) -> &str {
        "downloading"
    }
    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        Ok(Box::new(DownloadingDecoder { inner: self.inner.open(source)?, arrived: self.arrived.clone() }))
    }
    fn audio_output(&self, f: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        self.inner.audio_output(f)
    }
}

#[test]
fn the_seek_bar_shows_what_has_arrived_and_what_is_decoded_ahead() {
    let ms = Duration::from_millis;
    let arrived = Arc::new(Mutex::new(Some(vec![ms(0)..ms(1500)])));
    let host = Arc::new(Downloading {
        inner: Scripted { heard: Arc::new(AtomicU64::new(0)), buffer: ms(200), device: false, seconds: 10 },
        arrived: arrived.clone(),
    });
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::url("script:"), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Scripted"));
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    s.wait("what arrived", |_| p.buffered().first().is_some_and(|r| r.start == ms(0) && r.end >= ms(1500)));
    // More arrives, in two stretches (a seek ahead made the source fetch from 6 s).
    if let Ok(mut a) = arrived.lock() {
        *a = Some(vec![ms(0)..ms(3000), ms(6000)..ms(8000), ms(9000)..ms(20_000)]);
    }
    s.wait("both stretches", |_| {
        let b = p.buffered();
        b.len() == 3 && b.get(1) == Some(&(ms(6000)..ms(8000))) && b.last().map(|r| r.end) == Some(ms(10_000))
    });
    // A source that says nothing: what is decoded ahead of the playhead still shows.
    if let Ok(mut a) = arrived.lock() {
        *a = None;
    }
    p.seek(ms(4000));
    s.wait("decoded ahead of 4 s", |_| {
        let b = p.buffered();
        b.len() == 1 && b.first().is_some_and(|r| r.start == ms(4000) && r.end > ms(4000))
    });
    // The bar draws the buffered stretch from the playhead on: a translucent tone of the text
    // color over the line (the played part, 0 to 4 s, is the accent).
    fn fills(layer: &Layer, out: &mut Vec<(remedy_ui_core::Rect, remedy_ui_core::Color)>) {
        for item in layer.content.items() {
            if let Item::FillRect { rect, brush: remedy_ui_paint::Brush::Solid(c), .. } = item {
                out.push((*rect, *c));
            }
        }
        for c in &layer.children {
            fills(c, out);
        }
    }
    s.idle(50);
    let mut all = Vec::new();
    for l in &s.layers {
        fills(l, &mut all);
    }
    let toned: Vec<_> = all.iter().filter(|(_, c)| (c.a - 0.35).abs() < 0.01).collect();
    assert_eq!(toned.len(), 1, "one buffered stretch drawn: {toned:?}");
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
}

/// Scripted clips by name: `script:N` lasts N seconds, with no sound device.
struct Clips;

impl MediaHost for Clips {
    fn name(&self) -> &str {
        "clips"
    }
    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        let seconds = source.to_url().trim_start_matches("script:").parse().unwrap_or(1);
        Scripted {
            heard: Arc::new(AtomicU64::new(0)),
            buffer: Duration::from_millis(200),
            device: false,
            seconds,
        }
        .open(source)
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("no device in this script".into()))
    }
}

/// Whether the "up next" card shows.
fn card_shown(view: &El) -> bool {
    find(view, "player_next").iter().any(|c| c.info().is_some_and(|i| !i.hidden))
}

fn texts(el: &El, widget: &str) -> Vec<String> {
    find(el, widget)
        .iter()
        .filter_map(|l| l.info().filter(|i| !i.hidden).and_then(|i| i.text))
        .map(|t| plain(&t))
        .collect()
}

#[test]
fn next_up_counts_down_at_the_end_and_plays_in_the_same_player() {
    let mut s = Screen::new();
    let options = PlayerOptions::default().next_up_delay(Some(Duration::from_millis(1500)));
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:1"), options);
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.set_next_up(Some(NextUp::new(MediaSource::url("script:3"), "The long one")));
    assert!(!card_shown(&view), "no card before the end");
    p.play();
    s.wait("ended", |_| p.state() == PlaybackState::Ended);
    s.wait("counting down", |_| p.countdown().is_some());
    assert!(card_shown(&view));
    let shown = texts(&view, "label");
    assert!(shown.iter().any(|t| t == "Up next: The long one"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Playing in 2 seconds"), "{shown:?}");
    s.wait("one second left", |_| texts(&view, "label").iter().any(|t| t == "Playing in 1 second"));
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "the card passes the audit: {problems:?}");
    // The countdown ends: the next plays here, from its start, with every signal its own.
    s.wait("the next playing", |_| {
        p.state() == PlaybackState::Playing && p.duration() == Some(Duration::from_secs(3))
    });
    assert_eq!(p.next_up(), None);
    assert_eq!(p.countdown(), None);
    assert!(p.position() < Duration::from_secs(1));
    assert!(!card_shown(&view), "the card went");
    // It ends with nothing next: nothing counts down.
    s.wait("the second end", |_| p.state() == PlaybackState::Ended);
    s.idle(100);
    assert_eq!(p.countdown(), None);
}

#[test]
fn cancelling_keeps_the_next_for_when_the_person_asks() {
    let mut s = Screen::new();
    let options = PlayerOptions::default().next_up_delay(Some(Duration::from_millis(300)));
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:1"), options);
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.set_next_up(Some(NextUp::new(MediaSource::url("script:2"), "Two")));
    p.play();
    s.wait("counting down", |_| p.countdown().is_some());
    // Cancel on the card: the countdown stops and the card goes.
    named(&view, "Cancel").unwrap_or_else(|| unreachable!("a cancel button")).focus();
    assert!(press(&s.ui, Key::Enter));
    s.idle(600);
    assert_eq!(p.state(), PlaybackState::Ended, "it stays at the end");
    assert_eq!(p.countdown(), None);
    assert!(p.next_up().is_some(), "the next is kept");
    // Shift+N plays it.
    s.pump();
    let shift = remedy_ui_input::Modifiers { shift: true, ..Default::default() };
    assert!(!card_shown(&view), "cancelled: the card is put away");
    named(&view, "Seek").unwrap_or_else(|| unreachable!("a seek bar")).focus();
    assert!(s.ui.key(&KeyEvent::press(Key::Character("N".into())).with_modifiers(shift).with_text(None)));
    s.wait("the next playing", |_| {
        p.state() == PlaybackState::Playing && p.duration() == Some(Duration::from_secs(2))
    });
}

#[test]
fn under_reduced_motion_the_next_waits_for_the_person() {
    let mut s = Screen::new();
    s.ui.set_reduced_motion(true);
    let options = PlayerOptions::default().next_up_delay(Some(Duration::from_millis(200)));
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:1"), options);
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.set_next_up(Some(NextUp::new(MediaSource::url("script:2"), "Two")));
    p.play();
    s.wait("ended", |_| p.state() == PlaybackState::Ended);
    s.idle(500);
    assert_eq!(p.countdown(), None, "no countdown");
    assert_eq!(p.state(), PlaybackState::Ended);
    assert!(card_shown(&view), "the card still offers it");
    assert!(texts(&view, "label").iter().any(|t| t == "Up next: Two"));
    // Play now, though: chosen, it plays even under reduced motion.
    p.play_next();
    s.wait("the next playing", |_| p.state() == PlaybackState::Playing);
}

#[test]
fn a_queue_plays_each_in_turn_then_stops() {
    let mut s = Screen::new();
    let options = PlayerOptions::default().next_up_delay(Some(Duration::from_millis(200)));
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:1"), options);
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.set_queue(vec![
        NextUp::new(MediaSource::url("script:2"), "Two"),
        NextUp::new(MediaSource::url("script:3"), "Three"),
    ]);
    assert_eq!(p.queue().len(), 2);
    assert_eq!(p.next_up().map(|n| n.title), Some("Two".into()));
    p.play();
    s.wait("counting down to two", |_| p.countdown().is_some());
    assert!(texts(&view, "label").iter().any(|t| t == "Up next: Two"));
    s.wait("two playing", |_| {
        p.state() == PlaybackState::Playing && p.duration() == Some(Duration::from_secs(2))
    });
    // Taken off the queue as it starts: three is next now, and its card waits for the end.
    assert_eq!(p.queue().iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Three"]);
    assert!(!card_shown(&view));
    s.wait("counting down to three", |_| p.countdown().is_some());
    assert!(card_shown(&view));
    assert!(texts(&view, "label").iter().any(|t| t == "Up next: Three"));
    s.wait("three playing", |_| {
        p.state() == PlaybackState::Playing && p.duration() == Some(Duration::from_secs(3))
    });
    assert!(p.queue().is_empty());
    s.wait("the last end", |_| p.state() == PlaybackState::Ended);
    s.idle(300);
    assert_eq!(p.countdown(), None);
    assert!(!card_shown(&view), "nothing next: no card");
}

#[test]
fn enqueueing_at_the_end_counts_down_and_emptying_the_queue_stops_it() {
    let mut s = Screen::new();
    let options = PlayerOptions::default().next_up_delay(Some(Duration::from_secs(5)));
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:1"), options);
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.play();
    s.wait("ended", |_| p.state() == PlaybackState::Ended);
    assert_eq!(p.countdown(), None, "nothing queued");
    p.enqueue(NextUp::new(MediaSource::url("script:2"), "Two"));
    p.enqueue(NextUp::new(MediaSource::url("script:3"), "Three"));
    s.wait("counting down", |_| p.countdown().is_some());
    assert_eq!(p.next_up().map(|n| n.title), Some("Two".into()), "first in, first next");
    assert!(card_shown(&view));
    p.set_queue(Vec::new());
    s.pump();
    assert_eq!(p.countdown(), None, "an empty queue stops the countdown");
    assert!(!card_shown(&view));
    assert_eq!(p.state(), PlaybackState::Ended);
    // Play now with nothing queued does nothing.
    p.play_next();
    s.idle(100);
    assert_eq!(p.state(), PlaybackState::Ended);
}

#[test]
fn speed_runs_the_clock_faster_or_slower_without_a_sound_device() {
    for (speed, seconds, want) in [(2.0, 2u64, 1.0f64), (0.5, 1, 2.0)] {
        let mut s = Screen::new();
        let p = Player::open(
            &s.ui,
            Arc::new(Clips),
            MediaSource::url(format!("script:{seconds}")),
            PlayerOptions::default(),
        );
        s.ui.root().child(player(&s.ui, &p, "Scripted"));
        s.wait("opened", |_| p.state() == PlaybackState::Paused);
        p.set_speed(speed);
        let started = Instant::now();
        p.play();
        s.wait("ended", |_| p.state() == PlaybackState::Ended);
        let took = started.elapsed().as_secs_f64();
        assert!((want * 0.95..want + 0.6).contains(&took), "{seconds} s at {speed}x took {took} s");
        assert!(p.stats().frames_shown >= 8, "pictures follow the clock: {:?}", p.stats());
    }
}

#[test]
fn speed_follows_the_sound_device_clock_and_changes_mid_play() {
    let heard = Arc::new(AtomicU64::new(0));
    let host =
        Arc::new(Scripted { heard: heard.clone(), buffer: Duration::from_secs(2), device: true, seconds: 6 });
    let mut s = Screen::new();
    let p = Player::open(&s.ui, host, MediaSource::url("script:"), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Scripted"));
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.set_speed(2.0);
    p.play();
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    // Half a second heard at twice the speed is a second of media.
    heard.store(500, Ordering::SeqCst);
    s.wait("a second in", |_| near(p.position(), 1.0, 0.03));
    // Back to normal speed: the sound already with the device plays out at its own speed,
    // what follows at the new one, and the clock never jumps back.
    p.set_speed(1.0);
    let before = p.position();
    heard.store(1500, Ordering::SeqCst);
    s.wait("moving on", |_| p.position() > before + Duration::from_millis(200));
    assert!(p.position() >= before);
    assert!(p.position() < Duration::from_secs(4), "{:?}", p.position());
    // Seeking while fast still lands where asked.
    p.set_speed(1.5);
    p.seek(Duration::from_secs(3));
    s.wait("at three seconds", |_| near(p.position(), 3.0, 0.001));
    assert!(p.stats().audio_frames > 0, "sound reached the device: {:?}", p.stats());
}

#[test]
fn the_speed_button_and_keys_step_through_the_speeds() {
    let mut s = Screen::new();
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:3"), PlayerOptions::default());
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    let speed =
        named(&view, "Speed 1×").unwrap_or_else(|| unreachable!("a speed button named for screen readers"));
    speed.focus();
    assert!(press(&s.ui, Key::Enter));
    s.pump();
    assert_eq!(p.speed(), 1.25);
    assert!(named(&view, "Speed 1.25×").is_some(), "its name follows");
    // > and < from anywhere in the player step up and down; the ends hold.
    for _ in 0..10 {
        assert!(s.ui.key(&KeyEvent::press(Key::Character(">".into()))));
    }
    assert_eq!(p.speed(), 2.0);
    assert!(s.ui.key(&KeyEvent::press(Key::Character("<".into()))));
    assert_eq!(p.speed(), 1.75);
    // In the window's language (an app's runner also writes the number the locale's way,
    // 1,75 in German).
    s.ui.set_locale("de");
    s.pump();
    assert!(named(&view, "Tempo 1.75×").or_else(|| named(&view, "Tempo 1,75×")).is_some(), "in German");
    // Hostile values are bounded.
    p.set_speed(f64::NAN);
    assert_eq!(p.speed(), 1.0);
    p.set_speed(100.0);
    assert_eq!(p.speed(), MAX_SPEED);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn speed_carries_over_to_what_plays_next() {
    let mut s = Screen::new();
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:1"), PlayerOptions::default());
    s.ui.root().child(player(&s.ui, &p, "Scripted"));
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    p.set_speed(2.0);
    p.set_next_up(Some(NextUp::new(MediaSource::url("script:2"), "Two")));
    p.play_next();
    s.wait("the next playing", |_| {
        p.state() == PlaybackState::Playing && p.duration() == Some(Duration::from_secs(2))
    });
    let started = Instant::now();
    s.wait("ended", |_| p.state() == PlaybackState::Ended);
    let took = started.elapsed().as_secs_f64();
    assert_eq!(p.speed(), 2.0);
    assert!(took < 1.6, "two seconds at 2x took {took} s");
}

#[test]
fn at_another_speed_the_end_of_the_sound_is_played_too() {
    for speed in [2.0, 0.5] {
        let heard = Arc::new(AtomicU64::new(0));
        let host = Arc::new(Scripted {
            heard: heard.clone(),
            buffer: Duration::from_millis(500),
            device: true,
            seconds: 2,
        });
        let mut s = Screen::new();
        let p = Player::open(&s.ui, host, MediaSource::url("script:"), PlayerOptions::default());
        s.ui.root().child(player(&s.ui, &p, "Scripted"));
        s.wait("opened", |_| p.state() == PlaybackState::Paused);
        p.set_speed(speed);
        p.play();
        // The device plays all it is given.
        heard.store(u64::MAX / 2, Ordering::SeqCst);
        s.wait("ended", |_| p.state() == PlaybackState::Ended);
        // Two seconds of media at 1000 frames a second, stretched: all of it, to within one
        // block (20 frames at this rate).
        let want = (2000.0 / speed) as u64;
        let got = p.stats().audio_frames;
        assert!(got + 20 >= want && got <= want + 20, "{speed}x: {got} of {want} frames reached the device");
    }
}

/// A wall-clock scripted player, paused at the start, in a player widget.
fn scripted(s: &mut Screen) -> (Player, El) {
    let host = Arc::new(Scripted {
        heard: Arc::new(AtomicU64::new(0)),
        buffer: Duration::from_millis(200),
        device: false,
        seconds: 3,
    });
    let p = Player::open(&s.ui, host, MediaSource::url("script:"), PlayerOptions::default());
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    (p, view)
}

#[test]
fn a_caption_delay_shifts_cues_later_or_earlier_across_seeks() {
    let mut s = Screen::new();
    let (p, _view) = scripted(&mut s);
    let cues = parse_subtitles("WEBVTT\n\n00:01.000 --> 00:01.500\nOne\n", SubtitleFormat::WebVtt);
    let track = SubtitleTrack { label: "Downloaded".into(), language: None, cues };
    p.set_caption_track(Some(p.add_subtitles(track)));
    let at = |s: &mut Screen, ms: u64| {
        p.seek(Duration::from_millis(ms));
        s.wait("the seek", |_| near(p.position(), ms as f64 / 1000.0, 0.001));
        p.caption_text()
    };
    assert_eq!(at(&mut s, 1200).as_deref(), Some("One"));
    // Later: the cue shows from 1.5 s to 2 s.
    p.set_caption_delay(500);
    assert_eq!(p.caption_delay(), 500);
    assert_eq!(p.caption_text(), None, "not yet at 1.2 s");
    assert_eq!(at(&mut s, 1700).as_deref(), Some("One"));
    assert_eq!(at(&mut s, 300), None);
    // Earlier: from 0.5 s to 1 s.
    p.set_caption_delay(-500);
    assert_eq!(at(&mut s, 600).as_deref(), Some("One"));
    assert_eq!(at(&mut s, 1200), None);
    // The next media's subtitles are other files: the shift starts over.
    p.set_next_up(Some(NextUp::new(MediaSource::url("script:"), "Next")));
    p.play_next();
    assert_eq!(p.caption_delay(), 0);
}

#[test]
fn theater_mode_is_offered_once_the_app_listens_and_escape_leaves_it() {
    let mut s = Screen::new();
    let (p, view) = scripted(&mut s);
    named(&view, "Seek").unwrap_or_else(|| unreachable!("a seek bar")).focus();
    s.idle(30);
    // No app listening: no button, and T does nothing.
    let hidden = |v: &El| named(v, "Theater mode").and_then(|b| b.info()).is_none_or(|i| i.hidden);
    assert!(hidden(&view));
    assert!(!press(&s.ui, Key::Character("t".into())));
    assert!(!p.is_theater());

    let asked = Arc::new(Mutex::new(Vec::new()));
    let a = asked.clone();
    p.on_theater(move |on| a.lock().unwrap_or_else(|e| e.into_inner()).push(on));
    s.pump();
    assert!(!hidden(&view));
    let pressed = |v: &El| {
        named(v, "Theater mode")
            .and_then(|b| b.info())
            .and_then(|i| i.semantics)
            .and_then(|s| s.checked_state())
    };
    assert_eq!(pressed(&view), Some(false));
    // T asks for it; the button shows it pressed.
    assert!(press(&s.ui, Key::Character("t".into())));
    s.pump();
    assert!(p.is_theater());
    assert_eq!(pressed(&view), Some(true));
    // Escape leaves full screen first, then theater mode.
    assert!(press(&s.ui, Key::Character("f".into())));
    assert!(press(&s.ui, Key::Escape));
    assert!(!p.is_fullscreen() && p.is_theater());
    assert!(press(&s.ui, Key::Escape));
    assert!(!p.is_theater());
    assert!(!press(&s.ui, Key::Escape), "nothing left to leave");
    // The button toggles it too.
    named(&view, "Theater mode").unwrap_or_else(|| unreachable!()).focus();
    assert!(press(&s.ui, Key::Space));
    s.pump();
    assert!(p.is_theater());
    assert_eq!(*asked.lock().unwrap_or_else(|e| e.into_inner()), [true, false, true]);

    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
}

/// A TV the scripted app casts to: it notes every press.
struct Tv(Arc<Mutex<Vec<String>>>);

impl Tv {
    fn note(&self, what: String) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(what);
    }
}

impl RemoteControl for Tv {
    fn play(&self) {
        self.note("play".into());
    }
    fn pause(&self) {
        self.note("pause".into());
    }
    fn seek(&self, to: Duration) {
        self.note(format!("seek {}", to.as_secs()));
    }
    fn set_volume(&self, volume: f32) {
        self.note(format!("volume {volume:.2}"));
    }
    fn set_muted(&self, muted: bool) {
        self.note(format!("muted {muted}"));
    }
    fn set_captions(&self, track: Option<usize>) {
        self.note(format!("captions {track:?}"));
    }
    fn next(&self) {
        self.note("next".into());
    }
}

#[test]
fn casting_hands_every_control_to_the_remote_and_back() {
    let mut s = Screen::new();
    let (p, view) = scripted(&mut s);
    p.seek(Duration::from_secs(1));
    s.wait("the local seek", |_| near(p.position(), 1.0, 0.001));
    let visible = |v: &El, name: &str| named(v, name).and_then(|b| b.info()).is_some_and(|i| !i.hidden);
    // No app listening: no cast button.
    assert!(!visible(&view, "Cast"));
    let menus = Arc::new(Mutex::new(0));
    let m = menus.clone();
    p.on_cast(move |button| {
        assert!(button.info().is_some(), "the button, to anchor a menu to");
        *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    });
    s.pump();
    assert!(visible(&view, "Cast"));
    named(&view, "Cast").unwrap_or_else(|| unreachable!()).focus();
    assert!(press(&s.ui, Key::Enter));
    assert_eq!(*menus.lock().unwrap_or_else(|e| e.into_inner()), 1, "the app opens its menu");

    // The app chose a TV.
    let log = Arc::new(Mutex::new(Vec::new()));
    let art = s.ui.el("label").text("Poster");
    p.set_remote(Some(Remote::new("The Ris", Tv(log.clone())).poster(art.clone())));
    s.pump();
    assert!(p.is_remote() && p.remote_name().as_deref() == Some("The Ris"));
    assert!(visible(&view, "Connecting to The Ris"), "the stage says so");
    assert!(visible(&view, "Casting"), "the button says it is casting");
    let on_stage =
        find(&view, "player_poster").iter().flat_map(El::child_elements).map(|e| e.id()).collect::<Vec<_>>();
    assert_eq!(on_stage, [art.id()], "the poster on the stage");
    let status = RemoteStatus {
        state: PlaybackState::Paused,
        position: Duration::from_secs(30),
        length: Some(Duration::from_secs(100)),
        volume: 0.5,
        muted: false,
    };
    p.set_remote_status(status.clone());
    s.pump();
    let stage = named(&view, "Paused on The Ris").unwrap_or_else(|| unreachable!("the stage"));
    let said = stage.info().and_then(|i| i.semantics).unwrap_or_else(|| unreachable!());
    assert_eq!(said.role(), remedy_ui_a11y::Role::Status);
    assert_eq!(said.live_state(), remedy_ui_a11y::Live::Polite, "announced as it changes");
    assert!(
        texts(&view, "label").iter().any(|t| t == "0:30 / 1:40"),
        "the TV's time: {:?}",
        texts(&view, "label")
    );
    let seek = named(&view, "Seek").unwrap_or_else(|| unreachable!("a seek bar"));
    let value = || {
        named(&view, "Seek")
            .and_then(|b| b.info())
            .and_then(|i| i.semantics)
            .and_then(|s| s.value_text().map(plain))
    };
    assert_eq!(value().as_deref(), Some("0:30 of 1:40"));
    assert!(
        seek.info().is_some_and(|i| !i.states.contains(remedy_ui_style::States::DISABLED)),
        "seekable on the TV"
    );

    // Every control and key goes to the TV.
    p.add_subtitles(SubtitleTrack { label: "English".into(), language: None, cues: Vec::new() });
    p.set_next_up(Some(NextUp::new(MediaSource::url("script:"), "Next")));
    s.pump();
    seek.focus();
    let shift = remedy_ui_input::Modifiers { shift: true, ..Default::default() };
    assert!(press(&s.ui, Key::Character("k".into())));
    assert_eq!(p.state(), PlaybackState::Playing, "shown at once");
    assert!(press(&s.ui, Key::Space));
    assert!(press(&s.ui, Key::Character("l".into())));
    assert!(press(&s.ui, Key::Character("j".into())));
    assert!(press(&s.ui, Key::ArrowRight));
    assert!(press(&s.ui, Key::End));
    assert!(press(&s.ui, Key::Character("m".into())));
    assert!(press(&s.ui, Key::Character("c".into())));
    assert!(s.ui.key(&KeyEvent::press(Key::Character("N".into())).with_modifiers(shift).with_text(None)));
    named(&view, "Unmute").unwrap_or_else(|| unreachable!("the mute button says the TV is muted")).focus();
    assert!(press(&s.ui, Key::Enter));
    p.set_volume(0.8);
    s.pump();
    let pressed = log.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(
        pressed,
        [
            "play",
            "pause",
            "seek 40",
            "seek 30",
            "seek 35",
            "seek 100",
            "muted true",
            "captions Some(0)",
            "next",
            "muted false",
            "volume 0.80"
        ]
    );
    assert_eq!(value().as_deref(), Some("1:40 of 1:40"));
    assert_eq!(p.volume(), 0.8);
    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");

    // Until the TV says it reached the End press's target, the bar stays there.
    p.set_remote_status(RemoteStatus { state: PlaybackState::Buffering, ..status.clone() });
    s.pump();
    assert_eq!(value().as_deref(), Some("1:40 of 1:40"), "a report from before the seek");
    p.set_remote_status(RemoteStatus { position: Duration::from_secs(99), ..status.clone() });
    s.pump();
    assert_eq!(value().as_deref(), Some("1:39 of 1:40"), "landed");
    // Then the TV's word wins.
    p.set_remote_status(RemoteStatus { state: PlaybackState::Buffering, ..status });
    s.pump();
    assert!(visible(&view, "Loading on The Ris"));
    assert_eq!(value().as_deref(), Some("0:30 of 1:40"));

    // Back here: the local media kept its place, and the controls are its own again.
    let at = p.position();
    p.set_remote(None);
    s.pump();
    assert_eq!(at, Duration::from_secs(30), "the app reads where the TV was");
    assert!(
        find(&view, "player_cast").iter().all(|c| c.info().is_some_and(|i| i.hidden)),
        "the stage is gone"
    );
    assert!(visible(&view, "Cast"));
    assert_eq!(p.state(), PlaybackState::Paused);
    assert!(near(p.position(), 1.0, 0.001), "the local engine never moved: {:?}", p.position());
    assert_eq!(p.volume(), 1.0);
    seek.focus();
    assert!(press(&s.ui, Key::Character("k".into())));
    s.wait("local playing", |_| p.state() == PlaybackState::Playing);
    assert_eq!(log.lock().unwrap_or_else(|e| e.into_inner()).len(), 11, "the TV heard nothing more");
}

/// A paused scripted player of `seconds`, with `options`, in a player widget.
fn scripted_with(s: &mut Screen, seconds: u64, options: PlayerOptions) -> (Player, El) {
    let host = Arc::new(Scripted {
        heard: Arc::new(AtomicU64::new(0)),
        buffer: Duration::from_millis(200),
        device: false,
        seconds,
    });
    let p = Player::open(&s.ui, host, MediaSource::url("script:"), options);
    let view = player(&s.ui, &p, "Scripted");
    s.ui.root().child(view.clone());
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    (p, view)
}

/// Types `c` (with Shift held when `shift`).
fn letter(ui: &Ui, c: &str, shift: bool) -> bool {
    let shift = remedy_ui_input::Modifiers { shift, ..Default::default() };
    ui.key(&KeyEvent::press(Key::Character(c.into())).with_modifiers(shift).with_text(Some(c.into())))
}

fn el(view: &El, name: &str) -> El {
    named(view, name).unwrap_or_else(|| unreachable!("{name}"))
}

/// Double-clicks the middle of `target` at `at` ms (two presses 100 ms apart).
fn double_click(s: &mut Screen, target: &El, at: u64) {
    presses(s, target, &[at, at + 100], remedy_ui_input::PointerKind::Mouse);
}

/// Presses and lets go of the middle of `target` at each of `times` (ms), with `kind`.
fn presses(s: &mut Screen, target: &El, times: &[u64], kind: remedy_ui_input::PointerKind) {
    use remedy_ui_input::{Buttons, PointerEvent, PointerPhase};
    s.ui.set_double_click_time(Duration::from_millis(400));
    s.pump();
    let b = target.bounds().unwrap_or_default();
    let c = remedy_ui_core::Point::new((b.min.x + b.max.x) / 2.0, (b.min.y + b.max.y) / 2.0);
    for &t in times {
        let ev = |phase, buttons| {
            PointerEvent::new(phase, c)
                .with_kind(kind)
                .with_buttons(buttons)
                .with_time(Duration::from_millis(t))
        };
        s.ui.pointer(&ev(PointerPhase::Down, Buttons::PRIMARY));
        s.ui.pointer(&ev(PointerPhase::Up, Buttons::default()));
    }
    s.pump();
}

#[test]
fn the_arrows_scheme_answers_anywhere_in_the_player() {
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 60, PlayerOptions::default().keys(KeyScheme::Arrows));
    p.add_subtitles(SubtitleTrack { label: "English".into(), language: None, cues: Vec::new() });
    p.on_theater(|_| {});
    p.seek(Duration::from_secs(30));
    s.wait("the seek", |_| near(p.position(), 30.0, 0.001));
    p.set_volume(0.5);
    s.pump();
    // Focus on a button, not a slider: the arrows still act.
    el(&view, "Mute").focus();
    let at = |s: &mut Screen, secs: f64| s.wait(&format!("at {secs}"), |_| near(p.position(), secs, 0.001));
    assert!(press(&s.ui, Key::ArrowRight));
    at(&mut s, 40.0);
    assert!(press(&s.ui, Key::ArrowLeft));
    at(&mut s, 30.0);
    assert!(letter(&s.ui, "l", false));
    at(&mut s, 40.0);
    assert!(letter(&s.ui, "J", true), "letters work with Shift");
    at(&mut s, 30.0);
    assert!(press(&s.ui, Key::ArrowUp));
    assert_eq!(p.volume(), 0.55);
    assert!(press(&s.ui, Key::ArrowDown));
    assert!(press(&s.ui, Key::ArrowDown));
    assert_eq!(p.volume(), 0.45);
    // A step unmutes.
    assert!(letter(&s.ui, "m", false));
    assert!(p.is_muted());
    assert!(press(&s.ui, Key::ArrowUp));
    assert!(!p.is_muted() && p.volume() == 0.5);
    assert!(letter(&s.ui, "K", true));
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    assert!(letter(&s.ui, "k", false));
    s.wait("paused", |_| p.state() == PlaybackState::Paused);
    assert!(letter(&s.ui, "c", false));
    assert_eq!(p.caption_track(), Some(0));
    assert!(letter(&s.ui, "f", false) && p.is_fullscreen());
    assert!(press(&s.ui, Key::Escape) && !p.is_fullscreen());
    assert!(letter(&s.ui, "t", false) && p.is_theater());
    assert!(press(&s.ui, Key::Escape) && !p.is_theater());
    assert!(letter(&s.ui, ">", true) && p.speed() > 1.0);
    assert!(letter(&s.ui, "<", true) && p.speed() == 1.0);
    // Space presses the focused button, not play.
    assert!(press(&s.ui, Key::Space));
    assert!(p.is_muted() && p.state() == PlaybackState::Paused);
    p.set_muted(false);
    // A focused slider keeps its keys: the seek bar moves its own 5 seconds.
    let pos = p.position().as_secs_f64();
    el(&view, "Seek").focus();
    assert!(press(&s.ui, Key::ArrowRight));
    at(&mut s, pos + 5.0);
    // The volume slider moves itself, a step as the arrows do; Space plays there.
    el(&view, "Volume").focus();
    assert!(press(&s.ui, Key::ArrowUp));
    s.pump();
    assert_eq!(p.volume(), 0.55);
    assert!(press(&s.ui, Key::Space));
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    p.pause();
    // N plays next, plain or with Shift.
    p.set_queue(vec![
        NextUp::new(MediaSource::url("script:"), "One"),
        NextUp::new(MediaSource::url("script:"), "Two"),
    ]);
    assert!(letter(&s.ui, "n", false));
    assert_eq!(p.queue().len(), 1);
    assert!(letter(&s.ui, "N", true));
    assert!(p.queue().is_empty());
    assert!(!letter(&s.ui, "n", false), "nothing next");
    // Ctrl, Alt or the system key: never the player's.
    let ctrl = remedy_ui_input::Modifiers { ctrl: true, ..Default::default() };
    el(&view, "Mute").focus();
    assert!(!s.ui.key(&KeyEvent::press(Key::ArrowRight).with_modifiers(ctrl).with_text(None)));
    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn the_classic_scheme_keeps_the_arrows_on_the_sliders_and_next_on_shift_n() {
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 60, PlayerOptions::default());
    p.seek(Duration::from_secs(30));
    s.wait("the seek", |_| near(p.position(), 30.0, 0.001));
    p.set_next_up(Some(NextUp::new(MediaSource::url("script:"), "Next")));
    el(&view, "Mute").focus();
    for k in [Key::ArrowRight, Key::ArrowLeft, Key::ArrowUp, Key::ArrowDown] {
        assert!(!press(&s.ui, k), "the arrows are the sliders' alone");
    }
    assert!(!letter(&s.ui, "n", false), "plain N is not next");
    assert!(!letter(&s.ui, "K", true), "Shift+K is not K");
    s.pump();
    assert!(near(p.position(), 30.0, 0.001) && p.volume() == 1.0);
    el(&view, "Seek").focus();
    assert!(press(&s.ui, Key::ArrowUp), "the seek bar seeks on Up");
    s.wait("5 s on", |_| near(p.position(), 35.0, 0.001));
    for k in ["k", "k", "l", "j", "m", "m", "f", "f"] {
        assert!(letter(&s.ui, k, false), "{k}");
    }
    assert!(letter(&s.ui, "N", true));
    assert!(p.next_up().is_none(), "Shift+N played it");
}

#[test]
fn text_fields_and_open_menus_keep_their_keys_in_both_schemes() {
    for keys in [KeyScheme::Classic, KeyScheme::Arrows] {
        let mut s = Screen::new();
        let (p, view) = scripted_with(&mut s, 60, PlayerOptions::default().keys(keys));
        p.seek(Duration::from_secs(30));
        s.wait("the seek", |_| near(p.position(), 30.0, 0.001));
        p.set_volume(0.5);
        p.set_next_up(Some(NextUp::new(MediaSource::url("script:"), "Next")));
        let unmoved = |p: &Player| {
            p.state() == PlaybackState::Paused
                && near(p.position(), 30.0, 0.001)
                && p.volume() == 0.5
                && !p.is_muted()
                && !p.is_fullscreen()
                && p.next_up().is_some()
        };
        // A field an app put in the player.
        let field = remedy_ui_widgets_core::text_input(&s.ui, "Note");
        let _ = view.clone().child(field.el());
        s.pump();
        field.el().focus();
        for c in ["k", "j", "l", "m", "c", "f", "t", "n", " "] {
            letter(&s.ui, c, false);
        }
        letter(&s.ui, "N", true);
        for k in [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End] {
            press(&s.ui, k);
        }
        s.idle(30);
        assert_eq!(field.value(), "kjlmcftn N", "{keys:?}: the field took the letters");
        assert!(unmoved(&p), "{keys:?}: {:?}", p.position());
        // An open menu takes the keys.
        let anchor = el(&view, "Mute");
        anchor.focus();
        let item = remedy_ui_widgets_core::MenuItem::new("Something", || {});
        let _menu = remedy_ui_widgets_core::menu(&s.ui, &anchor, vec![item]);
        s.pump();
        for c in ["k", "l", "m", "f", "n"] {
            letter(&s.ui, c, false);
        }
        for k in [Key::ArrowRight, Key::ArrowUp, Key::ArrowDown] {
            press(&s.ui, k);
        }
        s.idle(30);
        assert!(unmoved(&p), "{keys:?}: {:?}", p.position());
    }
}

#[test]
fn double_clicking_the_picture_asks_for_full_screen_in_both_schemes() {
    for keys in [KeyScheme::Classic, KeyScheme::Arrows] {
        let mut s = Screen::new();
        let (p, view) = scripted_with(&mut s, 3, PlayerOptions::default().keys(keys));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let a = asked.clone();
        p.on_fullscreen(move |on| a.lock().unwrap_or_else(|e| e.into_inner()).push(on));
        let stage = find(&view, "player_stage").remove(0);
        double_click(&mut s, &stage, 1000);
        assert!(p.is_fullscreen(), "{keys:?}");
        double_click(&mut s, &stage, 3000);
        assert!(!p.is_fullscreen(), "{keys:?}");
        assert_eq!(*asked.lock().unwrap_or_else(|e| e.into_inner()), [true, false]);
        assert_eq!(p.state(), PlaybackState::Paused, "a double click does not play");
    }
}

#[test]
fn casting_disables_full_screen_and_theater_until_it_ends() {
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 3, PlayerOptions::default().keys(KeyScheme::Arrows));
    p.on_theater(|_| {});
    s.pump();
    let disabled = |v: &El, name: &str| {
        named(v, name)
            .and_then(|b| b.info())
            .is_some_and(|i| i.states.contains(remedy_ui_style::States::DISABLED))
    };
    let stage = find(&view, "player_stage").remove(0);
    let log = Arc::new(Mutex::new(Vec::new()));
    p.set_remote(Some(Remote::new("The Ris", Tv(log.clone()))));
    s.pump();
    assert!(disabled(&view, "Full screen") && disabled(&view, "Theater mode"));
    el(&view, "Mute").focus();
    assert!(!letter(&s.ui, "f", false));
    assert!(!letter(&s.ui, "t", false));
    double_click(&mut s, &stage, 1000);
    el(&view, "Full screen").focus();
    let _ = press(&s.ui, Key::Enter);
    assert!(!p.is_fullscreen() && !p.is_theater(), "nothing to enlarge while casting");
    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");

    // Back here: offered again.
    p.set_remote(None);
    s.pump();
    assert!(!disabled(&view, "Full screen") && !disabled(&view, "Theater mode"));
    el(&view, "Mute").focus();
    assert!(letter(&s.ui, "t", false) && p.is_theater());
    assert!(letter(&s.ui, "f", false) && p.is_fullscreen());

    // Casting from full screen and theater: they stay, and can only be left.
    p.set_remote(Some(Remote::new("The Ris", Tv(log.clone()))));
    s.pump();
    assert!(p.is_fullscreen() && p.is_theater(), "casting leaves the layout as it was");
    assert!(!disabled(&view, "Exit full screen") && !disabled(&view, "Theater mode"));
    double_click(&mut s, &stage, 5000);
    assert!(!p.is_fullscreen(), "a double click leaves full screen");
    assert!(disabled(&view, "Full screen"));
    el(&view, "Mute").focus();
    assert!(!letter(&s.ui, "f", false) && !p.is_fullscreen(), "and cannot come back");
    assert!(letter(&s.ui, "t", false) && !p.is_theater(), "T leaves theater mode");
    s.pump();
    assert!(disabled(&view, "Theater mode"));
    assert!(!letter(&s.ui, "t", false) && !p.is_theater());
    assert!(log.lock().unwrap_or_else(|e| e.into_inner()).is_empty(), "the TV heard none of it");
}

#[test]
fn a_click_on_the_picture_plays_and_pauses_once_no_second_click_comes() {
    use remedy_ui_input::PointerKind::{Mouse, Touch};
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 60, PlayerOptions::default().keys(KeyScheme::Arrows));
    let stage = find(&view, "player_stage").remove(0);
    // One click: nothing yet, then play once the double-click hold is out.
    presses(&mut s, &stage, &[1000], Mouse);
    assert_eq!(p.state(), PlaybackState::Paused, "waits for a second click");
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    presses(&mut s, &stage, &[2000], Mouse);
    s.wait("paused", |_| p.state() == PlaybackState::Paused);
    // A double click: full screen, and the first click's play is taken back.
    double_click(&mut s, &stage, 3000);
    s.idle(300);
    assert!(p.is_fullscreen() && p.state() == PlaybackState::Paused);
    // A third click in a row is a click again.
    presses(&mut s, &stage, &[4000, 4100, 4200], Mouse);
    assert!(!p.is_fullscreen(), "the double click in it leaves full screen");
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    p.pause();
    s.wait("paused", |_| p.state() == PlaybackState::Paused);
    // Fingers: a tap plays or pauses, a double tap toggles full screen alone.
    presses(&mut s, &stage, &[6000], Touch);
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    presses(&mut s, &stage, &[7000], Touch);
    s.wait("paused", |_| p.state() == PlaybackState::Paused);
    presses(&mut s, &stage, &[8000, 8150], Touch);
    s.idle(300);
    assert!(p.is_fullscreen() && p.state() == PlaybackState::Paused);
    presses(&mut s, &stage, &[9000, 9150], Touch);
    s.idle(300);
    assert!(!p.is_fullscreen() && p.state() == PlaybackState::Paused);

    // While casting a click plays and pauses the TV; a double click does nothing.
    let log = Arc::new(Mutex::new(Vec::new()));
    p.set_remote(Some(Remote::new("The Ris", Tv(log.clone()))));
    s.pump();
    presses(&mut s, &stage, &[10_000], Mouse);
    s.idle(300);
    double_click(&mut s, &stage, 11_000);
    s.idle(300);
    assert!(!p.is_fullscreen());
    assert_eq!(*log.lock().unwrap_or_else(|e| e.into_inner()), ["play"]);
    p.set_remote(None);
    s.pump();

    // A press outside an open menu only closes it, as everywhere in RemedyUI...
    let anchor = el(&view, "Mute");
    let items = || vec![remedy_ui_widgets_core::MenuItem::new("Something", || {})];
    let _menu = remedy_ui_widgets_core::menu(&s.ui, &anchor, items());
    s.pump();
    presses(&mut s, &stage, &[13_000], Mouse);
    s.idle(300);
    assert_eq!(p.state(), PlaybackState::Paused);
    // ...but a menu opened to let it through (an app's cast menu) closes and plays, as in a
    // web page's player.
    let menu = remedy_ui_widgets_core::menu_press_through(&s.ui, &anchor, items());
    s.pump();
    assert!(menu.info().is_some_and(|i| !i.hidden));
    presses(&mut s, &stage, &[14_000], Mouse);
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    assert!(menu.info().is_none(), "the menu closed");
}

#[test]
fn in_the_classic_scheme_a_click_on_the_picture_does_nothing() {
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 60, PlayerOptions::default());
    let stage = find(&view, "player_stage").remove(0);
    presses(&mut s, &stage, &[1000], remedy_ui_input::PointerKind::Mouse);
    presses(&mut s, &stage, &[2000], remedy_ui_input::PointerKind::Touch);
    s.idle(300);
    assert_eq!(p.state(), PlaybackState::Paused);
    presses(&mut s, &stage, &[3000, 3150], remedy_ui_input::PointerKind::Touch);
    assert!(p.is_fullscreen(), "a double tap is a double click");
}

#[test]
fn a_click_on_a_control_hands_the_keys_back_to_the_player_in_the_arrows_scheme() {
    use remedy_ui_input::PointerKind::{Mouse, Touch};
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 60, PlayerOptions::default().keys(KeyScheme::Arrows));
    p.seek(Duration::from_secs(30));
    s.wait("the seek", |_| near(p.position(), 30.0, 0.001));
    let mute = el(&view, "Mute");
    // A click mutes and leaves focus with the player, not the button.
    presses(&mut s, &mute, &[1000], Mouse);
    assert!(p.is_muted());
    assert!(!el(&view, "Unmute").is_focused(), "focus left the button");
    assert!(view.is_focused(), "the player has the keys");
    // Space plays and pauses instead of pressing the button again; the arrows skip.
    assert!(press(&s.ui, Key::Space));
    s.wait("playing", |_| p.state() == PlaybackState::Playing);
    assert!(p.is_muted());
    p.pause();
    s.wait("paused", |_| p.state() == PlaybackState::Paused);
    assert!(press(&s.ui, Key::ArrowRight));
    s.wait("10 s on", |_| near(p.position(), 40.0, 0.001));
    // A tap on the volume slider: the arrows then skip, not nudge the slider.
    let volume = el(&view, "Volume");
    presses(&mut s, &volume, &[2000], Touch);
    assert!(!volume.is_focused() && view.is_focused());
    assert!(press(&s.ui, Key::ArrowLeft));
    s.wait("10 s back", |_| near(p.position(), 30.0, 0.001));
    // Tab leaves the player as usual, and it is a plain group again.
    assert!(press(&s.ui, Key::Tab));
    assert!(!view.is_focused());
    s.idle(30);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
    // Keyboard activation keeps focus where the person put it.
    let unmute = el(&view, "Unmute");
    unmute.focus();
    assert!(press(&s.ui, Key::Enter));
    s.pump();
    assert!(!p.is_muted());
    assert!(el(&view, "Mute").is_focused(), "Enter kept focus on the button");
}

#[test]
fn in_the_classic_scheme_a_click_leaves_focus_on_the_control() {
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 60, PlayerOptions::default());
    presses(&mut s, &el(&view, "Mute"), &[1000], remedy_ui_input::PointerKind::Mouse);
    assert!(p.is_muted());
    assert!(el(&view, "Unmute").is_focused());
    assert!(press(&s.ui, Key::Space), "Space presses it again");
    s.pump();
    assert!(!p.is_muted());
}

/// The casting stage as Portal draws it: the poster behind a dark scrim, "Playing on" small
/// and the device's name large, centered. Writes `cast_stage.png` to the test's temp folder.
#[test]
fn the_casting_stage_centers_the_device_over_the_darkened_poster() {
    let preview = remedy_ui_devtools::Preview::new("cast stage", |ui| {
        let host = Arc::new(Scripted {
            heard: Arc::new(AtomicU64::new(0)),
            buffer: Duration::from_millis(200),
            device: false,
            seconds: 3,
        });
        let p = Player::open(ui, host, MediaSource::url("script:"), PlayerOptions::default());
        let art = ui.el("art").grow(1.0).fill_width().style(
            remedy_ui_style::StylePatch::default().background(remedy_ui_core::Color::rgb8(200, 60, 40)),
        );
        p.set_remote(Some(Remote::new("The Ris", Tv(Arc::default())).poster(art)));
        p.set_remote_status(RemoteStatus::new(PlaybackState::Playing, Duration::from_secs(30)));
        // Kept alive with the view.
        let view = player(ui, &p, "Scripted");
        let keep = p.clone();
        view.on_removed(move || drop(keep))
    })
    .sizes(&[(640.0, 400.0)]);
    let first = preview.variations().into_iter().next().unwrap_or_else(|| unreachable!("a size"));
    let shot = preview.render(&first);
    let png = shot.to_png().unwrap_or_else(|e| unreachable!("{e:?}"));
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cast_stage.png");
    std::fs::write(&out, png).unwrap_or_else(|e| unreachable!("{e}"));
    println!("{}", out.display());
    let px = |x: u32, y: u32| {
        let i = ((y * shot.width + x) * 4) as usize;
        match shot.rgba.get(i..i + 3) {
            Some(&[r, g, b]) => (r, g, b),
            _ => unreachable!("({x}, {y}) is in the picture"),
        }
    };
    // A corner of the stage: the poster's red, darkened, not black.
    let (r, g, _) = px(10, 10);
    assert!(r > 40 && r < 120 && g < 40, "darkened red: {:?}", px(10, 10));
}

#[test]
fn speed_is_the_devices_own_while_casting() {
    let mut s = Screen::new();
    let (p, view) = scripted_with(&mut s, 3, PlayerOptions::default());
    let speed_shown = |v: &El| named(v, "Speed 1×").and_then(|b| b.info()).is_some_and(|i| !i.hidden);
    assert!(speed_shown(&view), "{:?}", texts(&view, "label"));
    p.set_remote(Some(Remote::new("The Ris", Tv(Arc::default()))));
    s.pump();
    assert!(!speed_shown(&view), "no speed while casting");
    el(&view, "Mute").focus();
    assert!(!letter(&s.ui, ">", true) && !letter(&s.ui, "<", true));
    p.step_speed(true);
    assert_eq!(p.speed(), 1.0, "the local media keeps its speed");
    p.set_remote(None);
    s.pump();
    assert!(speed_shown(&view), "back with the local media");
    assert!(letter(&s.ui, ">", true) && p.speed() > 1.0);
}

#[test]
fn status_news_comes_from_any_thread_and_a_stale_poll_does_not_undo_a_seek() {
    let mut s = Screen::new();
    let (p, _view) = scripted_with(&mut s, 3, PlayerOptions::default());
    let log = Arc::new(Mutex::new(Vec::new()));
    p.set_remote(Some(Remote::new("The Ris", Tv(log.clone()))));
    let at = |secs| Duration::from_secs(secs);
    let polled = move |secs| RemoteStatus::new(PlaybackState::Playing, at(secs)).length(at(100)).volume(0.5);
    // A poll on its own thread.
    let sender = p.remote_status_sender();
    let sent = std::thread::spawn(move || sender.send(polled(30))).join().unwrap_or(false);
    assert!(sent);
    s.wait("the news", |_| p.position() == at(30));
    assert_eq!((p.duration(), p.volume(), p.state()), (Some(at(100)), 0.5, PlaybackState::Playing));
    // A seek: the bar shows its target at once, and a poll from before the TV moved does not
    // pull it back.
    p.seek(at(60));
    assert_eq!(p.position(), at(60));
    p.set_remote_status(polled(31));
    assert_eq!(p.position(), at(60), "a stale poll is held off");
    assert_eq!(p.volume(), 0.5, "the rest of the news still counts");
    // The TV has landed (within 4 s of the target): its word wins again.
    p.set_remote_status(polled(62));
    assert_eq!(p.position(), at(62));
    p.set_remote_status(polled(10));
    assert_eq!(p.position(), at(10), "no seek in flight: believed");
    assert_eq!(*log.lock().unwrap_or_else(|e| e.into_inner()), ["seek 60"]);
    // Gone UI thread or not, a sender outliving the player drops the news quietly.
    let sender = p.remote_status_sender();
    drop(p);
    assert!(sender.send(polled(5)));
    s.pump();
}

/// Endgame resumed at 396.7 s: the stream's time 0 is the film's 396.7 s, and the film lasts
/// 10892 s.
const ORIGIN: Duration = Duration::from_millis(396_700);
const FILM: Duration = Duration::from_secs(10_892);

/// A 20-second stream of the film from `ORIGIN`, in a player widget; and the seeks handed to
/// the app.
fn film(s: &mut Screen) -> (Player, El, Arc<Mutex<Vec<u64>>>) {
    let options = PlayerOptions::default().timeline(Timeline::new(ORIGIN).length(FILM));
    let p = Player::open(&s.ui, Arc::new(Clips), MediaSource::url("script:20"), options);
    let view = player(&s.ui, &p, "Endgame");
    s.ui.root().child(view.clone());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let a = asked.clone();
    p.on_seek_outside(move |to| a.lock().unwrap_or_else(|e| e.into_inner()).push(to.as_secs()));
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    (p, view, asked)
}

fn seek_value(view: &El) -> Option<String> {
    named(view, "Seek")
        .and_then(|b| b.info())
        .and_then(|i| i.semantics)
        .and_then(|s| s.value_text().map(plain))
}

fn seek_disabled(view: &El) -> bool {
    named(view, "Seek")
        .and_then(|b| b.info())
        .is_some_and(|i| i.states.contains(remedy_ui_style::States::DISABLED))
}

#[test]
fn on_a_timeline_everything_is_in_film_time_and_seeks_inside_the_stream_stay_in_place() {
    let mut s = Screen::new();
    let (p, view, asked) = film(&mut s);
    assert_eq!(p.position(), ORIGIN);
    assert_eq!(p.duration(), Some(FILM), "the film's length, not the stream's");
    s.idle(30);
    assert!(texts(&view, "label").iter().any(|t| t == "6:36 / 3:01:32"), "{:?}", texts(&view, "label"));
    assert_eq!(seek_value(&view).as_deref(), Some("6:36 of 3:01:32"));
    assert!(!seek_disabled(&view));
    assert!(p.buffered().iter().all(|r| r.start >= ORIGIN), "{:?}", p.buffered());
    // Inside what the stream holds: in place, in whole seconds.
    p.seek(Duration::from_millis(410_600));
    s.wait("the seek in place", |_| p.position() == Duration::from_secs(410));
    p.seek_by(-5.0);
    s.wait("back 5 s", |_| p.position() == Duration::from_secs(405));
    assert!(asked.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    assert_eq!(p.skipping(), None);
    // A caption track the app adds is read in film time.
    let cue = Cue {
        start: Duration::from_secs(404),
        end: Duration::from_secs(407),
        text: "Whatever it takes".into(),
    };
    let i = p.add_subtitles(SubtitleTrack { label: "English".into(), language: None, cues: vec![cue] });
    p.set_caption_track(Some(i));
    assert_eq!(p.caption_text().as_deref(), Some("Whatever it takes"));
}

#[test]
fn a_seek_outside_the_stream_goes_to_the_app_which_hands_over_a_new_one() {
    let mut s = Screen::new();
    let (p, view, asked) = film(&mut s);
    let asked_for = || asked.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // Before the origin.
    p.seek(Duration::from_secs(100));
    assert_eq!(asked_for(), [100]);
    assert_eq!(p.skipping(), Some(Duration::from_secs(100)));
    s.idle(30);
    assert_eq!(texts(&view, "player_caption"), ["Skipping to 1:40"]);
    assert!(seek_disabled(&view), "the bar waits for the new stream");
    // Presses of ±10 meanwhile add up; seeks wait.
    p.seek_by(10.0);
    p.seek_by(10.0);
    p.seek(Duration::from_secs(5000));
    assert_eq!(asked_for(), [100]);
    // The app's new stream starts at 100 s; the presses then run from there: 120 s, which the
    // new stream does not hold yet (it is still opening), so the app is asked again, as
    // Portal's page does.
    p.set_volume(0.4);
    p.replace_source(MediaSource::url("script:20"), Timeline::new(Duration::from_secs(100)).length(FILM));
    assert_eq!(asked_for(), [100, 120]);
    p.replace_source(MediaSource::url("script:20"), Timeline::new(Duration::from_secs(120)).length(FILM));
    assert_eq!(p.skipping(), None);
    assert_eq!(p.position(), Duration::from_secs(120));
    s.wait("plays the new stream", |_| p.state() == PlaybackState::Playing);
    assert_eq!(p.volume(), 0.4);
    assert!(!seek_disabled(&view));
    // Past what the stream holds (within 2 s of its end): to the app.
    p.pause();
    p.seek(Duration::from_secs(139));
    assert_eq!(asked_for(), [100, 120, 139]);
    p.cancel_skip();
    assert_eq!(p.skipping(), None);
    p.seek(Duration::from_secs(137));
    s.wait("in place, 17 s into the stream", |_| p.position() == Duration::from_secs(137));
    assert_eq!(asked_for(), [100, 120, 139]);
    let problems = remedy_ui_devtools::audit(&s.ui);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_new_stream_keeps_the_captions_and_remote_mode() {
    let mut s = Screen::new();
    let (p, _view, _asked) = film(&mut s);
    let cue =
        Cue { start: Duration::from_secs(600), end: Duration::from_secs(604), text: "On your left".into() };
    let i = p.add_subtitles(SubtitleTrack { label: "English".into(), language: None, cues: vec![cue] });
    p.set_caption_track(Some(i));
    p.set_caption_delay(250);
    p.set_remote(Some(Remote::new("The Ris", Tv(Arc::new(Mutex::new(Vec::new()))))));
    p.replace_source(MediaSource::url("script:20"), Timeline::new(Duration::from_secs(599)).length(FILM));
    assert_eq!((p.subtitle_tracks().len(), p.caption_track(), p.caption_delay()), (1, Some(0), 250));
    assert!(p.is_remote());
    p.set_remote(None);
    s.wait("opened", |_| p.state() == PlaybackState::Paused);
    s.idle(50);
    assert_eq!(p.state(), PlaybackState::Paused, "opened while remote: it does not play here");
    p.seek(Duration::from_secs(601));
    s.wait("601 s", |_| p.position() == Duration::from_secs(601));
    assert_eq!(p.caption_text().as_deref(), Some("On your left"));
}

#[test]
fn a_hand_off_to_a_tv_and_back_lands_at_the_same_film_time() {
    let mut s = Screen::new();
    let (p, _view, asked) = film(&mut s);
    p.seek(Duration::from_secs(405));
    s.wait("405 s", |_| p.position() == Duration::from_secs(405));
    // To the TV: the app casts at the film time the player shows.
    let log = Arc::new(Mutex::new(Vec::new()));
    let here = p.position();
    p.set_remote(Some(Remote::new("The Ris", Tv(log.clone()))));
    p.set_remote_status(RemoteStatus::new(PlaybackState::Paused, here).length(FILM));
    assert_eq!(p.position(), Duration::from_secs(405));
    assert_eq!(p.duration(), Some(FILM));
    // The TV takes film time.
    p.seek(Duration::from_millis(2_000_400));
    assert_eq!(*log.lock().unwrap_or_else(|e| e.into_inner()), ["seek 2000"]);
    assert!(asked.lock().unwrap_or_else(|e| e.into_inner()).is_empty(), "the TV seeks itself");
    p.set_remote_status(RemoteStatus::new(PlaybackState::Paused, Duration::from_secs(2000)).length(FILM));
    // Back here: the same film time, which this stream does not hold, goes to the app.
    let there = p.position();
    p.set_remote(None);
    p.seek(there);
    assert_eq!(*asked.lock().unwrap_or_else(|e| e.into_inner()), [2000]);
    p.replace_source(MediaSource::url("script:20"), Timeline::new(there).length(FILM));
    assert_eq!(p.position(), Duration::from_secs(2000));
}

#[test]
fn a_playing_tv_moves_on_between_reports_and_a_paused_one_does_not() {
    let mut s = Screen::new();
    let (p, _view) = scripted_with(&mut s, 3, PlayerOptions::default());
    p.set_remote(Some(Remote::new("The Ris", Tv(Arc::new(Mutex::new(Vec::new()))))));
    let at = Duration::from_secs;
    p.set_remote_status(RemoteStatus::new(PlaybackState::Playing, at(30)).length(at(100)));
    assert_eq!(p.position(), at(30), "the report as it is, at first");
    s.idle(700);
    let moved = p.position().as_secs_f64();
    assert!((30.4..31.5).contains(&moved), "moved on by the time since: {moved}");
    p.pause();
    let paused = p.position();
    s.idle(400);
    assert_eq!(p.position(), paused, "paused: held");
    // Never past the end.
    p.set_remote_status(
        RemoteStatus::new(PlaybackState::Playing, Duration::from_millis(99_800)).length(at(100)),
    );
    s.idle(700);
    assert_eq!(p.position(), at(100));
    // A seek in flight shows its target, not a moving guess.
    p.set_remote_status(RemoteStatus::new(PlaybackState::Playing, at(10)).length(at(100)));
    p.seek(at(50));
    s.idle(400);
    assert_eq!(p.position(), at(50));
}

#[test]
fn without_a_timeline_seeks_are_the_streams_own_and_not_whole_seconds() {
    let mut s = Screen::new();
    let (p, _view) = scripted(&mut s);
    assert_eq!(p.timeline(), None);
    p.seek(Duration::from_millis(1500));
    s.wait("1.5 s", |_| p.position() == Duration::from_millis(1500));
    assert_eq!(p.duration(), Some(Duration::from_secs(3)));
    assert!(p.buffered().first().is_some_and(|r| r.start < Duration::from_millis(1500)));
}
