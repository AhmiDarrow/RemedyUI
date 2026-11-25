//! A read-only snapshot of a whole LevelDB store: every live table and write-ahead log merged
//! into the newest value of each key.
//!
//! Each write has a sequence number. A key's newest write wins, whether it is a value or a
//! deletion, wherever it was found: a deletion in the log hides an older value in a table,
//! and a newer value hides an older deletion.

use std::collections::BTreeMap;
use std::path::Path;

use crate::services_webstorage::WebStorageError;
use crate::services_webstorage::coding::{fixed32, fixed64, length_prefixed};
use crate::services_webstorage::log::read_records;
use crate::services_webstorage::manifest::{Manifest, read_manifest};
use crate::services_webstorage::table::read_table;

/// A write batch's header: sequence number (8) and count (4).
const BATCH_HEADER: usize = 12;
const TYPE_DELETION: u8 = 0;
const TYPE_VALUE: u8 = 1;

/// The newest state of every key of a store.
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    /// Keys that hold a value, with it.
    pub entries: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Damaged log stretches skipped (as LevelDB skips them when it opens the store).
    pub damaged_log_records: usize,
}

/// What a file of the store is, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FileKind {
    Table(u64),
    Log(u64),
}

fn kind(name: &str) -> Option<FileKind> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = stem.parse().ok()?;
    match ext {
        "ldb" | "sst" => Some(FileKind::Table(number)),
        "log" => Some(FileKind::Log(number)),
        _ => None,
    }
}

/// Where the files come from: a folder on disk, or files the app already holds.
pub(crate) trait Source {
    /// The names of the files in the store.
    fn names(&self) -> &[String];
    /// A file's contents; `Ok(None)` when it is gone (deleted by a running writer since the
    /// names were listed).
    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, WebStorageError>;
}

/// Why a snapshot could not be made.
pub(crate) enum Failure {
    /// A file listed a moment ago is gone: the store changed under the read.
    Vanished(String),
    Error(WebStorageError),
}

impl From<WebStorageError> for Failure {
    fn from(e: WebStorageError) -> Self {
        Self::Error(e)
    }
}

fn read_needed(source: &mut dyn Source, name: &str) -> Result<Vec<u8>, Failure> {
    source.read(name)?.ok_or_else(|| Failure::Vanished(name.to_owned()))
}

/// The manifest `CURRENT` names, when there is a usable one.
fn manifest(source: &mut dyn Source) -> Result<Option<Manifest>, Failure> {
    if !source.names().iter().any(|n| n == "CURRENT") {
        return Ok(None);
    }
    let current = read_needed(source, "CURRENT")?;
    let Some(name) = std::str::from_utf8(&current).ok().map(str::trim_end).filter(|n| {
        n.strip_prefix("MANIFEST-").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    }) else {
        return Ok(None);
    };
    if !source.names().iter().any(|n| n == name) {
        return Ok(None);
    }
    let name = name.to_owned();
    Ok(read_manifest(&read_needed(source, &name)?))
}

/// Reads the store's live files and merges them.
pub(crate) fn snapshot(source: &mut dyn Source) -> Result<Snapshot, Failure> {
    let manifest = manifest(source)?;
    let mut files: Vec<(FileKind, String)> =
        source.names().iter().filter_map(|n| kind(n).map(|k| (k, n.clone()))).collect();
    files.sort();
    if let Some(m) = &manifest {
        for table in &m.tables {
            if !files.iter().any(|(k, _)| *k == FileKind::Table(*table)) {
                // Possibly mid-compaction: the caller lists the folder again.
                return Err(Failure::Vanished(format!("{table:06}.ldb")));
            }
        }
        files.retain(|(k, _)| match k {
            FileKind::Table(n) => m.tables.contains(n),
            FileKind::Log(n) => m.log_is_live(*n),
        });
    }

    let mut newest: BTreeMap<Vec<u8>, (u64, Option<Vec<u8>>)> = BTreeMap::new();
    let mut put = |key: Vec<u8>, seq: u64, value: Option<Vec<u8>>| match newest.get(&key) {
        Some((have, _)) if *have > seq => {}
        _ => {
            newest.insert(key, (seq, value));
        }
    };
    let mut damaged_log_records = 0;
    for (kind, name) in &files {
        let data = read_needed(source, name)?;
        match kind {
            FileKind::Table(_) => {
                for (internal, value) in read_table(name, &data)? {
                    let (user_key, seq, kind) = split_internal_key(&internal)
                        .ok_or_else(|| WebStorageError::corrupt(name, "an entry's key is too short"))?;
                    match kind {
                        TYPE_VALUE => put(user_key.to_vec(), seq, Some(value)),
                        TYPE_DELETION => put(user_key.to_vec(), seq, None),
                        other => {
                            return Err(WebStorageError::corrupt(
                                name,
                                format!("unknown entry type {other}"),
                            )
                            .into());
                        }
                    }
                }
            }
            FileKind::Log(_) => {
                let log = read_records(&data);
                damaged_log_records += log.damaged;
                for batch in &log.records {
                    if !apply_batch(batch, &mut put) {
                        damaged_log_records += 1;
                    }
                }
            }
        }
    }
    let entries = newest.into_iter().filter_map(|(k, (_, v))| v.map(|v| (k, v))).collect();
    Ok(Snapshot { entries, damaged_log_records })
}

/// An internal key: the user's key, then 8 bytes of (sequence << 8 | type).
fn split_internal_key(internal: &[u8]) -> Option<(&[u8], u64, u8)> {
    let (user_key, tag) = internal.split_at_checked(internal.len().checked_sub(8)?)?;
    let tag = fixed64(tag)?;
    Some((user_key, tag >> 8, (tag & 0xff) as u8))
}

/// Applies one write batch: its puts and deletes, numbered from its sequence number. Returns
/// false when the batch is malformed; the writes before the damage are kept, as LevelDB keeps
/// them.
fn apply_batch(batch: &[u8], put: &mut impl FnMut(Vec<u8>, u64, Option<Vec<u8>>)) -> bool {
    let Some((header, mut input)) = batch.split_at_checked(BATCH_HEADER) else {
        return false;
    };
    let (Some(seq), Some(count)) = (fixed64(header), header.get(8..).and_then(fixed32)) else {
        return false;
    };
    let mut found: u64 = 0;
    while let Some((&tag, rest)) = input.split_first() {
        let seq = seq.wrapping_add(found);
        match tag {
            TYPE_VALUE => {
                let Some((key, rest)) = length_prefixed(rest) else { return false };
                let Some((value, rest)) = length_prefixed(rest) else { return false };
                put(key.to_vec(), seq, Some(value.to_vec()));
                input = rest;
            }
            TYPE_DELETION => {
                let Some((key, rest)) = length_prefixed(rest) else { return false };
                put(key.to_vec(), seq, None);
                input = rest;
            }
            _ => return false,
        }
        found += 1;
    }
    found == u64::from(count)
}

/// A store folder on disk, read file by file with read-only opens.
struct Folder<'a> {
    dir: &'a Path,
    names: Vec<String>,
}

impl Source for Folder<'_> {
    fn names(&self) -> &[String] {
        &self.names
    }

    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, WebStorageError> {
        let path = self.dir.join(name);
        // `fs::read` opens for reading only. On Windows it lets other processes keep reading,
        // writing and deleting the file meanwhile, which is how a running app's web view has
        // it open; nothing is locked and nothing is written.
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(WebStorageError::Io { path, source }),
        }
    }
}

fn list(dir: &Path) -> Result<Vec<String>, WebStorageError> {
    let entries = std::fs::read_dir(dir).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            WebStorageError::NotFound { path: dir.to_owned() }
        } else {
            WebStorageError::Io { path: dir.to_owned(), source }
        }
    })?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| WebStorageError::Io { path: dir.to_owned(), source })?;
        // LevelDB names its files in ASCII; anything else is not part of the store.
        if entry.file_type().is_ok_and(|t| t.is_file())
            && let Ok(name) = entry.file_name().into_string()
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Snapshots a store folder. A running writer may compact while the files are read, deleting
/// one that was listed; the folder is then listed and read again, for as long as its contents
/// keep changing. A file that is gone while the listing stays the same is reported.
pub(crate) fn read_folder(dir: &Path) -> Result<Snapshot, WebStorageError> {
    let mut previous: Option<Vec<String>> = None;
    loop {
        let names = list(dir)?;
        let mut folder = Folder { dir, names };
        match snapshot(&mut folder) {
            Ok(snapshot) => return Ok(snapshot),
            Err(Failure::Error(e)) => return Err(e),
            Err(Failure::Vanished(name)) => {
                if previous.as_ref() == Some(&folder.names) {
                    return Err(WebStorageError::corrupt(&name, "listed in the store but missing"));
                }
                previous = Some(folder.names);
            }
        }
    }
}

/// Files the app holds in memory.
pub(crate) struct Files {
    pub names: Vec<String>,
    pub contents: BTreeMap<String, Vec<u8>>,
}

impl Source for Files {
    fn names(&self) -> &[String] {
        &self.names
    }

    fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, WebStorageError> {
        Ok(self.contents.get(name).cloned())
    }
}

/// Snapshots a store from files in memory.
pub(crate) fn read_files(files: Files) -> Result<Snapshot, WebStorageError> {
    let mut files = files;
    match snapshot(&mut files) {
        Ok(s) => Ok(s),
        Err(Failure::Error(e)) => Err(e),
        Err(Failure::Vanished(name)) => {
            Err(WebStorageError::corrupt(&name, "listed in the store but missing"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services_webstorage::coding::put_varint;
    use crate::services_webstorage::log::tests::write_log;
    use crate::services_webstorage::manifest::tests::{Edit, manifest};
    use crate::services_webstorage::table::tests::build_table;

    pub(crate) fn batch(seq: u64, ops: &[(&[u8], Option<&[u8]>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&u32::try_from(ops.len()).unwrap_or(0).to_le_bytes());
        for (key, value) in ops {
            out.push(if value.is_some() { TYPE_VALUE } else { TYPE_DELETION });
            put_varint(&mut out, key.len() as u64);
            out.extend_from_slice(key);
            if let Some(value) = value {
                put_varint(&mut out, value.len() as u64);
                out.extend_from_slice(value);
            }
        }
        out
    }

    fn internal(key: &[u8], seq: u64, kind: u8) -> Vec<u8> {
        let mut k = key.to_vec();
        k.extend_from_slice(&((seq << 8) | u64::from(kind)).to_le_bytes());
        k
    }

    fn files(list: Vec<(&str, Vec<u8>)>) -> Files {
        let names = list.iter().map(|(n, _)| (*n).to_owned()).collect();
        Files { names, contents: list.into_iter().map(|(n, c)| (n.to_owned(), c)).collect() }
    }

    fn entries(s: &Snapshot) -> Vec<(&str, &str)> {
        s.entries
            .iter()
            .map(|(k, v)| (std::str::from_utf8(k).unwrap_or("?"), std::str::from_utf8(v).unwrap_or("?")))
            .collect()
    }

    #[test]
    fn the_newest_write_wins_across_tables_and_logs() -> Result<(), WebStorageError> {
        // Table: a=1 (seq 1), b=1 (seq 2), c deleted (seq 3), d=1 (seq 4).
        let table = build_table(
            &[
                (internal(b"a", 1, TYPE_VALUE), b"a1".to_vec()),
                (internal(b"b", 2, TYPE_VALUE), b"b1".to_vec()),
                (internal(b"c", 3, TYPE_DELETION), Vec::new()),
                (internal(b"d", 4, TYPE_VALUE), b"d1".to_vec()),
            ],
            2,
            true,
        );
        // An older table entry for c, from before its deletion, at another level.
        let old = build_table(&[(internal(b"c", 0, TYPE_VALUE), b"c0".to_vec())], 1, false);
        // Log: b deleted (seq 10), c set again (11), a overwritten twice in one batch (12, 13).
        let log = write_log(&[
            &batch(10, &[(b"b", None), (b"c", Some(b"c2"))]),
            &batch(12, &[(b"a", Some(b"a2")), (b"a", Some(b"a3"))]),
        ]);
        let snap = read_files(files(vec![
            ("000005.ldb", table),
            ("000004.sst", old),
            ("000007.log", log),
            ("LOCK", Vec::new()),
            ("LOG", b"a text log".to_vec()),
        ]))?;
        assert_eq!(entries(&snap), [("a", "a3"), ("c", "c2"), ("d", "d1")]);
        assert_eq!(snap.damaged_log_records, 0);
        Ok(())
    }

    #[test]
    fn a_newer_table_deletion_hides_an_older_log_value() -> Result<(), WebStorageError> {
        let log = write_log(&[&batch(1, &[(b"k", Some(b"v"))])]);
        let table = build_table(&[(internal(b"k", 2, TYPE_DELETION), Vec::new())], 1, false);
        let snap = read_files(files(vec![("000003.log", log), ("000004.ldb", table)]))?;
        assert!(snap.entries.is_empty());
        Ok(())
    }

    #[test]
    fn the_manifest_leaves_out_obsolete_files() -> Result<(), WebStorageError> {
        // Table 5 held k=old; a compaction into table 8 dropped k's value and its deletion
        // (nothing older remained below). Table 5 and log 3 are still on disk.
        let obsolete = build_table(&[(internal(b"k", 1, TYPE_VALUE), b"old".to_vec())], 1, false);
        let live = build_table(&[(internal(b"x", 3, TYPE_VALUE), b"live".to_vec())], 1, true);
        let old_log = write_log(&[&batch(1, &[(b"k", Some(b"old"))])]);
        let new_log = write_log(&[&batch(4, &[(b"y", Some(b"new"))])]);
        let m = manifest(&[
            Edit { log_number: Some(3), new_files: vec![(0, 5)], ..Edit::default() },
            Edit {
                log_number: Some(9),
                deleted_files: vec![(0, 5)],
                new_files: vec![(1, 8)],
                ..Edit::default()
            },
        ]);
        let list = vec![
            ("CURRENT", b"MANIFEST-000002\n".to_vec()),
            ("MANIFEST-000002", m),
            ("000003.log", old_log),
            ("000005.ldb", obsolete),
            ("000008.ldb", live),
            ("000009.log", new_log),
        ];
        let with = read_files(files(list.clone()))?;
        assert_eq!(entries(&with), [("x", "live"), ("y", "new")]);
        // Without CURRENT, every file is read.
        let without = read_files(files(list.into_iter().filter(|(n, _)| *n != "CURRENT").collect()))?;
        assert_eq!(entries(&without), [("k", "old"), ("x", "live"), ("y", "new")]);
        Ok(())
    }

    #[test]
    fn a_table_the_manifest_names_must_exist() {
        let m = manifest(&[Edit { log_number: Some(1), new_files: vec![(0, 5)], ..Edit::default() }]);
        let result =
            read_files(files(vec![("CURRENT", b"MANIFEST-000002\n".to_vec()), ("MANIFEST-000002", m)]));
        assert!(matches!(result, Err(WebStorageError::Corrupt { .. })));
    }

    #[test]
    fn malformed_batches_keep_their_good_writes_and_are_counted() -> Result<(), WebStorageError> {
        let mut truncated = batch(1, &[(b"a", Some(b"1")), (b"b", Some(b"2"))]);
        truncated.truncate(truncated.len() - 1);
        let mut wrong_count = batch(5, &[(b"c", Some(b"3"))]);
        if let Some(b) = wrong_count.get_mut(8) {
            *b = 2;
        }
        let mut bad_tag = batch(7, &[(b"d", Some(b"4"))]);
        bad_tag.push(9);
        let log = write_log(&[&truncated, &wrong_count, &bad_tag, b"short"]);
        let snap = read_files(files(vec![("000001.log", log)]))?;
        assert_eq!(entries(&snap), [("a", "1"), ("c", "3"), ("d", "4")]);
        assert_eq!(snap.damaged_log_records, 4);
        Ok(())
    }

    #[test]
    fn file_names() {
        assert_eq!(kind("000123.ldb"), Some(FileKind::Table(123)));
        assert_eq!(kind("7.sst"), Some(FileKind::Table(7)));
        assert_eq!(kind("000004.log"), Some(FileKind::Log(4)));
        for other in
            ["LOG", "LOG.old", "LOCK", "CURRENT", "MANIFEST-000001", ".log", "a1.log", "000004.dbtmp"]
        {
            assert_eq!(kind(other), None, "{other}");
        }
    }
}
