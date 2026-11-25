//! The QR generator, checked by decoding what it makes with a reader written for the test
//! from the standard (format information, unmasking, de-interleaving, Reed-Solomon syndromes
//! and the data stream), and against numbers the standard prints.

use remedy_ui_widgets::extra::{
    ECI_UTF8, Ecc, EncodeOptions, Mode, QrCode, QrError, Segment, alignment_positions, block_layout,
    data_codewords, format_bits, rs_generator, segment_text, version_bits, version_size,
};

fn at<T: Copy + Default>(v: &[T], i: usize) -> T {
    v.get(i).copied().unwrap_or_default()
}

// --- a reader built from the standard, apart from the generator ---

struct Gf {
    exp: Vec<u8>,
    log: Vec<usize>,
}

impl Gf {
    fn new() -> Gf {
        let mut exp = vec![0u8; 512];
        let mut log = vec![0usize; 256];
        let mut x = 1u32;
        for i in 0..255 {
            if let Some(e) = exp.get_mut(i) {
                *e = x as u8;
            }
            if let Some(l) = log.get_mut(x as usize) {
                *l = i;
            }
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11D;
            }
        }
        for i in 255..512 {
            let v = at(&exp, i - 255);
            if let Some(e) = exp.get_mut(i) {
                *e = v;
            }
        }
        Gf { exp, log }
    }

    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            at(&self.exp, at(&self.log, usize::from(a)) + at(&self.log, usize::from(b)))
        }
    }

    fn alpha(&self, power: usize) -> u8 {
        at(&self.exp, power % 255)
    }

    /// The codeword read as a polynomial, evaluated at alpha^k.
    fn eval(&self, word: &[u8], k: usize) -> u8 {
        let x = self.alpha(k);
        word.iter().fold(0u8, |acc, c| self.mul(acc, x) ^ c)
    }
}

fn function_map(version: u8) -> Vec<bool> {
    let n = version_size(version);
    let mut f = vec![false; n * n];
    let mut mark = |x: usize, y: usize| {
        if let Some(c) = f.get_mut(y * n + x) {
            *c = true;
        }
    };
    for y in 0..n {
        for x in 0..n {
            let corner = (x < 8 && y < 8) || (x >= n - 8 && y < 8) || (x < 8 && y >= n - 8);
            let format = (x == 8 && (y <= 8 || y >= n - 8)) || (y == 8 && (x <= 8 || x >= n - 8));
            if corner || format || x == 6 || y == 6 {
                mark(x, y);
            }
        }
    }
    let pos = alignment_positions(version);
    let last = pos.len().saturating_sub(1);
    for (i, &cx) in pos.iter().enumerate() {
        for (j, &cy) in pos.iter().enumerate() {
            if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                continue;
            }
            for y in cy - 2..=cy + 2 {
                for x in cx - 2..=cx + 2 {
                    mark(x, y);
                }
            }
        }
    }
    if version >= 7 {
        for a in 0..6 {
            for b in 0..3 {
                mark(n - 11 + b, a);
                mark(a, n - 11 + b);
            }
        }
    }
    f
}

/// Reads the format information from the first copy, checking its BCH code.
fn read_format(code: &QrCode) -> (Ecc, u8) {
    let get = |x: usize, y: usize| u32::from(code.module(x, y));
    let mut bits = 0u32;
    let places: Vec<(usize, usize)> =
        (0..6).map(|i| (8, i)).chain([(8, 7), (8, 8), (7, 8)]).chain((9..15).map(|i| (14 - i, 8))).collect();
    for (i, (x, y)) in places.into_iter().enumerate() {
        bits |= get(x, y) << i;
    }
    // The second copy says the same.
    let n = code.size();
    let mut second = 0u32;
    for i in 0..8 {
        second |= get(n - 1 - i, 8) << i;
    }
    for i in 8..15 {
        second |= get(8, n - 15 + i) << i;
    }
    assert_eq!(bits, second, "both copies of the format information agree");
    let data = (bits ^ 0x5412) >> 10;
    // The remainder of the 15 bits by the generator is zero.
    let mut rem = bits ^ 0x5412;
    for i in (10..15).rev() {
        if rem >> i & 1 != 0 {
            rem ^= 0x537 << (i - 10);
        }
    }
    assert_eq!(rem, 0, "the format BCH code checks");
    let ecc = Ecc::from_format_bits(data >> 3).unwrap_or_else(|| unreachable!("format ecc"));
    (ecc, (data & 7) as u8)
}

fn mask_at(mask: u8, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y).is_multiple_of(2),
        1 => y.is_multiple_of(2),
        2 => x.is_multiple_of(3),
        3 => (x + y).is_multiple_of(3),
        4 => (y / 2 + x / 3).is_multiple_of(2),
        5 => (x * y) % 2 + (x * y) % 3 == 0,
        6 => ((x * y) % 2 + (x * y) % 3).is_multiple_of(2),
        _ => (((x + y) % 2) + (x * y) % 3).is_multiple_of(2),
    }
}

struct Decoded {
    /// Data codewords of each block, then the error correction codewords of each block.
    blocks: Vec<(Vec<u8>, Vec<u8>)>,
    payload: Vec<u8>,
    parsed: Parsed,
}

fn decode(code: &QrCode) -> Decoded {
    let version = code.version();
    let n = code.size();
    assert_eq!(n, version_size(version));
    let (ecc, mask) = read_format(code);
    assert_eq!((ecc, mask), (code.ecc(), code.mask()));
    if version >= 7 {
        // Both copies of the version information.
        let mut a = 0u32;
        let mut b = 0u32;
        for i in 0..18usize {
            a |= u32::from(code.module(n - 11 + i % 3, i / 3)) << i;
            b |= u32::from(code.module(i / 3, n - 11 + i % 3)) << i;
        }
        assert_eq!(a, version_bits(version));
        assert_eq!(b, version_bits(version));
    }
    let func = function_map(version);
    // The data modules, in the standard's zigzag order, unmasked.
    let mut bits: Vec<bool> = Vec::new();
    let mut right = n as i64 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for v in 0..n {
            for j in 0..2 {
                let x = (right - j) as usize;
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { n - 1 - v } else { v };
                if !at(&func, y * n + x) {
                    bits.push(code.module(x, y) ^ mask_at(mask, x, y));
                }
            }
        }
        right -= 2;
    }
    let bytes: Vec<u8> = bits
        .chunks(8)
        .filter(|c| c.len() == 8)
        .map(|c| c.iter().fold(0u8, |a, b| (a << 1) | u8::from(*b)))
        .collect();
    let (blocks_n, ecc_len) = block_layout(version, ecc);
    let total_codewords = bytes.len();
    // Whatever bits are left over are the remainder bits, all zero.
    assert!(bits.len() - total_codewords * 8 < 8);
    assert!(bits.iter().skip(total_codewords * 8).all(|b| !b), "remainder bits are zero");
    let short = blocks_n - total_codewords % blocks_n;
    let short_len = total_codewords / blocks_n;
    let data_per_short = short_len - ecc_len;
    let mut datas: Vec<Vec<u8>> = vec![Vec::new(); blocks_n];
    let mut k = 0;
    for i in 0..=data_per_short {
        for (j, d) in datas.iter_mut().enumerate() {
            if i < data_per_short || j >= short {
                d.push(at(&bytes, k));
                k += 1;
            }
        }
    }
    let mut eccs: Vec<Vec<u8>> = vec![Vec::new(); blocks_n];
    for _ in 0..ecc_len {
        for e in eccs.iter_mut() {
            e.push(at(&bytes, k));
            k += 1;
        }
    }
    assert_eq!(k, total_codewords);
    // Each block with its error correction is a multiple of the generator: every syndrome is zero.
    let gf = Gf::new();
    for (d, e) in datas.iter().zip(&eccs) {
        let word: Vec<u8> = d.iter().chain(e.iter()).copied().collect();
        for s in 0..ecc_len {
            assert_eq!(gf.eval(&word, s), 0, "syndrome {s} of a block");
        }
    }
    let all: Vec<u8> = datas.iter().flatten().copied().collect();
    assert_eq!(all.len(), data_codewords(version, ecc));
    let parsed = parse(&all, version);
    Decoded { blocks: datas.into_iter().zip(eccs).collect(), payload: parsed.bytes.clone(), parsed }
}

/// What the data stream says: the raw bytes of the byte, digit and capital segments, the text
/// a reader would show, and the segments' mode indicators and ECI designators in order.
struct Parsed {
    bytes: Vec<u8>,
    text: String,
    modes: Vec<u32>,
    ecis: Vec<u32>,
}

/// Reads the data stream: segments until the terminator, then zeros and the pad bytes.
fn parse(data: &[u8], version: u8) -> Parsed {
    let bits: Vec<bool> = data.iter().flat_map(|b| (0..8).rev().map(move |i| (b >> i) & 1 != 0)).collect();
    let pos = std::cell::Cell::new(0usize);
    let take = |count: usize| -> u32 {
        let mut v = 0u32;
        for _ in 0..count {
            v = (v << 1) | u32::from(bits.get(pos.get()).copied().unwrap_or(false));
            pos.set(pos.get() + 1);
        }
        v
    };
    let band = match version {
        1..=9 => 0,
        10..=26 => 1,
        _ => 2,
    };
    let count_bits = |m: u32| {
        at(
            &match m {
                1 => [10usize, 12, 14],
                2 => [9, 11, 13],
                8 => [8, 10, 12],
                _ => [8, 16, 16],
            },
            band,
        )
    };
    const ALNUM: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
    let mut out = Parsed { bytes: Vec::new(), text: String::new(), modes: Vec::new(), ecis: Vec::new() };
    // The bytes of a byte segment mean what the last ECI says (Latin-1 until then).
    let mut utf8 = false;
    let mut pending: Vec<u8> = Vec::new();
    let flush = |pending: &mut Vec<u8>, utf8: bool, text: &mut String| {
        if utf8 {
            text.push_str(&String::from_utf8_lossy(pending));
        } else {
            text.extend(pending.iter().map(|b| char::from(*b)));
        }
        pending.clear();
    };
    while bits.len() - pos.get() >= 4 {
        let mode = take(4);
        if mode == 0 {
            break;
        }
        out.modes.push(mode);
        if mode == 7 {
            // An ECI designator of one, two or three bytes.
            let first = take(8);
            let d = if first & 0x80 == 0 {
                first
            } else if first & 0xC0 == 0x80 {
                (first & 0x3F) << 8 | take(8)
            } else {
                (first & 0x1F) << 16 | take(16)
            };
            out.ecis.push(d);
            flush(&mut pending, utf8, &mut out.text);
            utf8 = d == 26;
            continue;
        }
        let count = take(count_bits(mode)) as usize;
        match mode {
            1 => {
                let mut left = count;
                while left > 0 {
                    let take_n = left.min(3);
                    let v = take(take_n * 3 + 1);
                    let s = format!("{v:0width$}", width = take_n);
                    out.bytes.extend(s.bytes());
                    pending.extend(s.bytes());
                    left -= take_n;
                }
            }
            2 => {
                let mut left = count;
                while left > 0 {
                    if left >= 2 {
                        let v = take(11) as usize;
                        for b in [at(ALNUM, v / 45), at(ALNUM, v % 45)] {
                            out.bytes.push(b);
                            pending.push(b);
                        }
                        left -= 2;
                    } else {
                        let b = at(ALNUM, take(6) as usize);
                        out.bytes.push(b);
                        pending.push(b);
                        left -= 1;
                    }
                }
            }
            4 => {
                for _ in 0..count {
                    let b = take(8) as u8;
                    out.bytes.push(b);
                    pending.push(b);
                }
            }
            8 => {
                flush(&mut pending, utf8, &mut out.text);
                let mut sjis = Vec::new();
                for _ in 0..count {
                    let v = take(13);
                    let assembled = (v / 0xC0) << 8 | (v % 0xC0);
                    let code = if assembled < 0x1F00 { assembled + 0x8140 } else { assembled + 0xC140 };
                    sjis.extend([(code >> 8) as u8, code as u8]);
                }
                let (text, _, bad) = encoding_rs::SHIFT_JIS.decode(&sjis);
                assert!(!bad, "valid Shift JIS");
                out.text.push_str(&text);
            }
            m => unreachable!("mode {m}"),
        }
    }
    flush(&mut pending, utf8, &mut out.text);
    // After the terminator: zeros to the byte, and alternating pad bytes.
    while !pos.get().is_multiple_of(8) {
        assert_eq!(take(1), 0, "zeros up to the byte");
    }
    let mut pad = 0xECu32;
    while pos.get() < bits.len() {
        assert_eq!(take(8), pad, "pad bytes alternate EC and 11");
        pad ^= 0xEC ^ 0x11;
    }
    out
}

/// Bytes that do not repeat in any pattern the masks could favor.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(12345);
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as u8
        })
        .collect()
}

// --- tests ---

#[test]
fn the_standards_example_encodes_to_the_codewords_it_prints() {
    // ISO/IEC 18004 Annex I: "01234567", version 1, level M.
    let code = QrCode::encode(b"01234567", {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Medium;
        v_.min_version = 1;
        v_.max_version = 1;
        v_.boost_ecc = false;
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(code.version(), 1);
    let d = decode(&code);
    assert_eq!(d.blocks.len(), 1);
    let (data, ecc) = d.blocks.first().cloned().unwrap_or_default();
    assert_eq!(data, [16, 32, 12, 86, 97, 128, 236, 17, 236, 17, 236, 17, 236, 17, 236, 17]);
    assert_eq!(ecc, [165, 36, 212, 193, 237, 54, 199, 135, 44, 85]);
    assert_eq!(d.payload, b"01234567");
}

#[test]
fn the_generator_polynomial_matches_the_standard() {
    // x^7 + a^87 x^6 + a^229 x^5 + a^146 x^4 + a^149 x^3 + a^238 x^2 + a^102 x + a^21 (annex A).
    let gf = Gf::new();
    let g = rs_generator(7);
    let want: Vec<u8> = [87, 229, 146, 149, 238, 102, 21].iter().map(|p| gf.alpha(*p)).collect();
    assert_eq!(g, want);
    assert_eq!(rs_generator(10).len(), 10);
    assert_eq!(rs_generator(30).len(), 30);
}

#[test]
fn capacities_match_the_standards_tables_for_every_version_and_level() {
    #[rustfmt::skip]
    let l = [19,34,55,80,108,136,156,194,232,274,324,370,428,461,523,589,647,721,795,861,932,1006,1094,1174,1276,1370,1468,1531,1631,1735,1843,1955,2071,2191,2306,2434,2566,2702,2812,2956];
    #[rustfmt::skip]
    let m = [16,28,44,64,86,108,124,154,182,216,254,290,334,365,415,453,507,563,627,669,714,782,860,914,1000,1062,1128,1193,1267,1373,1455,1541,1631,1725,1812,1914,1992,2102,2216,2334];
    #[rustfmt::skip]
    let q = [13,22,34,48,62,76,88,110,132,154,180,206,244,261,295,325,367,397,445,485,512,568,614,664,718,754,808,871,911,985,1033,1115,1171,1231,1286,1354,1426,1502,1582,1666];
    #[rustfmt::skip]
    let h = [9,16,26,36,46,60,66,86,100,122,140,158,180,197,223,253,283,313,341,385,406,442,464,514,538,596,628,661,701,745,793,845,901,961,986,1054,1096,1142,1222,1276];
    for v in 1..=40u8 {
        let i = usize::from(v) - 1;
        assert_eq!(data_codewords(v, Ecc::Low), at(&l, i), "L{v}");
        assert_eq!(data_codewords(v, Ecc::Medium), at(&m, i), "M{v}");
        assert_eq!(data_codewords(v, Ecc::Quartile), at(&q, i), "Q{v}");
        assert_eq!(data_codewords(v, Ecc::High), at(&h, i), "H{v}");
    }
}

#[test]
fn alignment_pattern_centers_match_the_standard() {
    assert_eq!(alignment_positions(1), Vec::<usize>::new());
    assert_eq!(alignment_positions(2), [6, 18]);
    assert_eq!(alignment_positions(7), [6, 22, 38]);
    assert_eq!(alignment_positions(14), [6, 26, 46, 66]);
    assert_eq!(alignment_positions(32), [6, 34, 60, 86, 112, 138]);
    assert_eq!(alignment_positions(40), [6, 30, 58, 86, 114, 142, 170]);
}

#[test]
fn format_and_version_information_match_the_standard() {
    assert_eq!(format_bits(Ecc::Medium, 0), 0x5412);
    assert_eq!(format_bits(Ecc::Low, 0), 0x77C4);
    assert_eq!(format_bits(Ecc::Quartile, 0), 0x355F);
    assert_eq!(format_bits(Ecc::High, 0), 0x1689);
    assert_eq!(version_bits(7), 0x07C94);
    assert_eq!(version_bits(8), 0x085BC);
    assert_eq!(version_bits(40), 0x28C69);
}

#[test]
fn every_version_and_level_decodes_when_filled_to_the_last_byte() {
    for v in 1..=40u8 {
        for ecc in Ecc::ALL {
            // The most bytes that version holds at this level.
            let count_bits = if v < 10 { 8 } else { 16 };
            let len = (data_codewords(v, ecc) * 8 - 4 - count_bits) / 8;
            let data = noise(len, u32::from(v) * 7 + ecc.format_bits());
            let options = {
                let mut v_ = EncodeOptions::default();
                v_.ecc = ecc;
                v_.min_version = v;
                v_.max_version = v;
                v_.boost_ecc = false;
                v_
            };
            let code = QrCode::encode(&data, options).unwrap_or_else(|e| unreachable!("v{v} {ecc:?}: {e}"));
            assert_eq!((code.version(), code.ecc()), (v, ecc));
            let d = decode(&code);
            assert_eq!(d.payload, data, "version {v} level {ecc:?}");
        }
    }
}

#[test]
fn the_smallest_version_that_fits_is_chosen() {
    let c = QrCode::encode(&noise(17, 1), {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Low;
        v_.boost_ecc = false;
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(c.version(), 1, "17 bytes fit version 1-L");
    let c = QrCode::encode(&noise(18, 1), {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Low;
        v_.boost_ecc = false;
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(c.version(), 2, "18 bytes need version 2");
    let c = QrCode::encode(&noise(2953, 1), {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Low;
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(c.version(), 40);
    assert_eq!(c.size(), 177);
}

#[test]
fn data_too_long_is_an_error_not_a_crash() {
    assert_eq!(
        QrCode::encode(&noise(2954, 1), {
            let mut v_ = EncodeOptions::default();
            v_.ecc = Ecc::Low;
            v_
        }),
        Err(QrError::TooLong)
    );
    assert_eq!(
        QrCode::encode(&noise(2000, 1), {
            let mut v_ = EncodeOptions::default();
            v_.ecc = Ecc::High;
            v_
        }),
        Err(QrError::TooLong)
    );
    assert_eq!(
        QrCode::encode(b"x", {
            let mut v_ = EncodeOptions::default();
            v_.min_version = 0;
            v_
        }),
        Err(QrError::BadOptions)
    );
    assert_eq!(
        QrCode::encode(b"x", {
            let mut v_ = EncodeOptions::default();
            v_.max_version = 41;
            v_
        }),
        Err(QrError::BadOptions)
    );
    assert_eq!(
        QrCode::encode(b"x", {
            let mut v_ = EncodeOptions::default();
            v_.min_version = 5;
            v_.max_version = 4;
            v_
        }),
        Err(QrError::BadOptions)
    );
    assert_eq!(
        QrCode::encode(b"x", {
            let mut v_ = EncodeOptions::default();
            v_.mask = Some(8);
            v_
        }),
        Err(QrError::BadOptions)
    );
    assert_eq!(
        QrCode::encode(b"12a", {
            let mut v_ = EncodeOptions::default();
            v_.mode = Some(Mode::Numeric);
            v_
        }),
        Err(QrError::BadOptions)
    );
    // The version range is a hard limit.
    assert_eq!(
        QrCode::encode(&noise(100, 1), {
            let mut v_ = EncodeOptions::default();
            v_.max_version = 2;
            v_
        }),
        Err(QrError::TooLong)
    );
}

#[test]
fn modes_are_chosen_by_what_the_data_is() {
    assert_eq!(Mode::best_for(b"0123456789"), Mode::Numeric);
    assert_eq!(Mode::best_for(b"HELLO WORLD $%*+-./:"), Mode::Alphanumeric);
    assert_eq!(Mode::best_for(b"hello"), Mode::Byte);
    assert_eq!(Mode::best_for("h\u{e9}llo".as_bytes()), Mode::Byte);
    assert_eq!(Mode::best_for(b""), Mode::Numeric);
    for text in [
        "",
        "7",
        "42",
        "123456789012345",
        "HELLO WORLD",
        "A",
        "AC-42",
        "Hello, QR!",
        "h\u{e9}llo \u{2603} \u{1F600}",
    ] {
        let code = QrCode::encode_text(text, Ecc::Quartile).unwrap_or_else(|e| unreachable!("{text}: {e}"));
        assert_eq!(decode(&code).payload, text.as_bytes(), "{text:?}");
    }
    // Digits take far fewer bits than bytes: 100 digits fit version 3-L numerically but 100 bytes do not.
    let digits: Vec<u8> = (0..100).map(|i| b'0' + (i % 10) as u8).collect();
    let numeric = QrCode::encode(&digits, {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Low;
        v_.boost_ecc = false;
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    let bytes = QrCode::encode(&digits, {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Low;
        v_.boost_ecc = false;
        v_.mode = Some(Mode::Byte);
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert!(numeric.version() < bytes.version());
    assert_eq!(decode(&numeric).payload, digits);
    assert_eq!(decode(&bytes).payload, digits);
}

#[test]
fn the_mask_with_the_lowest_penalty_is_chosen() {
    for text in ["https://remedy.example/pair?code=482913", "HELLO WORLD", &"a".repeat(300)] {
        let auto = QrCode::encode_text(text, Ecc::Medium).unwrap_or_else(|e| unreachable!("{e}"));
        let mut lowest = u32::MAX;
        for mask in 0..8u8 {
            let forced = QrCode::encode(text.as_bytes(), {
                let mut v_ = EncodeOptions::default();
                v_.ecc = auto.ecc();
                v_.min_version = auto.version();
                v_.max_version = auto.version();
                v_.mask = Some(mask);
                v_.boost_ecc = false;
                v_
            })
            .unwrap_or_else(|e| unreachable!("{e}"));
            assert_eq!(forced.mask(), mask);
            assert_eq!(decode(&forced).payload, text.as_bytes(), "mask {mask} decodes");
            lowest = lowest.min(forced.penalty());
        }
        assert_eq!(auto.penalty(), lowest, "{text:?}: mask {} has the lowest penalty", auto.mask());
    }
}

#[test]
fn error_correction_is_raised_while_the_version_stays() {
    let c = QrCode::encode_text("hi", Ecc::Low).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(c.version(), 1);
    assert_eq!(c.ecc(), Ecc::High, "two bytes leave room for the strongest level");
    let c = QrCode::encode(b"hi", {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Low;
        v_.boost_ecc = false;
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(c.ecc(), Ecc::Low);
}

#[test]
fn fixed_patterns_are_where_the_standard_puts_them() {
    let c = QrCode::encode_text("fixed", Ecc::Medium).unwrap_or_else(|e| unreachable!("{e}"));
    let n = c.size();
    // Finder pattern: ring of dark, ring of light, 3x3 dark center, light separator.
    for (ox, oy) in [(0, 0), (n - 7, 0), (0, n - 7)] {
        for dy in 0..7usize {
            for dx in 0..7usize {
                let ring = dx.min(6 - dx).min(dy.min(6 - dy));
                let want = ring != 1;
                assert_eq!(c.module(ox + dx, oy + dy), want, "finder at {ox},{oy} cell {dx},{dy}");
            }
        }
    }
    // The module that is always dark.
    assert!(c.module(8, n - 8));
    // Timing patterns alternate.
    for i in 8..n - 8 {
        assert_eq!(c.module(i, 6), i % 2 == 0);
        assert_eq!(c.module(6, i), i % 2 == 0);
    }
    // Outside is light, and the text form has a quiet zone.
    assert!(!c.module(n, 0) && !c.module(0, n));
    let text = c.to_text(2);
    assert_eq!(text.lines().count(), n + 4);
    assert!(text.lines().next().is_some_and(|l| l.chars().all(|ch| ch == ' ')));
}

#[test]
fn the_same_input_always_makes_the_same_code() {
    let a = QrCode::encode_text("repeatable", Ecc::Quartile).unwrap_or_else(|e| unreachable!("{e}"));
    let b = QrCode::encode_text("repeatable", Ecc::Quartile).unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(a, b);
    let c = QrCode::encode_text("repeatablf", Ecc::Quartile).unwrap_or_else(|e| unreachable!("{e}"));
    assert_ne!(a, c);
}

// --- mixed segments, Kanji and ECI ---

fn total_bits(segs: &[Segment], version: u8) -> usize {
    segs.iter().map(|s| s.bit_len(version).unwrap_or(usize::MAX / 8)).sum()
}

#[test]
fn kanji_values_match_the_standards_examples() {
    let value = |text: &str| -> u32 {
        let seg = Segment::kanji(text).unwrap_or_else(|| unreachable!("kanji {text}"));
        assert_eq!(seg.mode(), Some(Mode::Kanji));
        assert_eq!(seg.data_bits().len(), 13 * text.chars().count());
        seg.data_bits().iter().fold(0u32, |a, b| (a << 1) | u32::from(*b))
    };
    // ISO/IEC 18004 7.4.6: the character with Shift JIS 0x935F is 0x0D9F, and 0xE4AA is 0x1AAA.
    assert_eq!(value("\u{70b9}"), 0x0D9F);
    assert_eq!(value("\u{8317}"), 0x1AAA);
    assert!(Segment::kanji("a").is_none() && Segment::kanji("\u{70b9}x").is_none());
    assert!(Segment::kanji("\u{1F600}").is_none(), "no Shift JIS for emoji");
    assert!(Segment::numeric("12a").is_none() && Segment::alphanumeric("abc").is_none());
    assert_eq!(Segment::numeric("0123456789").map(|s| s.data_bits().len()), Some(34));
    assert_eq!(Segment::alphanumeric("AC-42").map(|s| s.data_bits().len()), Some(28));
}

#[test]
fn japanese_text_goes_in_kanji_mode_and_decodes() {
    let text = "\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30ad}\u{30b9}\u{30c8}";
    let code = QrCode::encode_text(text, Ecc::Low).unwrap_or_else(|e| unreachable!("{e}"));
    let d = decode(&code);
    // One Kanji segment after the ECI-free header: no UTF-8 bytes at all.
    assert_eq!(d.parsed.modes, [8]);
    assert!(d.parsed.ecis.is_empty());
    assert_eq!(d.parsed.text, text);
    // It is smaller than the same text as UTF-8 bytes.
    let utf8 = QrCode::encode_text_with(text, {
        let mut v_ = EncodeOptions::default();
        v_.ecc = Ecc::Low;
        v_.mode = Some(Mode::Byte);
        v_.boost_ecc = false;
        v_
    })
    .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(decode(&utf8).parsed.text, text);
    let kanji_bits = total_bits(&segment_text(text, 1), 1);
    let byte_bits = total_bits(&[Segment::eci(ECI_UTF8), Segment::bytes(text.as_bytes())], 1);
    assert!(kanji_bits < byte_bits, "{kanji_bits} against {byte_bits}");
    // Forced Kanji works for Kanji text and is refused for anything else.
    let forced = QrCode::encode_text_with(text, {
        let mut v_ = EncodeOptions::default();
        v_.mode = Some(Mode::Kanji);
        v_
    });
    assert!(forced.is_ok());
    assert_eq!(
        QrCode::encode_text_with("abc", {
            let mut v_ = EncodeOptions::default();
            v_.mode = Some(Mode::Kanji);
            v_
        }),
        Err(QrError::BadOptions)
    );
}

#[test]
fn text_outside_latin_1_gets_a_utf8_eci_header() {
    for text in ["snowman \u{2603}", "\u{645}\u{631}\u{62d}\u{628}\u{627}", "emoji \u{1F600} ok", "\u{20ac}5"]
    {
        let code = QrCode::encode_text(text, Ecc::Medium).unwrap_or_else(|e| unreachable!("{e}"));
        let d = decode(&code);
        assert_eq!(d.parsed.ecis, [26], "{text}");
        assert_eq!(d.parsed.text, text);
        assert_eq!(d.payload, text.as_bytes(), "the bytes are UTF-8");
        assert_eq!(d.parsed.modes.first(), Some(&7), "the ECI header comes first");
    }
    // Latin-1 text needs no header: readers assume it, and each character is one byte.
    let d = decode(
        &QrCode::encode_text("caf\u{e9} \u{fc}ber", Ecc::Medium).unwrap_or_else(|e| unreachable!("{e}")),
    );
    assert!(d.parsed.ecis.is_empty());
    assert_eq!(d.payload, [b'c', b'a', b'f', 0xE9, b' ', 0xFC, b'b', b'e', b'r']);
    assert_eq!(d.parsed.text, "caf\u{e9} \u{fc}ber");
    // The header is 12 bits: mode, then one byte.
    assert_eq!(Segment::eci(26).bit_len(1), Some(12));
    assert_eq!(Segment::eci(26).eci_designator(), Some(26));
    assert_eq!(Segment::eci(26).mode(), None);
}

#[test]
fn eci_designators_of_every_size_are_written_and_read() {
    for (designator, bits) in
        [(3u32, 12usize), (26, 12), (127, 12), (128, 20), (1000, 20), (16383, 20), (16384, 28), (999_999, 28)]
    {
        let segs = [Segment::eci(designator), Segment::bytes(b"hi")];
        assert_eq!(segs.first().and_then(|s| s.bit_len(1)), Some(bits), "designator {designator}");
        let code =
            QrCode::encode_segments(&segs, EncodeOptions::default()).unwrap_or_else(|e| unreachable!("{e}"));
        let d = decode(&code);
        assert_eq!(d.parsed.ecis, [designator]);
        assert_eq!(d.payload, b"hi");
    }
}

#[test]
fn mixed_modes_take_fewer_bits_than_any_one_mode() {
    let text = "ORDER 20240915-0042 / total 1234567890 EUR";
    let best = segment_text(text, 1);
    let modes: Vec<Option<Mode>> = best.iter().map(Segment::mode).collect();
    assert!(modes.contains(&Some(Mode::Numeric)) && modes.contains(&Some(Mode::Byte)), "{modes:?}");
    let bits = total_bits(&best, 1);
    assert!(bits < total_bits(&[Segment::bytes(text.as_bytes())], 1), "smaller than all bytes");
    let code = QrCode::encode_text(text, Ecc::Low).unwrap_or_else(|e| unreachable!("{e}"));
    let d = decode(&code);
    assert_eq!(d.parsed.text, text);
    assert!(d.parsed.modes.len() >= 3, "{:?}", d.parsed.modes);
    // A known split: capitals then digits beat all-alphanumeric.
    let t = "ABCDEFGH12345678901234567890";
    let modes: Vec<Option<Mode>> = segment_text(t, 1).iter().map(Segment::mode).collect();
    assert_eq!(modes, [Some(Mode::Alphanumeric), Some(Mode::Numeric)]);
    assert!(
        total_bits(&segment_text(t, 1), 1)
            < total_bits(&[Segment::alphanumeric(t).unwrap_or_else(|| unreachable!())], 1)
    );
    // Where one mode is best, the split is that mode alone.
    assert_eq!(
        segment_text("0123456789", 1).iter().map(Segment::mode).collect::<Vec<_>>(),
        [Some(Mode::Numeric)]
    );
    assert_eq!(segment_text("hello", 1).iter().map(Segment::mode).collect::<Vec<_>>(), [Some(Mode::Byte)]);
    // Mixed Kanji and ASCII.
    let mixed = "\u{6771}\u{4eac}\u{6771}\u{4eac}\u{6771}\u{4eac}Tokyo 2020";
    let d = decode(&QrCode::encode_text(mixed, Ecc::Medium).unwrap_or_else(|e| unreachable!("{e}")));
    assert_eq!(d.parsed.text, mixed);
    assert!(d.parsed.modes.contains(&8) && d.parsed.modes.len() >= 2, "{:?}", d.parsed.modes);
}

/// The cheapest split found by trying every assignment of modes (for short texts).
fn brute_force(chars: &[char], version: u8, eci: bool) -> usize {
    let n = chars.len();
    let modes = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte, Mode::Kanji];
    let mut best = usize::MAX;
    for code in 0..4usize.pow(n as u32) {
        let assign: Vec<Mode> = (0..n)
            .map(|i| modes.get(code / 4usize.pow(i as u32) % 4).copied().unwrap_or(Mode::Byte))
            .collect();
        let mut segs = Vec::new();
        let mut ok = true;
        let mut i = 0;
        while i < n {
            let m = assign.get(i).copied().unwrap_or(Mode::Byte);
            let mut j = i;
            while j < n && assign.get(j) == Some(&m) {
                j += 1;
            }
            let run: String = chars.get(i..j).unwrap_or(&[]).iter().collect();
            let seg = match m {
                Mode::Numeric => Segment::numeric(&run),
                Mode::Alphanumeric => Segment::alphanumeric(&run),
                Mode::Byte => Some(Segment::bytes(run.as_bytes())),
                Mode::Kanji => Segment::kanji(&run),
            };
            match seg {
                Some(s) => segs.push(s),
                None => {
                    ok = false;
                    break;
                }
            }
            i = j;
        }
        if ok {
            // Bytes that are not ASCII need the ECI header (12 bits).
            let needs = eci && assign.iter().zip(chars).any(|(m, c)| *m == Mode::Byte && !c.is_ascii());
            best = best.min(total_bits(&segs, version) + if needs { 12 } else { 0 });
        }
    }
    best
}

#[test]
fn the_split_is_the_cheapest_there_is() {
    // Digits, capitals, lower case and kanji, so every mode has a use; texts without
    // characters outside Latin-1 stay byte-per-character (the brute force uses UTF-8 bytes,
    // so it is only compared where each character is one byte).
    let pool: Vec<char> = "0123456789ABCDEFXYZ $%abcxyz".chars().collect();
    let mut seed = 12345u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for len in 1..=7usize {
        for _ in 0..25 {
            let chars: Vec<char> =
                (0..len).map(|_| at(&pool, (next() % pool.len() as u32) as usize)).collect();
            let text: String = chars.iter().collect();
            for version in [1u8, 10, 27] {
                let got = total_bits(&segment_text(&text, version), version);
                assert_eq!(got, brute_force(&chars, version, false), "{text:?} at version {version}");
            }
        }
    }
    // With kanji in the pool (each kanji is one char in Kanji mode, three UTF-8 bytes otherwise).
    let kanji = ['\u{65e5}', '\u{672c}', '\u{8a9e}'];
    for _ in 0..30 {
        let chars: Vec<char> = (0..6)
            .map(|_| {
                if next() % 2 == 0 {
                    at(&kanji, (next() % 3) as usize)
                } else {
                    at(&pool, (next() % pool.len() as u32) as usize)
                }
            })
            .collect();
        let text: String = chars.iter().collect();
        // Not Latin-1, so non-ASCII bytes would need the ECI header: 12 bits.
        let got = total_bits(&segment_text(&text, 1), 1);
        assert_eq!(got, brute_force(&chars, 1, true), "{text:?}");
    }
}
