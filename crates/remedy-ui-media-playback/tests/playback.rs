//! Playback through the platform's decoders (Media Foundation on Windows, GStreamer on
//! Linux) of a small generated clip (tests/fixtures), and the player's clock, controls and
//! widget on a real view. The engine's sync is also checked against a scripted host whose
//! sound clock the test moves by hand.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use remedy_ui_compositor::Layer;
use remedy_ui_core::Size;
use remedy_ui_input::{Key, KeyEvent, Modifiers};
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
    match platform_host() {
        Ok(h) => Some(h),
        Err(e) if cfg!(windows) => unreachable!("Media Foundation must be available on Windows: {e}"),
        Err(e) => {
            eprintln!("no platform media host here, skipping: {e}");
            None
        }
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
    assert!(info.duration.is_some_and(|d| near(d, 3.0, 0.1)), "{info:?}");
    assert!(info.seekable);
    let v = info.video.clone().unwrap_or_else(|| unreachable!("a picture: {info:?}"));
    // The coded picture is 112 rows; the display aperture is what shows.
    assert_eq!((v.width, v.height), (176, 100));
    assert!(v.frame_rate.is_some_and(|r| (r - 25.0).abs() < 0.01), "{v:?}");
    assert_eq!(info.audio, Some(AudioFormat { sample_rate: 48_000, channels: 2 }));

    let (mut frames, mut sound, mut last_video) = (Vec::new(), 0u64, None);
    while let Some(s) = dec.read().unwrap_or_else(|e| unreachable!("read: {e}")) {
        match s {
            Sample::Video { at, picture, .. } => {
                assert!(last_video.is_none_or(|l| at > l), "pictures in time order");
                last_video = Some(at);
                assert_eq!((picture.width, picture.height, picture.format), (176, 100, PixelFormat::Nv12));
                frames.push((at, picture_hue(&picture)));
            }
            Sample::Audio { samples, .. } => sound += samples.len() as u64 / 2,
            Sample::Text { .. } => {}
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
        Screen { ui: ui(), frame: None }
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
        }
        let _ = self.ui.take_requests();
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
    ui.key(&KeyEvent {
        key,
        physical: String::new(),
        modifiers: Modifiers::NONE,
        pressed: true,
        repeat: false,
        composing: false,
        text: None,
    })
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

fn named(el: &El, name: &str) -> Option<El> {
    let mut stack = vec![el.clone()];
    while let Some(e) = stack.pop() {
        if e.info().and_then(|i| i.semantics).is_some_and(|s| s.name() == Some(name)) {
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
    let p = Player::open(
        &s.ui,
        host,
        MediaSource::file(clip()),
        PlayerOptions { volume: 0.5, ..Default::default() },
    );
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
    assert_eq!(sem.value_text(), Some("0:00 of 0:03"));

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
    assert!(p.muted() && named(&view, "Unmute").is_some());
    assert!(press(&s.ui, Key::Character("c".into())));
    s.pump();
    assert_eq!(p.caption_track(), Some(0));
    assert!(named(&view, "Captions: fr").is_some());
    assert!(press(&s.ui, Key::Character("f".into())));
    s.pump();
    assert!(p.fullscreen() && named(&view, "Exit full screen").is_some());
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
        AudioFormat { sample_rate: RATE, channels: 1 }
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
            info: MediaInfo {
                duration: Some(Duration::from_secs(self.seconds)),
                seekable: true,
                video: Some(VideoInfo { width: 8, height: 8, frame_rate: Some(FPS as f64) }),
                audio: Some(AudioFormat { sample_rate: RATE, channels: 1 }),
                text_tracks: Vec::new(),
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
    s.wait("two and a half seconds", |s| luma(&s.frame) == 70);
    assert!(near(p.position(), 2.5, 0.001), "{:?}", p.position());
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
        let p = Player::open(
            &s.ui,
            host,
            MediaSource::url("script:"),
            PlayerOptions { autoplay: true, ..Default::default() },
        );
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
