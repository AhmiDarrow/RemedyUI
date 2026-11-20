//! The manifest (`MANIFEST-NNNNNN`, named by `CURRENT`): a log of version edits that says
//! which tables are live and which write-ahead logs still hold data not yet in a table.
//!
//! Reading it is optional, but it makes the read exact: a table or log that a compaction made
//! obsolete and that is still on disk (the writer stopped before deleting it) can hold values
//! whose deletion was later compacted away, and reading it would bring them back.

use std::collections::BTreeSet;

use crate::coding::{length_prefixed, varint32, varint64};
use crate::log::read_records;

const COMPARATOR: u32 = 1;
const LOG_NUMBER: u32 = 2;
const NEXT_FILE_NUMBER: u32 = 3;
const LAST_SEQUENCE: u32 = 4;
const COMPACT_POINTER: u32 = 5;
const DELETED_FILE: u32 = 6;
const NEW_FILE: u32 = 7;
const PREV_LOG_NUMBER: u32 = 9;

/// What the manifest says is live.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Manifest {
    /// The live tables' file numbers.
    pub tables: BTreeSet<u64>,
    /// Logs numbered at or above this one hold data not yet in a table.
    pub log_number: u64,
    /// A log from before the last recovery that is still live, or 0.
    pub prev_log_number: u64,
}

impl Manifest {
    /// Whether the log with this file number holds live data.
    pub(crate) fn log_is_live(&self, number: u64) -> bool {
        number >= self.log_number || (self.prev_log_number != 0 && number == self.prev_log_number)
    }
}

/// Reads a manifest. `None` when it is damaged or holds an edit this reader does not know;
/// the caller then reads every file instead.
pub(crate) fn read_manifest(data: &[u8]) -> Option<Manifest> {
    let log = read_records(data);
    if log.damaged > 0 {
        return None;
    }
    let mut live: BTreeSet<(u32, u64)> = BTreeSet::new();
    let mut manifest = Manifest::default();
    let mut saw_log_number = false;
    for record in &log.records {
        let mut input: &[u8] = record;
        while !input.is_empty() {
            let (tag, rest) = varint32(input)?;
            input = match tag {
                COMPARATOR => {
                    let (name, rest) = length_prefixed(rest)?;
                    // Chromium's stores use LevelDB's default byte order; another comparator
                    // would sort keys differently, and this reader would not know it.
                    if name != b"leveldb.BytewiseComparator" {
                        return None;
                    }
                    rest
                }
                LOG_NUMBER => {
                    let (n, rest) = varint64(rest)?;
                    manifest.log_number = n;
                    saw_log_number = true;
                    rest
                }
                PREV_LOG_NUMBER => {
                    let (n, rest) = varint64(rest)?;
                    manifest.prev_log_number = n;
                    rest
                }
                NEXT_FILE_NUMBER | LAST_SEQUENCE => varint64(rest)?.1,
                COMPACT_POINTER => {
                    let (_level, rest) = varint32(rest)?;
                    length_prefixed(rest)?.1
                }
                DELETED_FILE => {
                    let (level, rest) = varint32(rest)?;
                    let (number, rest) = varint64(rest)?;
                    live.remove(&(level, number));
                    rest
                }
                NEW_FILE => {
                    let (level, rest) = varint32(rest)?;
                    let (number, rest) = varint64(rest)?;
                    let (_size, rest) = varint64(rest)?;
                    let (_smallest, rest) = length_prefixed(rest)?;
                    let (_largest, rest) = length_prefixed(rest)?;
                    live.insert((level, number));
                    rest
                }
                _ => return None,
            };
        }
    }
    if !saw_log_number {
        return None;
    }
    manifest.tables = live.into_iter().map(|(_, number)| number).collect();
    Some(manifest)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::coding::put_varint;
    use crate::log::tests::write_log;

    /// One version edit.
    #[derive(Default)]
    pub(crate) struct Edit {
        pub log_number: Option<u64>,
        pub prev_log_number: Option<u64>,
        pub new_files: Vec<(u32, u64)>,
        pub deleted_files: Vec<(u32, u64)>,
    }

    pub(crate) fn encode(edit: &Edit) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, u64::from(COMPARATOR));
        let name = b"leveldb.BytewiseComparator";
        put_varint(&mut out, name.len() as u64);
        out.extend_from_slice(name);
        if let Some(n) = edit.log_number {
            put_varint(&mut out, u64::from(LOG_NUMBER));
            put_varint(&mut out, n);
        }
        if let Some(n) = edit.prev_log_number {
            put_varint(&mut out, u64::from(PREV_LOG_NUMBER));
            put_varint(&mut out, n);
        }
        put_varint(&mut out, u64::from(NEXT_FILE_NUMBER));
        put_varint(&mut out, 99);
        put_varint(&mut out, u64::from(LAST_SEQUENCE));
        put_varint(&mut out, 1234);
        put_varint(&mut out, u64::from(COMPACT_POINTER));
        put_varint(&mut out, 1);
        put_varint(&mut out, 3);
        out.extend_from_slice(b"abc");
        for (level, number) in &edit.deleted_files {
            put_varint(&mut out, u64::from(DELETED_FILE));
            put_varint(&mut out, u64::from(*level));
            put_varint(&mut out, *number);
        }
        for (level, number) in &edit.new_files {
            put_varint(&mut out, u64::from(NEW_FILE));
            put_varint(&mut out, u64::from(*level));
            put_varint(&mut out, *number);
            put_varint(&mut out, 4096);
            for key in [&b"a\x01\0\0\0\0\0\0\0"[..], &b"z\x01\0\0\0\0\0\0\0"[..]] {
                put_varint(&mut out, key.len() as u64);
                out.extend_from_slice(key);
            }
        }
        out
    }

    pub(crate) fn manifest(edits: &[Edit]) -> Vec<u8> {
        let records: Vec<Vec<u8>> = edits.iter().map(encode).collect();
        let refs: Vec<&[u8]> = records.iter().map(Vec::as_slice).collect();
        write_log(&refs)
    }

    #[test]
    fn edits_add_move_and_delete_tables() {
        let data = manifest(&[
            Edit { log_number: Some(3), new_files: vec![(0, 5), (0, 6)], ..Edit::default() },
            // A trivial move of 5 to level 1, a compaction of 6 into 8, and a new log.
            Edit {
                log_number: Some(9),
                prev_log_number: Some(7),
                deleted_files: vec![(0, 5), (0, 6)],
                new_files: vec![(1, 5), (1, 8)],
            },
        ]);
        let m = read_manifest(&data);
        assert_eq!(
            m,
            Some(Manifest { tables: [5, 8].into_iter().collect(), log_number: 9, prev_log_number: 7 })
        );
        let m = m.unwrap_or_default();
        assert!(m.log_is_live(9) && m.log_is_live(10) && m.log_is_live(7));
        assert!(!m.log_is_live(8) && !m.log_is_live(3));
    }

    #[test]
    fn unusable_manifests_are_not_trusted() {
        // No log number: not a complete manifest.
        assert_eq!(read_manifest(&manifest(&[Edit::default()])), None);
        // An unknown tag.
        let mut record = encode(&Edit { log_number: Some(1), ..Edit::default() });
        record.push(42);
        assert_eq!(read_manifest(&write_log(&[&record])), None);
        // A damaged record.
        let mut data = manifest(&[Edit { log_number: Some(1), ..Edit::default() }]);
        if let Some(b) = data.get_mut(10) {
            *b ^= 0xff;
        }
        assert_eq!(read_manifest(&data), None);
    }
}
