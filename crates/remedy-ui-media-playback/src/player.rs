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
    /// never advances by itself (the person chooses); zero plays it at once. A countdown does
    /// not start while the system asks for reduced motion (zero, which shows no countdown,
    /// still plays).
    pub next_up_delay: Option<Duration>,
    /// How long before the end the widget's "Up next" card shows, as Portal's page does (the
    /// last 20 seconds): while playing here (not remote) media over a minute long, measured
    /// in the work's time on a [timeline](Player::set_timeline). Cancel puts it away for that
    /// next; the end still plays it as [`next_up_delay`](Self::next_up_delay) says (Portal's
    /// autoplay is `Some(Duration::ZERO)`). While remote no card shows, at the end either: the
    /// device moves on by itself. `None` (the default): the card shows only at the end, also
    /// while remote.
    pub next_up_lead: Option<Duration>,
    /// Which keys the widget answers to; [`KeyScheme::Classic`] by default.
    pub keys: KeyScheme,
    /// Where the source sits in a longer work, if it is part of one ([`Player::set_timeline`]).
    pub timeline: Option<Timeline>,
}

/// Where a stream sits in the work it was made from: a film converted to a stream from a
/// resume or skip point, as a media server does. With one set ([`Player::set_timeline`]),
/// every position the player shows or takes is the work's ("film time"): stream time plus
/// [`origin`](Self::origin).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Timeline {
    /// The work's time at the stream's time 0.
    pub origin: Duration,
    /// How long the whole work lasts, when known: the seek bar spans it. Unknown, the
    /// stream's length plus the origin.
    pub length: Option<Duration>,
}

impl Timeline {
    /// The stream starts `origin` into the work: `Timeline::new(at).length(full)`.
    pub fn new(origin: Duration) -> Self {
        Timeline { origin, length: None }
    }

    /// Sets `length`: how long the whole work lasts.
    pub fn length(mut self, value: Duration) -> Self {
        self.length = Some(value);
        self
    }
}

/// Media queued to play after the current one ends ([`Player::set_next_up`],
/// [`Player::set_queue`], [`Player::enqueue`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NextUp {
    /// The media; `None` when the app makes it only once it is time to play it
    /// ([`NextUp::later`]).
    pub source: Option<MediaSource>,
    /// What to call it ("Episode 4: The Long Night"), shown and read out.
    pub title: String,
    /// Its timeline, if it is a stream of a longer work; none by default.
    pub timeline: Option<Timeline>,
}

impl NextUp {
    /// `source`, called `title`.
    pub fn new(source: MediaSource, title: impl Into<String>) -> Self {
        NextUp { source: Some(source), title: title.into(), timeline: None }
    }

    /// Something called `title` whose source the app makes only when it is time to play it,
    /// as a media server starts a play job for the next episode (Portal's page): the player
    /// asks with [`Player::on_next_needed`] and the app answers with
    /// [`Player::start_next`] or [`Player::next_failed`].
    pub fn later(title: impl Into<String>) -> Self {
        NextUp { source: None, title: title.into(), timeline: None }
    }

    /// Plays it on `timeline`.
    pub fn timeline(mut self, value: Timeline) -> Self {
        self.timeline = Some(value);
        self
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
    /// Its volume, 0 to 1; `None` until the device reports it: the volume slider and keys
    /// wait, since a step from a level the device has not said could jump it to full.
    pub volume: Option<f32>,
    /// Whether it is muted.
    pub muted: bool,
    /// The title of what the device starts next by itself (a media server's autoplay), while
    /// it starts it: the casting stage says so and the seek bar waits.
    pub next: Option<String>,
}

impl RemoteStatus {
    /// A device in `state` at `position`; its length and volume unknown, not muted until
    /// set: `RemoteStatus::new(PlaybackState::Playing, at).length(len).volume(0.5)`.
    pub fn new(state: PlaybackState, position: Duration) -> Self {
        RemoteStatus { state, position, ..RemoteStatus::default() }
    }

    /// Sets `length`: how long the media lasts.
    pub fn length(mut self, value: Duration) -> Self {
        self.length = Some(value);
        self
    }

    /// Sets `volume`, 0 to 1.
    pub fn volume(mut self, value: f32) -> Self {
        self.volume = Some(value.clamp(0.0, 1.0));
        self
    }

    /// Sets `muted`.
    pub fn muted(mut self, value: bool) -> Self {
        self.muted = value;
        self
    }

    /// Sets `next`: the device is starting `title` by itself.
    pub fn starting_next(mut self, title: impl Into<String>) -> Self {
        self.next = Some(title.into());
        self
    }
}

/// Hands [`RemoteStatus`] news to a [`Player`] from any thread (a poll of the TV's server
/// running on its own thread): [`send`](Self::send) queues it onto the player's UI thread,
/// where it is [`set_remote_status`](Player::set_remote_status). `Send` and cheap to clone;
/// from [`Player::remote_status_sender`].
#[derive(Clone)]
pub struct RemoteStatusSender {
    id: u64,
    waker: Option<remedy_ui_reactive::Waker>,
}

impl std::fmt::Debug for RemoteStatusSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteStatusSender").field("player", &self.id).finish()
    }
}

impl RemoteStatusSender {
    /// Queues `status` for the player. False once the player's UI thread is gone (the player
    /// itself gone is not an error: the news is dropped there).
    pub fn send(&self, status: RemoteStatus) -> bool {
        let id = self.id;
        self.waker.as_ref().is_some_and(|w| {
            w.post(move || {
                let p = PLAYERS.with(|p| p.borrow().get(&id).and_then(Weak::upgrade));
                if let Some(p) = p {
                    Player(p).set_remote_status(status);
                }
            })
            .is_ok()
        })
    }
}

/// How long a seek on the device shows its target, waiting for the device to report it is
/// there (within [`SEEK_LANDED`]), before its reports are believed again.
const SEEK_LAND: Duration = Duration::from_secs(8);
/// How near the target a device's report counts as having landed.
const SEEK_LANDED: Duration = Duration::from_secs(4);
/// How long a playing device's last report is moved on by the time since, before the shown
/// position waits for news (Portal's `DEVICE_STALE_MS`).
const REMOTE_STALE: Duration = Duration::from_secs(15);
/// How often a playing device's shown position moves on between reports.
const REMOTE_TICK: Duration = Duration::from_millis(250);
/// How far from the end of what a stream can play a seek in place may land (the next
/// segment may not exist yet), as in Portal's `seekInPlace`.
/// How long the volume rests before it goes to the device, as Portal's page waits: a dragged
/// slider sends where it stops.
const REMOTE_VOLUME_REST: Duration = Duration::from_millis(300);
const STREAM_EDGE: Duration = Duration::from_secs(2);

/// A device's report `at`, `elapsed` ago, moved on while it plays: at most [`REMOTE_STALE`]
/// and never past `end`.
fn moved_on(at: Duration, elapsed: Duration, end: Option<Duration>) -> Duration {
    let to = at + elapsed.min(REMOTE_STALE);
    end.map_or(to, |e| to.min(e))
}

/// A seek target on a work's timeline: whole seconds, within its length (Portal's `clampSkip`).
fn clamp_skip(to: Duration, length: Option<Duration>) -> Duration {
    let at = Duration::from_secs(to.as_secs());
    length.map_or(at, |l| at.min(Duration::from_secs(l.as_secs())))
}

impl Default for RemoteStatus {
    fn default() -> Self {
        RemoteStatus {
            state: PlaybackState::Opening,
            position: Duration::ZERO,
            length: None,
            volume: None,
            muted: false,
            next: None,
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

    /// Sets `next_up_lead`: how long before the end the "Up next" card shows.
    pub fn next_up_lead(mut self, value: Option<Duration>) -> Self {
        self.next_up_lead = value;
        self
    }

    /// Sets `keys`: which keys the widget answers to.
    pub fn keys(mut self, value: KeyScheme) -> Self {
        self.keys = value;
        self
    }

    /// Sets `timeline`: where the source sits in a longer work.
    pub fn timeline(mut self, value: Timeline) -> Self {
        self.timeline = Some(value);
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
            timeline: None,
            next_up_lead: None,
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
/// Hears a seek outside what the stream holds.
type SeekOutsideHandler = Rc<dyn Fn(Duration)>;
/// Hears that a next made later is due.
type NextNeededHandler = Rc<dyn Fn(&NextUp)>;
/// Hears Cancel pressed while the app works.
type CancelHandler = Rc<dyn Fn()>;

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
    /// A seek sent to the device: its target and when, until the device reports it landed.
    remote_seek: Cell<Option<(Duration, remedy_ui_core::time::Instant)>>,
    /// The device's last known position and when: moved on from there while it plays.
    remote_seen: Cell<Option<(Duration, remedy_ui_core::time::Instant)>>,
    remote_timer: Cell<Option<u64>>,
    /// The device's volume waiting to be sent ([`REMOTE_VOLUME_REST`]).
    remote_volume_timer: Cell<Option<u64>>,
    /// Where the seek bar is held while pressed or dragged, before the seek on release.
    scrubbing: Signal<Option<Duration>>,
    /// Where the stream sits in a longer work.
    timeline: Signal<Option<Timeline>>,
    /// A seek outside the stream handed to the app: its target, until a new source comes.
    skipping: Signal<Option<Duration>>,
    /// The skip is a hold for a new source of the same place ([`Player::hold_for_source`]).
    holding: Signal<bool>,
    /// Seconds pressed while skipping, from its target: run once the new source comes.
    queued_skip: Cell<Option<f64>>,
    seek_outside_handlers: RefCell<Vec<SeekOutsideHandler>>,
    /// The embedded track (by its order) chosen before the source was replaced: chosen again
    /// in the new source once open.
    rechoose_embedded: Cell<Option<usize>>,
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
    /// The title of a next made later, while the app makes it, and what the app says it is
    /// doing.
    starting: Signal<Option<String>>,
    starting_detail: Signal<Option<String>>,
    /// Why the app could not make the next, until something else happens.
    next_error: Signal<Option<String>>,
    next_needed_handlers: RefCell<Vec<NextNeededHandler>>,
    /// The app can stop its work (starting the next, or a skip) now: Cancel shows.
    cancel_offered: Signal<bool>,
    /// Cancel was pressed: the app is stopping its work.
    canceling: Signal<bool>,
    cancel_handlers: RefCell<Vec<CancelHandler>>,
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
            || self.is_starting()
            || self.queue.with_untracked(Vec::is_empty).unwrap_or(true)
        {
            return;
        }
        let Some(delay) = self.options.next_up_delay else { return };
        let Some(ui) = self.ui.upgrade() else { return };
        if ui.reduced_motion() && !delay.is_zero() {
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

    /// Plays what is next, in this player, taking it off the queue; a next made later is
    /// asked of the app instead, and stays queued until it comes.
    fn advance(&self) {
        self.stop_countdown();
        if self.is_starting() {
            return;
        }
        let Some(next) = self.queue.with_untracked(|q| q.first().cloned()).ok().flatten() else { return };
        if next.source.is_none() {
            let handlers = self.next_needed_handlers.borrow().clone();
            // While remote the device moves on by itself.
            if handlers.is_empty() || self.remote.with_untracked(Option::is_some).unwrap_or(false) {
                return;
            }
            remedy_ui_reactive::batch(|| {
                self.next_error.set_if_changed(None);
                self.starting_detail.set_if_changed(None);
                self.end_cancel();
                self.starting.set(Some(next.title.clone()));
            });
            // As Portal's page leaves the old episode: nothing plays while the next starts.
            self.send(Cmd::Pause);
            for h in handlers {
                h(&next);
            }
            return;
        }
        let mut next = None;
        self.queue.update(|q| next = Some(q.remove(0)));
        if let Some(NextUp { source: Some(source), timeline, .. }) = next {
            self.load(source, timeline, false, false);
        }
    }

    /// The app's work ended (or a new one began): no Cancel until it offers one again.
    fn end_cancel(&self) {
        self.cancel_offered.set_if_changed(false);
        self.canceling.set_if_changed(false);
    }

    /// Leaves the engine's decoder alone (no fetching) while skipping or remote, as Portal's
    /// page stops loading the stream; lets it go on otherwise.
    fn sync_suspend(&self) {
        let skipping = self.skipping.with_untracked(Option::is_some).unwrap_or(false);
        let remote = self.remote.with_untracked(Option::is_some).unwrap_or(false);
        self.send(Cmd::Suspend(skipping || remote));
    }

    /// Pauses and waits for the app's new source, from `at` (the work's time).
    fn begin_skip(&self, at: Duration, holding: bool) {
        self.queued_skip.set(None);
        self.end_cancel();
        remedy_ui_reactive::batch(|| {
            self.holding.set_if_changed(holding);
            self.skipping.set(Some(at));
        });
        self.send(Cmd::Pause);
        self.sync_suspend();
    }

    fn is_starting(&self) -> bool {
        self.starting.with_untracked(Option::is_some).unwrap_or(false)
    }

    /// Moves a playing device's shown position on in a while, and again after, until news
    /// comes or its last report is [stale](REMOTE_STALE).
    /// Sends `v` to the device once the volume has rested ([`REMOTE_VOLUME_REST`]), unless
    /// it is no longer in control.
    fn send_remote_volume_soon(&self, v: f32) {
        let Some(ui) = self.ui.upgrade() else { return };
        if let Some(t) = self.remote_volume_timer.take() {
            ui.cancel_timer(t);
        }
        let id = self.id;
        let timer = ui.after(REMOTE_VOLUME_REST, move || {
            if let Some(p) = PLAYERS.with(|p| p.borrow().get(&id).and_then(Weak::upgrade)) {
                p.remote_volume_timer.set(None);
                let control =
                    p.remote.with_untracked(|r| r.as_ref().map(|r| r.control.clone())).ok().flatten();
                if let Some(c) = control {
                    c.set_volume(v);
                }
            }
        });
        self.remote_volume_timer.set(Some(timer));
    }

    fn schedule_remote_tick(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if let Some(t) = self.remote_timer.take() {
            ui.cancel_timer(t);
        }
        let playing =
            self.remote_status.with_untracked(|s| s.state == PlaybackState::Playing).unwrap_or(false);
        let fresh = self.remote_seen.get().is_some_and(|(_, seen)| seen.elapsed() < REMOTE_STALE);
        // A seek in flight shows its target, not a report from before it.
        if !playing || !fresh || self.remote_seek.get().is_some() {
            return;
        }
        let id = self.id;
        let timer = ui.after(REMOTE_TICK, move || {
            if let Some(p) = PLAYERS.with(|p| p.borrow().get(&id).and_then(Weak::upgrade)) {
                p.remote_timer.set(None);
                if let Some((at, seen)) = p.remote_seen.get() {
                    let end = p.timeline.with_untracked(|t| t.and_then(|t| t.length)).ok().flatten();
                    let end = end.or_else(|| p.remote_status.with_untracked(|s| s.length).ok().flatten());
                    let to = moved_on(at, seen.elapsed(), end);
                    if p.remote_status.with_untracked(|s| s.position).ok() != Some(to) {
                        p.remote_status.update(|s| s.position = to);
                    }
                }
                p.schedule_remote_tick();
            }
        });
        self.remote_timer.set(Some(timer));
    }

    /// The device's shown position, from now on, is where it moves on from.
    fn anchor_remote(&self) {
        let at = self.remote_status.with_untracked(|s| s.position).unwrap_or_default();
        self.remote_seen.set(Some((at, remedy_ui_core::time::Instant::now())));
        self.schedule_remote_tick();
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

    /// Replaces the media with `source` on `timeline` and plays it once open (not in remote
    /// mode): a new engine, every signal about the media back to its start, the player's own
    /// settings (volume, picture, seen) carried over. `same_work` keeps the tracks the app
    /// added, the caption choice and its delay: the new source is more of the same work.
    /// `choice` keeps the choice of an embedded track (by its order) and the delay alone.
    fn load(&self, source: MediaSource, timeline: Option<Timeline>, same_work: bool, choice: bool) {
        // The app's tracks (on the work's timeline) stay; the old source's embedded ones go.
        let mut kept = (Vec::new(), None, None, 0);
        if same_work || choice {
            let embedded = self.embedded.borrow().clone();
            let chosen = self.caption.get_untracked().ok().flatten();
            for (i, t) in self.tracks.get_untracked().unwrap_or_default().into_iter().enumerate() {
                if let Some(k) = embedded.iter().position(|&e| e == i) {
                    kept.2 = kept.2.or((chosen == Some(i)).then_some(k));
                } else if same_work {
                    kept.1 = kept.1.or((chosen == Some(i)).then_some(kept.0.len()));
                    kept.0.push(t);
                }
            }
            kept.3 = self.caption_delay.get_untracked().unwrap_or(0);
        }
        let (tracks, caption, rechoose, delay) = kept;
        self.rechoose_embedded.set(rechoose);
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
            self.tracks.set(tracks);
            self.caption.set(caption);
            // A shift fixes one subtitle file's timing, so it goes with the tracks.
            self.caption_delay.set(delay);
            self.timeline.set(timeline);
            self.skipping.set(None);
            self.holding.set_if_changed(false);
            self.starting.set_if_changed(None);
            self.starting_detail.set_if_changed(None);
            self.next_error.set_if_changed(None);
            self.end_cancel();
        });
        self.embedded.borrow_mut().clear();
        self.play_on_open.set(self.remote.with_untracked(Option::is_none).unwrap_or(true));
        let muted = self.muted.get_untracked().unwrap_or(false);
        self.send(Cmd::Volume(if muted { 0.0 } else { self.volume.get_untracked().unwrap_or(1.0) }));
        self.send(Cmd::PauseUnseen(self.options.unseen == UnseenPolicy::PauseVideo));
        self.send(Cmd::Sink(self.sink.borrow().clone()));
        self.send(Cmd::Visible(self.visible.get()));
        self.send(Cmd::Speed(self.speed.get_untracked().unwrap_or(1.0)));
        self.sync_suspend();
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
            if let Some(&i) = self.rechoose_embedded.take().and_then(|k| map.get(k)) {
                self.caption.set(Some(i));
            }
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
        if let Some(ui) = self.ui.upgrade() {
            for t in [self.timer.take(), self.remote_timer.take()].into_iter().flatten() {
                ui.cancel_timer(t);
            }
        }
        let id = self.id;
        let _ = PLAYERS.try_with(|p| p.borrow_mut().remove(&id));
        #[cfg(feature = "media-session")]
        crate::session::forget(id);
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
        Self::start(ui, Ok(host), Some(source), options)
    }

    /// Opens `source` with the platform's own decoders ([`platform_host`](crate::platform_host)).
    pub fn open_platform(ui: &Ui, source: MediaSource, options: PlayerOptions) -> Player {
        Self::start(ui, crate::platform_host(), Some(source), options)
    }

    /// A player with nothing to play here yet, in [`PlaybackState::Opening`]: put in
    /// [remote mode](Self::set_remote) at once (as Portal's page picks up a cast already
    /// running when it opens), or given its media later with
    /// [`replace_source`](Self::replace_source) or [`start_next`](Self::start_next).
    pub fn empty(ui: &Ui, host: Arc<dyn MediaHost>, options: PlayerOptions) -> Player {
        Self::start(ui, Ok(host), None, options)
    }

    /// [`empty`](Self::empty), with the platform's own decoders.
    pub fn empty_platform(ui: &Ui, options: PlayerOptions) -> Player {
        Self::start(ui, crate::platform_host(), None, options)
    }

    fn start(
        ui: &Ui,
        host: Result<Arc<dyn MediaHost>, MediaError>,
        source: Option<MediaSource>,
        options: PlayerOptions,
    ) -> Player {
        static IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = IDS.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::new(Shared::default());
        let started = source.map(|s| launch(id, &host, s, options.find_subtitles, shared.clone()));
        let (tx, failed) = match started {
            Some(Ok(tx)) => (Some(tx), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, None),
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
            remote_seek: Cell::new(None),
            remote_seen: Cell::new(None),
            remote_timer: Cell::new(None),
            remote_volume_timer: Cell::new(None),
            scrubbing: Signal::new(None),
            timeline: Signal::new(options.timeline),
            skipping: Signal::new(None),
            holding: Signal::new(false),
            queued_skip: Cell::new(None),
            seek_outside_handlers: RefCell::new(Vec::new()),
            rechoose_embedded: Cell::new(None),
            embedded: RefCell::new(Vec::new()),
            autoplayed: Cell::new(false),
            queue: Signal::new(Vec::new()),
            countdown: Signal::new(None),
            timer: Cell::new(None),
            deadline: Cell::new(None),
            play_on_open: Cell::new(false),
            declined: Cell::new(false),
            starting: Signal::new(None),
            starting_detail: Signal::new(None),
            next_error: Signal::new(None),
            next_needed_handlers: RefCell::new(Vec::new()),
            cancel_offered: Signal::new(false),
            canceling: Signal::new(false),
            cancel_handlers: RefCell::new(Vec::new()),
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
    /// says) and what is decoded ahead of the playhead. The seek bar shows them. On a
    /// [timeline](Self::set_timeline), in the work's time. Tracked.
    pub fn buffered(&self) -> Vec<std::ops::Range<Duration>> {
        let origin = self.origin();
        self.0.buffered.with(|b| b.iter().map(|r| r.start + origin..r.end + origin).collect())
    }

    /// The work's time at the stream's time 0: the [timeline](Self::set_timeline)'s origin, or
    /// 0. Tracked.
    fn origin(&self) -> Duration {
        self.0.timeline.with(|t| t.map(|t| t.origin)).unwrap_or_default()
    }

    /// Where the stream sits in a longer work, if set. Tracked.
    pub fn timeline(&self) -> Option<Timeline> {
        self.0.timeline.get()
    }

    /// Puts the stream on `timeline` (`None`: the stream is the whole media), for a stream
    /// that holds part of a longer work, such as a film a media server converts from a resume
    /// or skip point: [`position`](Self::position), [`duration`](Self::duration),
    /// [`seek`](Self::seek), [`seek_by`](Self::seek_by), [`buffered`](Self::buffered), the
    /// caption timing of tracks the app adds, and the widget's seek bar and time are then in
    /// the work's time (stream time plus the origin), and the bar spans the work's length. A
    /// seek outside what the stream holds goes to [`on_seek_outside`](Self::on_seek_outside).
    /// A remote device's positions are taken as the work's already.
    pub fn set_timeline(&self, timeline: Option<Timeline>) {
        self.0.timeline.set_if_changed(timeline);
    }

    /// Calls `f` with a seek's target (in the work's time, whole seconds) when it lies outside
    /// what the stream can play now: before its start, or past what it holds (within 2
    /// seconds of the end of what has been converted so far, where the next segment may not
    /// exist yet). The player pauses and shows it is skipping ([`skipping`](Self::skipping)),
    /// the seek bar disabled, while presses of ±10 add up for later; the app makes a stream
    /// from there and hands it over with [`replace_source`](Self::replace_source), or gives up
    /// with [`cancel_skip`](Self::cancel_skip). Only on a [timeline](Self::set_timeline);
    /// with no app listening, such seeks are clamped to the stream as before.
    pub fn on_seek_outside(&self, f: impl Fn(Duration) + 'static) {
        self.0.seek_outside_handlers.borrow_mut().push(Rc::new(f));
    }

    /// The target of a seek handed to the app ([`on_seek_outside`](Self::on_seek_outside))
    /// while it makes a new stream. Tracked.
    pub fn skipping(&self) -> Option<Duration> {
        self.0.skipping.get()
    }

    /// Stops [skipping](Self::skipping) (the app could not make the new stream), dropping
    /// presses made meanwhile; the current stream stays where it was and fetches again,
    /// paused after a skip, playing again after a [hold](Self::hold_for_source) (as Portal's
    /// page goes back to the old stream when a quality or sound switch fails).
    pub fn cancel_skip(&self) {
        let held = self.0.holding.get_untracked().unwrap_or(false);
        self.0.queued_skip.set(None);
        self.0.end_cancel();
        remedy_ui_reactive::batch(|| {
            self.0.holding.set_if_changed(false);
            self.0.skipping.set_if_changed(None);
        });
        self.0.sync_suspend();
        if held && !self.is_remote_untracked() {
            self.play();
        }
    }

    /// Waits for a new source of the same place, from `at` (the work's time): a quality or
    /// sound track switch, where the app's server replaces the stream. As
    /// [skipping](Self::skipping) without a seek: paused, the current stream no longer
    /// fetched (its last picture stays), the seek bar disabled, presses of ±10 added up from
    /// `at`, and "Preparing the video." over the picture. Ended by
    /// [`replace_source`](Self::replace_source) (settings kept, presses run) or
    /// [`cancel_skip`](Self::cancel_skip).
    pub fn hold_for_source(&self, at: Duration) {
        self.0.begin_skip(at, true);
    }

    /// Whether the skip is a [hold](Self::hold_for_source). Tracked.
    pub(crate) fn holding(&self) -> bool {
        self.0.holding.get()
    }

    /// Offers a Cancel button (or takes it away) while the app works on
    /// [starting the next](Self::starting_next) or a [skip](Self::skipping), when that work
    /// can be stopped now: as Portal's page does while the add-on, which says whether it may
    /// be stopped, finds sources. It shows under the spinner in the middle of the picture,
    /// in the tab order; pressing it calls [`on_cancel`](Self::on_cancel), disables it and
    /// says "Canceling." until the app ends the work ([`cancel_next`](Self::cancel_next),
    /// [`next_failed`](Self::next_failed), [`cancel_skip`](Self::cancel_skip) or a new
    /// source). Each new start or skip begins without one.
    pub fn offer_cancel(&self, on: bool) {
        self.0.cancel_offered.set_if_changed(on);
    }

    /// Calls `f` when Cancel ([`offer_cancel`](Self::offer_cancel)) is pressed: the app stops
    /// its server's work (Portal answers the add-on's prompt with cancel) and then ends it
    /// here. [`starting_next`](Self::starting_next) and [`skipping`](Self::skipping) say
    /// which work it was.
    pub fn on_cancel(&self, f: impl Fn() + 'static) {
        self.0.cancel_handlers.borrow_mut().push(Rc::new(f));
    }

    /// Whether Cancel shows: offered, while starting the next or skipping here. Tracked.
    pub(crate) fn cancel_shown(&self) -> bool {
        self.0.cancel_offered.get()
            && !self.is_remote()
            && (self.0.starting.with(Option::is_some) || self.0.skipping.with(Option::is_some))
    }

    /// Whether Cancel was pressed and the app is stopping its work. Tracked.
    pub(crate) fn canceling(&self) -> bool {
        self.0.canceling.get()
    }

    /// Cancel was pressed.
    pub(crate) fn press_cancel(&self) {
        if self.0.canceling.get_untracked().unwrap_or(false) {
            return;
        }
        self.0.canceling.set(true);
        let handlers = self.0.cancel_handlers.borrow().clone();
        for h in handlers {
            h();
        }
    }

    /// Plays `source` in place of the current stream, on `timeline`: a stream of the same
    /// work made from elsewhere in it, after a [seek outside](Self::on_seek_outside). Volume,
    /// speed, the caption choice and delay, tracks the app added (in the work's time) and
    /// remote mode stay; it plays once open, unless remote. Presses made while
    /// [skipping](Self::skipping) then run from its target.
    pub fn replace_source(&self, source: MediaSource, timeline: Timeline) {
        let queued = self.0.queued_skip.take();
        let target = self.0.skipping.get_untracked().ok().flatten();
        self.0.load(source, Some(timeline), true, false);
        if let (Some(q), Some(t)) = (queued, target) {
            self.seek(Duration::try_from_secs_f64((t.as_secs_f64() + q).max(0.0)).unwrap_or_default());
        }
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
        // The device plays at its own speed; the paused local media is not the one heard.
        if self.control().is_some() {
            return;
        }
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
                self.0.anchor_remote();
            }
            // The next is starting: nothing here to play.
            None if self.0.is_starting() => {}
            None => {
                self.0.next_error.set_if_changed(None);
                self.0.send(Cmd::Play);
            }
        }
    }

    /// Pauses. In remote mode, tells the device.
    pub fn pause(&self) {
        match self.control() {
            Some(c) => {
                c.pause();
                self.0.remote_status.update(|s| s.state = PlaybackState::Paused);
                self.0.anchor_remote();
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

    /// Moves to `to` (clamped to the media's length, unless that may still grow: then it
    /// waits for the media to get there). Does nothing in media that cannot seek.
    /// In remote mode, tells the device.
    ///
    /// On a [timeline](Self::set_timeline), `to` is in the work's time, taken in whole
    /// seconds within its length: inside what the stream can play now it moves there in
    /// place; outside, it goes to [`on_seek_outside`](Self::on_seek_outside). While
    /// [skipping](Self::skipping) or [starting the next](Self::starting_next) it does nothing.
    pub fn seek(&self, to: Duration) {
        let timeline = self.0.timeline.get_untracked().ok().flatten();
        let length = remedy_ui_reactive::untrack(|| self.duration());
        let to = if timeline.is_some() { clamp_skip(to, length) } else { to };
        if let Some(c) = self.control() {
            let to = length.map_or(to, |d| to.min(d));
            c.seek(to);
            self.0.remote_status.update(|s| s.position = to);
            self.0.remote_seek.set(Some((to, remedy_ui_core::time::Instant::now())));
            self.0.anchor_remote();
            return;
        }
        if self.0.is_starting() {
            return;
        }
        let Some(timeline) = timeline else { return self.seek_stream(to) };
        if self.0.skipping.get_untracked().ok().flatten().is_some() {
            return;
        }
        let info = self.info_untracked();
        let local = to.checked_sub(timeline.origin);
        let end = info.as_ref().filter(|i| i.seekable).and_then(|i| i.duration);
        match (local, end) {
            (Some(l), Some(end)) if l + STREAM_EDGE <= end => self.seek_stream(l),
            _ if self.0.seek_outside_handlers.borrow().is_empty() => {
                self.seek_stream(local.unwrap_or_default())
            }
            _ => {
                self.0.begin_skip(to, false);
                let handlers = self.0.seek_outside_handlers.borrow().clone();
                for h in handlers {
                    h(to);
                }
            }
        }
    }

    /// Moves the stream to `to`, its own time.
    fn seek_stream(&self, to: Duration) {
        let info = self.info_untracked();
        let to = match info.as_ref().and_then(|i| i.duration) {
            Some(d) if !info.as_ref().is_some_and(|i| i.growing) => to.min(d),
            _ => to,
        };
        if info.is_some_and(|i| i.seekable) {
            self.0.position.set(to);
            self.0.send(Cmd::Seek(to));
        }
    }

    /// Moves `seconds` forward (or back, when negative) from the current position.
    /// While [skipping](Self::skipping), the press is kept, to run from where the skip lands.
    pub fn seek_by(&self, seconds: f64) {
        if self.0.skipping.get_untracked().ok().flatten().is_some() && !self.is_remote_untracked() {
            self.0.queued_skip.set(Some(self.0.queued_skip.get().unwrap_or(0.0) + seconds));
            return;
        }
        let now = remedy_ui_reactive::untrack(|| self.position());
        let to = (now.as_secs_f64() + seconds).max(0.0);
        self.seek(Duration::try_from_secs_f64(to).unwrap_or_default());
    }

    fn is_remote_untracked(&self) -> bool {
        self.0.remote.with_untracked(Option::is_some).unwrap_or(false)
    }

    /// What it is doing (in remote mode, the device). Tracked.
    pub fn state(&self) -> PlaybackState {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.state),
            false => self.0.state.get(),
        }
    }

    /// Where it is in the media (in remote mode, the device, moved on by the time since its
    /// last report while it plays, for at most 15 seconds). On a
    /// [timeline](Self::set_timeline), in the work's time. Tracked: changes as it plays
    /// (with each picture shown, and ten times a second for sound alone).
    pub fn position(&self) -> Duration {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.position),
            false => self.0.position.get() + self.origin(),
        }
    }

    /// How long the media lasts, once known (in remote mode, as the device says); `None`
    /// for live streams. On a [timeline](Self::set_timeline), the work's length when given
    /// (remote or not), else the stream's plus the origin. Tracked.
    pub fn duration(&self) -> Option<Duration> {
        if let Some(l) = self.0.timeline.with(|t| t.and_then(|t| t.length)) {
            return Some(l);
        }
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.length),
            false => self.0.info.with(|i| i.as_ref().and_then(|i| i.duration)).map(|d| d + self.origin()),
        }
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

    /// The volume, 0 to 1 (in remote mode, the device's; the local one until the device
    /// reports it, see [`volume_known`](Self::volume_known)). Tracked.
    pub fn volume(&self) -> f32 {
        let local = self.0.volume.get();
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.volume).unwrap_or(local),
            false => local,
        }
    }

    /// Whether [`volume`](Self::volume) is known: false in remote mode until the device
    /// reports its volume ([`RemoteStatus::volume`]), while the widget's volume slider and
    /// keys are disabled. Tracked.
    pub fn volume_known(&self) -> bool {
        !self.is_remote() || self.0.remote_status.with(|s| s.volume.is_some())
    }

    /// Where the widget's seek bar is held while it is pressed or dragged (on a
    /// [timeline](Self::set_timeline), in the work's time): the bar's thumb and the time show
    /// it, and the seek happens once, on release; `None` otherwise. Tracked.
    pub fn scrubbing(&self) -> Option<Duration> {
        self.0.scrubbing.get()
    }

    pub(crate) fn set_scrubbing(&self, at: Option<Duration>) {
        self.0.scrubbing.set_if_changed(at);
    }

    /// Sets the volume, 0 to 1. In remote mode, the device's: shown at once, sent once it
    /// has rested for 300 ms (as Portal's page does), so a dragged slider sends where it stops.
    pub fn set_volume(&self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        if self.control().is_some() {
            // The slider follows the device: what it already says is not sent back.
            if self.0.remote_status.with_untracked(|s| s.volume).ok() != Some(Some(v)) {
                self.0.remote_status.update(|s| s.volume = Some(v));
                self.0.send_remote_volume_soon(v);
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
    /// by the [caption delay](Self::caption_delay)). On a [timeline](Self::set_timeline),
    /// tracks the media carries are read in the stream's time and the rest in the work's.
    /// Tracked.
    pub fn caption_text(&self) -> Option<String> {
        let sel = self.0.caption.get()?;
        let at = self.0.position.get();
        let at = if self.0.embedded.borrow().contains(&sel) { at } else { at + self.origin() };
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

    /// Whether the "Up next" card is due before the end ([`PlayerOptions::next_up_lead`]):
    /// something is next, media over a minute long, within the lead of its end. Tracked.
    pub(crate) fn next_up_near(&self) -> bool {
        let Some(lead) = self.0.options.next_up_lead else { return false };
        if self.0.queue.with(Vec::is_empty) {
            return false;
        }
        let Some(len) = self.duration().filter(|d| *d > Duration::from_secs(60)) else { return false };
        let left = len.saturating_sub(self.position());
        !left.is_zero() && left <= lead
    }

    /// Whether the app asked for the card before the end, as Portal's page does: then, while
    /// remote, no card shows at all (the device moves on by itself).
    pub(crate) fn next_up_early(&self) -> bool {
        self.0.options.next_up_lead.is_some()
    }

    /// Whether an app listens for theater requests. Tracked.
    pub(crate) fn theater_offered(&self) -> bool {
        self.0.theater_offered.get()
    }

    /// Calls `f` with the cast button when it is pressed: the app opens its menu of devices
    /// anchored to it (with [`menu`](remedy_ui_widgets_core::menu)), and on a choice puts the
    /// player in [remote mode](Self::set_remote). The widget shows the button only once an app
    /// listens. Opened with [`menu_press_through`](remedy_ui_widgets_core::menu_press_through),
    /// a press on the picture that closes the menu also plays or pauses, as in a web page's
    /// player.
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
    /// (`None`). While remote, the local media pauses and keeps its place, no longer fetched
    /// (as Portal's page stops loading the stream while casting) until it leaves; the picture
    /// gives way to a casting stage ("Playing on The Ris"); and play, pause, seeking, volume, mute,
    /// captions and next (every control and key of the widget, and these methods) act on the
    /// device through [`Remote::control`], while [`state`](Self::state),
    /// [`position`](Self::position), [`duration`](Self::duration), [`volume`](Self::volume)
    /// and [`is_muted`](Self::is_muted) say what the app last told
    /// [`set_remote_status`](Self::set_remote_status). Leaving, the app picks up where the
    /// device was: it reads [`position`](Self::position) first, then [`seek`](Self::seek)s and
    /// [`play`](Self::play)s here.
    pub fn set_remote(&self, remote: Option<Remote>) {
        // A volume for the device that was is not sent to the next.
        if let (Some(t), Some(ui)) = (self.0.remote_volume_timer.take(), self.0.ui.upgrade()) {
            ui.cancel_timer(t);
        }
        if remote.is_some() {
            self.0.send(Cmd::Pause);
            // Connecting, its volume unknown, until the app hears from the device.
            let muted = self.0.muted.get_untracked();
            self.0
                .remote_status
                .set(RemoteStatus { muted: muted.unwrap_or(false), ..RemoteStatus::default() });
        }
        self.0.remote_seek.set(None);
        self.0.remote_seen.set(None);
        self.0.remote.set(remote);
        self.0.sync_suspend();
        self.0.schedule_remote_tick();
    }

    /// Shows `poster` on the casting stage in place of the remote's (the device moved on to
    /// other media), staying in remote mode. Nothing outside it.
    pub fn set_remote_poster(&self, poster: Option<El>) {
        if self.is_remote_untracked() {
            self.0.remote.update(|r| {
                if let Some(r) = r {
                    r.poster = poster;
                }
            });
        }
    }

    /// The device's name, in remote mode. Tracked.
    pub fn remote_name(&self) -> Option<String> {
        self.0.remote.with(|r| r.as_ref().map(|r| r.name.clone()))
    }

    /// What the device is doing, as the app hears it (polled from its server, or pushed): the
    /// seek bar, the time, the volume and the casting stage follow it.
    ///
    /// After a seek, the shown position stays at the seek's target until the device reports a
    /// position within 4 seconds of it while playing or paused, or for at most 8 seconds: a
    /// poll answered before the device moved does not pull the seek bar back.
    ///
    /// Between reports, a playing device's shown position moves on by the time since the last
    /// one, for at most 15 seconds and never past the length (as Portal's page does); a report
    /// older than when it is handed over is best moved on by its age first.
    pub fn set_remote_status(&self, mut status: RemoteStatus) {
        if let Some((to, at)) = self.0.remote_seek.get() {
            let landed = matches!(status.state, PlaybackState::Playing | PlaybackState::Paused)
                && status.position.abs_diff(to) < SEEK_LANDED;
            if landed || at.elapsed() > SEEK_LAND {
                self.0.remote_seek.set(None);
            } else {
                status.position = to;
            }
        }
        self.0.remote_status.set_if_changed(status);
        self.0.anchor_remote();
    }

    /// A handle that hands [`RemoteStatus`] news to this player from any thread.
    pub fn remote_status_sender(&self) -> RemoteStatusSender {
        RemoteStatusSender { id: self.0.id, waker: remedy_ui_reactive::waker().ok() }
    }

    /// The player's number on its UI thread (what a post from another thread carries).
    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }

    /// The player numbered `id` on this thread, while it lives.
    pub(crate) fn by_id(id: u64) -> Option<Player> {
        PLAYERS.with(|p| p.borrow().get(&id).and_then(Weak::upgrade)).map(Player)
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
    /// with the next. A next made [later](NextUp::later) is asked of the app instead
    /// ([`on_next_needed`](Self::on_next_needed)).
    /// In remote mode, the device is told ([`RemoteControl::next`]) and the app decides.
    pub fn play_next(&self) {
        match self.control() {
            Some(c) => c.next(),
            None => self.0.advance(),
        }
    }

    /// Calls `f` with next up when it is time to play a next made [later](NextUp::later): at
    /// the end once the countdown is up (at once with a zero
    /// [`next_up_delay`](PlayerOptions::next_up_delay)), or on Play now, Shift+N (N) or
    /// [`play_next`](Self::play_next); never in remote mode, where the device moves on by
    /// itself. The current media pauses and the player shows it is
    /// [starting the next](Self::starting_next) ("Starting Episode 2" over the picture,
    /// announced; the seek bar disabled; play and seeks do nothing) while the app makes it,
    /// then hands it over with [`start_next`](Self::start_next), or says why not with
    /// [`next_failed`](Self::next_failed), or gives up with
    /// [`cancel_next`](Self::cancel_next). With no app listening, such a next never plays.
    pub fn on_next_needed(&self, f: impl Fn(&NextUp) + 'static) {
        self.0.next_needed_handlers.borrow_mut().push(Rc::new(f));
    }

    /// The title of the next while the app makes it ([`on_next_needed`](Self::on_next_needed));
    /// in remote mode, while the device starts it by itself ([`RemoteStatus::next`]).
    /// Tracked.
    pub fn starting_next(&self) -> Option<String> {
        match self.is_remote() {
            true => self.0.remote_status.with(|s| s.next.clone()),
            false => self.0.starting.get(),
        }
    }

    /// What the app is doing to start the next, shown and announced under "Starting …"
    /// ("Asking the add-on to play this."); `None` clears it. Only while
    /// [starting the next](Self::starting_next).
    pub fn next_status(&self, detail: Option<String>) {
        if self.0.is_starting() {
            self.0.starting_detail.set_if_changed(detail);
        }
    }

    /// What the app says it is doing to start the next. Tracked.
    pub(crate) fn starting_detail(&self) -> Option<String> {
        self.0.starting_detail.get()
    }

    /// What the app says went wrong making the next ([`next_failed`](Self::next_failed)),
    /// until play is pressed or other media opens. Tracked.
    pub fn next_error(&self) -> Option<String> {
        self.0.next_error.get()
    }

    /// Plays `source` (on `timeline`, if a stream of a longer work) as the next: next up comes
    /// off the queue if it was made [later](NextUp::later), and it plays once open (unless
    /// remote). Volume, speed, the choice of an embedded caption track (by its order) and the
    /// caption delay carry over; tracks the app added belonged to the old media and go.
    pub fn start_next(&self, source: MediaSource, timeline: Option<Timeline>) {
        if self.0.queue.with_untracked(|q| q.first().is_some_and(|n| n.source.is_none())).unwrap_or(false) {
            self.0.queue.update(|q| {
                q.remove(0);
            });
        }
        self.0.load(source, timeline, false, true);
    }

    /// The app could not make the next: `message` shows over the picture and is announced;
    /// the current media stays (paused, or ended), next up stays queued for Play now to try
    /// again, and the end does not ask again by itself.
    pub fn next_failed(&self, message: impl Into<String>) {
        self.0.declined.set(true);
        remedy_ui_reactive::batch(|| {
            self.0.end_cancel();
            self.0.starting.set_if_changed(None);
            self.0.starting_detail.set_if_changed(None);
            self.0.next_error.set(Some(message.into()));
        });
    }

    /// Stops starting the next (the person or the app gave up): the current media stays,
    /// paused or ended, next up stays queued, and the end does not ask again by itself.
    pub fn cancel_next(&self) {
        self.0.declined.set(true);
        remedy_ui_reactive::batch(|| {
            self.0.end_cancel();
            self.0.starting.set_if_changed(None);
            self.0.starting_detail.set_if_changed(None);
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playing_device_moves_on_until_its_report_is_stale_and_never_past_the_end() {
        let s = Duration::from_secs;
        assert_eq!(moved_on(s(100), s(3), None), s(103));
        assert_eq!(moved_on(s(100), s(60), None), s(115), "15 s at most");
        assert_eq!(moved_on(s(100), s(3), Some(s(102))), s(102), "not past the end");
    }

    #[test]
    fn skips_are_whole_seconds_within_the_work() {
        let ms = Duration::from_millis;
        assert_eq!(clamp_skip(ms(396_700), None), ms(396_000));
        assert_eq!(clamp_skip(ms(20_000_000), Some(ms(10_892_500))), ms(10_892_000));
    }
}
