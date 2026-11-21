//! RemedyUI media playback: sound and video from files and streams, decoded by the
//! platform's own decoders, for VoiceRemedy, Portal and any app that plays media.
//!
//! - [`Player`]: opens a [`MediaSource`] on its own threads and plays it. Its state,
//!   position, length, volume, captions and errors are signals.
//! - [`player()`]: the widget: the picture with captions over it and a transport bar (play
//!   and pause, time, a keyboard-operable seek bar, mute and volume, captions, a full-screen
//!   request, and a theater-mode request for apps that lay one out). Captions can be shifted
//!   earlier or later ([`Player::set_caption_delay`]) when a subtitle file is out of step.
//! - Casting: [`Player::on_cast`] shows a Cast button whose press hands the app the button to
//!   anchor its own menu of devices to; [`Player::set_remote`] puts the player in remote mode,
//!   where the local media pauses in place, a casting stage replaces the picture, and every
//!   control and key acts on the app's [`RemoteControl`] while the bar shows the
//!   [`RemoteStatus`] the app feeds in, moved on between reports while the device plays.
//! - A [`Timeline`] ([`Player::set_timeline`]) puts a stream that holds part of a longer work
//!   (a film a media server converts from a resume or skip point) on the work's time: every
//!   position shown or taken is the work's, the seek bar spans its length, and a seek outside
//!   what the stream holds goes to the app ([`Player::on_seek_outside`]) while the player
//!   shows it is skipping, until the app hands over a new stream
//!   ([`Player::replace_source`]). [`Player::hold_for_source`] waits the same way for a new
//!   stream of the same place (a quality or sound switch); while skipping, holding or remote
//!   the old stream is not fetched. [`Player::empty`] starts with nothing to play here.
//! - [`MediaHost`]: where decoding and sound come from. [`platform_host`] gives the
//!   platform's: Media Foundation and WASAPI on Windows; the system's GStreamer on Linux,
//!   loaded at run time and never bundled; AVFoundation and Audio Queues on macOS;
//!   MediaCodec and AAudio on Android; in a browser page, the browser's WebCodecs decoders (MP4
//!   with H.264 and AAC, taken apart here) and WebAudio. An app with its own decoder
//!   implements the trait. HLS (`.m3u8` of MPEG-TS with H.264 and AAC or MP3) is taken apart
//!   by the crate's own reader and decoded by Media Foundation on Windows, GStreamer on Linux
//!   and VideoToolbox and AudioToolbox on macOS; Android refuses it as unsupported for now.
//! - [`Microphone`]: the default sound input, captured on a worker of its own and handed to
//!   the app as it arrives (a tuner's pitch detection, a singing coach's pitch graph, a
//!   level meter), with its state and level as signals. WASAPI on Windows, the system's
//!   GStreamer on Linux, AAudio on Android and `getUserMedia` with an audio worklet in a
//!   browser page.
//! - Subtitles: WebVTT, SRT and the basics of ASS/SSA, found next to a file, added by the
//!   app, or carried in the media: MP4 timed text and the text tracks of Matroska and WebM
//!   (SRT, ASS/SSA, WebVTT) are read from a local file by the crate's own readers, so they
//!   show on every platform; others where the platform's decoder gives them. Bitmap
//!   subtitles (PGS, VobSub, DVB) are pictures, not text, and are not shown.
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
// HLS (`.m3u8` of MPEG-TS) taken apart here; the platform decodes the compressed streams.
// shortcut: Android only refuses playlists (no `Elementary` there yet); a page reads HLS
// through it once its host does.
#[cfg_attr(
    not(any(windows, all(unix, not(any(target_os = "ios", target_os = "android"))))),
    allow(dead_code)
)]
mod hls;
// Matroska and WebM text subtitles, read from local files on every platform (the
// platforms' readers do not hand them out).
mod mkv;
// The MP4 reader feeds the browser's decoders, and reads MP4 timed text everywhere (the
// platforms' readers do not hand it out).
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), allow(dead_code))]
mod mp4;
#[allow(unsafe_code)]
mod os;
mod player;
mod stretch;
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
pub use player::{
    KeyScheme, NEXT_UP_DELAY, NextUp, Player, PlayerOptions, Remote, RemoteControl, RemoteStatus,
    RemoteStatusSender, StageStatus, Timeline, UnseenPolicy,
};
pub use stretch::{MAX_SPEED, MIN_SPEED, SPEEDS, clamp_speed};
pub use subtitles::{Cue, SubtitleFormat, SubtitleTrack, parse_subtitles};
pub use widget::{format_time, player};

/// The platform's own decoders and sound output, or [`MediaError::Unsupported`] with the
/// reason where there are none (no GStreamer installed on a Linux machine, a platform with
/// no backend).
pub fn platform_host() -> Result<Arc<dyn MediaHost>, MediaError> {
    os::platform_host()
}
