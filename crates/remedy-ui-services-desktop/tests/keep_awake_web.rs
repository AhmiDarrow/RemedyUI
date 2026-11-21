//! The browser's Screen Wake Lock for real (run with the web bridge's `tools/web-test.sh`):
//! `power::inhibit(Inhibit::Display, ..)` asks for it, lets it go when dropped, and asks again
//! when the page shows after being hidden (the browser lets go of it while hidden).

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use remedy_ui_services_desktop::power::{Inhibit, inhibit};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
export function spy() {
  window.__locks = []; window.__lockErrors = [];
  const wl = navigator.wakeLock;
  if (!wl) return "this browser has no navigator.wakeLock";
  if (!wl.__spied) {
    const request = wl.request.bind(wl);
    wl.request = (type) => {
      const p = request(type);
      p.then((s) => window.__locks.push(s), (e) => window.__lockErrors.push(String(e)));
      return p;
    };
    wl.__spied = true;
  }
  return "";
}
export function requested() { return window.__locks.length + window.__lockErrors.length; }
export function held() { return window.__locks.filter((s) => !s.released).length; }
export function errors() { return window.__lockErrors.join("; "); }
// As the browser does when the tab is hidden: the page is hidden and its locks let go.
export function setVisible(v) {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (v ? "visible" : "hidden") });
  Object.defineProperty(document, "hidden", { configurable: true, get: () => !v });
  if (!v) for (const s of window.__locks) if (!s.released) s.release();
  document.dispatchEvent(new Event("visibilitychange"));
}
export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }
"#)]
extern "C" {
    fn spy() -> String;
    fn requested() -> u32;
    fn held() -> u32;
    fn errors() -> String;
    #[wasm_bindgen(js_name = setVisible)]
    fn set_visible(v: bool);
    fn sleep(ms: u32) -> js_sys::Promise;
}

/// Waits (10 ms at a time, up to 3 s) for `ok`, then a little more for anything after.
async fn settle(ok: impl Fn() -> bool) {
    for _ in 0..300 {
        if ok() {
            break;
        }
        let _ = JsFuture::from(sleep(10)).await;
    }
    let _ = JsFuture::from(sleep(50)).await;
}

#[wasm_bindgen_test]
async fn the_screen_wake_lock_is_held_let_go_and_asked_again_when_the_page_shows() {
    let missing = spy();
    assert!(missing.is_empty(), "{missing}");
    let hold = inhibit(Inhibit::Display, "keep-awake test", "Playing video")
        .unwrap_or_else(|e| unreachable!("the browser has a Screen Wake Lock: {e}"));
    settle(|| requested() >= 1).await;
    assert_eq!(errors(), "", "the browser refused the lock");
    assert_eq!((requested(), held()), (1, 1), "asked for and held");

    set_visible(false);
    settle(|| false).await;
    assert_eq!((requested(), held()), (1, 0), "let go while hidden, and not asked for");
    set_visible(true);
    settle(|| held() == 1).await;
    assert_eq!(errors(), "");
    assert_eq!((requested(), held()), (2, 1), "asked for again once the page shows");

    drop(hold);
    settle(|| held() == 0).await;
    assert_eq!(held(), 0, "let go with the hold");
    set_visible(false);
    set_visible(true);
    settle(|| false).await;
    assert_eq!(requested(), 2, "a hold that went asks no more");
}

#[wasm_bindgen_test]
fn a_page_keeps_only_the_screen_on() {
    assert!(inhibit(Inhibit::Sleep, "keep-awake test", "Downloading").is_err());
}
