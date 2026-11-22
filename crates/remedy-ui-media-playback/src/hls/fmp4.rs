//! HLS units put back together as fragmented MP4, the form a browser's Media Source Extensions
//! take (`video/mp4`, `audio/mp4`): an initialization segment per stream (`ftyp` and a `moov`
//! that holds the codec's setup and no samples), then `moof` + `mdat` fragments of the units
//! [`Hls`](super::Hls) reads out of MPEG-TS. Each stream goes to a `SourceBuffer` of its own,
//! so each is a one-track file.
//!
//! A unit's duration is the time to the next one in decoding order, so the last unit of a batch
//! is held back until the next batch (or the end) says it: the fragments then follow on with no
//! gap the element would stall at.

use super::{AudioCodec, AudioConfig, Unit, VideoConfig, ts};
use std::time::Duration;

/// The clock video fragments count in (MPEG-TS's own).
const VIDEO_SCALE: u32 = 90_000;

/// How long a lone last unit lasts when nothing says (a stream's final picture).
const FALLBACK: Duration = Duration::from_millis(33);

/// A box: its type and body, sized.
fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&u32::try_from(body.len() + 8).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A full box: version and flags, then the body.
fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 4);
    b.push(version);
    b.extend_from_slice(&flags.to_be_bytes()[1..]);
    b.extend_from_slice(body);
    bx(kind, &b)
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// The identity matrix boxes carry.
const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn matrix() -> Vec<u8> {
    MATRIX.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// An MPEG-4 descriptor: its tag, its length (in the four-byte form every reader takes), its body.
fn descriptor(tag: u8, body: &[u8]) -> Vec<u8> {
    let n = u32::try_from(body.len()).unwrap_or(0).min(0x0fff_ffff);
    let mut out = vec![tag, 0x80 | (n >> 21) as u8 & 0x7f, 0x80 | (n >> 14) as u8 & 0x7f, 0x80 | (n >> 7) as u8 & 0x7f, n as u8 & 0x7f];
    out.extend_from_slice(body);
    out
}

/// `t` on a clock of `scale` ticks a second.
fn ticks(t: Duration, scale: u32) -> u64 {
    u64::try_from(t.as_nanos() * u128::from(scale) / 1_000_000_000).unwrap_or(u64::MAX)
}

/// The one-track `moov` around `trak_media` (the track's `mdia` contents after `mdhd`).
fn init(handler: &[u8; 4], scale: u32, header: &[u8], sample_entry: Vec<u8>, size: (u32, u32)) -> Vec<u8> {
    let ftyp = bx(b"ftyp", &cat(&[b"isom", &0x200u32.to_be_bytes(), b"isomiso6avc1mp41"]));
    let mvhd = full(
        b"mvhd",
        0,
        0,
        &cat(&[
            &[0; 8],
            &1000u32.to_be_bytes(),
            &[0; 4],
            &0x0001_0000u32.to_be_bytes(),
            &0x0100u16.to_be_bytes(),
            &[0; 10],
            &matrix(),
            &[0; 24],
            &2u32.to_be_bytes(),
        ]),
    );
    let audio = handler == b"soun";
    let tkhd = full(
        b"tkhd",
        0,
        3,
        &cat(&[
            &[0; 8],
            &1u32.to_be_bytes(),
            &[0; 4],
            &[0; 4],
            &[0; 8],
            &[0; 4],
            &(if audio { 0x0100u16 } else { 0 }).to_be_bytes(),
            &[0; 2],
            &matrix(),
            &(size.0 << 16).to_be_bytes(),
            &(size.1 << 16).to_be_bytes(),
        ]),
    );
    let mdhd = full(b"mdhd", 0, 0, &cat(&[&[0; 8], &scale.to_be_bytes(), &[0; 4], &0x55c4u16.to_be_bytes(), &[0; 2]]));
    let hdlr = full(b"hdlr", 0, 0, &cat(&[&[0; 4], handler, &[0; 12], b"RemedyUI\0"]));
    let dref = full(b"dref", 0, 0, &cat(&[&1u32.to_be_bytes(), &full(b"url ", 0, 1, &[])]));
    let stbl = bx(
        b"stbl",
        &cat(&[
            &full(b"stsd", 0, 0, &cat(&[&1u32.to_be_bytes(), &sample_entry])),
            &full(b"stts", 0, 0, &[0; 4]),
            &full(b"stsc", 0, 0, &[0; 4]),
            &full(b"stsz", 0, 0, &[0; 8]),
            &full(b"stco", 0, 0, &[0; 4]),
        ]),
    );
    let minf = bx(b"minf", &cat(&[header, &bx(b"dinf", &dref), &stbl]));
    let mdia = bx(b"mdia", &cat(&[&mdhd, &hdlr, &minf]));
    let trak = bx(b"trak", &cat(&[&tkhd, &mdia]));
    let trex = full(b"trex", 0, 0, &cat(&[&1u32.to_be_bytes(), &1u32.to_be_bytes(), &[0; 12]]));
    let moov = bx(b"moov", &cat(&[&mvhd, &trak, &bx(b"mvex", &trex)]));
    cat(&[&ftyp, &moov])
}

/// The initialization segment of an H.264 stream.
pub(crate) fn video_init(v: &VideoConfig) -> Vec<u8> {
    let sps = &v.sps;
    let byte = |i: usize| sps.get(i).copied().unwrap_or(0);
    let len16 = |b: &[u8]| u16::try_from(b.len()).unwrap_or(u16::MAX).to_be_bytes();
    let avcc = bx(
        b"avcC",
        &cat(&[&[1, byte(1), byte(2), byte(3), 0xff, 0xe1], &len16(sps), sps, &[1], &len16(&v.pps), &v.pps]),
    );
    let (w, h) = (u16::try_from(v.width).unwrap_or(u16::MAX), u16::try_from(v.height).unwrap_or(u16::MAX));
    let avc1 = bx(
        b"avc1",
        &cat(&[
            &[0; 6],
            &1u16.to_be_bytes(),
            &[0; 16],
            &w.to_be_bytes(),
            &h.to_be_bytes(),
            &0x0048_0000u32.to_be_bytes(),
            &0x0048_0000u32.to_be_bytes(),
            &[0; 4],
            &1u16.to_be_bytes(),
            &[0; 32],
            &0x0018u16.to_be_bytes(),
            &0xffffu16.to_be_bytes(),
            &avcc,
        ]),
    );
    let vmhd = full(b"vmhd", 0, 1, &[0; 8]);
    init(b"vide", VIDEO_SCALE, &vmhd, avc1, (v.width, v.height))
}

/// The MPEG-4 object type of a sound stream: AAC, or MPEG-1 (or, below 32 kHz, MPEG-2) audio.
fn object_type(a: &AudioConfig) -> u8 {
    match a.codec {
        AudioCodec::Aac { .. } => 0x40,
        AudioCodec::Mp3 if a.sample_rate >= 32_000 => 0x6b,
        AudioCodec::Mp3 => 0x69,
    }
}

/// The codec strings a `SourceBuffer` may know the sound by, best first.
pub(crate) fn audio_codecs(a: &AudioConfig) -> Vec<String> {
    match a.codec {
        AudioCodec::Aac { .. } => vec![a.codec_string.clone()],
        AudioCodec::Mp3 => vec![format!("mp4a.{:02X}", object_type(a)), "mp3".into()],
    }
}

/// The initialization segment of an AAC or MP3 stream.
pub(crate) fn audio_init(a: &AudioConfig) -> Vec<u8> {
    let specific = match &a.codec {
        AudioCodec::Aac { asc } => descriptor(5, asc),
        AudioCodec::Mp3 => Vec::new(),
    };
    let config = descriptor(4, &cat(&[&[object_type(a), 0x15, 0, 0, 0], &[0; 8], &specific]));
    let es = descriptor(3, &cat(&[&1u16.to_be_bytes(), &[0], &config, &descriptor(6, &[2])]));
    let esds = full(b"esds", 0, 0, &es);
    let rate = u32::try_from(u64::from(a.sample_rate) << 16).unwrap_or(0);
    let mp4a = bx(
        b"mp4a",
        &cat(&[
            &[0; 6],
            &1u16.to_be_bytes(),
            &[0; 8],
            &a.channels.to_be_bytes(),
            &16u16.to_be_bytes(),
            &[0; 4],
            &rate.to_be_bytes(),
            &esds,
        ]),
    );
    let smhd = full(b"smhd", 0, 0, &[0; 4]);
    init(b"soun", a.sample_rate.max(1), &smhd, mp4a, (0, 0))
}

/// One sample ready to go in a fragment.
struct Out {
    dts: u64,
    duration: u32,
    cto: i32,
    key: bool,
    data: Vec<u8>,
}

/// An Annex B access unit as MP4 samples carry it: each NAL unit behind its four-byte length,
/// the parameter sets and delimiters left out (the initialization segment has them).
// shortcut: a stream whose SPS changes part way (a new size) keeps the first; send a new
// initialization segment when one comes if a server ever switches inside a variant.
fn avcc(annex_b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annex_b.len() + 16);
    for nal in ts::nal_units(annex_b) {
        if matches!(nal.first().map(|b| b & 0x1f), Some(7..=9)) {
            continue;
        }
        out.extend_from_slice(&u32::try_from(nal.len()).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

/// One stream's fragments: its units in, `moof` + `mdat` out.
pub(crate) struct Track {
    scale: u32,
    sequence: u32,
    /// The last unit taken, waiting for the next to say how long it lasts: (pts, dts, key, data).
    held: Option<(Duration, Duration, bool, Vec<u8>)>,
    /// The last duration given, for a last unit with none after it.
    last: Duration,
    video: bool,
}

impl Track {
    /// Fragments for video (`video`) or for sound at `sample_rate`.
    pub(crate) fn new(video: bool, sample_rate: u32) -> Self {
        let scale = if video { VIDEO_SCALE } else { sample_rate.max(1) };
        Track { scale, sequence: 0, held: None, last: FALLBACK, video }
    }

    /// Forgets the unit held back (a seek: what comes next is from elsewhere).
    pub(crate) fn reset(&mut self) {
        self.held = None;
    }

    /// The fragment of `units` (this track's, in decoding order), the last held back; `None`
    /// when there is nothing to send yet. `end`: no more units follow, the held one goes too.
    pub(crate) fn fragment(&mut self, units: Vec<Unit>, end: bool) -> Option<Vec<u8>> {
        let mut all: Vec<(Duration, Duration, bool, Vec<u8>)> = self.held.take().into_iter().collect();
        for u in units {
            all.push(match u {
                Unit::Video { pts, dts, keyframe, data } => (pts, dts, keyframe, avcc(&data)),
                Unit::Audio { pts, data } => (pts, pts, true, data),
            });
        }
        if !end {
            self.held = all.pop();
        }
        let mut out = Vec::with_capacity(all.len());
        for (i, (pts, dts, key, data)) in all.iter().enumerate() {
            let next = all.get(i + 1).map(|n| n.1).or(self.held.as_ref().map(|h| h.1));
            let length = match next {
                Some(n) if n > *dts => n - *dts,
                _ => self.last,
            };
            self.last = length;
            let d = ticks(*dts, self.scale);
            let duration = u32::try_from(ticks(*dts + length, self.scale).saturating_sub(d)).unwrap_or(u32::MAX).max(1);
            let cto = i64::try_from(ticks(*pts, self.scale)).unwrap_or(0) - i64::try_from(d).unwrap_or(0);
            out.push(Out {
                dts: d,
                duration,
                cto: i32::try_from(cto).unwrap_or(0),
                key: *key || !self.video,
                data: data.clone(),
            });
        }
        let first = out.first()?.dts;
        self.sequence += 1;
        Some(moof_mdat(self.sequence, first, &out))
    }
}

fn moof_mdat(sequence: u32, base: u64, samples: &[Out]) -> Vec<u8> {
    let mfhd = full(b"mfhd", 0, 0, &sequence.to_be_bytes());
    // default-base-is-moof: the data offset counts from the moof's first byte.
    let tfhd = full(b"tfhd", 0, 0x02_0000, &1u32.to_be_bytes());
    let tfdt = full(b"tfdt", 1, 0, &base.to_be_bytes());
    let count = u32::try_from(samples.len()).unwrap_or(u32::MAX);
    // data offset, and each sample's duration, size, flags and (signed) composition offset.
    let trun_len = 12 + 8 + samples.len() * 16;
    let moof_len = 8 + mfhd.len() + 8 + tfhd.len() + tfdt.len() + trun_len;
    let mut trun = cat(&[&count.to_be_bytes(), &i32::try_from(moof_len + 8).unwrap_or(0).to_be_bytes()]);
    for s in samples {
        let flags: u32 = if s.key { 0x0200_0000 } else { 0x0101_0000 };
        let size = u32::try_from(s.data.len()).unwrap_or(u32::MAX);
        trun.extend_from_slice(&cat(&[&s.duration.to_be_bytes(), &size.to_be_bytes(), &flags.to_be_bytes(), &s.cto.to_be_bytes()]));
    }
    let trun = full(b"trun", 1, 0x0f01, &trun);
    let traf = bx(b"traf", &cat(&[&tfhd, &tfdt, &trun]));
    let moof = bx(b"moof", &cat(&[&mfhd, &traf]));
    debug_assert_eq!(moof.len(), moof_len);
    let data: Vec<u8> = samples.iter().flat_map(|s| s.data.iter().copied()).collect();
    cat(&[&moof, &bx(b"mdat", &data)])
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests index what they have just checked is there")]
mod tests {
    use super::*;

    /// The boxes at the top of `data`: (type, body).
    fn boxes(data: &[u8]) -> Vec<(String, &[u8])> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(h) = data.get(i..i + 8) {
            let len = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
            let body = data.get(i + 8..i + len).unwrap_or_else(|| unreachable!("a box runs past its parent"));
            out.push((String::from_utf8_lossy(&h[4..8]).into_owned(), body));
            i += len;
        }
        assert_eq!(i, data.len(), "boxes fill their parent exactly");
        out
    }

    fn child<'a>(data: &'a [u8], path: &[&str]) -> &'a [u8] {
        let mut at = data;
        for p in path {
            at = boxes(at).into_iter().find(|(k, _)| k == p).unwrap_or_else(|| unreachable!("no {p}")).1;
        }
        at
    }

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    fn video() -> VideoConfig {
        VideoConfig {
            codec: "avc1.64001f".into(),
            width: 176,
            height: 100,
            sps: vec![0x67, 0x64, 0x00, 0x1f, 0xac],
            pps: vec![0x68, 0xee, 0x3c],
        }
    }

    #[test]
    fn the_video_init_segment_holds_the_parameter_sets_and_the_size() {
        let init = video_init(&video());
        let top: Vec<String> = boxes(&init).into_iter().map(|b| b.0).collect();
        assert_eq!(top, ["ftyp", "moov"]);
        let stsd = child(&init, &["moov", "trak", "mdia", "minf", "stbl", "stsd"]);
        let entry = stsd.get(8..).unwrap_or_default();
        let avc1 = child(entry, &["avc1"]);
        assert_eq!(u16::from_be_bytes([avc1[24], avc1[25]]), 176);
        assert_eq!(u16::from_be_bytes([avc1[26], avc1[27]]), 100);
        let avcc = child(avc1.get(78..).unwrap_or_default(), &["avcC"]);
        assert_eq!(&avcc[..6], &[1, 0x64, 0x00, 0x1f, 0xff, 0xe1]);
        assert_eq!(&avcc[8..13], &video().sps[..]);
        assert_eq!(child(&init, &["moov", "trak", "mdia", "hdlr"]).get(8..12), Some(&b"vide"[..]));
        assert_eq!(be32(child(&init, &["moov", "trak", "mdia", "mdhd"]), 12), 90_000);
        let _ = child(&init, &["moov", "mvex", "trex"]);
    }

    #[test]
    fn the_sound_init_segment_carries_the_aac_config() {
        let a = AudioConfig {
            codec: AudioCodec::Aac { asc: vec![0x11, 0x90] },
            codec_string: "mp4a.40.2".into(),
            sample_rate: 48_000,
            channels: 2,
        };
        let init = audio_init(&a);
        assert_eq!(be32(child(&init, &["moov", "trak", "mdia", "mdhd"]), 12), 48_000);
        let entry = child(&init, &["moov", "trak", "mdia", "minf", "stbl", "stsd"]).get(8..).unwrap_or_default();
        let mp4a = child(entry, &["mp4a"]);
        assert_eq!(u16::from_be_bytes([mp4a[16], mp4a[17]]), 2, "channels");
        assert_eq!(be32(mp4a, 24) >> 16, 48_000, "rate");
        let esds = child(mp4a.get(28..).unwrap_or_default(), &["esds"]);
        assert!(esds.windows(7).any(|w| w == [5, 0x80, 0x80, 0x80, 2, 0x11, 0x90]), "the AudioSpecificConfig");
        assert_eq!(audio_codecs(&a), ["mp4a.40.2"]);
        let mp3 = AudioConfig { codec: AudioCodec::Mp3, codec_string: "mp3".into(), sample_rate: 44_100, channels: 2 };
        assert_eq!(audio_codecs(&mp3), ["mp4a.6B", "mp3"]);
    }

    fn picture(ms: u64, dts: u64, key: bool) -> Unit {
        let mut data = vec![0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3];
        data.extend_from_slice(&[0, 0, 0, 1, if key { 0x65 } else { 0x41 }, 9, 9, 9]);
        Unit::Video { pts: Duration::from_millis(ms), dts: Duration::from_millis(dts), keyframe: key, data }
    }

    /// A trun row: duration, size, flags, composition offset.
    type Row = (u32, u32, u32, i32);

    /// The tfdt, the trun's rows and its data offset.
    fn samples(frag: &[u8]) -> (u64, Vec<Row>, usize) {
        let traf = child(frag, &["moof", "traf"]);
        let tfdt = child(traf, &["tfdt"]);
        let base = u64::from_be_bytes(tfdt[4..12].try_into().unwrap_or_default());
        let trun = child(traf, &["trun"]);
        let n = be32(trun, 4) as usize;
        let offset = be32(trun, 8) as usize;
        let rows = (0..n)
            .map(|i| {
                let r = 12 + i * 16;
                (be32(trun, r), be32(trun, r + 4), be32(trun, r + 8), be32(trun, r + 12) as i32)
            })
            .collect();
        (base, rows, offset)
    }

    #[test]
    fn fragments_follow_on_with_the_last_unit_held_for_its_duration() {
        let mut t = Track::new(true, 0);
        // Decoding order with a B-frame: shown at 0, 80, 40.
        let first = t.fragment(vec![picture(0, 0, true), picture(80, 40, false), picture(40, 80, false)], false);
        let first = first.unwrap_or_else(|| unreachable!("a fragment"));
        assert_eq!(boxes(&first).into_iter().map(|b| b.0).collect::<Vec<_>>(), ["moof", "mdat"]);
        let (base, rows, offset) = samples(&first);
        assert_eq!(base, 0);
        assert_eq!(rows.len(), 2, "the last waits for the next to say how long it lasts");
        assert_eq!(rows[0], (3600, 8, 0x0200_0000, 0), "a key frame 40 ms long, the SPS and PPS left out");
        assert_eq!(rows[1], (3600, 8, 0x0101_0000, 3600), "shown 40 ms after it is decoded");
        // The data offset lands on the mdat's body.
        let moof_len = boxes(&first)[0].1.len() + 8;
        assert_eq!(offset, moof_len + 8);
        let second = t.fragment(vec![picture(120, 120, true)], true).unwrap_or_else(|| unreachable!());
        let (base, rows, _) = samples(&second);
        assert_eq!(base, 7200, "it starts where the held one decodes");
        assert_eq!(rows.len(), 2, "the end sends everything");
        assert_eq!(rows[0].3, -3600, "shown before it is decoded relative to its dts");
        assert_eq!(rows[1].0, 3600, "the last lasts as long as the one before");
        // A seek forgets what was held.
        let _ = t.fragment(vec![picture(0, 0, true)], false);
        t.reset();
        assert!(t.fragment(Vec::new(), true).is_none());
    }

    #[test]
    fn sound_fragments_count_in_the_sound_rate() {
        let mut t = Track::new(false, 48_000);
        let frame = |ms: u64| Unit::Audio { pts: Duration::from_nanos(ms * 21_333_333), data: vec![7; 5] };
        let f = t.fragment((0..4).map(frame).collect(), true).unwrap_or_else(|| unreachable!());
        let (_, rows, _) = samples(&f);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| (1023..=1024).contains(&r.0) && r.2 == 0x0200_0000), "{rows:?}");
    }
}
