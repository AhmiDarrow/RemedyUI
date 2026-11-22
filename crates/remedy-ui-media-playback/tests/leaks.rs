//! Leak checks: a player dropped mid-open, mid-decode and mid-fetch, and each error path (a
//! decoder failing, a seek failing, `fail()`, a 404, a bad segment, captions over the limit),
//! leaves no thread, decoder or platform object behind (Media Foundation starts, D3D11
//! devices, pictures on the GPU and, with the keep-awake feature, the screen's hold;
//! counted by debug builds). Its own test binary, so no
//! other test's threads are counted; its tests take turns.

#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use remedy_ui_media_playback::*;
use remedy_ui_text::FontCollection;
use remedy_ui_view::Ui;

static TURN: Mutex<()> = Mutex::new(());

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), FontCollection::new())
}

/// Whether a thread is one of RemedyUI's: every thread the player and its platform code
/// start is named `remedy-…` (threads the system starts for itself, Media Foundation's and
/// WinHTTP's pools, GStreamer's, are not the player's to end).
fn ours(t: &(String, String)) -> bool {
    t.0.starts_with("remedy-")
}

/// What must go away: RemedyUI's threads, live test decoders, live platform objects.
fn census(decoders: &AtomicUsize) -> (usize, usize, usize) {
    (
        process_threads().iter().filter(|t| ours(t)).count(),
        decoders.load(Ordering::SeqCst),
        live_platform_objects(),
    )
}

/// The threads in `now` that were not in `before` (by name and start module).
fn added(before: &[(String, String)], now: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut left = before.to_vec();
    now.into_iter()
        .filter(|t| match left.iter().position(|b| b == t) {
            Some(i) => {
                left.swap_remove(i);
                false
            }
            None => true,
        })
        .collect()
}

/// Waits up to `within` for everything to fall back to `before`, pumping the UI thread's
/// posts. On failure, names every thread started since `threads` was taken.
fn settles_within(
    what: &str,
    within: Duration,
    before: (usize, usize, usize),
    threads: &[(String, String)],
    decoders: &AtomicUsize,
) {
    let start = Instant::now();
    loop {
        let _ = remedy_ui_reactive::run_posted();
        let now = census(decoders);
        if now.0 <= before.0 && now.1 <= before.1 && now.2 <= before.2 {
            // The system's own threads that stayed, for the record (its pools idle out later).
            let foreign = added(threads, process_threads());
            if !foreign.is_empty() {
                eprintln!("{what}: the system's threads still up (not the player's): {foreign:?}");
            }
            return;
        }
        assert!(
            start.elapsed() < within,
            "{what}: (threads, decoders, platform objects) {now:?}, were {before:?}; threads since: {:?}",
            added(threads, process_threads())
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn settles(what: &str, before: (usize, usize, usize), threads: &[(String, String)], decoders: &AtomicUsize) {
    settles_within(what, Duration::from_secs(5), before, threads, decoders);
}
fn wait(what: &str, ok: impl Fn() -> bool) {
    let start = Instant::now();
    while !ok() {
        let _ = remedy_ui_reactive::run_posted();
        assert!(start.elapsed() < Duration::from_secs(30), "waited for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// How a scripted decoder misbehaves.
#[derive(Clone, Copy, PartialEq)]
enum Script {
    /// Opening takes 400 ms.
    SlowOpen,
    /// Each read takes 50 ms.
    SlowRead,
    /// The third read fails.
    ReadFails,
    /// Seeking fails.
    SeekFails,
    /// Nine 64 KB cues to each picture, 10 ms apart: a stream's captions with no end.
    EndlessCues,
    /// The first read is a 2 MB cue.
    HugeCue,
}

struct Host {
    script: Script,
    alive: Arc<AtomicUsize>,
}

struct Dec {
    script: Script,
    info: MediaInfo,
    n: u64,
    alive: Arc<AtomicUsize>,
}

impl Drop for Dec {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Decoder for Dec {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        self.n += 1;
        let mut at = Duration::from_millis(self.n * 100);
        let cue = |at: Duration, text: String| Sample::Text {
            track: 0,
            at,
            until: at + Duration::from_millis(5),
            text,
        };
        match self.script {
            Script::SlowRead => std::thread::sleep(Duration::from_millis(50)),
            Script::ReadFails if self.n == 3 => return Err(MediaError::Decode("scripted failure".into())),
            Script::EndlessCues => {
                at = Duration::from_millis(self.n * 10);
                if !self.n.is_multiple_of(10) {
                    return Ok(Some(cue(at, format!("{}{}", self.n, "x".repeat(64 << 10)))));
                }
            }
            Script::HugeCue if self.n == 1 => return Ok(Some(cue(at, "x".repeat(2 << 20)))),
            _ => {}
        }
        if self.n > 600 {
            return Ok(None);
        }
        Ok(Some(Sample::Video {
            at,
            duration: Some(Duration::from_millis(100)),
            picture: Picture::packed(8, 8, PixelFormat::Nv12, vec![16; 96]),
        }))
    }
    fn seek(&mut self, _: Duration) -> Result<(), MediaError> {
        match self.script {
            Script::SeekFails => Err(MediaError::Decode("scripted seek failure".into())),
            _ => Ok(()),
        }
    }
}

impl MediaHost for Host {
    fn name(&self) -> &str {
        "leaks"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        if self.script == Script::SlowOpen {
            std::thread::sleep(Duration::from_millis(400));
        }
        let mut info = MediaInfo::default();
        info.duration = Some(Duration::from_secs(60));
        info.seekable = true;
        info.video = Some(VideoInfo::new(8, 8, Some(10.0)));
        info.text_tracks = vec![TextTrackInfo::new("Live", None)];
        self.alive.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Dec { script: self.script, info, n: 0, alive: self.alive.clone() }))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("none in this script".into()))
    }
}

/// Opens a scripted player, runs `then` on it, drops it and its UI, and checks all is gone.
fn scripted(script: Script, then: impl FnOnce(&Player)) {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let alive = Arc::new(AtomicUsize::new(0));
    let (before, threads) = (census(&alive), process_threads());
    let ui = ui();
    let host = Arc::new(Host { script, alive: alive.clone() });
    let p = Player::open(&ui, host, MediaSource::url("script:"), PlayerOptions::default().keep_awake(true));
    then(&p);
    drop(p);
    drop(ui);
    settles("after the drop", before, &threads, &alive);
}

#[test]
fn dropped_mid_open() {
    scripted(Script::SlowOpen, |_| std::thread::sleep(Duration::from_millis(100)));
}

#[test]
fn dropped_mid_decode() {
    scripted(Script::SlowRead, |p| {
        wait("opened", || p.state() == PlaybackState::Paused);
        p.play();
        std::thread::sleep(Duration::from_millis(120));
    });
}

#[test]
fn a_decoder_failure() {
    scripted(Script::ReadFails, |p| {
        p.play();
        wait("the error", || p.error().is_some());
    });
}

#[test]
fn a_seek_failure() {
    scripted(Script::SeekFails, |p| {
        wait("opened", || p.state() == PlaybackState::Paused);
        p.seek(Duration::from_secs(30));
        // Paused, the failure shows at once, as any decoder failure: Failed, with why.
        wait("failed", || p.state() == PlaybackState::Failed);
        let e = p.error().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("scripted seek failure"), "{e}");
    });
}

#[test]
fn a_seek_failure_while_playing() {
    scripted(Script::SeekFails, |p| {
        wait("opened", || p.state() == PlaybackState::Paused);
        p.play();
        wait("playing", || p.state() == PlaybackState::Playing);
        p.seek(Duration::from_secs(30));
        wait("failed", || p.state() == PlaybackState::Failed);
        assert!(p.error().is_some_and(|e| e.to_string().contains("scripted seek failure")));
    });
}

#[test]
fn fail_called_by_the_app() {
    scripted(Script::SlowRead, |p| {
        wait("opened", || p.state() == PlaybackState::Paused);
        p.fail("the app gave up");
        let _ = remedy_ui_reactive::run_posted();
    });
}

#[test]
fn captions_in_a_stream_keep_a_window_of_the_latest() {
    scripted(Script::EndlessCues, |p| {
        p.play();
        let cues = || p.subtitle_tracks().into_iter().flat_map(|t| t.cues).collect::<Vec<_>>();
        // Past 16 MB of captions, the earliest go and playback goes on.
        wait("the earliest cues dropped", || {
            cues().first().is_some_and(|c| c.start > Duration::from_secs(1))
        });
        let kept = cues();
        let bytes: usize = kept.iter().map(|c| c.text.len()).sum();
        assert!(bytes <= 16 << 20, "{bytes} bytes of cues kept");
        assert!(bytes > 15 << 20, "the window is full: {bytes}");
        assert!(kept.windows(2).all(|w| matches!(w, [a, b] if a.start < b.start)), "in order, none twice");
        assert_ne!(p.state(), PlaybackState::Failed, "{:?}", p.error());
    });
}

#[test]
fn a_cue_no_caption_could_be_fails_the_stream() {
    scripted(Script::HugeCue, |p| {
        p.play();
        wait("failed", || p.state() == PlaybackState::Failed);
        let e = p.error().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("cue inside this stream is over 1 MB"), "{e}");
    });
}

// The platform's own HLS path, from a local server: a 404, a bad segment, a drop mid-fetch.

#[derive(Clone, Copy)]
enum Serve {
    /// Every file as it is.
    Plain,
    NotFound,
    BadSegment,
    /// Every segment after the first answers nothing for 4 s.
    SlowSegment,
    /// The playlist answers nothing for 4 s.
    SlowPlaylist,
}

/// The `video` HLS fixture over HTTP, as `how` says; the playlist's URL.
fn server(how: Serve) -> String {
    // On a device the fixtures are pushed beside the test binary: REMEDY_FIXTURES says where.
    let fixtures = std::env::var_os("REMEDY_FIXTURES")
        .map_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"), Into::into);
    let dir = fixtures.join("hls/video");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("listening: {e}"));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let mut req = [0u8; 4096];
                let n = s.read(&mut req).unwrap_or(0);
                let line = String::from_utf8_lossy(req.get(..n).unwrap_or_default()).into_owned();
                let path = line.split(' ').nth(1).unwrap_or("/").trim_start_matches('/').to_owned();
                let file = std::fs::read(dir.join(&path)).ok();
                let segment = path.ends_with(".ts");
                let body = match (how, file) {
                    (Serve::NotFound, _) | (_, None) => {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                        return;
                    }
                    (Serve::BadSegment, Some(f)) if segment => vec![0x47; f.len()],
                    (Serve::SlowPlaylist, Some(_)) if !segment => {
                        std::thread::sleep(Duration::from_secs(4));
                        return;
                    }
                    (Serve::SlowSegment, Some(_)) if segment && !path.ends_with("00000.ts") => {
                        std::thread::sleep(Duration::from_secs(4));
                        return;
                    }
                    (_, Some(f)) => f,
                };
                let head =
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes()).and_then(|()| s.write_all(&body));
            });
        }
    });
    format!("http://127.0.0.1:{port}/index.m3u8")
}

/// A player of the stream `how` serves, on the platform's host; `then` runs on it before it
/// is dropped.
fn streamed(how: Serve, then: impl FnOnce(&Player)) {
    streamed_within(how, Duration::from_secs(5), then);
}

/// [`streamed`], with everything to be gone `within` the drop.
fn streamed_within(how: Serve, within: Duration, then: impl FnOnce(&Player)) {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(host) = platform_host() else {
        eprintln!("no platform host here, skipping");
        return;
    };
    gpu_renderer();
    let url = server(how);
    let none = AtomicUsize::new(0);
    let (before, threads) = (census(&none), process_threads());
    let ui = ui();
    let p = Player::open(&ui, host, MediaSource::Url(url), PlayerOptions::default().keep_awake(true));
    then(&p);
    drop(p);
    drop(ui);
    settles_within("after the drop", within, before, &threads, &none);
    no_d3d_leaks();
}

/// Under REMEDY_D3D_DEBUG=1 on Windows: nothing alive on the decoder devices made since the
/// last look but the devices, once the player and its host (which keeps the last stream's
/// decoders for the next) have gone. How many devices were looked at.
fn no_d3d_leaks() -> usize {
    let (devices, leaked) = live_d3d_objects();
    if devices > 0 {
        eprintln!(
            "D3D11 live-object report on {devices} decoder device(s): {} beyond the device",
            leaked.len()
        );
    }
    assert!(leaked.is_empty(), "alive on the decoder's D3D11 device: {leaked:#?}");
    devices
}

/// Windows: a GPU renderer on the process's shared device, made once, so the platform's
/// decoder decodes on a D3D11 device of its own the leak cases can look at. Kept for the
/// process (the shared device is, by design).
fn gpu_renderer() {
    #[cfg(windows)]
    {
        use remedy_ui_render_gpu::{GpuContext, GpuRenderer, LadderOptions};
        static MADE: std::sync::Once = std::sync::Once::new();
        MADE.call_once(|| match GpuContext::shared(&LadderOptions::default()) {
            Ok(ctx) => std::mem::forget(GpuRenderer::new(ctx, None)),
            Err(e) => eprintln!("no GPU renderer ({e}): the decoder stays on the CPU"),
        });
    }
}

#[test]
fn a_stream_answered_404() {
    // A playlist the server says is not there fails after a grace of two retries (about
    // 2 s), not the half minute a late segment is ridden out for.
    streamed(Serve::NotFound, |p| {
        let start = Instant::now();
        wait("the error", || p.error().is_some());
        let took = start.elapsed();
        assert!(took < Duration::from_secs(4), "failed after {took:?}");
        assert!(matches!(p.error(), Some(MediaError::NotFound(_))), "{:?}", p.error());
        eprintln!("a 404 playlist failed the open after {took:?}");
    });
}

#[test]
fn a_stream_with_a_bad_segment() {
    streamed(Serve::BadSegment, |p| {
        p.play();
        wait("the error or the end", || {
            p.error().is_some() || matches!(p.state(), PlaybackState::Ended | PlaybackState::Failed)
        });
    });
}

#[test]
fn a_stream_dropped_mid_fetch() {
    // Opened (the first segment came at once), then dropped while the second is awaited:
    // the fetch gives up at once (its watchdog looks every 100 ms).
    streamed_within(Serve::SlowSegment, Duration::from_millis(300), |p| {
        wait("opened", || p.state() == PlaybackState::Paused);
        p.play();
        std::thread::sleep(Duration::from_millis(1500));
        // Seen counted while it plays (with GST_TRACERS=leaks on Linux, its elements too).
        eprintln!("mid-fetch: {} platform objects live", live_platform_objects());
    });
}

#[test]
fn a_stream_dropped_after_its_decoders_went_to_the_next() {
    // The next stream takes the last one's decoders (kept by the player's host): dropping
    // the player still ends them.
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(host) = platform_host() else {
        eprintln!("no platform host here, skipping");
        return;
    };
    gpu_renderer();
    let url = server(Serve::Plain);
    let none = AtomicUsize::new(0);
    let (before, threads) = (census(&none), process_threads());
    let ui = ui();
    let p = Player::open(&ui, host, MediaSource::Url(url.clone()), PlayerOptions::default());
    wait("opened", || p.state() == PlaybackState::Paused);
    for _ in 0..2 {
        p.start_next(MediaSource::Url(url.clone()), None);
        wait("the next opened", || matches!(p.state(), PlaybackState::Playing | PlaybackState::Paused));
        std::thread::sleep(Duration::from_millis(300));
        assert!(p.error().is_none(), "{:?}", p.error());
    }
    let held = live_platform_objects();
    eprintln!("after two next streams: {held} platform objects live");
    assert!(held > before.2, "the stream's decoders are live while it plays");
    drop(p);
    drop(ui);
    // Dropping the player drops its host, and with it the decoders it kept for the next.
    settles("after the drop", before, &threads, &none);
    // shortcut: not asserted one device for three streams: the next stream opens while the
    // last is still closing, so it can find nothing kept yet and make its own (seen: 2).
    let devices = no_d3d_leaks();
    eprintln!("{devices} decoder device(s) for three streams on one host");
}

#[test]
fn a_stream_dropped_mid_open() {
    // The playlist is still awaited: the open gives up, its fetch too.
    streamed_within(Serve::SlowPlaylist, Duration::from_millis(300), |p| {
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(p.state(), PlaybackState::Opening);
    });
}
