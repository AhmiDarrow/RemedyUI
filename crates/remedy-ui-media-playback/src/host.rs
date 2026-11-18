//! The media host: what a platform (or an app) provides so a [`Player`](crate::Player) can
//! play something. A decoder turns a file or a stream into timed pictures, sound and text;
//! an audio output plays sound and says how much of it has been heard, which is the clock
//! pictures are shown against.
//!
//! [`platform_host`](crate::platform_host) gives the platform's own: Media Foundation and
//! WASAPI on Windows, the system's GStreamer on Linux (loaded at run time, never bundled),
//! AVFoundation and Audio Queues on macOS, MediaCodec and AAudio on Android. An app with its
//! own decoder implements [`MediaHost`] and hands it to [`Player::open`](crate::Player::open).

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use remedy_ui_media_video::{Picture, PixelFormat, Plane, YuvMatrix};

/// What to play.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MediaSource {
    /// A file on this machine.
    File(PathBuf),
    /// A URL the platform's decoder can open (`https:`, `http:`, `rtsp:`, `file:` …).
    Url(String),
}

impl MediaSource {
    /// A file on this machine.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        MediaSource::File(path.into())
    }

    /// A URL the platform's decoder opens.
    pub fn url(url: impl Into<String>) -> Self {
        MediaSource::Url(url.into())
    }

    /// The file's path, for a local file.
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            MediaSource::File(p) => Some(p),
            MediaSource::Url(_) => None,
        }
    }

    /// A `file:` URL for a local file (percent-encoding what URLs cannot hold), or the URL.
    pub fn to_url(&self) -> String {
        match self {
            MediaSource::Url(u) => u.clone(),
            MediaSource::File(p) => {
                let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
                let s = abs.to_string_lossy().replace('\\', "/");
                let mut out = String::from("file://");
                if !s.starts_with('/') {
                    out.push('/');
                }
                for b in s.bytes() {
                    if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
                        out.push(char::from(b));
                    } else {
                        out.push_str(&format!("%{b:02X}"));
                    }
                }
                out
            }
        }
    }
}

/// The layout of decoded sound: interleaved 32-bit float samples, one per channel per frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Channels in each frame.
    pub channels: u16,
}

impl AudioFormat {
    /// How long `frames` frames last.
    pub fn duration_of(&self, frames: u64) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        let rate = u64::from(self.sample_rate);
        Duration::from_secs(frames / rate) + Duration::from_nanos((frames % rate) * 1_000_000_000 / rate)
    }

    /// How many frames last `d` (rounded down).
    pub fn frames_in(&self, d: Duration) -> u64 {
        let n = d.as_nanos() * u128::from(self.sample_rate) / 1_000_000_000;
        u64::try_from(n).unwrap_or(u64::MAX)
    }
}

/// The picture of a stream with video.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    /// Width shown, in pixels.
    pub width: u32,
    /// Height shown, in pixels.
    pub height: u32,
    /// Frames per second, when the stream says (variable-rate streams may not).
    pub frame_rate: Option<f64>,
}

/// A text track carried inside the media (subtitles or captions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextTrackInfo {
    /// What to call it ("English", "Commentary").
    pub label: String,
    /// Its language (BCP 47), when the file says.
    pub language: Option<String>,
}

/// What an opened source holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    /// How long it lasts; `None` for a live stream.
    pub duration: Option<Duration>,
    /// Whether the player can move to another position in it.
    pub seekable: bool,
    /// The picture, when there is one.
    pub video: Option<VideoInfo>,
    /// The sound, when there is some: the format its samples come in.
    pub audio: Option<AudioFormat>,
    /// Text tracks inside the media, in the order [`Sample::Text`] numbers them.
    pub text_tracks: Vec<TextTrackInfo>,
}

/// One timed piece of decoded media. Times are positions in the media.
#[derive(Clone, Debug)]
pub enum Sample {
    /// A picture to show from `at`.
    Video {
        /// When it shows.
        at: Duration,
        /// How long it shows, when the decoder knows.
        duration: Option<Duration>,
        /// The pixels.
        picture: Picture,
    },
    /// Sound starting at `at`, interleaved in the format [`MediaInfo::audio`] gives.
    Audio {
        /// When it starts.
        at: Duration,
        /// The samples.
        samples: Vec<f32>,
    },
    /// A caption or subtitle cue from an embedded text track.
    Text {
        /// Which of [`MediaInfo::text_tracks`].
        track: usize,
        /// When it shows.
        at: Duration,
        /// When it goes.
        until: Duration,
        /// What it says (plain text, lines separated by `\n`).
        text: String,
    },
}

impl Sample {
    /// When the sample starts.
    pub fn at(&self) -> Duration {
        match self {
            Sample::Video { at, .. } | Sample::Audio { at, .. } | Sample::Text { at, .. } => *at,
        }
    }
}

/// Why media could not be opened, played or heard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaError {
    /// This platform (or this build) cannot do it; the reason says why.
    Unsupported(String),
    /// The file or address does not exist or cannot be reached.
    NotFound(String),
    /// The data is not media the decoder understands, or it is damaged.
    Format(String),
    /// Decoding failed part way.
    Decode(String),
    /// The sound device failed or went away.
    Device(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::Unsupported(r) => write!(f, "not supported here: {r}"),
            MediaError::NotFound(r) => write!(f, "not found: {r}"),
            MediaError::Format(r) => write!(f, "not playable media: {r}"),
            MediaError::Decode(r) => write!(f, "decoding failed: {r}"),
            MediaError::Device(r) => write!(f, "sound device: {r}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// An opened source, decoding in order. It is used from one thread at a time (the player's
/// decoding thread), which may differ from the thread that opened it.
pub trait Decoder: Send {
    /// What the source holds.
    fn info(&self) -> &MediaInfo;
    /// The next sample in decoding order (sound and pictures interleaved roughly by time),
    /// waiting for it if the source is slow. `None` at the end.
    fn read(&mut self) -> Result<Option<Sample>, MediaError>;
    /// Moves to `to`: the samples read next start at or before it (a decoder may start at the
    /// key frame before it; the player drops what comes before `to`).
    fn seek(&mut self, to: Duration) -> Result<(), MediaError>;
}

/// The platform's sound output for one stream: samples written are queued in the device's
/// buffer and played in order. It starts paused.
pub trait AudioOutput: Send {
    /// The format samples are written in (the one asked for; the platform converts).
    fn format(&self) -> AudioFormat;
    /// How much sound the device's buffer holds when full.
    fn buffer(&self) -> Duration;
    /// How many frames can be written now without waiting.
    fn room(&mut self) -> Result<usize, MediaError>;
    /// Queues interleaved samples (whole frames, at most [`AudioOutput::room`] of them).
    fn write(&mut self, samples: &[f32]) -> Result<(), MediaError>;
    /// Frames heard since the output was made or last flushed: the audio clock.
    fn played(&mut self) -> Result<u64, MediaError>;
    /// Stops playing, keeping what is queued and the clock where it is.
    fn pause(&mut self) -> Result<(), MediaError>;
    /// Plays (or goes on playing) what is queued.
    fn resume(&mut self) -> Result<(), MediaError>;
    /// Drops what is queued and sets the clock back to zero (a seek). Leaves it paused.
    fn flush(&mut self) -> Result<(), MediaError>;
    /// Says no more sound follows until the next flush, so the device plays out all it holds
    /// (some outputs keep a last partial block until told). Outputs that always play
    /// everything written need do nothing.
    fn finish(&mut self) -> Result<(), MediaError> {
        Ok(())
    }
}

/// Decoders and sound outputs. The platform's own comes from
/// [`platform_host`](crate::platform_host); an app can plug in its own.
pub trait MediaHost: Send + Sync {
    /// A short name for logs and errors ("Media Foundation").
    fn name(&self) -> &str;
    /// Opens `source` for decoding.
    fn open(&self, source: &MediaSource) -> Result<Box<dyn Decoder>, MediaError>;
    /// Opens the default sound output for samples in `format`.
    fn audio_output(&self, format: AudioFormat) -> Result<Box<dyn AudioOutput>, MediaError>;
}
