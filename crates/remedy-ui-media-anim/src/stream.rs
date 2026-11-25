//! One animation being decoded, shared by every element that shows it at the same size.
//!
//! A worker decodes frames ahead of the one shown and the UI thread only ever picks the finished
//! picture up. Short animations are kept whole (they loop with no decoding); long ones keep a
//! window of frames around the playhead and decode the rest again each time round. All elements
//! showing a stream share its playhead, so copies of one animation play in step and the window
//! stays the same size however many show it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use remedy_ui_core::time::Instant;

use remedy_ui_media_image::{DecodeError, Source, SourceId, Stamp, decode_for_box, spawn_job};
use remedy_ui_paint::Image;
use remedy_ui_core::reactive::Waker;

use crate::policy;
use crate::route::{FrameSource, Loops, Opened, open};

/// What a stream tells the elements showing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// Something is new: a frame, the size, a failure.
    Changed,
    /// The file changed on disk: ask for the stream again.
    Stale,
}

/// A frame the playhead is on.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
    /// Its place in the endless sequence of frames (the first play is `0..n`, the next `n..2n`).
    pub seq: u64,
    pub image: Image,
    /// How long it stays.
    pub delay: Duration,
    /// When the playhead arrived on it.
    pub started: Instant,
}

/// What stepping the playhead did.
pub(crate) enum Step {
    /// The playhead is on this frame.
    Show(Shown),
    /// The next frame is still being decoded; the stream will say when it is.
    Wait,
    /// The last frame of the last play is up.
    End,
}

/// Whether a frame follows the one shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// Yes, ready or on its way.
    More,
    /// Not known yet.
    Wait,
    /// No: a still, or the last play is over.
    End,
}

struct Slot {
    image: Image,
    delay: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// The file has not been read (or was let go of to save memory).
    Unopened,
    /// Frames are coming from the decoder.
    Open,
    /// Everything there is to decode has been (a still, or a whole animation kept).
    Done,
    Failed,
}

struct Play {
    /// The frame shown.
    seq: u64,
    /// Where the current run of plays began (a multiple of the frame count).
    origin: u64,
    /// When the playhead arrived on `seq`.
    started: Instant,
    /// The last frame of the last play has been reached.
    finished: bool,
}

struct State {
    stage: Stage,
    failed: Option<DecodeError>,
    source_size: Option<(u32, u32)>,
    show_at: (u32, u32),
    loops: Loops,
    decoder: Option<Box<dyn FrameSource>>,
    /// A job is queued or running.
    job: bool,
    /// Bumped when the playhead is sent back, so a frame decoded for the old run is dropped.
    epoch: u64,
    /// The decoder is to start over before it makes another frame.
    restart: bool,
    /// The seq the next decoded frame gets.
    next_seq: u64,
    /// Frames decoded in the play the decoder is in.
    pass_count: u64,
    /// Frames in one play.
    total: Option<u64>,
    /// Every frame is kept (the animation is short).
    whole: bool,
    /// The most frames kept.
    limit: usize,
    frames: VecDeque<(u64, Slot)>,
    poster: Option<Image>,
    delay_sum: Duration,
    delay_n: u64,
    play: Play,
    /// Elements playing it.
    active: usize,
    /// An element is waiting for a frame.
    starved: bool,
    stamp: Option<Stamp>,
}

impl State {
    fn new(live: usize) -> State {
        State {
            stage: Stage::Unopened,
            failed: None,
            source_size: None,
            show_at: (1, 1),
            loops: Loops::Times(1),
            decoder: None,
            job: false,
            epoch: 0,
            restart: false,
            next_seq: 0,
            pass_count: 0,
            total: None,
            whole: true,
            limit: policy::resident_frames(0, live),
            frames: VecDeque::new(),
            poster: None,
            delay_sum: Duration::ZERO,
            delay_n: 0,
            play: Play { seq: 0, origin: 0, started: Instant::now(), finished: false },
            active: 0,
            starved: false,
            stamp: None,
        }
    }

    /// The first seq that is not shown (the end of the last play), when plays are counted.
    fn end_seq(&self) -> Option<u64> {
        match (self.loops, self.total) {
            (Loops::Times(n), Some(t)) => {
                Some(self.play.origin.saturating_add(t.saturating_mul(u64::from(n))))
            }
            _ => None,
        }
    }

    fn finite_end(&self, seq: u64) -> bool {
        self.end_seq().is_some_and(|e| seq >= e)
    }

    fn average_delay(&self) -> Duration {
        if self.delay_n == 0 {
            Duration::from_millis(100)
        } else {
            self.delay_sum / u32::try_from(self.delay_n).unwrap_or(u32::MAX)
        }
    }

    /// How many frames past the one shown are decoded in advance.
    fn ahead(&self) -> u64 {
        policy::ahead_frames(self.average_delay(), self.limit) as u64
    }

    /// Whether the worker has more to decode now.
    fn wants_more(&self) -> bool {
        match self.stage {
            Stage::Failed | Stage::Done => false,
            // A trimmed stream reads its file again only when something plays it.
            Stage::Unopened => self.source_size.is_none() || self.active > 0 || self.starved,
            Stage::Open => {
                if self.finite_end(self.next_seq) && !self.whole {
                    return false;
                }
                let engaged = self.active > 0 || self.starved;
                if self.whole {
                    // Every frame while something plays; otherwise just the first two.
                    self.frames.len() < 2 || engaged
                } else {
                    // The frame shown, and those the next quarter second needs.
                    self.next_seq <= self.play.seq + if engaged { self.ahead() } else { 0 }
                }
            }
        }
    }

    /// Where in `frames` the frame `seq` is, if it is kept.
    fn position(&self, seq: u64) -> Option<usize> {
        match self.total {
            Some(t) if self.whole && t > 0 => usize::try_from(seq % t).ok(),
            _ => usize::try_from(seq.checked_sub(self.frames.front()?.0)?).ok(),
        }
    }

    fn slot(&self, seq: u64) -> Option<&Slot> {
        self.frames.get(self.position(seq)?).map(|(_, s)| s)
    }

    fn shown(&self, seq: u64) -> Option<Shown> {
        let s = self.slot(seq)?;
        Some(Shown { seq, image: s.image.clone(), delay: s.delay, started: self.play.started })
    }

    /// Lets go of frames the playhead has passed (a streamed animation keeps its poster).
    fn evict(&mut self) {
        if self.whole {
            return;
        }
        while self.frames.front().is_some_and(|(s, _)| *s < self.play.seq) {
            self.frames.pop_front();
        }
    }

    fn resident(&self) -> usize {
        let poster_apart = self.poster.is_some() && self.frames.front().is_none_or(|(s, _)| *s != 0);
        self.frames.len() + usize::from(poster_apart)
    }

    fn fail(&mut self, e: DecodeError) {
        self.failed = Some(e);
        self.stage = Stage::Failed;
        self.decoder = None;
        self.frames.clear();
    }

    /// Keeps a decoded frame.
    fn push(&mut self, seq: u64, image: Image, delay: Duration) {
        if self.poster.is_none() {
            self.poster = Some(image.clone());
        }
        // Delays are learned from the first play only.
        if self.delay_n < self.total.unwrap_or(u64::MAX) {
            self.delay_sum += delay;
            self.delay_n += 1;
        }
        self.frames.push_back((seq, Slot { image, delay }));
        self.next_seq = seq + 1;
        self.pass_count += 1;
        self.evict();
    }

    /// The decoder has run out of frames. Returns true if it is to start over.
    fn pass_over(&mut self) -> bool {
        // A file with fewer frames than it said plays the ones it has.
        if self.total != Some(self.pass_count) {
            if self.pass_count == 0 {
                self.fail(DecodeError::Malformed(String::from("the animation has no frames")));
                return false;
            }
            self.total = Some(self.pass_count);
            self.whole = self.whole || self.pass_count <= self.limit as u64;
        }
        if self.whole {
            // Everything is kept: nothing more to decode.
            self.stage = Stage::Done;
            self.decoder = None;
            return false;
        }
        self.pass_count = 0;
        true
    }

    /// Sends the playhead back to the first frame.
    fn rewind(&mut self) {
        self.play.finished = false;
        let Some(total) = self.total.filter(|t| *t > 0) else {
            self.play.seq = 0;
            self.play.origin = 0;
            return;
        };
        if self.play.seq == self.play.origin {
            return;
        }
        // The next multiple of the frame count after anything shown or decoded.
        let new = (self.play.seq.max(self.next_seq) / total + 1) * total;
        self.play.seq = new;
        self.play.origin = new;
        if !self.whole {
            self.frames.clear();
            self.next_seq = new;
            self.pass_count = 0;
            self.epoch += 1;
            self.restart = true;
        }
    }
}

/// What picks a stream out in the cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    source: SourceId,
    bucket: (u32, u32),
    cover: bool,
}

pub(crate) struct Stream {
    id: u64,
    key: Key,
    source: Source,
    /// The box asked for, in device pixels (each side a power of two).
    box_px: (u32, u32),
    cover: bool,
    /// How the UI thread hears of news; `None` when there is no UI thread (a failed stream).
    waker: Option<Waker>,
    cancelled: Arc<AtomicBool>,
    notified: Arc<AtomicBool>,
    live_at_start: usize,
    st: Mutex<State>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

// ---------- the cache and the elements listening ----------

type Listeners = HashMap<u64, Vec<(u64, Rc<dyn Fn(Event)>)>>;

thread_local! {
    static STREAMS: RefCell<HashMap<Key, Weak<Stream>>> = RefCell::default();
    static LISTENERS: RefCell<Listeners> = RefCell::default();
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(0) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn bucket(px: u32) -> u32 {
    px.max(1).next_power_of_two()
}

/// Calls whoever listens to `stream`, on the UI thread.
fn notify(stream: u64, event: Event) {
    let list: Vec<Rc<dyn Fn(Event)>> = LISTENERS
        .try_with(|l| l.borrow().get(&stream).map(|v| v.iter().map(|(_, f)| f.clone()).collect()))
        .ok()
        .flatten()
        .unwrap_or_default();
    for f in list {
        f(event);
    }
}

/// How many animations are alive (for tests and the inspector).
pub fn cached() -> usize {
    STREAMS.with(|s| s.borrow().values().filter(|w| w.strong_count() > 0).count())
}

/// How many decoded frames the live animations hold in all (for tests and the inspector).
pub fn resident_frames() -> usize {
    STREAMS.with(|s| s.borrow().values().filter_map(Weak::upgrade).map(|st| st.lock().resident()).sum())
}

/// Lets go of the frames of animations nothing is playing; they are decoded again when played.
/// For memory pressure.
pub fn trim_memory() {
    let streams: Vec<Arc<Stream>> = STREAMS.with(|s| s.borrow().values().filter_map(Weak::upgrade).collect());
    for st in streams {
        let mut s = st.lock();
        if s.active == 0 && s.stage != Stage::Failed && s.source_size.is_some() && s.total != Some(1) {
            s.frames.clear();
            s.decoder = None;
            s.stage = Stage::Unopened;
            s.epoch += 1;
            s.restart = false;
            s.next_seq = 0;
            s.pass_count = 0;
            s.play = Play { seq: 0, origin: 0, started: Instant::now(), finished: false };
        }
    }
}

impl Stream {
    /// The stream for `source` shown in a box of `box_px` device pixels (fitting it, or
    /// covering it): the one in use if there is one.
    pub fn request(source: &Source, box_px: (u32, u32), cover: bool) -> Arc<Stream> {
        let key = Key { source: source.id(), bucket: (bucket(box_px.0), bucket(box_px.1)), cover };
        let found = STREAMS.with(|s| {
            let mut s = s.borrow_mut();
            s.retain(|_, w| w.strong_count() > 0);
            s.get(&key).and_then(Weak::upgrade)
        });
        if let Some(st) = found {
            st.revalidate();
            return st;
        }
        let live = STREAMS.with(|s| s.borrow().len()) + 1;
        let waker = remedy_ui_core::reactive::waker().ok();
        let mut state = State::new(live);
        if waker.is_none() {
            state.stage = Stage::Failed;
            state.failed = Some(DecodeError::Io(String::from("there is no UI thread to show it on")));
        }
        let st = Arc::new(Stream {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            box_px: key.bucket,
            key: key.clone(),
            source: source.clone(),
            cover,
            waker,
            cancelled: Arc::new(AtomicBool::new(false)),
            notified: Arc::new(AtomicBool::new(false)),
            live_at_start: live,
            st: Mutex::new(state),
        });
        STREAMS.with(|s| s.borrow_mut().insert(key, Arc::downgrade(&st)));
        st.kick();
        st
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts calling `f` when this stream has news. Returns the token to stop with.
    pub fn listen(&self, f: Rc<dyn Fn(Event)>) -> u64 {
        let token = NEXT_TOKEN.with(|t| {
            t.set(t.get() + 1);
            t.get()
        });
        LISTENERS.with(|l| l.borrow_mut().entry(self.id).or_default().push((token, f)));
        token
    }

    /// Stops the listener `token`.
    pub fn unlisten(&self, token: u64) {
        let _ = LISTENERS.try_with(|l| {
            let mut l = l.borrow_mut();
            if let Some(v) = l.get_mut(&self.id) {
                v.retain(|(t, _)| *t != token);
                if v.is_empty() {
                    l.remove(&self.id);
                }
            }
        });
    }

    /// Tells the listeners on the UI thread (a `Changed` is sent once however many things
    /// happened since the last).
    fn wake(&self, event: Event) {
        let Some(waker) = &self.waker else { return };
        if event == Event::Changed && self.notified.swap(true, Ordering::AcqRel) {
            return;
        }
        let (id, notified) = (self.id, self.notified.clone());
        let _ = waker.post(move || {
            notified.store(false, Ordering::Release);
            notify(id, event);
        });
    }

    /// Queues the worker if there is something for it to do.
    fn kick(self: &Arc<Self>) {
        let mut st = self.lock();
        self.kick_locked(&mut st);
    }

    fn kick_locked(self: &Arc<Self>, st: &mut State) {
        if st.job || !st.wants_more() {
            return;
        }
        st.job = true;
        let weak = Arc::downgrade(self);
        spawn_job(self.cancelled.clone(), move || run(weak));
    }

    /// A file may have changed since the stream was made: look off the UI thread, and have the
    /// elements ask again if it did.
    fn revalidate(self: &Arc<Self>) {
        if !matches!(self.source, Source::File(_)) || self.waker.is_none() {
            return;
        }
        let weak = Arc::downgrade(self);
        spawn_job(Arc::new(AtomicBool::new(false)), move || {
            let Some(me) = weak.upgrade() else { return };
            let old = me.lock().stamp;
            if old.is_none() || me.source.stamp() == old {
                return;
            }
            let (key, id) = (me.key.clone(), me.id);
            if let Some(waker) = &me.waker {
                let _ = waker.post(move || {
                    STREAMS.with(|s| {
                        let mut s = s.borrow_mut();
                        if s.get(&key).and_then(Weak::upgrade).is_some_and(|st| st.id == id) {
                            s.remove(&key);
                        }
                    });
                    notify(id, Event::Stale);
                });
            }
        });
    }

    // ----- what the elements ask -----

    /// The source's size, once known.
    pub fn source_size(&self) -> Option<(u32, u32)> {
        self.lock().source_size
    }

    /// Why it cannot be shown.
    pub fn failure(&self) -> Option<DecodeError> {
        self.lock().failed.clone()
    }

    /// The first frame, the picture shown when nothing plays.
    pub fn poster(&self) -> Option<Image> {
        self.lock().poster.clone()
    }

    /// True once it is known that there is more than one frame.
    pub fn is_animated(&self) -> bool {
        self.lock().total.is_some_and(|t| t > 1)
    }

    /// The frame the playhead is on, if decoded.
    pub fn current(self: &Arc<Self>) -> Option<Shown> {
        let mut st = self.lock();
        let seq = st.play.seq;
        let shown = st.shown(seq);
        if shown.is_none() {
            st.starved = true;
            self.kick_locked(&mut st);
        }
        shown
    }

    /// An element starts playing it. With `restart` and nothing else playing, it begins again
    /// from the first frame.
    pub fn activate(self: &Arc<Self>, restart: bool, now: Instant) {
        let mut st = self.lock();
        if st.active == 0 {
            if restart {
                st.rewind();
            }
            st.play.started = now;
        }
        st.active += 1;
        self.kick_locked(&mut st);
    }

    /// An element stops playing it.
    pub fn deactivate(&self) {
        let mut st = self.lock();
        st.active = st.active.saturating_sub(1);
    }

    /// Whether a frame follows the one at `seq`.
    pub fn after(self: &Arc<Self>, seq: u64) -> Next {
        let mut st = self.lock();
        if st.stage == Stage::Failed || st.total == Some(1) || st.finite_end(seq + 1) {
            return Next::End;
        }
        if st.slot(seq + 1).is_some() {
            return Next::More;
        }
        st.starved = true;
        self.kick_locked(&mut st);
        Next::Wait
    }

    /// The timer of the frame at `seq` ran out. Moves the playhead on unless another element
    /// already has, and says where it is. Frames a late timer has already missed are skipped.
    pub fn step(self: &Arc<Self>, seq: u64, now: Instant) -> Step {
        let mut st = self.lock();
        if st.play.seq != seq {
            // Another element moved it: this one catches up.
            let at = st.play.seq;
            return match st.shown(at) {
                Some(s) => Step::Show(s),
                None => {
                    st.starved = true;
                    self.kick_locked(&mut st);
                    Step::Wait
                }
            };
        }
        let next = seq + 1;
        if st.finite_end(next) {
            st.play.finished = true;
            return Step::End;
        }
        if st.slot(next).is_none() {
            st.starved = true;
            self.kick_locked(&mut st);
            return Step::Wait;
        }
        // How late the timer was, in whole frames that have been missed.
        let held = now.saturating_duration_since(st.play.started);
        let mut late = held.saturating_sub(st.slot(seq).map_or(Duration::ZERO, |s| s.delay));
        let mut at = next;
        loop {
            let delay = st.slot(at).map_or(Duration::MAX, |s| s.delay);
            if late < delay || st.finite_end(at + 1) || st.slot(at + 1).is_none() {
                break;
            }
            late -= delay;
            at += 1;
        }
        st.play.seq = at;
        st.play.started = now;
        st.evict();
        let shown = st.shown(at);
        self.kick_locked(&mut st);
        match shown {
            Some(s) => Step::Show(s),
            None => Step::Wait,
        }
    }
}

// ---------- the worker ----------

/// How long a job decodes before it lets other images have the worker.
const SLICE: Duration = Duration::from_millis(8);

fn run(weak: Weak<Stream>) {
    let started = Instant::now();
    loop {
        let Some(me) = weak.upgrade() else { return };
        // Decide under the lock, decode outside it.
        let (mut dec, epoch, seq, restart) = {
            let mut st = me.lock();
            if !st.wants_more() {
                st.job = false;
                return;
            }
            if st.stage == Stage::Unopened {
                drop(st);
                open_stage(&me);
                continue;
            }
            let Some(dec) = st.decoder.take() else {
                st.job = false;
                return;
            };
            (dec, st.epoch, st.next_seq, std::mem::take(&mut st.restart))
        };
        let restarted = if restart {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dec.restart())).unwrap_or_else(|_| {
                Err(DecodeError::Malformed(String::from("the decoder gave up on this file")))
            })
        } else {
            Ok(())
        };
        if let Err(e) = restarted {
            let mut st = me.lock();
            st.fail(e);
            st.job = false;
            drop(st);
            me.wake(Event::Changed);
            return;
        }
        // A decoder that trips over a hostile file must not take the worker with it.
        let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dec.next()));
        let Ok(made) = decoded else {
            let mut st = me.lock();
            st.fail(DecodeError::Malformed(String::from("the decoder gave up on this file")));
            st.job = false;
            drop(st);
            me.wake(Event::Changed);
            return;
        };
        {
            let mut st = me.lock();
            if st.epoch != epoch {
                // The playhead was sent back while this decoded: the frame belongs to a run that
                // is over. The decoder starts over next time.
                st.decoder = Some(dec);
            } else {
                let first = st.poster.is_none();
                let mut changed = first;
                match made {
                    Ok(Some((image, delay))) => {
                        st.push(seq, image, delay);
                        st.decoder = Some(dec);
                        changed |= std::mem::take(&mut st.starved);
                    }
                    Ok(None) => {
                        changed = true;
                        if st.pass_over() {
                            st.restart = true;
                            st.decoder = Some(dec);
                        }
                    }
                    // A damaged tail ends the animation where it is, as browsers do; damage
                    // before the first frame is a failure.
                    Err(e) => {
                        changed = true;
                        if st.pass_count > 0 {
                            if st.pass_over() {
                                st.restart = true;
                                st.decoder = Some(dec);
                            }
                        } else {
                            st.fail(e);
                        }
                    }
                }
                if changed {
                    me.wake(Event::Changed);
                }
            }
        }
        if started.elapsed() > SLICE {
            // Others have been waiting: go to the back of the queue (the job flag stays set).
            if me.lock().wants_more() {
                let weak = Arc::downgrade(&me);
                spawn_job(me.cancelled.clone(), move || run(weak));
            } else {
                me.lock().job = false;
            }
            return;
        }
    }
}

/// Reads the file and opens its decoder (or decodes it, if it is a still).
fn open_stage(me: &Arc<Stream>) {
    let opened = me.source.read().and_then(|(bytes, stamp)| {
        let animated =
            std::panic::catch_unwind(|| open(&bytes, me.box_px, me.cover)).unwrap_or_else(|_| {
                Err(DecodeError::Malformed(String::from("the decoder gave up on this file")))
            })?;
        Ok((bytes, stamp, animated))
    });
    let still = match opened {
        Ok((bytes, stamp, Opened::Still)) => Some((decode_for_box(&bytes, Some(me.box_px), me.cover), stamp)),
        Ok((_, stamp, Opened::Animated(dec))) => {
            let mut st = me.lock();
            let a = *dec.animation();
            let show_at = a.show_at;
            let frame_bytes = (show_at.0 as usize).saturating_mul(show_at.1 as usize).saturating_mul(4);
            st.stamp = stamp;
            st.source_size = Some(a.size);
            st.show_at = show_at;
            st.loops = a.loops;
            st.total = a.frames;
            st.limit = policy::resident_frames(frame_bytes, me.live_at_start);
            st.whole = st.total.is_none_or(|t| t <= st.limit as u64);
            st.delay_sum = Duration::ZERO;
            st.delay_n = 0;
            st.frames.clear();
            st.decoder = Some(dec);
            st.stage = Stage::Open;
            None
        }
        Err(e) => {
            me.lock().fail(e);
            None
        }
    };
    if let Some((decoded, stamp)) = still {
        let mut st = me.lock();
        match decoded {
            Ok(d) => {
                st.stamp = stamp;
                st.source_size = Some(d.source_size);
                st.show_at = (d.image.width, d.image.height);
                st.total = Some(1);
                st.poster = Some(d.image.clone());
                st.frames.clear();
                st.frames.push_back((0, Slot { image: d.image, delay: Duration::ZERO }));
                st.next_seq = 1;
                st.stage = Stage::Done;
            }
            Err(e) => st.fail(e),
        }
    }
    me.wake(Event::Changed);
}
