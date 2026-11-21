//! MPRIS (`org.mpris.MediaPlayer2` and its `.Player`) on the session bus, under
//! `org.mpris.MediaPlayer2.<app>.instance<pid>`: what GNOME's and KDE's media controls,
//! `playerctl` and the media keys' daemons read and press.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use zbus::object_server::SignalEmitter;
use zbus::{Connection, ObjectPath, OwnedObjectPath, OwnedVariant, Str, Variant, interface};

use crate::DesktopError;
use crate::media_session::{Artwork, Handler, MediaCommand, MediaState, NowPlaying};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// A position this far from where playback would be by now is a jump (`Seeked`), not drift.
const JUMP: Duration = Duration::from_millis(1500);

#[derive(Default)]
struct State {
    now: Option<NowPlaying>,
    /// Counts the items shown, for `mpris:trackid`.
    track: u64,
    art_url: Option<String>,
    art_file: Option<PathBuf>,
    state: MediaState,
    position: Duration,
    at: Option<Instant>,
    length: Option<Duration>,
    can_next: bool,
    /// 0 to 1; `None` until the player says (shown as full).
    volume: Option<f64>,
}

impl State {
    fn track_id(&self) -> String {
        match self.now {
            Some(_) => format!("/dev/remedyui/track/{}", self.track),
            None => NO_TRACK.to_owned(),
        }
    }

    /// Where playback is now: the last position given, moved on while playing.
    fn position(&self) -> Duration {
        let moved = match (self.state, self.at) {
            (MediaState::Playing, Some(at)) => self.position + at.elapsed(),
            _ => self.position,
        };
        self.length.map_or(moved, |l| moved.min(l))
    }

    fn metadata(&self) -> HashMap<String, OwnedVariant> {
        let mut m = HashMap::new();
        m.insert(
            "mpris:trackid".to_owned(),
            OwnedVariant::from(ObjectPath::from_string_unchecked(self.track_id())),
        );
        let Some(now) = &self.now else { return m };
        m.insert("xesam:title".to_owned(), OwnedVariant::from(Str::from(now.title.clone())));
        if let Some(s) = &now.subtitle
            && let Ok(v) = Variant::from(vec![s.clone()]).try_to_owned()
        {
            m.insert("xesam:artist".to_owned(), v);
        }
        if let Some(u) = &self.art_url {
            m.insert("mpris:artUrl".to_owned(), OwnedVariant::from(Str::from(u.clone())));
        }
        if let Some(l) = self.length {
            m.insert("mpris:length".to_owned(), OwnedVariant::from(micros(l)));
        }
        m
    }

    fn status(&self) -> &'static str {
        match self.state {
            MediaState::Playing => "Playing",
            MediaState::Paused => "Paused",
            MediaState::Stopped => "Stopped",
        }
    }
}

fn micros(d: Duration) -> i64 {
    i64::try_from(d.as_micros()).unwrap_or(i64::MAX)
}

fn lock(s: &Mutex<State>) -> MutexGuard<'_, State> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The app's name as the controls show it, and the bus name's middle part.
fn app_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "remedyui".to_owned())
}

/// A bus name element: letters, digits and `_`, not starting with a digit.
fn name_element(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        out.insert(0, '_');
    }
    out
}

struct Root {
    identity: String,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}

    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    state: Arc<Mutex<State>>,
    handler: Arc<Handler>,
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play(&self) {
        (self.handler)(MediaCommand::Play);
    }

    fn pause(&self) {
        (self.handler)(MediaCommand::Pause);
    }

    fn play_pause(&self) {
        (self.handler)(MediaCommand::PlayPause);
    }

    fn stop(&self) {
        (self.handler)(MediaCommand::Stop);
    }

    fn next(&self) {
        (self.handler)(MediaCommand::Next);
    }

    fn previous(&self) {
        (self.handler)(MediaCommand::Previous);
    }

    /// `offset` in microseconds.
    fn seek(&self, offset: i64) {
        #[allow(clippy::cast_precision_loss, reason = "seconds of media fit an f64")]
        (self.handler)(MediaCommand::SeekBy(offset as f64 / 1e6));
    }

    /// Ignored for an item no longer shown or a position before the start, as MPRIS says.
    fn set_position(&self, track_id: OwnedObjectPath, position: i64) {
        let current = lock(&self.state).track_id();
        if track_id.as_str() == current
            && let Ok(us) = u64::try_from(position)
        {
            (self.handler)(MediaCommand::SeekTo(Duration::from_micros(us)));
        }
    }

    fn open_uri(&self, _uri: String) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported("this player opens no URIs".into()))
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> &'static str {
        lock(&self.state).status()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        lock(&self.state).volume.unwrap_or(1.0)
    }

    /// Asks the player; the new volume shows once it says (`Session::volume`). Below 0 is 0,
    /// as MPRIS says; above 1 is 1 (the player goes no louder).
    #[zbus(property)]
    fn set_volume(&self, volume: f64) {
        if volume.is_finite() {
            (self.handler)(MediaCommand::SetVolume(volume.clamp(0.0, 1.0)));
        }
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedVariant> {
        lock(&self.state).metadata()
    }

    /// Microseconds; read when wanted (MPRIS sends no change for it, only `Seeked`).
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        micros(lock(&self.state).position())
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        lock(&self.state).can_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        lock(&self.state).now.is_some()
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        lock(&self.state).now.is_some()
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        lock(&self.state).now.is_some()
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        let s = lock(&self.state);
        s.now.is_some() && s.length.is_some()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
}

pub(crate) struct Session {
    conn: Connection,
    state: Arc<Mutex<State>>,
}

impl Session {
    pub(crate) fn new(handler: Handler) -> Result<Session, DesktopError> {
        let state = Arc::new(Mutex::new(State::default()));
        let identity = app_name();
        let name =
            format!("org.mpris.MediaPlayer2.{}.instance{}", name_element(&identity), std::process::id());
        let player = Player { state: state.clone(), handler: Arc::new(handler) };
        let conn = zbus::block_on(
            zbus::conn::Builder::session()
                .name(name)
                .serve_at(PATH, Root { identity })
                .serve_at(PATH, player)
                .build(),
        )
        .map_err(|e| DesktopError::unsupported("Media controls", format!("the session bus: {e}")))?;
        Ok(Session { conn, state })
    }

    /// Sends `PropertiesChanged` for the player's `names`, as they read now.
    fn changed(&self, names: &[&str]) {
        let values: Vec<(&str, OwnedVariant)> = {
            let s = lock(&self.state);
            names
                .iter()
                .filter_map(|&n| {
                    let v = match n {
                        "Metadata" => OwnedVariant::from(s.metadata()),
                        "PlaybackStatus" => OwnedVariant::from(Str::from_static(s.status())),
                        "CanGoNext" => OwnedVariant::from(s.can_next),
                        "Volume" => OwnedVariant::from(s.volume.unwrap_or(1.0)),
                        "CanSeek" => OwnedVariant::from(s.now.is_some() && s.length.is_some()),
                        "CanPlay" | "CanPause" | "CanGoPrevious" => OwnedVariant::from(s.now.is_some()),
                        _ => return None,
                    };
                    Some((n, v))
                })
                .collect()
        };
        if values.is_empty() {
            return;
        }
        let changed: HashMap<&str, Variant<'_>> =
            values.into_iter().map(|(n, v)| (n, Variant::from(v))).collect();
        let _ = zbus::block_on(async {
            let emitter = SignalEmitter::new(&self.conn, PATH)?;
            zbus::fdo::Properties::properties_changed(
                &emitter,
                zbus::names::InterfaceName::from_static_str_unchecked(PLAYER),
                changed,
                std::borrow::Cow::Borrowed(&[]),
            )
            .await
        });
    }

    pub(crate) fn now_playing(&self, now: Option<&NowPlaying>) {
        {
            let mut s = lock(&self.state);
            if let Some(old) = s.art_file.take() {
                let _ = std::fs::remove_file(old);
            }
            s.track += 1;
            s.art_url = None;
            match now.and_then(|n| n.art.as_ref()) {
                Some(Artwork::Url(u)) => s.art_url = Some(u.clone()),
                // MPRIS takes a URL only: the bytes go to a file the controls read.
                Some(Artwork::Bytes(b)) => {
                    let ext = if b.starts_with(b"\x89PNG") { "png" } else { "jpg" };
                    let file = std::env::temp_dir().join(format!(
                        "remedy-ui-art-{}-{}.{ext}",
                        std::process::id(),
                        s.track
                    ));
                    if std::fs::write(&file, b).is_ok() {
                        s.art_url = Some(crate::file_uri(&file));
                        s.art_file = Some(file);
                    }
                }
                None => {}
            }
            s.now = now.cloned();
        }
        self.changed(&["Metadata", "CanPlay", "CanPause", "CanGoPrevious", "CanSeek"]);
    }

    pub(crate) fn playback(&self, state: MediaState, position: Duration, length: Option<Duration>) {
        // The player calls this often; only what changed is sent.
        let (jumped, changed) = {
            let mut s = lock(&self.state);
            let jumped = s.at.is_some() && s.position().abs_diff(position) > JUMP;
            let mut changed = Vec::new();
            if s.state != state {
                changed.push("PlaybackStatus");
            }
            if s.length != length {
                changed.extend(["Metadata", "CanSeek"]);
            }
            s.state = state;
            s.position = position;
            s.at = Some(Instant::now());
            s.length = length;
            (jumped, changed)
        };
        self.changed(&changed);
        if jumped {
            let _ = zbus::block_on(async {
                let emitter = SignalEmitter::new(&self.conn, PATH)?;
                Player::seeked(&emitter, micros(position)).await
            });
        }
    }

    pub(crate) fn volume(&self, volume: f64) {
        let v = Some(volume.clamp(0.0, 1.0));
        if std::mem::replace(&mut lock(&self.state).volume, v) != v {
            self.changed(&["Volume"]);
        }
    }

    pub(crate) fn can_next(&self, can: bool) {
        lock(&self.state).can_next = can;
        self.changed(&["CanGoNext"]);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(f) = lock(&self.state).art_file.take() {
            let _ = std::fs::remove_file(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::name_element;

    #[test]
    fn bus_name_elements_are_valid() {
        assert_eq!(name_element("my-app.bin"), "my_app_bin");
        assert_eq!(name_element("9lives"), "_9lives");
    }
}
