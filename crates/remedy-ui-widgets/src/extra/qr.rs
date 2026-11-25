//! QR Code generation (ISO/IEC 18004, Model 2): numeric, alphanumeric, byte and Kanji modes,
//! versions 1 to 40, error correction levels L, M, Q and H, and the mask chosen by penalty score.

use std::fmt;

use crate::extra::qr_segments::{Item, Segment, forced, items_of_bytes, items_of_text, optimal};

/// How much damage a code survives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// Exhaustive on purpose: the QR standard's four error correction levels.
pub enum Ecc {
    /// About 7 percent of the code can be lost.
    Low,
    /// About 15 percent.
    #[default]
    Medium,
    /// About 25 percent.
    Quartile,
    /// About 30 percent.
    High,
}

impl Ecc {
    /// All levels, weakest first.
    pub const ALL: [Ecc; 4] = [Ecc::Low, Ecc::Medium, Ecc::Quartile, Ecc::High];

    fn ordinal(self) -> usize {
        match self {
            Ecc::Low => 0,
            Ecc::Medium => 1,
            Ecc::Quartile => 2,
            Ecc::High => 3,
        }
    }

    /// The two bits the format information carries for this level.
    pub fn format_bits(self) -> u32 {
        match self {
            Ecc::Low => 1,
            Ecc::Medium => 0,
            Ecc::Quartile => 3,
            Ecc::High => 2,
        }
    }

    /// The level from its format bits.
    pub fn from_format_bits(bits: u32) -> Option<Ecc> {
        Ecc::ALL.into_iter().find(|e| e.format_bits() == bits)
    }

    /// The letter people know it by: L, M, Q or H.
    pub fn letter(self) -> char {
        match self {
            Ecc::Low => 'L',
            Ecc::Medium => 'M',
            Ecc::Quartile => 'Q',
            Ecc::High => 'H',
        }
    }

    /// About how much of the code can be lost, in percent.
    pub fn recovery_percent(self) -> u32 {
        match self {
            Ecc::Low => 7,
            Ecc::Medium => 15,
            Ecc::Quartile => 25,
            Ecc::High => 30,
        }
    }
}

/// Why a code could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QrError {
    /// The data does not fit in the largest version allowed at this error correction level.
    TooLong,
    /// The version range is empty or outside 1 to 40, or the mask is not 0 to 7.
    BadOptions,
}

impl fmt::Display for QrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QrError::TooLong => {
                f.write_str("the data is too long for a QR code at this error correction level")
            }
            QrError::BadOptions => f.write_str("the QR code options are out of range"),
        }
    }
}

impl std::error::Error for QrError {}

/// How data is written into the code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Exhaustive on purpose: the QR standard's four data modes.
pub enum Mode {
    /// Digits only: 10 bits for 3 digits.
    Numeric,
    /// Digits, capital letters and ` $%*+-./:`: 11 bits for 2 characters.
    Alphanumeric,
    /// Any bytes: 8 bits per byte. Text goes in as Latin-1 or, with an ECI header, UTF-8.
    Byte,
    /// Shift JIS double-byte characters (kanji, kana and symbols): 13 bits for each.
    Kanji,
}

impl Mode {
    pub(crate) fn indicator(self) -> u32 {
        match self {
            Mode::Numeric => 0b0001,
            Mode::Alphanumeric => 0b0010,
            Mode::Byte => 0b0100,
            Mode::Kanji => 0b1000,
        }
    }

    /// Bits in the character count field at `version`.
    pub(crate) fn count_bits(self, version: u8) -> usize {
        let band = match version {
            1..=9 => 0,
            10..=26 => 1,
            _ => 2,
        };
        let table: [usize; 3] = match self {
            Mode::Numeric => [10, 12, 14],
            Mode::Alphanumeric => [9, 11, 13],
            Mode::Byte => [8, 16, 16],
            Mode::Kanji => [8, 10, 12],
        };
        table.get(band).copied().unwrap_or(8)
    }

    /// The smallest mode that can hold `data`.
    pub fn best_for(data: &[u8]) -> Mode {
        if data.iter().all(u8::is_ascii_digit) {
            Mode::Numeric
        } else if data.iter().all(|b| alnum_value(*b).is_some()) {
            Mode::Alphanumeric
        } else {
            Mode::Byte
        }
    }
}

pub(crate) fn alnum_value(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some(u32::from(b - b'0')),
        b'A'..=b'Z' => Some(u32::from(b - b'A') + 10),
        b' ' => Some(36),
        b'$' => Some(37),
        b'%' => Some(38),
        b'*' => Some(39),
        b'+' => Some(40),
        b'-' => Some(41),
        b'.' => Some(42),
        b'/' => Some(43),
        b':' => Some(44),
        _ => None,
    }
}

/// What to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeOptions {
    /// The error correction level to start from.
    pub ecc: Ecc,
    /// Never use a version below this (1 to 40).
    pub min_version: u8,
    /// Never use a version above this (1 to 40).
    pub max_version: u8,
    /// Force one of the eight masks (0 to 7); `None` picks the one with the lowest penalty.
    pub mask: Option<u8>,
    /// Raise the error correction level while the version stays the same.
    pub boost_ecc: bool,
    /// Force one mode for all the data; `None` splits it into the segments that take the
    /// fewest bits, mixing modes where that is smaller.
    pub mode: Option<Mode>,
}

impl EncodeOptions {
    /// Sets `ecc`: the error correction level to start from.
    pub fn ecc(mut self, value: Ecc) -> Self {
        self.ecc = value;
        self
    }

    /// Sets `min_version`: never use a version below this (1 to 40).
    pub fn min_version(mut self, value: u8) -> Self {
        self.min_version = value;
        self
    }

    /// Sets `max_version`: never use a version above this (1 to 40).
    pub fn max_version(mut self, value: u8) -> Self {
        self.max_version = value;
        self
    }

    /// Sets `mask`: force one of the eight masks (0 to 7); `None` picks the one with the lowest penalty.
    pub fn mask(mut self, value: u8) -> Self {
        self.mask = Some(value);
        self
    }

    /// Sets `boost_ecc`: raise the error correction level while the version stays the same.
    pub fn boost_ecc(mut self, value: bool) -> Self {
        self.boost_ecc = value;
        self
    }

    /// Sets `mode`: force one mode for all the data; `None` splits it into the segments that take the fewest bits, mixing modes where that is smaller.
    pub fn mode(mut self, value: Mode) -> Self {
        self.mode = Some(value);
        self
    }
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions {
            ecc: Ecc::Medium,
            min_version: 1,
            max_version: 40,
            mask: None,
            boost_ecc: true,
            mode: None,
        }
    }
}

/// A finished QR code: a square of dark and light modules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCode {
    version: u8,
    ecc: Ecc,
    mask: u8,
    size: usize,
    modules: Vec<bool>,
}

// Error correction codewords per block and number of blocks, by level (L, M, Q, H) and
// version 1 to 40 (ISO/IEC 18004, table 9).
const ECC_PER_BLOCK: [[u8; 40]; 4] = [
    [
        7, 10, 15, 20, 26, 18, 20, 24, 30, 18, 20, 24, 26, 30, 22, 24, 28, 30, 28, 28, 28, 28, 30, 30, 26,
        28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
    [
        10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28,
        28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    ],
    [
        13, 22, 18, 26, 18, 24, 18, 22, 20, 24, 28, 26, 24, 20, 30, 24, 28, 28, 26, 30, 28, 30, 30, 30, 30,
        28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
    [
        17, 28, 22, 16, 22, 28, 26, 26, 24, 28, 24, 28, 22, 24, 24, 30, 28, 28, 26, 28, 30, 24, 30, 30, 30,
        30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
];

const BLOCKS: [[u8; 40]; 4] = [
    [
        1, 1, 1, 1, 1, 2, 2, 2, 2, 4, 4, 4, 4, 4, 6, 6, 6, 6, 7, 8, 8, 9, 9, 10, 12, 12, 12, 13, 14, 15, 16,
        17, 18, 19, 19, 20, 21, 22, 24, 25,
    ],
    [
        1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28,
        29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49,
    ],
    [
        1, 1, 2, 2, 4, 4, 6, 6, 8, 8, 8, 10, 12, 16, 12, 17, 16, 18, 21, 20, 23, 23, 25, 27, 29, 34, 34, 35,
        38, 40, 43, 45, 48, 51, 53, 56, 59, 62, 65, 68,
    ],
    [
        1, 1, 2, 4, 4, 4, 5, 6, 8, 8, 11, 11, 16, 16, 18, 16, 19, 21, 25, 25, 25, 34, 30, 32, 35, 37, 40, 42,
        45, 48, 51, 54, 57, 60, 63, 66, 70, 74, 77, 81,
    ],
];

fn table(t: &[[u8; 40]; 4], version: u8, ecc: Ecc) -> usize {
    t.get(ecc.ordinal())
        .and_then(|row| row.get(usize::from(version).wrapping_sub(1)))
        .copied()
        .map_or(0, usize::from)
}

/// The side of a version's square, in modules.
pub fn version_size(version: u8) -> usize {
    usize::from(version) * 4 + 17
}

/// Bits a version has for data, format and error correction codewords together (everything
/// that is not a fixed pattern).
fn raw_data_modules(version: u8) -> usize {
    let v = usize::from(version);
    let mut n = (16 * v + 128) * v + 64;
    if v >= 2 {
        let align = v / 7 + 2;
        n -= (25 * align - 10) * align - 55;
        if v >= 7 {
            n -= 36;
        }
    }
    n
}

/// Codewords (bytes) of data a version holds at an error correction level.
pub fn data_codewords(version: u8, ecc: Ecc) -> usize {
    raw_data_modules(version) / 8 - table(&ECC_PER_BLOCK, version, ecc) * table(&BLOCKS, version, ecc)
}

/// The number of blocks and the error correction codewords in each, at a version and level.
pub fn block_layout(version: u8, ecc: Ecc) -> (usize, usize) {
    (table(&BLOCKS, version, ecc), table(&ECC_PER_BLOCK, version, ecc))
}

/// The centers of a version's alignment patterns along one axis.
pub fn alignment_positions(version: u8) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let v = usize::from(version);
    let size = version_size(version);
    let count = v / 7 + 2;
    let step = if v == 32 { 26 } else { (v * 4 + count * 2 + 1) / (count * 2 - 2) * 2 };
    let mut out: Vec<usize> = (0..count - 1).map(|i| size - 7 - i * step).collect();
    out.push(6);
    out.reverse();
    out
}

// --- Reed-Solomon over GF(256) with the polynomial 0x11D ---

fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= ((u32::from(y) >> i) & 1) * u32::from(x);
    }
    (z & 0xFF) as u8
}

/// The generator polynomial of `degree`, without its leading 1 coefficient, highest power first.
pub fn rs_generator(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    if let Some(last) = result.last_mut() {
        *last = 1;
    }
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            let next = result.get(j + 1).copied().unwrap_or(0);
            if let Some(c) = result.get_mut(j) {
                *c = gf_mul(*c, root) ^ next;
            }
        }
        root = gf_mul(root, 0x02);
    }
    result
}

/// The Reed-Solomon error correction codewords for `data`.
pub fn rs_remainder(data: &[u8], generator: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; generator.len()];
    for b in data {
        let factor = b ^ result.first().copied().unwrap_or(0);
        if !result.is_empty() {
            result.remove(0);
        }
        result.push(0);
        for (r, g) in result.iter_mut().zip(generator) {
            *r ^= gf_mul(*g, factor);
        }
    }
    result
}

// --- Building the code ---

fn bit(v: u32, i: usize) -> bool {
    (v >> i) & 1 != 0
}

struct Matrix {
    size: usize,
    dark: Vec<bool>,
    func: Vec<bool>,
}

impl Matrix {
    fn new(size: usize) -> Matrix {
        Matrix { size, dark: vec![false; size * size], func: vec![false; size * size] }
    }

    fn at(&self, x: usize, y: usize) -> bool {
        self.dark.get(y * self.size + x).copied().unwrap_or(false)
    }

    fn is_func(&self, x: usize, y: usize) -> bool {
        self.func.get(y * self.size + x).copied().unwrap_or(true)
    }

    fn set_func(&mut self, x: i64, y: i64, dark: bool) {
        let n = self.size as i64;
        if (0..n).contains(&x) && (0..n).contains(&y) {
            let i = (y * n + x) as usize;
            if let Some(d) = self.dark.get_mut(i) {
                *d = dark;
            }
            if let Some(f) = self.func.get_mut(i) {
                *f = true;
            }
        }
    }

    fn flip(&mut self, x: usize, y: usize) {
        if let Some(d) = self.dark.get_mut(y * self.size + x) {
            *d = !*d;
        }
    }

    fn draw_patterns(&mut self, version: u8) {
        let n = self.size as i64;
        for i in 0..n {
            self.set_func(6, i, i % 2 == 0);
            self.set_func(i, 6, i % 2 == 0);
        }
        for (cx, cy) in [(3, 3), (n - 4, 3), (3, n - 4)] {
            for dy in -4i64..=4 {
                for dx in -4i64..=4 {
                    let dist = dx.abs().max(dy.abs());
                    self.set_func(cx + dx, cy + dy, dist != 2 && dist != 4);
                }
            }
        }
        let pos = alignment_positions(version);
        let last = pos.len().saturating_sub(1);
        for (i, &px) in pos.iter().enumerate() {
            for (j, &py) in pos.iter().enumerate() {
                // The three corners with finder patterns have none.
                if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                    continue;
                }
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        let dist = dx.abs().max(dy.abs());
                        self.set_func(px as i64 + dx, py as i64 + dy, dist != 1);
                    }
                }
            }
        }
        self.draw_format(Ecc::Low, 0);
        self.draw_version(version);
    }

    fn draw_format(&mut self, ecc: Ecc, mask: u8) {
        let n = self.size as i64;
        let bits = format_bits(ecc, mask);
        for i in 0..=5 {
            self.set_func(8, i, bit(bits, i as usize));
        }
        self.set_func(8, 7, bit(bits, 6));
        self.set_func(8, 8, bit(bits, 7));
        self.set_func(7, 8, bit(bits, 8));
        for i in 9..15 {
            self.set_func(14 - i, 8, bit(bits, i as usize));
        }
        for i in 0..8 {
            self.set_func(n - 1 - i, 8, bit(bits, i as usize));
        }
        for i in 8..15 {
            self.set_func(8, n - 15 + i, bit(bits, i as usize));
        }
        self.set_func(8, n - 8, true);
    }

    fn draw_version(&mut self, version: u8) {
        if version < 7 {
            return;
        }
        let bits = version_bits(version);
        let n = self.size as i64;
        for i in 0..18i64 {
            let b = bit(bits, i as usize);
            let a = n - 11 + i % 3;
            let c = i / 3;
            self.set_func(a, c, b);
            self.set_func(c, a, b);
        }
    }

    fn place_codewords(&mut self, data: &[u8]) {
        let n = self.size;
        let mut i = 0usize;
        let mut right = n - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..n {
                for j in 0..2 {
                    let x = right - j;
                    let upward = ((right + 1) & 2) == 0;
                    let y = if upward { n - 1 - vert } else { vert };
                    if !self.is_func(x, y) && i < data.len() * 8 {
                        let byte = data.get(i >> 3).copied().unwrap_or(0);
                        if let Some(d) = self.dark.get_mut(y * n + x) {
                            *d = bit(u32::from(byte), 7 - (i & 7));
                        }
                        i += 1;
                    }
                }
            }
            right = right.saturating_sub(2);
            if right == 0 {
                break;
            }
        }
    }

    fn apply_mask(&mut self, mask: u8) {
        for y in 0..self.size {
            for x in 0..self.size {
                if !self.is_func(x, y) && mask_bit(mask, x, y) {
                    self.flip(x, y);
                }
            }
        }
    }

    fn penalty(&self) -> u32 {
        let n = self.size;
        let mut score = 0u32;
        // Rows and columns: runs of five or more, and finder-like patterns.
        for horizontal in [true, false] {
            for a in 0..n {
                let line: Vec<bool> =
                    (0..n).map(|b| if horizontal { self.at(b, a) } else { self.at(a, b) }).collect();
                let mut run = 1u32;
                for w in line.windows(2) {
                    if let [p, q] = w {
                        if p == q {
                            run += 1;
                            if run == 5 {
                                score += 3;
                            } else if run > 5 {
                                score += 1;
                            }
                        } else {
                            run = 1;
                        }
                    }
                }
                const A: [bool; 11] =
                    [true, false, true, true, true, false, true, false, false, false, false];
                const B: [bool; 11] =
                    [false, false, false, false, true, false, true, true, true, false, true];
                for w in line.windows(11) {
                    if w == A || w == B {
                        score += 40;
                    }
                }
            }
        }
        // 2 by 2 blocks of one color.
        for y in 0..n.saturating_sub(1) {
            for x in 0..n.saturating_sub(1) {
                let c = self.at(x, y);
                if c == self.at(x + 1, y) && c == self.at(x, y + 1) && c == self.at(x + 1, y + 1) {
                    score += 3;
                }
            }
        }
        // Balance of dark and light.
        let dark = self.dark.iter().filter(|d| **d).count() as u32;
        let total = (n * n) as u32;
        let k = (dark * 20).abs_diff(total * 10).div_ceil(total).saturating_sub(1);
        score + k * 10
    }
}

fn mask_bit(mask: u8, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y).is_multiple_of(2),
        1 => y.is_multiple_of(2),
        2 => x.is_multiple_of(3),
        3 => (x + y).is_multiple_of(3),
        4 => (x / 3 + y / 2).is_multiple_of(2),
        5 => x * y % 2 + x * y % 3 == 0,
        6 => (x * y % 2 + x * y % 3).is_multiple_of(2),
        _ => ((x + y) % 2 + x * y % 3).is_multiple_of(2),
    }
}

/// The 15 format bits (error correction level and mask, BCH-protected and masked).
pub fn format_bits(ecc: Ecc, mask: u8) -> u32 {
    let data = ecc.format_bits() << 3 | u32::from(mask & 7);
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    (data << 10 | rem) ^ 0x5412
}

/// The 18 version information bits (versions 7 to 40).
pub fn version_bits(version: u8) -> u32 {
    let v = u32::from(version);
    let mut rem = v;
    for _ in 0..12 {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
    }
    v << 12 | rem
}

fn interleave(data: &[u8], version: u8, ecc: Ecc) -> Vec<u8> {
    let (blocks, ecc_len) = block_layout(version, ecc);
    let raw = raw_data_modules(version) / 8;
    let short_blocks = blocks - raw % blocks;
    let short_len = raw / blocks;
    let generator = rs_generator(ecc_len);
    let mut parts: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(blocks);
    let mut k = 0usize;
    for i in 0..blocks {
        let len = short_len - ecc_len + usize::from(i >= short_blocks);
        let chunk = data.get(k..k + len).unwrap_or(&[]);
        k += len;
        parts.push((chunk.to_vec(), rs_remainder(chunk, &generator)));
    }
    let mut out = Vec::with_capacity(raw);
    let data_len = short_len - ecc_len;
    for i in 0..=data_len {
        for (j, (d, _)) in parts.iter().enumerate() {
            // Short blocks have one data codeword fewer.
            if (i < data_len || j >= short_blocks)
                && let Some(b) = d.get(i)
            {
                out.push(*b);
            }
        }
    }
    for i in 0..ecc_len {
        for (_, e) in &parts {
            if let Some(b) = e.get(i) {
                out.push(*b);
            }
        }
    }
    out
}

/// The data bits of `segments` at `version`, with terminator and padding, as the `capacity`
/// data codewords.
fn data_codeword_bytes(segments: &[Segment], version: u8, capacity: usize) -> Vec<u8> {
    let mut bits = Vec::new();
    for seg in segments {
        seg.write(&mut bits, version);
    }
    let cap_bits = capacity * 8;
    let room = 4.min(cap_bits.saturating_sub(bits.len()));
    bits.extend(std::iter::repeat_n(false, room));
    while !bits.len().is_multiple_of(8) {
        bits.push(false);
    }
    let mut bytes: Vec<u8> =
        bits.chunks(8).map(|c| c.iter().fold(0u8, |a, b| (a << 1) | u8::from(*b))).collect();
    let mut pad = 0xECu8;
    while bytes.len() < capacity {
        bytes.push(pad);
        pad = if pad == 0xEC { 0x11 } else { 0xEC };
    }
    bytes
}

/// The most characters any code holds (digits, at version 40 and level L): data with more
/// cannot fit, whatever its modes, and is turned down before any work is done on it.
const MAX_CHARS: usize = 7089;

fn band_of(version: u8) -> usize {
    match version {
        1..=9 => 0,
        10..=26 => 1,
        _ => 2,
    }
}

impl QrCode {
    /// A code for `text` at the error correction level `ecc`, in the smallest version that
    /// fits, raising the level while the version stays the same. The text is split into the
    /// segments that take the fewest bits (digits, capitals, Kanji and bytes mixed as it
    /// pays). Text that is all in Latin-1 is written as Latin-1 bytes, which readers assume;
    /// any other text is written as UTF-8 after an ECI header (26) saying so.
    pub fn encode_text(text: &str, ecc: Ecc) -> Result<QrCode, QrError> {
        QrCode::encode_text_with(text, EncodeOptions { ecc, ..EncodeOptions::default() })
    }

    /// Like [`QrCode::encode_text`] with every choice in `options`.
    pub fn encode_text_with(text: &str, options: EncodeOptions) -> Result<QrCode, QrError> {
        if text.chars().nth(MAX_CHARS).is_some() {
            return Err(QrError::TooLong);
        }
        let (items, eci) = items_of_text(text);
        QrCode::from_items(&items, eci, options)
    }

    /// A code for `data` with every choice in `options`. The bytes are written as they are;
    /// runs of digits and capitals still use the smaller modes, and there is no ECI header.
    pub fn encode(data: &[u8], options: EncodeOptions) -> Result<QrCode, QrError> {
        if data.len() > MAX_CHARS {
            return Err(QrError::TooLong);
        }
        QrCode::from_items(&items_of_bytes(data), None, options)
    }

    /// A code for segments the caller made (`Segment::kanji`, `Segment::eci`, ...), in the
    /// order given.
    pub fn encode_segments(segments: &[Segment], options: EncodeOptions) -> Result<QrCode, QrError> {
        QrCode::build(options, |_| Some(segments.to_vec()))
    }

    fn from_items(items: &[Item], eci: Option<u32>, options: EncodeOptions) -> Result<QrCode, QrError> {
        QrCode::build(options, |version| match options.mode {
            Some(mode) => forced(items, eci, mode),
            None => Some(optimal(items, eci, version)),
        })
    }

    /// Chooses the smallest version the segments fit and draws the code.
    fn build(
        options: EncodeOptions,
        segments_for: impl Fn(u8) -> Option<Vec<Segment>>,
    ) -> Result<QrCode, QrError> {
        let EncodeOptions { ecc, min_version, max_version, mask, boost_ecc, .. } = options;
        if !(1..=40).contains(&min_version) || !(1..=40).contains(&max_version) || min_version > max_version {
            return Err(QrError::BadOptions);
        }
        if mask.is_some_and(|m| m > 7) {
            return Err(QrError::BadOptions);
        }
        // The split depends only on the width of the count fields, which change in three bands.
        let mut cache: [Option<Vec<Segment>>; 3] = [None, None, None];
        let mut chosen: Option<(u8, Vec<Segment>, usize)> = None;
        for v in min_version..=max_version {
            let slot = cache.get_mut(band_of(v)).ok_or(QrError::BadOptions)?;
            if slot.is_none() {
                *slot = Some(segments_for(v).ok_or(QrError::BadOptions)?);
            }
            let segs = slot.clone().unwrap_or_default();
            let bits: Option<usize> = segs.iter().map(|s| s.bit_len(v)).sum();
            if let Some(bits) = bits
                && bits <= data_codewords(v, ecc) * 8
            {
                chosen = Some((v, segs, bits));
                break;
            }
        }
        let (version, segments, used) = chosen.ok_or(QrError::TooLong)?;
        let mut ecc = ecc;
        if boost_ecc {
            for next in [Ecc::Medium, Ecc::Quartile, Ecc::High] {
                if next.ordinal() > ecc.ordinal() && used <= data_codewords(version, next) * 8 {
                    ecc = next;
                }
            }
        }
        let bytes = data_codeword_bytes(&segments, version, data_codewords(version, ecc));
        let all = interleave(&bytes, version, ecc);
        let mut m = Matrix::new(version_size(version));
        m.draw_patterns(version);
        m.place_codewords(&all);
        let best = match mask {
            Some(k) => k,
            None => {
                let mut best = (u32::MAX, 0u8);
                for k in 0..8u8 {
                    m.apply_mask(k);
                    m.draw_format(ecc, k);
                    let p = m.penalty();
                    m.apply_mask(k);
                    if p < best.0 {
                        best = (p, k);
                    }
                }
                best.1
            }
        };
        m.apply_mask(best);
        m.draw_format(ecc, best);
        Ok(QrCode { version, ecc, mask: best, size: m.size, modules: m.dark })
    }

    /// The version, 1 to 40.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// The error correction level.
    pub fn ecc(&self) -> Ecc {
        self.ecc
    }

    /// The mask used, 0 to 7.
    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// The side of the square, in modules (without any quiet zone).
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the module at column `x`, row `y` is dark; outside the square is light.
    pub fn module(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules.get(y * self.size + x).copied().unwrap_or(false)
    }

    /// The mask penalty score of this code (ISO/IEC 18004, 7.8.3): lower is easier to scan.
    /// The generator picks the mask with the lowest.
    pub fn penalty(&self) -> u32 {
        Matrix { size: self.size, dark: self.modules.clone(), func: Vec::new() }.penalty()
    }

    /// The code as lines of text: `██` for dark modules and two spaces for light ones, with a
    /// quiet zone of `border` modules.
    pub fn to_text(&self, border: usize) -> String {
        let mut s = String::new();
        let n = self.size + 2 * border;
        for y in 0..n {
            for x in 0..n {
                let dark = x >= border && y >= border && self.module(x - border, y - border);
                s.push_str(if dark { "\u{2588}\u{2588}" } else { "  " });
            }
            s.push('\n');
        }
        s
    }
}
