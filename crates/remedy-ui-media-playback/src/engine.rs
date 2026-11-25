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
//! the reactive [`Waker`](remedy_ui_core::reactive::Waker) so only the player's signals change.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use remedy_ui_core::exec::{Bell, Next, Worker};
use remedy_ui_core::time::Instant;

use remedy_ui_media_video::{Picture, VideoSink};

use crate::host::{
    AudioFormat, AudioOutput, Decoder, MediaError, MediaHost, MediaInfo, MediaSource, Playout, Sample,
};
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
#[non_exhaustive]
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
#[non_exhaustive]
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
    /// Playback speed (1 is normal), pitch kept.
    Speed(f64),
    Visible(bool),
    /// Pause, rather than only stop showing pictures, while unseen.
    PauseUnseen(bool),
    Sink(Option<VideoSink>),
    /// Stop (or go on) asking the decoder for anything, so a stream is not fetched: while
    /// the app makes a new one, or a TV plays it. The last picture stays.
    Suspend(bool),
    /// Opens or closes the playout's own picture-in-picture window ([`Playout::set_pip`]).
    Pip(bool),
    /// Also hands each picture shown to this (a picture-in-picture window of the app's own).
    Mirror(Option<Mirror>),
    /// The decoder has something new.
    Wake,
    Close,
}

/// What else each picture shown goes to ([`Cmd::Mirror`]).
pub(crate) type Mirror = Arc<dyn Fn(&Picture) + Send + Sync>;

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
    /// What can play without waiting, merged ([`crate::host::merge_ranges`]).
    pub buffered: Vec<std::ops::Range<Duration>>,
    /// The playout can show its picture in a picture-in-picture window of its own, and does.
    pub pip_able: bool,
    pub in_pip: bool,
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
    /// The decoder's text samples are dropped: the source's captions were read from the
    /// file instead.
    skip_text: bool,
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
    /// What the source holds without waiting, as its decoder last said.
    source_ranges: Option<Vec<std::ops::Range<Duration>>>,
    /// The length as the decoder last said: a growing stream's (an HLS event playlist) grows.
    source_duration: Option<Duration>,
    /// Whether the decoder last said that length may still grow.
    source_growing: bool,
    /// The decoder is left alone ([`Cmd::Suspend`]): neither read nor asked what it holds.
    suspended: bool,
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
                let bytes = picture.bytes();
                self.video_newest = self.video_newest.max(at);
                self.video.push_back(VideoItem { at, picture });
                self.video_bytes += bytes;
                // Past the budget, the oldest pictures go: they are the ones the clock is
                // already past or nearly so.
                while self.video_bytes > self.budget_bytes && self.video.len() > 1 {
                    if let Some(old) = self.video.pop_front() {
                        self.video_bytes -= old.picture.bytes();
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
                if until > at && !self.skip_text {
                    // The player keeps a window of a stream's cues; one cue this long is no caption.
                    let limit = crate::media_limits().max_streamed_cue;
                    if text.len() > limit {
                        self.error = Some(MediaError::Format(crate::subtitles::cue_too_big(limit)));
                        self.eof = true;
                        return;
                    }
                    self.cues.push((track, Cue { start: at, end: until, text, ..Default::default() }));
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

/// Retires its engine ([`crate::hls::Retired`]) once dropped: held by both workers, after
/// what holds the decoder, so the last of them to end says the decoder is gone.
struct Retiring(Arc<crate::hls::Retired>);

impl Drop for Retiring {
    fn drop(&mut self) {
        self.0.retire();
    }
}

/// The decoding worker's state.
struct Decoding {
    dec: Box<dyn Decoder>,
    shared: Arc<FeedShared>,
    wake: WakePresenter,
    waker: Waker,
    /// When the decoder was last asked what the source holds.
    asked: Option<Instant>,
    _retiring: Arc<Retiring>,
}

/// How often the decoder is asked what the source holds (a query of the platform's pipeline).
const BUFFERED_INTERVAL: Duration = Duration::from_millis(250);

/// One step of decoding: a seek, or one sample read into the feed when it wants more.
fn decode_step(st: &mut Decoding) -> Next {
    // A stream's decoder fetches when asked anything (HLS reloads its playlist on `buffered`).
    if lock(&st.shared.feed).suspended {
        return Next::Idle;
    }
    // What the source holds, a few times a second: a stream goes on arriving while the
    // decoded buffer is full or playback is paused.
    let mut keep_asking = false;
    if st.asked.is_none_or(|a| a.elapsed() >= BUFFERED_INTERVAL) {
        st.asked = Some(Instant::now());
        let ranges = st.dec.buffered();
        let (duration, growing) = (st.dec.info().duration, st.dec.info().growing);
        let mut f = lock(&st.shared.feed);
        let changed =
            f.source_ranges != ranges || f.source_duration != duration || f.source_growing != growing;
        f.source_duration = duration;
        f.source_growing = growing;
        // Asked again later, even with nothing to decode, while the source says what it holds.
        keep_asking = ranges.is_some();
        f.source_ranges = ranges;
        drop(f);
        if changed && !(st.wake)() {
            return Next::Idle;
        }
    } else if lock(&st.shared.feed).source_ranges.is_some() {
        keep_asking = true;
    }
    let (generation, seek) = {
        let mut f = lock(&st.shared.feed);
        if f.closed {
            return Next::Idle;
        }
        if !(f.seek.is_some() || f.needs_more()) {
            return if keep_asking { Next::After(BUFFERED_INTERVAL) } else { Next::Idle };
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
    let (no_video, no_audio, no_cues) = (f.video.is_empty(), f.audio.is_empty(), f.cues.is_empty());
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
    // The presenting worker hears of the first picture or sound after it ran out, of the
    // first cue it has not taken, and of the end; the rest it takes on its own schedule.
    // Cues are heard of too: paused, the presenter runs only when woken, and a cue decoded
    // after the picture a seek shows would otherwise wait for the next play or seek.
    let arrived = (no_video && !f.video.is_empty())
        || (no_audio && !f.audio.is_empty())
        || (no_cues && !f.cues.is_empty());
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
    /// Set on closing: an open still under way (an HLS stream's fetches) gives up.
    stop: Arc<AtomicBool>,
    /// Set once the engine is done with its decoder: the next engine's open waits for it.
    pub(crate) retired: Arc<crate::hls::Retired>,
}

impl Drop for Control {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Control {
    /// Sends a command and wakes the worker to take it.
    pub(crate) fn send(&self, cmd: Cmd) {
        if matches!(cmd, Cmd::Close) {
            self.stop.store(true, Ordering::Relaxed);
        }
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
    after: Option<Arc<crate::hls::Retired>>,
) -> Result<Control, MediaError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let wake = tx.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let retired = Arc::new(crate::hls::Retired::default());
    let retiring = Arc::new(Retiring(retired.clone()));
    let worker = Worker::spawn_timed(
        "remedy-player",
        move |bell| {
            let mut presenter = Presenter::new(host, rx, shared, notify, retiring);
            presenter.open(&source, find_subtitles, wake, bell, &stopping, after);
            presenter
        },
        Presenter::turn,
    )
    .map_err(|e| MediaError::Decode(format!("could not start the player's worker: {e}")))?;
    Ok(Control { tx, worker, stop, retired })
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
    mirror: Option<Mirror>,
    last_picture: Option<Picture>,
    out: Option<Box<dyn AudioOutput>>,
    /// What plays a source that plays by itself (a browser's media element,
    /// [`Decoder::playout`]): its clock is the clock, and it plays the sound.
    playout: Option<Box<dyn Playout>>,
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
    /// Where each run of written sound starts: (device frame, media time, the speed it
    /// plays at: media time per device frame).
    chunks: VecDeque<(u64, Duration, f64)>,
    /// Playback speed: media time per wall or device time.
    speed: f64,
    /// Sound on its way to the device at a speed other than 1, and the media time of its
    /// first input frame (`None` while it holds nothing).
    stretcher: Option<crate::stretch::Stretcher>,
    stretch_from: Option<Duration>,
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
    /// The source is a file on this machine: all of it is there to play.
    local: bool,
    /// The source's captions were read from the file (an MP4's timed text, a Matroska
    /// file's text tracks), not from the decoder.
    own_text: bool,
    /// The device holds sound it is not playing; the wall clock leads meanwhile.
    stalled: bool,
    /// Last, after the decoder it holds while opening.
    retiring: Arc<Retiring>,
}

impl Presenter {
    fn new(
        host: Arc<dyn MediaHost>,
        rx: Receiver<Cmd>,
        shared: Arc<Shared>,
        notify: Notify,
        retiring: Arc<Retiring>,
    ) -> Self {
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
            mirror: None,
            last_picture: None,
            out: None,
            playout: None,
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
            speed: 1.0,
            stretcher: None,
            stretch_from: None,
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
            local: false,
            own_text: false,
            retiring,
        }
    }

    fn open(
        &mut self,
        source: &MediaSource,
        find_subtitles: bool,
        wake: Sender<Cmd>,
        own: Bell,
        stop: &Arc<AtomicBool>,
        after: Option<Arc<crate::hls::Retired>>,
    ) {
        if find_subtitles && let Some(path) = source.local_path() {
            let found = crate::player::sidecar_subtitles(path);
            if !found.is_empty() {
                lock(&self.shared.status).new_tracks.extend(found);
            }
        }
        self.wake = Some(wake);
        self.own = Some(own);
        self.local = source.local_path().is_some();
        match crate::hls::opening_until(stop, after, || crate::host::open_source(&*self.host, source)) {
            Ok(dec) => {
                // Timed text inside an MP4, and text subtitles inside Matroska and WebM, are
                // read from the file, whole, on every platform: Media Foundation,
                // AVFoundation, MediaCodec and WebCodecs do not hand them out, and GStreamer
                // hands each cue out well after the pictures around it (seconds late, past
                // its end, for a player reading just ahead of the clock). The decoder's copy
                // of the same tracks is then left out.
                if let Some(path) = source.local_path() {
                    let found = crate::mp4::text_tracks(path).and_then(|found| match found.is_empty() {
                        true => crate::mkv::text_tracks(path),
                        false => Ok(found),
                    });
                    let found = match found {
                        Ok(found) => found,
                        Err(e) => {
                            self.failed = Some(e);
                            self.publish(Duration::ZERO, true);
                            return;
                        }
                    };
                    if !found.is_empty() {
                        self.own_text = true;
                        lock(&self.shared.status).new_tracks.extend(found);
                    }
                }
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
                self.playout = dec.playout();
                if let Some(p) = self.playout.as_mut() {
                    p.set_volume(self.gain * self.gain);
                    p.set_speed(self.speed);
                }
                if self.own_text {
                    self.info.text_tracks.clear();
                }
                self.opened = true;
                {
                    let mut f = lock(&self.feed.feed);
                    f.skip_text = self.own_text;
                    // A playout plays its own sound: none is decoded here.
                    f.audio_format = self.info.audio.filter(|_| self.playout.is_none());
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
                let retiring = self.retiring.clone();
                let tell: WakePresenter = Arc::new(move || {
                    let sent = wake.send(Cmd::Wake).is_ok();
                    own.ring();
                    sent
                });
                let spawned = Worker::spawn_timed(
                    "remedy-player-decode",
                    move |bell| {
                        *lock(&shared.decoder) = Some(bell.clone());
                        Decoding {
                            dec,
                            shared,
                            wake: tell,
                            waker: Waker::from(Arc::new(BellWake(bell))),
                            asked: None,
                            _retiring: retiring,
                        }
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
        // Played faster, the same device buffer holds more media.
        let lead = lead.mul_f64(self.speed.max(1.0));
        let mut f = lock(&self.feed.feed);
        f.want_audio_frames =
            self.info.audio.filter(|_| self.playout.is_none()).map(|a| a.frames_in(lead)).unwrap_or(0);
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
                let machine = remedy_ui_engine::view::device::total_memory()
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
        if let Some(mut p) = self.playout.take() {
            p.pause();
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
            Cmd::Volume(v) => {
                self.gain = v.clamp(0.0, 1.0);
                // Loudness follows the square of the volume, as for sound written here.
                if let Some(p) = self.playout.as_mut() {
                    p.set_volume(self.gain * self.gain);
                }
            }
            Cmd::Speed(r) => self.set_speed(r),
            Cmd::Visible(v) => self.visible = v,
            Cmd::PauseUnseen(p) => self.pause_unseen = p,
            Cmd::Sink(s) => {
                self.sink = s;
                if let (Some(sink), Some(p)) = (&self.sink, &self.last_picture) {
                    sink.push(p.clone());
                }
            }
            Cmd::Pip(on) => {
                if let Some(p) = self.playout.as_mut() {
                    p.set_pip(on);
                }
            }
            Cmd::Mirror(m) => {
                self.mirror = m;
                if let (Some(m), Some(p)) = (&self.mirror, &self.last_picture) {
                    m(p);
                }
            }
            Cmd::Suspend(on) => {
                lock(&self.feed.feed).suspended = on;
                self.feed.ring();
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
        if let Some(p) = self.playout.as_mut() {
            if on { p.play() } else { p.pause() }
        }
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
        if self.audio_tried || self.playout.is_some() {
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

    /// Plays at `speed` from now: the wall clock is set going again at the new rate, sound
    /// already with the device plays out at the rate it was stretched at (the clock knows), and
    /// what is written next is stretched to the new one.
    fn set_speed(&mut self, speed: f64) {
        let speed = crate::stretch::clamp_speed(speed);
        if speed == self.speed {
            return;
        }
        let t = self.now();
        self.base = t;
        self.anchor = self.running().then(Instant::now);
        self.speed = speed;
        if let Some(p) = self.playout.as_mut() {
            p.set_speed(speed);
        }
        self.size_buffers();
        self.publish(t, true);
    }

    fn seek(&mut self, to: Duration) {
        if !self.opened || !self.info.seekable {
            return;
        }
        // A growing stream's length so far is no limit: the decoder waits for it to get there.
        let to = match self.info.duration {
            Some(d) if !self.info.growing => to.min(d),
            _ => to,
        };
        {
            let mut f = lock(&self.feed.feed);
            f.generation += 1;
            f.seek = Some(to);
            f.floor = to;
            f.clear();
        }
        self.feed.ring();
        if let Some(p) = self.playout.as_mut() {
            p.seek(to);
        }
        let running = self.running();
        if let Some(out) = self.out.as_mut() {
            let flushed = out.flush().and_then(|()| if running { out.resume() } else { Ok(()) });
            if let Err(e) = flushed {
                self.lose_audio(e);
            }
        }
        self.chunks.clear();
        self.written = 0;
        if let Some(s) = self.stretcher.as_mut() {
            s.reset();
        }
        self.stretch_from = None;
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
            Some(a) => self.base + a.elapsed().mul_f64(self.speed),
            None => self.base,
        };
        self.starved = false;
        if let Some(p) = self.playout.as_mut() {
            t = p.position();
            self.starved = p.waiting();
        } else if self.audio_clock
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
                        Some(&(start, at, speed)) if p >= start => {
                            at + fmt.duration_of(p - start).mul_f64(speed)
                        }
                        _ => self.base,
                    };
                    if p >= self.written {
                        // Everything written has been heard.
                        let f = lock(&self.feed.feed);
                        // No sound follows when the media has none left, or when the decoder is
                        // further ahead with pictures than its look-ahead past the last sound
                        // (the sound track has a gap, or ended before the pictures).
                        // Sound the time stretcher still holds is more to come.
                        let no_more_sound = f.audio.is_empty()
                            && self.stretch_from.is_none()
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
        let mut room = match self.out.as_mut().map(|o| o.room()) {
            Some(Ok(r)) => r,
            Some(Err(e)) => return self.lose_audio(e),
            // No sound device: sound is dropped as its time passes.
            None => return self.drop_passed_sound(fmt),
        };
        // Loudness follows the square of the volume, so the slider's middle sounds half as loud.
        let gain = self.gain * self.gain;
        let mut failed = None;
        let mut wrote = false;
        if self.speed != 1.0 || self.stretch_from.is_some() {
            // At another speed (or while sound stretched before is still held): through the
            // time stretcher, so the pitch stays.
            match self.feed_stretched(fmt, room, gain) {
                Ok(w) => wrote = w,
                Err(e) => failed = Some(e),
            }
            room = 0;
        }
        let Some(out) = self.out.as_mut() else { return };
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
                self.chunks.push_back((self.written, item.at, 1.0));
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
            if failed.is_none()
                && f.eof
                && f.audio.is_empty()
                && !self.finished
                && self.stretch_from.is_none()
            {
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

    /// Writes up to `room` frames of sound stretched to the current speed (see `stretch`). True
    /// when something was written.
    fn feed_stretched(&mut self, fmt: AudioFormat, mut room: usize, gain: f32) -> Result<bool, MediaError> {
        let ch = usize::from(fmt.channels.max(1));
        let speed = self.speed;
        let stretcher = self
            .stretcher
            .get_or_insert_with(|| crate::stretch::Stretcher::new(fmt.channels, fmt.sample_rate));
        let Some(out) = self.out.as_mut() else { return Ok(false) };
        let hop = stretcher.hop();
        let mut wrote = false;
        let mut block = Vec::with_capacity(hop * ch);
        let mut f = lock(&self.feed.feed);
        while room >= hop {
            block.clear();
            let from = self.stretch_from.unwrap_or_default();
            let at = from + fmt.duration_of(stretcher.position().max(0.0) as u64);
            if stretcher.step(speed, &mut block).is_some() {
                let samples: Vec<f32> = block.iter().map(|v| v * gain).collect();
                out.write(&samples)?;
                self.chunks.push_back((self.written, at, speed));
                self.written += hop as u64;
                self.stats.audio_frames += hop as u64;
                room -= hop;
                wrote = true;
                continue;
            }
            // More input: the next decoded sound, unless it does not follow on from what the
            // stretcher holds (a gap): then what it holds is dropped and it starts again. At
            // the end of the media, what it holds is played to its end.
            let Some(item) = f.audio.pop_front() else {
                if f.eof && self.stretch_from.is_some() && !stretcher.done() {
                    stretcher.finish();
                    continue;
                }
                break;
            };
            let frames = (item.samples.len() / ch) as u64;
            f.audio_frames -= frames;
            let held_end = from + fmt.duration_of(stretcher.input_end());
            let follows = self.stretch_from.is_some()
                && item.at.abs_diff(held_end) < Duration::from_millis(5).max(fmt.duration_of(hop as u64));
            if !follows {
                stretcher.reset();
                self.stretch_from = Some(item.at);
            }
            stretcher.push(&item.samples);
        }
        // Told nothing follows once the stretched sound has played to its end too.
        if f.eof && f.audio.is_empty() && (stretcher.done() || self.stretch_from.is_none()) && !self.finished
        {
            self.finished = true;
            out.finish()?;
        }
        if speed == 1.0 && stretcher.held() < hop && !f.eof {
            // Back at normal speed with nothing much held: plain writing takes over.
            stretcher.reset();
            self.stretch_from = None;
        }
        drop(f);
        Ok(wrote)
    }

    /// Shows the newest picture whose time has come (or, paused after a seek or at the start,
    /// the first one).
    fn show_due(&mut self, t: Duration, running: bool) {
        let mut f = lock(&self.feed.feed);

        let taken = if running {
            let mut latest: Option<VideoItem> = None;
            while f.video.front().is_some_and(|v| v.at <= t) {
                if let Some(v) = f.video.pop_front() {
                    f.video_bytes -= v.picture.bytes();
                    if latest.replace(v).is_some() {
                        self.stats.frames_dropped += 1;
                    }
                }
            }
            latest
        } else if self.preview {
            let v = f.video.pop_front();
            if let Some(v) = &v {
                f.video_bytes -= v.picture.bytes();
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
        if let Some(m) = &self.mirror {
            m(&v.picture);
        }
        self.last_picture = Some(v.picture);
        self.new_picture = true;
    }

    fn check_end(&mut self, t: Duration) {
        if self.ended || self.failed.is_some() {
            return;
        }
        if !self.playing {
            // Paused, nothing plays the decoded queue out: a decoder that failed (a seek
            // refused) fails the player now, as playing to the error would.
            let error = {
                let f = lock(&self.feed.feed);
                f.error.clone().filter(|_| f.eof)
            };
            if let Some(e) = error {
                self.failed = Some(e);
                self.publish(t, true);
            }
            return;
        }
        let f = lock(&self.feed.feed);
        // A playout's end is its own: the last picture it showed may say a time just past it.
        let done = f.eof && f.audio.is_empty() && (f.video.is_empty() || self.playout.is_some());
        let error = f.error.clone();
        drop(f);
        if done && self.playout.is_some() {
            self.show_due(Duration::MAX, true);
        }
        // Sound taken from the queue is not yet written while the time stretcher holds it (or
        // an output holds a last partial block): the end waits for the output to be told
        // nothing follows, unless the sound is not leading the clock (no device, stalled).
        let written_out = self.finished || self.out.is_none() || !self.audio_clock;
        if !done || !written_out {
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

    /// What can play from `t` without waiting: all of a local file; otherwise what the
    /// source holds (when its decoder says) and what is decoded ahead of `t`.
    fn buffered(&self, t: Duration) -> Vec<std::ops::Range<Duration>> {
        if !self.opened {
            return Vec::new();
        }
        let length = self.info.duration;
        if self.local
            && let Some(d) = length
        {
            return crate::host::merge_ranges(vec![Duration::ZERO..d], Some(d));
        }
        let f = lock(&self.feed.feed);
        let mut ranges = f.source_ranges.clone().unwrap_or_default();
        let ahead = if f.eof {
            length.unwrap_or(f.audio_end.max(f.video_newest))
        } else {
            f.audio_end.max(f.video_newest)
        };
        if ahead > t {
            ranges.push(t..ahead);
        }
        drop(f);
        crate::host::merge_ranges(ranges, length)
    }

    /// Tells the UI thread, when something it shows changed.
    fn publish(&mut self, t: Duration, force: bool) {
        // A growing stream's length, as its decoder says it (an HLS event playlist gains segments).
        let mut force = force;
        let (grown, growing) = {
            let f = lock(&self.feed.feed);
            (f.source_duration, f.source_growing)
        };
        if self.opened && grown.is_some() && (grown != self.info.duration || growing != self.info.growing) {
            self.info.duration = grown;
            self.info.growing = growing;
            lock(&self.shared.status).info = Some(self.info.clone());
            force = true;
        }
        // Positions shown stay within the stated length: the last picture may run a little
        // past it. A growing stream's position may be past its length so far (a seek ahead).
        let t = match self.info.duration {
            Some(d) if !self.info.growing => t.min(d),
            _ => t,
        };
        let state = self.state();
        let (pip_able, in_pip) = match self.playout.as_mut() {
            Some(p) => (p.can_pip(), p.in_pip()),
            None => (false, false),
        };
        let due = self.last_publish.is_none_or(|p| p.elapsed() >= POSITION_INTERVAL);
        let buffered = self.buffered(t);
        {
            let mut s = lock(&self.shared.status);
            let changed = force
                || s.state != state
                || self.new_picture
                || (t != s.position && due)
                // While playing, with the position; while paused, as it arrives.
                || (buffered != s.buffered && (due || !self.running()))
                || !s.cues.is_empty()
                || !s.new_tracks.is_empty()
                || (s.pip_able, s.in_pip) != (pip_able, in_pip);
            if !changed {
                return;
            }
            s.state = state;
            s.position = t;
            s.buffered = buffered;
            s.stats = self.stats;
            s.error = self.failed.clone();
            s.audio_error = self.audio_error.clone();
            (s.pip_able, s.in_pip) = (pip_able, in_pip);
        }
        self.new_picture = false;
        self.last_publish = Some(Instant::now());
        if !self.shared.posted.swap(true, Ordering::AcqRel) && !(self.notify)() {
            // The UI thread is gone: nobody will play this.
            self.gone = true;
        }
    }
}
