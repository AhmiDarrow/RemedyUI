//! Persisted window geometry.

use std::collections::BTreeMap;
use std::path::PathBuf;

use remedy_ui_core::{Point, Size};

/// Window positions and sizes by key, in a small text file in the app's data directory.
/// Each line is `x y width height key`; keys may contain spaces. Written to a temporary file
/// and renamed, so a crash never leaves a torn file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeometryStore {
    path: Option<PathBuf>,
    entries: BTreeMap<String, (Point, Size)>,
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
                    && [x, y, w, h].iter().all(|v| v.is_finite())
                    && *w > 0.0
                    && *h > 0.0
                {
                    entries.insert(key.to_owned(), (Point::new(*x, *y), Size::new(*w, *h)));
                }
            }
        }
        GeometryStore { path: Some(path), entries }
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

    /// Writes the file.
    pub fn save(&self) -> std::io::Result<()> {
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
/// `~/Library/Application Support/name` on macOS, `$XDG_CONFIG_HOME/name` (or
/// `~/.config/name`) elsewhere.
pub(crate) fn data_dir(name: &str) -> Option<PathBuf> {
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
