//! Work away from the UI thread, on every platform the engine runs on.
//!
//! The engine does slow work (decoding images, rastering document pages, sorting large
//! tables, checking spelling, compositing) away from the UI thread. On native platforms that
//! means threads. A browser page built without shared memory has one thread, and
//! `std::thread::spawn` fails there; the same work then runs cooperatively on the page's
//! thread, in time slices the event loop hands out between frames and input, so the page
//! stays responsive. Engine code uses this module instead of `std::thread` and runs
//! unchanged on both:
//!
//! - [`spawn`]: one piece of work, on its own thread or as one task between frames;
//! - [`Pool`]: workers that drain a queue the caller owns, woken by [`Pool::notify`]: up to
//!   [`parallelism`] threads natively, a cooperative task that works in slices on the web;
//! - [`recv`] and [`recv_timeout`]: waiting for a channel the way the platform can wait (on
//!   the web, by running the queued work that will send).
//!
//! The web bridge drives the cooperative queue: it installs a [`set_scheduler`] hook that
//! asks the page for a turn, and calls [`run_pending`] in that turn with a time budget.

use std::sync::mpsc::{Receiver, RecvError, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Why work could not be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnError {
    reason: String,
}

impl SpawnError {
    /// Why, in the system's words.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the work could not start: {}", self.reason)
    }
}

impl std::error::Error for SpawnError {}

/// True in a browser page without threads (`wasm32-unknown-unknown`).
const WEB: bool = cfg!(all(target_arch = "wasm32", target_os = "unknown"));

/// True when work runs on threads of its own; false in a browser page, where it runs on the
/// page's thread in slices between frames and input.
pub fn threads() -> bool {
    !WEB
}

/// How many pieces of work can run at the same time: the machine's hardware threads, or 1
/// in a browser page.
pub fn parallelism() -> usize {
    if WEB { 1 } else { std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get) }
}

/// Runs `work` away from the caller: on a new thread named `name`, or in a browser page as
/// one task the event loop runs between frames. A task runs whole, so work that takes long
/// belongs in a [`Pool`] whose step does a piece at a time.
pub fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> Result<(), SpawnError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let _ = name;
        web::push(web::Task::Once(Box::new(work)));
        Ok(())
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(work)
            .map(drop)
            .map_err(|e| SpawnError { reason: e.to_string() })
    }
}

/// The work a pool does: one unit per call, returning true when it found work (so it is
/// called again) and false when its queue is empty.
type Step = Box<dyn Fn() -> bool + Send + Sync>;

struct PoolState {
    /// Notifications not yet taken by a worker.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pending: usize,
    /// Workers waiting for a notification.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    idle: usize,
    /// Workers running.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    workers: usize,
    /// The pool's handle is gone: workers leave once nothing is pending.
    closed: bool,
    /// Web: a cooperative task for this pool is queued or running.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    scheduled: bool,
    /// Web: a notification arrived since the running task last looked.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    dirty: bool,
}

struct PoolInner {
    name: String,
    most: usize,
    step: Step,
    state: Mutex<PoolState>,
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    ready: std::sync::Condvar,
}

/// Workers draining a queue the caller owns.
///
/// The caller keeps its own queue (with its own priorities and cancellation) and gives the
/// pool a step function that takes and does one unit from it. After queueing work it calls
/// [`Pool::notify`]. Natively, threads are started as work arrives, up to the pool's limit,
/// and wait when the queue is empty; in a browser page one cooperative task calls the step
/// in time slices until the queue is empty. Dropping the pool lets its workers finish what is
/// queued and leave.
pub struct Pool {
    inner: Arc<PoolInner>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pool({}, up to {})", self.inner.name, self.inner.most)
    }
}

impl Pool {
    /// A pool named `name` (its threads carry the name) with at most `most` workers at once
    /// (at least 1), calling `step` for each unit of work.
    pub fn new(
        name: impl Into<String>,
        most: usize,
        step: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Pool {
        Pool {
            inner: Arc::new(PoolInner {
                name: name.into(),
                most: most.max(1),
                step: Box::new(step),
                state: Mutex::new(PoolState {
                    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                    pending: 0,
                    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                    idle: 0,
                    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                    workers: 0,
                    closed: false,
                    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                    scheduled: false,
                    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                    dirty: false,
                }),
                #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                ready: std::sync::Condvar::new(),
            }),
        }
    }

    /// Work was queued: wakes a waiting worker, starts one if none waits and the pool is
    /// below its limit, or (in a browser page) schedules the pool's cooperative task. When no
    /// worker exists and none can start, the work runs here, in the caller.
    pub fn notify(&self) {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let start = {
                let mut s = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
                s.dirty = true;
                !std::mem::replace(&mut s.scheduled, true)
            };
            if start {
                let inner = self.inner.clone();
                web::push(web::Task::Steps(Box::new(move || {
                    inner.state.lock().unwrap_or_else(PoisonError::into_inner).dirty = false;
                    if (inner.step)() {
                        return true;
                    }
                    let mut s = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
                    if s.dirty {
                        return true;
                    }
                    s.scheduled = false;
                    false
                })));
            }
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let mut s = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
            s.pending += 1;
            if s.idle == 0 && s.workers < self.inner.most {
                let inner = self.inner.clone();
                let started = std::thread::Builder::new()
                    .name(self.inner.name.clone())
                    .spawn(move || native_worker(&inner))
                    .is_ok();
                if started {
                    s.workers += 1;
                } else if s.workers == 0 {
                    // No thread to hand it to: do it now rather than never.
                    s.pending = 0;
                    drop(s);
                    while (self.inner.step)() {}
                    return;
                }
            }
            self.inner.ready.notify_one();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        let mut s = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.closed = true;
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        self.inner.ready.notify_all();
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn native_worker(inner: &PoolInner) {
    loop {
        {
            let mut s = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if s.pending > 0 {
                    s.pending -= 1;
                    break;
                }
                if s.closed {
                    s.workers -= 1;
                    return;
                }
                s.idle += 1;
                s = inner.ready.wait(s).unwrap_or_else(PoisonError::into_inner);
                s.idle -= 1;
            }
        }
        while (inner.step)() {}
    }
}

/// When a [`Worker`] has work, and whether its handle is gone.
#[derive(Default)]
struct WorkerFlags {
    /// Notifications not yet seen by the worker.
    pending: bool,
    /// The handle was dropped: the worker leaves once it has no work.
    closed: bool,
    /// Web: the worker's cooperative task is queued or running.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    scheduled: bool,
}

struct WorkerSignal {
    flags: Mutex<WorkerFlags>,
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    ready: std::sync::Condvar,
}

/// What a worker's step found, and so when it runs again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// There is more to do now.
    Now,
    /// Nothing more until the worker is rung, or until this much time has passed (a player
    /// showing its next picture, a clock to read again).
    After(Duration),
    /// Nothing more until the worker is rung.
    Idle,
}

/// Rings a [`Worker`]: work was queued for it. Cheap to clone, and sendable to any thread
/// (or, in a browser page, to the page's callbacks).
#[derive(Clone)]
pub struct Bell {
    signal: Arc<WorkerSignal>,
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    id: u64,
}

impl std::fmt::Debug for Bell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bell")
    }
}

impl Bell {
    /// Wakes the worker (or schedules its cooperative task).
    pub fn ring(&self) {
        let mut f = self.signal.flags.lock().unwrap_or_else(PoisonError::into_inner);
        f.pending = true;
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        self.signal.ready.notify_one();
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        if !std::mem::replace(&mut f.scheduled, true) {
            drop(f);
            let bell = self.clone();
            web::push(web::Task::Steps(Box::new(move || {
                bell.signal.flags.lock().unwrap_or_else(PoisonError::into_inner).pending = false;
                let next = web::step_worker(bell.id);
                if next == Next::Now {
                    return true;
                }
                let mut f = bell.signal.flags.lock().unwrap_or_else(PoisonError::into_inner);
                if f.pending {
                    return true;
                }
                f.scheduled = false;
                drop(f);
                if let Next::After(d) = next {
                    web::ring_after(&bell, d);
                }
                false
            })));
        }
    }
}

/// One worker with state that stays where it was made: natively a thread of its own that
/// builds the state and keeps it (state that must not move between threads, such as a
/// platform object, is fine); in a browser page a cooperative task on the page's thread.
///
/// The caller feeds it through its own channel or queue and rings it ([`Worker::notify`], or
/// a [`Bell`] from [`Worker::bell`]); the worker then calls its step until the step reports
/// no work. A timed worker's step can also ask to run again after a while
/// ([`Next::After`]). Dropping the handle lets the worker finish its work and end, dropping
/// its state.
pub struct Worker {
    bell: Bell,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Worker")
    }
}

impl Worker {
    /// Starts a worker named `name` whose state `make` builds (on the worker's thread), and
    /// whose `step` does one unit of work, returning false when it found none.
    pub fn spawn<S: 'static>(
        name: &str,
        make: impl FnOnce() -> S + Send + 'static,
        mut step: impl FnMut(&mut S) -> bool + Send + 'static,
    ) -> Result<Worker, SpawnError> {
        Self::spawn_timed(name, move |_| make(), move |s| if step(s) { Next::Now } else { Next::Idle })
    }

    /// Starts a worker whose `step` says when it runs again ([`Next`]). `make` builds its
    /// state on the worker's thread and is handed the worker's own [`Bell`] (for the state to
    /// give to whatever reports back to it). The step runs once at the start.
    pub fn spawn_timed<S: 'static>(
        name: &str,
        make: impl FnOnce(Bell) -> S + Send + 'static,
        mut step: impl FnMut(&mut S) -> Next + Send + 'static,
    ) -> Result<Worker, SpawnError> {
        let signal = Arc::new(WorkerSignal {
            flags: Mutex::new(WorkerFlags::default()),
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            ready: std::sync::Condvar::new(),
        });
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let _ = name;
            let mut state: Option<S> = None;
            let mut make = Some(make);
            let id = web::next_worker_id();
            let bell = Bell { signal, id };
            let own = bell.clone();
            web::add_worker(
                id,
                Box::new(move || {
                    if state.is_none()
                        && let Some(m) = make.take()
                    {
                        state = Some(m(own.clone()));
                    }
                    state.as_mut().map_or(Next::Idle, &mut step)
                }),
            );
            bell.ring();
            Ok(Worker { bell })
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let bell = Bell { signal };
            let own = bell.clone();
            std::thread::Builder::new()
                .name(name.to_owned())
                .spawn(move || {
                    let sig = own.signal.clone();
                    let mut state = make(own);
                    let mut next = Next::Now;
                    loop {
                        if next != Next::Now {
                            let deadline = match next {
                                Next::After(d) => Some(std::time::Instant::now() + d),
                                Next::Now | Next::Idle => None,
                            };
                            let mut f = sig.flags.lock().unwrap_or_else(PoisonError::into_inner);
                            loop {
                                if f.pending {
                                    break;
                                }
                                if f.closed {
                                    return;
                                }
                                match deadline {
                                    None => f = sig.ready.wait(f).unwrap_or_else(PoisonError::into_inner),
                                    Some(at) => {
                                        let now = std::time::Instant::now();
                                        if now >= at {
                                            break;
                                        }
                                        f = sig
                                            .ready
                                            .wait_timeout(f, at - now)
                                            .unwrap_or_else(PoisonError::into_inner)
                                            .0;
                                    }
                                }
                            }
                            f.pending = false;
                        }
                        next = step(&mut state);
                    }
                })
                .map_err(|e| SpawnError { reason: e.to_string() })?;
            Ok(Worker { bell })
        }
    }

    /// Work was queued: wakes the worker (or schedules its cooperative task).
    pub fn notify(&self) {
        self.bell.ring();
    }

    /// A bell that rings this worker, for other threads and callbacks.
    pub fn bell(&self) -> Bell {
        self.bell.clone()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let signal = &self.bell.signal;
        signal.flags.lock().unwrap_or_else(PoisonError::into_inner).closed = true;
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        signal.ready.notify_all();
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let running = signal.flags.lock().unwrap_or_else(PoisonError::into_inner).scheduled;
            web::close_worker(self.bell.id, running);
        }
    }
}

/// Sets how the cooperative queue asks for a turn later: `ask(d)` should arrange for
/// [`run_pending`] to run after `d` (a browser timer). Timed workers ([`Next::After`]) wait
/// through it in a browser page; natively their threads wait themselves and it is never
/// called.
pub fn set_timer(ask: Arc<dyn Fn(Duration) + Send + Sync>) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    web::set_timer(ask);
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    drop(ask);
}

/// Receives the next message from `rx`, waiting the way the platform can: natively by
/// blocking; in a browser page by running queued cooperative work (which is what sends)
/// until the message arrives. Fails when every sender is gone, or, in a browser page, when
/// no queued work is left that could send (a message only a browser callback would bring
/// cannot be waited for on the page's one thread).
pub fn recv<T>(rx: &Receiver<T>) -> Result<T, RecvError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use std::sync::mpsc::TryRecvError;
        loop {
            match rx.try_recv() {
                Ok(v) => return Ok(v),
                Err(TryRecvError::Disconnected) => return Err(RecvError),
                Err(TryRecvError::Empty) => {
                    if !web::run_one() {
                        return Err(RecvError);
                    }
                }
            }
        }
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        rx.recv()
    }
}

/// As [`recv`], giving up after `timeout`. In a browser page it also gives up (with
/// [`RecvTimeoutError::Timeout`]) when no queued work is left that could send.
pub fn recv_timeout<T>(rx: &Receiver<T>, timeout: Duration) -> Result<T, RecvTimeoutError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use std::sync::mpsc::TryRecvError;
        let start = crate::time::Instant::now();
        loop {
            match rx.try_recv() {
                Ok(v) => return Ok(v),
                Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
                Err(TryRecvError::Empty) => {
                    if start.elapsed() >= timeout || !web::run_one() {
                        return Err(RecvTimeoutError::Timeout);
                    }
                }
            }
        }
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        rx.recv_timeout(timeout)
    }
}

/// Sets how the cooperative queue asks for a turn: `ask` is called (once until the next
/// [`run_pending`]) when work is queued, and should arrange for [`run_pending`] to run soon
/// on the page's thread. Only browser pages queue work; natively work runs on threads and
/// the hook is never called.
pub fn set_scheduler(ask: Arc<dyn Fn() + Send + Sync>) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    web::set_scheduler(ask);
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    drop(ask);
}

/// Runs queued cooperative work until the queue is empty or `budget` has passed; true when
/// work is left (the scheduler has been asked for another turn). Natively nothing is ever
/// queued here, and it returns false at once.
pub fn run_pending(budget: Duration) -> bool {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        web::run_pending(budget)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = budget;
        false
    }
}

/// True when cooperative work is queued (only ever in a browser page).
pub fn has_pending() -> bool {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        web::has_pending()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        false
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web {
    //! The cooperative queue of a browser page. The page has one thread, so the queue lives
    //! in that thread's storage and tasks need not be `Send` (a worker's state may hold
    //! anything).

    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{Bell, Next};

    pub(super) enum Task {
        /// Runs once, whole.
        Once(Box<dyn FnOnce()>),
        /// Called until it returns false, one call per turn of the queue.
        Steps(Box<dyn FnMut() -> bool>),
    }

    type Ask = Arc<dyn Fn() + Send + Sync>;
    type TimerHook = Arc<dyn Fn(Duration) + Send + Sync>;

    /// A worker's step, and whether its handle is gone.
    struct WorkerSlot {
        /// Taken out while it runs.
        step: Option<Box<dyn FnMut() -> Next>>,
        closed: bool,
    }

    /// A worker that asked to run again at a time.
    struct Timed {
        due: crate::time::Instant,
        bell: Bell,
    }

    thread_local! {
        static QUEUE: RefCell<VecDeque<Task>> = const { RefCell::new(VecDeque::new()) };
        static SCHEDULER: RefCell<Option<Ask>> = const { RefCell::new(None) };
        /// The scheduler was asked for a turn that has not come yet.
        static ASKED: Cell<bool> = const { Cell::new(false) };
        static WORKERS: RefCell<HashMap<u64, WorkerSlot>> = RefCell::new(HashMap::new());
        static NEXT_WORKER: Cell<u64> = const { Cell::new(1) };
        static TIMED: RefCell<Vec<Timed>> = const { RefCell::new(Vec::new()) };
        static TIMER: RefCell<Option<TimerHook>> = const { RefCell::new(None) };
    }

    pub(super) fn set_timer(hook: TimerHook) {
        TIMER.set(Some(hook));
    }

    /// Rings `bell` after `d` (the one timer a worker has: a newer one replaces it).
    pub(super) fn ring_after(bell: &Bell, d: Duration) {
        let due = crate::time::Instant::now() + d;
        TIMED.with_borrow_mut(|t| {
            t.retain(|x| x.bell.id != bell.id);
            t.push(Timed { due, bell: bell.clone() });
        });
        if let Some(h) = TIMER.with_borrow(Clone::clone) {
            h(d);
        }
    }

    /// Rings the workers whose time has come.
    fn ring_due() {
        let now = crate::time::Instant::now();
        let due: Vec<Bell> = TIMED.with_borrow_mut(|t| {
            let (due, later): (Vec<Timed>, Vec<Timed>) = t.drain(..).partition(|x| x.due <= now);
            *t = later;
            due.into_iter().map(|x| x.bell).collect()
        });
        for b in due {
            b.ring();
        }
    }

    pub(super) fn next_worker_id() -> u64 {
        let id = NEXT_WORKER.get();
        NEXT_WORKER.set(id + 1);
        id
    }

    fn ask() {
        if ASKED.replace(true) {
            return;
        }
        match SCHEDULER.with_borrow(Clone::clone) {
            Some(h) => h(),
            // Nobody to ask yet: the hook asks when it is installed.
            None => ASKED.set(false),
        }
    }

    pub(super) fn push(t: Task) {
        QUEUE.with_borrow_mut(|q| q.push_back(t));
        ask();
    }

    pub(super) fn set_scheduler(hook: Ask) {
        SCHEDULER.set(Some(hook));
        ASKED.set(false);
        if has_pending() {
            ask();
        }
    }

    pub(super) fn has_pending() -> bool {
        QUEUE.with_borrow(|q| !q.is_empty())
    }

    pub(super) fn add_worker(id: u64, step: Box<dyn FnMut() -> Next>) {
        WORKERS.with_borrow_mut(|w| w.insert(id, WorkerSlot { step: Some(step), closed: false }));
    }

    /// One step of a worker. A closed worker with no work now is removed (and its state
    /// dropped).
    pub(super) fn step_worker(id: u64) -> Next {
        let Some(mut step) = WORKERS.with_borrow_mut(|w| w.get_mut(&id).and_then(|s| s.step.take())) else {
            return Next::Idle;
        };
        let next = step();
        let drop_it = WORKERS.with_borrow_mut(|w| match w.get_mut(&id) {
            Some(slot) if slot.closed && next != Next::Now => w.remove(&id).map(|_| step),
            Some(slot) => {
                slot.step = Some(step);
                None
            }
            None => Some(step),
        });
        // Dropped outside the borrow: the state may drop other workers.
        drop(drop_it);
        next
    }

    /// The worker's handle is gone: it is removed now when idle, or when its running task
    /// finds no more work.
    pub(super) fn close_worker(id: u64, running: bool) {
        let gone = WORKERS.with_borrow_mut(|w| {
            if running {
                if let Some(slot) = w.get_mut(&id) {
                    slot.closed = true;
                }
                None
            } else {
                w.remove(&id)
            }
        });
        drop(gone);
    }

    /// Runs the task at the front of the queue once; false when the queue was empty.
    pub(super) fn run_one() -> bool {
        // The queue is not borrowed while the task runs: tasks queue more work.
        let Some(task) = QUEUE.with_borrow_mut(VecDeque::pop_front) else { return false };
        match task {
            Task::Once(f) => f(),
            Task::Steps(mut f) => {
                if f() {
                    QUEUE.with_borrow_mut(|q| q.push_back(Task::Steps(f)));
                }
            }
        }
        true
    }

    pub(super) fn run_pending(budget: Duration) -> bool {
        ASKED.set(false);
        ring_due();
        let start = crate::time::Instant::now();
        while run_one() {
            if start.elapsed() >= budget {
                break;
            }
        }
        let more = has_pending();
        if more {
            ask();
        }
        more
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::mpsc::channel;

    #[test]
    fn spawned_work_runs_and_reports_back() {
        let (tx, rx) = channel();
        let started = spawn("remedy-test", move || {
            let _ = tx.send(42);
        });
        assert_eq!(started, Ok(()));
        assert_eq!(recv(&rx), Ok(42));
        assert!(threads());
        assert!(parallelism() >= 1);
    }

    #[test]
    fn a_pool_drains_its_queue_and_keeps_order_per_worker() {
        let queue: Arc<Mutex<VecDeque<u32>>> = Arc::default();
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let q = queue.clone();
        let pool = Pool::new("remedy-test-pool", 1, move || {
            let Some(n) = q.lock().unwrap_or_else(PoisonError::into_inner).pop_front() else { return false };
            let _ = tx.lock().unwrap_or_else(PoisonError::into_inner).send(n);
            true
        });
        for n in 0..100 {
            queue.lock().unwrap_or_else(PoisonError::into_inner).push_back(n);
            pool.notify();
        }
        let got: Vec<u32> =
            (0..100).filter_map(|_| recv_timeout(&rx, Duration::from_secs(10)).ok()).collect();
        assert_eq!(got, (0..100).collect::<Vec<_>>(), "one worker keeps the queue's order");
        drop(pool);
        assert!(!has_pending(), "nothing is queued cooperatively on native platforms");
        assert!(!run_pending(Duration::from_millis(1)));
    }

    #[test]
    fn several_workers_share_a_queue() {
        let queue: Arc<Mutex<Vec<u32>>> = Arc::default();
        let done: Arc<Mutex<Vec<u32>>> = Arc::default();
        let (q, d) = (queue.clone(), done.clone());
        let pool = Pool::new("remedy-test-pool", 4, move || {
            let Some(n) = q.lock().unwrap_or_else(PoisonError::into_inner).pop() else { return false };
            d.lock().unwrap_or_else(PoisonError::into_inner).push(n);
            true
        });
        for n in 0..1000 {
            queue.lock().unwrap_or_else(PoisonError::into_inner).push(n);
            pool.notify();
        }
        let start = crate::time::Instant::now();
        while done.lock().unwrap_or_else(PoisonError::into_inner).len() < 1000 {
            assert!(start.elapsed() < Duration::from_secs(10), "the pool finished its queue");
            std::thread::yield_now();
        }
        let mut all = done.lock().unwrap_or_else(PoisonError::into_inner).clone();
        all.sort_unstable();
        assert_eq!(all, (0..1000).collect::<Vec<_>>());
    }

    #[test]
    fn a_timed_worker_wakes_itself_and_a_bell_wakes_it_sooner() {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let start = crate::time::Instant::now();
        // Three turns 20 ms apart on its own, then idle until rung.
        let worker = Worker::spawn_timed(
            "remedy-test-timed",
            |_bell| 0u32,
            move |n| {
                *n += 1;
                let _ = tx.lock().unwrap_or_else(PoisonError::into_inner).send(*n);
                if *n < 3 { Next::After(Duration::from_millis(20)) } else { Next::Idle }
            },
        );
        let Ok(worker) = worker else { unreachable!("the worker starts") };
        let turns: Vec<u32> =
            (0..3).filter_map(|_| recv_timeout(&rx, Duration::from_secs(10)).ok()).collect();
        assert_eq!(turns, [1, 2, 3]);
        assert!(start.elapsed() >= Duration::from_millis(40), "it waited between turns");
        assert!(recv_timeout(&rx, Duration::from_millis(100)).is_err(), "idle until rung");
        worker.bell().ring();
        assert_eq!(recv_timeout(&rx, Duration::from_secs(10)), Ok(4), "a bell wakes it");
    }
}
