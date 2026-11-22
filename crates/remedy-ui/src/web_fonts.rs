//! Faces a browser page fetches the first time text needs them: the fallback faces that are
//! not built into the module (`remedy_ui_fonts::Face::is_remote`), from beside the page.

use std::cell::RefCell;
use std::collections::HashMap;

use remedy_ui_fonts::Face;
use remedy_ui_text::FontFile;

#[derive(Clone)]
enum Slot {
    Fetching,
    Ready(FontFile),
    Failed,
}

type Arrived = Box<dyn Fn()>;

thread_local! {
    static FILES: RefCell<HashMap<&'static str, Slot>> = RefCell::new(HashMap::new());
    /// What runs when a face arrives: every window shapes its text again.
    static ARRIVED: RefCell<Option<Arrived>> = const { RefCell::new(None) };
}

/// Sets what runs when a fetched face arrives (the runner: every window's text is shaped
/// again, so what waited for the face draws with it).
pub(crate) fn on_arrival(f: impl Fn() + 'static) {
    ARRIVED.with(|a| *a.borrow_mut() = Some(Box::new(f)));
}

/// The faces of `faces` that are here: built in, or fetched. Starts fetching the others and
/// returns none until each of them has arrived or failed (a failed one is left out), so the
/// text that asked is shaped again with the whole set.
pub(crate) fn faces(faces: &[&'static Face]) -> Vec<FontFile> {
    let mut out = Vec::new();
    let mut waiting = false;
    for face in faces {
        if !face.is_remote() {
            out.extend(FontFile::new(face.data).ok());
            continue;
        }
        match FILES.with(|f| f.borrow().get(face.file).cloned()) {
            Some(Slot::Ready(ff)) => out.push(ff),
            Some(Slot::Fetching) => waiting = true,
            Some(Slot::Failed) => {}
            None => {
                waiting = true;
                fetch(face.file);
            }
        }
    }
    if waiting { Vec::new() } else { out }
}

fn fetch(file: &'static str) {
    FILES.with(|f| f.borrow_mut().insert(file, Slot::Fetching));
    let url = remedy_ui_bridge_web::fonts::asset_url(&format!("fonts/{file}"));
    remedy_ui_bridge_web::fonts::fetch_bytes(&url, move |got| {
        let slot = match got.and_then(|b| FontFile::from_vec(b).map_err(|e| e.to_string())) {
            Ok(ff) => Slot::Ready(ff),
            Err(e) => {
                remedy_ui_trace::tracing::warn!("the font {file} could not be loaded: {e}");
                Slot::Failed
            }
        };
        FILES.with(|f| f.borrow_mut().insert(file, slot));
        ARRIVED.with(|a| {
            if let Some(a) = a.borrow().as_ref() {
                a();
            }
        });
    });
}
