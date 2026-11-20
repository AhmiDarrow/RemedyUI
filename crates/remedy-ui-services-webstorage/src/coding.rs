//! LevelDB's integer encodings and its checksum: varints, little-endian fixed-width integers
//! and masked CRC-32C.

/// Reads a varint of at most `max_bytes` bytes (5 for 32 bits, 10 for 64), returning the
/// value and the bytes after it. `None` when the input ends inside the varint, the varint is
/// longer than `max_bytes`, or its value does not fit.
fn varint(input: &[u8], max_bytes: usize, bits: u32) -> Option<(u64, &[u8])> {
    let mut value: u64 = 0;
    for (i, &byte) in input.iter().enumerate().take(max_bytes) {
        let shift = 7 * u32::try_from(i).ok()?;
        let part = u64::from(byte & 0x7f);
        // The last byte may only carry the bits that are left.
        if shift + 7 > bits && part >> (bits - shift) != 0 {
            return None;
        }
        value |= part << shift;
        if byte & 0x80 == 0 {
            return Some((value, input.get(i + 1..)?));
        }
    }
    None
}

/// A varint32 and the bytes after it.
pub(crate) fn varint32(input: &[u8]) -> Option<(u32, &[u8])> {
    let (v, rest) = varint(input, 5, 32)?;
    Some((u32::try_from(v).ok()?, rest))
}

/// A varint64 and the bytes after it.
pub(crate) fn varint64(input: &[u8]) -> Option<(u64, &[u8])> {
    varint(input, 10, 64)
}

/// A varint32 length followed by that many bytes: the bytes and what comes after.
pub(crate) fn length_prefixed(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let (len, rest) = varint32(input)?;
    rest.split_at_checked(usize::try_from(len).ok()?)
}

/// A little-endian u32 at the start of `input`.
pub(crate) fn fixed32(input: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(input.get(..4)?.try_into().ok()?))
}

/// A little-endian u64 at the start of `input`.
pub(crate) fn fixed64(input: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(input.get(..8)?.try_into().ok()?))
}

/// The CRC-32C (Castagnoli) table, reflected polynomial 0x82F63B78.
#[allow(clippy::indexing_slicing, reason = "i runs over the table's own 256 entries")]
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0x82F6_3B78 } else { crc >> 1 };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC-32C of the concatenation of `parts`.
pub(crate) fn crc32c(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for part in parts {
        for &byte in *part {
            let index = usize::from((crc as u8) ^ byte);
            crc = (crc >> 8) ^ CRC_TABLE.get(index).copied().unwrap_or(0);
        }
    }
    !crc
}

/// LevelDB stores CRCs masked, so that a CRC of data that itself holds CRCs stays well mixed.
pub(crate) fn unmask_crc(masked: u32) -> u32 {
    let rot = masked.wrapping_sub(0xa282_ead8);
    rot.rotate_left(15)
}

#[cfg(test)]
pub(crate) fn mask_crc(crc: u32) -> u32 {
    crc.rotate_right(15).wrapping_add(0xa282_ead8)
}

#[cfg(test)]
pub(crate) fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip_at_every_width() {
        for v in [0u64, 1, 127, 128, 255, 300, 16_383, 16_384, u64::from(u32::MAX), u64::MAX] {
            let mut buf = Vec::new();
            put_varint(&mut buf, v);
            buf.push(0xAB);
            assert_eq!(varint64(&buf), Some((v, &[0xAB][..])));
            if let Ok(small) = u32::try_from(v) {
                assert_eq!(varint32(&buf), Some((small, &[0xAB][..])));
            } else {
                assert_eq!(varint32(&buf), None, "{v} does not fit 32 bits");
            }
        }
    }

    #[test]
    fn truncated_and_overlong_varints_are_rejected() {
        assert_eq!(varint32(&[]), None);
        assert_eq!(varint32(&[0x80]), None);
        assert_eq!(varint32(&[0xff, 0xff, 0xff, 0xff, 0x7f]), None, "35 bits");
        assert_eq!(varint32(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]), None, "six bytes");
        assert_eq!(varint64(&[0xff; 10]), None);
        assert_eq!(varint64(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]), None);
        assert_eq!(
            varint64(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]),
            Some((u64::MAX, &[][..]))
        );
    }

    #[test]
    fn length_prefixed_slices() {
        assert_eq!(length_prefixed(&[3, b'a', b'b', b'c', b'd']), Some((&b"abc"[..], &b"d"[..])));
        assert_eq!(length_prefixed(&[4, b'a', b'b', b'c']), None);
    }

    #[test]
    fn fixed_width() {
        assert_eq!(fixed32(&[1, 2, 3, 4, 5]), Some(0x0403_0201));
        assert_eq!(fixed32(&[1, 2, 3]), None);
        assert_eq!(fixed64(&[1, 0, 0, 0, 0, 0, 0, 0x80]), Some(0x8000_0000_0000_0001));
    }

    #[test]
    fn crc32c_matches_the_standard_vectors() {
        // RFC 3720 (iSCSI) test vectors, as LevelDB's own crc32c tests use.
        assert_eq!(crc32c(&[&[0u8; 32]]), 0x8a91_36aa);
        assert_eq!(crc32c(&[&[0xffu8; 32]]), 0x62a8_ab43);
        let ascending: Vec<u8> = (0..32).collect();
        assert_eq!(crc32c(&[&ascending]), 0x46dd_794e);
        assert_eq!(crc32c(&[b"123456789"]), 0xe306_9283);
        assert_eq!(crc32c(&[b"1234", b"56789"]), 0xe306_9283, "split input");
        let crc = crc32c(&[b"foo"]);
        assert_ne!(mask_crc(crc), crc);
        assert_eq!(unmask_crc(mask_crc(crc)), crc);
    }
}
