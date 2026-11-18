//! A player's two workers (threads natively; cooperative tasks in a browser page, see
//! [`remedy_ui_core::exec`]).
//!
//! - **Decoding** reads ahead of the playhead into a buffer sized from the stream and the
//!   machine: enough sound to keep the device's buffer full twice over and the pictures that
//!   cover the same time, never more decoded pictures than the machine's memory allows.
//! - **Presenting** feeds the sound device, keeps the clock and hands each picture to the
//!   video sink when its time comes. With sound, the clock is the sound device's: how much
//!   of what was written has been heard. Without sound (or with no device), it is the
//!   monotonic wall clock.
//!
//! The UI thread talks to them with commands and hears back through [`Status`], posted with
//! the reactive [`Waker`](remedy_ui_reactive::Waker) so only the player's signals change.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use remedy_ui_core::exec::{Bell, Next, Worker};
use remedy_ui_core::time::Instant;

use remedy_ui_media_video::{Picture, VideoSink};

use crate::host::{AudioFormat, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Sample};
use crate::subtitles::{Cue, SubtitleTrack};

/// How far ahead pictures are decoded when there is no sound device buffer to go by, and the
/// least look-ahead in any case: a quarter second rides out a decoder's slow frames (key
/// frames, scene cuts) without a visible stall.
const MIN_LEAD: Duration = Duration::from_millis(250);

/// How often the position is reported while playing with no new picture (sound only): ten
/// times a second keeps a time label and a seek bar moving smoothly for people.
const POSITION_INTERVAL: Duration = Duration::from_millis(100);

/// Frame rate assumed for sizing the look-ahead when a stream does not say its own.
const ASSUMED_FPS: f64 = 30.0;

/// What a player is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PlaybackState {
    /// The source is being opened.
    #[default]
    Opening,
    /// Ready, not playing.
    Paused,
    /// Playing.
    Playing,
    /// Asked to play, waiting for data (a slow stream).
    Buffering,
    /// Played to the end.
    Ended,
    /// It could not be opened or decoding failed; [`Player::error`](crate::Player::error)
    /// says why.
    Failed,
}

/// How playback has gone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackStats {
    /// Pictures handed to the video surface.
    pub frames_shown: u64,
    /// Pictures decoded but skipped: late, replaced before their time, or let go because
    /// the decoded-picture budget was full.
    pub frames_dropped: u64,
    /// Sound frames written to the device.
    pub audio_frames: u64,
}

/// Commands from the UI thread.
pub(crate) enum Cmd {
    Play,
    Pause,
    Seek(Duration),
    /// Linear volume, 0 to 1 (0 when muted).
    Volume(f32),
    Visible(bool),
    /// Pause, rather than only stop showing pictures, while unseen.
    PauseUnseen(bool),
    Sink(Option<VideoSink>),
    /// The decoder has something new.
    Wake,
    Close,
}

/// What the presenting thread tells the UI thread.
#[derive(Default)]
pub(crate) struct Status {
    pub state: PlaybackState,
    pub position: Duration,
    pub info: Option<MediaInfo>,
    pub error: Option<MediaError>,
    pub audio_error: Option<MediaError>,
    pub stats: PlaybackStats,
    /// Subtitle files found next to the media, not yet taken.
    pub new_tracks: Vec<SubtitleTrack>,
    /// Cues from embedded text tracks, not yet taken.
    pub cues: Vec<(usize, Cue)>,
}

/// The status and whether a delivery to the UI thread is on its way.
#[derive(Default)]
pub(crate) struct Shared {
    pub status: Mutex<Status>,
    pub posted: AtomicBool,
}

/// Posts a delivery to the UI thread; false once the UI thread is gone.
pub(crate) type Notify = Box<dyn Fn() -> bool + Send>;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct VideoItem {
    at: Duration,
    picture: Picture,
}

struct AudioItem {
    at: Duration,
    samples: Vec<f32>,
}

/// The decoded buffer between the two threads.
#[derive(Default)]
struct Feed {
    /// Bumped by each seek: samples read under an older one are dropped.
    generation: u64,
    seek: Option<Duration>,
    /// Samples ending before this (the last seek's target) are dropped.
    floor: Duration,
    video: VecDeque<VideoItem>,
    video_bytes: usize,
    audio: VecDeque<AudioItem>,
    audio_frames: u64,
    cues: Vec<(usize, Cue)>,
    eof: bool,
    error: Option<MediaError>,
    closed: bool,
    // Targets, set by the presenting thread from the stream and the sound device.
    want_audio_frames: u64,
    want_video: usize,
    budget_bytes: usize,
    dropped: u64,
    audio_format: Option<AudioFormat>,
    /// How long one picture shows, when the stream says (for dropping before a seek's target).
    frame_time: Option<Duration>,
    /// How far ahead the decoder reads.
    lead: Duration,
    /// Where the newest sound decoded ends, and when the newest picture shows.
    audio_end: Duration,
    video_newest: Duration,
}

impl Feed {
    fn needs_more(&self) -> bool {
        !self.eof && (self.audio_frames < self.want_audio_frames || self.video.len() < self.want_video)
    }

    fn clear(&mut self) {
        self.audio_end = self.floor;
        self.video_newest = self.floor;
        self.video.clear();
        self.video_bytes = 0;
        self.audio.clear();
        self.audio_frames = 0;
        self.eof = false;
        self.error = None;
    }

    fn insert(&mut self, sample: Sample) {
        match sample {
            Sample::Video { at, duration, picture } => {
                let shows_for = duration.or(self.frame_time);
                let before = match shows_for {
                    Some(d) => at.saturating_add(d) <= self.floor,
                    None => at < self.floor,
                };
                if before {
                    return;
                }
                let bytes = picture.data.len();
                self.video_newest = self.video_newest.max(at);
                self.video.push_back(VideoItem { at, picture });
                self.video_bytes += bytes;
                // Past the budget, the oldest pictures go: they are the ones the clock is
                // already past or nearly so.
                while self.video_bytes > self.budget_bytes && self.video.len() > 1 {
                    if let Some(old) = self.video.pop_front() {
                        self.video_bytes -= old.picture.data.len();
                        self.dropped += 1;
                    }
                }
            }
            Sample::Audio { at, mut samples } => {
                let Some(fmt) = self.audio_format else { return };
                let ch = usize::from(fmt.channels.max(1));
                samples.truncate(samples.len() - samples.len() % ch);
                let frames = (samples.len() / ch) as u64;
                let end = at.saturating_add(fmt.duration_of(frames));
                if end <= self.floor || frames == 0 {
                    return;
                }
                let mut at = at;
                if at < self.floor {
                    let cut = fmt.frames_in(self.floor - at).min(frames);
                    let cut_samples = usize::try_from(cut).unwrap_or(usize::MAX).saturating_mul(ch);
                    samples.drain(..cut_samples.min(samples.len()));
                    at = self.floor;
                }
                self.audio_frames += (samples.len() / ch) as u64;
                self.audio_end = self.audio_end.max(end);
                self.audio.push_back(AudioItem { at, samples });
            }
            Sample::Text { track, at, until, text } => {
                if until > at {
                    self.cues.push((track, Cue { start: at, end: until, text }));
                }
            }
        }
    }
}

struct FeedShared {
    feed: Mutex<Feed>,
    /// Rings the decoding worker, once it runs.
    decoder: Mutex<Option<Bell>>,
}

impl FeedShared {
    /// Tells the decoding worker the feed changed (room freed, a seek, closing).
    fn ring(&self) {
        if let Some(b) = lock(&self.decoder).as_ref() {
            b.ring();
        }
    }
}

/// A waker that rings a worker (a decoder's answer arriving later, in a browser page).
struct BellWake(Bell);

impl Wake for BellWake {
    fn wake(self: Arc<Self>) {
        self.0.ring();
    }
}

/// Tells the presenting worker the decoder has news; false once it is gone.
type WakePresenter = Arc<dyn Fn() -> bool + Send + Sync>;

/// The decoding worker's state.
struct Decoding {
    dec: Box<dyn Decoder>,
    shared: Arc<FeedShared>,
    wake: WakePresenter,
    waker: Waker,
}

/// One step of decoding: a seek, or one sample read into the feed when it wants more.
fn decode_step(st: &mut Decoding) -> Next {
    let (generation, seek) = {
        let mut f = lock(&st.shared.feed);
        if f.closed || !(f.seek.is_some() || f.needs_more()) {
            return Next::Idle;
        }
        (f.generation, f.seek.take())
    };
    if let Some(t) = seek {
        if let Err(e) = st.dec.seek(t) {
            let mut f = lock(&st.shared.feed);
            if f.generation == generation {
                f.error = Some(e);
                f.eof = true;
            }
            drop(f);
            if !(st.wake)() {
                return Next::Idle;
            }
        }
        return Next::Now;
    }
    let read = match st.dec.poll_read(&mut Context::from_waker(&st.waker)) {
        Poll::Ready(r) => r,
        // The decoder rings this worker when the sample is ready (a browser's decoder).
        Poll::Pending => return Next::Idle,
    };
    let mut f = lock(&st.shared.feed);
    if f.generation != generation {
        // A seek came while this was read: it is from before it.
        return Next::Now;
    }
    let (no_video, no_audio) = (f.video.is_empty(), f.audio.is_empty());
    let ended = match read {
        Ok(Some(s)) => {
            f.insert(s);
            false
        }
        Ok(None) => {
            f.eof = true;
            true
        }
        Err(e) => {
            f.error = Some(e);
            f.eof = true;
            true
        }
    };
    // The presenting worker hears of the first picture or sound after it ran out, and of
    // the end; the rest it takes on its own schedule.
    let arrived = (no_video && !f.video.is_empty()) || (no_audio && !f.audio.is_empty());
    drop(f);
    if (arrived || ended) && !(st.wake)() {
        return Next::Idle;
    }
    Next::Now
}

/// The UI thread's line to a player's presenting worker.
pub(crate) struct Control {
    tx: Sender<Cmd>,
    worker: Worker,
}

impl Control {
    /// Sends a command and wakes the worker to take it.
    pub(crate) fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok() {
            self.worker.notify();
        }
    }
}

/// Starts a player's workers. `notify` posts a delivery of [`Shared`] to the UI thread.
pub(crate) fn start(
    host: Arc<dyn MediaHost>,
    source: MediaSource,
    find_subtitles: bool,
    shared: Arc<Shared>,
    notify: Notify,
) -> Result<Control, MediaError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let wake = tx.clone();
    let worker = Worker::spawn_timed(
        "remedy-player",
        move |bell| {
            let mut presenter = Presenter::new(host, rx, shared, notify);
            presenter.open(&source, find_subtitles, wake, bell);
            presenter
        },
        Presenter::turn,
    )
    .map_err(|e| MediaError::Decode(format!("could not start the player's worker: {e}")))?;
    Ok(Control { tx, worker })
}

struct Presenter {
    host: Arc<dyn MediaHost>,
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    notify: Notify,
    feed: Arc<FeedShared>,
    info: MediaInfo,
    opened: bool,
    failed: Option<MediaError>,
    sink: Option<VideoSink>,
    last_picture: Option<Picture>,
    out: Option<Box<dyn AudioOutput>>,
    audio_error: Option<MediaError>,
    audio_tried: bool,
    gain: f32,
    /// Asked to play (by the person or autoplay).
    playing: bool,
    visible: bool,
    pause_unseen: bool,
    // The clock.
    base: Duration,
    anchor: Option<Instant>,
    audio_clock: bool,
    /// Where each run of written sound starts: (device frame, media time).
    chunks: VecDeque<(u64, Duration)>,
    written: u64,
    last_t: Duration,
    starved: bool,
    // Pictures.
    preview: bool,
    ended: bool,
    stats: PlaybackStats,
    last_publish: Option<Instant>,
    new_picture: bool,
    gone: bool,
    /// The sound device's clock when it last moved, and when.
    heard: Option<(u64, Instant)>,
    /// The output was told no more sound follows.
    finished: bool,
    /// The decoding worker.
    decoder: Option<Worker>,
    /// A decoder still getting ready (fetching, configuring), before decoding starts.
    opening: Option<Box<dyn Decoder>>,
    /// Tells this worker the decoder has news.
    wake: Option<Sender<Cmd>>,
    /// This worker's own bell.
    own: Option<Bell>,
    /// Closed: nothing more is done.
    done: bool,
    /// The device holds sound it is not playing; the wall clock leads meanwhile.
    stalled: bool,
}

impl Presenter {
    fn new(host: Arc<dyn MediaHost>, rx: Receiver<Cmd>, shared: Arc<Shared>, notify: Notify) -> Self {
        Presenter {
            host,
            rx,
            shared,
            notify,
            feed: Arc::new(FeedShared { feed: Mutex::new(Feed::default()), decoder: Mutex::new(None) }),
            info: MediaInfo::default(),
            opened: false,
            failed: None,
            sink: None,
            last_picture: None,
            out: None,
            audio_error: None,
            audio_tried: false,
            gain: 1.0,
            playing: false,
            visible: true,
            pause_unseen: false,
            base: Duration::ZERO,
            anchor: None,
            audio_clock: false,
            chunks: VecDeque::new(),
            written: 0,
            last_t: Duration::ZERO,
            starved: false,
            preview: true,
            ended: false,
            stats: PlaybackStats::default(),
            last_publish: None,
            new_picture: false,
            gone: false,
            heard: None,
            finished: false,
            stalled: false,
            decoder: None,
            opening: None,
            wake: None,
            own: None,
            done: false,
        }
    }

    fn open(&mut self, source: &MediaSource, find_subtitles: bool, wake: Sender<Cmd>, own: Bell) {
        if find_subtitles && let Some(path) = source.local_path() {
            let found = crate::player::sidecar_subtitles(path);
            if !found.is_empty() {
                lock(&self.shared.status).new_tracks.extend(found);
            }
        }
        self.wake = Some(wake);
        self.own = Some(own);
        match self.host.open(source) {
            Ok(dec) => {
                self.opening = Some(dec);
                self.finish_open();
            }
            Err(e) => {
                self.failed = Some(e);
                self.publish(Duration::ZERO, true);
            }
        }
    }

    /// Starts decoding once the decoder is ready: at once natively, later in a browser page
    /// (it fetches the source and configures the browser's decoders first).
    fn finish_open(&mut self) {
        let (Some(mut dec), Some(own), Some(wake)) =
            (self.opening.take(), self.own.clone(), self.wake.clone())
        else {
            return;
        };
        let waker = Waker::from(Arc::new(BellWake(own.clone())));
        match dec.poll_ready(&mut Context::from_waker(&waker)) {
            Poll::Pending => {
                self.opening = Some(dec);
                return;
            }
            Poll::Ready(Err(e)) => self.failed = Some(e),
            Poll::Ready(Ok(())) => {
                self.info = dec.info().clone();
                self.opened = true;
                {
                    let mut f = lock(&self.feed.feed);
                    f.audio_format = self.info.audio;
                    f.frame_time = self
                        .info
                        .video
                        .as_ref()
                        .and_then(|v| v.frame_rate)
                        .filter(|r| *r > 0.0)
                        .map(|r| Duration::from_secs_f64(1.0 / r));
                }
                self.size_buffers();
                let shared = self.feed.clone();
                let tell: WakePresenter = Arc::new(move || {
                    let sent = wake.send(Cmd::Wake).is_ok();
                    own.ring();
                    sent
                });
                let spawned = Worker::spawn_timed(
                    "remedy-player-decode",
                    move |bell| {
                        *lock(&shared.decoder) = Some(bell.clone());
                        Decoding { dec, shared, wake: tell, waker: Waker::from(Arc::new(BellWake(bell))) }
                    },
                    decode_step,
                );
                match spawned {
                    Ok(w) => self.decoder = Some(w),
                    Err(e) => {
                        self.failed =
                            Some(MediaError::Decode(format!("could not start the decoding worker: {e}")));
                    }
                }
            }
        }
        lock(&self.shared.status).info = self.opened.then(|| self.info.clone());
        self.publish(Duration::ZERO, true);
    }

    /// Sizes the look-ahead from the stream, the sound device and the machine.
    fn size_buffers(&mut self) {
        let lead = self.out.as_ref().map(|o| o.buffer() * 2).unwrap_or(MIN_LEAD).max(MIN_LEAD);
        let mut f = lock(&self.feed.feed);
        f.want_audio_frames = self.info.audio.map(|a| a.frames_in(lead)).unwrap_or(0);
        f.lead = lead;
        match &self.info.video {
            Some(v) => {
                let fps = v.frame_rate.filter(|r| r.is_finite() && *r > 0.0).unwrap_or(ASSUMED_FPS);
                let frames = (fps * lead.as_secs_f64()).ceil().max(2.0);
                f.want_video = frames as usize;
                // A decoded picture is at least its NV12 size.
                let frame_bytes = (v.width as usize).saturating_mul(v.height as usize).saturating_mul(3) / 2;
                let wanted = frame_bytes.saturating_mul(f.want_video);
                // Decoded pictures may take a sixty-fourth of the machine's memory (a quarter
                // GB on a 16 GB machine): far more than look-ahead needs, but room for files
                // whose sound runs well ahead of their pictures. Never less than the look-ahead.
                let machine = remedy_ui_view::device::total_memory()
                    .map(|m| usize::try_from(m / 64).unwrap_or(usize::MAX))
                    .unwrap_or(wanted.saturating_mul(8));
                f.budget_bytes = machine.max(wanted.saturating_mul(2));
            }
            None => {
                f.want_video = 0;
                f.budget_bytes = 0;
            }
        }
        drop(f);
        self.feed.ring();
    }

    fn running(&self) -> bool {
        self.playing && !self.ended && !(self.pause_unseen && !self.visible && self.info.video.is_some())
    }

    /// One turn of the presenting worker: the commands that came, then the clock, sound and
    /// pictures; says when to look again.
    fn turn(&mut self) -> Next {
        if self.done {
            return Next::Idle;
        }
        if self.opening.is_some() {
            self.finish_open();
            if self.opening.is_some() {
                // Commands wait in the channel until the source is open.
                return Next::Idle;
            }
        }
        let mut cmds = Vec::new();
        let mut disconnected = false;
        loop {
            match self.rx.try_recv() {
                Ok(c) => cmds.push(c),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        // Only the last of several seeks matters (a seek bar being dragged).
        let last_seek = cmds.iter().rposition(|c| matches!(c, Cmd::Seek(_)));
        for (i, c) in cmds.into_iter().enumerate() {
            match c {
                Cmd::Close => {
                    self.close();
                    return Next::Idle;
                }
                Cmd::Seek(_) if Some(i) != last_seek => {}
                c => self.apply(c),
            }
        }
        if self.gone || disconnected {
            self.close();
            return Next::Idle;
        }
        self.step();
        match self.next_wake() {
            Some(d) => Next::After(d),
            None => Next::Idle,
        }
    }

    fn close(&mut self) {
        self.done = true;
        lock(&self.feed.feed).closed = true;
        self.feed.ring();
        self.decoder = None;
        if let Some(mut out) = self.out.take() {
            let _ = out.pause();
        }
    }

    fn apply(&mut self, cmd: Cmd) {
        let was_running = self.running();
        match cmd {
            Cmd::Play => {
                if self.failed.is_some() || !self.opened {
                    return;
                }
                if self.ended {
                    self.ended = false;
                    self.seek(Duration::ZERO);
                }
                self.playing = true;
                self.open_audio();
            }
            Cmd::Pause => self.playing = false,
            Cmd::Seek(t) => self.seek(t),
            Cmd::Volume(v) => self.gain = v.clamp(0.0, 1.0),
            Cmd::Visible(v) => self.visible = v,
            Cmd::PauseUnseen(p) => self.pause_unseen = p,
            Cmd::Sink(s) => {
                self.sink = s;
                if let (Some(sink), Some(p)) = (&self.sink, &self.last_picture) {
                    sink.push(p.clone());
                }
            }
            Cmd::Wake | Cmd::Close => {}
        }
        let now_running = self.running();
        if was_running != now_running {
            self.set_running(now_running);
        }
    }

    /// Starts or stops the clock and the sound device.
    fn set_running(&mut self, on: bool) {
        // A paused device's clock stands still: that is no stall.
        self.heard = None;
        self.stalled = false;
        if on {
            self.anchor = Some(Instant::now());
            if let Some(out) = self.out.as_mut()
                && let Err(e) = out.resume()
            {
                self.lose_audio(e);
            }
        } else {
            self.base = self.last_t;
            self.anchor = None;
            if let Some(out) = self.out.as_mut()
                && let Err(e) = out.pause()
            {
                self.lose_audio(e);
            }
        }
        self.publish(self.last_t, true);
    }

    fn open_audio(&mut self) {
        if self.audio_tried {
            return;
        }
        self.audio_tried = true;
        let Some(fmt) = self.info.audio else { return };
        match self.host.audio_output(fmt) {
            Ok(out) => {
                self.out = Some(out);
                self.audio_clock = true;
                self.size_buffers();
            }
            Err(e) => self.lose_audio(e),
        }
    }

    /// The sound device failed: go on with the wall clock, without sound, and say why.
    fn lose_audio(&mut self, e: MediaError) {
        self.out = None;
        self.audio_error = Some(e.clone());
        lock(&self.shared.status).audio_error = Some(e);
        if self.audio_clock {
            self.audio_clock = false;
            self.base = self.last_t;
            self.anchor = self.running().then(Instant::now);
        }
        self.size_buffers();
        self.publish(self.last_t, true);
    }

    fn seek(&mut self, to: Duration) {
        if !self.opened || !self.info.seekable {
            return;
        }
        let to = self.info.duration.map_or(to, |d| to.min(d));
        {
            let mut f = lock(&self.feed.feed);
            f.generation += 1;
            f.seek = Some(to);
            f.floor = to;
            f.clear();
        }
        self.feed.ring();
        let running = self.running();
        if let Some(out) = self.out.as_mut() {
            let flushed = out.flush().and_then(|()| if running { out.resume() } else { Ok(()) });
            if let Err(e) = flushed {
                self.lose_audio(e);
            }
        }
        self.chunks.clear();
        self.written = 0;
        self.heard = None;
        self.finished = false;
        self.stalled = false;
        self.base = to;
        self.last_t = to;
        self.anchor = self.running().then(Instant::now);
        self.audio_clock = self.out.is_some();
        self.ended = false;
        self.preview = true;
        self.publish(to, true);
    }

    /// When the presenting thread should look again, with nothing else happening.
    fn next_wake(&mut self) -> Option<Duration> {
        if !self.running() {
            return None;
        }
        let t = self.now();
        let mut wake = POSITION_INTERVAL;
        if let Some(out) = &self.out {
            // Refill four times per buffer, so a late wake never lets the device run dry.
            wake = wake.min(out.buffer() / 4);
        }
        if let Some(next) = lock(&self.feed.feed).video.front().map(|v| v.at) {
            wake = wake.min(next.saturating_sub(t));
        }
        Some(wake.max(Duration::from_millis(1)))
    }

    /// The media time now.
    fn now(&mut self) -> Duration {
        if !self.running() {
            return self.last_t;
        }
        let mut t = match self.anchor {
            Some(a) => self.base + a.elapsed(),
            None => self.base,
        };
        self.starved = false;
        if self.audio_clock
            && let (Some(out), Some(fmt)) = (self.out.as_mut(), self.info.audio)
        {
            match out.played() {
                Ok(p) => {
                    let p = p.min(self.written);
                    if self.heard.is_none_or(|(h, _)| h != p) {
                        self.heard = Some((p, Instant::now()));
                        self.stalled = false;
                    }
                    while self.chunks.get(1).is_some_and(|c| c.0 <= p) {
                        self.chunks.pop_front();
                    }
                    t = match self.chunks.front() {
                        Some(&(start, at)) if p >= start => at + fmt.duration_of(p - start),
                        _ => self.base,
                    };
                    if p >= self.written {
                        // Everything written has been heard.
                        let f = lock(&self.feed.feed);
                        // No sound follows when the media has none left, or when the decoder is
                        // further ahead with pictures than its look-ahead past the last sound
                        // (the sound track has a gap, or ended before the pictures).
                        let no_more_sound = f.audio.is_empty()
                            && (f.eof || f.video_newest > f.audio_end.saturating_add(f.lead));
                        let waiting = f.audio.is_empty() && !f.eof;
                        drop(f);
                        if no_more_sound {
                            // The pictures go on by the wall clock from here.
                            self.audio_clock = false;
                            self.base = t.max(self.last_t);
                            self.anchor = Some(Instant::now());
                        } else {
                            self.starved = waiting;
                        }
                    } else if self.heard.is_some_and(|(_, at)| at.elapsed() > out.buffer() * 2) {
                        // The device holds sound but has stopped playing it (a sound server
                        // hiccup): the pictures go on by the wall clock until it moves again.
                        self.stalled = true;
                        self.audio_clock = false;
                        self.base = t.max(self.last_t);
                        self.anchor = Some(Instant::now());
                    }
                }
                Err(e) => self.lose_audio(e),
            }
        }
        // The clock never runs backwards while playing.
        let t = t.max(self.last_t);
        self.last_t = t;
        t
    }

    fn step(&mut self) {
        let running = self.running();
        if running {
            self.feed_audio();
        }
        let t = self.now();
        self.show_due(t, running);
        self.check_end(t);
        self.publish(t, false);
    }

    /// Drops decoded sound whose time the clock has passed.
    fn drop_passed_sound(&mut self, fmt: AudioFormat) {
        let ch = usize::from(fmt.channels.max(1));
        let t = self.last_t;
        let mut f = lock(&self.feed.feed);
        while let Some(a) = f.audio.front() {
            let frames = (a.samples.len() / ch) as u64;
            if a.at + fmt.duration_of(frames) > t {
                break;
            }
            f.audio_frames -= frames;
            f.audio.pop_front();
        }
        drop(f);
        self.feed.ring();
    }

    /// Writes as much decoded sound as the device takes.
    fn feed_audio(&mut self) {
        let Some(fmt) = self.info.audio else { return };
        let ch = usize::from(fmt.channels.max(1));
        if self.stalled {
            // What the stalled device cannot take is dropped as its time passes.
            self.drop_passed_sound(fmt);
        }
        let Some(out) = self.out.as_mut() else {
            // No sound device: sound is dropped as its time passes.
            return self.drop_passed_sound(fmt);
        };
        let mut room = match out.room() {
            Ok(r) => r,
            Err(e) => return self.lose_audio(e),
        };
        // Loudness follows the square of the volume, so the slider's middle sounds half as loud.
        let gain = self.gain * self.gain;
        let mut failed = None;
        let mut wrote = false;
        {
            let mut f = lock(&self.feed.feed);
            while room > 0 {
                let Some(item) = f.audio.front_mut() else { break };
                let frames = item.samples.len() / ch;
                let n = frames.min(room);
                let chunk: Vec<f32> =
                    item.samples.get(..n * ch).unwrap_or_default().iter().map(|s| s * gain).collect();
                if let Err(e) = out.write(&chunk) {
                    failed = Some(e);
                    break;
                }
                self.chunks.push_back((self.written, item.at));
                self.written += n as u64;
                self.stats.audio_frames += n as u64;
                room -= n;
                wrote = true;
                if n == frames {
                    f.audio.pop_front();
                } else {
                    item.samples.drain(..n * ch);
                    item.at += fmt.duration_of(n as u64);
                }
                f.audio_frames -= n as u64;
            }
            if failed.is_none() && f.eof && f.audio.is_empty() && !self.finished {
                // Some outputs hold a last partial block until told nothing follows.
                self.finished = true;
                if let Err(e) = out.finish() {
                    failed = Some(e);
                }
            }
        }
        self.feed.ring();
        if let Some(e) = failed {
            return self.lose_audio(e);
        }
        if wrote && !self.audio_clock && !self.stalled {
            // Sound is flowing again after a gap: it sets the time again.
            self.audio_clock = true;
        }
    }

    /// Shows the newest picture whose time has come (or, paused after a seek or at the start,
    /// the first one).
    fn show_due(&mut self, t: Duration, running: bool) {
        let mut f = lock(&self.feed.feed);
        let taken = if running {
            let mut latest: Option<VideoItem> = None;
            while f.video.front().is_some_and(|v| v.at <= t) {
                if let Some(v) = f.video.pop_front() {
                    f.video_bytes -= v.picture.data.len();
                    if latest.replace(v).is_some() {
                        self.stats.frames_dropped += 1;
                    }
                }
            }
            latest
        } else if self.preview {
            let v = f.video.pop_front();
            if let Some(v) = &v {
                f.video_bytes -= v.picture.data.len();
            }
            v
        } else {
            None
        };
        self.stats.frames_dropped += std::mem::take(&mut f.dropped);
        let cues = std::mem::take(&mut f.cues);
        drop(f);
        if !cues.is_empty() {
            lock(&self.shared.status).cues.extend(cues);
        }
        let Some(v) = taken else { return };
        self.feed.ring();
        self.preview = false;
        let show = self.visible || !running;
        if show && let Some(sink) = &self.sink {
            if sink.push(v.picture.clone()) {
                self.stats.frames_shown += 1;
            } else {
                // The surface is gone or refused it.
                self.stats.frames_dropped += 1;
            }
        }
        self.last_picture = Some(v.picture);
        self.new_picture = true;
    }

    fn check_end(&mut self, t: Duration) {
        if !self.playing || self.ended {
            return;
        }
        let f = lock(&self.feed.feed);
        let done = f.eof && f.video.is_empty() && f.audio.is_empty();
        let error = f.error.clone();
        drop(f);
        if !done {
            return;
        }
        let heard = match self.out.as_mut() {
            Some(o) if self.audio_clock => {
                // Everything written was heard, or the device's clock has stood still for
                // twice its buffer since the last sound went in (it holds nothing more).
                let stalled = self.heard.is_some_and(|(_, at)| at.elapsed() > o.buffer() * 2);
                o.played().map(|p| p >= self.written).unwrap_or(true) || stalled
            }
            _ => true,
        };
        if !heard {
            return;
        }
        if let Some(e) = error {
            self.failed = Some(e);
        }
        let end = self.info.duration.unwrap_or(t);
        self.set_running(false);
        self.ended = true;
        self.playing = false;
        self.last_t = end;
        self.base = end;
        self.publish(end, true);
    }

    fn state(&self) -> PlaybackState {
        if self.failed.is_some() {
            PlaybackState::Failed
        } else if !self.opened {
            PlaybackState::Opening
        } else if self.ended {
            PlaybackState::Ended
        } else if self.running() {
            if self.starved { PlaybackState::Buffering } else { PlaybackState::Playing }
        } else {
            PlaybackState::Paused
        }
    }

    /// Tells the UI thread, when something it shows changed.
    fn publish(&mut self, t: Duration, force: bool) {
        // Positions shown stay within the stated length: the last picture may run a little
        // past it.
        let t = self.info.duration.map_or(t, |d| t.min(d));
        let state = self.state();
        let due = self.last_publish.is_none_or(|p| p.elapsed() >= POSITION_INTERVAL);
        {
            let mut s = lock(&self.shared.status);
            let changed = force
                || s.state != state
                || self.new_picture
                || (t != s.position && due)
                || !s.cues.is_empty()
                || !s.new_tracks.is_empty();
            if !changed {
                return;
            }
            s.state = state;
            s.position = t;
            s.stats = self.stats;
            s.error = self.failed.clone();
            s.audio_error = self.audio_error.clone();
        }
        self.new_picture = false;
        self.last_publish = Some(Instant::now());
        if !self.shared.posted.swap(true, Ordering::AcqRel) && !(self.notify)() {
            // The UI thread is gone: nobody will play this.
            self.gone = true;
        }
    }
}
