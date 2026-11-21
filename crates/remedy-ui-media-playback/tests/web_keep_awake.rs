//! A player keeping the page's screen on for real (`--features keep-awake`, run with the web
//! bridge's `tools/web-test.sh`): the Screen Wake Lock is asked for while the clip plays, let
//! go on pause and at the end.

#![cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "keep-awake"))]

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui_media_playback::{MediaSource, PlaybackState, Player, PlayerOptions, platform_host};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const CLIP: &[u8] = include_bytes!("fixtures/clip.mp4");

#[wasm_bindgen(inline_js = r#"
export function spy() {
  window.__locks = []; window.__lockErrors = [];
  const wl = navigator.wakeLock;
  if (!wl) return "this browser has no navigator.wakeLock";
  const request = wl.request.bind(wl);
  wl.request = (type) => {
    const p = request(type);
    p.then((s) => window.__locks.push(s), (e) => window.__lockErrors.push(String(e)));
    return p;
  };
  return "";
}
export function requested() { return window.__locks.length + window.__lockErrors.length; }
export function held() { return window.__locks.filter((s) => !s.released).length; }
export function errors() { return window.__lockErrors.join("; "); }
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function blobUrl(bytes) { return URL.createObjectURL(new Blob([bytes], { type: 'video/mp4' })); }
"#)]
extern "C" {
    fn spy() -> String;
    fn requested() -> u32;
    fn held() -> u32;
    fn errors() -> String;
    fn sleep(ms: u32) -> js_sys::Promise;
    #[wasm_bindgen(js_name = blobUrl)]
    fn blob_url(bytes: &[u8]) -> String;
}

/// Waits (10 ms at a time, up to `limit` times) for `ok`.
async fn wait_for(what: &str, limit: u32, ok: impl Fn() -> bool) {
    for _ in 0..limit {
        if ok() {
            return;
        }
        let _ = JsFuture::from(sleep(10)).await;
    }
    unreachable!("{what}: requested {}, held {}, refused {:?}", requested(), held(), errors());
}

#[wasm_bindgen_test]
async fn a_playing_video_holds_the_screen_wake_lock_and_lets_it_go() {
    let missing = spy();
    assert!(missing.is_empty(), "{missing}");
    let url = blob_url(CLIP);
    let slot: Rc<RefCell<Option<Player>>> = Rc::default();
    let s2 = slot.clone();
    let config = remedy_ui::AppConfig::new("Keep-awake test").cpu_only();
    remedy_ui::run(config, move |app: &remedy_ui::App| {
        let s3 = s2.clone();
        let url = url.clone();
        app.open(remedy_ui::WindowSpec::new("Keep-awake test"), move |ui, _| {
            let host = platform_host().unwrap_or_else(|e| unreachable!("host: {e}"));
            let p = Player::open(ui, host, MediaSource::url(url), PlayerOptions::default().keep_awake(true));
            let view = remedy_ui_media_playback::player(ui, &p, "Test clip").width(352.0).height(260.0);
            let _ = ui.root().child(view);
            *s3.borrow_mut() = Some(p);
        });
    })
    .unwrap_or_else(|e| unreachable!("the app starts: {e}"));
    wait_for("the player", 600, || slot.borrow().is_some()).await;
    let p = slot.borrow().clone().unwrap_or_else(|| unreachable!());
    wait_for("opened", 600, || p.state() == PlaybackState::Paused).await;
    assert_eq!(requested(), 0, "not while paused");
    p.play();
    wait_for("the lock while playing", 300, || held() == 1).await;
    assert_eq!(errors(), "", "the browser refused the lock");
    assert!(p.is_holding_awake());
    p.pause();
    wait_for("paused", 300, || p.state() == PlaybackState::Paused).await;
    wait_for("let go on pause", 100, || held() == 0).await;
    p.play();
    wait_for("the lock again", 300, || held() == 1).await;
    assert_eq!(requested(), 2);
    wait_for("the end", 600, || p.state() == PlaybackState::Ended).await;
    wait_for("let go at the end", 100, || held() == 0).await;
}
