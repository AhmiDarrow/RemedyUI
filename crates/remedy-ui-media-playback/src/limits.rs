//! How much the engine reads, holds and waits for while playing: every bound, with its
//! default, set by the app with [`set_media_limits`].

use std::sync::{PoisonError, RwLock};
use std::time::Duration;

/// The bounds playback works within, for every player in the process
/// ([`set_media_limits`]). Each default suits ordinary media; raise one for unusual media,
/// lower one to spend less memory or give up sooner.
///
/// The container caps (`max_ebml_header`, `max_mkv_header`, `max_mkv_cues`, `max_moov`,
/// `max_embedded_cue`) and the size caps (`max_fetch`, `max_playlist`,
/// `max_subtitle_bytes`, `max_streamed_cue`) guard against hostile or broken files and
/// servers: each is read whole into memory, so raising one lets such a file or server take
/// that much memory before it is refused.
///
/// ```
/// use remedy_ui_media_playback::{MediaLimits, set_media_limits};
/// set_media_limits(MediaLimits::default().hls_ahead(6).max_subtitle_bytes(64 << 20));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaLimits {
    /// HLS: segments fetched ahead of the one being read. Default 3.
    pub hls_ahead: usize,
    /// HLS: no more segments are fetched ahead once the media read ahead holds this many
    /// bytes, as hls.js's `maxBufferSize`. Default 60 MB.
    pub hls_max_held: usize,
    /// HLS: the largest segment or playlist fetch; a longer answer is refused, so an endless
    /// one cannot take all the memory. Default 256 MiB.
    pub max_fetch: usize,
    /// HLS: the largest playlist read (a day of 2 s segments is about 2 MB). Default 16 MiB.
    pub max_playlist: usize,
    /// HLS: a failed fetch is tried again this many times in a row. Default 3.
    pub error_retries: u32,
    /// HLS: a fetch that timed out is tried again this many times in a row. Default 4.
    pub timeout_retries: u32,
    /// HLS: a playlist the server says does not exist, before it was ever read, is tried
    /// again this many times and then fails. Default 2.
    pub missing_retries: u32,
    /// HLS: how many times a spot that keeps failing starts over (each segment read earns
    /// one back) before its error comes out. Default 6.
    pub recoveries: u32,
    /// HLS: the first pause before trying a failed fetch again; each next one doubles.
    /// Default 250 ms.
    pub retry_pause: Duration,
    /// HLS: the longest pause between tries, and every pause while a live playlist is still
    /// growing. Default 1 s.
    pub max_retry_pause: Duration,
    /// HLS: a fetch gives up when no answer has come this long after asking. Default 12 s.
    pub first_byte_timeout: Duration,
    /// HLS: a fetch gives up when it has taken this long in all. Default 60 s.
    pub load_timeout: Duration,
    /// HLS: a fetch whose bytes stop coming for this long is dropped and tried again.
    /// Default 2.5 s.
    pub quiet_timeout: Duration,
    /// The largest subtitle file read, and the most cue text kept from one media file or
    /// stream (a live stream keeps its latest within it). Default 16 MiB.
    pub max_subtitle_bytes: u64,
    /// The longest single cue a stream may hand out; a longer one ends the stream with an
    /// error. Default 1 MiB.
    pub max_streamed_cue: usize,
    /// Matroska and WebM: the largest EBML header read. Default 4 KiB.
    pub max_ebml_header: u64,
    /// Matroska and WebM: the largest SeekHead, Info or Tracks element read. Default 16 MiB.
    pub max_mkv_header: u64,
    /// Matroska and WebM: the largest Cues (seek index) element read. Default 64 MiB.
    pub max_mkv_cues: u64,
    /// MP4: the largest movie box (`moov`) read. Default 64 MiB.
    pub max_moov: u64,
    /// Matroska, WebM and MP4: the largest embedded subtitle block or sample read; a larger
    /// one is skipped. Default 64 KiB.
    pub max_embedded_cue: u64,
    /// Linux, what the computer plays ([`MediaHost::output_capture`](crate::MediaHost::output_capture)):
    /// how much of each program's sound is held for a reader that falls behind before the
    /// oldest is dropped. Default 500 ms.
    pub capture_queue: Duration,
}

impl MediaLimits {
    /// The defaults, as a constant.
    pub const DEFAULT: MediaLimits = MediaLimits {
        hls_ahead: 3,
        hls_max_held: 60_000_000,
        max_fetch: 256 << 20,
        max_playlist: 16 << 20,
        error_retries: 3,
        timeout_retries: 4,
        missing_retries: 2,
        recoveries: 6,
        retry_pause: Duration::from_millis(250),
        max_retry_pause: Duration::from_secs(1),
        first_byte_timeout: Duration::from_secs(12),
        load_timeout: Duration::from_secs(60),
        quiet_timeout: Duration::from_millis(2500),
        max_subtitle_bytes: 16 << 20,
        max_streamed_cue: 1 << 20,
        max_ebml_header: 4 << 10,
        max_mkv_header: 16 << 20,
        max_mkv_cues: 64 << 20,
        max_moov: 64 << 20,
        max_embedded_cue: 64 << 10,
        capture_queue: Duration::from_millis(500),
    };
}

impl Default for MediaLimits {
    fn default() -> Self {
        MediaLimits::DEFAULT
    }
}

macro_rules! setters {
    ($($name:ident: $t:ty),* $(,)?) => {
        impl MediaLimits {
            $(
                #[doc = concat!("Sets [`", stringify!($name), "`](Self::", stringify!($name), ").")]
                pub fn $name(mut self, value: $t) -> Self {
                    self.$name = value;
                    self
                }
            )*
        }
    };
}

setters! {
    hls_ahead: usize,
    hls_max_held: usize,
    max_fetch: usize,
    max_playlist: usize,
    error_retries: u32,
    timeout_retries: u32,
    missing_retries: u32,
    recoveries: u32,
    retry_pause: Duration,
    max_retry_pause: Duration,
    first_byte_timeout: Duration,
    load_timeout: Duration,
    quiet_timeout: Duration,
    max_subtitle_bytes: u64,
    max_streamed_cue: usize,
    max_ebml_header: u64,
    max_mkv_header: u64,
    max_mkv_cues: u64,
    max_moov: u64,
    max_embedded_cue: u64,
    capture_queue: Duration,
}

static LIMITS: RwLock<MediaLimits> = RwLock::new(MediaLimits::DEFAULT);

/// Sets the bounds playback works within, for every player in the process. A stream already
/// open keeps the HLS bounds it opened with; everything else applies from the next read.
pub fn set_media_limits(limits: MediaLimits) {
    *LIMITS.write().unwrap_or_else(PoisonError::into_inner) = limits;
}

/// The bounds playback works within now ([`set_media_limits`]).
pub fn media_limits() -> MediaLimits {
    *LIMITS.read().unwrap_or_else(PoisonError::into_inner)
}
