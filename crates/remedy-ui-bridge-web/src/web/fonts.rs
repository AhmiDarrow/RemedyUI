//! Files the page fetches when the app needs them (fallback fonts beside the page), and the
//! faces of the device's own interface font through the Local Font Access API.

use std::cell::RefCell;

use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

thread_local! {
    static BASE: RefCell<String> = const { RefCell::new(String::new()) };
}

pub(crate) fn set_asset_base(base: &str) {
    BASE.with(|b| *b.borrow_mut() = base.to_owned());
}

/// `path` under the asset base: relative to the page unless the base says otherwise.
pub(crate) fn asset_url(path: &str) -> String {
    BASE.with(|b| {
        let b = b.borrow();
        if b.is_empty() || b.ends_with('/') { format!("{b}{path}") } else { format!("{b}/{path}") }
    })
}

fn get(o: &JsValue, key: &str) -> Result<JsValue, String> {
    Reflect::get(o, &JsValue::from_str(key)).map_err(js_error)
}

/// Calls `o.method(args…)`.
fn call(o: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue, String> {
    let f: Function = get(o, method)?.dyn_into().map_err(|_| format!("{method} is not a function"))?;
    f.apply(o, &args.iter().collect::<Array>()).map_err(js_error)
}

async fn wait(p: JsValue) -> Result<JsValue, String> {
    let p: Promise = p.dyn_into().map_err(|_| "not a promise".to_owned())?;
    JsFuture::from(p).await.map_err(js_error)
}

/// A browser exception in words: `NotAllowedError: …`.
fn js_error(e: JsValue) -> String {
    if let Some(d) = e.dyn_ref::<js_sys::Error>() {
        format!("{}: {}", String::from(d.name()), String::from(d.message()))
    } else if let Some(s) = e.as_string() {
        s
    } else {
        format!("{e:?}")
    }
}

pub(crate) async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let global = js_sys::global();
    let response = wait(call(&global, "fetch", &[JsValue::from_str(url)])?).await?;
    if !get(&response, "ok")?.is_truthy() {
        let status = get(&response, "status")?.as_f64().unwrap_or(0.0);
        let text = get(&response, "statusText")?.as_string().unwrap_or_default();
        return Err(format!("{url}: HTTP {status} {text}").trim_end().to_owned());
    }
    let buffer = wait(call(&response, "arrayBuffer", &[])?).await?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

/// Text measured at 72 px in `font` (a CSS font shorthand), for telling faces apart.
const SAMPLES: [&str; 3] = ["Hamburgefonstiv 0123456789", "WAVE iiil Qgjy &%@", "The quick brown fox"];

fn widths(ctx: &JsValue, family: &str) -> Result<[f64; 3], String> {
    Reflect::set(ctx, &"font".into(), &JsValue::from_str(&format!("72px {family}"))).map_err(js_error)?;
    let mut out = [0.0; 3];
    for (o, s) in out.iter_mut().zip(SAMPLES) {
        *o = get(&call(ctx, "measureText", &[JsValue::from_str(s)])?, "width")?.as_f64().unwrap_or(0.0);
    }
    Ok(out)
}

fn quoted(family: &str) -> String {
    format!("\"{}\"", family.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The families the platform's interface font usually is, most likely first: what Chrome and
/// Firefox draw `system-ui` in there.
fn platform_candidates() -> Vec<&'static str> {
    let nav = get(&js_sys::global(), "navigator").unwrap_or(JsValue::UNDEFINED);
    let ua = get(&nav, "userAgent").ok().and_then(|v| v.as_string()).unwrap_or_default();
    if ua.contains("Windows") {
        vec!["Segoe UI", "Segoe UI Variable Text", "Tahoma"]
    } else if ua.contains("Android") {
        vec!["Roboto", "Google Sans", "Noto Sans"]
    } else if ua.contains("Mac OS") || ua.contains("iPhone") || ua.contains("iPad") {
        vec!["SF Pro Text", "SF Pro", "System Font", ".AppleSystemUIFont", "Helvetica Neue"]
    } else if ua.contains("CrOS") {
        vec!["Roboto", "Noto Sans"]
    } else {
        vec!["Cantarell", "Ubuntu", "Noto Sans", "DejaVu Sans", "Liberation Sans", "Inter"]
    }
}

/// Weight and width words a family's further faces add to its name in older fonts
/// (`Segoe UI Semibold`): faces of the same family.
const STYLE_WORDS: [&str; 14] = [
    "Thin",
    "Hairline",
    "ExtraLight",
    "UltraLight",
    "Light",
    "Semilight",
    "SemiLight",
    "Medium",
    "Semibold",
    "SemiBold",
    "DemiBold",
    "ExtraBold",
    "Black",
    "Heavy",
];

/// The family the browser draws `system-ui` in, among `families` (installed ones, from the
/// Local Font Access API): the one whose text measures the same, trying the platform's usual
/// ones first; else the first of those that is installed.
pub(crate) fn system_ui_family(families: &[String]) -> Result<Option<String>, String> {
    let document = get(&js_sys::global(), "document")?;
    let canvas = call(&document, "createElement", &[JsValue::from_str("canvas")])?;
    let ctx = call(&canvas, "getContext", &[JsValue::from_str("2d")])?;
    if ctx.is_null() || ctx.is_undefined() {
        return Err("the page cannot measure text (no 2D canvas)".into());
    }
    let system = widths(&ctx, "system-ui")?;
    let candidates = platform_candidates();
    let installed = |f: &str| families.iter().any(|g| g == f);
    let order = candidates
        .iter()
        .map(|s| (*s).to_owned())
        .filter(|f| installed(f))
        .chain(families.iter().filter(|f| !candidates.contains(&f.as_str())).cloned());
    for f in order {
        // Measured with two different fallbacks behind it, so a family the canvas cannot
        // use (which falls back) never passes for a match.
        let a = widths(&ctx, &format!("{}, monospace", quoted(&f)))?;
        let b = widths(&ctx, &format!("{}, serif", quoted(&f)))?;
        if a == b && a.iter().zip(system).all(|(x, y)| (x - y).abs() < 0.01) {
            return Ok(Some(f));
        }
    }
    Ok(candidates.into_iter().find(|f| installed(f)).map(str::to_owned))
}

/// The faces of the device's interface font, read through the Local Font Access API.
pub(crate) async fn system_ui_faces() -> Result<(String, Vec<Vec<u8>>), String> {
    let window = js_sys::global();
    if !Reflect::has(&window, &"queryLocalFonts".into()).unwrap_or(false) {
        return Err(
            "this browser does not let a page read the device's fonts (the Local Font Access API is in Chrome and Edge on desktops)"
                .into(),
        );
    }
    let fonts: Array = wait(call(&window, "queryLocalFonts", &[])?)
        .await?
        .dyn_into()
        .map_err(|_| "no font list".to_owned())?;
    let fonts: Vec<JsValue> = fonts.iter().collect();
    let family_of = |f: &JsValue| get(f, "family").ok().and_then(|v| v.as_string()).unwrap_or_default();
    let mut families: Vec<String> = fonts.iter().map(family_of).filter(|f| !f.is_empty()).collect();
    families.sort();
    families.dedup();
    if families.is_empty() {
        return Err("the device's fonts were not shared with the page (permission not granted)".into());
    }
    let Some(chosen) = system_ui_family(&families)? else {
        return Err("none of the installed fonts is the one this browser uses for system-ui".into());
    };
    let same = |f: &str| {
        f == chosen
            || f.strip_prefix(chosen.as_str())
                .and_then(|rest| rest.strip_prefix(' '))
                .is_some_and(|rest| rest.split(' ').all(|w| STYLE_WORDS.contains(&w)))
    };
    let mut faces = Vec::new();
    for f in fonts.iter().filter(|f| same(&family_of(f))) {
        let blob = wait(call(f, "blob", &[])?).await?;
        let buffer = wait(call(&blob, "arrayBuffer", &[])?).await?;
        faces.push(Uint8Array::new(&buffer).to_vec());
    }
    Ok((chosen, faces))
}
