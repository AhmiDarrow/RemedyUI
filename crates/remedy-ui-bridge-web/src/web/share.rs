//! The browser's share sheet: `navigator.share`, with the files as `File` objects where
//! `navigator.canShare` takes them.

use remedy_ui_core::host::{HostEvent, ShareId, ShareOptions, ShareOutcome, ShareStarted, Unsupported};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::{Driver, dom, input, post};
use crate::share::{Browser, Payload, payload};

fn refuse(reason: impl Into<String>) -> Unsupported {
    Unsupported::new("sharing", reason)
}

fn set(obj: &js_sys::Object, key: &str, value: &JsValue) {
    let _ = js_sys::Reflect::set(obj, &JsValue::from_str(key), value);
}

/// Calls `navigator[name](data)` where it is a function.
fn call(nav: &JsValue, name: &str, data: &JsValue) -> Option<JsValue> {
    let f = dom::get(nav, name).dyn_into::<js_sys::Function>().ok()?;
    f.call1(nav, data).ok()
}

/// The share's files as the browser's `File` objects.
fn files(share: &ShareOptions) -> Result<js_sys::Array, Unsupported> {
    let list = js_sys::Array::new();
    for f in &share.files {
        let bytes = js_sys::Uint8Array::from(f.bytes.as_slice());
        let parts = js_sys::Array::of1(&bytes);
        let opts = web_sys::FilePropertyBag::new();
        opts.set_type(f.mime_type());
        let file = web_sys::File::new_with_u8_array_sequence_and_options(&parts, &f.name, &opts)
            .map_err(|e| refuse(format!("the browser refused to make the file {}: {e:?}", f.name)))?;
        list.push(&file);
    }
    Ok(list)
}

/// Opens the share sheet; the outcome comes back as [`HostEvent::Share`].
pub(crate) fn share(d: &Driver, share: &ShareOptions) -> Result<ShareStarted, Unsupported> {
    let nav: JsValue =
        web_sys::window().ok_or_else(|| refuse("there is no browser window"))?.navigator().into();
    let text = js_sys::Object::new();
    if !share.title.is_empty() {
        set(&text, "title", &JsValue::from_str(&share.title));
    }
    if !share.text.is_empty() {
        set(&text, "text", &JsValue::from_str(&share.text));
    }
    let with_files = if share.files.is_empty() {
        None
    } else {
        let all = js_sys::Object::assign(&js_sys::Object::new(), &text);
        set(&all, "files", files(share)?.as_ref());
        Some(all)
    };
    let browser = Browser {
        share: dom::get(&nav, "share").is_function(),
        active: input::user_active(),
        files: with_files
            .as_ref()
            .and_then(|all| call(&nav, "canShare", all))
            .and_then(|ok| ok.as_bool())
            .unwrap_or(false),
    };
    let data: JsValue = match payload(share, browser)? {
        Payload::WithFiles => with_files.map_or_else(|| text.into(), Into::into),
        Payload::TextOnly => text.into(),
    };
    // A share the browser would refuse is refused here, with its reason, before any sheet.
    if call(&nav, "canShare", &data).and_then(|ok| ok.as_bool()) == Some(false) {
        return Err(refuse("the browser refuses to share this"));
    }
    let promise = call(&nav, "share", &data)
        .and_then(|p| p.dyn_into::<js_sys::Promise>().ok())
        .ok_or_else(|| refuse("navigator.share did not start"))?;
    let id = {
        let mut s = d.state.borrow_mut();
        let id = s.next_share;
        s.next_share += 1;
        ShareId(id)
    };
    wasm_bindgen_futures::spawn_local(async move {
        let outcome = match JsFuture::from(promise).await {
            Ok(_) => ShareOutcome::Shared,
            Err(e) => {
                let name = dom::get(&e, "name").as_string().unwrap_or_default();
                let message = dom::get(&e, "message").as_string().unwrap_or_else(|| format!("{e:?}"));
                if name == "AbortError" {
                    ShareOutcome::Cancelled
                } else {
                    ShareOutcome::Unsupported(refuse(format!(
                        "the browser did not share it ({name}: {message})"
                    )))
                }
            }
        };
        post(HostEvent::Share(id, outcome));
    });
    Ok(ShareStarted::Pending(id))
}
