//! The browser's media session for real (run with the web bridge's `tools/web-test.sh`): what
//! plays shows in `navigator.mediaSession`, with its state, and leaves it again.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::time::Duration;

use js_sys::Reflect;
use remedy_ui_platform::services_desktop::media_session::{MediaSession, MediaState, NowPlaying, NowPlayingSink};
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

fn text(o: &JsValue, k: &str) -> Option<String> {
    get(o, k).as_string()
}

#[wasm_bindgen_test]
fn the_pages_media_session_shows_what_plays() {
    let ms = get(&get(&js_sys::global(), "navigator"), "mediaSession");
    let session = MediaSession::new(|_| {})
        .unwrap_or_else(|e| unreachable!("a browser has navigator.mediaSession: {e}"));
    session.set_now_playing(Some(&NowPlaying::new("The Ris").subtitle("S1 E3")));
    session.set_playback(MediaState::Playing, Duration::from_secs(30), Some(Duration::from_secs(100)));
    session.set_can_next(true);
    let meta = get(&ms, "metadata");
    assert_eq!(text(&meta, "title").as_deref(), Some("The Ris"));
    assert_eq!(text(&meta, "artist").as_deref(), Some("S1 E3"));
    assert_eq!(text(&ms, "playbackState").as_deref(), Some("playing"));

    session.set_playback(MediaState::Paused, Duration::from_secs(31), Some(Duration::from_secs(100)));
    assert_eq!(text(&ms, "playbackState").as_deref(), Some("paused"));

    session.set_now_playing(None);
    assert!(get(&ms, "metadata").is_null(), "left the controls");
    assert_eq!(text(&ms, "playbackState").as_deref(), Some("none"));
}
