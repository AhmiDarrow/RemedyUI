//! RemedyUI media playback: sound and video from files and streams, decoded by the
//! platform's own decoders, for VoiceRemedy, Quickcast, Portal and any app that plays media.
//!
//! - [`Player`]: opens a [`MediaSource`] on its own threads and plays it. Its state,
//!   position, length, volume, captions and errors are signals.
//! - [`player()`]: the widget: the picture with captions over it and a transport bar (play
//!   and pause, time, a keyboard-operable seek bar, mute and volume, captions, a full-screen
//!   request).
//! - [`MediaHost`]: where decoding and sound come from. [`platform_host`] gives the
//!   platform's: Media Foundation and WASAPI on Windows; the system's GStreamer on Linux,
//!   loaded at run time and never bundled; AVFoundation and Audio Queues on macOS;
//!   MediaCodec and AAudio on Android; in a browser page, the browser's WebCodecs decoders (MP4
//!   with H.264 and AAC, taken apart here) and WebAudio. An app with its own decoder
//!   implements the trait.
//! - [`Microphone`]: the default sound input, captured on a worker of its own and handed to
//!   the app as it arrives (a tuner's pitch detection, a singing coach's pitch graph, a
//!   level meter), with its state and level as signals. WASAPI on Windows, the system's
//!   GStreamer on Linux, AAudio on Android and `getUserMedia` with an audio worklet in a
//!   browser page.
//! - Subtitles: WebVTT, SRT and the basics of ASS/SSA, found next to a file, added by the
//!   app, or carried in the media where the platform's decoder gives them.
//!
//! Pictures go to a [`video_surface`](remedy_ui_media_video::video_surface)'s
//! [`VideoSink`](remedy_ui_media_video::VideoSink) as NV12 (converted on the GPU). Sound goes
//! to the default output, and its clock (how much has been heard) decides when each picture
//! shows, so sound and picture stay together however the machine is loaded. Without sound,
//! the monotonic clock does.
//!
//! Nothing is decoded on the UI thread. A player decodes ahead only as far as the sound
//! device's buffer and a quarter second of pictures need, and never keeps more decoded
//! pictures than a sixty-fourth of the machine's memory.
//!
//! Layer 5. Calls into the platform live in the `os` module, the only place unsafe code is
//! allowed.

/// The player's own words (control names, tooltips, messages over the picture) in every
/// language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_view::strings::add_messages`] under the name `media-playback`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("media-playback");

mod capture;
pub mod deps;
mod engine;
mod host;
// The MP4 reader feeds the browser's decoders (and is tested everywhere).
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
mod mp4;
#[allow(unsafe_code)]
mod os;
mod player;
mod subtitles;
mod widget;

use std::sync::Arc;

pub use capture::{Microphone, MicrophoneOptions, MicrophoneState};
pub use deps::{DependencyState, MissingDependency};
pub use engine::{PlaybackState, PlaybackStats};
pub use host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, EncodedPiece, MediaError, MediaHost, MediaInfo,
    MediaSource, Picture, PixelFormat, Plane, Sample, ScreenFrame, ScreenSource, ScreenStream, TextTrackInfo,
    VideoCodec, VideoEncoderOptions, VideoInfo, VideoStreamDecoder, VideoStreamEncoder, YuvMatrix,
    merge_ranges,
};
pub use player::{Player, PlayerOptions, UnseenPolicy};
pub use subtitles::{Cue, SubtitleFormat, SubtitleTrack, parse_subtitles};
pub use widget::{format_time, player};

/// The platform's own decoders and sound output, or [`MediaError::Unsupported`] with the
/// reason where there are none (no GStreamer installed on a Linux machine, a platform with
/// no backend).
pub fn platform_host() -> Result<Arc<dyn MediaHost>, MediaError> {
    os::platform_host()
}
