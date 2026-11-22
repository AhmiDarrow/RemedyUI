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
    /// Wait this long after the last fetch of the same kind (playlist or segment) ended
    /// before asking: a growing playlist is asked again every target duration (half of one
    /// near its end), a failed fetch is retried after a pause. A driver waits in short slices
    /// ([`WAIT_SLICE`]) and asks [`Hls::wanted`] again after each, so a seek or a segment now
    /// wanted is not held up behind a playlist's wait.
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
    /// before the first [`Elementary::push`], and again when a host's next stream reuses
    /// these decoders ([`Spare`]): what they hold goes, and decoders that cannot take the
    /// new streams are made anew.
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

/// A host's decoders between streams: the last stream's, handed back when it is dropped and
/// taken by the next stream the host opens, so making the decoders again (tens of
/// milliseconds, hundreds cold) is saved. Dropped with the host, so with its player.
pub(crate) type Spare = std::sync::Arc<SpareSlot>;

/// What a [`Spare`] holds.
pub(crate) type SpareSlot = std::sync::Mutex<Option<Box<dyn Elementary>>>;

/// HLS decoders made (not taken from a [`Spare`]) since the process started: tests tell
/// reuse from it.
static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn decoders_made() -> usize {
    MADE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Set once an engine is done with its decoder (dropped on both its workers): an HLS
/// stream's decoders are then back in the host's [`Spare`] for the stream after it.
#[derive(Default)]
pub(crate) struct Retired {
    gone: std::sync::Mutex<bool>,
    ring: std::sync::Condvar,
}

impl Retired {
    pub(crate) fn retire(&self) {
        *self.gone.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        self.ring.notify_all();
    }

    /// Waits up to `up_to` for it; whether it came.
    fn wait(&self, up_to: Duration) -> bool {
        let gone = self.gone.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (gone, _) = self
            .ring
            .wait_timeout_while(gone, up_to, |g| !*g)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *gone
    }
}

/// How long a stream waits, once its decoders are wanted, for the one it replaces to hand
/// its own back (that one's workers end on their own threads, within a read).
const HANDBACK: Duration = Duration::from_millis(500);

/// No decoders: before configuring, and once handed back.
impl Elementary for () {
    fn configure(&mut self, _: Option<&VideoConfig>, _: Option<&AudioConfig>) -> Result<(), MediaError> {
        Ok(())
    }
    fn push(&mut self, _: Unit) -> Result<(), MediaError> {
        Err(MediaError::Decode("no decoders".into()))
    }
    fn pull(&mut self) -> Result<Option<Sample>, MediaError> {
        Ok(None)
    }
    fn end(&mut self) -> Result<(), MediaError> {
        Ok(())
    }
    fn flush(&mut self) -> Result<(), MediaError> {
        Ok(())
    }
}

/// Fetches a URL whole, waiting for it (native threads).
pub(crate) trait Fetch: Send {
    /// Gives up soon (within a tenth of a second or so) once `stop` is set: the reader is gone.
    fn get(&mut self, url: &str, stop: &std::sync::atomic::AtomicBool) -> Result<Vec<u8>, MediaError>;
}

mod playlist;
mod ts;

use crate::host::{AudioFormat, Decoder, MediaInfo, MediaSource, VideoInfo};
use playlist::{Playlist, Segment};
use std::collections::VecDeque;
use std::ops::Range;
use std::task::{Context, Poll};

// What is fetched ahead and held, and how long a fetch may take, are the app's
// (`MediaLimits`): a stream keeps those it opened with, a fetch those it started with.

/// A fetch longer than this is refused: no segment is near it, and an endless answer must not
/// take all the memory ([`MediaLimits::max_fetch`](crate::MediaLimits::max_fetch)).
pub(crate) fn max_fetch() -> usize {
    crate::media_limits().max_fetch
}
/// A fetch gives up when no answer has come this long after asking.
pub(crate) fn first_byte() -> Duration {
    crate::media_limits().first_byte_timeout
}
/// A fetch gives up when it has taken this long in all.
pub(crate) fn load() -> Duration {
    crate::media_limits().load_timeout
}
/// A fetch whose bytes stop coming for this long is dropped and started again.
pub(crate) fn quiet() -> Duration {
    crate::media_limits().quiet_timeout
}
/// The playlist of a growing stream is asked again once the reader is this many segments
/// from its end.
const NEAR_END: usize = 2;
/// The longest a driver waits at once for a fetch that is not due yet, before asking again
/// what is wanted (a seek may have come).
pub(crate) const WAIT_SLICE: Duration = Duration::from_millis(100);
// Riding out late and failed fetches, as Portal's hls.js setup does: a fetch that fails is
// tried again `error_retries` times (`timeout_retries` when it timed out), the pause doubling
// from `retry_pause` up to `max_retry_pause` (every pause, while the playlist is still growing:
// ffmpeg's HLS 404s a segment until it is renamed in); a spot that still fails starts over,
// `recoveries` times, and each segment read earns those back, so a long film with the odd
// hiccup never dies.
/// How a fetch that gave up waiting ends its error, so it is told from one that failed.
const TIMED_OUT: &str = "(timed out)";
/// How a fetch the server said does not exist (HTTP 404 or 410) ends its error.
const MISSING: &str = "(no such file)";
// A playlist the server says does not exist, before it was ever read, is tried again
// `missing_retries` times (a second apart: a live stream's playlist may be moments from being
// written) and then fails, rather than riding out the retries and recoveries a playing stream gets.

/// The error of a fetch of `url` the server answered with HTTP `status`: one that says the
/// file does not exist (404, 410) is told apart, so an open fails promptly.
#[cfg_attr(not(any(windows, target_os = "android", target_arch = "wasm32")), allow(dead_code))]
pub(crate) fn answered(url: &str, status: u32) -> MediaError {
    match status {
        404 | 410 => missing(url, &format!("the server answered {status}")),
        _ => MediaError::NotFound(format!("{url}: the server answered {status}")),
    }
}

/// The error of a fetch of `url` that the server says does not exist (`why`).
pub(crate) fn missing(url: &str, why: &str) -> MediaError {
    MediaError::NotFound(format!("{url}: {why} {MISSING}"))
}

fn is_missing(e: &MediaError) -> bool {
    matches!(e, MediaError::NotFound(m) if m.ends_with(MISSING))
}

/// The error of a fetch of `url` that gave up waiting (`why`): retried as a timeout.
pub(crate) fn timed_out(url: &str, why: &str) -> MediaError {
    MediaError::NotFound(format!("{url}: {why} {TIMED_OUT}"))
}

/// The path a `file:` URL names (`file:///C:/a%20b/c.ts` or `file:///tmp/a%20b/c.ts`, as
/// [`MediaSource::to_url`] makes, or with a segment name joined on unescaped); `None` for
/// other schemes.
///
/// shortcut: a literal `%` in an unescaped segment name is read as an escape; parse the
/// playlist's names apart from the base if that ever matters.
///
/// [`MediaSource::to_url`]: crate::host::MediaSource::to_url
pub(crate) fn file_path(url: &str) -> Option<String> {
    let rest = url.get(..5).filter(|s| s.eq_ignore_ascii_case("file:")).and(url.get(5..))?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    // On Windows `file:///C:/…` and `file://localhost/C:/…` are drive paths and
    // `file://server/share/…` a UNC one (kept as `//server/share/…`); elsewhere `file:///tmp/…`
    // and `file://localhost/tmp/…` are `/tmp/…`.
    let path = if cfg!(windows) {
        rest.strip_prefix("///").or_else(|| rest.strip_prefix("//localhost/"))
    } else {
        rest.strip_prefix("//localhost").filter(|p| p.starts_with('/')).or_else(|| rest.strip_prefix("//"))
    };
    Some(unescape(path.unwrap_or(rest)))
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(v) if c == b'%' => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_timeout(e: &MediaError) -> bool {
    matches!(e, MediaError::NotFound(m) if m.ends_with(TIMED_OUT))
}

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
    /// A seek past the end of a growing playlist so far: reading starts there once the
    /// playlist reaches it (or ends).
    waiting: Option<Duration>,
    /// The index in `segments` of the next segment to fetch.
    next: usize,
    /// Units read, each with the index of its segment.
    queue: VecDeque<(usize, Unit)>,
    /// The bytes the units in `queue` hold.
    queued: usize,
    /// The index of the segment the last unit handed out came from.
    current: usize,
    /// Failed fetches in a row of what is wanted now, since the spot last started over.
    failures: u32,
    /// Times the spot has started over since a segment was last read.
    recoveries: u32,
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
    /// The bounds it works within, as set when it opened.
    pub(crate) limits: crate::MediaLimits,
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
            waiting: None,
            next: 0,
            queue: VecDeque::new(),
            queued: 0,
            current: 0,
            failures: 0,
            recoveries: 0,
            anchor: None,
            last_ts: None,
            need_key: true,
            held: Duration::ZERO..Duration::ZERO,
            video: None,
            audio: None,
            started: false,
            limits: crate::media_limits(),
        }
    }

    /// What to fetch next, or `None` when nothing is wanted now (enough is held ahead, or the
    /// whole playlist has been fetched).
    pub(crate) fn wanted(&self) -> Option<Want> {
        let pause = self.retry_pause();
        if !self.loaded {
            return Some(Want { url: self.url.clone(), kind: WantKind::Playlist, after: pause });
        }
        let reading = self.queue.front().map_or(self.next, |(i, _)| *i).max(self.current);
        if let Some(s) = self.segments.get(self.next)
            && self.next <= reading.saturating_add(self.limits.hls_ahead)
            && (self.queued < self.limits.hls_max_held || self.queue.is_empty())
        {
            return Some(Want { url: s.url.clone(), kind: WantKind::Segment(s.sequence), after: pause });
        }
        self.reload()
    }

    /// The playlist again, while it is not complete (whatever else is wanted): kept fresh so
    /// its length grows while nothing is read (paused). `None` once it has ended.
    pub(crate) fn reload(&self) -> Option<Want> {
        if !self.loaded || self.complete {
            return None;
        }
        let reading = self.queue.front().map_or(self.next, |(i, _)| *i).max(self.current);
        let after = if self.failures > 0 || self.recoveries > 0 {
            self.retry_pause()
        } else if reading + NEAR_END >= self.segments.len() {
            self.target / 2
        } else {
            self.target
        };
        Some(Want { url: self.url.clone(), kind: WantKind::Playlist, after })
    }

    /// How long to wait before trying again what failed; zero when nothing did.
    fn retry_pause(&self) -> Duration {
        if self.failures == 0 && self.recoveries == 0 {
            return Duration::ZERO;
        }
        let l = &self.limits;
        if !self.complete {
            return l.max_retry_pause;
        }
        let doubled = 2u32.saturating_pow(self.failures.saturating_sub(1));
        l.retry_pause.saturating_mul(doubled).min(l.max_retry_pause)
    }

    /// Hands over what was fetched for `want`. A failed fetch is asked for again, after a
    /// growing pause, until the retries and recoveries run out, and only then does its error
    /// come out here; a playlist that cannot be played is an error at once.
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
                if !self.loaded && is_missing(&e) && self.failures > self.limits.missing_retries {
                    return Err(e);
                }
                let l = &self.limits;
                let retries = if is_timeout(&e) { l.timeout_retries } else { l.error_retries };
                if self.failures > retries {
                    if self.recoveries >= l.recoveries {
                        return Err(e);
                    }
                    self.recoveries += 1;
                    self.failures = 0;
                }
                return Ok(());
            }
        };
        self.failures = 0;
        match want.kind {
            WantKind::Playlist => self.read_playlist(&want.url, &data),
            WantKind::Segment(_) => {
                // Earned back only by a segment that gave something to play: one that comes
                // but cannot be read must not keep its spot alive for ever.
                let before = self.queue.len();
                self.read_segment(&data);
                if self.queue.len() > before {
                    self.recoveries = 0;
                }
                Ok(())
            }
        }
    }

    fn read_playlist(&mut self, url: &str, data: &[u8]) -> Result<(), MediaError> {
        if data.len() > self.limits.max_playlist {
            return Err(MediaError::Format(format!(
                "the HLS playlist at {url} is over {} bytes",
                self.limits.max_playlist
            )));
        }
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
                        let (seq, mut start) = (last.sequence, last.start.saturating_add(last.duration));
                        for mut s in m.segments.into_iter().filter(|s| s.sequence > seq) {
                            s.start = start;
                            start = start.saturating_add(s.duration);
                            self.segments.push(s);
                        }
                    }
                }
                if let Some(to) = self.waiting {
                    self.place(to);
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
            self.queued += unit_len(&unit);
            self.queue.push_back((index, unit));
        }
        self.held.end = seg.start.saturating_add(seg.duration);
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

    /// The length of the segments listed so far.
    fn length(&self) -> Duration {
        self.segments.last().map_or(Duration::ZERO, |s| s.start.saturating_add(s.duration))
    }

    /// What the stream holds. Its length is the playlist's, and grows with a growing one
    /// (`growing` until the playlist ends).
    pub(crate) fn info(&self) -> MediaInfo {
        MediaInfo {
            duration: Some(self.length()),
            seekable: true,
            growing: !self.complete,
            video: self.video.as_ref().map(|v| VideoInfo::new(v.width, v.height, None)),
            audio: self.audio.as_ref().map(|a| AudioFormat::new(a.sample_rate, a.channels)),
            text_tracks: Vec::new(),
        }
    }

    /// The next unit in decoding order, when one has been read.
    pub(crate) fn next_unit(&mut self) -> Option<Unit> {
        let (index, unit) = self.queue.pop_front()?;
        self.queued -= unit_len(&unit);
        self.current = index;
        Some(unit)
    }

    /// Moves to `to`: reading starts again at the segment holding it (each starts on a key
    /// frame); the player drops what comes before `to`. Past the end of a playlist that is
    /// still growing, reading waits for the playlist to reach `to`; past the end of a complete
    /// one, it starts at the last segment.
    pub(crate) fn seek(&mut self, to: Duration) {
        self.queue.clear();
        self.queued = 0;
        self.failures = 0;
        self.recoveries = 0;
        self.need_key = true;
        self.place(to);
    }

    /// Sets reading to start at `to`, or to wait for the playlist to reach it.
    fn place(&mut self, to: Duration) {
        let waiting = !self.complete && to >= self.length();
        self.waiting = waiting.then_some(to);
        let (index, start) = if waiting {
            (self.segments.len(), to)
        } else {
            let i = self.segments.iter().rposition(|s| s.start <= to).unwrap_or(0);
            (i, self.segments.get(i).map_or(Duration::ZERO, |s| s.start))
        };
        self.next = index;
        self.current = index;
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

fn unit_len(u: &Unit) -> usize {
    match u {
        Unit::Video { data, .. } | Unit::Audio { data, .. } => data.len(),
    }
}

/// The clock a native reader waits by: `std` natively; the tests' own, which does not wait.
#[derive(Clone, Copy)]
pub(crate) struct Clock {
    pub now: fn() -> std::time::Instant,
    pub sleep: fn(Duration),
}

/// What the fetching thread brings back: the want and what came of it.
type Fetched = (Want, Result<Vec<u8>, MediaError>);

/// An HLS stream decoded natively: fetched with a [`Fetch`] on a thread of its own (one fetch
/// at a time, at most [`MediaLimits::hls_ahead`](crate::MediaLimits::hls_ahead) segments ahead), so decoding goes on while a segment
/// downloads; decoded by the platform's [`Elementary`] decoders.
pub(crate) struct HlsDecoder {
    hls: Hls,
    /// Wants for the fetching thread; dropping it ends the thread once its fetch is done.
    asks: std::sync::mpsc::Sender<Want>,
    /// Set on drop: the fetch under way gives up at once rather than run on (up to [`load`]).
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    done: std::sync::mpsc::Receiver<Fetched>,
    /// The fetch under way. What it brings is handed to the reader, which drops a segment
    /// no longer wanted (a seek came meanwhile).
    in_flight: Option<Want>,
    es: Box<dyn Elementary>,
    info: MediaInfo,
    /// The decoders have been told no more units follow.
    ended: bool,
    clock: Clock,
    /// When the last playlist fetch and the last segment fetch ended.
    fetched: (std::time::Instant, std::time::Instant),
    /// A fetch handed over outside a read failed for good: the next read says so.
    failed: Option<MediaError>,
    /// Where the decoders go back to when this is dropped, once configured.
    spare: Option<std::sync::Weak<SpareSlot>>,
}

impl HlsDecoder {
    /// When the last fetch of `want`'s kind ended.
    fn last(&mut self, want: &Want) -> &mut std::time::Instant {
        match want.kind {
            WantKind::Playlist => &mut self.fetched.0,
            WantKind::Segment(_) => &mut self.fetched.1,
        }
    }

    /// How long until `want` is due.
    fn wait(&mut self, want: &Want) -> Duration {
        let since = (self.clock.now)().saturating_duration_since(*self.last(want));
        want.after.saturating_sub(since)
    }

    /// Hands over what the fetch under way brought, waiting up to `wait` for it; false when
    /// nothing came (or nothing is under way).
    fn take(&mut self, wait: Duration) -> Result<bool, MediaError> {
        if self.in_flight.is_none() {
            return Ok(false);
        }
        use std::sync::mpsc::RecvTimeoutError;
        let (want, got) = match self.done.recv_timeout(wait) {
            Ok(f) => f,
            Err(RecvTimeoutError::Timeout) => return Ok(false),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(MediaError::Decode("the HLS fetching thread stopped".into()));
            }
        };
        self.in_flight = None;
        *self.last(&want) = (self.clock.now)();
        self.hls.deliver(&want, got)?;
        self.refresh_info();
        Ok(true)
    }

    /// Starts the fetch wanted next when none is under way and it is due. How long until it
    /// is due when it is not; `None` when one was started, one is under way, or nothing is
    /// wanted.
    fn start(&mut self) -> Option<Duration> {
        if self.in_flight.is_some() {
            return None;
        }
        let want = self.hls.wanted()?;
        let wait = self.wait(&want);
        if !wait.is_zero() {
            return Some(wait);
        }
        if self.asks.send(want.clone()).is_ok() {
            self.in_flight = Some(want);
        }
        None
    }

    fn refresh_info(&mut self) {
        self.info = self.hls.info();
        if let Some(a) = self.es.audio_format() {
            self.info.audio = Some(a);
        }
    }

    /// The next sample, or `Pending` after waiting at most [`WAIT_SLICE`] for a fetch (the
    /// caller asks again, and a seek in between is taken at once).
    fn step(&mut self) -> Poll<Result<Option<Sample>, MediaError>> {
        if let Some(e) = self.failed.take() {
            return Poll::Ready(Err(e));
        }
        loop {
            // Keep the fetching thread busy while units are decoded.
            self.take(Duration::ZERO)?;
            let due_in = self.start();
            if let Some(s) = self.es.pull()? {
                return Poll::Ready(Ok(Some(s)));
            }
            if self.ended {
                return Poll::Ready(Ok(None));
            }
            if let Some(unit) = self.hls.next_unit() {
                self.es.push(unit)?;
                continue;
            }
            if self.hls.ended() {
                self.es.end()?;
                self.ended = true;
                continue;
            }
            if self.in_flight.is_some() {
                if !self.take(WAIT_SLICE)? {
                    return Poll::Pending;
                }
                continue;
            }
            // Nothing to hand out and nothing wanted cannot happen; stop rather than spin.
            let Some(wait) = due_in else { return Poll::Ready(Ok(None)) };
            (self.clock.sleep)(wait.min(WAIT_SLICE));
            return Poll::Pending;
        }
    }

    /// Hands over every fetch it starts until nothing more is due (the tests' clock stands
    /// still, so a growing playlist is not asked again).
    #[cfg(test)]
    fn settle(&mut self) -> Result<(), MediaError> {
        loop {
            self.take(Duration::MAX)?;
            self.start();
            if self.in_flight.is_none() {
                return Ok(());
            }
        }
    }
}

impl Drop for HlsDecoder {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(spare) = self.spare.take().and_then(|s| s.upgrade()) {
            let es = std::mem::replace(&mut self.es, Box::new(()));
            *spare.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(es);
        }
    }
}

impl Decoder for HlsDecoder {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn read(&mut self) -> Result<Option<Sample>, MediaError> {
        loop {
            if let Poll::Ready(r) = self.step() {
                return r;
            }
        }
    }

    fn poll_read(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Sample>, MediaError>> {
        let polled = self.step();
        if polled.is_pending() {
            // Asked again at once: the wait was a slice, so a seek is not held up behind it.
            cx.waker().wake_by_ref();
        }
        polled
    }

    fn seek(&mut self, to: Duration) -> Result<(), MediaError> {
        self.hls.seek(to);
        self.ended = false;
        self.es.flush()
    }

    /// Also hands over what was fetched and starts what is due (a growing playlist asked
    /// again, segments ahead), so its length grows while nothing is read (paused, or the
    /// decoded buffer full).
    fn buffered(&mut self) -> Option<Vec<Range<Duration>>> {
        if self.failed.is_none() {
            match self.take(Duration::ZERO) {
                Ok(_) => {
                    self.start();
                }
                Err(e) => self.failed = Some(e),
            }
        }
        Some(self.hls.buffered())
    }
}

/// Opens the HLS stream at `url`: fetches its playlist (and the variant a master playlist
/// chooses) and its first segment, so what it holds is known, then sets decoders up for it:
/// `spare`'s, when it holds some that take the streams, else made by `make`. They go back to
/// `spare` when the stream is dropped.
pub(crate) fn open(
    url: &str,
    fetch: Box<dyn Fetch>,
    spare: &Spare,
    make: impl FnOnce() -> Result<Box<dyn Elementary>, MediaError>,
) -> Result<Box<dyn Decoder>, MediaError> {
    let clock = Clock { now: std::time::Instant::now, sleep: std::thread::sleep };
    Ok(Box::new(open_with(url, fetch, Some(spare), make, clock)?))
}

thread_local! {
    /// The stop flag of the open running on this thread ([opening_until]).
    static OPEN_STOP: std::cell::RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>> =
        const { std::cell::RefCell::new(None) };
    /// The engine the open running on this thread replaces ([opening_until]).
    static OPEN_AFTER: std::cell::RefCell<Option<std::sync::Arc<Retired>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs open (a host's) with stop as the stop flag of any HLS stream it opens: once set,
/// an open still fetching gives up within a fetch watchdog's tick, and the stream's fetching
/// thread stops. `after` is the engine it replaces: an HLS stream waits briefly for its
/// decoders to come back before making its own.
pub(crate) fn opening_until<R>(
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    after: Option<std::sync::Arc<Retired>>,
    open: impl FnOnce() -> R,
) -> R {
    let before = OPEN_STOP.with(|s| s.replace(Some(stop.clone())));
    let before_after = OPEN_AFTER.with(|a| a.replace(after));
    let r = open();
    OPEN_STOP.with(|s| *s.borrow_mut() = before);
    OPEN_AFTER.with(|a| *a.borrow_mut() = before_after);
    r
}

fn open_with(
    url: &str,
    mut fetch: Box<dyn Fetch>,
    spare: Option<&Spare>,
    make: impl FnOnce() -> Result<Box<dyn Elementary>, MediaError>,
    clock: Clock,
) -> Result<HlsDecoder, MediaError> {
    let (asks, wants) = std::sync::mpsc::channel::<Want>();
    let (back, done) = std::sync::mpsc::channel::<Fetched>();
    let stop = OPEN_STOP.with(|s| s.borrow().clone()).unwrap_or_default();
    let stopped = stop.clone();
    std::thread::Builder::new()
        .name("remedy-hls-fetch".into())
        .spawn(move || {
            for want in wants {
                let got = fetch.get(&want.url, &stopped);
                if back.send((want, got)).is_err() {
                    return;
                }
            }
        })
        .map_err(|e| MediaError::Decode(format!("could not start the HLS fetching thread: {e}")))?;
    let hls = Hls::new(url);
    let info = hls.info();
    let now = (clock.now)();
    let mut d = HlsDecoder {
        hls,
        asks,
        stop,
        done,
        in_flight: None,
        es: Box::new(()),
        info,
        ended: false,
        clock,
        fetched: (now, now),
        failed: None,
        spare: None,
    };
    // The playlist and the first segment only: the rest comes while playing.
    while !d.hls.started() {
        if d.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(MediaError::Decode("closed while opening".into()));
        }
        if d.hls.ended() {
            return Err(MediaError::Format("the HLS playlist lists no segments".into()));
        }
        match d.start() {
            Some(wait) => (d.clock.sleep)(wait.min(WAIT_SLICE)),
            None if d.in_flight.is_none() => {
                return Err(MediaError::Format("the HLS playlist lists no segments".into()));
            }
            None => {
                d.take(WAIT_SLICE)?;
            }
        }
    }
    if d.hls.video().is_none() && d.hls.audio().is_none() {
        return Err(MediaError::Unsupported("HLS segments hold no H.264 video, AAC or MP3 sound".into()));
    }
    // Taken now, not on opening: the stream this one replaces has had the playlist's and the
    // first segment's fetch to hand its decoders back.
    if spare.is_some()
        && let Some(after) = OPEN_AFTER.with(|a| a.borrow().clone())
    {
        // A slice at a time, so a close meanwhile is heard.
        let until = std::time::Instant::now() + HANDBACK;
        while !after.wait(WAIT_SLICE.min(until.saturating_duration_since(std::time::Instant::now())))
            && !d.stop.load(std::sync::atomic::Ordering::Relaxed)
            && std::time::Instant::now() < until
        {}
    }
    let kept = spare.and_then(|s| s.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take());
    d.es = match kept.and_then(|mut es| es.configure(d.hls.video(), d.hls.audio()).map(|()| es).ok()) {
        Some(es) => es,
        None => {
            MADE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut es = make()?;
            es.configure(d.hls.video(), d.hls.audio())?;
            es
        }
    };
    d.spare = spare.map(std::sync::Arc::downgrade);
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
