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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct AudioFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Channels in each frame.
    pub channels: u16,
}

impl AudioFormat {
    /// Samples at `sample_rate` frames per second, `channels` to a frame.
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        AudioFormat { sample_rate, channels }
    }

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
#[non_exhaustive]
pub struct VideoInfo {
    /// Width shown, in pixels.
    pub width: u32,
    /// Height shown, in pixels.
    pub height: u32,
    /// Frames per second, when the stream says (variable-rate streams may not).
    pub frame_rate: Option<f64>,
}

impl VideoInfo {
    /// A picture `width` by `height` pixels as shown, at `frame_rate` frames per second when
    /// the stream says.
    pub fn new(width: u32, height: u32, frame_rate: Option<f64>) -> Self {
        VideoInfo { width, height, frame_rate }
    }
}

/// A text track carried inside the media (subtitles or captions).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextTrackInfo {
    /// What to call it ("English", "Commentary").
    pub label: String,
    /// Its language (BCP 47), when the file says.
    pub language: Option<String>,
}

impl TextTrackInfo {
    /// A track called `label`, in `language` (BCP 47) when the media says.
    pub fn new(label: impl Into<String>, language: Option<String>) -> Self {
        TextTrackInfo { label: label.into(), language }
    }
}

/// What an opened source holds.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct MediaInfo {
    /// How long it lasts; `None` for a live stream. While [`MediaInfo::growing`], the length
    /// so far.
    pub duration: Option<Duration>,
    /// Whether the player can move to another position in it.
    pub seekable: bool,
    /// The media is still being made and `duration` may still grow (an HLS event playlist
    /// without its end yet). A seek past the length so far is not cut to it: the player waits
    /// for the media to reach the target. A decoder says so in [`Decoder::info`], and clears
    /// it once the length is final.
    pub growing: bool,
    /// The picture, when there is one.
    pub video: Option<VideoInfo>,
    /// The sound, when there is some: the format its samples come in.
    pub audio: Option<AudioFormat>,
    /// Text tracks inside the media, in the order [`Sample::Text`] numbers them.
    pub text_tracks: Vec<TextTrackInfo>,
}

/// One timed piece of decoded media. Times are positions in the media.
#[derive(Clone, Debug)]
#[non_exhaustive]
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
#[non_exhaustive]
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
    /// The person, or the system's privacy settings, did not allow the device (a microphone
    /// the app may not hear).
    Denied(String),
    /// A system component it needs is not installed, or is installed but cannot run here: which
    /// one, what for, and the package to install ([`MissingDependency`](crate::deps::MissingDependency)).
    Missing(crate::deps::MissingDependency),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::Unsupported(r) => write!(f, "not supported here: {r}"),
            MediaError::NotFound(r) => write!(f, "not found: {r}"),
            MediaError::Format(r) => write!(f, "not playable media: {r}"),
            MediaError::Decode(r) => write!(f, "decoding failed: {r}"),
            MediaError::Device(r) => write!(f, "sound device: {r}"),
            MediaError::Denied(r) => write!(f, "not allowed: {r}"),
            MediaError::Missing(d) => write!(f, "{d}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// An opened source, decoding in order. It is used from one thread at a time (the player's
/// decoding thread), which may differ from the thread that opened it.
pub trait Decoder: Send {
    /// What the source holds (known once [`Decoder::poll_ready`] is ready).
    fn info(&self) -> &MediaInfo;
    /// Whether the decoder is ready to decode: `Pending` while it is still opening without
    /// blocking (a browser fetching the source and configuring its decoders), waking `cx`'s
    /// waker when it is. Decoders that open inside [`MediaHost::open`] are ready at once.
    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), MediaError>> {
        let _ = cx;
        std::task::Poll::Ready(Ok(()))
    }
    /// The next sample in decoding order (sound and pictures interleaved roughly by time),
    /// waiting for it if the source is slow. `None` at the end.
    fn read(&mut self) -> Result<Option<Sample>, MediaError>;
    /// The next sample if it is ready, without waiting: decoders that work asynchronously
    /// (a browser's) return `Pending` and wake `cx`'s waker when one is. By default this
    /// reads, waiting as [`Decoder::read`] does (the player calls it off the UI thread).
    fn poll_read(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<Option<Sample>, MediaError>> {
        let _ = cx;
        std::task::Poll::Ready(self.read())
    }
    /// Moves to `to`: the samples read next start at or before it (a decoder may start at the
    /// key frame before it; the player drops what comes before `to`).
    fn seek(&mut self, to: Duration) -> Result<(), MediaError>;
    /// The stretches of the media the source holds without waiting (downloaded, for a
    /// stream), as positions in the media, when the decoder knows; `None` when it does not.
    /// The player asks a few times a second, from the decoding thread, also while paused, and
    /// adds what it has decoded ahead itself; a local file counts as held whole. A decoder
    /// may use the call to keep its source fresh (a growing playlist asked again), and
    /// [`Decoder::info`] is read after it.
    fn buffered(&mut self) -> Option<Vec<std::ops::Range<Duration>>> {
        None
    }
}

/// `ranges` sorted, with those that touch or overlap joined and empty ones dropped, each cut
/// to `0..limit` when there is a limit.
pub fn merge_ranges(
    mut ranges: Vec<std::ops::Range<Duration>>,
    limit: Option<Duration>,
) -> Vec<std::ops::Range<Duration>> {
    if let Some(l) = limit {
        for r in &mut ranges {
            r.end = r.end.min(l);
            r.start = r.start.min(r.end);
        }
    }
    ranges.retain(|r| r.start < r.end);
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<std::ops::Range<Duration>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
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
    /// Opens the default sound input (the microphone the person chose in the system's
    /// settings), asking for samples in `format`. A platform that cannot convert from the
    /// device's own rate and channels gives those instead, as [`AudioInput::format`] reports;
    /// [`Microphone`](crate::Microphone) converts them. It is capturing when this returns.
    /// Hosts without one say [`MediaError::Unsupported`].
    fn audio_input(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        let _ = format;
        Err(MediaError::Unsupported(format!("{} has no sound input", self.name())))
    }
    /// The decoder [`MediaHost::video_stream_decoder`] would use (its name, "vah264dec"):
    /// the best one shown to run on this machine, for an app's "decodes with" line. The first
    /// call may take a moment while decoders are tried; the answer is kept for the process.
    fn video_stream_decoder_choice(&self, codec: VideoCodec, hardware: bool) -> Result<String, MediaError> {
        let _ = (codec, hardware);
        Err(MediaError::Unsupported(format!("{} cannot decode a video stream", self.name())))
    }
    /// A decoder for a compressed video stream that arrives piece by piece (a shared screen, a
    /// camera call), rather than a file or address: pieces go in with
    /// [`VideoStreamDecoder::push`] and pictures come out as soon as they are decoded.
    /// `hardware` asks for the graphics card's decoder first, falling back to the processor's.
    /// Hosts without one say [`MediaError::Unsupported`]; a missing or unusable decoder is
    /// [`MediaError::Missing`], naming what to install.
    fn video_stream_decoder(
        &self,
        codec: VideoCodec,
        hardware: bool,
    ) -> Result<Box<dyn VideoStreamDecoder>, MediaError> {
        let _ = (codec, hardware);
        Err(MediaError::Unsupported(format!("{} cannot decode a video stream", self.name())))
    }
    /// An encoder that turns pictures into a compressed video stream as they are made (a
    /// shared screen): pictures go in with [`VideoStreamEncoder::encode`] and the stream's
    /// pieces come out as soon as they are encoded. Hosts without one say
    /// [`MediaError::Unsupported`] with what is missing.
    fn video_stream_encoder(
        &self,
        options: &VideoEncoderOptions,
    ) -> Result<Box<dyn VideoStreamEncoder>, MediaError> {
        let _ = options;
        Err(MediaError::Unsupported(format!("{} cannot encode a video stream", self.name())))
    }
    /// The encoder [`MediaHost::video_stream_encoder`] chooses for `codec` (`hardware`: the
    /// graphics card's first), by name, after making sure it runs on this machine: for a
    /// settings line saying what encodes. Hosts without one say [`MediaError::Unsupported`].
    fn video_stream_encoder_choice(&self, codec: VideoCodec, hardware: bool) -> Result<String, MediaError> {
        let _ = (codec, hardware);
        Err(MediaError::Unsupported(format!("{} cannot encode a video stream", self.name())))
    }
    /// Pictures of a screen or window the system granted (on Linux, the PipeWire stream the
    /// desktop's ScreenCast portal opened). Hosts that cannot read it say
    /// [`MediaError::Unsupported`].
    fn screen_stream(&self, source: ScreenSource) -> Result<Box<dyn ScreenStream>, MediaError> {
        let _ = source;
        Err(MediaError::Unsupported(format!("{} cannot read a screen stream", self.name())))
    }
    /// Sound the computer is playing, from every program except this one, in `format` (so a
    /// shared screen's sound never carries this app's own sound, such as a call's voices).
    /// Hosts that cannot leave this app out say [`MediaError::Unsupported`] rather than
    /// capture it too.
    fn output_capture(&self, format: AudioFormat) -> Result<Box<dyn AudioInput>, MediaError> {
        let _ = format;
        Err(MediaError::Unsupported(format!("{} cannot record what the computer plays", self.name())))
    }
}

/// What a stream encoder makes ([`MediaHost::video_stream_encoder`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VideoEncoderOptions {
    /// The compressed format.
    pub codec: VideoCodec,
    /// Picture width in pixels (even).
    pub width: u32,
    /// Picture height in pixels (even).
    pub height: u32,
    /// Pictures a second.
    pub fps: u32,
    /// The target rate in kilobits a second.
    pub kbps: u32,
    /// Pictures from one key frame to the next; 0 picks three seconds' worth.
    pub keyframe_interval: u32,
    /// Try the graphics card's encoders first (falling back to the processor's).
    pub hardware: bool,
    /// The graphics card to prefer by its PCI vendor id (0x10de NVIDIA, 0x1002 AMD, 0x8086
    /// Intel), or 0 for no preference.
    pub vendor: u32,
}

impl VideoEncoderOptions {
    /// H.264 at `width` by `height`, `fps` pictures a second and `kbps`, on the graphics card
    /// where there is an encoder, key frames every three seconds.
    pub fn new(width: u32, height: u32, fps: u32, kbps: u32) -> Self {
        VideoEncoderOptions {
            codec: VideoCodec::H264,
            width,
            height,
            fps,
            kbps,
            keyframe_interval: 0,
            hardware: true,
            vendor: 0,
        }
    }

    /// Whether to try the graphics card's encoders first.
    pub fn hardware(mut self, on: bool) -> Self {
        self.hardware = on;
        self
    }

    /// The graphics card to prefer, by PCI vendor id.
    pub fn vendor(mut self, vendor: u32) -> Self {
        self.vendor = vendor;
        self
    }

    /// Pictures from one key frame to the next (0: three seconds' worth).
    pub fn keyframe_interval(mut self, pictures: u32) -> Self {
        self.keyframe_interval = pictures;
        self
    }
}

/// One piece of an encoded stream: for H.264, one access unit as an Annex B byte stream, with
/// the parameter sets (SPS and PPS) before every key frame, as [`VideoCodec::H264`] describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedPiece {
    /// The piece can be decoded without any before it.
    pub keyframe: bool,
    /// The bytes.
    pub data: Vec<u8>,
}

/// A video stream encoder ([`MediaHost::video_stream_encoder`]), used from one thread at a time.
pub trait VideoStreamEncoder: Send {
    /// The encoder's name ("vaapih264enc", "x264enc").
    fn name(&self) -> &str;
    /// True when the graphics card encodes.
    fn is_hardware(&self) -> bool;
    /// Makes the next picture a key frame. False when this encoder cannot be asked while it
    /// runs (open a new one: it starts with a key frame).
    fn request_keyframe(&mut self) -> bool;
    /// Changes the target rate. False when this encoder cannot change it while it runs.
    fn set_kbps(&mut self, kbps: u32) -> bool;
    /// Encodes one picture, blue-green-red-unused (`width * height * 4` bytes, top row first),
    /// and returns the pieces that are ready: usually this picture's, sometimes none yet (an
    /// encoder that looks ahead) or an earlier one's as well.
    fn encode(&mut self, bgrx: &[u8]) -> Result<Vec<EncodedPiece>, MediaError>;
}

/// A screen or window the system granted for capture ([`MediaHost::screen_stream`]).
#[derive(Debug)]
#[non_exhaustive]
pub enum ScreenSource {
    /// A PipeWire stream: the connection the ScreenCast portal opened and the stream's node.
    #[cfg(unix)]
    PipeWire {
        /// The connection (from the portal's `OpenPipeWireRemote`).
        fd: std::os::fd::OwnedFd,
        /// The stream's node id (from the portal's `Start`).
        node: u32,
    },
}

/// One captured picture, blue-green-red-unused, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub bgrx: Vec<u8>,
}

/// Pictures of a granted screen or window ([`MediaHost::screen_stream`]), as they change.
pub trait ScreenStream: Send {
    /// The newest picture since the last call, waiting up to `timeout` for one; `Ok(None)` when
    /// the screen did not change in that time. The stream having ended (the person stopped it
    /// in the system's own controls, the window closed) is [`MediaError::NotFound`].
    fn next(&mut self, timeout: Duration) -> Result<Option<ScreenFrame>, MediaError>;
}

/// A compressed video format a stream decoder takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VideoCodec {
    /// H.264 (AVC) as an Annex B byte stream (start codes), one access unit per piece, with its
    /// parameter sets (SPS and PPS) in the stream before each key frame.
    H264,
}

/// A video stream decoder ([`MediaHost::video_stream_decoder`]). It is used from one thread at
/// a time, which may differ from the thread that made it.
pub trait VideoStreamDecoder: Send {
    /// The decoder's name ("vaapih264dec", "avdec_h264").
    fn name(&self) -> &str;
    /// True when the graphics card decodes.
    fn is_hardware(&self) -> bool;
    /// Hands over one piece of the stream (for H.264, one access unit).
    fn push(&mut self, data: &[u8]) -> Result<(), MediaError>;
    /// The next decoded picture if one is ready, without waiting; `Ok(None)` when none is yet.
    fn next(&mut self) -> Result<Option<Picture>, MediaError>;
}

/// Sound coming in from a microphone, as interleaved 32-bit float samples in the format it
/// was opened with. Capturing starts when it is opened and stops when it is dropped.
pub trait AudioInput: Send {
    /// The layout of the samples [`AudioInput::read`] gives.
    fn format(&self) -> AudioFormat;
    /// How often the device hands over new sound: a reader that reads this often keeps up
    /// without waiting on an empty device.
    fn period(&self) -> Duration;
    /// Appends the sound captured since the last read to `into` (whole frames) and returns
    /// how many frames it appended: 0 when nothing new has arrived. Never waits for sound.
    /// A device that stopped (unplugged, access withdrawn) is an error.
    fn read(&mut self, into: &mut Vec<f32>) -> Result<usize, MediaError>;
}
