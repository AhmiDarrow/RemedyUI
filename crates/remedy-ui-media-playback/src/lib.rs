//! RemedyUI media playback: sound and video from files and streams, decoded by the
//! platform's own decoders, for VoiceRemedy, Portal and any app that plays media.
//!
//! - [`Player`]: opens a [`MediaSource`] on its own threads and plays it. Its state,
//!   position, length, volume, captions and errors are signals.
//! - [`player()`]: the widget: the picture with captions over it and a transport bar (play
//!   and pause, time, a keyboard-operable seek bar, mute and volume, captions, a full-screen
//!   request, and a theater-mode request for apps that lay one out). Captions can be shifted
//!   earlier or later ([`Player::set_caption_delay`]) when a subtitle file is out of step.
//!   [`Player::keys_from`] hears the player's keys from the whole window, as a web page's
//!   player does, leaving text fields, menus and dialogs theirs.
//! - Captions: one button says what shows and opens a menu of Off, Auto (the app's pick,
//!   [`Player::on_caption_pick`]), the tracks, the app's not-yet-loaded
//!   [choices](Player::set_caption_choices) with their details (hearing impaired), and the
//!   timing; the app says when its pick is [loading](Player::set_captions_loading). The pick
//!   carries to the next media.
//! - Portal's bar ([`player_with`], [`PlayerOptions`]): Back and Forward 10 seconds, no speed
//!   button, two rows, the app's own controls ([`PlayerParts`]), and a compact mini player ([`Player::set_compact`]). The picture alone
//!   is [`player_picture`]: the player is seen while any of its pictures is, so a
//!   picture-in-picture or a second window keeps it playing.
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
//!   with H.264 and AAC, taken apart here) and WebAudio, or, where a page may not use
//!   WebCodecs (one not served securely), a hidden media element fed through Media Source
//!   Extensions ([`WebBackend`]). An app with its own decoder
//!   implements the trait. HLS (`.m3u8` of MPEG-TS with H.264 and AAC or MP3) is taken apart
//!   by the crate's own reader and decoded by Media Foundation on Windows and GStreamer on
//!   Linux; macOS and Android refuse it as unsupported for now.
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

mod awake;
mod capture;
mod cue_layout;
pub mod deps;
mod engine;
mod feed;
mod host;
// HLS (`.m3u8` of MPEG-TS) taken apart here; the platform decodes the compressed streams.
// shortcut: macOS only refuses playlists (no `Elementary` there yet); a page reads HLS through
// it once its host does.
#[cfg_attr(not(any(windows, all(unix, not(any(target_os = "macos", target_os = "ios"))))), allow(dead_code))]
mod hls;
mod limits;
mod media_keys;
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
#[cfg(feature = "media-session")]
mod session;
mod stretch;
mod subtitles;
pub mod synth;
mod widget;

use std::sync::Arc;

pub use capture::{Microphone, MicrophoneOptions, MicrophoneState};
pub use deps::{DependencyState, MissingDependency};
pub use engine::{PlaybackState, PlaybackStats};
pub use feed::AudioFeed;
pub use host::{
    AudioFormat, AudioInput, AudioOutput, Decoder, EncodedPiece, Latency, MediaError, MediaHost, MediaInfo,
    MediaSource, Picture, PixelFormat, Plane, Playout, Sample, ScreenFrame, ScreenSource, ScreenStream,
    TextTrackInfo, VideoCodec, VideoEncoderOptions, VideoInfo, VideoStreamDecoder, VideoStreamEncoder,
    YuvMatrix, merge_ranges,
};
pub use limits::{MediaLimits, media_limits, set_media_limits};
pub use player::{
    CaptionChoice, CaptionPick, KeyScheme, NEXT_UP_DELAY, NextUp, PipWindowOpener, Player, PlayerOptions,
    Remote, RemoteControl, RemoteStatus, RemoteStatusSender, StageStatus, Timeline, UnseenPolicy,
    set_pip_window,
};
#[cfg(feature = "media-session")]
pub use remedy_ui_services_desktop::media_session::{Artwork, NowPlaying};
pub use stretch::{MAX_SPEED, MIN_SPEED, SPEEDS, clamp_speed};
pub use subtitles::{
    Cue, CueAlign, CueAnchor, CueLine, CueSettings, SubtitleFormat, SubtitleTrack, Vertical, parse_subtitles,
};
pub use widget::{
    NextUpPlacement, PlayerIcons, PlayerParts, format_time, player, player_picture, player_with,
};

/// The platform's own decoders and sound output, or [`MediaError::Unsupported`] with the
/// reason where there are none (no GStreamer installed on a Linux machine, a platform with
/// no backend).
pub fn platform_host() -> Result<Arc<dyn MediaHost>, MediaError> {
    os::platform_host(WebBackend::Auto)
}

/// The platform's own host, as [`platform_host`], playing through `backend` in a browser page
/// (elsewhere `backend` makes no difference).
pub fn platform_host_with(backend: WebBackend) -> Result<Arc<dyn MediaHost>, MediaError> {
    os::platform_host(backend)
}

/// Which of a browser's ways to play media a page uses ([`PlayerOptions::web_backend`]).
/// Elsewhere there is one way, and this makes no difference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WebBackend {
    /// WebCodecs where the page may use it, else Media Source Extensions. Browsers offer
    /// WebCodecs only to secure pages (`https:`, or `localhost`), so a page opened at a
    /// machine's LAN address (`http://192.168.1.20:8490`) plays through the media element.
    /// Firefox plays through the media element too, as a page's own `<video>` does (an HLS
    /// stream was seen to stall on its WebCodecs); `WebCodecs` still asks for its decoders.
    #[default]
    Auto,
    /// The browser's WebCodecs decoders, with sound through WebAudio and the crate's own
    /// MP4 and HLS readers: every picture passes through the player.
    WebCodecs,
    /// A hidden `<video>` element plays: HLS remuxed here from MPEG-TS to fragmented MP4 and
    /// fed to a Media Source Extensions `SourceBuffer`, any other source given to the element
    /// as it is. Its sound and clock are the element's; each frame it shows is drawn into the
    /// engine (uploaded from the element on the GPU, through a 2D canvas on the CPU renderer).
    MediaSource,
}

/// For leak tests: the platform objects alive (Media Foundation starts, D3D11 devices,
/// pictures on the GPU). Debug builds count them; release builds always say 0.
#[doc(hidden)]
pub fn live_platform_objects() -> usize {
    os::live_objects()
}

/// For measurements in a browser: a WebCodecs `VideoFrame`'s bytes copied into a picture as
/// the decoder copies each one it does not hand over on the GPU.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[doc(hidden)]
pub async fn copy_video_frame(frame: &wasm_bindgen::JsValue) -> Result<Picture, String> {
    os::web::copy_frame(frame).await
}

/// For leak tests under `REMEDY_D3D_DEBUG=1` on Windows: decoder D3D11 devices made from
/// now on are kept for [`live_d3d_objects`] (each until it reports on them). Without this
/// call none are kept, so no device outlives its decoder to the end of the process.
#[doc(hidden)]
pub fn watch_d3d_devices() {
    os::watch_d3d_devices();
}

/// For leak tests under `REMEDY_D3D_DEBUG=1` on Windows: `ReportLiveDeviceObjects` on every
/// decoder D3D11 device made since [`watch_d3d_devices`] or the last call, then those devices let go; the debug
/// layer's line for each object alive beyond the device itself. Empty elsewhere.
#[doc(hidden)]
pub fn live_d3d_objects() -> (usize, Vec<String>) {
    os::live_d3d_objects()
}

/// For tests and measurements: the HLS decoders made (not reused from the stream before on
/// the same host) since the process started.
#[doc(hidden)]
pub fn hls_decoders_made() -> usize {
    hls::decoders_made()
}

/// For leak tests: the threads this process runs, as (name, module it started in); empty
/// where the platform cannot tell, and the module empty where it does not say.
#[doc(hidden)]
pub fn process_threads() -> Vec<(String, String)> {
    os::process_threads()
}
