//! Images from addresses in a real browser page (run with the web bridge's
//! `tools/web-test.sh`, which starts `tools/test-serve.py` on another port, so every image is
//! from another origin): a server that allows it (CORS) shows, its real size reported; one
//! that does not fails, saying why, and the app's fallback shows, unless the app names an
//! image proxy (`set_web_proxy`), through which it shows; a fetch is aborted when its element
//! goes.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::Size;
use remedy_ui_media_image::*;
use remedy_ui_text::FontCollection;
use remedy_ui_engine::view::{El, Ui};
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
        let _ = remedy_ui_core::reactive::run_posted();
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
    set_web_proxy(None);
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
async fn an_image_without_cors_shows_through_the_apps_proxy() {
    let Some(base) = server() else { return };
    set_web_proxy(Some(format!("{base}/proxy?url=")));
    let ui = ui();
    let size: Rc<RefCell<Option<(u32, u32)>>> = Rc::default();
    let s2 = size.clone();
    // Its own address: the failure test's is not remembered as fetched.
    let el = image_ext(
        &ui,
        Source::Url(format!("{base}/nocors.png?via=proxy")),
        "Poster",
        Fit::Contain,
        ImageOptions::new().on_state(move |s| {
            if let ImageState::Ready(_, sz) = s {
                *s2.borrow_mut() = Some(*sz);
            }
        }),
    );
    ui.root().child(el.clone());
    let shown = until(&ui, || size.borrow().is_some()).await;
    set_web_proxy(None);
    assert!(shown, "shown through the proxy");
    assert_eq!(*size.borrow(), Some((30, 10)));
    el.remove();
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

/// Waits for `state` to settle on a decoded image or a failure.
async fn settled(ui: &Ui, state: remedy_ui_core::reactive::Signal<ImageState>) -> ImageState {
    let _ = until(ui, || {
        state.with_untracked(|s| matches!(s, ImageState::Ready(..) | ImageState::Failed(_))).unwrap_or(false)
    })
    .await;
    state.get_untracked().unwrap_or(ImageState::Loading)
}

#[wasm_bindgen_test]
async fn the_browser_decodes_and_scales_pictures_to_the_size_they_show() {
    let ui = ui();
    // A 300×200 gradient shown in a 60×60 box: decoded at 60×40, the size it is drawn.
    let mut png = Vec::new();
    let pic =
        image::RgbaImage::from_fn(300, 200, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]));
    assert!(
        image::DynamicImage::ImageRgba8(pic)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
    );
    let state = remedy_ui_core::reactive::Signal::new(ImageState::Loading);
    let _req = load(&Source::Bytes(png.into()), Some((60, 60)), false, state);
    match settled(&ui, state).await {
        ImageState::Ready(img, source) => {
            assert_eq!((img.width, img.height), (60, 40), "decoded at the size it shows");
            assert_eq!(source, (300, 200), "its own size is reported");
        }
        other => unreachable!("not decoded: {other:?}"),
    }
    // AVIF: a format this build's decoder does not read, so only the browser's decoder can
    // have decoded it.
    let avif: &[u8] = include_bytes!("fixtures/red.avif");
    let state = remedy_ui_core::reactive::Signal::new(ImageState::Loading);
    let _req = load(&Source::Bytes(avif.into()), Some((32, 32)), false, state);
    match settled(&ui, state).await {
        ImageState::Ready(img, source) => {
            assert_eq!(source, (64, 48));
            assert_eq!((img.width, img.height), (32, 24), "scaled once its size was known");
            let px = img.pixels.get(..4).map(<[u8]>::to_vec).unwrap_or_default();
            assert!(px.first().is_some_and(|r| r.abs_diff(200) < 8) && px.get(3) == Some(&255), "{px:?}");
        }
        other => unreachable!("the browser's decoder was not used: {other:?}"),
    }
    assert!(matches!(decode(avif, None), Err(DecodeError::Unrecognized)), "the engine's own cannot");
}
