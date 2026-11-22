//! Images from addresses in a real browser page (run with the web bridge's
//! `tools/web-test.sh`, which starts `tools/test-serve.py` on another port, so every image is
//! from another origin): a server that allows it (CORS) shows, its real size reported; one
//! that does not fails, saying why, and the app's fallback shows; a fetch is aborted when its
//! element goes.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::Size;
use remedy_ui_media_image::*;
use remedy_ui_text::FontCollection;
use remedy_ui_view::{El, Ui};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SERVER: Option<&str> = option_env!("REMEDY_IMAGE_URL");

async fn sleep(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms);
        }
    });
    let _ = JsFuture::from(p).await;
}

fn ui() -> Ui {
    let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
    let mut fonts = FontCollection::new();
    for face in remedy_ui_fonts::all() {
        let _ = fonts.add_static(face.data);
    }
    Ui::new(theme.clone(), remedy_ui_themes::sheet(&theme), fonts)
}

/// Frames, cooperative work and the page's own events until `done`, or 10 seconds.
async fn until(ui: &Ui, done: impl Fn() -> bool) -> bool {
    let start = js_sys::Date::now();
    while js_sys::Date::now() - start < 10_000.0 {
        let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
        let _ = remedy_ui_core::exec::run_pending(Duration::from_millis(8));
        let _ = remedy_ui_reactive::run_posted();
        if done() {
            let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
            return true;
        }
        sleep(10).await;
    }
    false
}

fn server() -> Option<&'static str> {
    if SERVER.is_none() {
        web_sys::console::log_1(&"REMEDY_IMAGE_URL not set at build time: skipping".into());
    }
    SERVER
}

#[wasm_bindgen_test]
async fn a_cors_image_from_another_origin_shows_with_its_real_size() {
    let Some(base) = server() else { return };
    let ui = ui();
    let sizes: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
    let s2 = sizes.clone();
    let el = image_ext(
        &ui,
        Source::Url(format!("{base}/cors.png")),
        "Art",
        Fit::Contain,
        ImageOptions::new().on_state(move |s| {
            if let ImageState::Ready(_, z) = s {
                s2.borrow_mut().push(*z);
            }
        }),
    );
    ui.root().child(ui.el("row").row().child(el.clone()));
    assert!(until(&ui, || !sizes.borrow().is_empty()).await, "loaded");
    assert_eq!(sizes.borrow().first(), Some(&(30, 10)));
    let b = el.bounds().unwrap_or_default();
    assert_eq!((b.width(), b.height()), (30.0, 10.0));
    el.remove();
    let _ = ui.frame(Size::new(400.0, 300.0), 1.0);
    assert_eq!(cached(), 0, "freed");
    assert_eq!(fetching(), 0);
}

#[wasm_bindgen_test]
async fn an_image_without_cors_fails_saying_why_and_the_fallback_shows() {
    let Some(base) = server() else { return };
    let ui = ui();
    let why: Rc<RefCell<Option<String>>> = Rc::default();
    let (w2, ui2) = (why.clone(), ui.clone());
    let el = image_ext(
        &ui,
        Source::Url(format!("{base}/nocors.png")),
        "Poster",
        Fit::Cover,
        ImageOptions::new().fallback(move || ui2.el("box").text("P").width(40.0).height(60.0)).on_state(
            move |s| {
                if let ImageState::Failed(e) = s {
                    *w2.borrow_mut() = Some(e.to_string());
                }
            },
        ),
    );
    ui.root().child(ui.el("row").row().child(el.clone()));
    assert!(until(&ui, || why.borrow().is_some()).await, "failed");
    assert!(why.borrow().as_deref().is_some_and(|w| w.contains("CORS")), "{:?}", why.borrow());
    let _ = until(&ui, || el.bounds().is_some_and(|b| b.height() == 60.0)).await;
    let b = el.bounds().unwrap_or_default();
    assert_eq!((b.width(), b.height()), (40.0, 60.0), "the fallback shows");
}

#[wasm_bindgen_test]
async fn a_fetch_is_aborted_when_its_element_goes() {
    let Some(base) = server() else { return };
    let ui = ui();
    let el: El =
        image(&ui, Source::Url(format!("{base}/slow.png")), "Slow", Fit::Contain).width(30.0).height(10.0);
    ui.root().child(el.clone());
    assert!(until(&ui, || fetching() == 1).await);
    sleep(200).await;
    el.remove();
    assert_eq!(fetching(), 0, "nobody waits for it");
    // The abort reaches the server: its trickle (50 s long) finds the connection closed.
    let start = js_sys::Date::now();
    let mut cut = false;
    while !cut && js_sys::Date::now() - start < 8_000.0 {
        sleep(100).await;
        cut = status(&format!("{base}/status")).await == "cut";
    }
    assert!(cut, "the request was aborted");
    assert_eq!(cached(), 0);
}

async fn status(url: &str) -> String {
    let Some(w) = web_sys::window() else { return String::new() };
    let Ok(resp) = JsFuture::from(w.fetch_with_str(url)).await else { return String::new() };
    let resp: web_sys::Response = resp.unchecked_into();
    let Ok(text) = resp.text() else { return String::new() };
    JsFuture::from(text).await.ok().and_then(|t| t.as_string()).unwrap_or_default()
}
