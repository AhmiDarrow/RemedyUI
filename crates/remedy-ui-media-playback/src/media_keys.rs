//! The keyboard's media keys and the system's media controls, as presses on a [`Player`]
//! (Portal's TV page: `MediaPlayPause`, `MediaPlay`, `MediaPause`, `MediaRewind`,
//! `MediaFastForward`; with Stop, Next and Previous besides).

use std::time::Duration;

use crate::{PlaybackState, Player};

/// How far Rewind and Fast-forward move, in seconds (as Portal's `skipBy`).
pub(crate) const MEDIA_SKIP: f64 = 10.0;

/// One press on the media.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Press {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    SeekBy(f64),
    SeekTo(Duration),
}

/// What a named media key means (the platform's and the web's names agree:
/// `KeyboardEvent.key` and winit's named keys).
pub(crate) fn key_press(name: &str) -> Option<Press> {
    Some(match name {
        "MediaPlayPause" => Press::Toggle,
        "MediaPlay" => Press::Play,
        "MediaPause" => Press::Pause,
        "MediaStop" => Press::Stop,
        "MediaTrackNext" => Press::Next,
        "MediaTrackPrevious" => Press::Previous,
        "MediaRewind" => Press::SeekBy(-MEDIA_SKIP),
        "MediaFastForward" => Press::SeekBy(MEDIA_SKIP),
        _ => return None,
    })
}

/// Does `press` on `p` (in remote mode, on the device); false when it means nothing now
/// (Next with nothing queued).
pub(crate) fn apply(p: &Player, press: Press) -> bool {
    let playing = remedy_ui_reactive::untrack(|| {
        matches!(p.state(), PlaybackState::Playing | PlaybackState::Buffering)
    });
    match press {
        Press::Toggle => p.toggle(),
        Press::Play if !playing => p.play(),
        Press::Pause | Press::Stop if playing => p.pause(),
        Press::Play | Press::Pause | Press::Stop => {}
        Press::Next if p.is_remote() || p.next_up().is_some() => p.play_next(),
        Press::Next => return false,
        Press::Previous => p.seek(Duration::ZERO),
        Press::SeekBy(s) => p.seek_by(s),
        Press::SeekTo(t) => p.seek(t),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_keys_mean_what_they_do_on_portals_page() {
        assert_eq!(key_press("MediaPlayPause"), Some(Press::Toggle));
        assert_eq!(key_press("MediaPlay"), Some(Press::Play));
        assert_eq!(key_press("MediaPause"), Some(Press::Pause));
        assert_eq!(key_press("MediaStop"), Some(Press::Stop));
        assert_eq!(key_press("MediaTrackNext"), Some(Press::Next));
        assert_eq!(key_press("MediaTrackPrevious"), Some(Press::Previous));
        assert_eq!(key_press("MediaRewind"), Some(Press::SeekBy(-10.0)));
        assert_eq!(key_press("MediaFastForward"), Some(Press::SeekBy(10.0)));
        assert_eq!(key_press("Insert"), None);
    }
}
