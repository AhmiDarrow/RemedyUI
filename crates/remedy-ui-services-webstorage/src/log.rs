//! LevelDB's log format: the write-ahead log (`NNNNNN.log`) and the manifest are both
//! sequences of records cut into 32 KiB blocks. A record that does not fit the rest of a block
//! continues in the next as FIRST, MIDDLE… and LAST fragments; each fragment has a 7-byte
//! header (masked CRC-32C, length, type).
//!
//! Damage is handled as LevelDB handles it when it opens a store: a fragment whose checksum
//! or length is wrong is dropped with the rest of its block and counted, and reading goes on
//! at the next block. A record cut off at the end of the file is what a writer that stopped
//! mid-write leaves, and is dropped without counting.

use crate::coding::{crc32c, fixed32, unmask_crc};

/// The log's block size.
pub(crate) const BLOCK_SIZE: usize = 32 * 1024;
/// A fragment's header: checksum (4), length (2), type (1).
const HEADER_SIZE: usize = 7;

const ZERO: u8 = 0;
const FULL: u8 = 1;
const FIRST: u8 = 2;
const MIDDLE: u8 = 3;
const LAST: u8 = 4;

/// The records of one log file.
#[derive(Debug, Default)]
pub(crate) struct LogRecords {
    /// Whole records, in file order.
    pub records: Vec<Vec<u8>>,
    /// Damaged stretches that were skipped.
    pub damaged: usize,
}

/// One physical read: a fragment, or why there was none.
enum Physical<'a> {
    Fragment(u8, &'a [u8]),
    /// The fragment (and the rest of its block) is unusable; `reported` when it is damage
    /// rather than padding.
    Bad {
        reported: bool,
    },
    /// The file ends inside this fragment: the writer stopped mid-write.
    Eof,
}

/// Splits a log file into records.
pub(crate) fn read_records(data: &[u8]) -> LogRecords {
    let mut out = LogRecords::default();
    let mut scratch: Vec<u8> = Vec::new();
    let mut in_fragment = false;
    let block_count = data.len().div_ceil(BLOCK_SIZE);
    for (index, block) in data.chunks(BLOCK_SIZE).enumerate() {
        let last_block = index + 1 == block_count;
        let mut rest = block;
        loop {
            if rest.len() < HEADER_SIZE {
                // The block's zero trailer, or (in the last block) a header cut off by the
                // end of the file.
                break;
            }
            let (physical, after) = physical(rest, last_block);
            rest = after;
            match physical {
                Physical::Fragment(FULL, data) => {
                    if in_fragment && !scratch.is_empty() {
                        out.damaged += 1;
                    }
                    in_fragment = false;
                    scratch.clear();
                    out.records.push(data.to_vec());
                }
                Physical::Fragment(FIRST, data) => {
                    if in_fragment && !scratch.is_empty() {
                        out.damaged += 1;
                    }
                    scratch.clear();
                    scratch.extend_from_slice(data);
                    in_fragment = true;
                }
                Physical::Fragment(MIDDLE, data) => {
                    if in_fragment {
                        scratch.extend_from_slice(data);
                    } else {
                        out.damaged += 1;
                    }
                }
                Physical::Fragment(LAST, data) => {
                    if in_fragment {
                        scratch.extend_from_slice(data);
                        out.records.push(std::mem::take(&mut scratch));
                        in_fragment = false;
                    } else {
                        out.damaged += 1;
                    }
                }
                Physical::Fragment(_, _) => {
                    out.damaged += 1;
                    in_fragment = false;
                    scratch.clear();
                }
                Physical::Eof => {
                    in_fragment = false;
                    scratch.clear();
                }
                Physical::Bad { reported } => {
                    if reported {
                        out.damaged += 1;
                    }
                    if in_fragment {
                        if !reported {
                            out.damaged += 1;
                        }
                        in_fragment = false;
                        scratch.clear();
                    }
                }
            }
        }
    }
    // A record still open at the end of the file was being written when the writer stopped:
    // dropped, as LevelDB drops it.
    out
}

/// Reads the fragment at the start of `rest` (which holds at least a header), returning it
/// and what follows it in the block. A bad fragment consumes the rest of the block.
fn physical(rest: &[u8], last_block: bool) -> (Physical<'_>, &[u8]) {
    let (header, body) = rest.split_at(HEADER_SIZE.min(rest.len()));
    let masked = fixed32(header).unwrap_or(0);
    let length = header
        .get(4..6)
        .and_then(|b| <[u8; 2]>::try_from(b).ok())
        .map_or(0, |b| usize::from(u16::from_le_bytes(b)));
    let kind = header.get(6).copied().unwrap_or(ZERO);
    let Some((data, after)) = body.split_at_checked(length) else {
        // The length runs past the block: damage, unless the file simply ends here.
        return (if last_block { Physical::Eof } else { Physical::Bad { reported: true } }, &[]);
    };
    if kind == ZERO && length == 0 {
        // Space preallocated by a writer that maps the file; never written.
        return (Physical::Bad { reported: false }, &[]);
    }
    if unmask_crc(masked) != crc32c(&[&[kind], data]) {
        return (Physical::Bad { reported: true }, &[]);
    }
    (Physical::Fragment(kind, data), after)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::coding::mask_crc;

    /// Writes records as LevelDB's log writer does.
    pub(crate) fn write_log(records: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for record in records {
            let mut left: &[u8] = record;
            let mut begin = true;
            loop {
                let leftover = BLOCK_SIZE - out.len() % BLOCK_SIZE;
                if leftover < HEADER_SIZE {
                    out.resize(out.len() + leftover, 0);
                    continue;
                }
                let avail = BLOCK_SIZE - out.len() % BLOCK_SIZE - HEADER_SIZE;
                let take = avail.min(left.len());
                let end = take == left.len();
                let kind = match (begin, end) {
                    (true, true) => FULL,
                    (true, false) => FIRST,
                    (false, true) => LAST,
                    (false, false) => MIDDLE,
                };
                let (chunk, more) = left.split_at(take);
                fragment(&mut out, kind, chunk);
                left = more;
                begin = false;
                if end {
                    break;
                }
            }
        }
        out
    }

    pub(crate) fn fragment(out: &mut Vec<u8>, kind: u8, data: &[u8]) {
        let crc = mask_crc(crc32c(&[&[kind], data]));
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&u16::try_from(data.len()).unwrap_or(u16::MAX).to_le_bytes());
        out.push(kind);
        out.extend_from_slice(data);
    }

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    #[test]
    fn records_round_trip_across_blocks() {
        let small = b"hello".to_vec();
        let big = pattern(100_000, 1);
        let exact = pattern(BLOCK_SIZE - HEADER_SIZE, 2);
        let empty: Vec<u8> = Vec::new();
        let log = write_log(&[&small, &big, &exact, &empty, &small]);
        let read = read_records(&log);
        assert_eq!(read.damaged, 0);
        assert_eq!(read.records, vec![small.clone(), big, exact, empty, small]);
    }

    #[test]
    fn a_trailer_shorter_than_a_header_is_skipped() {
        // A first record that leaves 6 bytes in the block: the writer pads them with zeros.
        let first = pattern(BLOCK_SIZE - HEADER_SIZE - 6, 3);
        let second = b"next block".to_vec();
        let log = write_log(&[&first, &second]);
        assert_eq!(log.get(BLOCK_SIZE - 6..BLOCK_SIZE), Some(&[0u8; 6][..]));
        let read = read_records(&log);
        assert_eq!((read.records, read.damaged), (vec![first, second], 0));
    }

    #[test]
    fn a_record_cut_off_by_the_end_of_the_file_is_dropped_quietly() {
        let whole = b"kept".to_vec();
        let big = pattern(50_000, 4);
        let log = write_log(&[&whole, &big]);
        for cut in [log.len() - 1, BLOCK_SIZE + 3, BLOCK_SIZE, 20, 12] {
            let read = read_records(log.get(..cut).unwrap_or_default());
            assert_eq!(read.records, vec![whole.clone()], "cut at {cut}");
            assert_eq!(read.damaged, 0, "cut at {cut}");
        }
    }

    #[test]
    fn a_bad_checksum_drops_the_rest_of_the_block_only() {
        let a = b"first".to_vec();
        let b = b"second".to_vec();
        let c = pattern(BLOCK_SIZE, 5); // starts in block 0, ends in block 1
        let d = b"after".to_vec();
        let mut log = write_log(&[&a, &b, &c, &d]);
        // Damage the second record's data.
        let second_data = HEADER_SIZE + a.len() + HEADER_SIZE;
        if let Some(byte) = log.get_mut(second_data) {
            *byte ^= 0xff;
        }
        let read = read_records(&log);
        // `b` fails its checksum and takes `c`'s first fragment with it; `c`'s LAST fragment
        // then has no start.
        assert_eq!(read.records, vec![a, d]);
        assert_eq!(read.damaged, 2);
    }

    #[test]
    fn fragments_out_of_order_are_counted() {
        let mut log = Vec::new();
        fragment(&mut log, MIDDLE, b"orphan middle");
        fragment(&mut log, LAST, b"orphan last");
        fragment(&mut log, FIRST, b"start ");
        fragment(&mut log, FULL, b"whole");
        fragment(&mut log, 9, b"unknown type");
        fragment(&mut log, FIRST, b"a ");
        fragment(&mut log, MIDDLE, b"b ");
        fragment(&mut log, LAST, b"c");
        let read = read_records(&log);
        assert_eq!(read.records, vec![b"whole".to_vec(), b"a b c".to_vec()]);
        // orphan middle, orphan last, FIRST without end, unknown type.
        assert_eq!(read.damaged, 4);
    }

    #[test]
    fn preallocated_zeros_are_not_damage() {
        let mut log = write_log(&[b"one"]);
        log.resize(BLOCK_SIZE, 0);
        log.extend(write_log(&[b"two"]));
        let read = read_records(&log);
        assert_eq!((read.records, read.damaged), (vec![b"one".to_vec(), b"two".to_vec()], 0));
    }

    #[test]
    fn a_length_past_the_block_is_damage_before_the_last_block() {
        let mut log = Vec::new();
        fragment(&mut log, FULL, b"ok");
        // A header claiming more bytes than the block has left.
        log.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, FULL]);
        log.resize(BLOCK_SIZE, 0xAA);
        log.extend(write_log(&[b"next"]));
        let read = read_records(&log);
        assert_eq!(read.records, vec![b"ok".to_vec(), b"next".to_vec()]);
        assert_eq!(read.damaged, 1);
    }
}
