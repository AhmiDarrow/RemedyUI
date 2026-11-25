//! Persisted window geometry.

use std::collections::BTreeMap;
use std::path::PathBuf;

use remedy_ui_core::{Point, Size};
use remedy_ui_core::reactive::{JsonValue, Persist, PersistStore};

/// The key window geometry is kept under in a [`PersistStore`]
/// ([`GeometryStore::persisted_in`]).
pub const GEOMETRY_KEY: &str = "remedy-ui.windows";

/// Window positions and sizes by key. Natively a small text file in the app's data
/// directory ([`GeometryStore::open`]): each line is `x y width height key`; keys may contain
/// spaces; written to a temporary file and renamed, so a crash never leaves a torn file. In a
/// browser page, where the app has no files, an entry of the app's [`PersistStore`] (its
/// `localStorage`; [`GeometryStore::persisted_in`]), so floating windows come back where they
/// were after a reload.
#[derive(Clone, Debug, Default)]
pub struct GeometryStore {
    path: Option<PathBuf>,
    store: Option<PersistStore>,
    entries: BTreeMap<String, (Point, Size)>,
}

impl PartialEq for GeometryStore {
    fn eq(&self, other: &Self) -> bool {
        // Two stores are equal when they hold the same entries for the same place.
        let place = |s: &GeometryStore| {
            (
                s.path.clone(),
                s.store.as_ref().map(|p| (p.dir().to_path_buf(), p.storage_key_of(GEOMETRY_KEY))),
            )
        };
        self.entries == other.entries && place(self) == place(other)
    }
}

fn valid(x: f32, y: f32, w: f32, h: f32) -> bool {
    [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0
}

impl GeometryStore {
    /// A store backed by `path`, loaded now (a missing or unreadable file is an empty store).
    pub fn open(path: PathBuf) -> Self {
        let mut entries = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                let mut parts = line.splitn(5, ' ');
                let nums: Vec<f32> = parts.by_ref().take(4).filter_map(|p| p.parse().ok()).collect();
                if let ([x, y, w, h], Some(key)) = (nums.as_slice(), parts.next())
                    && valid(*x, *y, *w, *h)
                {
                    entries.insert(key.to_owned(), (Point::new(*x, *y), Size::new(*w, *h)));
                }
            }
        }
        GeometryStore { path: Some(path), store: None, entries }
    }

    /// A store kept in `store` under [`GEOMETRY_KEY`], loaded now (nothing saved, or an
    /// unreadable entry, is an empty store). The entry is a JSON list of
    /// `[key, x, y, width, height]`. This is how a browser page keeps window geometry; any
    /// platform can use it.
    pub fn persisted_in(store: &PersistStore) -> Self {
        let mut entries = BTreeMap::new();
        if let Ok(Some(JsonValue::Array(rows))) = store.load::<JsonValue>(&Persist::new(GEOMETRY_KEY)) {
            for row in rows {
                let JsonValue::Array(cells) = row else { continue };
                let num = |i: usize| cells.get(i).and_then(JsonValue::as_f64).map(|v| v as f32);
                if let (Some(JsonValue::String(key)), Some(x), Some(y), Some(w), Some(h)) =
                    (cells.first(), num(1), num(2), num(3), num(4))
                    && valid(x, y, w, h)
                {
                    entries.insert(key.clone(), (Point::new(x, y), Size::new(w, h)));
                }
            }
        }
        GeometryStore { path: None, store: Some(store.clone()), entries }
    }

    /// A store that keeps nothing on disk.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The saved geometry for `key`.
    pub fn get(&self, key: &str) -> Option<(Point, Size)> {
        self.entries.get(key).copied()
    }

    /// Records geometry for `key` (saved with [`GeometryStore::save`]).
    pub fn set(&mut self, key: &str, position: Point, size: Size) {
        self.entries.insert(key.to_owned(), (position, size));
    }

    /// Forgets `key` (a note was deleted).
    pub fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Writes the file, or the store's entry (after the store's debounce; see
    /// [`PersistStore::flush`]). An in-memory store writes nothing.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(store) = &self.store {
            let rows: Vec<JsonValue> = self
                .entries
                .iter()
                .map(|(k, (p, s))| {
                    JsonValue::Array(vec![
                        JsonValue::from(k.as_str()),
                        JsonValue::from(p.x),
                        JsonValue::from(p.y),
                        JsonValue::from(s.width),
                        JsonValue::from(s.height),
                    ])
                })
                .collect();
            return store.save(&Persist::new(GEOMETRY_KEY), &rows).map_err(std::io::Error::other);
        }
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = String::new();
        for (k, (p, s)) in &self.entries {
            out.push_str(&format!("{} {} {} {} {}\n", p.x, p.y, s.width, s.height, k.replace('\n', " ")));
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, path)
    }
}

/// The directory an app named `name` keeps its data in: `%APPDATA%\name` on Windows,
/// `~/Library/Application Support/name` on macOS, the app's private internal storage on
/// Android (`/data/user/<user>/<package>/files`, already the app's own, so without the name),
/// `$XDG_CONFIG_HOME/name` (or `~/.config/name`) on Linux and the other Unixes. A browser page
/// has no folder: `None` (its state is kept in `localStorage`, [`crate::App::persist_store`]).
pub(crate) fn data_dir(name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        let _ = name;
        remedy_ui_bridge_android::data_dir()
    }
    #[cfg(not(target_os = "android"))]
    {
        folder(name)
    }
}

/// The directory an app named `name` keeps what it can fetch again (downloaded images):
/// `%LOCALAPPDATA%\name\cache` on Windows, `~/Library/Caches/name` on macOS, the app's cache
/// folder on Android (`/data/user/<user>/<package>/cache`), `$XDG_CACHE_HOME/name` (or
/// `~/.cache/name`) on Linux and the other Unixes. `None` in a browser page.
#[cfg_attr(not(feature = "image"), allow(dead_code))]
pub(crate) fn cache_dir(name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        let _ = name;
        remedy_ui_bridge_android::data_dir().and_then(|d| d.parent().map(|p| p.join("cache")))
    }
    #[cfg(not(target_os = "android"))]
    {
        if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
            return None;
        }
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        if cfg!(windows) {
            env("LOCALAPPDATA").map(|b| b.join(name).join("cache"))
        } else if cfg!(target_os = "macos") {
            env("HOME").map(|h| h.join("Library").join("Caches").join(name))
        } else {
            env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache"))).map(|b| b.join(name))
        }
    }
}

/// The platform's place for app data, by environment variables.
#[cfg(not(target_os = "android"))]
fn folder(name: &str) -> Option<PathBuf> {
    if cfg!(all(target_arch = "wasm32", target_os = "unknown")) {
        return None;
    }
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    }?;
    Some(base.join(name))
}
