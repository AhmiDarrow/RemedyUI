//! A player in the system's media controls ([`Player::now_playing`]): what plays, its state
//! and position kept in step, and the controls' presses brought back to the player on its UI
//! thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::time::Instant;
use remedy_ui_core::reactive::{Effect, Scope};
use remedy_ui_services_desktop::DesktopError;
use remedy_ui_services_desktop::media_session::{
    MediaCommand, MediaSession, MediaState, NowPlaying, NowPlayingSink,
};

use crate::media_keys::{Press, apply};
use crate::{PlaybackState, Player};

/// How far the shown position may drift from where the system has it by now before it is
/// told again (while playing it moves the shown position on by itself).
const DRIFT: Duration = Duration::from_secs(1);

/// What the system was last told: state, position, length, and when.
type Shown = (MediaState, Duration, Option<Duration>, Instant);

/// A player's entry in the controls while it lives; dropped, the entry is cleared.
struct Mirror {
    sink: Rc<dyn NowPlayingSink>,
    scope: Scope,
}

impl Drop for Mirror {
    fn drop(&mut self) {
        self.scope.dispose();
        self.sink.set_now_playing(None);
    }
}

thread_local! {
    static MIRRORS: RefCell<HashMap<u64, Mirror>> = RefCell::new(HashMap::new());
}

/// Clears a dropped player's entry from the controls.
pub(crate) fn forget(id: u64) {
    // Taken out first so the entry's drop runs outside the borrow.
    let gone = MIRRORS.try_with(|m| m.try_borrow_mut().ok().and_then(|mut m| m.remove(&id)));
    drop(gone);
}

fn press(c: MediaCommand) -> Option<Press> {
    Some(match c {
        MediaCommand::Play => Press::Play,
        MediaCommand::Pause => Press::Pause,
        MediaCommand::PlayPause => Press::Toggle,
        MediaCommand::Stop => Press::Stop,
        MediaCommand::Next => Press::Next,
        MediaCommand::Previous => Press::Previous,
        MediaCommand::SeekBy(s) => Press::SeekBy(s),
        MediaCommand::SeekTo(t) => Press::SeekTo(t),
        _ => return None,
    })
}

/// Does what the controls asked of `p`: a button, or the volume (MPRIS).
fn handle(p: &Player, c: MediaCommand) {
    if let MediaCommand::SetVolume(v) = c {
        #[allow(clippy::cast_possible_truncation, reason = "a volume, 0 to 1")]
        p.set_volume(v as f32);
    } else if let Some(m) = press(c) {
        apply(p, m);
    }
}

fn media_state(s: PlaybackState) -> MediaState {
    match s {
        PlaybackState::Playing | PlaybackState::Buffering => MediaState::Playing,
        PlaybackState::Paused | PlaybackState::Opening => MediaState::Paused,
        PlaybackState::Ended | PlaybackState::Failed => MediaState::Stopped,
    }
}

/// Whether the system must hear of `state` at `position` of `length` now: another state or
/// length, or a position more than [`DRIFT`] from where the last news has it by `at`.
fn due(
    last: Option<Shown>,
    state: MediaState,
    position: Duration,
    length: Option<Duration>,
    at: Instant,
) -> bool {
    let Some((s, p, l, then)) = last else { return true };
    if s != state || l != length {
        return true;
    }
    let expected = if s == MediaState::Playing { p + at.duration_since(then) } else { p };
    expected.abs_diff(position) > DRIFT
}

impl Player {
    /// Shows `now` in the system's media controls (Windows' SMTC on the lock screen and the
    /// volume flyout, a browser's media hub) and keeps them in step with the player: its
    /// state, position and length (on a [timeline](Self::set_timeline), the work's time; in
    /// remote mode, the device's). The controls' buttons, the keyboard's media keys wherever
    /// focus is, and headsets come back to the player (in remote mode, to the device).
    /// Again with `Some` changes what shows; `None` leaves the controls. Where the system has
    /// no media controls (macOS for now), says
    /// [`DesktopError::Unsupported`]; the player's own media keys still work.
    pub fn now_playing(&self, now: Option<NowPlaying>) -> Result<(), DesktopError> {
        let id = self.id();
        let Some(now) = now else {
            let gone = MIRRORS.with(|m| m.borrow_mut().remove(&id));
            drop(gone);
            return Ok(());
        };
        if let Some(sink) = MIRRORS.with(|m| m.borrow().get(&id).map(|m| m.sink.clone())) {
            sink.set_now_playing(Some(&now));
            return Ok(());
        }
        let waker =
            remedy_ui_core::reactive::waker().map_err(|e| DesktopError::Platform { reason: e.to_string() })?;
        let session = MediaSession::new(move |c| {
            let _ = waker.post(move || {
                if let Some(p) = Player::by_id(id) {
                    handle(&p, c);
                }
            });
        })?;
        self.mirror_to(Rc::new(session), &now);
        Ok(())
    }

    /// Keeps `sink` in step with the player, showing `now`.
    fn mirror_to(&self, sink: Rc<dyn NowPlayingSink>, now: &NowPlaying) {
        let id = self.id();
        sink.set_now_playing(Some(now));
        let s = sink.clone();
        let shown: Cell<Option<Shown>> = Cell::new(None);
        let next: Cell<Option<bool>> = Cell::new(None);
        let volume: Cell<Option<f32>> = Cell::new(None);
        let ((), scope) = Scope::detached(|| {
            let _ = Effect::new(move || {
                let Some(p) = Player::by_id(id) else { return };
                let (state, position, length) = (media_state(p.state()), p.position(), p.duration());
                let at = Instant::now();
                if due(shown.get(), state, position, length, at) {
                    s.set_playback(state, position, length);
                    shown.set(Some((state, position, length, at)));
                }
                let can = p.is_remote() || p.next_up().is_some();
                if next.replace(Some(can)) != Some(can) {
                    s.set_can_next(can);
                }
                let v = p.volume();
                if volume.replace(Some(v)) != Some(v) {
                    s.set_volume(f64::from(v));
                }
            });
        });
        let old = MIRRORS.with(|m| m.borrow_mut().insert(id, Mirror { sink, scope }));
        drop(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_hears_of_changes_not_of_every_frame() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let len = Some(s(100));
        assert!(due(None, MediaState::Paused, s(0), len, t0), "the first news");
        let playing = Some((MediaState::Playing, s(10), len, t0));
        assert!(!due(playing, MediaState::Playing, s(15), len, t0 + s(5)), "playing on as expected");
        assert!(due(playing, MediaState::Playing, s(40), len, t0 + s(5)), "a seek");
        assert!(due(playing, MediaState::Paused, s(15), len, t0 + s(5)), "paused");
        assert!(due(playing, MediaState::Playing, s(15), Some(s(90)), t0 + s(5)), "a new length");
        let paused = Some((MediaState::Paused, s(10), len, t0));
        assert!(!due(paused, MediaState::Paused, s(10), len, t0 + s(60)), "paused stays put");
    }

    #[test]
    fn the_controls_buttons_are_the_media_keys() {
        assert_eq!(press(MediaCommand::PlayPause), Some(Press::Toggle));
        assert_eq!(press(MediaCommand::SeekBy(-10.0)), Some(Press::SeekBy(-10.0)));
        assert_eq!(
            press(MediaCommand::SeekTo(Duration::from_secs(5))),
            Some(Press::SeekTo(Duration::from_secs(5)))
        );
        assert_eq!(media_state(PlaybackState::Buffering), MediaState::Playing);
        assert_eq!(media_state(PlaybackState::Ended), MediaState::Stopped);
    }

    struct Fake(Rc<RefCell<Vec<String>>>);

    impl NowPlayingSink for Fake {
        fn set_now_playing(&self, now: Option<&NowPlaying>) {
            let title = now.map_or("none", |n| n.title.as_str());
            self.0.borrow_mut().push(format!("now {title}"));
        }
        fn set_playback(&self, state: MediaState, position: Duration, length: Option<Duration>) {
            let l = length.map(|l| l.as_secs());
            self.0.borrow_mut().push(format!("{state:?} {} {l:?}", position.as_secs()));
        }
        fn set_can_next(&self, can: bool) {
            self.0.borrow_mut().push(format!("next {can}"));
        }
        fn set_volume(&self, volume: f64) {
            self.0.borrow_mut().push(format!("volume {volume}"));
        }
    }

    struct Tv(Rc<RefCell<Vec<String>>>);

    impl crate::RemoteControl for Tv {
        fn play(&self) {
            self.0.borrow_mut().push("play".into());
        }
        fn pause(&self) {
            self.0.borrow_mut().push("pause".into());
        }
        fn seek(&self, to: Duration) {
            self.0.borrow_mut().push(format!("seek {}", to.as_secs()));
        }
        fn set_volume(&self, _: f32) {}
        fn set_muted(&self, _: bool) {}
        fn set_captions(&self, _: Option<usize>) {}
        fn next(&self) {
            self.0.borrow_mut().push("next".into());
        }
    }

    #[test]
    fn the_controls_follow_a_remote_player_and_press_the_device() {
        let theme = remedy_ui_themes::ThemeChoice::default().resolve(None, false);
        let ui = remedy_ui_engine::view::Ui::new(
            theme.clone(),
            remedy_ui_themes::sheet(&theme),
            remedy_ui_text::FontCollection::new(),
        );
        let frame = || {
            let _ = ui.frame(remedy_ui_core::Size { width: 100.0, height: 100.0 }, 1.0);
        };
        let source = crate::MediaSource::file("no-such.mp4");
        let p = Player::open_platform(&ui, source, crate::PlayerOptions::default());
        let tv = Rc::new(RefCell::new(Vec::new()));
        p.set_remote(Some(crate::Remote::new("The Ris", Tv(tv.clone()))));
        let s = Duration::from_secs;
        p.set_remote_status(crate::RemoteStatus::new(PlaybackState::Paused, s(30)).length(s(100)));
        let seen = Rc::new(RefCell::new(Vec::new()));
        p.mirror_to(Rc::new(Fake(seen.clone())), &NowPlaying::new("The Ris").subtitle("S1 E3"));
        frame();
        assert_eq!(*seen.borrow(), ["now The Ris", "Paused 30 Some(100)", "next true", "volume 1"]);

        p.set_remote_status(crate::RemoteStatus::new(PlaybackState::Playing, s(30)).length(s(100)));
        frame();
        assert_eq!(seen.borrow().last().map(String::as_str), Some("Playing 30 Some(100)"));

        // The controls' buttons go to the device.
        for c in [MediaCommand::PlayPause, MediaCommand::SeekTo(s(50)), MediaCommand::Next] {
            assert!(press(c).is_some_and(|m| apply(&p, m)));
        }
        assert_eq!(*tv.borrow(), ["pause", "seek 50", "next"]);

        // The controls' volume (MPRIS) sets the player's, and the controls hear it back.
        handle(&p, MediaCommand::SetVolume(0.5));
        frame();
        assert!((p.volume() - 0.5).abs() < 1e-6);
        assert_eq!(seen.borrow().last().map(String::as_str), Some("volume 0.5"));

        assert!(p.now_playing(None).is_ok());
        assert_eq!(seen.borrow().last().map(String::as_str), Some("now none"), "left the controls");
        p.mirror_to(Rc::new(Fake(seen.clone())), &NowPlaying::new("Again"));
        frame();
        drop(p);
        assert_eq!(seen.borrow().last().map(String::as_str), Some("now none"), "dropped, cleared");
    }
}
