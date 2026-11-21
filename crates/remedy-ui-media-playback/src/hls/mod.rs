//! HTTP Live Streaming: a playlist of MPEG-TS segments (`.m3u8`), the form Portal's server and
//! most streaming servers hand out, played on every platform by the crate's own reader.
//!
//! The platforms' readers do not take HLS (Media Foundation refuses an `.m3u8` as "byte stream
//! type unsupported"), so it is taken apart here and only the compressed streams go to the
//! platform: [`Elementary`] decodes H.264 access units and AAC or MP3 frames (Media Foundation
//! transforms on Windows, WebCodecs in a page).
//!
//! The reader works without doing any I/O itself, so a page (which cannot block) and a native
//! thread (which can) drive the same code: it says which URL it wants next ([`Want`]), is
//! handed the bytes, and hands out [`Unit`]s in decoding order.
//!
//! Positions are media time: the first segment of the playlist starts at zero, whatever the
//! stream's own clock says (ffmpeg with `-copyts` starts it 1.4 s in, and MPEG-TS clocks wrap
//! every 26.5 hours).

use crate::host::{MediaError, Sample};
use std::time::Duration;

/// The setup of an H.264 stream, known from the first key frame's parameter sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VideoConfig {
    /// The WebCodecs / RFC 6381 codec string from the SPS (`avc1.64001f`).
    pub codec: String,
    /// The picture shown, in pixels (after the SPS's cropping).
    pub width: u32,
    /// The picture shown, in pixels.
    pub height: u32,
    /// The sequence parameter set, without a start code.
    pub sps: Vec<u8>,
    /// The picture parameter set, without a start code.
    pub pps: Vec<u8>,
}

/// The compressed sound format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AudioCodec {
    /// AAC; `asc` is the AudioSpecificConfig built from the ADTS header (2 bytes for AAC-LC).
    Aac { asc: Vec<u8> },
    /// MPEG-1/2 Layer III.
    Mp3,
}

/// The setup of the sound stream, known from its first frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AudioConfig {
    pub codec: AudioCodec,
    /// The WebCodecs codec string (`mp4a.40.2`, `mp3`).
    pub codec_string: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// One compressed piece, in decoding order. Times are media time (see the module).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    /// One H.264 access unit as an Annex B byte stream (start codes), with its SPS and PPS
    /// before every key frame.
    Video { pts: Duration, dts: Duration, keyframe: bool, data: Vec<u8> },
    /// One AAC frame without its ADTS header, or one whole MP3 frame.
    Audio { pts: Duration, data: Vec<u8> },
}

/// What the reader wants fetched next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Want {
    /// The absolute URL.
    pub url: String,
    /// What it is.
    pub kind: WantKind,
    /// Wait this long before asking (a growing playlist is asked again after half its target
    /// duration; a failed fetch is retried after a pause).
    pub after: Duration,
}

/// What a [`Want`] fetches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WantKind {
    /// A playlist (the first, a variant chosen from a master playlist, or a growing one again).
    Playlist,
    /// The segment with this media sequence number.
    Segment(u64),
}

/// The platform's decoders for the compressed streams. Used from one thread at a time.
pub(crate) trait Elementary: Send {
    /// Sets the decoders up for the streams the media holds (either may be absent). Called
    /// once, before the first [`Elementary::push`].
    fn configure(
        &mut self,
        video: Option<&VideoConfig>,
        audio: Option<&AudioConfig>,
    ) -> Result<(), MediaError>;
    /// Hands over one compressed piece.
    fn push(&mut self, unit: Unit) -> Result<(), MediaError>;
    /// The next decoded sample if one is ready (pictures and sound, each in presentation
    /// order), without waiting; `None` when none is yet.
    fn pull(&mut self) -> Result<Option<Sample>, MediaError>;
    /// No more pieces follow: what is held inside the decoders comes out of `pull`.
    fn end(&mut self) -> Result<(), MediaError>;
    /// Drops everything held (a seek); the next piece pushed is a key frame.
    fn flush(&mut self) -> Result<(), MediaError>;
    /// The sound's format as the decoder gives it, when that is not the stream's own (an
    /// AAC decoder's SBR doubles the rate); known once configured.
    fn audio_format(&self) -> Option<AudioFormat> {
        None
    }
}

/// Fetches a URL whole, waiting for it (native threads).
pub(crate) trait Fetch: Send {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, MediaError>;
}

mod playlist;
mod ts;

use crate::host::{AudioFormat, Decoder, MediaInfo, MediaSource, VideoInfo};
use playlist::{Playlist, Segment};
use std::collections::VecDeque;
use std::ops::Range;

/// Segments fetched ahead of the one being handed out: enough to ride out a slow fetch,
/// few enough to keep memory bounded (a 2160p segment is several megabytes).
const AHEAD: usize = 3;
/// The playlist of a growing stream is asked again once the reader is this many segments
/// from its end.
const NEAR_END: usize = 2;
/// A failed fetch is tried again this many times before the error comes out.
const RETRIES: u32 = 3;
/// How long to wait before trying a failed fetch again.
const RETRY_PAUSE: Duration = Duration::from_secs(1);

/// One unit read from a segment, its times still on the stream's clock (unwrapped 90 kHz).
enum Raw {
    Video { pts: u64, dts: u64, keyframe: bool, data: Vec<u8> },
    Audio { pts: u64, data: Vec<u8> },
}

impl Raw {
    fn pts(&self) -> u64 {
        match self {
            Raw::Video { pts, .. } | Raw::Audio { pts, .. } => *pts,
        }
    }
}

/// The HLS reader, without I/O: it says what it wants fetched ([`Hls::wanted`]), is handed the
/// bytes ([`Hls::deliver`]) and hands out [`Unit`]s ([`Hls::next_unit`]). Nothing in it waits
/// or reads a clock, so a page drives it as a native thread does.
pub(crate) struct Hls {
    /// The media playlist's URL (the variant, once a master playlist chose one).
    url: String,
    /// Whether the media playlist has been read once.
    loaded: bool,
    segments: Vec<Segment>,
    target: Duration,
    /// The playlist is complete.
    complete: bool,
    /// The index in `segments` of the next segment to fetch.
    next: usize,
    /// Units read, each with the index of its segment.
    queue: VecDeque<(usize, Unit)>,
    /// The index of the segment the last unit handed out came from.
    current: usize,
    /// Failed fetches in a row of what is wanted now.
    failures: u32,
    /// The stream's clock at a media time: (unwrapped 90 kHz time, media time, the segment
    /// it was taken at).
    anchor: Option<(u64, Duration, usize)>,
    /// The last time seen on the stream's clock (unwrapped), for unwrapping the next.
    last_ts: Option<u64>,
    /// Video before the first key frame after a start or a seek is dropped.
    need_key: bool,
    /// The stretch held: from where reading started to the end of the last segment fetched.
    held: Range<Duration>,
    video: Option<VideoConfig>,
    audio: Option<AudioConfig>,
    /// A segment has been read (the streams' setups are known).
    started: bool,
}

impl Hls {
    /// A reader for the playlist at `url` (a media or a master playlist).
    pub(crate) fn new(url: &str) -> Self {
        Hls {
            url: url.to_string(),
            loaded: false,
            segments: Vec::new(),
            target: Duration::ZERO,
            complete: false,
            next: 0,
            queue: VecDeque::new(),
            current: 0,
            failures: 0,
            anchor: None,
            last_ts: None,
            need_key: true,
            held: Duration::ZERO..Duration::ZERO,
            video: None,
            audio: None,
            started: false,
        }
    }

    /// What to fetch next, or `None` when nothing is wanted now (enough is held ahead, or the
    /// whole playlist has been fetched).
    pub(crate) fn wanted(&self) -> Option<Want> {
        let pause = if self.failures > 0 { RETRY_PAUSE } else { Duration::ZERO };
        if !self.loaded {
            return Some(Want { url: self.url.clone(), kind: WantKind::Playlist, after: pause });
        }
        let reading = self.queue.front().map_or(self.next, |(i, _)| *i).max(self.current);
        if let Some(s) = self.segments.get(self.next)
            && self.next <= reading + AHEAD
        {
            return Some(Want { url: s.url.clone(), kind: WantKind::Segment(s.sequence), after: pause });
        }
        if !self.complete && reading + NEAR_END >= self.segments.len() {
            let after = if self.failures > 0 { RETRY_PAUSE } else { self.target / 2 };
            return Some(Want { url: self.url.clone(), kind: WantKind::Playlist, after });
        }
        None
    }

    /// Hands over what was fetched for `want`. A failed fetch is asked for again (after a
    /// pause) a few times before its error comes out here; a playlist that cannot be played
    /// is an error at once.
    pub(crate) fn deliver(
        &mut self,
        want: &Want,
        got: Result<Vec<u8>, MediaError>,
    ) -> Result<(), MediaError> {
        // A segment no longer wanted (a seek came while it was fetched) is dropped.
        if let WantKind::Segment(seq) = want.kind
            && self.segments.get(self.next).is_none_or(|s| s.sequence != seq)
        {
            return Ok(());
        }
        let data = match got {
            Ok(d) => d,
            Err(e) => {
                self.failures += 1;
                return if self.failures > RETRIES { Err(e) } else { Ok(()) };
            }
        };
        self.failures = 0;
        match want.kind {
            WantKind::Playlist => self.read_playlist(&want.url, &data),
            WantKind::Segment(_) => {
                self.read_segment(&data);
                Ok(())
            }
        }
    }

    fn read_playlist(&mut self, url: &str, data: &[u8]) -> Result<(), MediaError> {
        let text = String::from_utf8_lossy(data);
        match playlist::parse(&text, url)? {
            Playlist::Master(variant) => {
                if self.loaded {
                    return Err(MediaError::Format("an HLS media playlist became a master playlist".into()));
                }
                self.url = variant;
            }
            Playlist::Media(m) => {
                self.loaded = true;
                self.target = m.target;
                self.complete = m.ended;
                match self.segments.last() {
                    None => self.segments = m.segments,
                    Some(last) => {
                        // Segments are only added at the end; those already known keep their place.
                        let (seq, mut start) = (last.sequence, last.start + last.duration);
                        for mut s in m.segments.into_iter().filter(|s| s.sequence > seq) {
                            s.start = start;
                            start += s.duration;
                            self.segments.push(s);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn read_segment(&mut self, data: &[u8]) {
        let index = self.next;
        self.next += 1;
        let Some(seg) = self.segments.get(index).cloned() else { return };
        // Times go on from the anchor's unless a discontinuity lies between.
        let continuous = self.anchor.is_some_and(|(_, _, at)| {
            let (lo, hi) = (at.min(index), at.max(index));
            !self.segments.get(lo + 1..=hi).is_some_and(|s| s.iter().any(|s| s.discontinuity))
        });
        if !continuous {
            self.last_ts = None;
        }
        let mut raws = Vec::new();
        for pes in ts::demux(data) {
            // shortcut: packets without a time are dropped (ffmpeg times every one); carry the
            // audio clock on if a server leaves them out.
            let Some(pts) = pes.pts else { continue };
            let pts = self.unwrap(pts);
            match pes.stream {
                ts::Stream::H264 => {
                    let dts = pes.dts.map_or(pts, |d| self.unwrap(d));
                    let (keyframe, data) = self.access_unit(pes.data);
                    raws.push(Raw::Video { pts, dts, keyframe, data });
                }
                ts::Stream::Aac => {
                    for (n, (h, frame)) in ts::adts_frames(&pes.data).into_iter().enumerate() {
                        if self.audio.is_none() {
                            self.audio = Some(AudioConfig {
                                codec: AudioCodec::Aac { asc: h.asc() },
                                codec_string: format!("mp4a.40.{}", h.object_type),
                                sample_rate: h.sample_rate,
                                channels: u16::from(h.channels),
                            });
                        }
                        let at = pts + n as u64 * 1024 * ts::CLOCK / u64::from(h.sample_rate);
                        raws.push(Raw::Audio { pts: at, data: frame.to_vec() });
                    }
                }
                ts::Stream::Mp3 => {
                    let mut at = pts;
                    for (h, frame) in ts::mp3_frames(&pes.data) {
                        if self.audio.is_none() {
                            self.audio = Some(AudioConfig {
                                codec: AudioCodec::Mp3,
                                codec_string: "mp3".into(),
                                sample_rate: h.sample_rate,
                                channels: h.channels,
                            });
                        }
                        raws.push(Raw::Audio { pts: at, data: frame.to_vec() });
                        at += u64::from(h.samples) * ts::CLOCK / u64::from(h.sample_rate);
                    }
                }
            }
        }
        if !continuous && let Some(first) = raws.iter().map(Raw::pts).min() {
            self.anchor = Some((first, seg.start, index));
        }
        let Some((origin, media, _)) = self.anchor else { return };
        let time = |t: u64| {
            let ticks = i128::from(t) - i128::from(origin);
            let nanos = media.as_nanos() as i128 + ticks * 1_000_000_000 / i128::from(ts::CLOCK);
            Duration::from_nanos(u64::try_from(nanos.max(0)).unwrap_or(u64::MAX))
        };
        for raw in raws {
            let unit = match raw {
                Raw::Video { pts, dts, keyframe, data } => {
                    if self.need_key && !keyframe {
                        continue;
                    }
                    self.need_key = false;
                    Unit::Video { pts: time(pts), dts: time(dts), keyframe, data }
                }
                Raw::Audio { pts, data } => Unit::Audio { pts: time(pts), data },
            };
            self.queue.push_back((index, unit));
        }
        self.held.end = seg.start + seg.duration;
        self.started = true;
    }

    /// A time on the stream's 33-bit clock, unwrapped against the last one seen.
    // shortcut: a seek more than 13 hours from the last time read unwraps wrongly; estimate
    // from the playlist's segment starts if streams that long come up.
    fn unwrap(&mut self, raw: u64) -> u64 {
        let t = self.last_ts.map_or(raw, |near| ts::unwrap(near, raw));
        self.last_ts = Some(t);
        t
    }

    /// Whether an access unit is a key frame (holds an IDR slice), with the SPS and PPS put
    /// before it when it lacks them; the first SPS and PPS set up [`Hls::video`].
    fn access_unit(&mut self, data: Vec<u8>) -> (bool, Vec<u8>) {
        let nals = ts::nal_units(&data);
        let of = |t: u8| nals.iter().find(|n| n.first().is_some_and(|b| b & 0x1f == t)).copied();
        let keyframe = of(5).is_some();
        let (sps, pps) = (of(7), of(8));
        if self.video.is_none()
            && let (Some(sps), Some(pps)) = (sps, pps)
            && let Some(s) = ts::parse_sps(sps)
        {
            self.video = Some(VideoConfig {
                codec: s.codec,
                width: s.width,
                height: s.height,
                sps: sps.to_vec(),
                pps: pps.to_vec(),
            });
        }
        if keyframe
            && sps.is_none()
            && let Some(v) = &self.video
        {
            let mut with = Vec::with_capacity(data.len() + v.sps.len() + v.pps.len() + 8);
            for nal in [&v.sps, &v.pps] {
                with.extend_from_slice(&[0, 0, 0, 1]);
                with.extend_from_slice(nal);
            }
            with.extend_from_slice(&data);
            return (true, with);
        }
        (keyframe, data)
    }

    /// The H.264 stream's setup, once the first segment has been read (`None` without video).
    pub(crate) fn video(&self) -> Option<&VideoConfig> {
        self.video.as_ref()
    }

    /// The sound's setup, once the first segment has been read (`None` without sound).
    pub(crate) fn audio(&self) -> Option<&AudioConfig> {
        self.audio.as_ref()
    }

    /// Whether the first segment has been read, so [`Hls::video`], [`Hls::audio`] and
    /// [`Hls::info`] are known.
    pub(crate) fn started(&self) -> bool {
        self.started
    }

    /// What the stream holds. Its length is the playlist's, and grows with a growing one.
    pub(crate) fn info(&self) -> MediaInfo {
        MediaInfo {
            duration: Some(self.segments.iter().map(|s| s.duration).sum()),
            seekable: true,
            video: self.video.as_ref().map(|v| VideoInfo::new(v.width, v.height, None)),
            audio: self.audio.as_ref().map(|a| AudioFormat::new(a.sample_rate, a.channels)),
            text_tracks: Vec::new(),
        }
    }

    /// The next unit in decoding order, when one has been read.
    pub(crate) fn next_unit(&mut self) -> Option<Unit> {
        let (index, unit) = self.queue.pop_front()?;
        self.current = index;
        Some(unit)
    }

    /// Moves to `to`: reading starts again at the segment holding it (each starts on a key
    /// frame); the player drops what comes before `to`.
    pub(crate) fn seek(&mut self, to: Duration) {
        let index = self.segments.iter().rposition(|s| s.start <= to).unwrap_or(0);
        self.queue.clear();
        self.next = index;
        self.current = index;
        self.failures = 0;
        self.need_key = true;
        let start = self.segments.get(index).map_or(Duration::ZERO, |s| s.start);
        self.held = start..start;
    }

    /// The stretches of the media held, in media time: from where reading started to the end
    /// of the last segment fetched.
    pub(crate) fn buffered(&self) -> Vec<Range<Duration>> {
        crate::host::merge_ranges(vec![self.held.clone()], None)
    }

    /// Whether everything has been handed out: the playlist is complete, every segment has
    /// been read and every unit taken.
    pub(crate) fn ended(&self) -> bool {
        self.loaded && self.complete && self.next >= self.segments.len() && self.queue.is_empty()
    }
}

/// An HLS stream decoded natively: fetched with a [`Fetch`] (waiting), decoded by the
/// platform's [`Elementary`] decoders.
pub(crate) struct HlsDecoder {
    hls: Hls,
    fetch: Box<dyn Fetch>,
    es: Box<dyn Elementary>,
    info: MediaInfo,
    /// The decoders have been told no more units follow.
    ended: bool,
    /// Waits before a fetch (`std::thread::sleep`; the tests do not wait).
    sleep: fn(Duration),
}

impl HlsDecoder {
    /// Fetches what is wanted next and hands it over; false when nothing is wanted.
    fn fetch_next(&mut self) -> Result<bool, MediaError> {
        let Some(want) = self.hls.wanted() else { return Ok(false) };
        if !want.after.is_zero() {
            (self.sleep)(want.after);
        }
        let got = self.fetch.get(&want.url);
        self.hls.deliver(&want, got)?;
        self.refresh_info();
        Ok(true)
    }

    fn refresh_info(&mut self) {
        self.info = self.hls.info();
        if let Some(a) = self.es.audio_format() {
            self.info.audio = Some(a);
        }
    }
}

impl Decoder for HlsDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        loop {
            if let Some(s) = self.es.pull()? {
                return Ok(Some(s));
            }
            if self.ended {
                return Ok(None);
            }
            if let Some(unit) = self.hls.next_unit() {
                self.es.push(unit)?;
            } else if self.hls.ended() {
                self.es.end()?;
                self.ended = true;
            } else if !self.fetch_next()? {
                // Nothing to hand out and nothing wanted cannot happen; stop rather than spin.
                return Ok(None);
            }
        }
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.hls.seek(to);
        self.ended = false;
        self.es.flush()
    }

    fn buffered(&mut self) -> Option<Vec<Range<Duration>>> {
        Some(self.hls.buffered())
    }
}

/// Opens the HLS stream at `url`: fetches its playlist (and the variant a master playlist
/// chooses) and its first segment, so what it holds is known, then sets `es` up for it.
pub(crate) fn open(
    url: &str,
    fetch: Box<dyn Fetch>,
    es: Box<dyn Elementary>,
) -> Result<Box<dyn Decoder>, MediaError> {
    Ok(Box::new(open_with(url, fetch, es, std::thread::sleep)?))
}

fn open_with(
    url: &str,
    fetch: Box<dyn Fetch>,
    es: Box<dyn Elementary>,
    sleep: fn(Duration),
) -> Result<HlsDecoder, MediaError> {
    let hls = Hls::new(url);
    let info = hls.info();
    let mut d = HlsDecoder { hls, fetch, es, info, ended: false, sleep };
    while !d.hls.started() {
        if d.hls.ended() || !d.fetch_next()? {
            return Err(MediaError::Format("the HLS playlist lists no segments".into()));
        }
    }
    if d.hls.video().is_none() && d.hls.audio().is_none() {
        return Err(MediaError::Unsupported("HLS segments hold no H.264 video, AAC or MP3 sound".into()));
    }
    d.es.configure(d.hls.video(), d.hls.audio())?;
    d.refresh_info();
    Ok(d)
}

/// Whether `source` is an HLS playlist: its path ends in `.m3u8` (any case), whatever query
/// or fragment follows.
pub(crate) fn is_hls(source: &MediaSource) -> bool {
    let url = source.to_url();
    let path = url.split(['?', '#']).next().unwrap_or(&url);
    path.to_ascii_lowercase().ends_with(".m3u8")
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests index what they have just checked is there")]
mod tests;
