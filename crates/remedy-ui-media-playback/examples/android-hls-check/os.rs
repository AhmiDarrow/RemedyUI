//! The example's JNI entry point: all of its unsafe code.

use std::ffi::c_void;
use std::time::Duration;

use remedy_ui_media_playback::{MediaSource, Sample, platform_host};

/// Plays `url` to the end: the pictures and the sound's length, or what went wrong.
fn play(url: &str) -> Result<(usize, Duration), String> {
    let host = platform_host().map_err(|e| format!("no platform host: {e}"))?;
    let mut d = host.open(&MediaSource::Url(url.into())).map_err(|e| format!("opening: {e}"))?;
    let format = d.info().audio.ok_or("no sound in the info")?;
    let (mut pictures, mut frames) = (0, 0u64);
    while let Some(s) = d.read().map_err(|e| format!("reading: {e}"))? {
        match s {
            Sample::Video { picture, .. } => {
                if (picture.width, picture.height) != (176, 100) {
                    return Err(format!("a {}x{} picture", picture.width, picture.height));
                }
                pictures += 1;
            }
            Sample::Audio { samples, .. } => frames += samples.len() as u64 / u64::from(format.channels),
            _ => {}
        }
    }
    Ok((pictures, format.duration_of(frames)))
}

/// Each check by name: the clip over https, from a server 5 s slow to answer each request
/// (within the 12 s allowed for a first byte), and from one whose segment stalls half way
/// for 4 s (past the 2.5 s allowed, so it is fetched again). How many failed.
#[unsafe(no_mangle)]
pub extern "C" fn Java_HlsCheck_run(_env: *mut c_void, _class: *mut c_void, port: i32) -> i32 {
    let mut failed = 0;
    for name in ["plain", "slow", "stall"] {
        let url = format!("https://127.0.0.1:{port}/{name}/aac/index.m3u8");
        let started = std::time::Instant::now();
        let got = play(&url);
        let ok = matches!(got, Ok((75, sound)) if sound > Duration::from_millis(2900));
        eprintln!(
            "hls-https-check: {name}: {} {got:?} in {:?}",
            if ok { "ok" } else { "FAILED" },
            started.elapsed()
        );
        failed += i32::from(!ok);
    }
    failed
}
