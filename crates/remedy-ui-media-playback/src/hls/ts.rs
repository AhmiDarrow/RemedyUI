//! Taking an MPEG-TS segment apart: the programme tables, the H.264 and AAC or MP3 streams'
//! packets (PES) put back together, and what the codecs' own headers say (an H.264 sequence
//! parameter set, an ADTS or MP3 frame header). Untrusted input: every length is checked
//! against the data, and what is not understood is skipped.

use std::collections::HashMap;

/// MPEG-TS clocks count at 90 kHz and are 33 bits wide.
pub(crate) const CLOCK: u64 = 90_000;
const WRAP: u64 = 1 << 33;

/// What an elementary stream holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stream {
    H264,
    /// AAC in ADTS frames.
    Aac,
    Mp3,
}

/// One PES packet's payload: an H.264 access unit, or some audio frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pes {
    pub stream: Stream,
    /// 33-bit presentation time, when the packet has one.
    pub pts: Option<u64>,
    /// 33-bit decoding time (the presentation time when the packet gives only that).
    pub dts: Option<u64>,
    pub data: Vec<u8>,
}

/// The PES packets of one segment, in the order they arrive. ffmpeg's segments each start with
/// the programme tables and whole packets, so each is read on its own.
// shortcut: a PAT or PMT section must fit in one TS packet (ffmpeg's do); upgrade if a server splits them.
pub(crate) fn demux(data: &[u8]) -> Vec<Pes> {
    let mut pmt_pid: Option<u16> = None;
    let mut streams: HashMap<u16, (Stream, Vec<u8>)> = HashMap::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 188 <= data.len() {
        let Some(p) = data.get(i..).and_then(<[u8]>::first_chunk::<188>).filter(|p| p[0] == 0x47) else {
            // Lost sync: find the next packet start.
            i += 1;
            continue;
        };
        i += 188;
        let start = p[1] & 0x40 != 0;
        let pid = u16::from(p[1] & 0x1f) << 8 | u16::from(p[2]);
        let control = (p[3] >> 4) & 3;
        if control & 1 == 0 {
            continue;
        }
        let at = if control & 2 != 0 { 5 + usize::from(p[4]) } else { 4 };
        let Some(payload) = p.get(at..) else { continue };
        if pid == 0 || Some(pid) == pmt_pid {
            if !start {
                continue;
            }
            let pointer = payload.first().map_or(0, |b| usize::from(*b));
            let Some(section) = payload.get(1 + pointer..) else { continue };
            if pid == 0 {
                pmt_pid = pat(section).or(pmt_pid);
            } else {
                for (stream_pid, kind) in pmt(section) {
                    streams.entry(stream_pid).or_insert((kind, Vec::new()));
                }
            }
        } else if let Some((kind, buf)) = streams.get_mut(&pid) {
            if start && !buf.is_empty() {
                out.extend(pes(*kind, buf));
                buf.clear();
            }
            if start || !buf.is_empty() {
                buf.extend_from_slice(payload);
            }
        }
    }
    let mut rest: Vec<(u16, (Stream, Vec<u8>))> = streams.into_iter().collect();
    rest.sort_by_key(|(pid, _)| *pid);
    for (_, (kind, buf)) in rest {
        out.extend(pes(kind, &buf));
    }
    out
}

/// The PMT's PID from a PAT section (the first programme's).
fn pat(s: &[u8]) -> Option<u16> {
    let h = s.first_chunk::<3>().filter(|h| h[0] == 0)?;
    s.get(8..section_end(h, s.len()))?
        .as_chunks::<4>()
        .0
        .iter()
        .find(|e| e[0] != 0 || e[1] != 0)
        .map(|e| u16::from(e[2] & 0x1f) << 8 | u16::from(e[3]))
}

/// Where a section's entries end (before its CRC), within the `len` bytes there are.
fn section_end(h: &[u8; 3], len: usize) -> usize {
    (3 + (usize::from(h[1] & 0x0f) << 8 | usize::from(h[2]))).saturating_sub(4).min(len)
}

/// The streams a PMT section lists that are read here.
fn pmt(s: &[u8]) -> Vec<(u16, Stream)> {
    let mut out = Vec::new();
    let Some(h) = s.first_chunk::<12>().filter(|h| h[0] == 2) else { return out };
    let end = section_end(&[h[0], h[1], h[2]], s.len());
    let mut i = 12 + (usize::from(h[10] & 0x0f) << 8 | usize::from(h[11]));
    while let Some(e) = s.get(i..end).and_then(<[u8]>::first_chunk::<5>) {
        let pid = u16::from(e[1] & 0x1f) << 8 | u16::from(e[2]);
        let info = usize::from(e[3] & 0x0f) << 8 | usize::from(e[4]);
        let kind = match e[0] {
            0x1b => Some(Stream::H264),
            0x0f => Some(Stream::Aac),
            0x03 | 0x04 => Some(Stream::Mp3),
            _ => None,
        };
        out.extend(kind.map(|k| (pid, k)));
        i += 5 + info;
    }
    out
}

/// A whole PES packet: its times and payload.
fn pes(stream: Stream, b: &[u8]) -> Option<Pes> {
    let h = b.first_chunk::<9>().filter(|h| h.starts_with(&[0, 0, 1]))?;
    let flags = h[7] >> 6;
    let header = 9 + usize::from(h[8]);
    let at = |from: usize| b.get(from..).and_then(<[u8]>::first_chunk::<5>).map(timestamp);
    let pts = if flags & 2 != 0 { at(9) } else { None };
    let dts = if flags == 3 { at(14) } else { pts };
    Some(Pes { stream, pts, dts, data: b.get(header..)?.to_vec() })
}

fn timestamp(b: &[u8; 5]) -> u64 {
    u64::from(b[0] >> 1 & 7) << 30
        | u64::from(b[1]) << 22
        | u64::from(b[2] >> 1) << 15
        | u64::from(b[3]) << 7
        | u64::from(b[4] >> 1)
}

/// The 33-bit `raw` as a count that goes on past the wrap: the one nearest `near` (the last
/// time seen, already unwrapped).
pub(crate) fn unwrap(near: u64, raw: u64) -> u64 {
    let base = near & !(WRAP - 1);
    [base.checked_sub(WRAP), Some(base), base.checked_add(WRAP)]
        .into_iter()
        .flatten()
        .filter_map(|b| b.checked_add(raw))
        .min_by_key(|t| t.abs_diff(near))
        .unwrap_or(raw)
}

/// The NAL units of an H.264 access unit (Annex B), without their start codes, up to its first
/// slice: the parameter sets and SEI before it whole, and the slice as the rest of the data,
/// not scanned. All that is asked of an access unit is known by then (whether it is a key
/// frame, its SPS and PPS), and the slices are nearly all of its bytes.
pub(crate) fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let next = |from: usize| data.get(from..)?.windows(3).position(|w| w == [0, 0, 1]).map(|p| from + p + 3);
    let mut out = Vec::new();
    let mut at = next(0);
    while let Some(s) = at {
        if data.get(s).is_some_and(|b| (1..=5).contains(&(b & 0x1f))) {
            out.extend(data.get(s..));
            break;
        }
        at = next(s);
        let mut end = at.map_or(data.len(), |n| n - 3);
        // The zero before a four-byte start code belongs to it.
        while end > s && at.is_some() && data.get(end - 1) == Some(&0) {
            end -= 1;
        }
        out.extend(data.get(s..end));
    }
    out
}

/// What an H.264 sequence parameter set says about the picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sps {
    /// `avc1.PPCCLL`.
    pub codec: String,
    pub width: u32,
    pub height: u32,
}

struct Bits {
    data: Vec<u8>,
    pos: usize,
}

impl Bits {
    fn bit(&mut self) -> Option<u32> {
        let b = self.data.get(self.pos / 8)?;
        let v = u32::from(b >> (7 - self.pos % 8) & 1);
        self.pos += 1;
        Some(v)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let mut v = 0u64;
        for _ in 0..zeros {
            v = v << 1 | u64::from(self.bit()?);
        }
        u32::try_from((1u64 << zeros) - 1 + v).ok()
    }
    fn se(&mut self) -> Option<i64> {
        let k = i64::from(self.ue()?);
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }
}

/// Reads an SPS NAL unit (with its one-byte header).
pub(crate) fn parse_sps(nal: &[u8]) -> Option<Sps> {
    let h = nal.first_chunk::<4>().filter(|h| h[0] & 0x1f == 7)?;
    let codec = format!("avc1.{:02x}{:02x}{:02x}", h[1], h[2], h[3]);
    // Emulation-prevention bytes (00 00 03) out.
    let mut data = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in nal.get(4..)? {
        if b == 3 && zeros >= 2 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        data.push(b);
    }
    let mut r = Bits { data, pos: 0 };
    r.ue()?;
    let mut chroma = 1;
    let mut separate = false;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&h[1]) {
        chroma = r.ue()?;
        if chroma == 3 {
            separate = r.bit()? == 1;
        }
        r.ue()?;
        r.ue()?;
        r.bit()?;
        if r.bit()? == 1 {
            for i in 0..if chroma == 3 { 12 } else { 8 } {
                if r.bit()? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i64, 8i64);
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + r.se()? + 256) % 256;
                        }
                        if next != 0 {
                            last = next;
                        }
                    }
                }
            }
        }
    }
    r.ue()?;
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bit()?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?;
    r.bit()?;
    let w_mbs = r.ue()?.checked_add(1)?;
    let h_units = r.ue()?.checked_add(1)?;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 {
        r.bit()?;
    }
    r.bit()?;
    let mut width = w_mbs.checked_mul(16)?;
    let mut height = (2 - frame_mbs_only).checked_mul(h_units)?.checked_mul(16)?;
    if r.bit()? == 1 {
        let (l, rt, t, b) = (r.ue()?, r.ue()?, r.ue()?, r.ue()?);
        let (cx, cy) = match (separate, chroma) {
            (true, _) | (_, 0) => (1, 2 - frame_mbs_only),
            (_, 1) => (2, 2 * (2 - frame_mbs_only)),
            (_, 2) => (2, 2 - frame_mbs_only),
            _ => (1, 2 - frame_mbs_only),
        };
        width = width.checked_sub(l.checked_add(rt)?.checked_mul(cx)?)?;
        height = height.checked_sub(t.checked_add(b)?.checked_mul(cy)?)?;
    }
    Some(Sps { codec, width, height })
}

/// What an AAC ADTS header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Adts {
    /// The MPEG-4 audio object type (2 for AAC-LC).
    pub object_type: u8,
    pub rate_index: u8,
    pub sample_rate: u32,
    pub channels: u8,
}

impl Adts {
    /// The two-byte AudioSpecificConfig decoders are set up with.
    pub fn asc(&self) -> Vec<u8> {
        vec![self.object_type << 3 | self.rate_index >> 1, (self.rate_index & 1) << 7 | self.channels << 3]
    }
}

const AAC_RATES: [u32; 13] =
    [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

/// The raw AAC frames in ADTS data, each with its header.
pub(crate) fn adts_frames(data: &[u8]) -> Vec<(Adts, &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rest) = data.get(i..)
        && let Some(h) = rest.first_chunk::<7>()
    {
        if h[0] != 0xff || h[1] & 0xf6 != 0xf0 {
            i += 1;
            continue;
        }
        let header = if h[1] & 1 == 1 { 7 } else { 9 };
        let len = usize::from(h[3] & 3) << 11 | usize::from(h[4]) << 3 | usize::from(h[5] >> 5);
        let rate_index = h[2] >> 2 & 0xf;
        let (Some(&sample_rate), Some(frame)) =
            (AAC_RATES.get(usize::from(rate_index)), rest.get(header..len.max(header)))
        else {
            break;
        };
        let adts = Adts {
            object_type: (h[2] >> 6) + 1,
            rate_index,
            sample_rate,
            channels: (h[2] & 1) << 2 | h[3] >> 6,
        };
        out.push((adts, frame));
        i += len.max(header);
    }
    out
}

/// What an MP3 (MPEG audio Layer III) frame header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mp3 {
    /// The whole frame, header included, in bytes.
    pub size: usize,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: u32,
}

/// Reads an MP3 frame header; `None` for anything but Layer III.
pub(crate) fn mp3_header(h: &[u8]) -> Option<Mp3> {
    let h = h.first_chunk::<4>().filter(|h| h[0] == 0xff && h[1] & 0xe0 == 0xe0 && h[1] >> 1 & 3 == 1)?;
    let version = h[1] >> 3 & 3; // 3: MPEG-1, 2: MPEG-2, 0: MPEG-2.5
    let v1 = version == 3;
    let kbps: [u32; 15] = if v1 {
        [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320]
    } else {
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
    };
    let bitrate = *kbps.get(usize::from(h[2] >> 4)).filter(|b| **b != 0)? * 1000;
    let base = *[44100, 48000, 32000].get(usize::from(h[2] >> 2 & 3))?;
    let sample_rate = match version {
        3 => base,
        2 => base / 2,
        0 => base / 4,
        _ => return None,
    };
    let samples = if v1 { 1152 } else { 576 };
    let size = (samples / 8 * bitrate / sample_rate + u32::from(h[2] >> 1 & 1)) as usize;
    Some(Mp3 { size, sample_rate, channels: if h[3] >> 6 == 3 { 1 } else { 2 }, samples })
}

/// The whole MP3 frames in `data`, each with its header.
pub(crate) fn mp3_frames(data: &[u8]) -> Vec<(Mp3, &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rest) = data.get(i..).filter(|r| r.len() >= 4) {
        match mp3_header(rest).and_then(|h| Some((h, rest.get(..h.size)?))) {
            Some((h, frame)) if h.size >= 4 => {
                out.push((h, frame));
                i += h.size;
            }
            _ => i += 1,
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests index what they have just checked is there")]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls").join(name);
        std::fs::read(path).unwrap_or_else(|e| unreachable!("{name}: {e}"))
    }

    fn sps_of(name: &str) -> Sps {
        let pes = demux(&fixture(name));
        let Some(v) = pes.iter().find(|p| p.stream == Stream::H264) else { unreachable!("video in {name}") };
        let Some(sps) = nal_units(&v.data).into_iter().find_map(parse_sps) else { unreachable!("an SPS") };
        sps
    }

    #[test]
    fn the_fixtures_sps_give_their_size_and_profile() {
        // 176x100: 7 macroblock rows cropped by 12 lines.
        let high = sps_of("aac/seg00000.ts");
        assert_eq!((high.width, high.height), (176, 100));
        assert!(high.codec.starts_with("avc1.64"), "{}", high.codec);
        let base = sps_of("mp3/seg00000.ts");
        assert_eq!((base.width, base.height), (176, 100));
        assert!(base.codec.starts_with("avc1.42"), "{}", base.codec);
        let main = sps_of("video/seg00000.ts");
        assert_eq!((main.width, main.height), (176, 100));
        assert!(main.codec.starts_with("avc1.4d"), "{}", main.codec);
    }

    #[test]
    fn an_sps_with_scaling_lists_and_fields_is_read() {
        // High profile 1920x1080 with seq_scaling_matrix_present and a scaling list,
        // written bit by bit.
        let mut bits = String::new();
        let ue = |v: u32| {
            let n = v + 1;
            let len = 32 - n.leading_zeros();
            format!("{}{:0w$b}", "0".repeat(len as usize - 1), n, w = len as usize)
        };
        let se = |v: i32| ue(if v > 0 { (2 * v - 1) as u32 } else { (-2 * v) as u32 });
        bits += &ue(0); // sps id
        bits += &ue(1); // 4:2:0
        bits += &ue(0);
        bits += &ue(0);
        bits += "0";
        bits += "1"; // scaling matrix present
        bits += "1"; // list 0 present: 16 deltas, the first -8 (next = 0 ends it)
        bits += &se(-8);
        bits += &"0".repeat(7); // lists 1..7 absent
        bits += &ue(0); // log2_max_frame_num
        bits += &ue(0); // poc type 0
        bits += &ue(0);
        bits += &ue(4); // ref frames
        bits += "0";
        bits += &ue(119); // 120 macroblocks wide
        bits += &ue(33); // 34 map units: 68 rows interlaced
        bits += "0"; // frame_mbs_only = 0
        bits += "1"; // mb_adaptive
        bits += "1";
        bits += "1"; // cropping
        bits += &ue(0);
        bits += &ue(0);
        bits += &ue(0);
        bits += &ue(2); // 2 * 2 * 2 = 8 lines off the bottom
        bits += "1";
        while !bits.len().is_multiple_of(8) {
            bits.push('0');
        }
        let mut nal = vec![0x67, 100, 0, 40];
        nal.extend(bits.as_bytes().chunks(8).map(|c| c.iter().fold(0u8, |a, b| a << 1 | (b - b'0'))));
        let sps = parse_sps(&nal);
        assert_eq!(sps, Some(Sps { codec: "avc1.640028".into(), width: 1920, height: 1080 }));
    }

    #[test]
    fn adts_headers_give_the_audio_specific_config() {
        let pes = demux(&fixture("aac/seg00000.ts"));
        let Some(a) = pes.iter().find(|p| p.stream == Stream::Aac) else { unreachable!("audio") };
        let frames = adts_frames(&a.data);
        assert!(!frames.is_empty());
        let (h, _) = frames[0];
        assert_eq!((h.object_type, h.sample_rate, h.channels), (2, 48000, 2));
        // AAC-LC, 48 kHz (index 3), stereo.
        assert_eq!(h.asc(), [0x11, 0x90]);
        let total: usize = frames.iter().map(|(_, f)| f.len() + 7).sum();
        assert_eq!(total, a.data.len(), "the frames cover the packet");
    }

    #[test]
    fn mp3_headers_give_size_rate_and_channels() {
        // MPEG-1 Layer III, 128 kb/s, 44.1 kHz, no padding, joint stereo.
        assert_eq!(
            mp3_header(&[0xff, 0xfb, 0x90, 0x44]),
            Some(Mp3 { size: 417, sample_rate: 44100, channels: 2, samples: 1152 })
        );
        // MPEG-2, 32 kb/s, 22.05 kHz, padded, mono.
        assert_eq!(
            mp3_header(&[0xff, 0xf3, 0x42, 0xc4]),
            Some(Mp3 { size: 105, sample_rate: 22050, channels: 1, samples: 576 })
        );
        // Layer II is not read.
        assert_eq!(mp3_header(&[0xff, 0xfd, 0x90, 0x44]), None);
        let pes = demux(&fixture("mp3/seg00000.ts"));
        let Some(a) = pes.iter().find(|p| p.stream == Stream::Mp3) else { unreachable!("audio") };
        let frames = mp3_frames(&a.data);
        assert!(!frames.is_empty());
        assert_eq!((frames[0].0.sample_rate, frames[0].0.channels), (44100, 2));
    }

    #[test]
    fn timestamps_unwrap_across_the_33_bit_wrap() {
        assert_eq!(unwrap(0, 90_000), 90_000);
        assert_eq!(unwrap(WRAP - 100, 50), WRAP + 50);
        assert_eq!(unwrap(WRAP + 50, WRAP - 100), WRAP - 100);
        assert_eq!(unwrap(3 * WRAP + 10, 20), 3 * WRAP + 20);
        assert_eq!(unwrap(10, WRAP - 10), WRAP - 10, "nothing goes below zero");
        // Nothing goes past the top (this overflowed).
        assert_eq!(unwrap(u64::MAX - 5, 0), u64::MAX & !(WRAP - 1));
        assert_eq!(timestamp(&[0x21, 0x00, 0x07, 0xd8, 0x61]), 126_000);
    }

    #[test]
    fn a_segment_gives_its_packets_with_their_times() {
        let pes = demux(&fixture("aac/seg00000.ts"));
        let video: Vec<&Pes> = pes.iter().filter(|p| p.stream == Stream::H264).collect();
        assert_eq!(video.len(), 25, "one access unit per frame");
        assert_eq!(video[0].dts, Some(126_000));
        assert_eq!(video[0].pts, Some(133_200));
        assert!(nal_units(&video[0].data).iter().any(|n| n[0] & 0x1f == 5), "starts on a key frame");
    }

    #[test]
    fn damaged_segments_are_read_without_panicking() {
        let seg = fixture("aac/seg00000.ts");
        let mut noisy = seg.clone();
        for (i, b) in noisy.iter_mut().enumerate().filter(|(i, _)| i % 97 == 5) {
            *b ^= (i % 251) as u8;
        }
        for data in [&seg[..1000], &seg[3..], &noisy[..], &[0x47; 400][..], &[0xff; 300][..]] {
            for p in demux(data) {
                let _ = adts_frames(&p.data);
                let _ = mp3_frames(&p.data);
                let _ = nal_units(&p.data).into_iter().map(parse_sps).count();
            }
        }
    }
}
