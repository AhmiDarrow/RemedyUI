//! Reading MP4 (ISO base media) files: the tracks, their codecs and configuration, and where
//! each sample is and when it plays. Decoders that take compressed samples (a browser's
//! WebCodecs) need the container taken apart first; this does it for H.264 video and AAC
//! sound, the pair nearly every MP4 carries. Untrusted input: every size and offset is
//! checked against the data, and tables are as large as the file says, never larger than the
//! file could hold.

use std::path::Path;
use std::time::Duration;

use crate::host::MediaError;
use crate::subtitles::{Cue, SubtitleTrack, clean_markup, cues_too_big, over_cue_budget, srt_settings};

/// Why a file could not be read as MP4.
pub(crate) type Error = String;

/// What kind of track.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    /// Pictures.
    Video {
        /// Width and height of the coded picture.
        width: u32,
        /// Height.
        height: u32,
    },
    /// Sound.
    Audio {
        /// Frames per second.
        sample_rate: u32,
        /// Channels.
        channels: u16,
    },
    /// MPEG-4 timed text (`tx3g`): subtitles or captions.
    Text {
        /// Its language (ISO 639-2), when the file names one.
        language: Option<String>,
    },
}

/// Where one sample is and when it plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SampleRef {
    /// Byte offset in the file.
    pub offset: u64,
    /// Bytes.
    pub size: u32,
    /// Presentation time, in the track's timescale (edit list applied).
    pub pts: i64,
    /// How long it lasts, in the timescale.
    pub duration: u32,
    /// A sync sample (a key frame).
    pub sync: bool,
}

/// One track.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Track {
    /// What it holds.
    pub kind: Kind,
    /// The WebCodecs codec string (`avc1.42c01e`, `mp4a.40.2`).
    pub codec: String,
    /// The decoder configuration (`avcC` contents, or AAC's AudioSpecificConfig).
    pub description: Vec<u8>,
    /// Units per second of its times.
    pub timescale: u32,
    /// Its samples in decoding order.
    pub samples: Vec<SampleRef>,
}

impl Track {
    /// A time in the track's timescale as a duration (negative times are zero).
    pub fn time(&self, t: i64) -> Duration {
        if t <= 0 || self.timescale == 0 {
            return Duration::ZERO;
        }
        let t = t as u64;
        let s = u64::from(self.timescale);
        Duration::from_secs(t / s) + Duration::from_nanos((t % s) * 1_000_000_000 / s)
    }

    /// How long the track lasts.
    pub fn length(&self) -> Duration {
        let end = self.samples.iter().map(|s| s.pts.saturating_add(i64::from(s.duration))).max().unwrap_or(0);
        self.time(end)
    }
}

/// A parsed file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mp4 {
    /// Its tracks (only those with a codec this reader knows).
    pub tracks: Vec<Track>,
}

impl Mp4 {
    /// The first video track.
    pub fn video(&self) -> Option<usize> {
        self.tracks.iter().position(|t| matches!(t.kind, Kind::Video { .. }))
    }

    /// The first sound track.
    pub fn audio(&self) -> Option<usize> {
        self.tracks.iter().position(|t| matches!(t.kind, Kind::Audio { .. }))
    }
}

/// A box: its type and its contents.
struct Atom<'a> {
    kind: [u8; 4],
    body: &'a [u8],
}

/// The boxes in `data`, in order.
fn atoms(mut data: &[u8]) -> Result<Vec<Atom<'_>>, Error> {
    let mut out = Vec::new();
    while data.len() >= 8 {
        let size32 = be32(data, 0)?;
        let kind: [u8; 4] = data.get(4..8).and_then(|k| k.try_into().ok()).ok_or("short box")?;
        let (header, size) = match size32 {
            0 => (8usize, data.len()),
            1 => {
                let s = usize::try_from(be64(data, 8)?).map_err(|_| "a box too large")?;
                (16usize, s)
            }
            s => (8usize, s as usize),
        };
        if size < header || size > data.len() {
            return Err(format!("the {} box runs past its parent", String::from_utf8_lossy(&kind)));
        }
        out.push(Atom { kind, body: data.get(header..size).ok_or("short box")? });
        data = data.get(size..).unwrap_or_default();
    }
    Ok(out)
}

fn child<'a>(boxes: &'a [Atom<'a>], kind: &[u8; 4]) -> Option<&'a Atom<'a>> {
    boxes.iter().find(|a| &a.kind == kind)
}

fn be16(d: &[u8], at: usize) -> Result<u16, Error> {
    d.get(at..at + 2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_be_bytes)
        .ok_or_else(|| "short data".into())
}

fn be32(d: &[u8], at: usize) -> Result<u32, Error> {
    d.get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| "short data".into())
}

fn be64(d: &[u8], at: usize) -> Result<u64, Error> {
    d.get(at..at + 8)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_be_bytes)
        .ok_or_else(|| "short data".into())
}

/// The entries of a table box (`count` after the version and flags, then `width`-byte rows),
/// checked against the box's size.
fn table(body: &[u8], header: usize, width: usize) -> Result<(usize, &[u8]), Error> {
    let count = be32(body, header)? as usize;
    let rows = body.get(header + 4..).unwrap_or_default();
    if count.checked_mul(width).is_none_or(|n| n > rows.len()) {
        return Err("a sample table is longer than its box".into());
    }
    Ok((count, rows))
}

/// Reads `data` as an MP4 file.
pub(crate) fn parse(data: &[u8]) -> Result<Mp4, Error> {
    let top = atoms(data)?;
    let moov = child(&top, b"moov").ok_or("no movie box (moov): not an MP4 file, or a fragmented one")?;
    let tracks = moov_tracks(moov.body, data.len() as u64)?;
    if tracks.iter().all(|t| matches!(t.kind, Kind::Text { .. })) {
        return Err("no H.264 video or AAC sound in the file".into());
    }
    Ok(Mp4 { tracks })
}

/// Reads an MP4 file `len` bytes long from `head`, the start of it that has arrived (a
/// download under way): `Ok(None)` until all of its movie box is in. The samples it lists
/// may lie past `head`.
pub(crate) fn parse_head(head: &[u8], len: u64) -> Result<Option<Mp4>, Error> {
    // Walk the top-level boxes by their headers alone until the movie box.
    let mut at = 0u64;
    loop {
        if at.saturating_add(8) > len {
            return Err("no movie box (moov): not an MP4 file, or a fragmented one".into());
        }
        let start = usize::try_from(at).map_err(|_| "the file is too large")?;
        let h = head.get(start..head.len().min(start.saturating_add(16))).unwrap_or_default();
        if h.len() < 8 {
            return Ok(None);
        }
        let (header, size) = match be32(h, 0)? {
            0 => (8, len - at),
            1 if h.len() < 16 => return Ok(None),
            1 => (16, be64(h, 8)?),
            s => (8, u64::from(s)),
        };
        if size < header || size > len - at {
            return Err("a box runs past the end of the file".into());
        }
        if h.get(4..8) == Some(b"moov".as_slice()) {
            if size > crate::media_limits().max_moov {
                return Err("the movie box is too large".into());
            }
            let end = usize::try_from(size).ok().and_then(|s| start.checked_add(s));
            let Some(body) = end.and_then(|end| head.get(start + header as usize..end)) else {
                return Ok(None);
            };
            let tracks = moov_tracks(body, len)?;
            if tracks.iter().all(|t| matches!(t.kind, Kind::Text { .. })) {
                return Err("no H.264 video or AAC sound in the file".into());
            }
            return Ok(Some(Mp4 { tracks }));
        }
        at += size;
    }
}

fn moov_tracks(moov: &[u8], file_len: u64) -> Result<Vec<Track>, Error> {
    let moov = atoms(moov)?;
    let mut tracks = Vec::new();
    for trak in moov.iter().filter(|a| &a.kind == b"trak") {
        if let Some(t) = track(trak.body, file_len)? {
            tracks.push(t);
        }
    }
    Ok(tracks)
}

/// The MPEG-4 timed-text tracks in the file at `path`, as subtitle tracks; none when it is
/// not an MP4 or holds none. Only the movie box and the text samples are read, so a long
/// film costs a few small reads.
/// Their text over the subtitle file limit is an error.
pub(crate) fn text_tracks(path: &Path) -> Result<Vec<SubtitleTrack>, MediaError> {
    match read_text_tracks(path) {
        Err(e) if e == cues_too_big() => Err(MediaError::Format(e)),
        r => Ok(r.unwrap_or_default()),
    }
}

fn read_text_tracks(path: &Path) -> Result<Vec<SubtitleTrack>, Error> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |e: std::io::Error| e.to_string();
    let mut f = std::fs::File::open(path).map_err(io)?;
    let len = f.metadata().map_err(io)?.len();
    // Walk the top-level boxes by their headers alone until the movie box.
    let mut at = 0u64;
    let moov = loop {
        if at.saturating_add(8) > len {
            return Ok(Vec::new());
        }
        let mut h = [0u8; 16];
        f.seek(SeekFrom::Start(at)).map_err(io)?;
        f.read_exact(&mut h[..8]).map_err(io)?;
        let (header, size) = match be32(&h, 0)? {
            0 => (8, len - at),
            1 => {
                f.read_exact(&mut h[8..]).map_err(io)?;
                (16, be64(&h, 8)?)
            }
            s => (8, u64::from(s)),
        };
        if size < header || size > len - at {
            return Err("a box runs past the end of the file".into());
        }
        if &h[4..8] == b"moov" {
            if size > crate::media_limits().max_moov {
                return Err("the movie box is too large".into());
            }
            let body_len = usize::try_from(size - header).map_err(|_| "the movie box is too large")?;
            let mut body = vec![0u8; body_len];
            f.read_exact(&mut body).map_err(io)?;
            break body;
        }
        at += size;
    };
    let mut out = Vec::new();
    let mut text_bytes = 0;
    for t in moov_tracks(&moov, len)? {
        let Kind::Text { language } = &t.kind else { continue };
        let mut cues = Vec::new();
        let max_cue = crate::media_limits().max_embedded_cue;
        for s in t.samples.iter().filter(|s| (2..=max_cue).contains(&u64::from(s.size))) {
            let mut b = vec![0u8; s.size as usize];
            f.seek(SeekFrom::Start(s.offset)).map_err(io)?;
            f.read_exact(&mut b).map_err(io)?;
            let text = cue_text(&b);
            let (start, end) = (t.time(s.pts), t.time(s.pts.saturating_add(i64::from(s.duration))));
            if !text.is_empty() && end > start {
                if over_cue_budget(&mut text_bytes, &text) {
                    return Err(cues_too_big());
                }
                // 3GPP timed text has no tags, but `{\anN}` is carried by some muxers from SRT.
                let settings = srt_settings(&text);
                let text = if settings == Default::default() { text } else { clean_markup(&text) };
                cues.push(Cue { start, end, text, settings });
            }
        }
        cues.sort_by_key(|c| c.start);
        if !cues.is_empty() {
            let label = language.clone().unwrap_or_else(|| "Subtitles".into());
            out.push(SubtitleTrack { label, language: language.clone(), cues });
        }
    }
    Ok(out)
}

/// A `tx3g` sample's text: a big-endian length, then UTF-8 (or UTF-16 after a byte-order
/// mark); style boxes after it are left out.
fn cue_text(b: &[u8]) -> String {
    let n = be16(b, 0).map_or(0, usize::from);
    let t = b.get(2..(2 + n).min(b.len())).unwrap_or_default();
    let text = match t {
        [0xFE, 0xFF, rest @ ..] => {
            let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(t).into_owned(),
    };
    text.replace("\r\n", "\n").trim().to_owned()
}

/// A media header's language: three letters packed five bits each (ISO 639-2/T), `und`
/// for none.
fn mdhd_language(mdhd: &[u8]) -> Option<String> {
    let at = if mdhd.first() == Some(&1) { 32 } else { 20 };
    let packed = be16(mdhd, at).ok()?;
    let letters: String =
        [10u16, 5, 0].iter().map(|s| char::from(((packed >> s) & 0x1F) as u8 + 0x60)).collect();
    (letters.chars().all(|c| c.is_ascii_lowercase()) && letters != "und").then_some(letters)
}

fn track(trak: &[u8], file_len: u64) -> Result<Option<Track>, Error> {
    let trak = atoms(trak)?;
    let Some(mdia) = child(&trak, b"mdia") else { return Ok(None) };
    let mdia = atoms(mdia.body)?;
    let mdhd = child(&mdia, b"mdhd").ok_or("a track has no media header")?.body;
    let timescale = if mdhd.first() == Some(&1) { be32(mdhd, 20)? } else { be32(mdhd, 12)? };
    let Some(minf) = child(&mdia, b"minf") else { return Ok(None) };
    let minf = atoms(minf.body)?;
    let Some(stbl) = child(&minf, b"stbl") else { return Ok(None) };
    let stbl = atoms(stbl.body)?;
    let stsd = child(&stbl, b"stsd").ok_or("a track has no sample descriptions")?.body;
    let entries = atoms(stsd.get(8..).unwrap_or_default())?;
    let Some(entry) = entries.first() else { return Ok(None) };
    let (kind, codec, description) = match &entry.kind {
        b"avc1" | b"avc3" => {
            let e = entry.body;
            let width = u32::from(be16(e, 24)?);
            let height = u32::from(be16(e, 26)?);
            let inner = atoms(e.get(78..).unwrap_or_default())?;
            let avcc = child(&inner, b"avcC").ok_or("H.264 video without its configuration (avcC)")?.body;
            let p = |i: usize| avcc.get(i).copied().unwrap_or(0);
            let codec =
                format!("{}.{:02x}{:02x}{:02x}", String::from_utf8_lossy(&entry.kind), p(1), p(2), p(3));
            (Kind::Video { width, height }, codec, avcc.to_vec())
        }
        b"mp4a" => {
            let e = entry.body;
            let channels = be16(e, 16)?;
            let sample_rate = be32(e, 24)? >> 16;
            let inner = atoms(e.get(28..).unwrap_or_default())?;
            let esds = child(&inner, b"esds").ok_or("AAC sound without its configuration (esds)")?.body;
            let asc = audio_specific_config(esds.get(4..).unwrap_or_default())
                .ok_or("AAC sound whose configuration cannot be read")?;
            let object = asc.first().map_or(2, |b| b >> 3);
            (Kind::Audio { sample_rate, channels }, format!("mp4a.40.{object}"), asc)
        }
        b"tx3g" => (Kind::Text { language: mdhd_language(mdhd) }, "tx3g".into(), Vec::new()),
        _ => return Ok(None),
    };
    let edit_shift = child(&trak, b"edts").map(|e| edit_shift(e.body)).transpose()?.flatten().unwrap_or(0);
    let samples = samples(&stbl, edit_shift, file_len)?;
    Ok(Some(Track { kind, codec, description, timescale, samples }))
}

/// The media time the presentation starts at (the first edit's `media_time`), which every
/// presentation time is shifted back by (AAC's encoder delay, a B-frame delay).
fn edit_shift(edts: &[u8]) -> Result<Option<i64>, Error> {
    let edts = atoms(edts)?;
    let Some(elst) = child(&edts, b"elst") else { return Ok(None) };
    let b = elst.body;
    let v1 = b.first() == Some(&1);
    let (count, rows) = table(b, 4, if v1 { 20 } else { 12 })?;
    for i in 0..count {
        let media_time =
            if v1 { be64(rows, i * 20 + 8)? as i64 } else { i64::from(be32(rows, i * 12 + 4)? as i32) };
        // An empty edit (-1) delays the start; the first real edit says where it begins.
        if media_time >= 0 {
            return Ok(Some(media_time));
        }
    }
    Ok(None)
}

/// AAC's AudioSpecificConfig from an `esds` box's descriptors.
fn audio_specific_config(mut d: &[u8]) -> Option<Vec<u8>> {
    // Descriptors: a tag, a length in 7-bit groups, the body.
    fn descriptor(d: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let tag = *d.first()?;
        let mut len = 0usize;
        let mut i = 1;
        loop {
            let b = *d.get(i)?;
            len = (len << 7) | usize::from(b & 0x7f);
            i += 1;
            if b & 0x80 == 0 || i > 4 {
                break;
            }
        }
        let body = d.get(i..i + len)?;
        Some((tag, body, d.get(i + len..).unwrap_or_default()))
    }
    let (tag, es, _) = descriptor(d)?;
    if tag != 0x03 {
        return None;
    }
    // ES_ID (2) and flags (1), then what the flags add.
    let flags = *es.get(2)?;
    let mut at = 3;
    if flags & 0x80 != 0 {
        at += 2;
    }
    if flags & 0x40 != 0 {
        at += 1 + usize::from(*es.get(at)?);
    }
    if flags & 0x20 != 0 {
        at += 2;
    }
    d = es.get(at..)?;
    while !d.is_empty() {
        let (tag, body, rest) = descriptor(d)?;
        if tag == 0x04 {
            // objectTypeIndication, streamType, buffer size (3), max and average bit rates.
            let (tag, asc, _) = descriptor(body.get(13..)?)?;
            return (tag == 0x05).then(|| asc.to_vec());
        }
        d = rest;
    }
    None
}

/// The samples of a track from its sample tables.
fn samples(stbl: &[Atom<'_>], shift: i64, file_len: u64) -> Result<Vec<SampleRef>, Error> {
    let need = |k: &[u8; 4]| {
        child(stbl, k).map(|a| a.body).ok_or_else(|| format!("missing {} table", String::from_utf8_lossy(k)))
    };
    // Sizes.
    let stsz = need(b"stsz")?;
    let fixed = be32(stsz, 4)?;
    let count = be32(stsz, 8)? as usize;
    // Each sample takes at least a byte of the file (or four bytes of the size table).
    if fixed == 0 && count.checked_mul(4).is_none_or(|n| n > stsz.len()) || count as u64 > file_len {
        return Err("the sample size table is longer than its box".into());
    }
    let size =
        |i: usize| -> Result<u32, Error> { if fixed != 0 { Ok(fixed) } else { be32(stsz, 12 + i * 4) } };
    // Chunk offsets.
    let offsets: Vec<u64> = if let Ok(stco) = need(b"stco") {
        let (n, rows) = table(stco, 4, 4)?;
        (0..n).map(|i| be32(rows, i * 4).map(u64::from)).collect::<Result<_, _>>()?
    } else {
        let co64 = need(b"co64")?;
        let (n, rows) = table(co64, 4, 8)?;
        (0..n).map(|i| be64(rows, i * 8)).collect::<Result<_, _>>()?
    };
    // Samples per chunk, in runs.
    let stsc = need(b"stsc")?;
    let (runs, rows) = table(stsc, 4, 12)?;
    let run = |i: usize| -> Result<(u32, u32), Error> { Ok((be32(rows, i * 12)?, be32(rows, i * 12 + 4)?)) };
    // The count is the file's claim: with one size for all, nothing above has tied it to the
    // tables' own lengths, so only as many as that size fits in the file are reserved.
    let fit = usize::try_from(file_len / u64::from(fixed.max(1))).unwrap_or(usize::MAX);
    let mut out: Vec<SampleRef> = Vec::with_capacity(count.min(fit).min(1 << 20));
    let mut sample = 0usize;
    for r in 0..runs {
        let (first, per) = run(r)?;
        let next_first = if r + 1 < runs { run(r + 1)?.0 } else { offsets.len() as u32 + 1 };
        for chunk in first.max(1)..next_first {
            // A run that adds no sample must not walk every chunk (runs times chunks, for nothing).
            if per == 0 || sample >= count {
                break;
            }
            let Some(&base) = offsets.get(chunk as usize - 1) else { break };
            let mut at = base;
            for _ in 0..per {
                if sample >= count {
                    break;
                }
                let sz = size(sample)?;
                if at.checked_add(u64::from(sz)).is_none_or(|end| end > file_len) {
                    return Err("a sample lies outside the file".into());
                }
                out.push(SampleRef { offset: at, size: sz, pts: 0, duration: 0, sync: true });
                at += u64::from(sz);
                sample += 1;
            }
        }
    }
    // Decoding times.
    let stts = need(b"stts")?;
    let (n, rows) = table(stts, 4, 8)?;
    let mut dts: i64 = 0;
    let mut i = 0usize;
    for e in 0..n {
        let (k, delta) = (be32(rows, e * 8)?, be32(rows, e * 8 + 4)?);
        // A count past the samples there are is not walked (it may claim billions).
        for _ in 0..k {
            let Some(s) = out.get_mut(i) else { break };
            s.pts = dts;
            s.duration = delta;
            dts = dts.saturating_add(i64::from(delta));
            i += 1;
        }
    }
    // Composition offsets.
    if let Ok(ctts) = need(b"ctts") {
        let signed = ctts.first() == Some(&1);
        let (n, rows) = table(ctts, 4, 8)?;
        let mut i = 0usize;
        for e in 0..n {
            let k = be32(rows, e * 8)?;
            let raw = be32(rows, e * 8 + 4)?;
            let off = if signed { i64::from(raw as i32) } else { i64::from(raw) };
            for _ in 0..k {
                let Some(s) = out.get_mut(i) else { break };
                s.pts = s.pts.saturating_add(off);
                i += 1;
            }
        }
    }
    for s in &mut out {
        s.pts = s.pts.saturating_sub(shift);
    }
    // Sync samples (all of them when the table is absent).
    if let Ok(stss) = need(b"stss") {
        let (n, rows) = table(stss, 4, 4)?;
        for s in &mut out {
            s.sync = false;
        }
        for e in 0..n {
            let k = be32(rows, e * 4)? as usize;
            if let Some(s) = k.checked_sub(1).and_then(|k| out.get_mut(k)) {
                s.sync = true;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip.mp4");
        std::fs::read(path).unwrap_or_else(|e| unreachable!("the test clip: {e}"))
    }

    #[test]
    fn the_test_clip_has_its_video_and_sound() {
        let data = clip();
        let Ok(m) = parse(&data) else { unreachable!("the clip parses") };
        let Some(v) = m.video().and_then(|i| m.tracks.get(i)) else { unreachable!("a video track") };
        assert_eq!(v.kind, Kind::Video { width: 176, height: 100 });
        assert!(v.codec.starts_with("avc1.42"), "baseline H.264: {}", v.codec);
        assert_eq!(v.samples.len(), 75, "three seconds at 25 fps");
        assert_eq!(v.samples.iter().filter(|s| s.sync).count(), 1, "one key frame");
        assert!((v.length().as_secs_f64() - 3.0).abs() < 0.05, "{:?}", v.length());
        let first = v.samples.first().map(|s| v.time(s.pts));
        assert_eq!(first, Some(Duration::ZERO));
        let Some(a) = m.audio().and_then(|i| m.tracks.get(i)) else { unreachable!("a sound track") };
        assert_eq!(a.kind, Kind::Audio { sample_rate: 48_000, channels: 2 });
        assert_eq!(a.codec, "mp4a.40.2");
        assert!(
            a.description.len() >= 2 && a.description.first().map(|b| b >> 3) == Some(2),
            "an AAC-LC config: {:?}",
            a.description
        );
        assert!((a.length().as_secs_f64() - 3.0).abs() < 0.1, "{:?}", a.length());
        for s in v.samples.iter().chain(&a.samples) {
            assert!(s.offset + u64::from(s.size) <= data.len() as u64);
        }
    }

    #[test]
    fn a_download_under_way_reads_once_its_movie_box_is_in() {
        let data = clip();
        let len = data.len() as u64;
        let Ok(whole) = parse(&data) else { unreachable!("the clip parses") };
        // The clip's movie box follows its 32-byte file type box.
        let moov_end = 32 + 3033;
        for cut in [0, 7, 8, 31, 40, moov_end - 1] {
            let head = data.get(..cut).unwrap_or_default();
            assert!(matches!(parse_head(head, len), Ok(None)), "{cut} bytes in: not yet");
        }
        for cut in [moov_end, moov_end + 100, data.len()] {
            let head = data.get(..cut).unwrap_or_default();
            let Ok(Some(m)) = parse_head(head, len) else { unreachable!("{cut} bytes in: the movie box") };
            assert_eq!(m, whole, "{cut} bytes in");
        }
        // Not an MP4, or a length the boxes run past, says so instead of waiting.
        assert!(parse_head(b"not an mp4 at all", 17).is_err());
        assert!(parse_head(data.get(..moov_end).unwrap_or_default(), 1000).is_err());
    }

    #[test]
    fn timed_text_is_read_from_the_file_with_its_language() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip-subs.mp4");
        let tracks = text_tracks(&path).unwrap_or_default();
        assert_eq!(tracks.len(), 1, "{tracks:?}");
        let Some(t) = tracks.first() else { unreachable!("a track") };
        assert_eq!(t.language.as_deref(), Some("eng"));
        let ms = |d: Duration| d.as_millis();
        let cues: Vec<_> = t.cues.iter().map(|c| (ms(c.start), ms(c.end), c.text.as_str())).collect();
        assert_eq!(cues, [(500, 1500, "One"), (1500, 2500, "Two & more")]);
        // The plain clip has none, and something that is not an MP4 has none.
        assert!(text_tracks(&path.with_file_name("clip.mp4")).unwrap_or_default().is_empty());
        assert!(text_tracks(&path.with_file_name("clip.vtt")).unwrap_or_default().is_empty());
    }

    #[test]
    fn timed_text_samples_are_bounded_by_their_length() {
        assert_eq!(cue_text(b"\x00\x03OneTRAILING-STYLE"), "One");
        assert_eq!(cue_text(b"\x00\x40short"), "short");
        assert_eq!(cue_text(b"\x00\x06\xFE\xFF\x00h\x00i"), "hi");
        assert_eq!(cue_text(b"\x00"), "");
    }

    #[test]
    fn tables_that_claim_billions_of_samples_are_not_walked() {
        let be = |v: &[u32]| -> Vec<u8> { v.iter().flat_map(|n| n.to_be_bytes()).collect() };
        // Two one-byte samples in one chunk; a run of none first; times for 4 billion samples
        // of the longest length each, shown before they are decoded, and the latest edit start.
        let stsz = be(&[0, 1, 2]);
        let stco = be(&[0, 1, 0]);
        let stsc = be(&[0, 2, 1, 0, 1, 1, 2, 1]);
        let stts = be(&[0, 1, u32::MAX, u32::MAX]);
        let ctts = be(&[0x0100_0000, 1, u32::MAX, u32::MAX]);
        let stbl = [
            Atom { kind: *b"stsz", body: &stsz },
            Atom { kind: *b"stco", body: &stco },
            Atom { kind: *b"stsc", body: &stsc },
            Atom { kind: *b"stts", body: &stts },
            Atom { kind: *b"ctts", body: &ctts },
        ];
        let got = samples(&stbl, i64::MAX, 2).unwrap_or_default();
        assert_eq!(got.iter().map(|s| (s.offset, s.size)).collect::<Vec<_>>(), [(0, 1), (1, 1)]);
        let track = Track {
            kind: Kind::Text { language: None },
            codec: String::new(),
            description: Vec::new(),
            timescale: 1,
            samples: got,
        };
        assert_eq!(track.length(), Duration::ZERO, "times before the start are zero");
    }

    #[test]
    fn damaged_files_are_refused_not_trusted() {
        assert!(parse(b"not an mp4 at all").is_err());
        let data = clip();
        for cut in [16, data.len() / 3, data.len() - 1] {
            let short = data.get(..cut).unwrap_or_default();
            // A file cut short either still reads (its samples all inside it) or says why.
            if let Ok(m) = parse(short) {
                for t in &m.tracks {
                    for s in &t.samples {
                        assert!(s.offset + u64::from(s.size) <= short.len() as u64);
                    }
                }
            }
        }
        let mut bad = data.clone();
        // A box claiming to be larger than the file.
        if let Some(b) = bad.get_mut(0..4) {
            b.copy_from_slice(&u32::MAX.to_be_bytes());
        }
        assert!(parse(&bad).is_err());
    }
}
