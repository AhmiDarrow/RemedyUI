//! The clipboard in a page.
//!
//! A page may write and read the clipboard without asking only inside the browser's `copy`,
//! `cut` and `paste` events. Clipboard shortcuts therefore reach the browser: the app handles
//! the key first, then the event carries what it copied out, or brings the pasted text in
//! before the app's paste runs. Copies made outside those events go through the async
//! Clipboard API, which the browser completes after the call. Reading outside a paste needs
//! the user's permission; without it the page reads only what it last saw (its own copies
//! and pastes) while it keeps focus, since the user may copy elsewhere once it loses focus.

use std::rc::Rc;

use remedy_ui_core::host::{HostEvent, Unsupported};
use remedy_ui_core::input::{Key, KeyEvent, Modifiers, Platform};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use super::dom::Listener;
use super::{Driver, driver};

/// What the page knows of the clipboard.
#[derive(Default)]
pub(crate) struct Clip {
    /// What the app copied, for the `copy` or `cut` event under way: text and HTML.
    pending: Option<(String, Option<String>)>,
    /// What the page last saw on the clipboard (text and HTML), while it is still known to be
    /// there.
    seen: Option<(String, Option<String>)>,
    /// The user let the page read the clipboard (`clipboard-read` is granted).
    readable: bool,
}

/// A key event for the platform's clipboard shortcut `letter` (c, x or v).
pub(crate) fn shortcut(letter: &str, platform: Platform, pressed: bool) -> KeyEvent {
    KeyEvent::press(Key::Character(letter.to_owned()))
        .with_physical(format!("Key{}", letter.to_uppercase()))
        .with_modifiers(Modifiers::primary(platform))
        .with_pressed(pressed)
        .with_text(None)
}

/// Forgets what an earlier copy left for a `copy` or `cut` event that never came (a copy
/// made from a button): the event about to come carries only what the app copies for it.
pub(crate) fn forget_pending(d: &Driver) {
    d.state.borrow_mut().clip.pending = None;
}

fn focused_window(d: &Driver) -> Option<Rc<super::window::Win>> {
    let active = super::dom::active_element()?;
    d.windows().into_iter().find(|w| w.frame.contains(Some(&active)))
}

/// Listens for the clipboard events and focus, which decide what the page may know.
pub(crate) fn install(d: &Rc<Driver>, win: &web_sys::Window, doc: &web_sys::Document) {
    for kind in ["copy", "cut"] {
        d.add_global(Listener::new(doc, kind, move |e| {
            let Some(d) = driver() else { return };
            let Some(w) = focused_window(&d) else { return };
            let Ok(ce) = e.dyn_into::<web_sys::ClipboardEvent>() else { return };
            if !w.clipboard_key.replace(false) {
                // From the browser's menu, not a shortcut: ask the app as the shortcut would.
                let platform = d.state.borrow().settings.platform;
                let letter = if kind == "copy" { "c" } else { "x" };
                forget_pending(&d);
                d.post(HostEvent::Key(w.id, shortcut(letter, platform, true)));
                d.post(HostEvent::Key(w.id, shortcut(letter, platform, false)));
            }
            let pending = d.state.borrow_mut().clip.pending.take();
            if let (Some((text, html)), Some(data)) = (pending, ce.clipboard_data()) {
                let _ = data.set_data("text/plain", &text);
                if let Some(h) = html {
                    let _ = data.set_data("text/html", &h);
                }
                ce.prevent_default();
            }
        }));
    }
    d.add_global(Listener::new(doc, "paste", move |e| {
        let Some(d) = driver() else { return };
        let Some(w) = focused_window(&d) else { return };
        let Ok(ce) = e.dyn_into::<web_sys::ClipboardEvent>() else { return };
        let Some(data) = ce.clipboard_data() else { return };
        ce.prevent_default();
        let text = data.get_data("text/plain").unwrap_or_default();
        let html = data.get_data("text/html").ok().filter(|h| !h.is_empty());
        d.state.borrow_mut().clip.seen = Some((text, html));
        let key = w.deferred_paste.borrow_mut().take();
        let platform = d.state.borrow().settings.platform;
        match key {
            // The shortcut's own key; its release follows from the keyboard.
            Some(press) => d.post(HostEvent::Key(w.id, press)),
            // The browser's Paste (a menu, a soft keyboard's button): as the shortcut would.
            None => {
                d.post(HostEvent::Key(w.id, shortcut("v", platform, true)));
                d.post(HostEvent::Key(w.id, shortcut("v", platform, false)));
            }
        }
    }));
    d.add_global(Listener::new(win, "blur", |_| {
        if let Some(d) = driver() {
            let mut s = d.state.borrow_mut();
            if !s.clip.readable {
                // The user may copy something else now; what the page saw is no longer known.
                s.clip.seen = None;
            }
        }
    }));
    d.add_global(Listener::new(win, "focus", |_| refresh()));
    check_permission(win);
}

/// Asks the browser whether the user lets the page read the clipboard (without prompting).
fn check_permission(win: &web_sys::Window) {
    let Ok(perms) = win.navigator().permissions() else { return };
    let desc = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&desc, &"name".into(), &"clipboard-read".into());
    let Ok(query) = perms.query(&desc) else { return };
    wasm_bindgen_futures::spawn_local(async move {
        // Browsers that do not know the permission reject the query: not readable.
        let Ok(status) = wasm_bindgen_futures::JsFuture::from(query).await else { return };
        let Ok(status) = status.dyn_into::<web_sys::PermissionStatus>() else { return };
        let granted = status.state() == web_sys::PermissionState::Granted;
        if let Some(d) = driver() {
            d.state.borrow_mut().clip.readable = granted;
        }
        if granted {
            refresh();
        }
    });
}

/// Reads the clipboard again where the user allowed it.
fn refresh() {
    let Some(d) = driver() else { return };
    if !d.state.borrow().clip.readable {
        return;
    }
    let Ok(win) = super::web_window() else { return };
    let read = win.navigator().clipboard().read_text();
    wasm_bindgen_futures::spawn_local(async move {
        if let Ok(t) = wasm_bindgen_futures::JsFuture::from(read).await
            && let (Some(text), Some(d)) = (t.as_string(), driver())
        {
            d.state.borrow_mut().clip.seen = Some((text, None));
        }
    });
}

fn write_async(text: &str, html: Option<&str>) {
    let Ok(win) = super::web_window() else { return };
    let clipboard = win.navigator().clipboard();
    let promise = match html {
        Some(h) => {
            let items = js_sys::Object::new();
            let blob = |data: &str, mime: &str| {
                let parts = js_sys::Array::of1(&JsValue::from_str(data));
                let opts = web_sys::BlobPropertyBag::new();
                opts.set_type(mime);
                web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).ok()
            };
            if let (Some(t), Some(hb)) = (blob(text, "text/plain"), blob(h, "text/html")) {
                let _ = js_sys::Reflect::set(&items, &"text/plain".into(), &t);
                let _ = js_sys::Reflect::set(&items, &"text/html".into(), &hb);
            }
            match web_sys::ClipboardItem::new_with_record_from_str_to_blob_promise(&items) {
                Ok(item) => clipboard.write(&js_sys::Array::of1(&item)),
                Err(_) => clipboard.write_text(text),
            }
        }
        None => clipboard.write_text(text),
    };
    // The browser completes the write later; a page without focus is refused and the
    // clipboard keeps what it held.
    wasm_bindgen_futures::spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    });
}

pub(crate) fn set_text(d: &Driver, text: &str, sensitive: bool) {
    {
        let mut s = d.state.borrow_mut();
        s.clip.pending = Some((text.to_owned(), None));
        // A secret is remembered only so its time-out can tell whether it is still there; it
        // is forgotten as soon as the page loses focus.
        s.clip.seen = Some((text.to_owned(), None));
        let _ = sensitive;
    }
    write_async(text, None);
}

pub(crate) fn set_html(d: &Driver, text: &str, html: &str) {
    {
        let mut s = d.state.borrow_mut();
        s.clip.pending = Some((text.to_owned(), Some(html.to_owned())));
        s.clip.seen = Some((text.to_owned(), Some(html.to_owned())));
    }
    write_async(text, Some(html));
}

pub(crate) fn clear(d: &Driver) {
    {
        let mut s = d.state.borrow_mut();
        s.clip.pending = None;
        s.clip.seen = None;
    }
    write_async("", None);
}

fn unreadable() -> Unsupported {
    Unsupported::new(
        "reading the clipboard",
        "a browser lets a page read the clipboard only during a paste (Ctrl+V or the browser's Paste) or with the user's clipboard permission",
    )
}

pub(crate) fn text(d: &Driver) -> Result<Option<String>, Unsupported> {
    d.state.borrow().clip.seen.as_ref().map(|(t, _)| Some(t.clone())).ok_or_else(unreadable)
}

pub(crate) fn html(d: &Driver) -> Result<Option<String>, Unsupported> {
    d.state.borrow().clip.seen.as_ref().map(|(_, h)| h.clone()).ok_or_else(unreadable)
}
