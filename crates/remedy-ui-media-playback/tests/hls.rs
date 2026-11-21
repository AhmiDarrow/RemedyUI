//! HLS through the platform host: `tests/fixtures/hls/aac` (the test clip as three one-second
//! MPEG-TS segments) served over HTTP from this process, opened by URL, and played to the end,
//! sought, played while its playlist grows, and failing on a missing segment.
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

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls/aac").join(name))
        .unwrap_or_else(|e| unreachable!("{name}: {e}"))
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
    let mut map = HashMap::new();
    for n in ["seg00000.ts", "seg00001.ts", "seg00002.ts"] {
        map.insert(n.to_string(), fixture(n));
    }
    map.insert("index.m3u8".to_string(), playlist.into_bytes());
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
    let host = match platform_host() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("skipped: no platform host ({e})");
            return None;
        }
    };
    Some(host.open(&MediaSource::Url(url.into())).unwrap_or_else(|e| unreachable!("opening {url}: {e}")))
}

/// Every picture's time and the sound's length, read to the end.
fn read_all(d: &mut dyn Decoder) -> (Vec<Duration>, Duration) {
    let format = d.info().audio.unwrap_or_else(|| unreachable!("the clip has sound"));
    let (mut pictures, mut frames) = (Vec::new(), 0u64);
    let mut last_sound = None;
    while let Some(s) = d.read().unwrap_or_else(|e| unreachable!("reading: {e}")) {
        match s {
            Sample::Video { at, picture, .. } => {
                assert_eq!((picture.width, picture.height), (176, 100));
                pictures.push(at);
            }
            Sample::Audio { at, samples } => {
                assert!(last_sound.is_none_or(|l| at > l), "sound in order");
                last_sound = Some(at);
                frames += samples.len() as u64 / u64::from(format.channels);
            }
            _ => {}
        }
    }
    assert!(pictures.windows(2).all(|w| w.first() < w.last()), "pictures in order");
    (pictures, format.duration_of(frames))
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
