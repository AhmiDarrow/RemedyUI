//! HLS through the platform host: `tests/fixtures/hls/aac` (the test clip as three one-second
//! MPEG-TS segments) served over HTTP from this process, opened by URL, and played to the end,
//! sought, played while its playlist grows, and failing on a missing segment; the other
//! fixtures in `tests/fixtures/hls` played to the end; and a playlist opened as a local file.
//!
//! Skips, saying why, where there is no platform host.

#![cfg(not(target_arch = "wasm32"))]

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

fn serve(mut s: TcpStream, files: &Files) -> std::io::Result<()> {
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
    let body = files.lock().unwrap_or_else(PoisonError::into_inner).get(&path).cloned();
    let (status, body) = match body {
        Some(b) => ("200 OK", b),
        None => ("404 Not Found", Vec::new()),
    };
    write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(&body)
}

/// A server on a free local port with the clip's segments and `playlist`; its playlist URL and
/// its files.
fn server(playlist: String) -> (String, Files) {
    serve_dir("aac", Some(playlist))
}

/// A server with the files of the fixture `stream` (its playlist replaced by `playlist`).
fn serve_dir(stream: &str, playlist: Option<String>) -> (String, Files) {
    let mut map = HashMap::new();
    for e in std::fs::read_dir(dir(stream)).unwrap_or_else(|e| unreachable!("{stream}: {e}")).flatten() {
        let data = std::fs::read(e.path()).unwrap_or_else(|e| unreachable!("{stream}: {e}"));
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
            std::thread::spawn(move || serve(s, &files));
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
