//! Customization files on disk: read whole, written atomically (a temporary file in the same
//! folder, flushed, then renamed over the old one), so a crash or a full disk never leaves a
//! half-written file behind.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Files and their contents (`None`: the file does not exist).
pub(crate) type FileSet = Vec<(PathBuf, Option<Vec<u8>>)>;

/// The bytes of `path`, or `None` when there is no such file.
pub(crate) fn read(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes `bytes` to `path` atomically, creating its folder when needed.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp =
        dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Removes `path`; a file that is already gone is fine.
pub(crate) fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Writes `bytes` to `path`, or removes it for `None`.
pub(crate) fn put(path: &Path, bytes: Option<&[u8]>) -> std::io::Result<()> {
    match bytes {
        Some(b) => write_atomic(path, b),
        None => remove(path),
    }
}

/// What each of `paths` holds now. A file that cannot be read counts as absent: restoring it
/// later would remove it, which is what reading it already amounted to.
pub(crate) fn snapshot(paths: &[PathBuf]) -> FileSet {
    paths.iter().map(|p| (p.clone(), read(p).ok().flatten())).collect()
}

/// Puts every file of `set` back as it was. The first failure is returned, after trying the
/// rest.
pub(crate) fn restore(set: &FileSet) -> Result<(), (PathBuf, std::io::Error)> {
    let mut first = None;
    for (path, bytes) in set {
        if let Err(e) = put(path, bytes.as_deref())
            && first.is_none()
        {
            first = Some((path.clone(), e));
        }
    }
    first.map_or(Ok(()), Err)
}

/// The files in `dir` with one of `extensions`, sorted (none when the folder is missing).
pub(crate) fn list(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().and_then(|x| x.to_str()).is_some_and(|x| extensions.contains(&x))
                && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    out.sort();
    out
}

/// Every file in `dir` (not in subfolders, and not our own temporary files), sorted.
pub(crate) fn list_all(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    out.sort();
    out
}

/// The files under `dir` (in subfolders too) with one of `extensions`, sorted.
pub(crate) fn list_tree(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut out = list(dir, extensions);
    let mut subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();
    for d in subdirs {
        out.extend(list_tree(&d, extensions));
    }
    out
}

/// A JSON document as people read it in a file: pretty, keys sorted, a final newline.
pub(crate) fn json_text(v: &serde_json::Value) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
    s.push('\n');
    s.into_bytes()
}
