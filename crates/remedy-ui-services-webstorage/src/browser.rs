//! The page's own `localStorage`, for an app that runs in a browser (`wasm32`) from the same
//! origin as its web version did.

use js_sys::{Function, JsString, Reflect};
use wasm_bindgen::{JsCast, JsValue};

use crate::{LocalStorage, WebStorageError};

fn unavailable(what: &str, e: Option<JsValue>) -> WebStorageError {
    let reason = match e {
        Some(e) => format!("{what}: {e:?}"),
        None => what.to_owned(),
    };
    WebStorageError::Unavailable { reason }
}

/// A method of the storage object, called with one argument; strings come back as
/// `JsString` so their UTF-16 is read exactly (a lone surrogate included).
fn call(storage: &JsValue, method: &Function, arg: &JsValue) -> Result<Option<JsString>, WebStorageError> {
    let out = method.call1(storage, arg).map_err(|e| unavailable("localStorage refused a read", Some(e)))?;
    Ok(out.dyn_into::<JsString>().ok())
}

/// Every item of this page's `window.localStorage`, read and left as it is: the same
/// [`LocalStorage`] that [`crate::ChromiumStorage::local_storage`] gives for a web view's
/// store, so an app's first-run import is one call on each platform.
///
/// # Errors
///
/// [`WebStorageError::Unavailable`] when there is no window, the browser has storage turned
/// off, or it refuses access (a `SecurityError`).
pub fn browser_local_storage() -> Result<LocalStorage, WebStorageError> {
    let window = web_sys::window().ok_or_else(|| unavailable("no window", None))?;
    let origin = window.location().origin().map_err(|e| unavailable("the page has no origin", Some(e)))?;
    let storage = window
        .local_storage()
        .map_err(|e| unavailable("access denied", Some(e)))?
        .ok_or_else(|| unavailable("this browser has no localStorage", None))?;
    let length = storage.length().map_err(|e| unavailable("access denied", Some(e)))?;
    let storage: &JsValue = storage.as_ref();
    let method = |name: &str| -> Result<Function, WebStorageError> {
        Reflect::get(storage, &JsValue::from_str(name))
            .map_err(|e| unavailable("localStorage has no methods", Some(e)))?
            .dyn_into::<Function>()
            .map_err(|_| unavailable("localStorage has no methods", None))
    };
    let key_of = method("key")?;
    let get_item = method("getItem")?;
    let mut pairs = Vec::new();
    for index in 0..length {
        // An item removed by another tab meanwhile reads as null and is left out.
        let Some(key) = call(storage, &key_of, &JsValue::from(index))? else { continue };
        let Some(value) = call(storage, &get_item, &key)? else { continue };
        pairs.push((key.iter().collect(), value.iter().collect()));
    }
    Ok(LocalStorage::from_utf16(origin, pairs))
}
