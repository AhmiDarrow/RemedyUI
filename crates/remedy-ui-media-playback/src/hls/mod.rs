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
}

/// Fetches a URL whole, waiting for it (native threads).
pub(crate) trait Fetch: Send {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, MediaError>;
}
