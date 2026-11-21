//! The system's media controls: what plays now (title, episode, artwork) on the lock screen,
//! the volume flyout or the browser's media hub, and the keyboard's media keys, headsets and
//! those controls passed back to the app as [`MediaCommand`]s.
//!
//! - Windows: the System Media Transport Controls (SMTC), through a `Windows.Media.Playback`
//!   `MediaPlayer` whose own command handling is off, so no window is needed
//! - Browser: `navigator.mediaSession`
//! - Linux: MPRIS on the session bus (`org.mpris.MediaPlayer2.<app>.instance<pid>`); artwork
//!   given as bytes goes to a temporary file, since MPRIS takes a URL
//! - Android: a `MediaSession` with its media notification, through the Android bridge's
//!   Java (`dev.remedyui.android.MediaControls`, in the APK as `cargo remedy package` makes it)
//! - macOS (`MPNowPlayingInfoCenter`): [`DesktopError::Unsupported`] for now
//!
//! The media-playback pack keeps one in step with a player (`Player::now_playing`).

use std::time::Duration;

use crate::DesktopError;

/// What plays now, as the system's media controls show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NowPlaying {
    /// The title ("The Ris").
    pub title: String,
    /// The line under it: the episode, the artist, the show ("S1 E3 · The Crossing").
    pub subtitle: Option<String>,
    /// The picture beside it.
    pub art: Option<Artwork>,
}

impl NowPlaying {
    /// `title`, with no subtitle or artwork.
    pub fn new(title: impl Into<String>) -> Self {
        NowPlaying { title: title.into(), subtitle: None, art: None }
    }

    /// Sets the line under the title.
    pub fn subtitle(mut self, value: impl Into<String>) -> Self {
        self.subtitle = Some(value.into());
        self
    }

    /// Sets the picture.
    pub fn art(mut self, value: Artwork) -> Self {
        self.art = Some(value);
        self
    }
}

/// A picture for [`NowPlaying`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Artwork {
    /// An `http(s)` URL the system fetches itself.
    Url(String),
    /// A PNG or JPEG file's bytes.
    Bytes(Vec<u8>),
}

/// Whether the media plays, as the controls show it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MediaState {
    /// Nothing loaded, or it ended.
    #[default]
    Stopped,
    /// Playing (or waiting for data to play).
    Playing,
    /// Paused.
    Paused,
}

/// What the person asked of the media through the system's controls or the media keys.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum MediaCommand {
    /// Play.
    Play,
    /// Pause.
    Pause,
    /// Play when paused, pause when playing.
    PlayPause,
    /// Stop.
    Stop,
    /// The next item.
    Next,
    /// The previous item (or the start of this one).
    Previous,
    /// Move this many seconds (back when negative).
    SeekBy(f64),
    /// Move to this position.
    SeekTo(Duration),
}

/// Something that shows what plays: the platform's controls ([`MediaSession`]), or a stand-in
/// in tests.
pub trait NowPlayingSink {
    /// Shows `now`, or clears what was shown.
    fn set_now_playing(&self, now: Option<&NowPlaying>);
    /// Shows the state, the position and the length (`None`: a live stream, no seek bar).
    fn set_playback(&self, state: MediaState, position: Duration, length: Option<Duration>);
    /// Offers the Next button.
    fn set_can_next(&self, can: bool);
}

pub(crate) type Handler = Box<dyn Fn(MediaCommand) + Send + Sync>;

/// The app's entry in the system's media controls, while it lives. Commands come to the
/// handler given to [`MediaSession::new`] from a thread of the system's (in a browser, the
/// page's own).
pub struct MediaSession {
    inner: platform::Session,
}

impl std::fmt::Debug for MediaSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MediaSession")
    }
}

impl MediaSession {
    /// Registers with the system's media controls; `on_command` gets what the person presses.
    /// Nothing shows until [`set_now_playing`](NowPlayingSink::set_now_playing).
    pub fn new(
        on_command: impl Fn(MediaCommand) + Send + Sync + 'static,
    ) -> Result<MediaSession, DesktopError> {
        platform::Session::new(Box::new(on_command)).map(|inner| MediaSession { inner })
    }
}

impl NowPlayingSink for MediaSession {
    fn set_now_playing(&self, now: Option<&NowPlaying>) {
        self.inner.now_playing(now);
    }

    fn set_playback(&self, state: MediaState, position: Duration, length: Option<Duration>) {
        self.inner.playback(state, position, length);
    }

    fn set_can_next(&self, can: bool) {
        self.inner.can_next(can);
    }
}

#[cfg(windows)]
mod platform {
    pub(super) use crate::os::media::Session;
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod platform {
    pub(super) use crate::web_media::Session;
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
mod platform {
    pub(super) use crate::mpris::Session;
}

#[cfg(target_os = "android")]
mod platform {
    pub(super) use crate::os::android_media::Session;
}

#[cfg(not(any(
    windows,
    all(target_arch = "wasm32", target_os = "unknown"),
    target_os = "android",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
mod platform {
    use super::{DesktopError, Handler, MediaState, NowPlaying};
    use std::time::Duration;

    pub(crate) enum Session {}

    impl Session {
        pub(super) fn new(_: Handler) -> Result<Session, DesktopError> {
            let reason = if cfg!(target_os = "macos") {
                "MPNowPlayingInfoCenter is not wired up yet"
            } else if cfg!(target_os = "ios") {
                "MPNowPlayingInfoCenter is not wired up on iOS yet"
            } else {
                "this platform has no media controls"
            };
            Err(DesktopError::unsupported("Media controls", reason))
        }

        pub(super) fn now_playing(&self, _: Option<&NowPlaying>) {
            match *self {}
        }

        pub(super) fn playback(&self, _: MediaState, _: Duration, _: Option<Duration>) {
            match *self {}
        }

        pub(super) fn can_next(&self, _: bool) {
            match *self {}
        }
    }
}
