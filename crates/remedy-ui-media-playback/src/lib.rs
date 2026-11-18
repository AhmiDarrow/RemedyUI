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
//!   MediaCodec and AAudio on Android. An app with its own decoder implements the trait.
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

mod engine;
mod host;
#[allow(unsafe_code)]
mod os;
mod player;
mod subtitles;
mod widget;

use std::sync::Arc;

pub use engine::{PlaybackState, PlaybackStats};
pub use host::{
    AudioFormat, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Picture, PixelFormat,
    Plane, Sample, TextTrackInfo, VideoInfo, YuvMatrix,
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
