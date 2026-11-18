//! File pickers, downloads, and files dropped on a window.

use std::path::PathBuf;

use remedy_ui_host::{DialogId, DialogStarted, FileDialog, HostEvent, Unsupported};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::dom::{self, Listener};
use super::{Driver, post};
use crate::{convert, files};

/// Reads every file into the store under one new batch folder; returns the paths, and for
/// folder picks the chosen top folders instead.
async fn read_files(list: Vec<web_sys::File>, folders: bool) -> Vec<PathBuf> {
    let batch = files::new_batch("opened");
    let mut paths = Vec::new();
    let mut tops: Vec<PathBuf> = Vec::new();
    for f in list {
        let relative = dom::get(&f, "webkitRelativePath")
            .as_string()
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| f.name());
        let Ok(buf) = JsFuture::from(f.array_buffer()).await else { continue };
        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
        let path = files::insert(&batch, &relative, bytes);
        if folders {
            if let Some(top) = path.strip_prefix(&batch).ok().and_then(|r| r.components().next()) {
                let top = batch.join(top.as_os_str());
                if !tops.contains(&top) {
                    tops.push(top);
                }
            }
        } else {
            paths.push(path);
        }
    }
    if folders { tops } else { paths }
}

fn file_list(list: Option<web_sys::FileList>) -> Vec<web_sys::File> {
    let Some(list) = list else { return Vec::new() };
    (0..list.length()).filter_map(|i| list.get(i)).collect()
}

/// Opens a picker, or answers a save at once with the path its download will be named after.
pub(crate) fn file_dialog(d: &Driver, dialog: &FileDialog) -> Result<DialogStarted, Unsupported> {
    if dialog.save {
        if dialog.folders {
            return Err(Unsupported::new(
                "saving into a folder",
                "a page can only offer downloads, one file at a time",
            ));
        }
        let name = dialog.file_name.clone().unwrap_or_else(|| "download".to_owned());
        let batch = files::new_batch("save");
        let path = batch.join(files::download_name(std::path::Path::new(&name)));
        return Ok(DialogStarted::Finished(vec![path]));
    }
    if !super::input::user_active() {
        return Err(Unsupported::new(
            "file dialogs",
            "a browser opens its file picker only in answer to a click or key press",
        ));
    }
    let input: web_sys::HtmlInputElement = dom::element("input")
        .ok_or_else(|| Unsupported::new("file dialogs", "the page refused to make a file input"))?;
    input.set_type("file");
    input.set_multiple(dialog.multiple);
    let accept = convert::accept_list(&dialog.filters);
    if !accept.is_empty() && !dialog.folders {
        input.set_accept(&accept);
    }
    if dialog.folders {
        let _ = input.set_attribute("webkitdirectory", "");
    }
    dom::style(&input, &[("display", "none")]);
    let body = dom::document()
        .and_then(|d| d.body())
        .ok_or_else(|| Unsupported::new("file dialogs", "the page has no body"))?;
    let _ = body.append_child(&input);
    let id = {
        let mut s = d.state.borrow_mut();
        let id = s.next_dialog;
        s.next_dialog += 1;
        DialogId(id)
    };
    let folders = dialog.folders;
    // The listeners live until the picker answers, then go with the input.
    let holder: std::rc::Rc<std::cell::RefCell<Vec<Listener>>> = Default::default();
    let (h1, h2) = (holder.clone(), holder.clone());
    let (i1, i2) = (input.clone(), input.clone());
    let change = Listener::new(&input, "change", move |_| {
        let chosen = file_list(i1.files());
        i1.remove();
        h1.borrow_mut().clear();
        wasm_bindgen_futures::spawn_local(async move {
            let paths = read_files(chosen, folders).await;
            post(HostEvent::FileDialog(id, paths));
        });
    });
    let cancel = Listener::new(&input, "cancel", move |_| {
        i2.remove();
        h2.borrow_mut().clear();
        post(HostEvent::FileDialog(id, Vec::new()));
    });
    holder.borrow_mut().extend([change, cancel]);
    input.click();
    Ok(DialogStarted::Pending(id))
}

/// Reads files dropped on a window, then reports the drop.
pub(crate) fn dropped(
    window: remedy_ui_host::WindowId,
    position: Option<remedy_ui_core::Point>,
    list: Vec<web_sys::File>,
    text: Option<String>,
    uris: Vec<String>,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let files = read_files(list, false).await;
        let data = remedy_ui_host::DragData { files, text, uris };
        post(HostEvent::Drag(window, remedy_ui_host::DragEvent::Drop { position, data }));
    });
}

/// Hands `bytes` to the browser as a download named `name`.
pub(crate) fn download(name: &str, bytes: &[u8]) -> Result<(), Unsupported> {
    let fail = |e: JsValue| Unsupported::new("downloads", format!("{e:?}"));
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&array);
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(fail)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(fail)?;
    let a: web_sys::HtmlAnchorElement =
        dom::element("a").ok_or_else(|| Unsupported::new("downloads", "the page refused to make a link"))?;
    a.set_href(&url);
    a.set_download(name);
    dom::style(&a, &[("display", "none")]);
    let body = dom::document()
        .and_then(|d| d.body())
        .ok_or_else(|| Unsupported::new("downloads", "the page has no body"))?;
    let _ = body.append_child(&a);
    a.click();
    a.remove();
    // The browser has taken the download; the object URL is let go after this task.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    if let Some(w) = web_sys::window() {
        let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 0);
    }
    Ok(())
}
