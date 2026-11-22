//! Persisted signals: state that survives a restart (window splits, scroll anchors, drafts,
//! choices people made), saved to the app's data folder and restored when made.
//!
//! A [`PersistStore`] is a folder ([`PersistStore::open`]), or in a browser page the page's
//! `localStorage` (`PersistStore::local_storage`, which a page has instead of files). A
//! signal marked with [`Signal::persist`] (or [`Signal::persist_in`]) is restored from its
//! entry at once and saved after it changes. Saves are debounced (many changes in a short
//! time make one write) and written off the UI thread (in a page, between frames),
//! atomically: a file is written to a temporary file that is synced and then renamed over the
//! old one, and a `localStorage` entry is replaced whole, so a crash leaves the old value or
//! the new one, never half of each. Each entry holds a version and the value as JSON; an app
//! that changes a value's shape raises the version and gives a migration from older versions.
//!
//! The same store keeps an app's own documents: [`PersistStore::save`],
//! [`PersistStore::load`] and [`PersistStore::remove`] take any serializable value under a
//! key, with the same versions and migrations, on every platform.
//!
//! ```
//! use remedy_ui_reactive::{Persist, PersistStore, Scope, Signal};
//!
//! let dir = std::env::temp_dir().join(format!("remedy-persist-doc-{}", std::process::id()));
//! let store = PersistStore::open(&dir)?;
//! let ((), scope) = Scope::new(|| {
//!     let split = Signal::new(0.3f32).persist_in(&store, Persist::new("split")).unwrap_or_else(|(s, _)| s);
//!     split.set(0.6);
//! });
//! remedy_ui_reactive::batch(|| {}); // effects have run
//! store.flush()?;
//! scope.dispose();
//! // Next start: the value comes back.
//! let ((), _again) = Scope::new(|| {
//!     let split = Signal::new(0.3f32).persist_in(&store, Persist::new("split")).unwrap_or_else(|(s, _)| s);
//!     assert_eq!(split.get(), 0.6);
//! });
//! # let _ = std::fs::remove_dir_all(&dir);
//! # Ok::<(), std::io::Error>(())
//! ```

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use remedy_ui_core::exec::Next;
use remedy_ui_core::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::context::use_context;
use crate::signal::{Effect, Signal, untrack};

/// The JSON value type migrations work on.
pub use serde_json::Value as JsonValue;

/// How long a store waits after a change before writing, by default: changes that follow
/// within it (a split being dragged, text being typed) are written together.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(300);

/// Starts a store's slot for a plain value ([`Persist::plain`]), whose key is the exact
/// storage key with no app prefix.
const EXACT: char = '\0';

/// A slot's key, and whether it is exact (a plain value's).
fn split_slot(slot: &str) -> (&str, bool) {
    match slot.strip_prefix(EXACT) {
        Some(k) => (k, true),
        None => (slot, false),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a value could not be restored or saved.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PersistError {
    /// The key is empty.
    EmptyKey,
    /// Reading or writing the file failed.
    Io(String),
    /// The file is not a saved value (or not one of this type).
    Format(String),
    /// The file was written by a newer version of the app; it is left alone.
    Newer {
        /// The file's version.
        found: u32,
        /// The version this app writes.
        current: u32,
    },
    /// The migration from an older version refused the value.
    Migration(MigrationError),
    /// No store is provided as context ([`Signal::persist`]).
    NoStore,
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::EmptyKey => f.write_str("a persisted value needs a key"),
            PersistError::Io(e) => write!(f, "the saved value could not be read or written: {e}"),
            PersistError::Format(e) => write!(f, "the saved value is damaged or of another type: {e}"),
            PersistError::Newer { found, current } => write!(
                f,
                "the saved value is from a newer version of the app (version {found}; this one writes {current}), so it is left alone"
            ),
            PersistError::Migration(e) => write!(f, "the saved value could not be brought up to date: {e}"),
            PersistError::NoStore => f.write_str("no persist store is provided here; the value is not saved"),
        }
    }
}

impl std::error::Error for PersistError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PersistError::Migration(e) => Some(e),
            _ => None,
        }
    }
}

/// Why a migration could not bring a saved value up to date. Made from a reason
/// (`MigrationError::new`, or `From` a `String` or `&str`, so a migration can use `?` on its
/// own messages).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationError(String);

impl MigrationError {
    /// A migration failure for `reason`.
    pub fn new(reason: impl Into<String>) -> Self {
        MigrationError(reason.into())
    }

    /// Why, in words.
    pub fn reason(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MigrationError {}

impl From<String> for MigrationError {
    fn from(s: String) -> Self {
        MigrationError(s)
    }
}

impl From<&str> for MigrationError {
    fn from(s: &str) -> Self {
        MigrationError(s.to_owned())
    }
}

/// Brings a saved value from an older version (the first argument) to the current shape.
pub type Migrate = Rc<dyn Fn(u32, JsonValue) -> Result<JsonValue, MigrationError>>;

/// How a signal is persisted: under which key, at which version, and how older versions are
/// brought up to date.
#[derive(Clone)]
pub struct Persist {
    key: String,
    version: u32,
    migrate: Option<Migrate>,
    plain: Option<Plain>,
    session: bool,
}

/// A value kept alone, with no version around it, under exactly its key: for storage other
/// code shares and reads as it is, such as the items an older web page of the app keeps in
/// the origin's `localStorage` ([`Persist::plain`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Plain {
    /// The value as JSON, as `JSON.stringify` writes it: `{"text":"remedy-sans"}`.
    Json,
    /// A value that is a string, as the string itself with no quotes:
    /// `localStorage.setItem("theme", "dark")`. A value that is not a string is not saved
    /// ([`PersistError::Format`]).
    Text,
}

impl std::fmt::Debug for Persist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Persist")
            .field("key", &self.key)
            .field("version", &self.version)
            .field("plain", &self.plain)
            .field("session", &self.session)
            .finish()
    }
}

impl Persist {
    /// Persisted under `key`, at version 1.
    pub fn new(key: impl Into<String>) -> Self {
        Persist { key: key.into(), version: 1, migrate: None, plain: None, session: false }
    }

    /// Keeps the value alone, as `plain` says, under exactly the key: in a page store
    /// (`PersistStore::local_storage`), the page's own `localStorage` item of that name,
    /// outside the app's `remedy:<app>:` prefix, so another page of the origin (the app's
    /// older web version) reads and writes the same item; in a folder store, the key's file,
    /// holding just the value. Plain values have no version: [`Persist::version`] and
    /// [`Persist::migrate`] do not apply, so read a value another program writes leniently
    /// (serde defaults), as that program would.
    ///
    /// ```
    /// use remedy_ui_reactive::{Persist, Plain};
    ///
    /// // What `localStorage.setItem("portal-theme", "midnight")` wrote, and
    /// // `localStorage.setItem("portal-fonts", JSON.stringify(fonts))`.
    /// let theme = Persist::new("portal-theme").plain(Plain::Text);
    /// let fonts = Persist::new("portal-fonts").plain(Plain::Json);
    /// assert_eq!((theme.plain_as(), fonts.plain_as()), (Some(Plain::Text), Some(Plain::Json)));
    /// ```
    pub fn plain(mut self, plain: Plain) -> Self {
        self.plain = Some(plain);
        self
    }

    /// Keeps the value only for this session, plainly (as [`Persist::plain`], JSON unless
    /// `plain` said otherwise) under exactly the key: in a browser page, the tab's own
    /// `sessionStorage` item of that name, which other pages of the origin in the same tab
    /// share and which ends with the tab; elsewhere, memory shared by every store for the
    /// app's run, which ends when the app does. Saved and removed at once, with no debounce
    /// (a tab may close at any moment, and nothing is written to disk).
    ///
    /// ```
    /// use remedy_ui_reactive::{Persist, Plain};
    ///
    /// // What `sessionStorage.setItem("portal-seasons", JSON.stringify(memory))` wrote.
    /// let seasons = Persist::new("portal-seasons").session();
    /// assert!(seasons.is_session() && seasons.plain_as() == Some(Plain::Json));
    /// ```
    pub fn session(mut self) -> Self {
        self.session = true;
        self.plain.get_or_insert(Plain::Json);
        self
    }

    /// Whether the value is kept only for this session ([`Persist::session`]).
    pub fn is_session(&self) -> bool {
        self.session
    }

    /// How the value is kept alone ([`Persist::plain`]), or `None` for a versioned value.
    pub fn plain_as(&self) -> Option<Plain> {
        self.plain
    }

    /// The slot the store keeps the value in: plain values apart from versioned ones, since
    /// in a page they are different `localStorage` items.
    fn slot(&self) -> String {
        if self.plain.is_some() { format!("{EXACT}{}", self.key) } else { self.key.clone() }
    }

    /// The version this app writes (raise it when the value's shape changes).
    pub fn version(mut self, v: u32) -> Self {
        self.version = v;
        self
    }

    /// How a value saved by an older version becomes the current shape: `f(old_version,
    /// value)` returns the value as the current version reads it, or why it cannot.
    pub fn migrate(
        mut self,
        f: impl Fn(u32, JsonValue) -> Result<JsonValue, MigrationError> + 'static,
    ) -> Self {
        self.migrate = Some(Rc::new(f));
        self
    }

    /// The key.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl From<&str> for Persist {
    fn from(k: &str) -> Self {
        Persist::new(k)
    }
}

impl From<String> for Persist {
    fn from(k: String) -> Self {
        Persist::new(k)
    }
}

/// Where a store keeps its entries: files in a folder, or a key-value store of strings (a
/// browser page's `localStorage`). Loads read it on the UI thread; the writer writes it.
trait Backend: Send + Sync {
    /// The bytes saved under `key`, or `None` when nothing is.
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    /// Saves `bytes` under `key`, replacing what was there whole.
    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String>;
    /// Forgets `key`; nothing saved under it is not an error.
    fn remove(&self, key: &str) -> Result<(), String>;
    /// Where `key` lives, for messages: its file name or storage key.
    fn name_of(&self, key: &str) -> String;
}

/// Entries as files in a folder, written atomically.
struct Folder(PathBuf);

impl Backend for Folder {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(self.0.join(self.name_of(key))) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        write_atomic(&self.0, &self.name_of(key), bytes).map_err(|e| e.to_string())
    }

    fn remove(&self, key: &str) -> Result<(), String> {
        match std::fs::remove_file(self.0.join(self.name_of(key))) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn name_of(&self, key: &str) -> String {
        // A plain value natively is the key's own file, holding just the value.
        file_name(split_slot(key).0)
    }
}

/// A key-value store of strings that other code shares (a page's `localStorage`). Each
/// call stands alone: a `set` either stores the whole value or fails.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
trait KeyValue: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// A store's entries in a [`KeyValue`] store, each under the store's prefix.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
struct Keyed<K> {
    prefix: String,
    kv: K,
}

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
impl<K: KeyValue> Backend for Keyed<K> {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.kv.get(&self.name_of(key))?.map(String::into_bytes))
    }

    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        // Saved values are JSON or a string's text, which are always UTF-8.
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        self.kv.set(&self.name_of(key), text)
    }

    fn remove(&self, key: &str) -> Result<(), String> {
        self.kv.delete(&self.name_of(key))
    }

    fn name_of(&self, key: &str) -> String {
        match split_slot(key) {
            (exact, true) => exact.to_owned(),
            (key, false) => format!("{}{key}", self.prefix),
        }
    }
}

/// The prefix of app `app`'s entries in a page's storage: `remedy:<app>:`, with `~` and `:`
/// in the name written as `~` and two hex digits, so no app's entries can be read as
/// another's (the name holds no `:`, so one prefix never starts another).
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn page_prefix(app: &str) -> String {
    let mut out = String::from("remedy:");
    for c in app.chars() {
        match c {
            '~' => out.push_str("~7e"),
            ':' => out.push_str("~3a"),
            c => out.push(c),
        }
    }
    out.push(':');
    out
}

/// The origin's `localStorage`: synchronous, kept across reloads and restarts, shared by the
/// origin's pages. Reached afresh on each call, so the handle stays `Send`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
struct LocalStorage;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl LocalStorage {
    fn storage() -> Result<web_sys::Storage, String> {
        let window =
            web_sys::window().ok_or("there is no browser window here (a worker has no localStorage)")?;
        window
            .local_storage()
            .map_err(js_error)?
            .ok_or_else(|| "the browser gives this page no localStorage".to_owned())
    }
}

/// A browser exception in words: `QuotaExceededError: …` for a full storage.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn js_error(e: wasm_bindgen::JsValue) -> String {
    use wasm_bindgen::JsCast as _;
    if let Some(d) = e.dyn_ref::<web_sys::DomException>() {
        format!("{}: {}", d.name(), d.message())
    } else if let Some(s) = e.as_string() {
        s
    } else {
        format!("{e:?}")
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl KeyValue for LocalStorage {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Self::storage()?.get_item(key).map_err(js_error)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        Self::storage()?.set_item(key, value).map_err(js_error)
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        Self::storage()?.remove_item(key).map_err(js_error)
    }
}

/// Session values ([`Persist::session`]): the tab's `sessionStorage` in a page, memory for
/// the app's run elsewhere.
mod session {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn storage() -> Result<web_sys::Storage, String> {
        web_sys::window()
            .ok_or("there is no browser window here (a worker has no sessionStorage)")?
            .session_storage()
            .map_err(super::js_error)?
            .ok_or_else(|| "the browser gives this page no sessionStorage".to_owned())
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(super) fn get(key: &str) -> Result<Option<String>, String> {
        storage()?.get_item(key).map_err(super::js_error)
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(super) fn set(key: &str, value: Option<&str>) -> Result<(), String> {
        let s = storage()?;
        match value {
            Some(v) => s.set_item(key, v),
            None => s.remove_item(key),
        }
        .map_err(super::js_error)
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    static MEMORY: std::sync::Mutex<std::collections::BTreeMap<String, String>> =
        std::sync::Mutex::new(std::collections::BTreeMap::new());

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub(super) fn get(key: &str) -> Result<Option<String>, String> {
        Ok(super::lock(&MEMORY).get(key).cloned())
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub(super) fn set(key: &str, value: Option<&str>) -> Result<(), String> {
        let mut m = super::lock(&MEMORY);
        match value {
            Some(v) => m.insert(key.to_owned(), v.to_owned()),
            None => m.remove(key),
        };
        Ok(())
    }
}

/// Values saved but not yet written, by key (`None`: removed). The store and its writer
/// share them, so a load sees what was saved before the write lands; the writer takes an
/// entry out only once it is written and no newer value replaced it.
type Unwritten = Arc<Mutex<BTreeMap<String, Option<Arc<[u8]>>>>>;

fn same(a: Option<&Option<Arc<[u8]>>>, b: &Option<Arc<[u8]>>) -> bool {
    match (a, b) {
        (Some(Some(x)), Some(y)) => Arc::ptr_eq(x, y),
        (Some(None), None) => true,
        _ => false,
    }
}

enum Msg {
    /// Something was saved or removed: start the debounce if it is not running.
    Changed,
    Flush(mpsc::Sender<Result<(), String>>),
}

struct Inner {
    /// The folder, or empty for a page's storage.
    dir: PathBuf,
    /// The prefix of the entries in a page's storage, or `None` for a folder.
    prefix: Option<String>,
    backend: Arc<dyn Backend>,
    unwritten: Unwritten,
    tx: Mutex<Option<mpsc::Sender<Msg>>>,
    errors: Arc<Mutex<Vec<String>>>,
    /// The writer: a thread natively, a cooperative task in a browser page.
    worker: remedy_ui_core::exec::Worker,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Closing the channel and ringing the writer makes it write what is pending; the
        // worker then ends with its handle. Nothing waits for it.
        lock(&self.tx).take();
        self.worker.notify();
    }
}

/// Persisted values, with the writer that saves them: a folder natively
/// ([`PersistStore::open`]), the page's `localStorage` in a browser
/// (`PersistStore::local_storage`, wasm32 only). Cloning gives another handle. When the last one is
/// dropped, the writer writes what is pending and stops on its own; code that must know the
/// values are saved (an app quitting or going to the background) calls
/// [`PersistStore::flush`], which waits.
///
/// Besides persisted signals, [`PersistStore::load`], [`PersistStore::save`] and
/// [`PersistStore::remove`] keep any serializable value under a key: an app's own documents
/// (a list of recipes, a library) are kept this way on every platform.
#[derive(Clone)]
pub struct PersistStore {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PersistStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner.prefix {
            Some(p) => write!(f, "PersistStore(localStorage {p}*)"),
            None => write!(f, "PersistStore({})", self.inner.dir.display()),
        }
    }
}

/// The file name for `key`: letters, digits, `.`, `-` and `_` stay; everything else is
/// written as `~` and two hex digits per byte, so any key is a safe, distinct file name.
fn file_name(key: &str) -> String {
    let mut out = String::new();
    for b in key.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || (b == b'.' && !out.is_empty()) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("~{b:02x}"));
        }
    }
    out.push_str(".json");
    out
}

fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)
}

/// The writer's state: where it writes and since when changes wait.
struct Writer {
    backend: Arc<dyn Backend>,
    unwritten: Unwritten,
    rx: mpsc::Receiver<Msg>,
    debounce: Duration,
    errors: Arc<Mutex<Vec<String>>>,
    deadline: Option<Instant>,
}

impl Writer {
    /// Writes every unwritten value. One that fails stays unwritten (loads still see it, and
    /// the next write tries again) and is reported.
    fn write_all(&mut self) -> Result<(), String> {
        self.deadline = None;
        let snapshot = lock(&self.unwritten).clone();
        let mut first_error = None;
        for (key, value) in snapshot {
            let done = match &value {
                Some(bytes) => self.backend.write(&key, bytes),
                None => self.backend.remove(&key),
            };
            match done {
                Ok(()) => {
                    let mut unwritten = lock(&self.unwritten);
                    if same(unwritten.get(&key), &value) {
                        unwritten.remove(&key);
                    }
                }
                Err(e) => {
                    let msg = format!("{}: {e}", self.backend.name_of(&key));
                    lock(&self.errors).push(msg.clone());
                    first_error.get_or_insert(msg);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Takes what arrived, writes when the debounce is over (or a flush asks), and says when
    /// to look again.
    fn step(&mut self) -> Next {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Changed) => {
                    // The first change starts the wait; later ones join the same write.
                    let debounce = self.debounce;
                    self.deadline.get_or_insert_with(|| Instant::now() + debounce);
                }
                Ok(Msg::Flush(reply)) => {
                    let _ = reply.send(self.write_all());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = self.write_all();
                    return Next::Idle;
                }
            }
        }
        match self.deadline {
            None => Next::Idle,
            Some(at) => {
                let now = Instant::now();
                if now >= at {
                    let _ = self.write_all();
                    Next::Idle
                } else {
                    Next::After(at - now)
                }
            }
        }
    }
}

impl PersistStore {
    /// A store in `dir` (created if missing), writing [`DEFAULT_DEBOUNCE`] after a change.
    pub fn open(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        Self::with_debounce(dir, DEFAULT_DEBOUNCE)
    }

    /// A store in `dir` writing `debounce` after the first of a run of changes.
    pub fn with_debounce(dir: impl Into<PathBuf>, debounce: Duration) -> std::io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let backend = Arc::new(Folder(dir.clone()));
        Self::start(dir, None, backend, debounce)
    }

    /// A store for app `app` in the page's `localStorage`, writing [`DEFAULT_DEBOUNCE`] after
    /// a change. Each value is the same versioned JSON a folder store writes, under the key
    /// `remedy:<app>:<key>` (`~` and `:` in the app's name are written as `~7e` and `~3a`), so
    /// apps sharing an origin keep apart. Fails when the browser gives the page no
    /// `localStorage` (storage blocked, or not in a page). A write the browser refuses (the
    /// origin's storage is full) is reported by [`PersistStore::take_errors`] and
    /// [`PersistStore::flush`].
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn local_storage(app: &str) -> std::io::Result<Self> {
        Self::local_storage_with_debounce(app, DEFAULT_DEBOUNCE)
    }

    /// [`PersistStore::local_storage`] writing `debounce` after the first of a run of
    /// changes.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn local_storage_with_debounce(app: &str, debounce: Duration) -> std::io::Result<Self> {
        LocalStorage::storage().map_err(std::io::Error::other)?;
        let prefix = page_prefix(app);
        let backend = Arc::new(Keyed { prefix: prefix.clone(), kv: LocalStorage });
        Self::start(PathBuf::new(), Some(prefix), backend, debounce)
    }

    fn start(
        dir: PathBuf,
        prefix: Option<String>,
        backend: Arc<dyn Backend>,
        debounce: Duration,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let errors: Arc<Mutex<Vec<String>>> = Arc::default();
        let unwritten: Unwritten = Arc::default();
        let (b2, u2, e2) = (backend.clone(), unwritten.clone(), errors.clone());
        let make = move |_| Writer { backend: b2, unwritten: u2, rx, debounce, errors: e2, deadline: None };
        let worker = remedy_ui_core::exec::Worker::spawn_timed("remedy-persist", make, Writer::step)
            .map_err(std::io::Error::other)?;
        Ok(PersistStore {
            inner: Arc::new(Inner {
                dir,
                prefix,
                backend,
                unwritten,
                tx: Mutex::new(Some(tx)),
                errors,
                worker,
            }),
        })
    }

    /// The folder. Empty for a store in a page's `localStorage`, which has none.
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// The file `key` is saved in. Empty for a store in a page's `localStorage`, which keeps
    /// no files ([`PersistStore::storage_key_of`] names its entry).
    pub fn path_of(&self, key: &str) -> PathBuf {
        if self.inner.prefix.is_some() {
            return PathBuf::new();
        }
        self.inner.dir.join(file_name(key))
    }

    /// The `localStorage` key `key` is saved under, for a store in a page
    /// (`PersistStore::local_storage`, in a browser page); `None` for a folder store.
    pub fn storage_key_of(&self, key: &str) -> Option<String> {
        self.inner.prefix.as_ref().map(|p| format!("{p}{key}"))
    }

    /// The value saved under `persist`'s key, brought up to its version: `Ok(None)` when
    /// nothing is saved yet. A value saved and not yet written is seen at once.
    pub fn load<T: DeserializeOwned>(&self, persist: &Persist) -> Result<Option<T>, PersistError> {
        if persist.key.is_empty() {
            return Err(PersistError::EmptyKey);
        }
        let slot = persist.slot();
        let queued = if persist.session {
            Some(session::get(&persist.key).map_err(PersistError::Io)?.map(|t| Arc::from(t.into_bytes())))
        } else {
            lock(&self.inner.unwritten).get(&slot).cloned()
        };
        let bytes: Arc<[u8]> = match queued {
            Some(Some(bytes)) => bytes,
            Some(None) => return Ok(None),
            None => match self.inner.backend.read(&slot).map_err(PersistError::Io)? {
                Some(bytes) => bytes.into(),
                None => return Ok(None),
            },
        };
        let format = |e: serde_json::Error| PersistError::Format(e.to_string());
        match persist.plain {
            Some(Plain::Json) => return serde_json::from_slice(&bytes).map(Some).map_err(format),
            Some(Plain::Text) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                return serde_json::from_value(JsonValue::String(text)).map(Some).map_err(format);
            }
            None => {}
        }
        let file = serde_json::from_slice::<JsonValue>(&bytes).map_err(format)?;
        let found = file
            .get("version")
            .and_then(JsonValue::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| PersistError::Format("no version".into()))?;
        let mut value = file.get("value").cloned().ok_or_else(|| PersistError::Format("no value".into()))?;
        if found > persist.version {
            return Err(PersistError::Newer { found, current: persist.version });
        }
        if found < persist.version {
            value = match &persist.migrate {
                Some(m) => m(found, value).map_err(PersistError::Migration)?,
                None => {
                    return Err(PersistError::Migration(MigrationError::new(format!(
                        "version {found} is older than {} and no migration is given",
                        persist.version
                    ))));
                }
            };
        }
        serde_json::from_value(value).map(Some).map_err(|e| PersistError::Format(e.to_string()))
    }

    /// Saves `value` under `persist`'s key: after the debounce, off this thread (in a page,
    /// between frames). [`PersistStore::load`] sees it at once.
    pub fn save<T: Serialize + ?Sized>(&self, persist: &Persist, value: &T) -> Result<(), PersistError> {
        if persist.key.is_empty() {
            return Err(PersistError::EmptyKey);
        }
        let format = |e: serde_json::Error| PersistError::Format(e.to_string());
        let bytes = match persist.plain {
            Some(Plain::Json) => serde_json::to_vec(value).map_err(format)?,
            Some(Plain::Text) => match serde_json::to_value(value).map_err(format)? {
                JsonValue::String(s) => s.into_bytes(),
                other => {
                    return Err(PersistError::Format(format!("a plain text value must be a string, not {other}")));
                }
            },
            None => {
                let file = serde_json::json!({ "version": persist.version, "value": value });
                serde_json::to_vec_pretty(&file).map_err(format)?
            }
        };
        if persist.session {
            // JSON and a string's text are UTF-8.
            let text = String::from_utf8(bytes).map_err(|e| PersistError::Format(e.to_string()))?;
            return session::set(&persist.key, Some(&text)).map_err(PersistError::Io);
        }
        self.queue(&persist.slot(), Some(bytes.into()))
    }

    /// Forgets the value kept as `persist` says: a versioned one as [`PersistStore::remove`]
    /// does, a plain one ([`Persist::plain`]) under exactly its key, a session one
    /// ([`Persist::session`]) at once.
    pub fn forget(&self, persist: &Persist) -> Result<(), PersistError> {
        if persist.key.is_empty() {
            return Err(PersistError::EmptyKey);
        }
        if persist.session {
            return session::set(&persist.key, None).map_err(PersistError::Io);
        }
        self.queue(&persist.slot(), None)
    }

    /// Forgets the value saved under `key` (a document was deleted), with the same debounce
    /// as a save. Nothing saved under it is not an error.
    pub fn remove(&self, key: &str) -> Result<(), PersistError> {
        if key.is_empty() {
            return Err(PersistError::EmptyKey);
        }
        self.queue(key, None)
    }

    fn queue(&self, key: &str, value: Option<Arc<[u8]>>) -> Result<(), PersistError> {
        let Some(tx) = lock(&self.inner.tx).clone() else {
            return Err(PersistError::Io("the store's writer has stopped".into()));
        };
        lock(&self.inner.unwritten).insert(key.to_owned(), value);
        let sent = tx.send(Msg::Changed).is_ok();
        self.inner.worker.notify();
        if sent { Ok(()) } else { Err(PersistError::Io("the store's writer has stopped".into())) }
    }

    /// Writes everything pending now and waits for it. Fails with the first write that
    /// failed (all of them are in [`PersistStore::take_errors`]).
    pub fn flush(&self) -> std::io::Result<()> {
        let (reply, answer) = mpsc::channel();
        let tx = lock(&self.inner.tx).clone();
        let sent = tx.is_some_and(|tx| tx.send(Msg::Flush(reply)).is_ok());
        if !sent {
            return Err(std::io::Error::other("the store's writer has stopped"));
        }
        self.inner.worker.notify();
        match remedy_ui_core::exec::recv(&answer) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(std::io::Error::other(e)),
            Err(_) => Err(std::io::Error::other("the store's writer has stopped")),
        }
    }

    /// Writes that failed in the background since the last call (the file or storage key,
    /// then why).
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.inner.errors))
    }
}

impl<T: Serialize + DeserializeOwned + 'static> Signal<T> {
    /// Persists the signal in the store provided as context (`PersistStore`; the app
    /// provides one in its windows): the saved value replaces the current one now, and each
    /// change is saved. Without a store, or when the saved value cannot be read, the signal
    /// keeps its value; the reason is logged.
    #[track_caller]
    pub fn persist(self, persist: impl Into<Persist>) -> Self {
        let persist = persist.into();
        let Some(store) = use_context::<PersistStore>() else {
            remedy_ui_core::trace::tracing::warn!(key = persist.key(), "{}", PersistError::NoStore);
            return self;
        };
        match self.persist_in(&store, persist) {
            Ok(s) => s,
            Err((s, e)) => {
                remedy_ui_core::trace::tracing::warn!("{e}");
                s
            }
        }
    }

    /// Persists the signal in `store` ([`Signal::persist`]). On an error the signal is
    /// returned with it: unreadable or damaged files leave the value as it is, and the
    /// value is still saved on change, replacing them; a file from a newer version is left
    /// alone and nothing is saved over it.
    #[track_caller]
    pub fn persist_in(self, store: &PersistStore, persist: Persist) -> Result<Self, (Self, PersistError)> {
        let loaded = store.load::<T>(&persist);
        let error = match loaded {
            Ok(Some(v)) => {
                let _ = self.try_set(v);
                None
            }
            Ok(None) => None,
            Err(e) => Some(e),
        };
        if matches!(error, Some(PersistError::Newer { .. } | PersistError::EmptyKey)) {
            return Err((self, error.unwrap_or(PersistError::EmptyKey)));
        }
        let (store, signal) = (store.clone(), self);
        let mut first = true;
        Effect::new(move || {
            let saved = signal.try_with(|v| {
                if std::mem::take(&mut first) {
                    // The value just restored (or the default) needs no write.
                    return Ok(());
                }
                untrack(|| store.save(&persist, v))
            });
            if let Ok(Err(e)) = saved {
                remedy_ui_core::trace::tracing::warn!("{e}");
            }
        })
        .named("persist");
        match error {
            Some(e) => Err((self, e)),
            None => Ok(self),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The key-value backend (a page's `localStorage`) on an in-memory map, so its logic runs
    //! natively; the browser side is only the three `Storage` calls.

    use super::*;
    use crate::signal::Scope;

    /// A `localStorage` stand-in: a map of strings, with the browser's refusal of a full
    /// storage when a test gives it room for only so many bytes.
    #[derive(Default)]
    struct MemoryKv {
        map: Mutex<BTreeMap<String, String>>,
        room: Mutex<Option<usize>>,
    }

    impl KeyValue for Arc<MemoryKv> {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            Ok(lock(&self.map).get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            let mut map = lock(&self.map);
            if let Some(room) = *lock(&self.room) {
                let others: usize =
                    map.iter().filter(|(k, _)| k.as_str() != key).map(|(k, v)| k.len() + v.len()).sum();
                if others + key.len() + value.len() > room {
                    return Err("QuotaExceededError: the quota has been exceeded.".into());
                }
            }
            map.insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            lock(&self.map).remove(key);
            Ok(())
        }
    }

    fn page_store(app: &str, kv: &Arc<MemoryKv>, debounce: Duration) -> PersistStore {
        let prefix = page_prefix(app);
        let backend = Arc::new(Keyed { prefix: prefix.clone(), kv: kv.clone() });
        match PersistStore::start(PathBuf::new(), Some(prefix), backend, debounce) {
            Ok(s) => s,
            Err(e) => unreachable!("the writer starts: {e}"),
        }
    }

    fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => unreachable!("{e:?}"),
        }
    }

    #[test]
    fn page_prefixes_keep_apps_apart() {
        assert_eq!(page_prefix("Craft Calc"), "remedy:Craft Calc:");
        assert_eq!(page_prefix("a:b~c"), "remedy:a~3ab~7ec:");
        // App "a" with key "b:k" and app "a:b" with key "k" are different entries.
        assert_ne!(format!("{}b:k", page_prefix("a")), format!("{}k", page_prefix("a:b")));
    }

    #[test]
    fn a_page_store_keeps_values_under_the_apps_prefix_in_the_same_envelope() {
        let kv = Arc::new(MemoryKv::default());
        let store = page_store("Craft Calc", &kv, Duration::from_millis(20));
        assert_eq!(store.storage_key_of("recipes").as_deref(), Some("remedy:Craft Calc:recipes"));
        assert_eq!(store.path_of("recipes"), PathBuf::new(), "a page store has no files");
        let recipes = vec![serde_json::json!({"name": "Castile", "oils": {"olive": 1000}})];
        ok(store.save(&Persist::new("recipes"), &recipes));
        ok(store.flush());
        let saved = ok(lock(&kv.map).get("remedy:Craft Calc:recipes").cloned().ok_or("written"));
        let file: JsonValue = ok(serde_json::from_str(&saved));
        assert_eq!(file.get("version"), Some(&serde_json::json!(1)));
        assert_eq!(file.pointer("/value/0/name"), Some(&serde_json::json!("Castile")));
        // A new store over the same storage (the page reloaded) reads it back.
        drop(store);
        let again = page_store("Craft Calc", &kv, Duration::from_millis(20));
        let back: Option<Vec<JsonValue>> = ok(again.load(&Persist::new("recipes")));
        assert_eq!(back, Some(recipes));
        // Another app on the same origin sees nothing of it.
        let other = page_store("Notes", &kv, Duration::from_millis(20));
        assert_eq!(ok(other.load::<Vec<JsonValue>>(&Persist::new("recipes"))), None);
        assert!(again.take_errors().is_empty());
    }

    #[test]
    fn a_persisted_signal_survives_a_reload_of_a_page_store() {
        let kv = Arc::new(MemoryKv::default());
        let store = page_store("Notes", &kv, Duration::from_millis(20));
        let ((), scope) = Scope::new(|| {
            let wrap =
                Signal::new(true).persist_in(&store, Persist::new("editor.wrap")).unwrap_or_else(|(s, _)| s);
            wrap.set(false);
        });
        crate::batch(|| {});
        ok(store.flush());
        scope.dispose();
        drop(store);
        let reloaded = page_store("Notes", &kv, Duration::from_millis(20));
        let ((), _scope) = Scope::new(|| {
            let wrap = Signal::new(true)
                .persist_in(&reloaded, Persist::new("editor.wrap"))
                .unwrap_or_else(|(s, _)| s);
            assert!(!wrap.get(), "the saved value came back");
        });
    }

    #[test]
    fn page_values_migrate_and_newer_ones_are_left_alone() {
        let kv = Arc::new(MemoryKv::default());
        ok(kv.set("remedy:Calc:batch", r#"{"version": 1, "value": {"grams": 500}}"#));
        ok(kv.set("remedy:Calc:future", r#"{"version": 3, "value": 1}"#));
        let store = page_store("Calc", &kv, Duration::from_millis(20));
        let persist = Persist::new("batch").version(2).migrate(|from, v| {
            if from != 1 {
                return Err(format!("unknown version {from}").into());
            }
            let g = v.get("grams").and_then(JsonValue::as_f64).ok_or("no grams")?;
            Ok(serde_json::json!({ "kilograms": g / 1000.0 }))
        });
        let v: Option<JsonValue> = ok(store.load(&persist));
        assert_eq!(v, Some(serde_json::json!({ "kilograms": 0.5 })));
        assert_eq!(
            store.load::<u32>(&Persist::new("future")),
            Err(PersistError::Newer { found: 3, current: 1 })
        );
        ok(kv.set("remedy:Calc:broken", "{ not json"));
        assert!(matches!(store.load::<u32>(&Persist::new("broken")), Err(PersistError::Format(_))));
    }

    #[test]
    fn page_writes_are_debounced_and_loads_see_unwritten_values() {
        let kv = Arc::new(MemoryKv::default());
        let store = page_store("Calc", &kv, Duration::from_millis(400));
        for i in 1..=50u32 {
            ok(store.save(&Persist::new("count"), &i));
        }
        assert!(lock(&kv.map).is_empty(), "nothing is written before the debounce ends");
        assert_eq!(ok(store.load::<u32>(&Persist::new("count"))), Some(50), "a load sees the last save");
        let start = Instant::now();
        while lock(&kv.map).is_empty() {
            assert!(start.elapsed() < Duration::from_secs(20), "the debounced write never came");
            std::thread::sleep(Duration::from_millis(5));
        }
        let saved = ok(lock(&kv.map).get("remedy:Calc:count").cloned().ok_or("written"));
        assert!(saved.contains("50"), "one write, of the last value: {saved}");
        assert!(lock(&store.inner.unwritten).is_empty(), "written values leave the queue");
    }

    #[test]
    fn removing_a_page_value_forgets_it() {
        let kv = Arc::new(MemoryKv::default());
        let store = page_store("Calc", &kv, Duration::from_millis(20));
        ok(store.save(&Persist::new("draft"), "soap"));
        ok(store.flush());
        ok(store.remove("draft"));
        assert_eq!(ok(store.load::<String>(&Persist::new("draft"))), None, "gone at once for loads");
        ok(store.flush());
        assert!(!lock(&kv.map).contains_key("remedy:Calc:draft"));
        assert_eq!(store.remove(""), Err(PersistError::EmptyKey));
    }

    #[test]
    fn a_full_page_storage_is_reported_and_the_value_kept_for_the_next_write() {
        let kv = Arc::new(MemoryKv::default());
        *lock(&kv.room) = Some(64);
        let store = page_store("Calc", &kv, Duration::from_millis(20));
        let big = "x".repeat(200);
        ok(store.save(&Persist::new("library"), &big));
        let Err(e) = store.flush() else { unreachable!("the browser refused the write") };
        assert!(e.to_string().contains("QuotaExceededError"), "{e}");
        let errors = store.take_errors();
        assert!(
            matches!(errors.as_slice(), [e] if e.starts_with("remedy:Calc:library: QuotaExceededError")),
            "{errors:?}"
        );
        assert_eq!(
            ok(store.load::<String>(&Persist::new("library"))),
            Some(big.clone()),
            "still there this session"
        );
        // Room is made (the app or the person cleared space): the next write lands it.
        *lock(&kv.room) = None;
        ok(store.flush());
        assert!(lock(&kv.map).contains_key("remedy:Calc:library"));
        assert!(store.take_errors().is_empty());
    }

    #[test]
    fn a_folder_store_removes_files_and_sees_unwritten_values() {
        let dir = std::env::temp_dir().join(format!("remedy-persist-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = ok(PersistStore::with_debounce(&dir, Duration::from_secs(60)));
        assert_eq!(store.storage_key_of("a"), None);
        ok(store.save(&Persist::new("a"), &1u8));
        assert_eq!(ok(store.load::<u8>(&Persist::new("a"))), Some(1), "seen before the write");
        assert!(!store.path_of("a").exists());
        ok(store.flush());
        assert!(store.path_of("a").exists());
        ok(store.remove("a"));
        ok(store.flush());
        assert!(!store.path_of("a").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_values_are_the_pages_own_items_with_no_envelope() {
        let kv = Arc::new(MemoryKv::default());
        // What the app's older page saved.
        ok(kv.set("portal-theme", "midnight"));
        ok(kv.set("portal-fonts", r#"{"text":"ahmi-print","headings":"system"}"#));
        let store = page_store("Portal", &kv, Duration::from_millis(20));
        let theme = Persist::new("portal-theme").plain(Plain::Text);
        let fonts = Persist::new("portal-fonts").plain(Plain::Json);
        assert_eq!(ok(store.load::<String>(&theme)), Some("midnight".to_owned()));
        let f: Option<BTreeMap<String, String>> = ok(store.load(&fonts));
        assert_eq!(f.and_then(|m| m.get("headings").cloned()).as_deref(), Some("system"));
        // Written back exactly as the old page writes them, beside a versioned value of the
        // same key under the app's prefix.
        ok(store.save(&theme, "paper"));
        let mut both = BTreeMap::new();
        both.insert("headings", "ahmi-pixel");
        both.insert("text", "remedy-sans");
        ok(store.save(&fonts, &both));
        ok(store.save(&Persist::new("portal-theme"), "versioned"));
        ok(store.flush());
        let map = lock(&kv.map).clone();
        assert_eq!(map.get("portal-theme").map(String::as_str), Some("paper"));
        assert_eq!(
            map.get("portal-fonts").map(String::as_str),
            Some(r#"{"headings":"ahmi-pixel","text":"remedy-sans"}"#)
        );
        assert!(map.get("remedy:Portal:portal-theme").is_some_and(|v| v.contains("\"version\"")));
        // A text value must be a string; damaged JSON is a format error, left alone.
        assert!(matches!(store.save(&theme, &3), Err(PersistError::Format(_))));
        ok(kv.set("portal-fonts", "{ not json"));
        assert!(matches!(store.load::<JsonValue>(&fonts), Err(PersistError::Format(_))));
        ok(store.forget(&theme));
        ok(store.flush());
        assert!(!lock(&kv.map).contains_key("portal-theme"));
        assert!(lock(&kv.map).contains_key("remedy:Portal:portal-theme"), "the versioned one stays");
    }

    #[test]
    fn session_values_last_the_run_shared_by_every_store_and_unwritten() {
        let dir = std::env::temp_dir().join(format!("remedy-persist-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = ok(PersistStore::with_debounce(&dir, Duration::from_secs(60)));
        let seasons = Persist::new("portal-seasons-test").session();
        assert_eq!(ok(store.load::<JsonValue>(&seasons)), None);
        let memory = serde_json::json!({"seasonToShow": "x", "seasonNumber": 2, "showName": "S", "labels": {}});
        ok(store.save(&seasons, &memory));
        // Another store of the same run (another window's) sees it at once; nothing is on disk.
        let kv = Arc::new(MemoryKv::default());
        let other = page_store("Other", &kv, Duration::from_secs(60));
        assert_eq!(ok(other.load::<JsonValue>(&seasons)), Some(memory));
        assert!(!store.path_of("portal-seasons-test").exists() && lock(&kv.map).is_empty());
        assert_eq!(ok(session::get("portal-seasons-test")).map(|t| t.contains("\"seasonNumber\":2")), Some(true));
        ok(other.forget(&seasons));
        assert_eq!(ok(store.load::<JsonValue>(&seasons)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plain_signal_follows_the_item_and_a_folder_keeps_just_the_value() {
        let dir = std::env::temp_dir().join(format!("remedy-persist-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = ok(PersistStore::with_debounce(&dir, Duration::from_millis(20)));
        let ((), scope) = Scope::new(|| {
            let theme = Signal::new("portal".to_owned())
                .persist_in(&store, Persist::new("portal-theme").plain(Plain::Text))
                .unwrap_or_else(|(s, _)| s);
            theme.set("midnight".into());
        });
        crate::batch(|| {});
        ok(store.flush());
        scope.dispose();
        assert_eq!(ok(std::fs::read_to_string(store.path_of("portal-theme"))), "midnight");
        let ((), _scope) = Scope::new(|| {
            let theme = Signal::new("portal".to_owned())
                .persist_in(&store, Persist::new("portal-theme").plain(Plain::Text))
                .unwrap_or_else(|(s, _)| s);
            assert_eq!(theme.get(), "midnight");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
