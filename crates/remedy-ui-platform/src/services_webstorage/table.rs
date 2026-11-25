//! LevelDB's sorted tables (`NNNNNN.ldb`, `NNNNNN.sst` from older versions): data blocks of
//! prefix-compressed entries with restart points, an index block that points at each data
//! block, and a 48-byte footer that points at the index. Each block is followed by a
//! compression type byte and a masked CRC-32C of the block and that byte.

use std::borrow::Cow;

use crate::services_webstorage::WebStorageError;
use crate::services_webstorage::coding::{crc32c, fixed32, fixed64, unmask_crc, varint32, varint64};

/// The footer's size: two block handles padded to 40 bytes, then the magic number.
const FOOTER_SIZE: usize = 48;
/// The magic number that ends every table.
const MAGIC: u64 = 0xdb47_7524_8b80_fb57;
/// After each block: compression type (1) and checksum (4).
const TRAILER_SIZE: usize = 5;

const NO_COMPRESSION: u8 = 0;
const SNAPPY: u8 = 1;

/// A table's or block's entries in order: (key, value).
pub(crate) type Entries = Vec<(Vec<u8>, Vec<u8>)>;

/// Where a block is in the file.
#[derive(Clone, Copy, Debug)]
struct BlockHandle {
    offset: u64,
    size: u64,
}

impl BlockHandle {
    fn decode(input: &[u8]) -> Option<(Self, &[u8])> {
        let (offset, rest) = varint64(input)?;
        let (size, rest) = varint64(rest)?;
        Some((Self { offset, size }, rest))
    }
}

/// Every entry of a table, in file order: (internal key, value).
pub(crate) fn read_table(file: &str, data: &[u8]) -> Result<Entries, WebStorageError> {
    let footer = data
        .len()
        .checked_sub(FOOTER_SIZE)
        .and_then(|start| data.get(start..))
        .ok_or_else(|| WebStorageError::corrupt(file, "shorter than a table footer"))?;
    if fixed64(footer.get(40..).unwrap_or_default()) != Some(MAGIC) {
        return Err(WebStorageError::corrupt(file, "not a LevelDB table (bad magic number)"));
    }
    let (_metaindex, rest) =
        BlockHandle::decode(footer).ok_or_else(|| WebStorageError::corrupt(file, "bad footer"))?;
    let (index, _) = BlockHandle::decode(rest).ok_or_else(|| WebStorageError::corrupt(file, "bad footer"))?;

    let index_block = read_block(file, data, index)?;
    let mut entries = Vec::new();
    for (_separator, handle) in block_entries(file, &index_block)? {
        let (handle, _) = BlockHandle::decode(&handle)
            .ok_or_else(|| WebStorageError::corrupt(file, "bad block handle in the index"))?;
        let block = read_block(file, data, handle)?;
        entries.extend(block_entries(file, &block)?);
    }
    Ok(entries)
}

/// A block's contents, checked and decompressed.
fn read_block<'a>(file: &str, data: &'a [u8], handle: BlockHandle) -> Result<Cow<'a, [u8]>, WebStorageError> {
    let out_of_file = || WebStorageError::corrupt(file, "a block handle points past the end of the file");
    let start = usize::try_from(handle.offset).map_err(|_| out_of_file())?;
    let size = usize::try_from(handle.size).map_err(|_| out_of_file())?;
    let end = start.checked_add(size).ok_or_else(out_of_file)?;
    let contents = data.get(start..end).ok_or_else(out_of_file)?;
    let trailer = end.checked_add(TRAILER_SIZE).and_then(|t| data.get(end..t)).ok_or_else(out_of_file)?;
    let (kind, crc) = trailer.split_first().ok_or_else(out_of_file)?;
    let stored = fixed32(crc).ok_or_else(out_of_file)?;
    if unmask_crc(stored) != crc32c(&[contents, &[*kind]]) {
        return Err(WebStorageError::corrupt(file, format!("block at offset {start} fails its checksum")));
    }
    match *kind {
        NO_COMPRESSION => Ok(Cow::Borrowed(contents)),
        SNAPPY => snap::raw::Decoder::new()
            .decompress_vec(contents)
            .map(Cow::Owned)
            .map_err(|e| WebStorageError::corrupt(file, format!("block at offset {start}: {e}"))),
        kind => Err(WebStorageError::UnsupportedCompression { file: file.to_owned(), kind }),
    }
}

/// The (key, value) entries of a block. Keys share prefixes with the key before them, except
/// at restart points; the restart array at the end of the block marks where entries stop.
pub(crate) fn block_entries(file: &str, block: &[u8]) -> Result<Entries, WebStorageError> {
    let bad = |what: &str| WebStorageError::corrupt(file, format!("bad block: {what}"));
    let restart_count = block
        .len()
        .checked_sub(4)
        .and_then(|at| block.get(at..))
        .and_then(fixed32)
        .ok_or_else(|| bad("too short for its restart count"))?;
    let restart_bytes = usize::try_from(restart_count)
        .ok()
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| bad("restart count out of range"))?;
    let entries_end = block.len().checked_sub(restart_bytes).ok_or_else(|| bad("restart array too large"))?;
    let mut input = block.get(..entries_end).unwrap_or_default();
    let mut key: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    while !input.is_empty() {
        let (shared, rest) = varint32(input).ok_or_else(|| bad("entry header"))?;
        let (unshared, rest) = varint32(rest).ok_or_else(|| bad("entry header"))?;
        let (value_len, rest) = varint32(rest).ok_or_else(|| bad("entry header"))?;
        let shared = usize::try_from(shared).map_err(|_| bad("entry header"))?;
        let unshared = usize::try_from(unshared).map_err(|_| bad("entry header"))?;
        let value_len = usize::try_from(value_len).map_err(|_| bad("entry header"))?;
        if shared > key.len() {
            return Err(bad("an entry shares more of the previous key than there is"));
        }
        let (delta, rest) = rest.split_at_checked(unshared).ok_or_else(|| bad("key runs past the block"))?;
        let (value, rest) =
            rest.split_at_checked(value_len).ok_or_else(|| bad("value runs past the block"))?;
        key.truncate(shared);
        key.extend_from_slice(delta);
        out.push((key.clone(), value.to_vec()));
        input = rest;
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::services_webstorage::coding::{mask_crc, put_varint};

    /// Builds a block as LevelDB's block builder does, with a restart every `interval` entries.
    pub(crate) fn build_block(entries: &[(Vec<u8>, Vec<u8>)], interval: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut restarts = vec![0u32];
        let mut last: &[u8] = &[];
        for (i, (key, value)) in entries.iter().enumerate() {
            let shared = if i % interval == 0 {
                if i > 0 {
                    restarts.push(u32::try_from(out.len()).unwrap_or(0));
                }
                0
            } else {
                last.iter().zip(key).take_while(|(a, b)| a == b).count()
            };
            put_varint(&mut out, shared as u64);
            put_varint(&mut out, (key.len() - shared) as u64);
            put_varint(&mut out, value.len() as u64);
            out.extend_from_slice(key.get(shared..).unwrap_or_default());
            out.extend_from_slice(value);
            last = key;
        }
        for r in &restarts {
            out.extend_from_slice(&r.to_le_bytes());
        }
        out.extend_from_slice(&u32::try_from(restarts.len()).unwrap_or(0).to_le_bytes());
        out
    }

    fn put_block(file: &mut Vec<u8>, block: &[u8], snappy: bool) -> Vec<u8> {
        let offset = file.len();
        let (stored, kind) = if snappy {
            (snap::raw::Encoder::new().compress_vec(block).unwrap_or_default(), SNAPPY)
        } else {
            (block.to_vec(), NO_COMPRESSION)
        };
        file.extend_from_slice(&stored);
        file.push(kind);
        file.extend_from_slice(&mask_crc(crc32c(&[&stored, &[kind]])).to_le_bytes());
        let mut handle = Vec::new();
        put_varint(&mut handle, offset as u64);
        put_varint(&mut handle, stored.len() as u64);
        handle
    }

    /// Builds a table of sorted internal-key entries, `per_block` entries to a data block.
    pub(crate) fn build_table(entries: &[(Vec<u8>, Vec<u8>)], per_block: usize, snappy: bool) -> Vec<u8> {
        let mut file = Vec::new();
        let mut index = Vec::new();
        for chunk in entries.chunks(per_block.max(1)) {
            let handle = put_block(&mut file, &build_block(chunk, 16), snappy);
            let last_key = chunk.last().map(|(k, _)| k.clone()).unwrap_or_default();
            index.push((last_key, handle));
        }
        let meta = put_block(&mut file, &build_block(&[], 16), false);
        let index_handle = put_block(&mut file, &build_block(&index, 1), snappy);
        let mut footer = Vec::new();
        footer.extend_from_slice(&meta);
        footer.extend_from_slice(&index_handle);
        footer.resize(40, 0);
        footer.extend_from_slice(&MAGIC.to_le_bytes());
        file.extend_from_slice(&footer);
        file
    }

    fn entries(n: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
        (0..n)
            .map(|i| {
                let key = format!("key-{i:05}").into_bytes();
                let value = format!("value {i} ").repeat(i % 7 + 1).into_bytes();
                (key, value)
            })
            .collect()
    }

    #[test]
    fn blocks_with_shared_prefixes_and_restarts_decode() -> Result<(), WebStorageError> {
        let list = entries(100);
        for interval in [1, 2, 16, 1000] {
            assert_eq!(block_entries("t", &build_block(&list, interval))?, list, "restart every {interval}");
        }
        assert_eq!(block_entries("t", &build_block(&[], 16))?, Vec::new());
        Ok(())
    }

    #[test]
    fn tables_decode_plain_and_snappy() -> Result<(), WebStorageError> {
        let list = entries(500);
        for snappy in [false, true] {
            for per_block in [1, 7, 500] {
                let table = build_table(&list, per_block, snappy);
                assert_eq!(read_table("t", &table)?, list, "snappy {snappy}, {per_block} per block");
            }
        }
        Ok(())
    }

    #[test]
    fn damaged_tables_are_errors_not_partial_reads() {
        let table = build_table(&entries(50), 10, true);
        // Every single flipped byte in a data block is caught by its checksum.
        for at in [0, 5, 40] {
            let mut bad = table.clone();
            if let Some(b) = bad.get_mut(at) {
                *b ^= 0x01;
            }
            assert!(matches!(read_table("t", &bad), Err(WebStorageError::Corrupt { .. })), "flip at {at}");
        }
        let mut no_magic = table.clone();
        if let Some(b) = no_magic.last_mut() {
            *b ^= 0x01;
        }
        assert!(matches!(read_table("t", &no_magic), Err(WebStorageError::Corrupt { .. })));
        assert!(matches!(
            read_table("t", table.get(..20).unwrap_or_default()),
            Err(WebStorageError::Corrupt { .. })
        ));
        assert!(matches!(read_table("t", &[]), Err(WebStorageError::Corrupt { .. })));
    }

    #[test]
    fn unknown_compression_is_reported() {
        let mut file = Vec::new();
        let block = build_block(&entries(3), 16);
        file.extend_from_slice(&block);
        file.push(2); // zstd in newer LevelDB
        file.extend_from_slice(&mask_crc(crc32c(&[&block, &[2]])).to_le_bytes());
        let mut index_entry = Vec::new();
        put_varint(&mut index_entry, 0);
        put_varint(&mut index_entry, block.len() as u64);
        let index = put_block(&mut file, &build_block(&[(b"z".to_vec(), index_entry)], 1), false);
        let mut footer = Vec::new();
        footer.extend_from_slice(&index);
        footer.extend_from_slice(&index);
        footer.resize(40, 0);
        footer.extend_from_slice(&MAGIC.to_le_bytes());
        file.extend_from_slice(&footer);
        assert!(matches!(
            read_table("t", &file),
            Err(WebStorageError::UnsupportedCompression { kind: 2, .. })
        ));
    }

    #[test]
    fn malformed_blocks_are_rejected() {
        assert!(block_entries("t", &[1, 2]).is_err(), "no restart count");
        assert!(
            block_entries("t", &[0xff, 0xff, 0xff, 0x0f]).is_err(),
            "restart array larger than the block"
        );
        // One entry sharing 3 bytes of a previous key that does not exist.
        let mut block = vec![3, 1, 0, b'k'];
        block.extend_from_slice(&0u32.to_le_bytes());
        block.extend_from_slice(&1u32.to_le_bytes());
        assert!(block_entries("t", &block).is_err());
        // A value longer than the block.
        let mut block = vec![0, 1, 50, b'k'];
        block.extend_from_slice(&0u32.to_le_bytes());
        block.extend_from_slice(&1u32.to_le_bytes());
        assert!(block_entries("t", &block).is_err());
    }

    #[test]
    fn chromes_table_is_snappy_compressed() -> Result<(), WebStorageError> {
        // The compacted fixture's table, as Chrome wrote it: every data block is Snappy.
        let data = include_bytes!("../../tests/services_webstorage/fixtures/compacted/000003.ldb");
        let footer = data.get(data.len() - FOOTER_SIZE..).unwrap_or_default();
        let (_, rest) = BlockHandle::decode(footer).unwrap_or((BlockHandle { offset: 0, size: 0 }, &[]));
        let (index, _) = BlockHandle::decode(rest).unwrap_or((BlockHandle { offset: 0, size: 0 }, &[]));
        let handles = block_entries("t", &read_block("t", data, index)?)?;
        assert!(handles.len() > 1, "several data blocks");
        for (_, handle) in handles {
            let (handle, _) =
                BlockHandle::decode(&handle).unwrap_or((BlockHandle { offset: 0, size: 0 }, &[]));
            let kind = usize::try_from(handle.offset + handle.size).ok().and_then(|at| data.get(at)).copied();
            assert_eq!(kind, Some(SNAPPY));
        }
        // 400 items, VERSION, META and METAACCESS.
        assert_eq!(read_table("t", data)?.len(), 403);
        Ok(())
    }

    #[test]
    fn snappy_edge_cases() {
        // Empty input, literals only, long back-references and overlapping copies all round
        // trip through the decoder a table uses.
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"x".to_vec(),
            b"abcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabc".to_vec(),
            vec![0u8; 200_000],
            (0..70_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect(),
        ];
        for case in cases {
            let packed = snap::raw::Encoder::new().compress_vec(&case).unwrap_or_default();
            assert_eq!(snap::raw::Decoder::new().decompress_vec(&packed).ok(), Some(case));
        }
        // A copy that reaches before the start of the output is an error, not garbage.
        assert!(snap::raw::Decoder::new().decompress_vec(&[4, 0x01, 0x05]).is_err());
        // A declared length the data does not fill.
        assert!(snap::raw::Decoder::new().decompress_vec(&[10, 0x00, b'a']).is_err());
    }
}
