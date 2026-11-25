//! Printing: the page goes into a hidden frame, and the browser's print dialog prints it.

use remedy_ui_core::host::Unsupported;
use wasm_bindgen::JsCast;

/// Marks the frames this module adds, so the next print removes the last one.
const MARK: &str = "data-remedy-print";

/// Loads `html` into a hidden frame and opens the browser's print dialog for it.
pub(crate) fn print_html(html: &str) -> Result<(), Unsupported> {
    let err = |e: wasm_bindgen::JsValue| Unsupported::new("printing", format!("{e:?}"));
    let window = web_sys::window().ok_or_else(|| Unsupported::new("printing", "no window"))?;
    let document = window.document().ok_or_else(|| Unsupported::new("printing", "no document"))?;
    if let Ok(old) = document.query_selector_all(&format!("iframe[{MARK}]")) {
        for i in 0..old.length() {
            if let Some(n) = old.item(i)
                && let Some(p) = n.parent_node()
            {
                let _ = p.remove_child(&n);
            }
        }
    }
    let frame = document.create_element("iframe").map_err(err)?;
    frame.set_attribute(MARK, "").map_err(err)?;
    frame.set_attribute("style", "position:fixed;width:0;height:0;border:0;right:0;bottom:0").map_err(err)?;
    frame.set_attribute("srcdoc", html).map_err(err)?;
    let body = document.body().ok_or_else(|| Unsupported::new("printing", "no body"))?;
    body.append_child(&frame).map_err(err)?;
    let frame: web_sys::HtmlIFrameElement =
        frame.dyn_into().map_err(|_| Unsupported::new("printing", "not a frame"))?;
    let f2 = frame.clone();
    let onload = wasm_bindgen::closure::Closure::once_into_js(move || {
        if let Some(w) = f2.content_window() {
            let _ = w.print();
        }
    });
    frame.set_onload(Some(onload.unchecked_ref()));
    Ok(())
}
