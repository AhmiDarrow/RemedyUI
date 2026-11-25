//! QR data segments: the modes (numeric, alphanumeric, byte, Kanji), ECI headers, and the
//! split of a text into the segments that take the fewest bits (ISO/IEC 18004 annex J).

use crate::extra::qr::{Mode, alnum_value};

/// The ECI designator for UTF-8, which tells readers how to turn byte segments into text.
pub const ECI_UTF8: u32 = 26;

/// What a segment is: data in one of the modes, or an ECI header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Data(Mode),
    Eci(u32),
}

/// One run of data in one mode (or an ECI header). A code is a list of segments; mixing modes
/// lets digits, capitals, Kanji and bytes each take the fewest bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    kind: Kind,
    /// The number the count field holds: digits, characters or bytes, as the mode counts.
    count: usize,
    bits: Vec<bool>,
}

fn push(bits: &mut Vec<bool>, value: u32, count: usize) {
    for i in (0..count).rev() {
        bits.push((value >> i) & 1 != 0);
    }
}

/// The Shift JIS double byte of `c` as the 13-bit Kanji mode value, when the character has one.
fn kanji_value(c: char) -> Option<u16> {
    let mut buf = [0u8; 4];
    let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(c.encode_utf8(&mut buf));
    if unmappable {
        return None;
    }
    let [hi, lo] = *bytes else { return None };
    let v = u32::from(hi) << 8 | u32::from(lo);
    let adjusted = match v {
        0x8140..=0x9FFC => v - 0x8140,
        0xE040..=0xEBBF => v - 0xC140,
        _ => return None,
    };
    // The low byte must be a real trail byte (0x40 to 0xFC, not 0x7F).
    if !(0x40..=0xFC).contains(&lo) || lo == 0x7F {
        return None;
    }
    Some(((adjusted >> 8) * 0xC0 + (adjusted & 0xFF)) as u16)
}

impl Segment {
    /// A numeric segment; `None` if `digits` has anything but ASCII digits.
    pub fn numeric(digits: &str) -> Option<Segment> {
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut bits = Vec::new();
        for chunk in digits.as_bytes().chunks(3) {
            let v = chunk.iter().fold(0u32, |a, d| a * 10 + u32::from(d - b'0'));
            push(&mut bits, v, chunk.len() * 3 + 1);
        }
        Some(Segment { kind: Kind::Data(Mode::Numeric), count: digits.len(), bits })
    }

    /// An alphanumeric segment (digits, capitals and ` $%*+-./:`); `None` for anything else.
    pub fn alphanumeric(text: &str) -> Option<Segment> {
        let values: Option<Vec<u32>> = text.bytes().map(alnum_value).collect();
        let mut bits = Vec::new();
        for pair in values?.chunks(2) {
            match pair {
                [a, b] => push(&mut bits, a * 45 + b, 11),
                [a] => push(&mut bits, *a, 6),
                _ => {}
            }
        }
        Some(Segment { kind: Kind::Data(Mode::Alphanumeric), count: text.len(), bits })
    }

    /// A byte segment: any bytes, written as they are.
    pub fn bytes(data: &[u8]) -> Segment {
        let mut bits = Vec::new();
        for b in data {
            push(&mut bits, u32::from(*b), 8);
        }
        Segment { kind: Kind::Data(Mode::Byte), count: data.len(), bits }
    }

    /// A Kanji segment: every character must be a Shift JIS double-byte character (JIS X 0208
    /// kanji, kana and symbols); `None` if one is not.
    pub fn kanji(text: &str) -> Option<Segment> {
        let mut bits = Vec::new();
        let mut count = 0;
        for c in text.chars() {
            push(&mut bits, u32::from(kanji_value(c)?), 13);
            count += 1;
        }
        Some(Segment { kind: Kind::Data(Mode::Kanji), count, bits })
    }

    /// An ECI header: what the byte segments after it mean. 26 is UTF-8, 3 is Latin-1,
    /// 20 is Shift JIS.
    pub fn eci(designator: u32) -> Segment {
        let mut bits = Vec::new();
        if designator < 1 << 7 {
            push(&mut bits, designator, 8);
        } else if designator < 1 << 14 {
            push(&mut bits, 0b10 << 14 | designator, 16);
        } else {
            push(&mut bits, 0b110 << 21 | designator.min(999_999), 24);
        }
        Segment { kind: Kind::Eci(designator), count: 0, bits }
    }

    /// The mode of a data segment; `None` for an ECI header.
    pub fn mode(&self) -> Option<Mode> {
        match self.kind {
            Kind::Data(m) => Some(m),
            Kind::Eci(_) => None,
        }
    }

    /// The designator of an ECI header.
    pub fn eci_designator(&self) -> Option<u32> {
        match self.kind {
            Kind::Eci(d) => Some(d),
            Kind::Data(_) => None,
        }
    }

    /// How many digits, characters or bytes the count field says (0 for an ECI header).
    pub fn count(&self) -> usize {
        self.count
    }

    /// The bits the segment takes at `version`, header included; `None` when its count does
    /// not fit the version's count field.
    pub fn bit_len(&self, version: u8) -> Option<usize> {
        match self.kind {
            Kind::Eci(_) => Some(4 + self.bits.len()),
            Kind::Data(m) => {
                let width = m.count_bits(version);
                ((self.count as u64) < (1u64 << width)).then_some(4 + width + self.bits.len())
            }
        }
    }

    /// Writes the segment at `version` onto `out`.
    pub(crate) fn write(&self, out: &mut Vec<bool>, version: u8) {
        match self.kind {
            Kind::Eci(_) => push(out, 0b0111, 4),
            Kind::Data(m) => {
                push(out, m.indicator(), 4);
                push(out, self.count as u32, m.count_bits(version));
            }
        }
        out.extend_from_slice(&self.bits);
    }
}

/// One character of the input, with every way it can be written.
#[derive(Clone, Debug)]
pub(crate) struct Item {
    digit: bool,
    alnum: bool,
    kanji: Option<u16>,
    bytes: Vec<u8>,
    ch: char,
}

/// The items of a piece of text, and the ECI header its bytes need (UTF-8 when some character
/// is not in Latin-1; otherwise bytes are Latin-1, which readers assume with no header).
pub(crate) fn items_of_text(text: &str) -> (Vec<Item>, Option<u32>) {
    let latin1 = text.chars().all(|c| u32::from(c) < 256);
    let items = text
        .chars()
        .map(|c| {
            let ascii = c.is_ascii();
            let bytes = if latin1 { vec![c as u32 as u8] } else { c.to_string().into_bytes() };
            Item {
                digit: c.is_ascii_digit(),
                alnum: ascii && alnum_value(c as u8).is_some(),
                kanji: if ascii { None } else { kanji_value(c) },
                bytes,
                ch: c,
            }
        })
        .collect();
    (items, (!latin1).then_some(ECI_UTF8))
}

/// The items of raw bytes: each is one byte, and digits and capitals can still use the
/// smaller modes. Never Kanji.
pub(crate) fn items_of_bytes(data: &[u8]) -> Vec<Item> {
    data.iter()
        .map(|b| Item {
            digit: b.is_ascii_digit(),
            alnum: alnum_value(*b).is_some(),
            kanji: None,
            bytes: vec![*b],
            ch: char::from(*b),
        })
        .collect()
}

const MODES: [Mode; 4] = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte, Mode::Kanji];

/// Whether `mode` can hold `item`.
fn eligible(item: &Item, mode: Mode, ascii_bytes: bool) -> bool {
    match mode {
        Mode::Numeric => item.digit,
        Mode::Alphanumeric => item.alnum,
        Mode::Byte => !ascii_bytes || item.ch.is_ascii(),
        Mode::Kanji => item.kanji.is_some(),
    }
}

/// The states of the search: a mode and how far into a group of digits (threes) or capitals
/// (pairs) the segment is, since a partial group costs what its length says.
const STATES: [(Mode, u8); 7] = [
    (Mode::Numeric, 0),
    (Mode::Numeric, 1),
    (Mode::Numeric, 2),
    (Mode::Alphanumeric, 0),
    (Mode::Alphanumeric, 1),
    (Mode::Byte, 0),
    (Mode::Kanji, 0),
];

/// Writing one more item in `mode` after `phase` items of its group: the new phase and the
/// extra bits (the 10 bits of three digits arrive as 4, 3 and 3; 11 for a pair as 6 and 5).
fn step(item: &Item, mode: Mode, phase: u8) -> (u8, u32) {
    match mode {
        Mode::Numeric => ((phase + 1) % 3, [4, 3, 3].get(usize::from(phase)).copied().unwrap_or(3)),
        Mode::Alphanumeric => ((phase + 1) % 2, [6, 5].get(usize::from(phase)).copied().unwrap_or(5)),
        Mode::Byte => (0, 8 * item.bytes.len() as u32),
        Mode::Kanji => (0, 13),
    }
}

/// Builds the segment for `items` in `mode`.
fn build(mode: Mode, items: &[Item]) -> Option<Segment> {
    match mode {
        Mode::Numeric => Segment::numeric(&items.iter().map(|i| i.ch).collect::<String>()),
        Mode::Alphanumeric => Segment::alphanumeric(&items.iter().map(|i| i.ch).collect::<String>()),
        Mode::Byte => {
            let bytes: Vec<u8> = items.iter().flat_map(|i| i.bytes.iter().copied()).collect();
            Some(Segment::bytes(&bytes))
        }
        Mode::Kanji => {
            let mut bits = Vec::new();
            for i in items {
                push(&mut bits, u32::from(i.kanji?), 13);
            }
            Some(Segment { kind: Kind::Data(Mode::Kanji), count: items.len(), bits })
        }
    }
}

/// Groups items given the mode of each into segments.
fn group(items: &[Item], modes: &[Mode]) -> Option<Vec<Segment>> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < items.len() {
        let mode = *modes.get(start)?;
        let mut end = start + 1;
        while modes.get(end) == Some(&mode) {
            end += 1;
        }
        out.push(build(mode, items.get(start..end)?)?);
        start = end;
    }
    Some(out)
}

/// The split of `items` into segments that takes the fewest bits at `version`: a search over
/// the mode of each character that pays a header whenever the mode changes and counts a
/// partial group of digits or capitals exactly. `eci` puts an ECI header first.
pub(crate) fn optimal(items: &[Item], eci: Option<u32>, version: u8) -> Vec<Segment> {
    // Bytes that are not ASCII need the ECI header to mean UTF-8. Text that can do without
    // (its other characters are Kanji, digits or capitals) saves the header's 12 bits, so
    // both are tried and the smaller wins.
    let Some(d) = eci else { return split(items, version, false).unwrap_or_default() };
    let with = split(items, version, false).map(|mut v| {
        v.insert(0, Segment::eci(d));
        v
    });
    let without = split(items, version, true);
    let bits =
        |v: &Vec<Segment>| -> usize { v.iter().map(|s| s.bit_len(version).unwrap_or(usize::MAX / 8)).sum() };
    match (with, without) {
        (Some(a), Some(b)) => {
            if bits(&b) <= bits(&a) {
                b
            } else {
                a
            }
        }
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => Vec::new(),
    }
}

/// The cheapest segments; with `ascii_bytes`, byte segments may hold only ASCII. `None` when
/// some character has no mode allowed.
fn split(items: &[Item], version: u8, ascii_bytes: bool) -> Option<Vec<Segment>> {
    let header = |m: Mode| (4 + m.count_bits(version)) as u32;
    // best[i][state]: the fewest bits for items up to i ending in that state, and the state before.
    let mut best: Vec<[Option<(u32, usize)>; 7]> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let mut row: [Option<(u32, usize)>; 7] = [None; 7];
        for m in &MODES {
            if !eligible(item, *m, ascii_bytes) {
                continue;
            }
            // Each earlier state this one can follow (none at all for the first item).
            let mut from: Vec<(u32, u8, usize, bool)> = Vec::new();
            if i == 0 {
                from.push((0, 0, usize::MAX, true));
            } else if let Some(prev) = best.get(i - 1) {
                for (si, (sm, sphase)) in STATES.iter().enumerate() {
                    let Some((c, _)) = prev.get(si).copied().flatten() else { continue };
                    from.push((c, if *sm == *m { *sphase } else { 0 }, si, *sm != *m));
                }
            }
            for (c, phase, si, new_segment) in from {
                let (after, bits) = step(item, *m, phase);
                let total = c + bits + if new_segment { header(*m) } else { 0 };
                let Some(target) = STATES.iter().position(|(sm, sp)| sm == m && *sp == after) else {
                    continue;
                };
                if let Some(slot) = row.get_mut(target)
                    && slot.is_none_or(|(b, _)| total < b)
                {
                    *slot = Some((total, si));
                }
            }
        }
        best.push(row);
    }
    if items.is_empty() {
        // An empty text is an empty byte segment: some readers want a segment.
        return Some(vec![Segment::bytes(&[])]);
    }
    let last = best
        .last()
        .and_then(|r| r.iter().enumerate().filter_map(|(si, c)| c.map(|(cost, _)| (cost, si))).min())?;
    let mut modes = vec![Mode::Byte; items.len()];
    let mut st = last.1;
    for i in (0..items.len()).rev() {
        if let Some(slot) = modes.get_mut(i) {
            *slot = STATES.get(st).map_or(Mode::Byte, |s| s.0);
        }
        st = best.get(i).and_then(|r| r.get(st).copied().flatten()).map_or(st, |(_, p)| p);
    }
    group(items, &modes)
}

/// All the items in one mode, as a caller asked; `None` if some item cannot be written in it.
pub(crate) fn forced(items: &[Item], eci: Option<u32>, mode: Mode) -> Option<Vec<Segment>> {
    let mut out = Vec::new();
    if let Some(d) = eci {
        out.push(Segment::eci(d));
    }
    if items.is_empty() {
        out.push(Segment::bytes(&[]));
    } else {
        out.extend(group(items, &vec![mode; items.len()])?);
    }
    Some(out)
}

/// The split into segments that takes the fewest bits at `version` for `text`, with the ECI
/// header first when the text is not Latin-1. This is what `QrCode::encode_text` writes.
pub fn segment_text(text: &str, version: u8) -> Vec<Segment> {
    let (items, eci) = items_of_text(text);
    optimal(&items, eci, version)
}

impl Segment {
    /// The data bits after the mode indicator and count field (all of an ECI header's bits).
    pub fn data_bits(&self) -> &[bool] {
        &self.bits
    }
}
