//! Persisted signals: state that survives a restart (window splits, scroll anchors, drafts,
//! choices people made), saved to the app's data folder and restored when made.
//!
//! A [`PersistStore`] is a folder. A signal marked with [`Signal::persist`] (or
//! [`Signal::persist_in`]) is restored from its file at once and saved after it changes.
//! Saves are debounced (many changes in a short time make one write) and written off the UI
//! thread, atomically: to a temporary file that is synced and then renamed over the old one,
//! so a crash leaves the old value or the new one, never half of each. Each file holds a
//! version and the value as JSON; an app that changes a value's shape raises the version and
//! gives a migration from older versions.
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
}

impl std::fmt::Debug for Persist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Persist").field("key", &self.key).field("version", &self.version).finish()
    }
}

impl Persist {
    /// Persisted under `key`, at version 1.
    pub fn new(key: impl Into<String>) -> Self {
        Persist { key: key.into(), version: 1, migrate: None }
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

enum Msg {
    Write(String, Vec<u8>),
    Flush(mpsc::Sender<Result<(), String>>),
}

struct Inner {
    dir: PathBuf,
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

/// A folder of persisted values, with the writer that saves them. Cloning gives another
/// handle. When the last one is dropped, the writer writes what is pending and stops on its
/// own; code that must know the values are on disk (an app quitting) calls
/// [`PersistStore::flush`], which waits.
#[derive(Clone)]
pub struct PersistStore {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PersistStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PersistStore({})", self.inner.dir.display())
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

/// The writer's state: what waits to be written and since when.
struct Writer {
    dir: PathBuf,
    rx: mpsc::Receiver<Msg>,
    debounce: Duration,
    errors: Arc<Mutex<Vec<String>>>,
    pending: BTreeMap<String, Vec<u8>>,
    deadline: Option<Instant>,
}

impl Writer {
    fn write_all(&mut self) -> Result<(), String> {
        self.deadline = None;
        let mut first_error = None;
        for (name, bytes) in std::mem::take(&mut self.pending) {
            if let Err(e) = write_atomic(&self.dir, &name, &bytes) {
                let msg = format!("{name}: {e}");
                lock(&self.errors).push(msg.clone());
                first_error.get_or_insert(msg);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Takes what arrived, writes when the debounce is over (or a flush asks), and says when
    /// to look again.
    fn step(&mut self) -> Next {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Write(name, bytes)) => {
                    self.pending.insert(name, bytes);
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
        let (tx, rx) = mpsc::channel();
        let errors: Arc<Mutex<Vec<String>>> = Arc::default();
        let (d2, e2) = (dir.clone(), errors.clone());
        let make =
            move |_| Writer { dir: d2, rx, debounce, errors: e2, pending: BTreeMap::new(), deadline: None };
        let worker = remedy_ui_core::exec::Worker::spawn_timed("remedy-persist", make, Writer::step)
            .map_err(std::io::Error::other)?;
        Ok(PersistStore { inner: Arc::new(Inner { dir, tx: Mutex::new(Some(tx)), errors, worker }) })
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// The file `key` is saved in.
    pub fn path_of(&self, key: &str) -> PathBuf {
        self.inner.dir.join(file_name(key))
    }

    /// The value saved under `persist`'s key, brought up to its version: `Ok(None)` when
    /// nothing is saved yet.
    pub fn load<T: DeserializeOwned>(&self, persist: &Persist) -> Result<Option<T>, PersistError> {
        if persist.key.is_empty() {
            return Err(PersistError::EmptyKey);
        }
        let bytes = match std::fs::read(self.path_of(&persist.key)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(PersistError::Io(e.to_string())),
        };
        let file: JsonValue =
            serde_json::from_slice(&bytes).map_err(|e| PersistError::Format(e.to_string()))?;
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

    /// Saves `value` under `persist`'s key: after the debounce, off this thread.
    pub fn save<T: Serialize + ?Sized>(&self, persist: &Persist, value: &T) -> Result<(), PersistError> {
        if persist.key.is_empty() {
            return Err(PersistError::EmptyKey);
        }
        let file = serde_json::json!({ "version": persist.version, "value": value });
        let bytes = serde_json::to_vec_pretty(&file).map_err(|e| PersistError::Format(e.to_string()))?;
        let tx = lock(&self.inner.tx).clone();
        let sent = tx.is_some_and(|tx| tx.send(Msg::Write(file_name(&persist.key), bytes)).is_ok());
        self.inner.worker.notify();
        if sent { Ok(()) } else { Err(PersistError::Io("the store's writer has stopped".into())) }
    }

    /// Writes everything pending now and waits for it.
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

    /// Writes that failed in the background since the last call (file, then why).
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
            remedy_ui_trace::tracing::warn!(key = persist.key(), "{}", PersistError::NoStore);
            return self;
        };
        match self.persist_in(&store, persist) {
            Ok(s) => s,
            Err((s, e)) => {
                remedy_ui_trace::tracing::warn!("{e}");
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
                remedy_ui_trace::tracing::warn!("{e}");
            }
        })
        .named("persist");
        match error {
            Some(e) => Err((self, e)),
            None => Ok(self),
        }
    }
}
