//! The system's media controls for real: on Windows the session registers with the SMTC,
//! shows its title, and gets the buttons the controlling side (the volume flyout's API,
//! `GlobalSystemMediaTransportControlsSessionManager`) presses.

#![cfg(windows)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use remedy_ui_services_desktop::media_session::{
    MediaCommand, MediaSession, MediaState, NowPlaying, NowPlayingSink,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Remote,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
};

/// Our session as the controlling side sees it, once the system lists it.
fn find(title: &str) -> Option<Remote> {
    let manager = Manager::RequestAsync().ok()?.join().ok()?;
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        for s in manager.GetSessions().ok()? {
            let shown = s.TryGetMediaPropertiesAsync().ok().and_then(|p| p.join().ok());
            if shown.and_then(|p| p.Title().ok()).is_some_and(|t| t == title) {
                return Some(s);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

#[test]
fn the_smtc_shows_what_plays_and_passes_its_buttons_on() {
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let session = MediaSession::new(move |c| {
        let _ = tx.lock().map(|t| t.send(c));
    })
    .unwrap_or_else(|e| unreachable!("the SMTC is on every Windows 10 and later: {e}"));
    let title = format!("RemedyUI test {}", std::process::id());
    session.set_now_playing(Some(&NowPlaying::new(title.as_str()).subtitle("S1 E3")));
    session.set_playback(MediaState::Playing, Duration::from_secs(30), Some(Duration::from_secs(100)));
    session.set_can_next(true);

    let Some(remote) = find(&title) else {
        // A machine without the shell's media service (Server Core, some CI images).
        eprintln!("the system lists no media sessions here; registering and showing worked");
        return;
    };
    let shown = remote.TryGetMediaPropertiesAsync().and_then(|p| p.join());
    let subtitle = shown.and_then(|p| p.Subtitle()).map(|s| s.to_string()).unwrap_or_default();
    assert_eq!(subtitle, "S1 E3");

    let pressed = |f: &dyn Fn() -> windows::core::Result<bool>, want: MediaCommand| {
        assert!(f().unwrap_or(false), "the system took the press");
        let got = rx.recv_timeout(Duration::from_secs(5));
        assert_eq!(got.ok(), Some(want));
    };
    pressed(&|| remote.TryPauseAsync()?.join(), MediaCommand::Pause);
    // The app paused; the system offers Play again only once it says so.
    session.set_playback(MediaState::Paused, Duration::from_secs(30), Some(Duration::from_secs(100)));
    pressed(&|| remote.TryPlayAsync()?.join(), MediaCommand::Play);
    pressed(&|| remote.TrySkipNextAsync()?.join(), MediaCommand::Next);
    pressed(
        &|| remote.TryChangePlaybackPositionAsync(50 * 10_000_000)?.join(),
        MediaCommand::SeekTo(Duration::from_secs(50)),
    );

    session.set_now_playing(None);
}
