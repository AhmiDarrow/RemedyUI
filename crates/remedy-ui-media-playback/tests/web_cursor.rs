//! The mouse pointer over a full-screen player in a real browser (run with the web bridge's
//! `tools/web-test.sh`): the page's canvas takes `cursor: none` once the controls rest, and
//! its own pointer again when the mouse moves. A page of its own: one app per page.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_media_playback::{
    AudioFormat, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, PlaybackState, Player,
    PlayerOptions, Sample, VideoInfo,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
export function pointerMove(target, x, y) {
  target.dispatchEvent(new PointerEvent('pointermove', {
    bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y,
    pointerId: 1, pointerType: 'mouse', isPrimary: true, buttons: 0, button: -1,
  }));
}
export function cursorOf(el) { return getComputedStyle(el).cursor; }
"#)]
extern "C" {
    fn sleep(ms: u32) -> js_sys::Promise;
    #[wasm_bindgen(js_name = pointerMove)]
    fn pointer_move(target: &web_sys::EventTarget, x: f64, y: f64);
    #[wasm_bindgen(js_name = cursorOf)]
    fn cursor_of(el: &web_sys::Element) -> String;
}

async fn wait_ms(ms: u32) {
    let _ = JsFuture::from(sleep(ms)).await;
}

/// Waits (10 ms at a time, up to `limit` times) for `ok`.
async fn until(limit: u32, ok: impl Fn() -> bool) -> bool {
    for _ in 0..limit {
        if ok() {
            return true;
        }
        wait_ms(10).await;
    }
    ok()
}

const RATE: u32 = 8000;

/// A minute of silence with a picture's size and no pictures, so the player plays by the
/// clock with its stage shown.
struct Silence {
    info: MediaInfo,
    next: u32,
}

impl Decoder for Silence {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        if self.next == 600 {
            return Ok(None);
        }
        let at = Duration::from_millis(u64::from(self.next) * 100);
        self.next += 1;
        Ok(Some(Sample::Audio { at, samples: vec![0.0; (RATE / 10) as usize] }))
    }
    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.next = u32::try_from(to.as_millis() / 100).unwrap_or(600).min(600);
        Ok(())
    }
}

struct SilenceHost;

impl MediaHost for SilenceHost {
    fn name(&self) -> &str {
        "silence"
    }
    fn open(&self, _: &MediaSource) -> Result<Box<dyn Decoder>, MediaError> {
        let mut info = MediaInfo::default();
        info.duration = Some(Duration::from_secs(60));
        info.seekable = true;
        info.audio = Some(AudioFormat::new(RATE, 1));
        info.video = Some(VideoInfo::new(640, 360, None));
        Ok(Box::new(Silence { info, next: 0 }))
    }
    fn audio_output(&self, _: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError> {
        Err(MediaError::Device("no sound device here".into()))
    }
}

#[wasm_bindgen_test]
async fn the_canvas_hides_the_pointer_while_full_screen_controls_rest() {
    let slot: Rc<RefCell<Option<Player>>> = Rc::default();
    let s2 = slot.clone();
    let config = remedy_ui::AppConfig::new("Cursor test").cpu_only();
    remedy_ui::run(config, move |app: &remedy_ui::App| {
        let s3 = s2.clone();
        app.open(remedy_ui::WindowSpec::new("Cursor test"), move |ui, _| {
            let p = Player::open(
                ui,
                Arc::new(SilenceHost),
                MediaSource::url("silence:"),
                PlayerOptions::default(),
            );
            let _ = ui.root().child(remedy_ui_media_playback::player(ui, &p, "Silence"));
            // The player's full screen alone: a page may fill the screen only on a click.
            p.request_fullscreen(true);
            p.play();
            *s3.borrow_mut() = Some(p);
        });
    })
    .unwrap_or_else(|e| unreachable!("the app starts: {e:?}"));
    assert!(until(600, || slot.borrow().is_some()).await, "the window opened");
    let p = slot.borrow().clone().unwrap_or_else(|| unreachable!("the player"));
    assert!(until(600, || p.state() == PlaybackState::Playing).await, "playing: {:?}", p.state());
    let doc = web_sys::window().and_then(|w| w.document()).unwrap_or_else(|| unreachable!("a document"));
    let canvas = doc
        .query_selector("[data-remedy-window] canvas")
        .ok()
        .flatten()
        .unwrap_or_else(|| unreachable!("the window's canvas"));
    let r = canvas.get_bounding_client_rect();
    let (cx, cy) = (r.left() + r.width() / 2.0, r.top() + r.height() / 3.0);
    let target: &web_sys::EventTarget = canvas.unchecked_ref();

    // The mouse over the picture: the controls and the pointer show.
    pointer_move(target, cx, cy);
    assert!(until(50, || p.controls_shown()).await);
    assert_ne!(cursor_of(&canvas), "none", "the pointer shows while the controls do");
    // 2.6 s with no activity: both rest.
    assert!(until(500, || !p.controls_shown()).await, "the controls rest");
    assert!(until(50, || cursor_of(&canvas) == "none").await, "the pointer hides: {}", cursor_of(&canvas));
    // A move brings both back.
    pointer_move(target, cx + 10.0, cy + 10.0);
    assert!(until(50, || p.controls_shown()).await, "a move wakes them");
    assert!(until(50, || cursor_of(&canvas) != "none").await, "the pointer is back: {}", cursor_of(&canvas));
}
