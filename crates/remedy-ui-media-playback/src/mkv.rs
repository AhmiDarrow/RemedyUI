//! Reading the text subtitle tracks of Matroska and WebM files: SubRip (`S_TEXT/UTF8`),
//! ASS/SSA (`S_TEXT/ASS`, `S_TEXT/SSA`) and WebVTT (`S_TEXT/WEBVTT`, WebM's
//! `D_WEBVTT/SUBTITLES` and `D_WEBVTT/CAPTIONS`), with their languages. The platforms'
//! readers do not hand text out of these files (GStreamer does, seconds late), so the player
//! reads them itself.
//!
//! Only the header elements (the seek head, segment info and tracks), the cue index and the
//! subtitle blocks are read: the index says where each subtitle block is, so a long film
//! costs a few small reads. A file without an index (written to a pipe, a live recording) is
//! walked cluster by cluster, reading the head of each block and skipping the pictures and
//! sound. Untrusted input: every size is checked against its parent and the file, nothing
//! larger than a bound below is read into memory, and a file that does not add up is
//! refused.
//!
//! Bitmap subtitles (PGS `S_HDMV/PGS`, VobSub `S_VOBSUB`, DVB) are pictures, not text, and
//! are left out, as are tracks whose blocks are compressed with zlib or encrypted.

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use crate::host::MediaError;
use crate::subtitles::{
    AssHeader, CUES_TOO_BIG, Cue, CueSettings, SubtitleTrack, ass_text, clean_markup, over_cue_budget,
    srt_settings,
};

/// Why a file could not be read as Matroska.
pub(crate) type Error = String;

// Element IDs (with their length marker, as written).
const EBML: u32 = 0x1A45_DFA3;
const DOC_TYPE: u32 = 0x4282;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const NAME: u32 = 0x536E;
const CODEC_PRIVATE: u32 = 0x63A2;
const LANGUAGE: u32 = 0x22_B59C;
const LANGUAGE_BCP47: u32 = 0x22_B59D;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const CONTENT_ENCODING: u32 = 0x6240;
const CONTENT_COMPRESSION: u32 = 0x5034;
const CONTENT_COMP_ALGO: u32 = 0x4254;
const CONTENT_COMP_SETTINGS: u32 = 0x4255;
const CONTENT_ENCRYPTION: u32 = 0x5035;
const CLUSTER: u32 = 0x1F43_B675;
const TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CUE_CLUSTER_POSITION: u32 = 0xF1;
const CUE_RELATIVE_POSITION: u32 = 0xF0;

/// The elements that sit directly in a segment: one of them ends a cluster of unknown size.
const TOP_LEVEL: [u32; 9] =
    [SEEK_HEAD, INFO, TRACKS, CLUSTER, CUES, 0x1043_A770, 0x1941_A469, 0x1254_C367, SEGMENT];

/// The EBML header is a handful of short fields.
const MAX_EBML_HEADER: u64 = 4 << 10;
/// The largest seek head, segment info or track list read: far above any real file's.
const MAX_HEADER: u64 = 16 << 20;
/// The largest cue index read: a feature film's is a few megabytes.
const MAX_CUES: u64 = 64 << 20;
/// The largest subtitle block read; a cue is a line or two.
const MAX_CUE: u64 = 64 << 10;
/// How long a cue lasts when its block says nothing and no cue follows it.
const LAST_CUE: Duration = Duration::from_secs(5);

/// The text subtitle tracks in the Matroska or WebM file at `path`; none when it is not one
/// or holds none (or cannot be read: a damaged file shows no embedded captions rather than
/// wrong ones). Their text over the subtitle file limit is an error.
pub(crate) fn text_tracks(path: &Path) -> Result<Vec<SubtitleTrack>, MediaError> {
    let Ok(f) = std::fs::File::open(path) else { return Ok(Vec::new()) };
    let Ok(len) = f.metadata().map(|m| m.len()) else { return Ok(Vec::new()) };
    match read(BufReader::with_capacity(16 << 10, f), len) {
        Err(e) if e == CUES_TOO_BIG => Err(MediaError::Format(e)),
        r => Ok(r.unwrap_or_default()),
    }
}

/// How a track's block holds its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// SubRip: the text, with SRT's tags.
    Srt,
    /// ASS/SSA: `ReadOrder, Layer, Style, Name, MarginL, MarginR, MarginV, Effect, Text`.
    Ass,
    /// Matroska's WebVTT: the cue text (its settings are in block additions).
    Vtt,
    /// WebM's WebVTT: the cue's identifier line, its settings line, then its text.
    WebmVtt,
}

/// A subtitle track from the track list.
#[derive(Clone, Debug)]
struct TextTrack {
    number: u64,
    format: Format,
    name: Option<String>,
    language: Option<String>,
    /// Bytes every block of the track had taken off its front ("header stripping").
    prefix: Vec<u8>,
    cues: Vec<(Duration, Option<Duration>, String, CueSettings)>,
    /// An ASS track's script header (its codec private data): size and styles.
    ass: AssHeader,
}

/// An element: its ID, where its data starts and how long it is (`None`: unknown, running
/// to the end of its parent).
#[derive(Clone, Copy, Debug)]
struct Element {
    id: u32,
    data: u64,
    size: Option<u64>,
}

impl Element {
    /// Where it ends, within a parent ending at `end`.
    fn end(&self, end: u64) -> u64 {
        self.size.map_or(end, |s| self.data + s)
    }
}

/// The file, read at offsets.
struct Src<R> {
    r: BufReader<R>,
    /// Where the reader is.
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> Src<R> {
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Result<(), Error> {
        if at.checked_add(buf.len() as u64).is_none_or(|end| end > self.len) {
            return Err("a read past the end of the file".into());
        }
        if at != self.pos {
            // A short hop stays in the read buffer.
            let hop = i64::try_from(at).ok().zip(i64::try_from(self.pos).ok()).map(|(a, p)| a - p);
            match hop {
                Some(h) => self.r.seek_relative(h),
                None => self.r.seek(SeekFrom::Start(at)).map(drop),
            }
            .map_err(|e| e.to_string())?;
        }
        self.r.read_exact(buf).map_err(|e| e.to_string())?;
        self.pos = at + buf.len() as u64;
        Ok(())
    }

    /// The element whose header is at `at`, inside a parent ending at `end`.
    fn element(&mut self, at: u64, end: u64) -> Result<Element, Error> {
        let mut h = [0u8; 12];
        let n = end.saturating_sub(at).min(12) as usize;
        let head = h.get_mut(..n).ok_or("short element")?;
        self.read_at(at, head)?;
        let (id, a) = vint(head, true).filter(|(_, l)| *l <= 4).ok_or("an element ID that cannot be read")?;
        let (size, b) =
            vint(head.get(a..).unwrap_or_default(), false).ok_or("an element size that cannot be read")?;
        let unknown = size == (1u64 << (7 * b)) - 1;
        let data = at + (a + b) as u64;
        let size = (!unknown).then_some(size);
        if data > end || size.is_some_and(|s| s > end - data) {
            return Err(format!("element {id:#x} runs past its parent"));
        }
        Ok(Element { id: id as u32, data, size })
    }

    /// An element's data, when it has a known size no larger than `max`.
    fn body(&mut self, e: &Element, max: u64) -> Result<Vec<u8>, Error> {
        let size = e.size.ok_or_else(|| format!("element {:#x} of unknown size", e.id))?;
        if size > max {
            return Err(format!("element {:#x} is too large", e.id));
        }
        let mut b = vec![0u8; size as usize];
        self.read_at(e.data, &mut b)?;
        Ok(b)
    }
}

/// An EBML variable-length integer at the start of `d`, with the length marker kept (an
/// element ID) or taken off (a size or a track number), and its length in bytes.
fn vint(d: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *d.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    let mut v = if keep_marker { u64::from(first) } else { u64::from(first) & (0xFF >> len) };
    for b in d.get(1..len)? {
        v = (v << 8) | u64::from(*b);
    }
    Some((v, len))
}

/// The children of an element held in memory, as (ID, data).
fn children(mut d: &[u8]) -> Result<Vec<(u32, &[u8])>, Error> {
    let mut out = Vec::new();
    while !d.is_empty() {
        let (id, a) = vint(d, true).filter(|(_, l)| *l <= 4).ok_or("an element ID that cannot be read")?;
        let rest = d.get(a..).unwrap_or_default();
        let (size, b) = vint(rest, false).ok_or("an element size that cannot be read")?;
        let body = rest.get(b..).unwrap_or_default();
        let size = usize::try_from(size)
            .ok()
            .filter(|s| *s <= body.len())
            .ok_or("an element runs past its parent")?;
        out.push((id as u32, body.get(..size).unwrap_or_default()));
        d = body.get(size..).unwrap_or_default();
    }
    Ok(out)
}

/// A big-endian unsigned integer element (up to eight bytes).
fn uint(d: &[u8]) -> Result<u64, Error> {
    if d.len() > 8 {
        return Err("an integer longer than eight bytes".into());
    }
    Ok(d.iter().fold(0u64, |v, b| (v << 8) | u64::from(*b)))
}

/// A string element (padded with zeros at its end).
fn string(d: &[u8]) -> String {
    String::from_utf8_lossy(d).trim_end_matches('\0').trim().to_owned()
}

fn read<R: Read + Seek>(r: BufReader<R>, len: u64) -> Result<Vec<SubtitleTrack>, Error> {
    let mut src = Src { r, pos: 0, len };
    if len < 4 {
        return Ok(Vec::new());
    }
    // The EBML header: a Matroska or WebM document, or not ours at all.
    let mut magic = [0u8; 4];
    src.read_at(0, &mut magic)?;
    if u32::from_be_bytes(magic) != EBML {
        return Ok(Vec::new());
    }
    let header = src.element(0, len)?;
    let doc = src.body(&header, MAX_EBML_HEADER)?;
    let doc_type = children(&doc)?.into_iter().find(|(id, _)| *id == DOC_TYPE).map(|(_, d)| string(d));
    if !matches!(doc_type.as_deref(), Some("matroska" | "webm")) {
        return Ok(Vec::new());
    }
    let segment = src.element(header.end(len), len)?;
    if segment.id != SEGMENT {
        return Err("no segment after the EBML header".into());
    }
    let (seg, seg_end) = (segment.data, segment.end(len));

    // The segment's elements up to the first cluster, then those the seek head points at.
    let mut found: Vec<(u32, u64)> = Vec::new();
    let mut first_cluster = None;
    let mut at = seg;
    while at < seg_end {
        let e = src.element(at, seg_end)?;
        if e.id == CLUSTER {
            first_cluster = Some(at);
            break;
        }
        let Some(size) = e.size else { return Err(format!("element {:#x} of unknown size", e.id)) };
        if e.id == SEEK_HEAD {
            for (id, pos) in seek_head(&src.body(&e, MAX_HEADER)?)? {
                if let Some(p) = seg.checked_add(pos).filter(|p| *p < seg_end) {
                    found.push((id, p));
                }
            }
        }
        found.insert(0, (e.id, at));
        at = e.data + size;
    }
    let locate = |src: &mut Src<R>, id: u32| -> Result<Option<Element>, Error> {
        for (_, p) in found.iter().filter(|(i, _)| *i == id) {
            let e = src.element(*p, seg_end)?;
            if e.id == id {
                return Ok(Some(e));
            }
        }
        Ok(None)
    };
    let scale = match locate(&mut src, INFO)? {
        Some(info) => timestamp_scale(&src.body(&info, MAX_HEADER)?)?,
        None => 1_000_000,
    };
    let Some(tracks) = locate(&mut src, TRACKS)? else { return Ok(Vec::new()) };
    let mut tracks = text_track_list(&src.body(&tracks, MAX_HEADER)?)?;
    if tracks.is_empty() {
        return Ok(Vec::new());
    }

    // Where the subtitle blocks are: from the cue index when it lists every subtitle track,
    // else from every cluster.
    let mut places: Vec<Place> = Vec::new();
    if let Some(cues) = locate(&mut src, CUES)? {
        let numbers: Vec<u64> = tracks.iter().map(|t| t.number).collect();
        let (listed, p) = cue_places(&src.body(&cues, MAX_CUES)?, &numbers)?;
        if numbers.iter().all(|n| listed.contains(n)) {
            places = p
                .into_iter()
                .filter_map(|(c, r)| seg.checked_add(c).filter(|c| *c < seg_end).map(|c| (c, r)))
                .collect();
            places.sort_unstable();
            places.dedup();
        }
    }
    let mut reader =
        Blocks { src: &mut src, tracks: &mut tracks, scale, seg_end, seen: Vec::new(), text_bytes: 0 };
    if places.is_empty() {
        let mut at = first_cluster.unwrap_or(seg_end);
        while at < seg_end {
            let e = reader.src.element(at, seg_end)?;
            at = if e.id == CLUSTER {
                reader.cluster(&e, None)?
            } else {
                e.size.map(|s| e.data + s).ok_or("an element of unknown size")?
            };
        }
    } else {
        let mut i = 0;
        while let Some(&(cluster, _)) = places.get(i) {
            let e = reader.src.element(cluster, seg_end)?;
            if e.id != CLUSTER {
                return Err("the cue index points at something that is not a cluster".into());
            }
            let mut rel = Vec::new();
            while let Some(&(_, r)) = places.get(i).filter(|(c, _)| *c == cluster) {
                rel.push(r);
                i += 1;
            }
            // An entry without its block's place has the whole cluster read.
            let rel: Option<Vec<u64>> = rel.into_iter().collect();
            reader.cluster(&e, rel.as_deref())?;
        }
    }

    Ok(tracks.into_iter().filter_map(finish).collect())
}

/// The (ID, position in the segment) entries of a seek head.
fn seek_head(d: &[u8]) -> Result<Vec<(u32, u64)>, Error> {
    let mut out = Vec::new();
    for (id, seek) in children(d)? {
        if id != SEEK {
            continue;
        }
        let fields = children(seek)?;
        let target = fields.iter().find(|(i, _)| *i == SEEK_ID).map(|(_, d)| uint(d)).transpose()?;
        let pos = fields.iter().find(|(i, _)| *i == SEEK_POSITION).map(|(_, d)| uint(d)).transpose()?;
        if let (Some(t), Some(p)) = (target, pos) {
            out.push((u32::try_from(t).map_err(|_| "a seek ID longer than four bytes")?, p));
        }
    }
    Ok(out)
}

/// Nanoseconds per timestamp unit, from segment info.
fn timestamp_scale(d: &[u8]) -> Result<u64, Error> {
    let scale =
        children(d)?.into_iter().find(|(id, _)| *id == TIMESTAMP_SCALE).map(|(_, d)| uint(d)).transpose()?;
    match scale {
        None => Ok(1_000_000),
        Some(s) if (1..=1_000_000_000).contains(&s) => Ok(s),
        Some(_) => Err("a timestamp scale out of range".into()),
    }
}

/// The text subtitle tracks of a track list.
fn text_track_list(d: &[u8]) -> Result<Vec<TextTrack>, Error> {
    let mut out: Vec<TextTrack> = Vec::new();
    for (id, entry) in children(d)? {
        if id != TRACK_ENTRY {
            continue;
        }
        let fields = children(entry)?;
        let field = |id: u32| fields.iter().find(|(i, _)| *i == id).map(|(_, d)| *d);
        let number = field(TRACK_NUMBER).map(uint).transpose()?.unwrap_or(0);
        let kind = field(TRACK_TYPE).map(uint).transpose()?;
        let format = match field(CODEC_ID).map(string).as_deref() {
            Some("S_TEXT/UTF8") => Format::Srt,
            Some("S_TEXT/ASS" | "S_TEXT/SSA") => Format::Ass,
            Some("S_TEXT/WEBVTT") => Format::Vtt,
            Some("D_WEBVTT/SUBTITLES" | "D_WEBVTT/CAPTIONS") => Format::WebmVtt,
            _ => continue,
        };
        // Subtitles are type 17; WebM's WebVTT tracks are type 17 too.
        if number == 0 || kind.is_some_and(|k| k != 17) || out.iter().any(|t| t.number == number) {
            continue;
        }
        let Some(prefix) = field(CONTENT_ENCODINGS).map(stripped_prefix).transpose()? else {
            out.push(text_track(number, format, &field));
            continue;
        };
        // Compressed with anything but header stripping, or encrypted: not readable here.
        let Some(prefix) = prefix else { continue };
        let mut t = text_track(number, format, &field);
        t.prefix = prefix;
        out.push(t);
    }
    Ok(out)
}

fn text_track<'a>(number: u64, format: Format, field: &impl Fn(u32) -> Option<&'a [u8]>) -> TextTrack {
    // Matroska's default language is English; "und" is none.
    let language = field(LANGUAGE_BCP47).or_else(|| field(LANGUAGE)).map_or_else(|| "eng".to_owned(), string);
    let language = (!language.is_empty() && language != "und").then_some(language);
    let name = field(NAME).map(string).filter(|n| !n.is_empty());
    let ass = match format {
        Format::Ass => {
            field(CODEC_PRIVATE).map(|d| AssHeader::parse(&String::from_utf8_lossy(d))).unwrap_or_default()
        }
        _ => AssHeader::default(),
    };
    TextTrack { number, format, name, language, prefix: Vec::new(), cues: Vec::new(), ass }
}

/// What header stripping takes off each block, `Some(empty)` for nothing; `None` when the
/// track's blocks are compressed another way or encrypted.
fn stripped_prefix(d: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    let mut prefix = Vec::new();
    for (id, enc) in children(d)? {
        if id != CONTENT_ENCODING {
            continue;
        }
        for (id, part) in children(enc)? {
            if id == CONTENT_ENCRYPTION {
                return Ok(None);
            }
            if id != CONTENT_COMPRESSION {
                continue;
            }
            let fields = children(part)?;
            let algo =
                fields.iter().find(|(i, _)| *i == CONTENT_COMP_ALGO).map(|(_, d)| uint(d)).transpose()?;
            if algo != Some(3) {
                return Ok(None);
            }
            if let Some((_, s)) = fields.iter().find(|(i, _)| *i == CONTENT_COMP_SETTINGS) {
                prefix.extend_from_slice(s);
            }
        }
    }
    Ok(Some(prefix))
}

/// From a cue index: the tracks among `numbers` it lists, and for those, each (cluster
/// position in the segment, block position in the cluster's data when given).
fn cue_places(d: &[u8], numbers: &[u64]) -> Result<(Vec<u64>, Vec<Place>), Error> {
    let (mut listed, mut places) = (Vec::new(), Vec::new());
    for (id, point) in children(d)? {
        if id != CUE_POINT {
            continue;
        }
        for (id, pos) in children(point)? {
            if id != CUE_TRACK_POSITIONS {
                continue;
            }
            let fields = children(pos)?;
            let field = |id: u32| fields.iter().find(|(i, _)| *i == id).map(|(_, d)| uint(d)).transpose();
            let (Some(track), Some(cluster)) = (field(CUE_TRACK)?, field(CUE_CLUSTER_POSITION)?) else {
                continue;
            };
            if numbers.contains(&track) {
                if !listed.contains(&track) {
                    listed.push(track);
                }
                places.push((cluster, field(CUE_RELATIVE_POSITION)?));
            }
        }
    }
    Ok((listed, places))
}

/// Where a subtitle block is: its cluster's position in the segment, and its own in the
/// cluster's data when the index gives it.
type Place = (u64, Option<u64>);

/// Reads subtitle blocks out of clusters.
struct Blocks<'a, R> {
    src: &'a mut Src<R>,
    tracks: &'a mut [TextTrack],
    scale: u64,
    seg_end: u64,
    /// Blocks already read (an index may name one twice).
    seen: Vec<u64>,
    /// The bytes of cue text read, against the subtitle file limit.
    text_bytes: u64,
}

impl<R: Read + Seek> Blocks<'_, R> {
    /// Reads a cluster's subtitle blocks: those at `only` (positions in its data), or all of
    /// them. Returns where the cluster ends.
    fn cluster(&mut self, c: &Element, only: Option<&[u64]>) -> Result<u64, Error> {
        let end = c.end(self.seg_end);
        let mut timestamp = None;
        let mut at = c.data;
        // Its timestamp comes before its blocks.
        while at < end {
            let e = self.src.element(at, end)?;
            if c.size.is_none() && TOP_LEVEL.contains(&e.id) {
                return Ok(at);
            }
            let size = e.size.ok_or("an element of unknown size in a cluster")?;
            match e.id {
                TIMESTAMP => timestamp = Some(uint(&self.src.body(&e, 8)?)?),
                SIMPLE_BLOCK | BLOCK_GROUP if only.is_none() => {
                    let ts = timestamp.ok_or("a block before its cluster's timestamp")?;
                    self.block(&e, ts)?;
                }
                _ => {}
            }
            if only.is_some() && timestamp.is_some() {
                break;
            }
            at = e.data + size;
        }
        if let Some(only) = only {
            let ts = timestamp.ok_or("a cluster without its timestamp")?;
            for rel in only {
                let p = c
                    .data
                    .checked_add(*rel)
                    .filter(|p| *p < end)
                    .ok_or("a cue points outside its cluster")?;
                let e = self.src.element(p, end)?;
                if !matches!(e.id, SIMPLE_BLOCK | BLOCK_GROUP) {
                    return Err("a cue points at something that is not a block".into());
                }
                self.block(&e, ts)?;
            }
        }
        Ok(at)
    }

    /// Reads one block (a simple block or a block group) if it is a subtitle track's.
    fn block(&mut self, e: &Element, cluster_ts: u64) -> Result<(), Error> {
        if self.seen.contains(&e.data) {
            return Ok(());
        }
        let size = e.size.ok_or("a block of unknown size")?;
        // Its track first, from its head: pictures and sound are skipped unread.
        let mut head = [0u8; 16];
        let head = head.get_mut(..size.min(16) as usize).unwrap_or_default();
        self.src.read_at(e.data, head)?;
        let block_head = if e.id == BLOCK_GROUP {
            let (id, a) = vint(head, true).ok_or("a block group that cannot be read")?;
            if id != u64::from(BLOCK) {
                // The block is not first in its group: the group is read to find it.
                &[][..]
            } else {
                let (_, b) = vint(head.get(a..).unwrap_or_default(), false)
                    .ok_or("a block group that cannot be read")?;
                head.get(a + b..).unwrap_or_default()
            }
        } else {
            head
        };
        if let Some((track, _)) = vint(block_head, false)
            && !self.tracks.iter().any(|t| t.number == track)
        {
            return Ok(());
        }
        if size > MAX_CUE {
            return Ok(());
        }
        self.seen.push(e.data);
        let body = self.src.body(e, MAX_CUE)?;
        let (block, duration) = if e.id == BLOCK_GROUP {
            let fields = children(&body)?;
            let block = fields
                .iter()
                .find(|(i, _)| *i == BLOCK)
                .map(|(_, d)| *d)
                .ok_or("a block group without its block")?;
            let duration =
                fields.iter().find(|(i, _)| *i == BLOCK_DURATION).map(|(_, d)| uint(d)).transpose()?;
            (block, duration)
        } else {
            (body.as_slice(), None)
        };
        let (track, n) = vint(block, false).ok_or("a block that cannot be read")?;
        let Some(t) = self.tracks.iter_mut().find(|t| t.number == track) else { return Ok(()) };
        let (Some(&[hi, lo]), Some(&flags)) = (block.get(n..n + 2), block.get(n + 2)) else {
            return Err("a block shorter than its header".into());
        };
        // Laced blocks hold several frames; text is never laced.
        if flags & 0x06 != 0 {
            return Ok(());
        }
        let tc = i64::from(i16::from_be_bytes([hi, lo]));
        let at = i64::try_from(cluster_ts).ok().and_then(|c| c.checked_add(tc));
        let time = |units: u64| units.checked_mul(self.scale).map(Duration::from_nanos);
        let Some(start) = at.and_then(|a| u64::try_from(a).ok()).and_then(time) else { return Ok(()) };
        let end = match duration {
            Some(d) => {
                Some(time(d).and_then(|d| start.checked_add(d)).ok_or("a block duration out of range")?)
            }
            None => None,
        };
        let mut payload = t.prefix.clone();
        payload.extend_from_slice(block.get(n + 3..).unwrap_or_default());
        let text = cue_text(&payload, t.format);
        if !text.is_empty() {
            if over_cue_budget(&mut self.text_bytes, &text) {
                return Err(CUES_TOO_BIG.into());
            }
            let settings = cue_settings(&payload, t.format, &t.ass);
            t.cues.push((start, end, text, settings));
        }
        Ok(())
    }
}

/// A subtitle block's text, as plain text with its line breaks. Matroska's text is UTF-8:
/// a block that is not (or holds control characters other than line breaks and tabs) is
/// not text, and has none.
fn cue_text(b: &[u8], format: Format) -> String {
    let Ok(s) = std::str::from_utf8(b) else { return String::new() };
    if s.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return String::new();
    }
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let text = match format {
        Format::Ass => {
            // Eight fields, then the text (which may hold commas).
            let mut fields = s.splitn(9, ',');
            match fields.nth(8) {
                Some(t) => return ass_text(t),
                None => return String::new(),
            }
        }
        Format::WebmVtt => s.splitn(3, '\n').nth(2).unwrap_or_default().to_owned(),
        Format::Srt | Format::Vtt => s,
    };
    let lines: Vec<String> = text.lines().map(clean_markup).collect();
    lines.join("\n").trim().to_owned()
}

/// Where a subtitle block's cue goes: WebM's WebVTT settings line, SRT's `{\anN}`, and an
/// ASS block's `\anN`, style and margins read against the track's script header.
fn cue_settings(b: &[u8], format: Format, ass: &AssHeader) -> CueSettings {
    let s = String::from_utf8_lossy(b).replace("\r\n", "\n");
    match format {
        // ReadOrder, Layer, Style, Name, MarginL, MarginR, MarginV, Effect, Text.
        Format::Ass => {
            let f: Vec<&str> = s.splitn(9, ',').collect();
            let num = |i: usize| f.get(i).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
            ass.settings(
                f.get(2).copied().unwrap_or_default(),
                [num(4), num(5), num(6)],
                f.get(8).copied().unwrap_or_default(),
            )
        }
        Format::WebmVtt => CueSettings::parse_webvtt(s.split('\n').nth(1).unwrap_or_default()),
        Format::Srt | Format::Vtt => srt_settings(&s),
    }
}

/// A track's cues in time order, those without a duration lasting until the next; `None`
/// when it has none.
fn finish(t: TextTrack) -> Option<SubtitleTrack> {
    let mut raw = t.cues;
    raw.sort_by_key(|c| c.0);
    let starts: Vec<Duration> = raw.iter().map(|c| c.0).collect();
    let cues: Vec<Cue> = raw
        .into_iter()
        .enumerate()
        .filter_map(|(i, (start, end, text, settings))| {
            let end = end.unwrap_or_else(|| {
                starts
                    .get(i + 1..)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .find(|s| *s > start)
                    .unwrap_or(start + LAST_CUE)
            });
            (end > start).then_some(Cue { start, end, text, settings })
        })
        .collect();
    if cues.is_empty() {
        return None;
    }
    let label = t.name.clone().or_else(|| t.language.clone()).unwrap_or_else(|| "Subtitles".into());
    Some(SubtitleTrack { label, language: t.language, cues })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    fn parse(data: &[u8]) -> Result<Vec<SubtitleTrack>, Error> {
        read(BufReader::new(Cursor::new(data.to_vec())), data.len() as u64)
    }

    fn cues(t: &SubtitleTrack) -> Vec<(u128, u128, &str)> {
        t.cues.iter().map(|c| (c.start.as_millis(), c.end.as_millis(), c.text.as_str())).collect()
    }

    /// Each track's label, language and cues (start and end in milliseconds, text).
    type Summary<'a> = Vec<(&'a str, Option<&'a str>, Vec<(u128, u128, &'a str)>)>;

    fn summary(tracks: &[SubtitleTrack]) -> Summary<'_> {
        tracks.iter().map(|t| (t.label.as_str(), t.language.as_deref(), cues(t))).collect()
    }

    /// The three tracks of `clip-subs.mkv` (and its streamed copy): ffmpeg shifted the whole
    /// file 21 ms later, to the sound's first sample.
    fn mkv_tracks() -> Summary<'static> {
        vec![
            ("eng", Some("eng"), vec![(521, 1521, "One"), (1521, 2521, "Two & more")]),
            ("fre", Some("fre"), vec![(21, 3021, "Un")]),
            ("Subtitles", None, vec![(521, 1521, "One"), (1521, 2521, "Two & more")]),
        ]
    }

    #[test]
    fn srt_ass_and_webvtt_tracks_are_read_with_their_languages() {
        let tracks = text_tracks(&fixture("clip-subs.mkv")).unwrap_or_default();
        assert_eq!(summary(&tracks), mkv_tracks());
    }

    #[test]
    fn a_webm_file_s_webvtt_track_is_read_across_its_clusters() {
        let tracks = text_tracks(&fixture("clip-subs.webm")).unwrap_or_default();
        assert_eq!(
            summary(&tracks),
            [("eng", Some("eng"), vec![(500, 1500, "One"), (1500, 2500, "Two & more")])]
        );
    }

    #[test]
    fn a_file_without_an_index_is_walked_cluster_by_cluster() {
        // Written to a pipe: a segment of unknown size, no cue index; the clusters differ.
        let tracks = text_tracks(&fixture("clip-subs-streamed.mkv")).unwrap_or_default();
        assert_eq!(summary(&tracks), mkv_tracks());
    }

    #[test]
    fn other_files_have_none() {
        for name in ["clip.mp4", "clip-subs.mp4", "clip.vtt", "no-such-file.mkv"] {
            assert!(text_tracks(&fixture(name)).unwrap_or_default().is_empty(), "{name}");
        }
        assert_eq!(parse(b""), Ok(Vec::new()));
        // An EBML document of another type.
        assert_eq!(parse(b"\x1A\x45\xDF\xA3\x87\x42\x82\x84abcd"), Ok(Vec::new()));
    }

    #[test]
    fn block_placement_in_each_format() {
        use crate::subtitles::{CueAlign, CueLine};
        assert_eq!(
            cue_settings(b"{\\an8}Top", Format::Srt, &AssHeader::default()).line,
            Some(CueLine::Percent(0.0))
        );
        let s = cue_settings(b"3,0,Default,,96,0,0,,{\\an1}Hi", Format::Ass, &AssHeader::default());
        assert_eq!(
            (s.line, s.align, s.position),
            (Some(CueLine::Percent(100.0)), CueAlign::Left, Some(25.0))
        );
        // The track's script header (codec private data): a style's alignment and margins.
        let header = AssHeader::parse(
            "[Script Info]\nPlayResX: 400\nPlayResY: 200\n\n[V4+ Styles]\nFormat: Name, Alignment, MarginL, MarginR, MarginV\nStyle: Sign,8,40,40,20\n",
        );
        let s = cue_settings(b"3,0,Sign,,0,0,0,,Hi", Format::Ass, &header);
        assert_eq!(
            (s.line, s.align, s.box_across()),
            (Some(CueLine::Percent(10.0)), CueAlign::Center, (10.0, 80.0))
        );
        assert_eq!(cue_settings(b"3,0,Default,,0,0,0,,Hi", Format::Ass, &header), CueSettings::default());
        let s = cue_settings(b"id\nline:0 align:start\nHi", Format::WebmVtt, &AssHeader::default());
        assert_eq!((s.line, s.align), (Some(CueLine::Number(0)), CueAlign::Start));
        assert_eq!(cue_settings(b"plain", Format::Vtt, &AssHeader::default()), CueSettings::default());
    }

    #[test]
    fn block_text_in_each_format() {
        assert_eq!(cue_text(b"{\\an8}<b>Top</b>\r\nline", Format::Srt), "Top\nline");
        assert_eq!(
            cue_text(b"3,0,Default,,0,0,0,,{\\i1}Hi,{\\i0} you\\Nthere", Format::Ass),
            "Hi, you\nthere"
        );
        assert_eq!(cue_text(b"too,few,fields", Format::Ass), "");
        assert_eq!(cue_text(b"id\nline:0\n<v Al>Hi</v> &amp; bye", Format::WebmVtt), "Hi & bye");
        assert_eq!(cue_text(b"no lines", Format::WebmVtt), "");
        assert_eq!(cue_text(b"<i>Plain</i>", Format::Vtt), "Plain");
        // Not text: not UTF-8, or binary.
        assert_eq!(cue_text(b"\xFF\xFEbad", Format::Srt), "");
        assert_eq!(cue_text(b"a\0b", Format::Srt), "");
    }

    #[test]
    fn header_stripping_is_undone_and_other_encodings_leave_the_track_out() {
        let stripped = b"\x62\x40\x8C\x50\x34\x89\x42\x54\x81\x03\x42\x55\x82ab";
        assert_eq!(stripped_prefix(stripped), Ok(Some(b"ab".to_vec())));
        let zlib = b"\x62\x40\x87\x50\x34\x84\x42\x54\x81\x00";
        assert_eq!(stripped_prefix(zlib), Ok(None));
        let encrypted = b"\x62\x40\x83\x50\x35\x80";
        assert_eq!(stripped_prefix(encrypted), Ok(None));
        assert!(stripped_prefix(b"\x62\x40\x8F\x50").is_err(), "an encoding longer than its list");
    }

    #[test]
    fn an_ass_tracks_codec_private_gives_its_styles() {
        // EBML elements with one-byte sizes.
        fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
            let mut v = id.to_vec();
            v.push(0x80 | u8::try_from(body.len()).unwrap_or_else(|_| unreachable!("short")));
            v.extend_from_slice(body);
            v
        }
        let header =
            b"[Script Info]\nPlayResY: 200\n\n[V4+ Styles]\nFormat: Name, Alignment\nStyle: Sign,8\n";
        let entry =
            [el(&[0xD7], &[1]), el(&[0x83], &[17]), el(&[0x86], b"S_TEXT/ASS"), el(&[0x63, 0xA2], header)]
                .concat();
        let tracks = text_track_list(&el(&[0xAE], &entry)).unwrap_or_else(|e| unreachable!("{e}"));
        let t = tracks.first().unwrap_or_else(|| unreachable!("one track"));
        let s = cue_settings(b"1,0,Sign,,0,0,0,,Hi", t.format, &t.ass);
        assert_eq!(
            (s.line, s.align),
            (Some(crate::subtitles::CueLine::Percent(0.0)), crate::subtitles::CueAlign::Center)
        );
    }

    #[test]
    fn variable_length_integers() {
        assert_eq!(vint(&[0x81], false), Some((1, 1)));
        assert_eq!(vint(&[0x40, 0x02], false), Some((2, 2)));
        assert_eq!(vint(&[0x1A, 0x45, 0xDF, 0xA3], true), Some((0x1A45_DFA3, 4)));
        assert_eq!(vint(&[0x00, 1, 2, 3, 4, 5, 6, 7, 8], false), None, "nine bytes long");
        assert_eq!(vint(&[0x20, 1], false), None, "shorter than it says");
    }

    #[test]
    fn damaged_files_are_refused_not_trusted() {
        let path = fixture("clip-subs.mkv");
        let data = std::fs::read(&path).unwrap_or_else(|e| unreachable!("the fixture: {e}"));
        // Cut short anywhere: refused, or read with every cue one the whole file has.
        let whole = mkv_tracks();
        for cut in (4..data.len()).step_by(97).chain([data.len() - 1]) {
            if let Ok(tracks) = parse(data.get(..cut).unwrap_or_default()) {
                for (label, lang, cues) in summary(&tracks) {
                    let full = whole.iter().find(|t| t.0 == label && t.1 == lang);
                    assert!(
                        full.is_some_and(|f| cues.iter().all(|c| f.2.contains(c))),
                        "cut at {cut}: {label} {cues:?}"
                    );
                }
            }
        }
        // Every byte of the headers, the index and the subtitle blocks flipped in turn:
        // never a panic, never a cue that is not in the file.
        for at in
            (0..1300).chain(28_950..data.len()).chain(2700..2760).chain(6740..6790).chain(15_760..15_830)
        {
            for flip in [0xFF, 0x80, 0x01] {
                let mut bad = data.clone();
                if let Some(b) = bad.get_mut(at) {
                    *b ^= flip;
                }
                if let Ok(tracks) = parse(&bad) {
                    for t in &tracks {
                        for c in &t.cues {
                            assert!(c.end > c.start && !c.text.is_empty(), "flip at {at}: {c:?}");
                            assert!(c.text.len() <= 32, "flip at {at}: {c:?}");
                        }
                    }
                }
            }
        }
        // The segment claiming to be larger than the file.
        let mut bad = data.clone();
        if let Some(b) = bad.get_mut(44..52) {
            b.copy_from_slice(&[0x01, 0, 0, 0, 0, 0x10, 0, 0]);
        }
        assert!(parse(&bad).is_err());
        // The segment ending inside its track list.
        let mut bad = data.clone();
        if let Some(b) = bad.get_mut(44..52) {
            b.copy_from_slice(&[0x01, 0, 0, 0, 0, 0, 0x01, 0xF4]);
        }
        assert!(parse(&bad).is_err(), "{:?}", parse(&bad));
        // The index pointing a cue outside its cluster.
        let mut bad = data;
        if let Some(b) = bad.get_mut(29_065..29_067) {
            b.copy_from_slice(&[0x7F, 0xFF]);
        }
        assert!(parse(&bad).is_err());
    }
}
