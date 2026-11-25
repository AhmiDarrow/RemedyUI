//! Files in a browser page.
//!
//! A page has no file system an app can open by path. Files the user picks in an open dialog
//! or drops on a window are read into this store and handed to the app as paths under
//! [`ROOT`], as the host contract's dialogs and drops do everywhere; the app reads them with
//! [`read`] (and lists a chosen folder with [`list`]). A save dialog answers with a path
//! under [`ROOT`] at once (the browser, not the page, chooses where downloads go), and
//! [`save`] hands the bytes to the browser as a download of that name.
//!
//! The store keeps files until the app lets go of them with [`remove`], or the page closes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// Where the page's files appear: picked and dropped files under `opened`, files to save under
/// `save`.
pub const ROOT: &str = "/remedy-web";

#[derive(Default)]
struct Store {
    files: BTreeMap<PathBuf, Arc<[u8]>>,
    next: u64,
}

static STORE: Mutex<Store> = Mutex::new(Store { files: BTreeMap::new(), next: 0 });

fn store() -> std::sync::MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A file name as one safe path component: separators and `..` cannot climb out of the
/// file's folder.
fn component(name: &str) -> String {
    let clean: String =
        name.chars().map(|c| if matches!(c, '/' | '\\') || c.is_control() { '_' } else { c }).collect();
    match clean.trim() {
        "" | "." | ".." => "file".to_owned(),
        t => t.to_owned(),
    }
}

/// A new folder for one batch of files (one dialog's choice, one drop, or files the app fetched
/// itself and hands to code that reads paths): `<ROOT>/<kind>/<n>`.
pub fn new_batch(kind: &str) -> PathBuf {
    let mut s = store();
    s.next += 1;
    Path::new(ROOT).join(kind).join(s.next.to_string())
}

/// Puts `bytes` in the store at `relative` (a file's name, or its path inside a chosen
/// folder) under `batch`, and returns the path the app sees.
pub fn insert(batch: &Path, relative: &str, bytes: Vec<u8>) -> PathBuf {
    let mut path = batch.to_path_buf();
    for part in relative.split(['/', '\\']).filter(|p| !p.is_empty()) {
        path.push(component(part));
    }
    if path == batch {
        path.push("file");
    }
    store().files.insert(path.clone(), bytes.into());
    path
}

/// The bytes of a file the user picked or dropped, if `path` names one still held.
pub fn read(path: &Path) -> Option<Vec<u8>> {
    store().files.get(path).map(|b| b.to_vec())
}

/// The files held under `dir` (a chosen folder, or a drop's folder), at any depth, in order.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    store().files.keys().filter(|p| p.starts_with(dir) && p.as_path() != dir).cloned().collect()
}

/// Lets go of a file, or of every file under a folder.
pub fn remove(path: &Path) {
    store().files.retain(|p, _| !p.starts_with(path));
}

/// The name a save path downloads as: its last component.
pub fn download_name(path: &Path) -> String {
    path.file_name().map_or_else(|| "download".to_owned(), |n| component(&n.to_string_lossy()))
}

/// Hands `bytes` to the browser as a download named after `path`'s file name (a path a save
/// dialog answered with). The browser shows it in its downloads and, as the user set it,
/// asks where to put it or saves it to the downloads folder.
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), remedy_ui_core::host::Unsupported> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        crate::web::download(&download_name(path), bytes)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = (path, bytes);
        Err(remedy_ui_core::host::Unsupported::new("downloads", "downloads exist only in a browser page"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picked_files_are_read_by_path_and_cannot_climb_out() {
        let batch = new_batch("opened");
        let a = insert(&batch, "notes.txt", b"hello".to_vec());
        let b = insert(&batch, "photos/../../etc/passwd", b"x".to_vec());
        assert!(a.starts_with(ROOT));
        assert_eq!(read(&a).as_deref(), Some(&b"hello"[..]));
        assert!(b.starts_with(&batch), "`..` is a name here, not a way out: {b:?}");
        assert_eq!(list(&batch), {
            let mut v = vec![a.clone(), b.clone()];
            v.sort();
            v
        });
        remove(&batch);
        assert_eq!(read(&a), None);
        assert!(list(&batch).is_empty());
        assert_eq!(download_name(Path::new("/remedy-web/save/1/report.pdf")), "report.pdf");
    }
}
