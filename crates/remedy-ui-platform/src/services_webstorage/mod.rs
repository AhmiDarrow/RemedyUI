//! RemedyUI web storage import: the `localStorage` an app kept while it was a web app (React
//! in Tauri, Capacitor or a PWA), read losslessly so that its first run on RemedyUI starts with
//! everything its users saved.
//!
//! - [`ChromiumStorage`]: a web view's store on disk. WebView2 (Tauri on Windows), the Android
//!   System WebView (Capacitor) and Chrome keep localStorage in a LevelDB folder,
//!   `…/Local Storage/leveldb`. This reads it in pure Rust: the live tables (Snappy-compressed
//!   blocks), the write-ahead logs (writes not yet in a table, such as the last ones before
//!   the app closed) and the manifest that says which files are live, keeping the newest
//!   write of each key, deletions included. Then [`ChromiumStorage::origins`] lists whose
//!   storage is there and [`ChromiumStorage::local_storage`] decodes one origin's items.
//! - `browser_local_storage` (in a browser, on `wasm32` only): the page's own `window.localStorage`,
//!   for an app that stays on the same origin. It returns the same [`LocalStorage`], so the
//!   import is one call on each platform.
//! - Where stores are: [`tauri_windows_dir`], [`android_webview_dir`] and the origins
//!   [`TAURI_WINDOWS_ORIGINS`] and [`CAPACITOR_ORIGINS`]. These only compute paths; every
//!   reading call takes its folder explicitly, so tests and careful owners can point it
//!   anywhere (a backup copy, say).
//!
//! **The source is never changed.** Files are opened read-only, one at a time, and read
//! whole into memory; nothing is created, locked, repaired or deleted in the folder, and the
//! app that owns it may be running meanwhile. Text is never altered: items keep every
//! character, and the rare item that is not valid Unicode (a lone UTF-16 surrogate, which
//! JavaScript allows) is kept as UTF-16 in [`LocalStorage::unpaired`] rather than changed. An
//! import either has every item or fails with a [`WebStorageError`] that says why.
//!
//! ```
//! use remedy_ui_platform::services_webstorage::ChromiumStorage;
//!
//! // A store Chrome wrote, from this crate's tests (a page on http://127.0.0.1:47123).
//! let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/services_webstorage/fixtures/basic");
//! let store = ChromiumStorage::read_dir(dir)?;
//! assert_eq!(store.origins(), ["http://127.0.0.1:47123"]);
//!
//! let saved = store.local_storage("http://127.0.0.1:47123")?;
//! assert_eq!(saved.get("german"), Some("Grüße aus Köln"));
//! assert_eq!(saved.get("emoji"), Some("🎉 party ✓"));
//! assert_eq!(saved.get("counter"), Some("2")); // overwritten after the first write
//! assert_eq!(saved.get("gone"), None); // removed with removeItem
//! # Ok::<(), remedy_ui_platform::services_webstorage::WebStorageError>(())
//! ```
//!
//! An app's first run, importing what its Tauri version saved and leaving the old store as it
//! was:
//!
//! ```no_run
//! use remedy_ui_platform::services_webstorage::{
//!     ChromiumStorage, LocalStorage, TAURI_WINDOWS_ORIGINS, WebStorageError, tauri_windows_dir,
//! };
//!
//! fn saved_by_the_web_version() -> Result<Option<LocalStorage>, WebStorageError> {
//!     let Some(dir) = tauri_windows_dir("com.remedy.craftcalc") else { return Ok(None) };
//!     match ChromiumStorage::read_dir(&dir) {
//!         Ok(store) => store.first_of(&TAURI_WINDOWS_ORIGINS),
//!         // The web version never ran here: nothing to import.
//!         Err(WebStorageError::NotFound { .. }) => Ok(None),
//!         // Anything else: keep the old store and try again next start.
//!         Err(e) => Err(e),
//!     }
//! }
//! ```

mod chromium;
mod coding;
mod db;
mod error;
mod log;
mod manifest;
mod paths;
mod table;

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_arch = "wasm32")]
pub use browser::browser_local_storage;
pub use chromium::{ChromiumStorage, LocalStorage, Utf16Item};
pub use error::WebStorageError;
pub use paths::{
    CAPACITOR_ORIGINS, TAURI_WINDOWS_ORIGINS, android_webview_dir, android_webview_dir_from_files_dir,
    tauri_windows_dir, tauri_windows_dir_in,
};
