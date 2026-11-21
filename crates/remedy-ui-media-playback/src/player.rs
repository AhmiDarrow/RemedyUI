//! The UI thread's handle on a player: commands out, state in as signals.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use remedy_ui_media_video::VideoSink;
use remedy_ui_reactive::Signal;
use remedy_ui_view::{El, Ui};

use crate::engine::{self, Cmd, PlaybackState, PlaybackStats, Shared, lock};
use crate::host::{MediaError, MediaHost, MediaInfo, MediaSource};
use crate::subtitles::{SubtitleFormat, SubtitleTrack, parse_subtitles};

/// What a player does while it cannot be seen (scrolled away, hidden, its window minimized
/// or covered).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnseenPolicy {
    /// Media with pictures pauses and plays on when seen again; sound-only media plays on.
    #[default]
    PauseVideo,
    /// Plays on (sound keeps going); pictures are not drawn while unseen.
    KeepPlaying,
}

/// Which keys the [`player`](crate::player) widget answers to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyScheme {
    /// The arrows act on the focused seek bar or volume slider only, and next is Shift+N.
    #[default]
    Classic,
    /// As in a streaming site's player: anywhere in the player, Left and Right skip 10
    /// seconds, Up and Down change the volume by 5%, and N plays next (Shift+N too); letters
    /// work with Shift held; a click or tap on the picture plays and pauses. A focused seek bar or volume slider still takes the arrows,
    /// Home, End, Page Up and Page Down itself, Space plays and pauses unless a button has
    /// focus (which it presses), and a focused text field takes every key.
    Arrows,
}

/// How a player starts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlayerOptions {
    /// Starts playing once opened. Media with pictures does not start by itself while the
    /// system asks for reduced motion; the person presses play.
    pub autoplay: bool,
    /// What happens while the player is unseen.
    pub unseen: UnseenPolicy,
    /// Looks for subtitle files next to a local file: the same name with `.vtt`, `.srt`,
    /// `.ass` or `.ssa`, or with a language before it (`film.en.srt`).
    pub find_subtitles: bool,
    /// The starting volume, 0 to 1.
    pub volume: f32,
    /// How long the "up next" countdown runs once the media ends with a
    /// [`next up`](Player::set_next_up) set, before the next plays in this player; `None`
    /// never advances by itself (the person chooses). The countdown does not start while the
    /// system asks for reduced motion.
    pub next_up_delay: Option<Duration>,
    /// Which keys the widget answers to; [`KeyScheme::Classic`] by default.
    pub keys: KeyScheme,
}

/// Media queued to play after the current one ends ([`Player::set_next_up`],
/// [`Player::set_queue`], [`Player::enqueue`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NextUp {
    /// The media.
    pub source: MediaSource,
    /// What to call it ("Episode 4: The Long Night"), shown and read out.
    pub title: String,
}

impl NextUp {
    /// `source`, called `title`.
    pub fn new(source: MediaSource, title: impl Into<String>) -> Self {
        NextUp { source, title: title.into() }
    }
}

/// What a player in remote mode ([`Player::set_remote`]) passes each press on to: the app
/// implements it and tells the device (a TV it casts to, through its own server or session).
/// The player shows each change straight away; the app's next
/// [`set_remote_status`](Player::set_remote_status) says what the device really did.
pub trait RemoteControl {
    /// Play.
    fn play(&self);
    /// Pause.
    fn pause(&self);
    /// Move to `to`.
    fn seek(&self, to: Duration);
    /// Set the volume, 0 to 1.
    fn set_volume(&self, volume: f32);
    /// Mute or unmute.
    fn set_muted(&self, muted: bool);
    /// Show captions from the player's track `track` (an index into
    /// [`Player::subtitle_tracks`]), or none.
    fn set_captions(&self, track: Option<usize>);
    /// Play what is next (Shift+N, or Play now on the "Up next" card).
    fn next(&self);
}

/// The device a player controls in place of playing itself ([`Player::set_remote`]).
#[derive(Clone)]
#[non_exhaustive]
pub struct Remote {
    /// The device's name ("The Ris", "Living room TV"), shown and read out.
    pub name: String,
    /// What the player's controls act on.
    pub control: Rc<dyn RemoteControl>,
    /// Shown on the casting stage in place of the picture (the media's poster), if any.
    pub poster: Option<El>,
}

impl Remote {
    /// The device called `name`, driven through `control`.
    pub fn new(name: impl Into<String>, control: impl RemoteControl + 'static) -> Self {
        Remote { name: name.into(), control: Rc::new(control), poster: None }
    }

    /// Shows `poster` on the casting stage.
    pub fn poster(mut self, poster: El) -> Self {
        self.poster = Some(poster);
        self
    }
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Remote").field("name", &self.name).finish_non_exhaustive()
    }
}

/// What the remote device is doing, as the app last heard ([`Player::set_remote_status`]).
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteStatus {
    /// Playing, paused, buffering; [`PlaybackState::Opening`] while it connects.
    pub state: PlaybackState,
    /// Where it is in the media.
    pub position: Duration,
    /// How long the media lasts, once the device knows.
    pub length: Option<Duration>,
    /// Its volume, 0 to 1.
    pub volume: f32,
    /// Whether it is muted.
    pub muted: bool,
}

impl Default for RemoteStatus {
    fn default() -> Self {
        RemoteStatus {
            state: PlaybackState::Opening,
            position: Duration::ZERO,
            length: None,
            volume: 1.0,
            muted: false,
        }
    }
}

impl PlayerOptions {
    /// Sets `autoplay`: starts playing once opened.
    pub fn autoplay(mut self, value: bool) -> Self {
        self.autoplay = value;
        self
    }

    /// Sets `unseen`: what happens while the player is unseen.
    pub fn unseen(mut self, value: UnseenPolicy) -> Self {
        self.unseen = value;
        self
    }

    /// Sets `find_subtitles`: looks for subtitle files next to a local file: the same name with `.vtt`, `.srt`, `.ass` or `.ssa`, or with a language before it (`film.en.srt`).
    pub fn find_subtitles(mut self, value: bool) -> Self {
        self.find_subtitles = value;
        self
    }

    /// Sets `volume`: the starting volume, 0 to 1.
    pub fn volume(mut self, value: f32) -> Self {
        self.volume = value;
        self
    }

    /// Sets `next_up_delay`: how long the "up next" countdown runs; `None` never advances
    /// by itself.
    pub fn next_up_delay(mut self, value: Option<Duration>) -> Self {
        self.next_up_delay = value;
        self
    }

    /// Sets `keys`: which keys the widget answers to.
    pub fn keys(mut self, value: KeyScheme) -> Self {
        self.keys = value;
        self
    }
}

/// How long the "up next" countdown runs by default.
pub const NEXT_UP_DELAY: Duration = Duration::from_secs(5);

impl Default for PlayerOptions {
    fn default() -> Self {
        PlayerOptions {
            autoplay: false,
            unseen: UnseenPolicy::default(),
            find_subtitles: true,
            volume: 1.0,
            next_up_delay: Some(NEXT_UP_DELAY),
            keys: KeyScheme::Classic,
        }
    }
}

thread_local! {
    /// Each player on this (UI) thread, by number: what takes its news. The wake a player's
    /// thread posts carries only the number.
    static PLAYERS: RefCell<HashMap<u64, Weak<Inner>>> = RefCell::new(HashMap::new());
}

fn deliver(id: u64) {
    let p = PLAYERS.with(|p| p.borrow().get(&id).and_then(Weak::upgrade));
    if let Some(p) = p {
        p.pull();
    }
}

/// Hears a full-screen or theater request.
type FullscreenHandler = Rc<dyn Fn(bool)>;
/// Hears a press on the cast button.
type CastHandler = Rc<dyn Fn(&El)>;

struct Inner {
    id: u64,
    ui: remedy_ui_view::WeakUi,
    /// The engine playing the current media: replaced when the next one plays.
    tx: RefCell<Option<engine::Control>>,
    shared: RefCell<Arc<Shared>>,
    host: Result<Arc<dyn MediaHost>, MediaError>,
    options: PlayerOptions,
    state: Signal<PlaybackState>,
    position: Signal<Duration>,
    info: Signal<Option<MediaInfo>>,
    error: Signal<Option<MediaError>>,
    audio_error: Signal<Option<MediaError>>,
    stats: Signal<PlaybackStats>,
    buffered: Signal<Vec<std::ops::Range<Duration>>>,
    speed: Signal<f64>,
    volume: Signal<f32>,
    muted: Signal<bool>,
    tracks: Signal<Vec<SubtitleTrack>>,
    caption: Signal<Option<usize>>,
    /// Milliseconds every caption is shifted by: positive shows cues later.
    caption_delay: Signal<i64>,
    fullscreen: Signal<bool>,
    fullscreen_handlers: RefCell<Vec<FullscreenHandler>>,
    theater: Signal<bool>,
    theater_handlers: RefCell<Vec<FullscreenHandler>>,
    /// An app heard theater requests ([`Player::on_theater`]): the widget offers them.
    theater_offered: Signal<bool>,
    cast_handlers: RefCell<Vec<CastHandler>>,
    /// An app opens a device menu ([`Player::on_cast`]): the widget shows the cast button.
    cast_offered: Signal<bool>,
    /// The device the controls act on, in remote mode.
    remote: Signal<Option<Remote>>,
    remote_status: Signal<RemoteStatus>,
    /// Where each embedded text track sits in `tracks`.
    embedded: RefCell<Vec<usize>>,
    autoplayed: Cell<bool>,
    /// What plays after the current media, in order: the first is next up.
    queue: Signal<Vec<NextUp>>,
    /// Time left before the next plays, while the countdown runs.
    countdown: Signal<Option<Duration>>,
    /// The countdown's timer and when it ends.
    timer: Cell<Option<u64>>,
    deadline: Cell<Option<remedy_ui_core::time::Instant>>,
    /// Play once opened (the next media, chosen or counted down to).
    play_on_open: Cell<bool>,
    /// The person stopped the countdown: it does not start again at this end.
    declined: Cell<bool>,
    /// What the engine was told, to tell the next one: the picture's sink and whether it is
    /// seen.
    sink: RefCell<Option<VideoSink>>,
    visible: Cell<bool>,
}

impl Inner {
    fn send(&self, cmd: Cmd) {
        if let Some(tx) = self.tx.borrow().as_ref() {
            tx.send(cmd);
        }
    }

    /// Takes the news from the player's thread into the signals.
    fn pull(&self) {
        let shared = self.shared.borrow().clone();
        shared.posted.store(false, Ordering::Release);
        let (state, position, info, error, audio_error, stats, new_tracks, cues, buffered) = {
            let mut s = lock(&shared.status);
            (
                s.state,
                s.position,
                s.info.clone(),
                s.error.clone(),
                s.audio_error.clone(),
                s.stats,
                std::mem::take(&mut s.new_tracks),
                std::mem::take(&mut s.cues),
                s.buffered.clone(),
            )
        };
        remedy_ui_reactive::batch(|| {
            if info.is_some() && self.info.get_untracked().ok().flatten().is_none() {
                self.on_opened(info.as_ref());
            }
            if !new_tracks.is_empty() {
                self.tracks.update(|t| t.extend(new_tracks));
            }
            if !cues.is_empty() {
                let map = self.embedded.borrow().clone();
                self.tracks.update(|t| {
                    for (track, cue) in cues {
                        if let Some(tr) = map.get(track).and_then(|&i| t.get_mut(i)) {
                            let at = tr.cues.partition_point(|c| c.start <= cue.start);
                            tr.cues.insert(at, cue);
                        }
                    }
                });
            }
            self.info.set_if_changed(info);
            self.state.set_if_changed(state);
            self.position.set_if_changed(position);
            self.error.set_if_changed(error);
            self.audio_error.set_if_changed(audio_error);
            self.stats.set_if_changed(stats);
            self.buffered.set_if_changed(buffered);
        });
        // The end starts the countdown to what is next; leaving it (playing again, seeking
        // back) stops it.
        if state == PlaybackState::Ended {
            self.count_down();
        } else {
            self.declined.set(false);
            self.stop_countdown();
        }
    }

    /// Starts the "up next" countdown, unless it runs, nothing is next, the app asked for no
    /// countdown, or the system asks for reduced motion.
    fn count_down(&self) {
        if self.deadline.get().is_some()
            || self.declined.get()
            || self.queue.with_untracked(Vec::is_empty).unwrap_or(true)
        {
            return;
        }
        let Some(delay) = self.options.next_up_delay else { return };
        let Some(ui) = self.ui.upgrade() else { return };
        if ui.reduced_motion() {
            return;
        }
        self.deadline.set(Some(remedy_ui_core::time::Instant::now() + delay));
        self.tick();
    }

    /// One step of the countdown: shows the time left, or plays the next when it is up.
    fn tick(&self) {
        let Some(deadline) = self.deadline.get() else { return };
        let left = deadline.saturating_duration_since(remedy_ui_core::time::Instant::now());
        if left.is_zero() {
            self.advance();
            return;
        }
        self.countdown.set_if_changed(Some(left));
        let Some(ui) = self.ui.upgrade() else { return };
        let id = self.id;
        // Woken at each whole second left, and at the end.
        let step = match left.subsec_nanos() {
            0 => Duration::from_secs(1),
            n => Duration::from_nanos(u64::from(n)),
        };
        let timer = ui.after(step.min(left), move || {
            if let Some(p) = PLAYERS.with(|p| p.borrow().get(&id).and_then(Weak::upgrade)) {
                p.timer.set(None);
                p.tick();
            }
        });
        self.timer.set(Some(timer));
    }

    fn stop_countdown(&self) {
        self.deadline.set(None);
        if let Some(t) = self.timer.take()
            && let Some(ui) = self.ui.upgrade()
        {
            ui.cancel_timer(t);
        }
        self.countdown.set_if_changed(None);
    }

    /// Plays what is next, in this player, taking it off the queue.
    fn advance(&self) {
        self.stop_countdown();
        if self.queue.with_untracked(Vec::is_empty).unwrap_or(true) {
            return;
        }
        let mut next = None;
        self.queue.update(|q| next = Some(q.remove(0)));
        if let Some(next) = next {
            self.load(next.source);
        }
    }

    /// The queue changed: a new next lets the countdown start again at this end, and an
    /// empty queue stops it.
    fn queue_changed(&self) {
        self.declined.set(false);
        if self.queue.with_untracked(Vec::is_empty).unwrap_or(true) {
            self.stop_countdown();
        } else if self.state.get_untracked().ok() == Some(PlaybackState::Ended) {
            self.count_down();
        }
    }

    /// Replaces the media with `source` and plays it once open: a new engine, every signal
    /// about the media back to its start, the player's own settings (volume, picture, seen)
    /// carried over.
    fn load(&self, source: MediaSource) {
        self.send(Cmd::Close);
        let shared = Arc::new(Shared::default());
        let started = launch(self.id, &self.host, source, self.options.find_subtitles, shared.clone());
        *self.shared.borrow_mut() = shared;
        let failed = match started {
            Ok(tx) => {
                *self.tx.borrow_mut() = Some(tx);
                None
            }
            Err(e) => {
                *self.tx.borrow_mut() = None;
                Some(e)
            }
        };
        remedy_ui_reactive::batch(|| {
            self.state.set(if failed.is_some() { PlaybackState::Failed } else { PlaybackState::Opening });
            self.error.set(failed);
            self.position.set(Duration::ZERO);
            self.info.set(None);
            self.audio_error.set(None);
            self.stats.set(PlaybackStats::default());
            self.buffered.set(Vec::new());
            self.tracks.set(Vec::new());
            self.caption.set(None);
            // A shift fixes one subtitle file's timing, so it goes with the tracks.
            self.caption_delay.set(0);
        });
        self.embedded.borrow_mut().clear();
        self.play_on_open.set(true);
        let muted = self.muted.get_untracked().unwrap_or(false);
        self.send(Cmd::Volume(if muted { 0.0 } else { self.volume.get_untracked().unwrap_or(1.0) }));
        self.send(Cmd::PauseUnseen(self.options.unseen == UnseenPolicy::PauseVideo));
        self.send(Cmd::Sink(self.sink.borrow().clone()));
        self.send(Cmd::Visible(self.visible.get()));
        self.send(Cmd::Speed(self.speed.get_untracked().unwrap_or(1.0)));
    }

    fn on_opened(&self, info: Option<&MediaInfo>) {
        let Some(info) = info else { return };
        if !info.text_tracks.is_empty() {
            let mut map = Vec::new();
            self.tracks.update(|t| {
                for tt in &info.text_tracks {
                    map.push(t.len());
                    t.push(SubtitleTrack {
                        label: tt.label.clone(),
                        language: tt.language.clone(),
                        cues: Vec::new(),
                    });
                }
            });
            *self.embedded.borrow_mut() = map;
        }
        if self.play_on_open.replace(false) {
            // Chosen, or counted down to: it plays, reduced motion or not.
            self.send(Cmd::Play);
        } else if self.options.autoplay && !self.autoplayed.replace(true) {
            let reduced = self.ui.upgrade().is_some_and(|u| u.reduced_motion());
            if !(reduced && info.video.is_some()) {
                self.send(Cmd::Play);
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.send(Cmd::Close);
        if let (Some(t), Some(ui)) = (self.timer.take(), self.ui.upgrade()) {
            ui.cancel_timer(t);
        }
        let id = self.id;
        let _ = PLAYERS.try_with(|p| p.borrow_mut().remove(&id));
    }
}

/// A media player: opens a source on its own threads, decodes it with a [`MediaHost`], plays
/// its sound on the default output and shows its pictures in a video surface, with sound and
/// pictures kept together by the sound device's clock.
///
/// Its state, position and the rest are signals: reading them in a binding keeps the UI in
/// step. Cheap to clone; the player stops when the last clone is dropped.
///
/// ```no_run
/// # fn demo(ui: &remedy_ui_view::Ui) {
/// use remedy_ui_media_playback::{MediaSource, Player, PlayerOptions, player};
/// let p = Player::open_platform(ui, MediaSource::file("talk.mp4"), PlayerOptions::default());
/// ui.root().child(player(ui, &p, "Talk recording"));
/// # }
/// ```
#[derive(Clone)]
pub struct Player(Rc<Inner>);

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player").field("id", &self.0.id).finish_non_exhaustive()
    }
}

impl Player {
    /// Opens `source` with `host`'s decoders, on the UI thread of `ui`. The player starts in
    /// [`PlaybackState::Opening`]; it is [`PlaybackState::Failed`] with an
    /// [`error`](Player::error) if the source cannot be opened.
    pub fn open(ui: &Ui, host: Arc<dyn MediaHost>, source: MediaSource, options: PlayerOptions) -> Player {
        Self::start(ui, Ok(host), source, options)
    }

    /// Opens `source` with the platform's own decoders ([`platform_host`](crate::platform_host)).
    pub fn open_platform(ui: &Ui, source: MediaSource, options: PlayerOptions) -> Player {
        Self::start(ui, crate::platform_host(), source, options)
    }

    fn start(
        ui: &Ui,
        host: Result<Arc<dyn MediaHost>, MediaError>,
        source: MediaSource,
        options: PlayerOptions,
    ) -> Player {
        static IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = IDS.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::new(Shared::default());
        let started = launch(id, &host, source, options.find_subtitles, shared.clone());
        let (tx, failed) = match started {
            Ok(tx) => (Some(tx), None),
            Err(e) => (None, Some(e)),
        };
        let volume = options.volume.clamp(0.0, 1.0);
        let inner = Rc::new(Inner {
            id,
            ui: ui.downgrade(),
            tx: RefCell::new(tx),
            shared: RefCell::new(shared),
            host,
            state: Signal::new(if failed.is_some() { PlaybackState::Failed } else { PlaybackState::Opening }),
            position: Signal::new(Duration::ZERO),
            info: Signal::new(None),
            error: Signal::new(failed),
            audio_error: Signal::new(None),
            stats: Signal::new(PlaybackStats::default()),
            buffered: Signal::new(Vec::new()),
            speed: Signal::new(1.0),
            volume: Signal::new(volume),
            muted: Signal::new(false),
            tracks: Signal::new(Vec::new()),
            caption: Signal::new(None),
            caption_delay: Signal::new(0),
            fullscreen: Signal::new(false),
            fullscreen_handlers: RefCell::new(Vec::new()),
            theater: Signal::new(false),
            theater_handlers: RefCell::new(Vec::new()),
            theater_offered: Signal::new(false),
            cast_handlers: RefCell::new(Vec::new()),
            cast_offered: Signal::new(false),
            remote: Signal::new(None),
            remote_status: Signal::new(RemoteStatus::default()),
            embedded: RefCell::new(Vec::new()),
            autoplayed: Cell::new(false),
            queue: Signal::new(Vec::new()),
            countdown: Signal::new(None),
            timer: Cell::new(None),
            deadline: Cell::new(None),
            play_on_open: Cell::new(false),
            declined: Cell::new(false),
            sink: RefCell::new(None),
            visible: Cell::new(true),
            options,
        });
        PLAYERS.with(|p| p.borrow_mut().insert(id, Rc::downgrade(&inner)));
        let p = Player(inner);
        p.0.send(Cmd::Volume(volume));
        p.0.send(Cmd::PauseUnseen(p.0.options.unseen == UnseenPolicy::PauseVideo));
        p
    }

    /// The stretches of the media that can play without waiting for the source, in order:
    /// all of a local file; for a stream, what has arrived (where the platform's decoder
    /// says) and what is decoded ahead of the playhead. The seek bar shows them. Tracked.
    pub fn buffered(&self) -> Vec<std::ops::Range<Duration>> {
        self.0.buffered.get()
    }

    /// The playback speed: 1 is normal, 2 twice as fast. Tracked.
    pub fn speed(&self) -> f64 {
        self.0.speed.get()
    }

    /// Plays at `speed` (1 is normal), within [`MIN_SPEED`](crate::MIN_SPEED) to
    /// [`MAX_SPEED`](crate::MAX_SPEED) (a value that is not a number is 1). Sound keeps its
    /// pitch: it is time-stretched, not sped up. Pictures follow the same clock. It carries
    /// over to the next media ([`set_next_up`](Self::set_next_up)).
    pub fn set_speed(&self, speed: f64) {
        let speed = crate::stretch::clamp_speed(speed);
        self.0.speed.set_if_changed(speed);
        self.0.send(Cmd::Speed(speed));
    }

    /// Steps the speed to the next of [`SPEEDS`](crate::SPEEDS) up (`faster`) or down from
    /// the current one, staying at the ends.
    pub fn step_speed(&self, faster: bool) {
        let now = self.0.speed.get_untracked().unwrap_or(1.0);
        let next = if faster {
            crate::SPEEDS.iter().copied().find(|s| *s > now + 1e-9)
        } else {
            crate::SPEEDS.iter().rev().copied().find(|s| *s < now - 1e-9)
        };
        if let Some(s) = next {
            self.set_speed(s);
        }
    }

    /// Plays (from the start again, once ended). In remote mode, tells the device.
    pub fn play(&self) {
        match self.control() {
            Some(c) => {
                c.play();
                self.0.remote_status.update(|s| s.state = PlaybackState::Playing);
            }
            None => self.0.send(Cmd::Play),
        }
    }

    /// Pauses. In remote mode, tells the device.
    pub fn pause(&self) {
        match self.control() {
            Some(c) => {
                c.pause();
                self.0.remote_status.update(|s| s.state = PlaybackState::Paused);
            }
            None => self.0.send(Cmd::Pause),
        }
    }

    /// Plays when paused or ended, pauses when playing.
    pub fn toggle(&self) {
        let state = match self.control() {
            Some(_) => self.0.remote_status.with_untracked(|s| s.state).unwrap_or_default(),
            None => self.0.state.get_untracked().unwrap_or_default(),
        };
        match state {
            PlaybackState::Playing | PlaybackState::Buffering => self.pause(),
            _ => self.play(),
        }
    }

    /// Moves to `to` (clamped to the media's length). Does nothing in media that cannot seek.
    /// In remote mode, tells the device.
    pub fn seek(&self, to: Duration) {
        if let Some(c) = self.control() {
            let to =
                self.0.remote_status.with_untracked(|s| s.length).ok().flatten().map_or(to, |d| to.min(d));
            c.seek(to);
            self.0.remote_status.update(|s| s.position = to);
            return;
        }
        let to = self.duration_untracked().map_or(to, |d| to.min(d));
        if self.info_untracked().is_some_and(|i| i.seekable) {
            self.0.position.set(to);
            self.0.send(Cmd::Seek(to));
        }
    }

    /// Moves `seconds` forward (or back, when negative) from the current position.
    pub fn seek_by(&self, seconds: f64) {
        let now = match self.control() {
            Some(_) => self.0.remote_status.with_untracked(|s| s.position).unwrap_or_default(),
            None => self.0.position.get_untracked().unwrap_or_default(),
        };
        let to = (now.as_secs_f64() + seconds).max(0.0);
        self.seek(Duration::try_from_secs_f64(to).unwrap_or_default());
    }

    /// What it is doing (in remote mode, the device). Tracked.
    pub fn state(&self) -> PlaybackState {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.state),
            false => self.0.state.get(),
        }
    }

    /// Where it is in the media (in remote mode, the device). Tracked: changes as it plays
    /// (with each picture shown, and ten times a second for sound alone).
    pub fn position(&self) -> Duration {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.position),
            false => self.0.position.get(),
        }
    }

    /// How long the media lasts, once known (in remote mode, as the device says); `None`
    /// for live streams. Tracked.
    pub fn duration(&self) -> Option<Duration> {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.length),
            false => self.0.info.with(|i| i.as_ref().and_then(|i| i.duration)),
        }
    }

    fn duration_untracked(&self) -> Option<Duration> {
        self.info_untracked().and_then(|i| i.duration)
    }

    fn info_untracked(&self) -> Option<MediaInfo> {
        self.0.info.get_untracked().ok().flatten()
    }

    /// What the media holds, once opened. Tracked.
    pub fn info(&self) -> Option<MediaInfo> {
        self.0.info.get()
    }

    /// Why it failed, in [`PlaybackState::Failed`]. Tracked.
    pub fn error(&self) -> Option<MediaError> {
        self.0.error.get()
    }

    /// Why there is no sound, when the media has some but no output could play it (it plays
    /// on silently by the wall clock). Tracked.
    pub fn audio_error(&self) -> Option<MediaError> {
        self.0.audio_error.get()
    }

    /// How playback has gone. Tracked.
    pub fn stats(&self) -> PlaybackStats {
        self.0.stats.get()
    }

    /// The volume, 0 to 1 (in remote mode, the device's). Tracked.
    pub fn volume(&self) -> f32 {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.volume),
            false => self.0.volume.get(),
        }
    }

    /// Sets the volume, 0 to 1. In remote mode, the device's.
    pub fn set_volume(&self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        if let Some(c) = self.control() {
            // The slider follows the device: what it already says is not sent back.
            if self.0.remote_status.with_untracked(|s| s.volume).ok() != Some(v) {
                c.set_volume(v);
                self.0.remote_status.update(|s| s.volume = v);
            }
            return;
        }
        self.0.volume.set_if_changed(v);
        self.send_volume();
    }

    /// Whether the sound is muted (in remote mode, the device's). Tracked.
    pub fn is_muted(&self) -> bool {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.muted),
            false => self.0.muted.get(),
        }
    }

    /// Mutes or unmutes, keeping the volume. In remote mode, the device.
    pub fn set_muted(&self, on: bool) {
        if let Some(c) = self.control() {
            c.set_muted(on);
            self.0.remote_status.update(|s| s.muted = on);
            return;
        }
        self.0.muted.set_if_changed(on);
        self.send_volume();
    }

    fn send_volume(&self) {
        let muted = self.0.muted.get_untracked().unwrap_or(false);
        let v = if muted { 0.0 } else { self.0.volume.get_untracked().unwrap_or(1.0) };
        self.0.send(Cmd::Volume(v));
    }

    /// The caption tracks: embedded ones, files found next to the media, and those added.
    /// Tracked.
    pub fn subtitle_tracks(&self) -> Vec<SubtitleTrack> {
        self.0.tracks.get()
    }

    /// Adds a caption track; returns its index.
    pub fn add_subtitles(&self, track: SubtitleTrack) -> usize {
        let mut i = 0;
        self.0.tracks.update(|t| {
            i = t.len();
            t.push(track);
        });
        i
    }

    /// Reads a subtitle file (WebVTT, SRT, ASS/SSA, by its extension or its contents) and adds
    /// it as a track named `label`.
    pub fn load_subtitles(&self, path: &Path, label: impl Into<String>) -> Result<usize, MediaError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| MediaError::NotFound(format!("{}: {e}", path.display())))?;
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(SubtitleFormat::from_extension)
            .or_else(|| SubtitleFormat::sniff(&text))
            .ok_or_else(|| MediaError::Format(format!("{} is not a subtitle file", path.display())))?;
        let cues = parse_subtitles(&text, format);
        if cues.is_empty() {
            return Err(MediaError::Format(format!("{} holds no cues", path.display())));
        }
        Ok(self.add_subtitles(SubtitleTrack { label: label.into(), language: None, cues }))
    }

    /// The caption track shown, if any. Tracked.
    pub fn caption_track(&self) -> Option<usize> {
        self.0.caption.get()
    }

    /// Shows captions from track `i`, or none. In remote mode the device is told too: the
    /// choice is the same on both.
    pub fn set_caption_track(&self, i: Option<usize>) {
        self.0.caption.set_if_changed(i);
        if let Some(c) = self.control() {
            c.set_captions(i);
        }
    }

    /// The caption showing now, if captions are on and a cue covers the position (shifted
    /// by the [caption delay](Self::caption_delay)). Tracked.
    pub fn caption_text(&self) -> Option<String> {
        let sel = self.0.caption.get()?;
        let at = self.0.position.get();
        let delay = self.0.caption_delay.get();
        // Cues shown later are looked up earlier in the track, and the other way round.
        let at = if delay >= 0 {
            at.checked_sub(Duration::from_millis(delay.unsigned_abs()))?
        } else {
            at + Duration::from_millis(delay.unsigned_abs())
        };
        self.0.tracks.with(|t| t.get(sel).and_then(|tr| tr.text_at(at)))
    }

    /// How far every caption track is shifted, in milliseconds: positive shows each cue
    /// later, negative earlier. Tracked.
    pub fn caption_delay(&self) -> i64 {
        self.0.caption_delay.get()
    }

    /// Shifts every caption track (added, found next to the file, or embedded) by `ms`
    /// milliseconds, for subtitles out of step with the media: positive shows each cue later.
    /// Like the caption track chosen, it goes back to 0 when the next media plays
    /// ([`set_next_up`](Self::set_next_up)), whose subtitles are other files.
    pub fn set_caption_delay(&self, ms: i64) {
        self.0.caption_delay.set_if_changed(ms);
    }

    /// Whether the player asked to fill the screen. Tracked.
    pub fn is_fullscreen(&self) -> bool {
        self.0.fullscreen.get()
    }

    /// Asks to fill the screen, or to leave it. The app decides: it hears the request through
    /// [`Player::on_fullscreen`] (and typically makes its window full screen).
    pub fn request_fullscreen(&self, on: bool) {
        self.0.fullscreen.set_if_changed(on);
        let handlers = self.0.fullscreen_handlers.borrow().clone();
        for h in handlers {
            h(on);
        }
    }

    /// Calls `f` when the player asks to fill the screen (`true`) or leave it.
    pub fn on_fullscreen(&self, f: impl Fn(bool) + 'static) {
        self.0.fullscreen_handlers.borrow_mut().push(Rc::new(f));
    }

    /// Whether the player asked for theater mode (the picture wide in the page). Tracked.
    pub fn is_theater(&self) -> bool {
        self.0.theater.get()
    }

    /// Asks for theater mode, or to leave it. The app decides: it hears the request through
    /// [`Player::on_theater`] and lays its page out around the wide picture.
    pub fn request_theater(&self, on: bool) {
        self.0.theater.set_if_changed(on);
        let handlers = self.0.theater_handlers.borrow().clone();
        for h in handlers {
            h(on);
        }
    }

    /// Calls `f` when the player asks for theater mode (`true`) or to leave it. Theater mode
    /// is the app's to lay out, so the widget offers it (a button and the T key) only once an
    /// app listens.
    pub fn on_theater(&self, f: impl Fn(bool) + 'static) {
        self.0.theater_handlers.borrow_mut().push(Rc::new(f));
        self.0.theater_offered.set_if_changed(true);
    }

    /// Which keys the widget answers to.
    pub(crate) fn key_scheme(&self) -> KeyScheme {
        self.0.options.keys
    }

    /// Whether an app listens for theater requests. Tracked.
    pub(crate) fn theater_offered(&self) -> bool {
        self.0.theater_offered.get()
    }

    /// Calls `f` with the cast button when it is pressed: the app opens its menu of devices
    /// anchored to it (with [`menu`](remedy_ui_widgets_core::menu)), and on a choice puts the
    /// player in [remote mode](Self::set_remote). The widget shows the button only once an app
    /// listens.
    pub fn on_cast(&self, f: impl Fn(&El) + 'static) {
        self.0.cast_handlers.borrow_mut().push(Rc::new(f));
        self.0.cast_offered.set_if_changed(true);
    }

    /// Whether an app listens for the cast button. Tracked.
    pub(crate) fn cast_offered(&self) -> bool {
        self.0.cast_offered.get()
    }

    /// Tells the app the cast button (`button`) was pressed.
    pub(crate) fn request_cast(&self, button: &El) {
        let handlers = self.0.cast_handlers.borrow().clone();
        for h in handlers {
            h(button);
        }
    }

    /// Controls `remote` (a TV the app casts to) in place of playing here, or plays here again
    /// (`None`). While remote, the local media pauses and keeps its place; the picture gives
    /// way to a casting stage ("Playing on The Ris"); and play, pause, seeking, volume, mute,
    /// captions and next (every control and key of the widget, and these methods) act on the
    /// device through [`Remote::control`], while [`state`](Self::state),
    /// [`position`](Self::position), [`duration`](Self::duration), [`volume`](Self::volume)
    /// and [`is_muted`](Self::is_muted) say what the app last told
    /// [`set_remote_status`](Self::set_remote_status). Leaving, the app picks up where the
    /// device was: it reads [`position`](Self::position) first, then [`seek`](Self::seek)s and
    /// [`play`](Self::play)s here.
    pub fn set_remote(&self, remote: Option<Remote>) {
        if remote.is_some() {
            self.0.send(Cmd::Pause);
            // Connecting, at the local volume, until the app hears from the device.
            let (volume, muted) = (self.0.volume.get_untracked(), self.0.muted.get_untracked());
            self.0.remote_status.set(RemoteStatus {
                volume: volume.unwrap_or(1.0),
                muted: muted.unwrap_or(false),
                ..RemoteStatus::default()
            });
        }
        self.0.remote.set(remote);
    }

    /// The device's name, in remote mode. Tracked.
    pub fn remote_name(&self) -> Option<String> {
        self.0.remote.with(|r| r.as_ref().map(|r| r.name.clone()))
    }

    /// What the device is doing, as the app hears it (polled from its server, or pushed): the
    /// seek bar, the time, the volume and the casting stage follow it.
    pub fn set_remote_status(&self, status: RemoteStatus) {
        self.0.remote_status.set_if_changed(status);
    }

    /// Whether the player is in remote mode. Tracked.
    pub fn is_remote(&self) -> bool {
        self.0.remote.with(Option::is_some)
    }

    /// The remote's poster, in remote mode. Tracked.
    pub(crate) fn remote_poster(&self) -> Option<El> {
        self.0.remote.with(|r| r.as_ref().and_then(|r| r.poster.clone()))
    }

    fn control(&self) -> Option<Rc<dyn RemoteControl>> {
        self.0.remote.with_untracked(|r| r.as_ref().map(|r| r.control.clone())).ok().flatten()
    }

    /// Shows pictures in `sink` (the [`player`](crate::player) widget attaches its surface).
    pub fn attach(&self, sink: Option<VideoSink>) {
        *self.0.sink.borrow_mut() = sink.clone();
        self.0.send(Cmd::Sink(sink));
    }

    /// Tells the player whether it is seen (the widget does, from the engine's visibility).
    pub fn set_visible(&self, seen: bool) {
        self.0.visible.set(seen);
        self.0.send(Cmd::Visible(seen));
    }

    /// Sets what plays after this media ends, in this player, in place of the whole
    /// [queue](Self::set_queue) (`None`: nothing). At the end a countdown of
    /// [`PlayerOptions::next_up_delay`] runs ([`countdown`](Self::countdown)) and then the
    /// next plays; the [`player`](crate::player) widget shows it as "Up next" with Play now
    /// and Cancel. Setting it again while the countdown runs keeps the countdown going,
    /// towards the new media.
    pub fn set_next_up(&self, next: Option<NextUp>) {
        self.set_queue(next.into_iter().collect());
    }

    /// Sets what plays after this media, in order: when one ends the first of the rest
    /// counts down and plays, and so on until the queue is empty. Each plays in this player,
    /// taken off the queue as it starts. An empty queue stops a countdown that runs.
    pub fn set_queue(&self, queue: Vec<NextUp>) {
        self.0.queue.set(queue);
        self.0.queue_changed();
    }

    /// Adds `next` at the end of the queue (as next up, when the queue is empty; and if the
    /// media has already ended, its countdown starts).
    pub fn enqueue(&self, next: NextUp) {
        self.0.queue.update(|q| q.push(next));
        self.0.queue_changed();
    }

    /// What plays after this media, in order; the first is [next up](Self::next_up).
    /// Tracked.
    pub fn queue(&self) -> Vec<NextUp> {
        self.0.queue.get()
    }

    /// What plays after this media ends: the first in the queue. Tracked.
    pub fn next_up(&self) -> Option<NextUp> {
        self.0.queue.with(|q| q.first().cloned())
    }

    /// The time left before the next media plays, while the countdown at the end runs.
    /// Tracked.
    pub fn countdown(&self) -> Option<Duration> {
        self.0.countdown.get()
    }

    /// Stops the countdown; the next media stays set, to play when asked.
    pub fn cancel_countdown(&self) {
        self.0.declined.set(true);
        self.0.stop_countdown();
    }

    /// Plays the next media now, in this player, taking it off the queue (nothing when the
    /// queue is empty): the current one stops, and every signal about the media starts over
    /// with the next.
    /// In remote mode, the device is told ([`RemoteControl::next`]) and the app decides.
    pub fn play_next(&self) {
        match self.control() {
            Some(c) => c.next(),
            None => self.0.advance(),
        }
    }
}

/// Starts an engine playing `source` for player `id`, reporting into `shared`.
fn launch(
    id: u64,
    host: &Result<Arc<dyn MediaHost>, MediaError>,
    source: MediaSource,
    find_subtitles: bool,
    shared: Arc<Shared>,
) -> Result<engine::Control, MediaError> {
    let waker = remedy_ui_reactive::waker();
    match (host, waker) {
        (Ok(host), Ok(waker)) => {
            let notify: engine::Notify = Box::new(move || waker.post(move || deliver(id)).is_ok());
            engine::start(host.clone(), source, find_subtitles, shared, notify)
        }
        (Err(e), _) => Err(e.clone()),
        (_, Err(e)) => Err(MediaError::Unsupported(format!("no UI thread to report to: {e}"))),
    }
}

/// Subtitle files next to `media`: `name.vtt`, `name.srt`, `name.ass`, `name.ssa`, and the
/// same with a language or label between (`name.en.srt`, `name.forced.vtt`).
pub(crate) fn sidecar_subtitles(media: &Path) -> Vec<SubtitleTrack> {
    let (Some(dir), Some(stem)) = (media.parent(), media.file_stem().and_then(|s| s.to_str())) else {
        return Vec::new();
    };
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(String, SubtitleTrack)> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(rest) = name.strip_prefix(stem).and_then(|r| r.strip_prefix('.')) else { continue };
        let (middle, ext) = match rest.rsplit_once('.') {
            Some((m, x)) => (Some(m), x),
            None => (None, rest),
        };
        let Some(format) = SubtitleFormat::from_extension(ext) else { continue };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let cues = parse_subtitles(&text, format);
        if cues.is_empty() {
            continue;
        }
        let label = match middle {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => "Subtitles".to_string(),
        };
        let language = middle
            .filter(|m| (2..=3).contains(&m.len()) && m.bytes().all(|b| b.is_ascii_alphabetic()))
            .map(str::to_ascii_lowercase);
        found.push((name.to_string(), SubtitleTrack { label, language, cues }));
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().map(|(_, t)| t).collect()
}
