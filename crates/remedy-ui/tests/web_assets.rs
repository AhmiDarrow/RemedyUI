//! What a page fetches and reads rather than carries (run with
//! `remedy-ui-bridge-web/tools/web-test.sh`): fallback
//! faces fetched from beside the page the first time text needs them, the device's
//! interface font through the Local Font Access API, language names from the browser's own
//! CLDR data, and plain `localStorage` and `sessionStorage` items shared with other pages of
//! the origin.
//!
//! `REMEDY_FONTS_URL` (set by `web-test.sh`) is a server for `crates/remedy-ui-fonts`, whose
//! `fonts/` folder stands in for the one `cargo remedy package` writes beside a page.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;

use remedy_ui::reactive::{Persist, PersistStore, Plain};
use remedy_ui::view::Ui;
use remedy_ui::web::WebConfig;
use remedy_ui::{App, AppConfig, WindowSpec};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const FONTS_URL: Option<&str> = option_env!("REMEDY_FONTS_URL");

trait Must<T> {
    fn must(self, why: &str) -> T;
}

impl<T> Must<T> for Option<T> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|| unreachable!("{why}"))
    }
}

impl<T, E: std::fmt::Debug> Must<T> for Result<T, E> {
    fn must(self, why: &str) -> T {
        self.unwrap_or_else(|e| unreachable!("{why}: {e:?}"))
    }
}

async fn frame() {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window().must("a window").request_animation_frame(&resolve);
    });
    let _ = JsFuture::from(p).await;
}

async fn until(limit: u32, ok: impl Fn() -> bool) -> bool {
    for _ in 0..limit {
        if ok() {
            return true;
        }
        frame().await;
    }
    ok()
}

fn storage() -> web_sys::Storage {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten()).must("localStorage")
}

/// The names of the files the page fetched.
fn fetched() -> Vec<String> {
    let perf = web_sys::window().and_then(|w| w.performance()).must("performance");
    perf.get_entries_by_type("resource")
        .iter()
        .filter_map(|e| js_sys::Reflect::get(&e, &"name".into()).ok()?.as_string())
        .collect()
}

type Opened = Rc<RefCell<Option<(App, Ui)>>>;

#[wasm_bindgen_test]
async fn fallback_faces_are_fetched_when_text_first_needs_them() {
    let Some(base) = FONTS_URL else {
        web_sys::console::warn_1(&"REMEDY_FONTS_URL is not set: the fetched faces are not tested".into());
        return;
    };
    let opened: Opened = Rc::default();
    let o = opened.clone();
    let config = AppConfig::new("Fonts test").cpu_only().web(WebConfig::default().assets(base));
    remedy_ui::run(config, move |app: &App| {
        let app2 = app.clone();
        app.open(WindowSpec::new("Fonts"), move |ui, _| {
            let _ = ui.root().child(ui.el("label").text("Hello"));
            *o.borrow_mut() = Some((app2, ui.clone()));
        });
    })
    .must("the app starts");
    assert!(until(240, || opened.borrow().is_some()).await, "the window was built");
    let (app, ui) = opened.borrow().clone().must("opened");
    let has = |f: &str| ui.font_families().iter().any(|g| g == f);
    assert!(has("Remedy Sans UI") && !has("Noto Sans Arabic"), "the script faces are not in the module");
    assert!(!fetched().iter().any(|f| f.contains("NotoSans")), "nothing fetched before text needs it");

    // Arabic text: its face is fetched, and the text drawn with it once it arrives.
    let _ = ui.root().child(ui.el("label").text("مرحبا بالعالم"));
    assert!(until(600, || has("Noto Sans Arabic")).await, "the Arabic face arrived: {:?}", ui.font_families());
    let got = fetched();
    assert!(got.iter().any(|f| f.ends_with("fonts/NotoSansArabic-Regular.ttf")), "{got:?}");
    assert!(!got.iter().any(|f| f.contains("NotoSansSC") || f.contains("NotoSansJP")), "only what was needed: {got:?}");
    app.quit();
    assert!(until(60, || !remedy_ui::web::is_running()).await);
}

#[wasm_bindgen_test]
async fn the_system_ui_font_is_read_where_the_browser_allows_and_refused_with_a_reason_elsewhere() {
    let opened: Opened = Rc::default();
    let o = opened.clone();
    remedy_ui::run(AppConfig::new("System font test").cpu_only(), move |app: &App| {
        let app2 = app.clone();
        app.open(WindowSpec::new("System"), move |ui, _| {
            *o.borrow_mut() = Some((app2, ui.clone()));
        });
    })
    .must("the app starts");
    assert!(until(240, || opened.borrow().is_some()).await);
    let (app, ui) = opened.borrow().clone().must("opened");
    let answer: Rc<RefCell<Option<Result<Vec<String>, String>>>> = Rc::default();
    let a = answer.clone();
    app.load_system_ui_font(move |got| *a.borrow_mut() = Some(got));
    assert!(until(600, || answer.borrow().is_some()).await, "an answer came");
    let has_api = js_sys::Reflect::has(&js_sys::global(), &"queryLocalFonts".into()).unwrap_or(false);
    match answer.borrow().clone().must("answered") {
        // A test page has no click and no permission: only a browser that grants it reads it.
        Ok(families) => {
            assert!(has_api && !families.is_empty());
            assert_eq!(ui.update_fonts(|f| f.generic_family("system-ui")).first(), families.first());
        }
        Err(why) => {
            assert!(!why.is_empty());
            if !has_api {
                assert!(why.contains("Local Font Access"), "{why}");
            }
        }
    }
    // Telling the face apart by measuring needs no permission.
    let none = remedy_ui::web::fonts::system_ui_family(&["No Such Family Anywhere".into()]);
    assert_eq!(none, Ok(None));
    app.quit();
    assert!(until(60, || !remedy_ui::web::is_running()).await);
}

#[wasm_bindgen_test]
fn language_names_come_from_the_browsers_own_cldr_data() {
    use remedy_ui::i18n::language_name_in;
    assert_eq!(language_name_in("de", "fr"), "Französisch");
    assert_eq!(language_name_in("en", "ger"), "German", "ISO 639-2 codes, as media files tag tracks");
    assert_eq!(language_name_in("en", "fre"), "French");
    assert_eq!(language_name_in("en", "pt-BR"), "Brazilian Portuguese");
    assert_eq!(language_name_in("en", "zh-Hant"), "Traditional Chinese");
    assert_eq!(language_name_in("ko", "de"), "독일어");
    assert_eq!(language_name_in("en", "qqq-not-a-language"), "qqq-not-a-language", "unnamed comes back as given");
}

#[wasm_bindgen_test]
async fn plain_items_are_the_pages_own_shared_with_its_other_pages() {
    let s = storage();
    s.set_item("remedy-test-theme", "midnight").must("set");
    s.set_item("remedy-test-fonts", r#"{"text":"ahmi-print","headings":"system"}"#).must("set");
    let store = PersistStore::local_storage("Plain test").must("a page store");
    let theme = Persist::new("remedy-test-theme").plain(Plain::Text);
    let fonts = Persist::new("remedy-test-fonts").plain(Plain::Json);
    assert_eq!(store.load::<String>(&theme).must("reads"), Some("midnight".to_owned()));
    let f: serde_json::Value = store.load(&fonts).must("reads").must("there");
    assert_eq!(f["headings"], "system");
    store.save(&theme, "paper").must("saves");
    store.save(&fonts, &serde_json::json!({"text": "remedy-sans", "headings": "ahmi-pixel"})).must("saves");
    store.flush().must("written");
    assert_eq!(s.get_item("remedy-test-theme").must("get").as_deref(), Some("paper"), "no quotes, no envelope");
    assert_eq!(
        s.get_item("remedy-test-fonts").must("get").as_deref(),
        Some(r#"{"headings":"ahmi-pixel","text":"remedy-sans"}"#)
    );
    store.forget(&theme).must("removes");
    store.forget(&fonts).must("removes");
    store.flush().must("written");
    assert_eq!(s.get_item("remedy-test-theme").must("get"), None);
}

#[wasm_bindgen_test]
fn session_items_are_the_tabs_own_session_storage() {
    let s = web_sys::window().and_then(|w| w.session_storage().ok().flatten()).must("sessionStorage");
    s.set_item("remedy-test-seasons", r#"{"seasonToShow":"tt1","seasonNumber":3,"showName":"Show","labels":{"1":"One"}}"#)
        .must("set");
    let store = PersistStore::local_storage("Session test").must("a page store");
    let seasons = Persist::new("remedy-test-seasons").session();
    let v: serde_json::Value = store.load(&seasons).must("reads").must("there");
    assert_eq!(v["seasonNumber"], 3);
    store.save(&seasons, &serde_json::json!({"labels": {}, "seasonNumber": 4, "seasonToShow": "tt2", "showName": "Other"})).must("saves");
    // Written at once, with no debounce: a tab may close at any moment.
    assert_eq!(
        s.get_item("remedy-test-seasons").must("get").as_deref(),
        Some(r#"{"labels":{},"seasonNumber":4,"seasonToShow":"tt2","showName":"Other"}"#)
    );
    assert_eq!(storage().get_item("remedy-test-seasons").must("get"), None, "not in localStorage");
    store.forget(&seasons).must("removes");
    assert_eq!(s.get_item("remedy-test-seasons").must("get"), None);
}
