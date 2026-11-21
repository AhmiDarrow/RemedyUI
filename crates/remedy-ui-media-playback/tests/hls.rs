//! HLS through the platform host: `tests/fixtures/hls/aac` (the test clip as three one-second
//! MPEG-TS segments) served over HTTP from this process, opened by URL, and played to the end,
//! sought, played while its playlist grows, and failing on a missing segment; the other
//! fixtures in `tests/fixtures/hls` played to the end; and a playlist opened as a local file.
//!
//! Skips, saying why, where there is no platform host. Not built for macOS, whose host refuses
//! HLS for now.

#![cfg(not(any(target_arch = "wasm32", target_os = "macos")))]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use remedy_ui_media_playback::*;

/// The files served, by path; the playlist can be changed while it is served.
type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

fn dir(stream: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls").join(stream)
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(dir("aac").join(name)).unwrap_or_else(|e| unreachable!("{name}: {e}"))
}

fn playlist() -> String {
    String::from_utf8(fixture("index.m3u8")).unwrap_or_else(|e| unreachable!("the playlist: {e}"))
}

/// The playlist with only its first `n` segments and no end: a stream still being made.
fn growing(n: usize) -> String {
    let mut out = String::new();
    let mut segments = 0;
    for line in playlist().lines() {
        if line.starts_with("#EXT-X-ENDLIST") || segments == n && line.starts_with("#EXTINF") {
            break;
        }
        segments += usize::from(line.ends_with(".ts"));
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Answers one request from `files`, at most `rate` bytes a second when there is one.
fn serve(mut s: TcpStream, files: &Files, rate: Option<usize>) -> std::io::Result<()> {
    let mut reader = BufReader::new(s.try_clone()?);
    let (mut line, mut path) = (String::new(), String::new());
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix("GET ") {
            path = rest.split(' ').next().unwrap_or_default().trim_start_matches('/').to_string();
        }
        if line.trim_end().is_empty() {
            break;
        }
    }
    let (body, stall) = {
        let mut files = files.lock().unwrap_or_else(PoisonError::into_inner);
        // Every request is logged under `asked`; a file marked `stall:` stops half way, once.
        files.entry("asked".into()).or_default().extend(format!("{path}\n").bytes());
        (files.get(&path).cloned(), files.remove(&format!("stall:{path}")).is_some())
    };
    let trickle =
        files.lock().unwrap_or_else(PoisonError::into_inner).contains_key(&format!("trickle:{path}"));
    let (status, body) = match body {
        Some(b) => ("200 OK", b),
        None => ("404 Not Found", Vec::new()),
    };
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if trickle {
        // A byte every tenth of a second, never quiet long enough to time out, until the client
        // hangs up; when it did is kept under `closed:` (ms since the Unix epoch).
        let (head, rest) = body.split_at(body.len() / 2);
        s.write_all(head)?;
        for b in rest.chunks(1) {
            if s.write_all(b).and_then(|()| s.flush()).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let at = now.as_millis().to_string().into_bytes();
        files.lock().unwrap_or_else(PoisonError::into_inner).insert(format!("closed:{path}"), at);
        return Ok(());
    }
    if stall {
        let (head, rest) = body.split_at(body.len() / 2);
        s.write_all(head)?;
        s.flush()?;
        std::thread::sleep(Duration::from_secs(3));
        return s.write_all(rest);
    }
    let Some(rate) = rate else { return s.write_all(&body) };
    // A tenth of a second's worth at a time.
    for chunk in body.chunks((rate / 10).max(1)) {
        s.write_all(chunk)?;
        std::thread::sleep(Duration::from_millis(100));
    }
    // Where the reader had got to (what it keeps under `pos`) when this was all sent.
    let mut files = files.lock().unwrap_or_else(PoisonError::into_inner);
    let pos = files.get("pos").cloned().unwrap_or_default();
    files.insert(format!("sent-at:{path}"), pos);
    Ok(())
}

/// A server on a free local port with the clip's segments and `playlist`; its playlist URL and
/// its files.
fn server(playlist: String) -> (String, Files) {
    serve_dir("aac", Some(playlist))
}

/// A server with the files of the fixture `stream` (its playlist replaced by `playlist`).
fn serve_dir(stream: &str, playlist: Option<String>) -> (String, Files) {
    serve_path(&dir(stream), playlist, None)
}

/// A server with the files in `folder` (its playlist replaced by `playlist`), sending at most
/// `rate` bytes a second when there is one.
fn serve_path(folder: &std::path::Path, playlist: Option<String>, rate: Option<usize>) -> (String, Files) {
    let mut map = HashMap::new();
    for e in std::fs::read_dir(folder).unwrap_or_else(|e| unreachable!("{folder:?}: {e}")).flatten() {
        let data = std::fs::read(e.path()).unwrap_or_else(|e| unreachable!("{folder:?}: {e}"));
        map.insert(e.file_name().to_string_lossy().into_owned(), data);
    }
    if let Some(p) = playlist {
        map.insert("index.m3u8".to_string(), p.into_bytes());
    }
    let files: Files = Arc::new(Mutex::new(map));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| unreachable!("a local port: {e}"));
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let served = files.clone();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let files = served.clone();
            std::thread::spawn(move || serve(s, &files, rate));
        }
    });
    (format!("http://127.0.0.1:{port}/index.m3u8"), files)
}

fn open(url: &str) -> Option<Box<dyn Decoder>> {
    open_source(&MediaSource::Url(url.into()))
}

fn open_source(source: &MediaSource) -> Option<Box<dyn Decoder>> {
    let host = match platform_host() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("skipped: no platform host ({e})");
            return None;
        }
    };
    Some(host.open(source).unwrap_or_else(|e| unreachable!("opening {source:?}: {e}")))
}

/// Every picture's time and the sound's length, read to the end.
fn read_all(d: &mut dyn Decoder) -> (Vec<Duration>, Duration) {
    let (pictures, sound) = read_any(d);
    (pictures, sound.unwrap_or_else(|| unreachable!("the clip has sound")))
}

/// Every picture's time and the sound's length (`None` without sound), read to the end. Each
/// piece of sound starts where the frames before it, at the format's rate, end.
fn read_any(d: &mut dyn Decoder) -> (Vec<Duration>, Option<Duration>) {
    let format = d.info().audio;
    let (mut pictures, mut frames) = (Vec::new(), 0u64);
    let (mut first_sound, mut last_sound) = (None, None);
    while let Some(s) = d.read().unwrap_or_else(|e| unreachable!("reading: {e}")) {
        match s {
            Sample::Video { at, picture, .. } => {
                assert_eq!((picture.width, picture.height), (176, 100));
                pictures.push(at);
            }
            Sample::Audio { at, samples } => {
                let format = format.unwrap_or_else(|| unreachable!("sound the info did not mention"));
                assert_eq!(samples.len() % usize::from(format.channels), 0, "whole frames");
                assert!(last_sound.is_none_or(|l| at > l), "sound in order");
                last_sound = Some(at);
                // Sound read at the wrong rate drifts from its times.
                let drift = (at - *first_sound.get_or_insert(at)).abs_diff(format.duration_of(frames));
                assert!(drift < Duration::from_millis(30), "the sound at {at:?} is {drift:?} off");
                frames += samples.len() as u64 / u64::from(format.channels);
            }
            _ => {}
        }
    }
    assert!(pictures.windows(2).all(|w| w.first() < w.last()), "pictures in order");
    (pictures, format.map(|f| f.duration_of(frames)))
}

#[test]
fn plays_to_the_end() {
    let (url, _files) = server(playlist());
    let Some(mut d) = open(&url) else { return };
    let info = d.info().clone();
    let length = info.duration.unwrap_or_default();
    assert!(length.abs_diff(Duration::from_secs(3)) < Duration::from_millis(100), "length {length:?}");
    let video = info.video.unwrap_or_else(|| unreachable!("the clip has video"));
    assert_eq!((video.width, video.height), (176, 100));
    let (pictures, sound) = read_all(d.as_mut());
    assert_eq!(pictures.len(), 75, "pictures");
    assert!(pictures.first().is_some_and(|t| *t < Duration::from_millis(50)), "first {:?}", pictures.first());
    assert!(pictures.last().is_some_and(|t| *t > Duration::from_millis(2900)), "last {:?}", pictures.last());
    assert!(sound > Duration::from_millis(2900), "sound {sound:?}");
}

#[test]
fn seeks_to_the_middle() {
    let (url, _files) = server(playlist());
    let Some(mut d) = open(&url) else { return };
    d.seek(Duration::from_millis(1500)).unwrap_or_else(|e| unreachable!("seeking: {e}"));
    let first = loop {
        match d.read().unwrap_or_else(|e| unreachable!("reading: {e}")) {
            Some(Sample::Video { at, .. }) => break at,
            Some(_) => {}
            None => unreachable!("no picture after the seek"),
        }
    };
    // The segment holding 1.5 s starts at its key frame, 1 s.
    assert!(
        first >= Duration::from_millis(960) && first <= Duration::from_millis(1540),
        "first picture {first:?}"
    );
}

#[test]
fn plays_a_growing_playlist() {
    let (url, files) = server(growing(2));
    let Some(mut d) = open(&url) else { return };
    // The rest of the stream, and its end, arrive while it plays.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert("index.m3u8".into(), playlist().into_bytes());
    });
    let (pictures, _) = read_all(d.as_mut());
    assert_eq!(pictures.len(), 75, "pictures");
    assert!(pictures.last().is_some_and(|t| *t > Duration::from_millis(2900)), "last {:?}", pictures.last());
}

/// Takes what the player's workers posted until `ok` holds, failing after a while.
fn wait_for(what: &str, ok: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while !ok() {
        let _ = remedy_ui_reactive::run_posted();
        assert!(start.elapsed() < Duration::from_secs(20), "waited for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// An event playlist growing through a `Player`: its length grows while paused; a seek past
/// the length so far is not cut to it and plays from there once the segments are listed; a
/// seek within works; once the playlist has ended, a seek past its end stops at the end.
#[test]
fn a_player_follows_a_growing_playlist() {
    let Ok(host) = platform_host() else { return eprintln!("skipped: no platform host") };
    let (url, files) = server(growing(1));
    let set = |p: String| {
        files.lock().unwrap_or_else(PoisonError::into_inner).insert("index.m3u8".into(), p.into_bytes())
    };
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let ui = remedy_ui_view::Ui::new(
        theme.clone(),
        remedy_ui_themes::sheet(&theme),
        remedy_ui_text::FontCollection::new(),
    );
    let p = Player::open(&ui, host, MediaSource::url(&url), PlayerOptions::default());
    let secs = |s: f64| Duration::from_secs_f64(s);
    let length = || p.info().map(|i| (i.duration, i.growing));
    wait_for("opened", || p.state() == PlaybackState::Paused);
    assert_eq!(length(), Some((Some(secs(1.0)), true)));
    // Paused, nothing read: the playlist is still asked again and the length grows.
    set(growing(2));
    wait_for("the length to grow while paused", || length() == Some((Some(secs(2.0)), true)));
    // Past the length so far: not cut to it.
    p.seek(secs(2.5));
    wait_for("the seek", || p.position() == secs(2.5));
    std::thread::sleep(Duration::from_millis(300));
    let _ = remedy_ui_reactive::run_posted();
    assert_eq!(p.position(), secs(2.5), "the seek waits for the playlist to get there");
    set(playlist());
    wait_for("the playlist's end", || length() == Some((Some(secs(3.0)), false)));
    p.play();
    wait_for("playing from 2.5 s to the end", || {
        assert!(p.position() >= secs(2.5), "went back to {:?}", p.position());
        p.state() == PlaybackState::Ended
    });
    // Within, and past the end of the ended playlist.
    p.seek(secs(1.2));
    wait_for("the seek within", || p.position() == secs(1.2));
    p.seek(secs(60.0));
    wait_for("the seek past the end", || p.position() == secs(3.0));
}

/// A 1080p stream of 4 MB, two-second segments (made with the `ffmpeg` on the PATH; skipped
/// without one), served at 6 MB/s, read at the pace it plays: segments are fetched ahead while
/// pictures are decoded, so each has arrived a second before playing reaches it, and opening
/// waits only for the playlist and the first segment. Told by where the reader was when each
/// segment had arrived, not by how late reads were: a loaded machine (a parallel build) slows
/// decoding, not the fetching ahead.
#[test]
fn big_segments_are_read_at_the_pace_they_play() {
    let folder = std::env::temp_dir().join(format!("remedy-hls-1080p-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&folder);
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=25,noise=alls=40:allf=t",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "6"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "16M",
            "-maxrate",
            "16M",
            "-bufsize",
            "16M",
        ])
        .args(["-g", "50", "-keyint_min", "50", "-sc_threshold", "0", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "128k", "-f", "hls", "-hls_time", "2", "-hls_playlist_type", "vod"])
        .arg("-hls_segment_filename")
        .arg(folder.join("seg%05d.ts"))
        .arg(folder.join("index.m3u8"))
        .status();
    if !made.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&folder);
        return eprintln!("skipped: no ffmpeg on the PATH to make the stream");
    }
    let (url, files) = serve_path(&folder, None, Some(6_000_000));
    let _ = std::fs::remove_dir_all(&folder);
    let opening = std::time::Instant::now();
    let Some(mut d) = open(&url) else { return };
    let opened = opening.elapsed();
    let asked_at_open = String::from_utf8_lossy(
        files.lock().unwrap_or_else(PoisonError::into_inner).get("asked").map_or(&[][..], |a| a),
    )
    .into_owned();
    let start = std::time::Instant::now();
    let (mut slowest, mut behind, mut pictures) = (Duration::ZERO, Duration::ZERO, 0);
    loop {
        let asked = std::time::Instant::now();
        let s = d.read().unwrap_or_else(|e| unreachable!("reading: {e}"));
        slowest = slowest.max(asked.elapsed());
        let Some(s) = s else { break };
        // Paced by the pictures: a segment carries its sound up to 0.7 s ahead of them, and a
        // player holds both.
        let Sample::Video { at, .. } = s else { continue };
        pictures += 1;
        let pos = at.as_millis().to_string().into_bytes();
        files.lock().unwrap_or_else(PoisonError::into_inner).insert("pos".into(), pos);
        // Played in time: wait for the picture's moment; a picture already late is behind.
        let now = start.elapsed();
        behind = behind.max(now.saturating_sub(at));
        std::thread::sleep(at.saturating_sub(now));
    }
    eprintln!(
        "opened in {opened:?}; slowest read {slowest:?}; at most {behind:?} behind; {pictures} pictures"
    );
    assert_eq!(pictures, 150, "pictures");
    // Opening fetches the playlist and one 4 MB segment (0.7 s at this rate), no more: told by
    // what was asked, not by the clock, which a loaded machine (other builds, GStreamer's
    // first start) slows to several seconds. The clock only catches a hang.
    assert_eq!(asked_at_open, "index.m3u8\nseg00000.ts\n", "what opening fetched");
    assert!(opened < Duration::from_secs(15), "opening took {opened:?}");
    let files = files.lock().unwrap_or_else(PoisonError::into_inner);
    for k in 1..3u64 {
        let at =
            files.get(&format!("sent-at:seg0000{k}.ts")).map(|p| String::from_utf8_lossy(p).into_owned());
        let at: u64 = at.and_then(|p| p.parse().ok()).unwrap_or_default();
        // It starts at 2k s; its sound plays up to 0.7 s before that, so a second ahead at least.
        assert!(at + 1000 <= 2000 * k, "segment {k} had arrived only at {at} ms, playing");
    }
}

#[test]
fn a_missing_segment_errors() {
    let (url, files) = server(playlist());
    files.lock().unwrap_or_else(PoisonError::into_inner).remove("seg00001.ts");
    let Some(mut d) = open(&url) else { return };
    let mut result = Ok(());
    for _ in 0..1000 {
        match d.read() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    assert!(result.is_err(), "reading past a segment that is not there");
}

/// How often `path` was asked for.
fn asked(files: &Files, path: &str) -> usize {
    let files = files.lock().unwrap_or_else(PoisonError::into_inner);
    files.get("asked").map_or(0, |a| String::from_utf8_lossy(a).lines().filter(|l| *l == path).count())
}

/// A segment the server does not have yet (ffmpeg still writing it) is waited for, then plays.
#[test]
fn a_late_segment_is_waited_for() {
    let (url, files) = server(growing(3));
    let seg = files.lock().unwrap_or_else(PoisonError::into_inner).remove("seg00001.ts");
    let Some(mut d) = open(&url) else { return };
    let late = files.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let mut files = late.lock().unwrap_or_else(PoisonError::into_inner);
        files.extend(seg.map(|s| ("seg00001.ts".to_string(), s)));
        files.insert("index.m3u8".into(), playlist().into_bytes());
    });
    let (pictures, _) = read_all(d.as_mut());
    assert_eq!(pictures.len(), 75, "pictures");
    assert!(asked(&files, "seg00001.ts") > 2, "asked again while it was missing");
}

/// A download that stops half way is dropped after a quiet while and fetched again.
#[test]
fn a_stalled_download_is_started_again() {
    let (url, files) = server(playlist());
    files.lock().unwrap_or_else(PoisonError::into_inner).insert("stall:seg00001.ts".into(), Vec::new());
    let Some(mut d) = open(&url) else { return };
    let (pictures, _) = read_all(d.as_mut());
    assert_eq!(pictures.len(), 75, "pictures");
    assert_eq!(asked(&files, "seg00001.ts"), 2, "fetched again after it stalled");
}

/// The fixture `stream` served over HTTP and read to the end: what it said it holds, every
/// picture's time and the sound's length.
fn play(stream: &str) -> Option<(MediaInfo, Vec<Duration>, Option<Duration>)> {
    let (url, _files) = serve_dir(stream, None);
    let mut d = open(&url)?;
    let info = d.info().clone();
    let (pictures, sound) = read_any(d.as_mut());
    Some((info, pictures, sound))
}

fn about(got: Option<Duration>, want: Duration) -> bool {
    got.is_some_and(|g| g.abs_diff(want) < Duration::from_millis(120))
}

#[test]
fn plays_mp3_sound() {
    let Some((info, pictures, sound)) = play("mp3") else { return };
    assert_eq!(info.audio, Some(AudioFormat::new(44100, 2)));
    assert_eq!(pictures.len(), 50, "pictures");
    assert!(about(sound, Duration::from_secs(2)), "sound {sound:?}");
}

#[test]
fn plays_video_without_sound() {
    let Some((info, pictures, sound)) = play("video") else { return };
    assert_eq!((info.audio, sound), (None, None));
    assert_eq!(pictures.len(), 75, "pictures");
    assert!(about(pictures.first().copied(), Duration::ZERO), "first {:?}", pictures.first());
    assert!(about(pictures.last().copied(), Duration::from_millis(2960)), "last {:?}", pictures.last());
}

/// The 33-bit clock wraps 1.32 s in: times still run on from 0 (read_any checks the order).
#[test]
fn plays_across_the_clock_wrap() {
    let Some((_, pictures, sound)) = play("wrap") else { return };
    assert_eq!(pictures.len(), 50, "pictures");
    assert!(about(pictures.first().copied(), Duration::ZERO), "first {:?}", pictures.first());
    assert!(about(pictures.last().copied(), Duration::from_millis(1960)), "last {:?}", pictures.last());
    assert!(about(sound, Duration::from_secs(2)), "sound {sound:?}");
}

/// AAC-LC at 24 kHz, mono: a decoder assuming SBR (twice the rate) must still give the
/// sound in the format it said, at the right speed (read_any checks its times).
#[test]
fn plays_low_rate_aac() {
    let Some((info, pictures, sound)) = play("mono24") else { return };
    assert!(pictures.is_empty() && info.audio.is_some(), "{info:?}");
    assert!(about(sound, Duration::from_millis(2048)), "sound {sound:?} as {:?}", info.audio);
}

/// 5.1 AAC: every channel passed on, as for a file.
#[test]
fn plays_surround_aac() {
    let Some((info, _, sound)) = play("surround") else { return };
    assert_eq!(info.audio.map(|a| a.channels), Some(6));
    assert!(about(sound, Duration::from_millis(1024)), "sound {sound:?}");
}

/// The clip's playlist opened as a local file, in a folder whose name has a space and a
/// non-ASCII letter, with a segment named so too.
#[test]
fn plays_a_local_playlist() {
    let folder = std::env::temp_dir().join(format!("remedy hls é {}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap_or_else(|e| unreachable!("{e}"));
    for n in ["seg00000.ts", "seg00001.ts", "seg00002.ts"] {
        let to = folder.join(if n == "seg00001.ts" { "seg ü 1.ts" } else { n });
        std::fs::write(to, fixture(n)).unwrap_or_else(|e| unreachable!("{e}"));
    }
    let index = folder.join("index.m3u8");
    std::fs::write(&index, playlist().replace("seg00001.ts", "seg ü 1.ts"))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let d = open_source(&MediaSource::File(index));
    let result = d.map(|mut d| read_all(d.as_mut()));
    let _ = std::fs::remove_dir_all(&folder);
    let Some((pictures, sound)) = result else { return };
    assert_eq!(pictures.len(), 75, "pictures");
    assert!(sound > Duration::from_millis(2900), "sound {sound:?}");
}

/// With a GPU renderer taking pictures on the GPU (made first, so under
/// `REMEDY_D3D_DEBUG=1` it reports the Direct3D debug layers' messages), the pictures stay
/// there and show red, green and blue in their seconds.
#[cfg(windows)]
#[test]
fn pictures_on_the_gpu() {
    use remedy_ui_render_gpu::{GpuContext, GpuRenderer, LadderOptions};
    let ctx = GpuContext::new(&LadderOptions::default()).unwrap_or_else(|e| unreachable!("GPU: {e}"));
    let _renderer = GpuRenderer::new(ctx, None);
    if remedy_ui_paint::gpu_video_adapter().is_none() {
        return eprintln!("skipped: this GPU cannot sample NV12 textures shared from D3D11");
    }
    let (url, _files) = server(playlist());
    let Some(mut d) = open(&url) else { return };
    let mut n = 0u64;
    while let Some(s) = d.read().unwrap_or_else(|e| unreachable!("reading: {e}")) {
        let Sample::Video { at, picture: p, .. } = s else { continue };
        assert!(p.gpu.is_some() && p.data.is_empty(), "the picture at {at:?} came through CPU memory");
        if n % 25 == 10 {
            let mut f =
                remedy_ui_paint::VideoFrame::packed(1, n, p.width, p.height, p.format, p.data.clone());
            f.matrix = p.matrix;
            f.full_range = p.full_range;
            f.gpu = p.gpu.clone();
            let rgba = f.to_rgba().unwrap_or_default();
            let mid = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
            let rgb = rgba.get(mid..mid + 3).unwrap_or_default();
            let strongest = (0..3).max_by_key(|&i| rgb.get(i).copied().unwrap_or(0));
            assert_eq!(strongest, Some((at.as_secs() as usize).min(2)), "at {at:?}: {rgb:?}");
        }
        n += 1;
    }
    assert_eq!(n, 75, "pictures");
}

/// A stream dropped while a segment is still coming (slowly, but never quiet long enough to
/// time out): the fetch is given up at once, not run on for up to a minute.
#[test]
fn dropping_a_stream_mid_fetch_hangs_up_promptly() {
    let (url, files) = server(playlist());
    let marked = |k: &str| files.lock().unwrap_or_else(PoisonError::into_inner).get(k).cloned();
    files.lock().unwrap_or_else(PoisonError::into_inner).insert("trickle:seg00001.ts".into(), Vec::new());
    let Some(mut d) = open(&url) else { return };
    let asked = |p: &str| marked("asked").is_some_and(|a| String::from_utf8_lossy(&a).contains(p));
    // Read until the trickling segment is being fetched.
    let started = std::time::Instant::now();
    while !asked("seg00001.ts") {
        assert!(started.elapsed() < Duration::from_secs(10), "the second segment is asked for");
        let _ = d.buffered();
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(marked("closed:seg00001.ts").is_none(), "still coming");
    let dropped = std::time::Instant::now();
    drop(d);
    while marked("closed:seg00001.ts").is_none() {
        assert!(dropped.elapsed() < Duration::from_secs(2), "the fetch hung up within 2 s of the drop");
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!("hung up {:?} after the drop", dropped.elapsed());
}
