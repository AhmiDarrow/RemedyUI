//! The UI thread's handle on a player: commands out, state in as signals.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use remedy_ui_media_video::VideoSink;
use remedy_ui_reactive::{Memo, Signal};
use remedy_ui_view::{El, Ui};

use crate::engine::{self, Cmd, PlaybackState, PlaybackStats, Shared, lock};
use crate::host::{MediaError, MediaHost, MediaInfo, MediaSource};
use crate::subtitles::{SubtitleFormat, SubtitleTrack, cue_reach, parse_subtitles, read_subtitle_file};

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
    /// The widget shows Back 10 seconds and Forward 10 seconds buttons after Play, as Portal's.
    pub skip_buttons: bool,
    /// The widget shows the speed button (Portal's page has none); on by default.
    pub speed_button: bool,
    /// The widget's bar in two rows, as Portal's: the seek bar above; below, Play (and the
    /// skip buttons), mute and volume and the time at the start, the rest at the end.
    pub two_rows: bool,
    /// Starts in the compact layout ([`Player::set_compact`]).
    pub compact: bool,
    /// Keeps the screen on while video plays here ([`Player::set_keep_awake`]); off by
    /// default.
    pub keep_awake: bool,
}

/// What the person chose in the widget's captions menu ([`Player::caption_pick`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CaptionPick {
    /// No captions.
    #[default]
    Off,
    /// The app picks (Portal's Auto: the best match in the person's languages). Offered once
    /// the app listens ([`Player::on_caption_pick`]); it shows its pick with
    /// [`Player::set_caption_track`].
    Auto,
    /// One of the player's [tracks](Player::subtitle_tracks): the media's own and the app's.
    Track(usize),
    /// One of the app's [choices](Player::set_caption_choices) not loaded yet (one found
    /// online), by its id: the app fetches it ([`Player::set_captions_loading`]), adds it and
    /// shows it.
    Choice(String),
}

/// A captions choice the app offers before it has the track (Portal's online subtitles):
/// `CaptionChoice::new("os:42", "English · 720p.BluRay").detail("Hearing impaired · Best match")`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CaptionChoice {
    /// The app's id for it, given back in [`CaptionPick::Choice`].
    pub id: String,
    /// Its name in the menu.
    pub label: String,
    /// What else to know, in words ("Hearing impaired · Best match"), after the name.
    pub detail: Option<String>,
}

impl CaptionChoice {
    /// `id`, named `label`.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        CaptionChoice { id: id.into(), label: label.into(), detail: None }
    }

    /// Sets `detail`.
    pub fn detail(mut self, value: impl Into<String>) -> Self {
        self.detail = Some(value.into());
        self
    }
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

/// What the app is doing, said over the picture ([`Player::set_status`]): Portal's "Asking
/// the add-on to play this.", "Preparing the video." while it switches the quality.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StageStatus {
    /// The words, shown and announced politely.
    pub text: String,
    /// The app is working: a spinner shows, the seek bar is disabled, and play and seeks do
    /// nothing until it is done. Not busy, the words alone show (a message).
    pub busy: bool,
}

impl StageStatus {
    /// `text`, not busy: `StageStatus::new("Preparing the video.").busy(true)`.
    pub fn new(text: impl Into<String>) -> Self {
        StageStatus { text: text.into(), busy: false }
    }

    /// Sets `busy`.
    pub fn busy(mut self, value: bool) -> Self {
        self.busy = value;
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
    /// When the device made this report (`None`: just now). A playing device's position is
    /// moved on from then, so a report that took a while to arrive shows where the device is
    /// by now.
    pub reported_at: Option<remedy_ui_core::time::Instant>,
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

    /// Sets `reported_at`: when the device made the report.
    pub fn reported_at(mut self, at: remedy_ui_core::time::Instant) -> Self {
        self.reported_at = Some(at);
        self
    }

    /// Sets `reported_at` to `age` ago: the report is that old (a server's poll answer that
    /// says how long ago the TV last reported).
    pub fn age(self, age: Duration) -> Self {
        let now = remedy_ui_core::time::Instant::now();
        self.reported_at(now.checked_sub(age).unwrap_or(now))
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
            reported_at: None,
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

    /// Sets `keep_awake`: the screen stays on while video plays here.
    pub fn keep_awake(mut self, value: bool) -> Self {
        self.keep_awake = value;
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

    /// Sets `skip_buttons`: Back and Forward 10 seconds buttons.
    pub fn skip_buttons(mut self, value: bool) -> Self {
        self.skip_buttons = value;
        self
    }

    /// Sets `speed_button`: whether the speed button shows.
    pub fn speed_button(mut self, value: bool) -> Self {
        self.speed_button = value;
        self
    }

    /// Sets `two_rows`: the bar in two rows, the seek bar above.
    pub fn two_rows(mut self, value: bool) -> Self {
        self.two_rows = value;
        self
    }

    /// Sets `compact`: starts in the compact layout.
    pub fn compact(mut self, value: bool) -> Self {
        self.compact = value;
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
            skip_buttons: false,
            speed_button: true,
            two_rows: false,
            compact: false,
            keep_awake: false,
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
/// Hears Play pressed after a failure.
type RetryHandler = Rc<dyn Fn(Duration)>;
/// Hears a captions pick.
type PickHandler = Rc<dyn Fn(&CaptionPick)>;

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
    /// Each track's `cue_reach`, so the caption showing is found by binary search.
    cue_reach: Memo<Vec<Vec<Duration>>>,
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
    /// What the app says it is doing, over the picture.
    status: Signal<Option<StageStatus>>,
    /// Why the app stopped playback ([`Player::fail`]), until a new source.
    app_failure: Signal<Option<String>>,
    retry_handlers: RefCell<Vec<RetryHandler>>,
    /// What the engine was told, to tell the next one: the picture's sink and whether it is
    /// seen.
    sink: RefCell<Option<VideoSink>>,
    visible: Cell<bool>,
    /// Each [`player_picture`](crate::player_picture): its number, sink and whether it is
    /// seen; and which one holds the sink.
    pictures: RefCell<Vec<(u64, VideoSink, bool)>>,
    holder: Cell<Option<u64>>,
    caption_pick: Signal<CaptionPick>,
    /// The last pick that showed captions, for C to come back to.
    last_pick: RefCell<Option<CaptionPick>>,
    /// The name of the track picked in the last work, to pick again in this one.
    want_label: RefCell<Option<String>>,
    pick_handlers: RefCell<Vec<PickHandler>>,
    pick_offered: Signal<bool>,
    caption_choices: Signal<Vec<CaptionChoice>>,
    caption_note: Signal<Option<String>>,
    captions_loading: Signal<bool>,
    compact: Signal<bool>,
    /// What plays, shown at the top of the picture in full screen ([`Player::set_title`]).
    title: Signal<Option<String>>,
    /// In full screen, the controls are showing (not faded after a rest).
    controls_awake: Signal<bool>,
    /// Theater was on when the player went compact, so leaving it brings it back (Portal's).
    theater_before_compact: Cell<bool>,
    /// [`PlayerOptions::next_up_delay`] and [`next_up_lead`](PlayerOptions::next_up_lead),
    /// changed while it plays ([`Player::set_next_up_delay`]).
    next_up_delay: Signal<Option<Duration>>,
    next_up_lead: Signal<Option<Duration>>,
    /// Keeps the screen on while video plays here ([`Player::set_keep_awake`]).
    keep_awake: Cell<bool>,
    /// What holds the screen on, and the hold while it does.
    awake_hold: RefCell<Option<AwakeHold>>,
    awake: RefCell<Option<Box<dyn std::any::Any>>>,
}

/// Holds the screen on until what it returns is dropped ([`Player::set_awake_hold`]).
type AwakeHold = Rc<dyn Fn() -> Option<Box<dyn std::any::Any>>>;

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
        self.sync_awake();
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
        let Some(delay) = self.next_up_delay.get_untracked().ok().flatten() else { return };
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

    /// Whether the screen should be held on now: asked for, video playing here and seen.
    fn wants_awake(&self) -> bool {
        self.keep_awake.get()
            && self.visible.get()
            && self.state.get_untracked().ok() == Some(PlaybackState::Playing)
            && self.remote.with_untracked(Option::is_none).unwrap_or(false)
            && self.info.with_untracked(|i| i.as_ref().is_some_and(|i| i.video.is_some())).unwrap_or(false)
    }

    /// Takes or lets go of the screen hold as [`wants_awake`](Self::wants_awake) says.
    fn sync_awake(&self) {
        let want = self.wants_awake();
        if want == self.awake.borrow().is_some() {
            return;
        }
        let hold = match want {
            true => {
                let f = self.awake_hold.borrow().clone();
                match f {
                    Some(f) => f(),
                    None => {
                        let reason =
                            self.ui.upgrade().map(|ui| crate::STRINGS.get(&ui, "playback-keep-awake"));
                        crate::awake::platform_hold(reason.as_deref().unwrap_or("Playing video"))
                    }
                }
            }
            false => None,
        };
        // A hold the platform refused leaves `awake` empty: asked again at the next change.
        let old = std::mem::replace(&mut *self.awake.borrow_mut(), hold);
        drop(old);
    }

    /// Where a remote device's position stops: the work's length, else the device's.
    fn remote_end(&self) -> Option<Duration> {
        let end = self.timeline.with_untracked(|t| t.and_then(|t| t.length)).ok().flatten();
        end.or_else(|| self.remote_status.with_untracked(|s| s.length).ok().flatten())
    }

    /// [`remote_end`](Self::remote_end) for a report not yet shown.
    fn remote_end_for(&self, status: &RemoteStatus) -> Option<Duration> {
        let end = self.timeline.with_untracked(|t| t.and_then(|t| t.length)).ok().flatten();
        end.or(status.length)
    }

    fn is_starting(&self) -> bool {
        self.starting.with_untracked(Option::is_some).unwrap_or(false)
    }

    fn busy_untracked(&self) -> bool {
        self.status.with_untracked(|s| s.as_ref().is_some_and(|s| s.busy)).unwrap_or(false)
    }

    fn failed_untracked(&self) -> bool {
        self.app_failure.with_untracked(Option::is_some).unwrap_or(false)
            || self.state.get_untracked().ok() == Some(PlaybackState::Failed)
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
                    let to = moved_on(at, seen.elapsed(), p.remote_end());
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
        // The pick carries over: a track by its name, since the next media numbers its own.
        let pick = self.caption_pick.get_untracked().unwrap_or_default();
        let picked_label = match &pick {
            CaptionPick::Track(i) => {
                self.tracks.with_untracked(|t| t.get(*i).map(|t| t.label.clone())).ok().flatten()
            }
            _ => None,
        };
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
            // As Portal's page attaching a stream: its words go.
            self.status.set_if_changed(None);
            self.app_failure.set_if_changed(None);
            self.end_cancel();
        });
        self.embedded.borrow_mut().clear();
        self.sync_awake();
        self.carry_pick(pick, picked_label, !same_work);
        self.play_on_open.set(self.remote.with_untracked(Option::is_none).unwrap_or(true));
        let muted = self.muted.get_untracked().unwrap_or(false);
        self.send(Cmd::Volume(if muted { 0.0 } else { self.volume.get_untracked().unwrap_or(1.0) }));
        self.send(Cmd::PauseUnseen(self.options.unseen == UnseenPolicy::PauseVideo));
        self.send(Cmd::Sink(self.sink.borrow().clone()));
        self.send(Cmd::Visible(self.visible.get()));
        self.send(Cmd::Speed(self.speed.get_untracked().unwrap_or(1.0)));
        self.sync_suspend();
    }

    /// After new media: a picked track still shown keeps its pick; one gone is picked again
    /// by `label` once a track of that name comes; in a new work an app's choice becomes Auto
    /// (or Off without an app), and Auto is asked of the app again.
    fn carry_pick(&self, pick: CaptionPick, label: Option<String>, new_work: bool) {
        let shown = self.caption.get_untracked().ok().flatten();
        let next = match pick {
            CaptionPick::Track(_) => match shown {
                Some(t) => CaptionPick::Track(t),
                None => {
                    *self.want_label.borrow_mut() = label;
                    *self.last_pick.borrow_mut() = None;
                    CaptionPick::Off
                }
            },
            CaptionPick::Choice(_) if new_work => match self.pick_offered.get_untracked().unwrap_or(false) {
                true => CaptionPick::Auto,
                false => CaptionPick::Off,
            },
            p => p,
        };
        self.captions_loading.set_if_changed(false);
        self.caption_pick.set_if_changed(next.clone());
        if new_work && next == CaptionPick::Auto {
            let handlers = self.pick_handlers.borrow().clone();
            for h in handlers {
                h(&next);
            }
        }
    }

    /// A track named as the one picked in the last media came: it is picked again.
    fn repick(&self, tracks: &[SubtitleTrack], from: usize) -> Option<usize> {
        let want = self.want_label.borrow().clone()?;
        let at = tracks.iter().enumerate().skip(from).find(|(_, t)| t.label == want)?.0;
        self.want_label.borrow_mut().take();
        Some(at)
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
            let from = map.first().copied().unwrap_or(0);
            *self.embedded.borrow_mut() = map;
            let tracks = self.tracks.get_untracked().unwrap_or_default();
            if let Some(i) = self.repick(&tracks, from) {
                self.caption.set(Some(i));
                self.caption_pick.set_if_changed(CaptionPick::Track(i));
                *self.last_pick.borrow_mut() = Some(CaptionPick::Track(i));
            } else if let Some(i) = self.caption.get_untracked().ok().flatten() {
                // A choice made again by its order is the pick.
                if self.caption_pick.get_untracked().ok() == Some(CaptionPick::Off) {
                    self.caption_pick.set_if_changed(CaptionPick::Track(i));
                }
            }
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
        self.awake.get_mut().take();
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
        let tracks = Signal::new(Vec::<SubtitleTrack>::new());
        let cue_reach = Memo::new(move || tracks.with(|t| t.iter().map(|tr| cue_reach(&tr.cues)).collect()));
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
            tracks,
            cue_reach,
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
            status: Signal::new(None),
            app_failure: Signal::new(None),
            retry_handlers: RefCell::new(Vec::new()),
            sink: RefCell::new(None),
            visible: Cell::new(true),
            pictures: RefCell::new(Vec::new()),
            holder: Cell::new(None),
            caption_pick: Signal::new(CaptionPick::Off),
            last_pick: RefCell::new(None),
            want_label: RefCell::new(None),
            pick_handlers: RefCell::new(Vec::new()),
            pick_offered: Signal::new(false),
            caption_choices: Signal::new(Vec::new()),
            caption_note: Signal::new(None),
            captions_loading: Signal::new(false),
            compact: Signal::new(options.compact),
            title: Signal::new(None),
            controls_awake: Signal::new(true),
            theater_before_compact: Cell::new(false),
            next_up_delay: Signal::new(options.next_up_delay),
            next_up_lead: Signal::new(options.next_up_lead),
            keep_awake: Cell::new(options.keep_awake),
            awake_hold: RefCell::new(None),
            awake: RefCell::new(None),
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
    /// [starting the next](Self::starting_next), a [skip](Self::skipping) or what a
    /// [busy status](Self::set_status) says, when that work
    /// can be stopped now: as Portal's page does while the add-on, which says whether it may
    /// be stopped, finds sources. It shows under the spinner in the middle of the picture,
    /// in the tab order; pressing it calls [`on_cancel`](Self::on_cancel), disables it and
    /// says "Canceling." until the app ends the work ([`cancel_next`](Self::cancel_next),
    /// [`next_failed`](Self::next_failed), [`cancel_skip`](Self::cancel_skip),
    /// [`set_status(None)`](Self::set_status), [`fail`](Self::fail) or a new source). Each new
    /// start or skip begins without one.
    pub fn offer_cancel(&self, on: bool) {
        self.0.cancel_offered.set_if_changed(on);
    }

    /// Calls `f` when Cancel ([`offer_cancel`](Self::offer_cancel)) is pressed: the app stops
    /// its server's work (Portal answers the add-on's prompt with cancel) and then ends it
    /// here. [`starting_next`](Self::starting_next), [`skipping`](Self::skipping) and
    /// [`status`](Self::status) say which work it was.
    pub fn on_cancel(&self, f: impl Fn() + 'static) {
        self.0.cancel_handlers.borrow_mut().push(Rc::new(f));
    }

    /// Whether Cancel shows: offered, while starting the next or skipping here. Tracked.
    pub(crate) fn cancel_shown(&self) -> bool {
        self.0.cancel_offered.get()
            && !self.is_remote()
            && (self.0.starting.with(Option::is_some)
                || self.0.skipping.with(Option::is_some)
                || self.0.status.with(|s| s.as_ref().is_some_and(|s| s.busy)))
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
            // The next is starting, or the app is busy: nothing here to play.
            None if self.0.is_starting() || self.0.busy_untracked() => {}
            None if self.0.failed_untracked() => {
                let at = remedy_ui_reactive::untrack(|| self.position());
                let handlers = self.0.retry_handlers.borrow().clone();
                for h in handlers {
                    h(at);
                }
            }
            None => {
                self.0.next_error.set_if_changed(None);
                // Still opening: it plays once open (the engine takes Play only then).
                if self.0.info.with_untracked(Option::is_none).unwrap_or(true) {
                    self.0.play_on_open.set(true);
                }
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
            None => {
                self.0.play_on_open.set(false);
                self.0.send(Cmd::Pause);
            }
        }
    }

    /// Calls `f` with the last position (in the work's time on a
    /// [timeline](Self::set_timeline)) when play is pressed after playback
    /// [failed](PlaybackState::Failed) here (not remote): the play button, K, Space, or a click
    /// on the picture, as Portal's page asks its server again on Play. The failure then says
    /// "… Press Play to try again." The app answers with
    /// [`replace_source`](Self::replace_source) (or [`start_next`](Self::start_next)), or
    /// [`fail`](Self::fail)s again; meanwhile it may [`set_status`](Self::set_status). With no
    /// app listening, play does nothing after a failure.
    pub fn on_retry(&self, f: impl Fn(Duration) + 'static) {
        self.0.retry_handlers.borrow_mut().push(Rc::new(f));
    }

    /// Whether play after a failure goes to the app ([`on_retry`](Self::on_retry)).
    pub(crate) fn retries(&self) -> bool {
        !self.0.retry_handlers.borrow().is_empty()
    }

    /// Says what the app is doing over the picture (`None` takes it away): Portal's resolve
    /// phases, "Preparing the video." while it switches quality or sound, or a message such
    /// as "The stream did not arrive.". It is announced politely as it changes and shows in
    /// place of the player's own words (a failure, skipping, starting the next). While
    /// [busy](StageStatus::busy) a spinner shows, the seek bar is disabled, play and seeks do
    /// nothing, and Cancel shows when the app [offers it](Self::offer_cancel). Only here, not
    /// in remote mode; a new source takes it away. Taking it away ends an offered Cancel.
    pub fn set_status(&self, status: Option<StageStatus>) {
        if status.is_none() {
            self.0.end_cancel();
        }
        self.0.status.set_if_changed(status);
    }

    /// What the app last said it is doing ([`set_status`](Self::set_status)). Tracked.
    pub fn status(&self) -> Option<StageStatus> {
        self.0.status.get()
    }

    /// Stops playback with the app's words (Portal's "The stream stopped loading.", its
    /// server's reason a stream died): the player stops fetching and is
    /// [failed](PlaybackState::Failed), `message` over the picture (with "Press Play to try
    /// again." when the app [retries](Self::on_retry)) and announced as an alert. Skipping,
    /// starting the next and the app's status end. A new source clears it.
    pub fn fail(&self, message: impl Into<String>) {
        self.0.send(Cmd::Close);
        *self.0.tx.borrow_mut() = None;
        self.0.queued_skip.set(None);
        self.0.play_on_open.set(false);
        remedy_ui_reactive::batch(|| {
            self.0.end_cancel();
            self.0.holding.set_if_changed(false);
            self.0.skipping.set_if_changed(None);
            self.0.starting.set_if_changed(None);
            self.0.starting_detail.set_if_changed(None);
            self.0.status.set_if_changed(None);
            self.0.app_failure.set(Some(message.into()));
        });
    }

    /// The app's words for a failure ([`fail`](Self::fail)). Tracked.
    pub(crate) fn app_failure(&self) -> Option<String> {
        self.0.app_failure.get()
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
        if self.0.is_starting() || self.0.busy_untracked() {
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
            // The app stopped it ([`fail`](Self::fail)).
            false if self.0.app_failure.with(Option::is_some) => PlaybackState::Failed,
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

    /// Adds a caption track; returns its index. On a [timeline](Self::set_timeline) its cues
    /// are in the work's time; [`add_subtitles_stream`](Self::add_subtitles_stream) takes them
    /// in the stream's.
    pub fn add_subtitles(&self, track: SubtitleTrack) -> usize {
        let mut i = 0;
        self.0.tracks.update(|t| {
            i = t.len();
            t.push(track);
        });
        let tracks = self.0.tracks.get_untracked().unwrap_or_default();
        if let Some(at) = self.0.repick(&tracks, i) {
            self.set_caption_pick(CaptionPick::Track(at));
        }
        i
    }

    /// Adds a caption track timed to the stream rather than the work (subtitles a media server
    /// cut to the stream it made from a resume or skip point); returns its index. On a
    /// [timeline](Self::set_timeline) its cues are moved onto the work's time by the origin now,
    /// so they stay right after a [`replace_source`](Self::replace_source) from elsewhere in
    /// the work (another origin) and the app never adds the origin itself. Without a timeline,
    /// as [`add_subtitles`](Self::add_subtitles).
    pub fn add_subtitles_stream(&self, mut track: SubtitleTrack) -> usize {
        let origin = remedy_ui_reactive::untrack(|| self.origin());
        for c in &mut track.cues {
            c.start += origin;
            c.end += origin;
        }
        self.add_subtitles(track)
    }

    /// Reads a subtitle file (WebVTT, SRT, ASS/SSA, by its extension or its contents) and adds
    /// it as a track named `label`.
    pub fn load_subtitles(&self, path: &Path, label: impl Into<String>) -> Result<usize, MediaError> {
        let text = read_subtitle_file(path)?;
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
        // Shown: no longer loading; the menu's pick follows a track set from outside it.
        let pick = self.0.caption_pick.get_untracked().unwrap_or_default();
        match i {
            Some(t) => {
                self.0.captions_loading.set_if_changed(false);
                match pick {
                    CaptionPick::Off | CaptionPick::Track(_) => {
                        self.0.caption_pick.set_if_changed(CaptionPick::Track(t));
                        *self.0.last_pick.borrow_mut() = Some(CaptionPick::Track(t));
                    }
                    // A choice the app loaded: C comes back to its track, not to fetching it.
                    CaptionPick::Choice(_) => *self.0.last_pick.borrow_mut() = Some(CaptionPick::Track(t)),
                    CaptionPick::Auto => {}
                }
            }
            None if matches!(pick, CaptionPick::Track(_)) => {
                self.0.caption_pick.set_if_changed(CaptionPick::Off)
            }
            None => {}
        }
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
        // The memo first: computing it reads the tracks.
        self.0.cue_reach.with(|r| {
            let r = r.get(sel)?;
            self.0.tracks.with(|t| t.get(sel).and_then(|tr| tr.text_at_with(r, at)))
        })
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
        let Some(lead) = self.0.next_up_lead.get() else { return false };
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
        self.0.next_up_lead.with(Option::is_some)
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

    /// Applies the player's keys to `e` under its [`KeyScheme`], as the
    /// [`player`](crate::player) widget does for keys from inside it (K, J, L, M, C, F, T,
    /// Escape, > and <, N; the arrows and Space in [`KeyScheme::Arrows`]; the media keys), in
    /// [remote mode](Self::set_remote) on the device. True when the key was the player's;
    /// keys with Ctrl, Alt or the system key held never are.
    pub fn handle_key(&self, e: &remedy_ui_input::KeyEvent) -> bool {
        crate::widget::apply_key(self, e, true)
    }

    /// Hears the player's keys wherever focus is inside `el` (the window's
    /// [`root`](Ui::root), for the whole window), as Portal's page does: [`handle_key`]
    /// (Self::handle_key) for each key nothing focused took, except that a focused text field
    /// keeps every key, a focused slider its own (the arrows, Home, End, Page Up and Down), a
    /// focused button or other control Space and Enter, and nothing reaches the player while
    /// focus is in a menu or dialog or one is open. Keys inside the player widget reach it
    /// first.
    pub fn keys_from(&self, el: &El) {
        let (p, ui) = (self.clone(), el.ui().clone());
        let _ = el.clone().on_key(move |e| !focus_keeps(&ui, e) && p.handle_key(e));
    }

    /// What the person picked in the widget's captions menu (or C, or the app). Tracked.
    pub fn caption_pick(&self) -> CaptionPick {
        self.0.caption_pick.get()
    }

    /// Picks captions, as the widget's menu does: Off hides them, a track shows it, Auto and
    /// an app's choice clear what shows until the app shows its pick. Then the app hears it
    /// ([`on_caption_pick`](Self::on_caption_pick)). The pick stays from one media to the next:
    /// Off stays off, Auto asks the app again, a track is picked again by its name when the
    /// next media has one.
    pub fn set_caption_pick(&self, pick: CaptionPick) {
        if pick != CaptionPick::Off {
            *self.0.last_pick.borrow_mut() = Some(pick.clone());
        }
        self.0.want_label.borrow_mut().take();
        self.0.captions_loading.set_if_changed(false);
        self.0.caption_pick.set_if_changed(pick.clone());
        let shown = match &pick {
            CaptionPick::Track(i) => Some(*i),
            _ => None,
        };
        self.0.caption.set_if_changed(shown);
        if let Some(c) = self.control() {
            c.set_captions(shown);
        }
        let handlers = self.0.pick_handlers.borrow().clone();
        for h in handlers {
            h(&pick);
        }
    }

    /// Calls `f` with each pick from the widget's captions menu (or C), and with Auto again
    /// for each new media while Auto is picked. Listening offers Auto in the menu.
    pub fn on_caption_pick(&self, f: impl Fn(&CaptionPick) + 'static) {
        self.0.pick_handlers.borrow_mut().push(Rc::new(f));
        self.0.pick_offered.set_if_changed(true);
    }

    /// Whether an app listens for picks (and so offers Auto). Tracked.
    pub(crate) fn auto_offered(&self) -> bool {
        self.0.pick_offered.get()
    }

    /// Sets the choices the app offers before it has their tracks (Portal's online
    /// subtitles), listed in the captions menu after the player's tracks.
    pub fn set_caption_choices(&self, choices: Vec<CaptionChoice>) {
        self.0.caption_choices.set_if_changed(choices);
    }

    /// The app's captions choices. Tracked.
    pub fn caption_choices(&self) -> Vec<CaptionChoice> {
        self.0.caption_choices.get()
    }

    /// A line the captions menu shows under the choices ("Searching online…", "Search
    /// failed: …"), or none.
    pub fn set_caption_note(&self, note: Option<String>) {
        self.0.caption_note.set_if_changed(note);
    }

    /// The captions menu's note. Tracked.
    pub fn caption_note(&self) -> Option<String> {
        self.0.caption_note.get()
    }

    /// Says the picked captions are on their way (fetched or made): the captions button reads
    /// "Loading…" until the app shows them ([`set_caption_track`](Self::set_caption_track)) or
    /// says otherwise.
    pub fn set_captions_loading(&self, on: bool) {
        self.0.captions_loading.set_if_changed(on);
    }

    /// Whether the picked captions are loading. Tracked.
    pub fn captions_loading(&self) -> bool {
        self.0.captions_loading.get()
    }

    /// C: captions off, or back on as last picked (else Auto when offered, else the first
    /// track, else the app's first choice).
    pub(crate) fn toggle_captions(&self) {
        if self.0.caption_pick.get_untracked().unwrap_or_default() != CaptionPick::Off {
            self.set_caption_pick(CaptionPick::Off);
            return;
        }
        let last = self.0.last_pick.borrow().clone();
        let tracks = self.0.tracks.with_untracked(Vec::len).unwrap_or(0);
        let first_choice =
            self.0.caption_choices.with_untracked(|c| c.first().map(|c| c.id.clone())).ok().flatten();
        let pick = match last {
            Some(CaptionPick::Track(i)) if i < tracks => Some(CaptionPick::Track(i)),
            Some(p @ (CaptionPick::Auto | CaptionPick::Choice(_))) => Some(p),
            _ if self.0.pick_offered.get_untracked().unwrap_or(false) => Some(CaptionPick::Auto),
            _ if tracks > 0 => Some(CaptionPick::Track(0)),
            _ => first_choice.map(CaptionPick::Choice),
        };
        if let Some(p) = pick {
            self.set_caption_pick(p);
        }
    }

    /// Whether the captions control can do anything: tracks, choices or an app's Auto. Tracked.
    pub(crate) fn captions_offered(&self) -> bool {
        self.auto_offered()
            || !self.0.tracks.with(Vec::is_empty)
            || !self.0.caption_choices.with(Vec::is_empty)
    }

    /// The options the player was made with.
    pub(crate) fn options(&self) -> &PlayerOptions {
        &self.0.options
    }

    /// Puts the [`player`](crate::player) widget in its compact layout (`true`), as Portal's
    /// mini player: the picture, play and pause, the time and the app's
    /// [title](crate::PlayerParts::title) alone, the "Up next" card under them; or back.
    /// Playback carries on. Going compact closes the menus opened from the widget (its
    /// captions menu, the app's [cast](Self::on_cast) menu anchored to its button), leaves
    /// full screen and theater mode, and coming back
    /// asks for theater mode again if it was on. A full-screen or theater request from the
    /// widget (F, T, a double click) while compact comes back first.
    pub fn set_compact(&self, on: bool) {
        if self.0.compact.get_untracked().unwrap_or(false) == on {
            return;
        }
        if on {
            let (full, theater) = remedy_ui_reactive::untrack(|| (self.is_fullscreen(), self.is_theater()));
            if full {
                self.request_fullscreen(false);
            }
            self.0.theater_before_compact.set(theater);
            if theater {
                self.request_theater(false);
            }
        }
        self.0.compact.set(on);
        if !on && self.0.theater_before_compact.replace(false) {
            self.request_theater(true);
        }
    }

    /// Whether the widget is in its compact layout. Tracked.
    pub fn is_compact(&self) -> bool {
        self.0.compact.get()
    }

    /// Names what plays ("Episode 4 · The Long Night"): the [`player`](crate::player) widget
    /// shows it at the top of the picture in full screen while its controls show. `None`
    /// shows no title line.
    pub fn set_title(&self, title: Option<String>) {
        self.0.title.set_if_changed(title);
    }

    /// The [title](Player::set_title). Tracked.
    pub fn title(&self) -> Option<String> {
        self.0.title.get()
    }

    /// Whether the [`player`](crate::player) widget shows its controls: always, except in
    /// full screen after 2.6 s with no pointer, touch or key activity while playing (they
    /// stay while paused, busy, hovered, focused from the keyboard, scrubbed or with a menu
    /// open). Tracked.
    pub fn controls_shown(&self) -> bool {
        !self.is_fullscreen() || self.0.controls_awake.get()
    }

    /// Rests the controls now, as 2.6 s with no activity would: for pictures of the faded
    /// state (a preview fires no timers).
    #[doc(hidden)]
    pub fn rest_controls(&self) {
        self.set_awake(false);
    }

    pub(crate) fn set_awake(&self, on: bool) {
        self.0.controls_awake.set_if_changed(on);
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
        self.0.sync_awake();
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
    ///
    /// A report that says when the device made it ([`RemoteStatus::reported_at`],
    /// [`RemoteStatus::age`]) is moved on from then, at once.
    pub fn set_remote_status(&self, mut status: RemoteStatus) {
        let now = remedy_ui_core::time::Instant::now();
        let reported = status.reported_at.filter(|t| *t <= now).unwrap_or(now);
        // Where the device made the report, and when: the shown position moves on from there.
        let mut seen = (status.position, reported);
        if status.state == PlaybackState::Playing && reported < now {
            status.position =
                moved_on(status.position, now.duration_since(reported), self.0.remote_end_for(&status));
        }
        if let Some((to, at)) = self.0.remote_seek.get() {
            let landed = matches!(status.state, PlaybackState::Playing | PlaybackState::Paused)
                && status.position.abs_diff(to) < SEEK_LANDED;
            if landed || at.elapsed() > SEEK_LAND {
                self.0.remote_seek.set(None);
            } else {
                status.position = to;
                seen = (to, now);
            }
        }
        self.0.remote_status.set_if_changed(status);
        self.0.remote_seen.set(Some(seen));
        self.0.schedule_remote_tick();
    }

    /// A handle that hands [`RemoteStatus`] news to this player from any thread.
    pub fn remote_status_sender(&self) -> RemoteStatusSender {
        RemoteStatusSender { id: self.0.id, waker: remedy_ui_reactive::waker().ok() }
    }

    /// The player's number on its UI thread (what a post from another thread carries).
    #[cfg(feature = "media-session")]
    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }

    /// The player numbered `id` on this thread, while it lives.
    #[cfg(feature = "media-session")]
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

    /// Shows pictures in `sink`, in place of any [`player_picture`](crate::player_picture)
    /// (which attaches itself when shown).
    pub fn attach(&self, sink: Option<VideoSink>) {
        self.0.holder.set(None);
        self.send_sink(sink);
    }

    fn send_sink(&self, sink: Option<VideoSink>) {
        *self.0.sink.borrow_mut() = sink.clone();
        self.0.send(Cmd::Sink(sink));
    }

    /// A [`player_picture`](crate::player_picture) `id` with `sink` was made: it takes the
    /// pictures while nothing shows them yet.
    pub(crate) fn add_picture(&self, id: u64, sink: VideoSink) {
        self.0.pictures.borrow_mut().push((id, sink.clone(), false));
        if self.0.sink.borrow().is_none() {
            self.0.holder.set(Some(id));
            self.send_sink(Some(sink));
        }
    }

    /// Picture `id` is seen or not (`None`: gone). A picture just seen takes the pictures; one
    /// that holds them and stops being seen hands them to another seen one. The player is seen
    /// while any picture is.
    pub(crate) fn picture_seen(&self, id: u64, seen: Option<bool>) {
        let hand = {
            let mut pics = self.0.pictures.borrow_mut();
            match seen {
                Some(on) => pics.iter_mut().filter(|p| p.0 == id).for_each(|p| p.2 = on),
                None => pics.retain(|p| p.0 != id),
            }
            match seen {
                Some(true) => pics.iter().find(|p| p.0 == id).map(|p| (p.0, p.1.clone())),
                _ if self.0.holder.get() == Some(id) => pics.iter().find(|p| p.2).map(|p| (p.0, p.1.clone())),
                _ => None,
            }
        };
        if let Some((id, sink)) = hand
            && self.0.holder.get() != Some(id)
        {
            self.0.holder.set(Some(id));
            self.send_sink(Some(sink));
        }
        let any = self.0.pictures.borrow().iter().any(|p| p.2);
        if self.0.visible.get() != any {
            self.set_visible(any);
        }
    }

    /// Tells the player whether it is seen (the widget does, from the engine's visibility).
    pub fn set_visible(&self, seen: bool) {
        self.0.visible.set(seen);
        self.0.send(Cmd::Visible(seen));
        self.0.sync_awake();
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

    /// How long the countdown at the end runs ([`PlayerOptions::next_up_delay`]), as set now.
    /// Tracked.
    pub fn next_up_delay(&self) -> Option<Duration> {
        self.0.next_up_delay.get()
    }

    /// Changes [`PlayerOptions::next_up_delay`] while the player runs (the person turns
    /// autoplay off or on mid-episode). It takes effect at once: a countdown running counts to
    /// the new length from when it began (playing the next now if that is past), `None` stops
    /// it (the card stays, Play now still plays), and at the end a countdown starts if none
    /// runs and the person did not cancel one.
    pub fn set_next_up_delay(&self, delay: Option<Duration>) {
        let old = self.0.next_up_delay.get_untracked().ok().flatten();
        self.0.next_up_delay.set_if_changed(delay);
        match (self.0.deadline.get(), old, delay) {
            (Some(end), Some(old), Some(new)) => {
                let began = end.checked_sub(old).unwrap_or(end);
                self.0.stop_countdown();
                self.0.deadline.set(Some(began + new));
                self.0.tick();
            }
            (Some(_), _, None) => self.0.stop_countdown(),
            _ if self.0.state.get_untracked().ok() == Some(PlaybackState::Ended) => self.0.count_down(),
            _ => {}
        }
    }

    /// How long before the end the "Up next" card shows ([`PlayerOptions::next_up_lead`]), as
    /// set now. Tracked.
    pub fn next_up_lead(&self) -> Option<Duration> {
        self.0.next_up_lead.get()
    }

    /// Changes [`PlayerOptions::next_up_lead`] while the player runs; the widget's card follows
    /// at once.
    pub fn set_next_up_lead(&self, lead: Option<Duration>) {
        self.0.next_up_lead.set_if_changed(lead);
    }

    /// Whether the player keeps the screen on while video plays here
    /// ([`set_keep_awake`](Self::set_keep_awake)).
    pub fn keep_awake(&self) -> bool {
        self.0.keep_awake.get()
    }

    /// Keeps the screen on (or not) while video plays here, as a browser does for a playing
    /// video: held while playing and seen, not while paused, ended, opening, failed, unseen,
    /// remote (casting) or for sound only, and let go when the player is dropped. With the
    /// `keep-awake` feature the hold is the system's (the desktop services' `power::inhibit`:
    /// a power request on Windows, a power assertion on macOS, the session's screen saver on
    /// Linux, the window's keep-screen-on flag on Android, the Screen Wake Lock in a browser);
    /// without it nothing is held unless the app gives a hold of its own
    /// ([`set_awake_hold`](Self::set_awake_hold)). A platform that refuses is asked again at
    /// the next change.
    pub fn set_keep_awake(&self, on: bool) {
        self.0.keep_awake.set(on);
        self.0.sync_awake();
    }

    /// Whether the player holds the screen on now ([`set_keep_awake`](Self::set_keep_awake)).
    pub fn is_holding_awake(&self) -> bool {
        self.0.awake.borrow().is_some()
    }

    /// Holds the screen on with `hold` in place of the system's: it is called when the hold
    /// starts and returns a guard, dropped when it ends (`None`: refused). For tests, or an
    /// app with a hold of its own.
    pub fn set_awake_hold(&self, hold: impl Fn() -> Option<Box<dyn std::any::Any>> + 'static) {
        self.0.awake.borrow_mut().take();
        *self.0.awake_hold.borrow_mut() = Some(Rc::new(hold));
        self.0.sync_awake();
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

/// Whether what has focus keeps `e` from [`Player::keys_from`], as Portal's `typingTarget`:
/// a text field every key, a slider its own, a button Space and Enter; and a menu or dialog
/// every key while one has focus or, with nothing focused, is open.
fn focus_keeps(ui: &Ui, e: &remedy_ui_input::KeyEvent) -> bool {
    use remedy_ui_a11y::{Action, Role};
    use remedy_ui_input::Key;
    let tree = ui.tree();
    let Some(focused) = tree.focused() else {
        // Menus and dialogs sit in the overlay layers beside the content, as focus scopes.
        let layers = ui.root().parent().map(|w| w.child_elements()).unwrap_or_default();
        return layers
            .iter()
            .skip(1)
            .flat_map(El::child_elements)
            .any(|o| tree.focus_scope(o.id()) == o.id());
    };
    let role = |el: &El| el.info().and_then(|i| i.semantics).map(|s| s.role());
    let mut at = Some(ui.element(focused));
    while let Some(el) = at {
        if matches!(role(&el), Some(Role::Menu | Role::MenuBar | Role::Dialog)) {
            return true;
        }
        at = el.parent();
    }
    let Some(info) = ui.element(focused).info() else { return false };
    let role = info.semantics.as_ref().map(|s| s.role());
    let text = matches!(
        role,
        Some(Role::TextInput | Role::MultilineTextInput | Role::PasswordInput | Role::SpinButton)
    );
    let slider_key = matches!(
        e.key,
        Key::ArrowLeft
            | Key::ArrowRight
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
    );
    let presses = info.clickable || info.semantics.is_some_and(|s| s.actions().contains(&Action::Click));
    info.field
        || text
        || (role == Some(Role::Slider) && slider_key)
        || (presses && matches!(e.key, Key::Space | Key::Enter))
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
        let Ok(text) = read_subtitle_file(&path) else { continue };
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
