//! The page's own localStorage in a real browser (run with `wasm-bindgen-test-runner` as the
//! wasm32 runner, with geckodriver or chromedriver).

#![cfg(target_arch = "wasm32")]

use js_sys::{Function, JsString, Reflect};
use remedy_ui_platform::services_webstorage::browser_local_storage;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// `localStorage.setItem(key, value)` with JavaScript strings, so a lone surrogate reaches the
/// store as it would from a page.
fn set_item(key: &JsString, value: &JsString) {
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
    let Some(storage) = storage else { unreachable!("the test page has localStorage") };
    let storage: &JsValue = storage.as_ref();
    let set =
        Reflect::get(storage, &JsValue::from_str("setItem")).ok().and_then(|f| f.dyn_into::<Function>().ok());
    let Some(set) = set else { unreachable!("setItem") };
    assert!(set.call2(storage, key, value).is_ok());
}

#[wasm_bindgen_test]
fn reads_every_item_of_the_page() {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        assert!(storage.clear().is_ok());
    }
    set_item(&"german".into(), &"Grüße aus Köln".into());
    set_item(&"emoji".into(), &"🎉 مرحبا".into());
    set_item(&"empty".into(), &"".into());
    let lone = JsString::from_char_code1(0xD800);
    set_item(&"lone".into(), &lone);

    let saved = match browser_local_storage() {
        Ok(saved) => saved,
        Err(e) => unreachable!("{e}"),
    };
    let origin = web_sys::window().and_then(|w| w.location().origin().ok());
    assert_eq!(Some(saved.origin.clone()), origin);
    assert_eq!(saved.get("german"), Some("Grüße aus Köln"));
    assert_eq!(saved.get("emoji"), Some("🎉 مرحبا"));
    assert_eq!(saved.get("empty"), Some(""));
    assert_eq!(saved.items.len(), 3);
    assert_eq!(saved.unpaired.len(), 1);
    let item = saved.unpaired.first();
    assert_eq!(item.map(|i| i.key.clone()), Some("lone".encode_utf16().collect()));
    assert_eq!(item.map(|i| i.value.clone()), Some(vec![0xD800]));
    assert_eq!(saved.len(), 4);
}
