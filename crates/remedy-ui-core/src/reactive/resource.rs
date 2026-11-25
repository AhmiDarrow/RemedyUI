//! Data that arrives later: a [`Resource`] fetched off the UI thread, the [`Executor`] that
//! runs fetches (the [`WorkerPool`] by default), and the [`Suspense`] registry boundaries use
//! to show a placeholder while what is read inside them still loads.
//!
//! A resource follows a key: when the key changes it cancels the fetch under way and starts
//! another, and when its owner is disposed it cancels for good. Results come back through
//! the thread's [`crate::reactive::Waker`], so they land in one batch with the frame's other posts.
//! RemedyUI depends on no async runtime; an app with one implements [`Executor`] for it and
//! provides it as context.
//!
//! ```
//! use remedy_ui_core::reactive::{Resource, Signal, run_posted, untrack};
//!
//! let user = Signal::new(7u32);
//! let name: Resource<String, String> = Resource::new(move || user.get(), |id, _cancel| Ok(format!("user {id}")));
//! assert!(untrack(|| name.loading()));
//! // The fetch runs on a worker; its result is posted back to this thread.
//! while untrack(|| name.loading()) {
//!     run_posted();
//!     std::thread::yield_now();
//! }
//! assert_eq!(untrack(|| name.get()), Some("user 7".to_string()));
//! ```

use std::cell::RefCell;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::reactive::context::{panic_message, use_context};
use crate::reactive::signal::{Effect, Memo, Signal, batch, on_cleanup, untrack};

/// Work for an [`Executor`].
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// Runs jobs off the UI thread. The default is the shared [`WorkerPool`]; an app with its
/// own async runtime implements this for it and provides it as context
/// (`provide_context::<Arc<dyn Executor>>(…)`), and resources created under that context
/// fetch on it.
pub trait Executor: Send + Sync + 'static {
    /// Runs `job` soon, on some other thread.
    fn spawn(&self, job: Job);
}

/// Locks a mutex; the data behind these locks is a queue that a panic cannot leave
/// half-changed, so a poisoned lock is used as it is.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A pool of worker threads, started as work arrives, up to one per processor the machine
/// has (or the number asked for); in a browser page without threads, one cooperative task
/// that runs jobs between frames (`crate::exec`). Jobs run oldest first; a job
/// that panics ends only itself. Cloning gives another handle to the same pool; its workers
/// finish what is queued and leave when the last handle is dropped.
#[derive(Clone)]
pub struct WorkerPool {
    queue: Arc<Mutex<VecDeque<Job>>>,
    pool: Arc<crate::exec::Pool>,
    size: usize,
}

impl std::fmt::Debug for WorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerPool").field("size", &self.size).field("queued", &self.queued()).finish()
    }
}

impl WorkerPool {
    /// A pool of up to `threads` workers (at least one).
    pub fn new(threads: usize) -> Self {
        let queue: Arc<Mutex<VecDeque<Job>>> = Arc::default();
        let q = queue.clone();
        let step = move || {
            let job = lock(&q).pop_front();
            match job {
                Some(job) => {
                    // A panicking job ends itself; the worker goes on with the next.
                    let _ = catch_unwind(AssertUnwindSafe(job));
                    true
                }
                None => false,
            }
        };
        let size = threads.max(1);
        WorkerPool { queue, pool: Arc::new(crate::exec::Pool::new("remedy-worker", size, step)), size }
    }

    /// The process's shared pool, sized from the machine's processors.
    pub fn shared() -> WorkerPool {
        static SHARED: OnceLock<WorkerPool> = OnceLock::new();
        SHARED.get_or_init(|| WorkerPool::new(crate::exec::parallelism())).clone()
    }

    /// The most workers it runs at once.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Jobs waiting for a worker.
    pub fn queued(&self) -> usize {
        lock(&self.queue).len()
    }
}

impl Executor for WorkerPool {
    fn spawn(&self, job: Job) {
        lock(&self.queue).push_back(job);
        self.pool.notify();
    }
}

/// Tells a fetch it is no longer wanted: its key changed or its resource was disposed.
/// Long fetches check it and stop early; a result delivered after cancelling is dropped.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A token not cancelled yet.
    pub fn new() -> Self {
        Cancel::default()
    }

    /// True once the fetch is no longer wanted.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Cancels it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Why a fetch gave no value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchError<E> {
    /// The fetch returned this error.
    Failed(E),
    /// The fetch panicked, with this message.
    Panicked(String),
    /// The UI thread was gone, or the executor dropped the job without running it.
    Lost,
}

impl<E: std::fmt::Display> std::fmt::Display for FetchError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Failed(e) => e.fmt(f),
            FetchError::Panicked(m) => write!(f, "the fetch failed unexpectedly: {m}"),
            FetchError::Lost => f.write_str("the fetch was lost before it finished"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for FetchError<E> {}

/// What a [`Suspense`] boundary watches: whether resources read inside it are loading.
/// Resources register themselves when read under one (through context); the boundary reads
/// [`Suspense::pending`].
#[derive(Clone)]
pub struct Suspense {
    entries: Rc<RefCell<Registered>>,
    version: Signal<u64>,
}

impl std::fmt::Debug for Suspense {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Suspense({} resources)", self.entries.borrow().len())
    }
}

impl Suspense {
    /// An empty registry, owned by the current scope.
    #[track_caller]
    pub fn new() -> Self {
        Suspense { entries: Rc::default(), version: Signal::new(0) }
    }

    /// Adds a resource's loading flag (once per resource).
    pub fn register(&self, id: u64, loading: Signal<bool>) {
        let added = {
            let mut e = self.entries.borrow_mut();
            if e.iter().any(|(i, _)| *i == id) {
                false
            } else {
                e.push((id, loading));
                true
            }
        };
        if added {
            let _ = self.version.try_update(|v| *v += 1);
        }
    }

    /// True while any registered resource is loading; reading it subscribes to every
    /// registered resource and to new registrations. Disposed resources are dropped.
    pub fn pending(&self) -> bool {
        let _ = self.version.try_with(|_| ());
        let entries: Vec<(u64, Signal<bool>)> = self.entries.borrow().clone();
        let mut any = false;
        let mut dead = Vec::new();
        for (id, s) in entries {
            match s.try_get() {
                Ok(true) => any = true,
                Ok(false) => {}
                Err(_) => dead.push(id),
            }
        }
        if !dead.is_empty() {
            self.entries.borrow_mut().retain(|(i, _)| !dead.contains(i));
        }
        any
    }

    /// How many resources are registered.
    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    /// True when no resource is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }
}

impl Default for Suspense {
    fn default() -> Self {
        Suspense::new()
    }
}

/// Resources a [`Suspense`] watches: id and loading flag.
type Registered = Vec<(u64, Signal<bool>)>;

static NEXT_RESOURCE: AtomicU64 = AtomicU64::new(1);

/// Data fetched off the UI thread for a key. While a fetch runs, [`Resource::loading`] is
/// true and the last value stays readable; when it ends, the value or the error replaces
/// it. Reading any of them inside a [`Suspense`] boundary registers the resource there.
pub struct Resource<T, E> {
    id: u64,
    value: Signal<Option<T>>,
    error: Signal<Option<FetchError<E>>>,
    loading: Signal<bool>,
    again: Signal<u64>,
}

impl<T, E> Clone for Resource<T, E> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, E> Copy for Resource<T, E> {}

impl<T, E> std::fmt::Debug for Resource<T, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Resource({})", self.id)
    }
}

impl<T: Send + 'static, E: Send + 'static> Resource<T, E> {
    /// A resource fetching `fetch(key)` for the key `source` returns, on the executor
    /// provided as context (`Arc<dyn Executor>`) or the shared [`WorkerPool`]. A new key
    /// cancels the fetch under way; so does disposing the current owner.
    #[track_caller]
    pub fn new<K>(
        source: impl Fn() -> K + 'static,
        fetch: impl Fn(K, &Cancel) -> Result<T, E> + Send + Sync + 'static,
    ) -> Self
    where
        K: PartialEq + Clone + Send + 'static,
    {
        let executor: Arc<dyn Executor> =
            use_context::<Arc<dyn Executor>>().unwrap_or_else(|| Arc::new(WorkerPool::shared()));
        Self::with_executor(executor, source, fetch)
    }

    /// Like [`Resource::new`], fetching on `executor`.
    #[track_caller]
    pub fn with_executor<K>(
        executor: Arc<dyn Executor>,
        source: impl Fn() -> K + 'static,
        fetch: impl Fn(K, &Cancel) -> Result<T, E> + Send + Sync + 'static,
    ) -> Self
    where
        K: PartialEq + Clone + Send + 'static,
    {
        let fetch = Arc::new(fetch);
        Self::with_reply(source, move |k, cancel, reply| {
            let fetch = fetch.clone();
            executor.spawn(Box::new(move || {
                let mut deliver = reply.0;
                if cancel.is_cancelled() {
                    deliver.done = true;
                    return;
                }
                let out = match catch_unwind(AssertUnwindSafe(|| fetch(k, &cancel))) {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(FetchError::Failed(e)),
                    Err(p) => Err(FetchError::Panicked(panic_message(p.as_ref()))),
                };
                deliver.send(out);
            }));
        })
    }

    /// A resource whose fetch answers later instead of blocking a worker: `start(key, cancel,
    /// reply)` runs on the UI thread when the key changes, begins the work (a browser
    /// `fetch`, a request on a thread of its own) and returns; the work calls
    /// [`Reply::send`] from any thread when it ends. A reply dropped unsent reports the
    /// fetch lost. This is how fetches run in a browser page, where nothing may block.
    #[track_caller]
    pub fn with_reply<K>(
        source: impl Fn() -> K + 'static,
        start: impl Fn(K, Cancel, Reply<T, E>) + 'static,
    ) -> Self
    where
        K: PartialEq + Clone + 'static,
    {
        let r = Resource {
            id: NEXT_RESOURCE.fetch_add(1, Ordering::Relaxed),
            value: Signal::new(None),
            error: Signal::new(None),
            loading: Signal::new(true),
            again: Signal::new(0),
        };
        let key = Memo::new(source);
        let generation = Arc::new(AtomicU64::new(0));
        let current: Rc<RefCell<Option<Cancel>>> = Rc::default();
        let waker = crate::reactive::waker().ok();
        let cur2 = current.clone();
        let again = r.again;
        Effect::new(move || {
            let k = key.get();
            let _ = again.try_with(|_| ());
            untrack(|| {
                if let Some(c) = cur2.borrow_mut().take() {
                    c.cancel();
                }
                let cancel = Cancel::new();
                *cur2.borrow_mut() = Some(cancel.clone());
                let gen_now = generation.fetch_add(1, Ordering::AcqRel) + 1;
                batch(|| {
                    let _ = r.loading.try_set(true);
                });
                let Some(waker) = waker.clone() else {
                    let _ = r.error.try_set(Some(FetchError::Lost));
                    let _ = r.loading.try_set(false);
                    return;
                };
                let (value, error, loading) = (r.value, r.error, r.loading);
                // Delivered on the UI thread, even when the work drops it without answering.
                let deliver = Deliver {
                    waker,
                    generation: generation.clone(),
                    gen_now,
                    cancel: cancel.clone(),
                    value,
                    error,
                    loading,
                    done: false,
                };
                start(k, cancel, Reply(deliver));
            });
        });
        on_cleanup(move || {
            if let Some(c) = current.borrow_mut().take() {
                c.cancel();
            }
        });
        r
    }
}

/// The answer to one fetch of a [`Resource::with_reply`] resource, sent once from any
/// thread. Dropped unsent, it reports the fetch lost; sent after a newer key or a disposal,
/// it is ignored.
pub struct Reply<T: Send + 'static, E: Send + 'static>(Deliver<T, E>);

impl<T: Send + 'static, E: Send + 'static> Reply<T, E> {
    /// Delivers the fetch's value or error to the resource, on the UI thread.
    pub fn send(mut self, out: Result<T, E>) {
        self.0.send(out.map_err(FetchError::Failed));
    }
}

impl<T: Send + 'static, E: Send + 'static> std::fmt::Debug for Reply<T, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Reply({})", self.0.gen_now)
    }
}

/// Carries a fetch's result back to the UI thread; dropped unsent (the executor dropped
/// the job), it reports the fetch lost so nothing stays loading forever.
struct Deliver<T: Send + 'static, E: Send + 'static> {
    waker: crate::reactive::Waker,
    generation: Arc<AtomicU64>,
    gen_now: u64,
    cancel: Cancel,
    value: Signal<Option<T>>,
    error: Signal<Option<FetchError<E>>>,
    loading: Signal<bool>,
    done: bool,
}

impl<T: Send + 'static, E: Send + 'static> Deliver<T, E> {
    fn send(&mut self, out: Result<T, FetchError<E>>) {
        self.done = true;
        let (generation, gen_now, cancel) = (self.generation.clone(), self.gen_now, self.cancel.clone());
        let (value, error, loading) = (self.value, self.error, self.loading);
        let _ = self.waker.post(move || {
            // A newer fetch or a disposal made this one stale.
            if cancel.is_cancelled() || generation.load(Ordering::Acquire) != gen_now {
                return;
            }
            batch(|| match out {
                Ok(v) => {
                    let _ = value.try_set(Some(v));
                    let _ = error.try_set(None);
                    let _ = loading.try_set(false);
                }
                Err(e) => {
                    let _ = error.try_set(Some(e));
                    let _ = loading.try_set(false);
                }
            });
        });
    }
}

impl<T: Send + 'static, E: Send + 'static> Drop for Deliver<T, E> {
    fn drop(&mut self) {
        if !self.done {
            self.send(Err(FetchError::Lost));
        }
    }
}

impl<T: 'static, E: 'static> Resource<T, E> {
    fn register(&self) {
        if let Some(s) = use_context::<Suspense>() {
            s.register(self.id, self.loading);
        }
    }

    /// True while a fetch runs (subscribing the current observer).
    pub fn loading(&self) -> bool {
        self.register();
        self.loading.try_get().unwrap_or(false)
    }

    /// True once a value has arrived (it stays while a newer key loads).
    pub fn ready(&self) -> bool {
        self.register();
        self.value.try_with(Option::is_some).unwrap_or(false)
    }

    /// The last value fetched, if any (subscribing the current observer).
    pub fn get(&self) -> Option<T>
    where
        T: Clone,
    {
        self.register();
        self.value.try_get().ok().flatten()
    }

    /// Runs `f` with the last value fetched.
    pub fn with<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> Option<R> {
        self.register();
        self.value.try_with(|v| f(v.as_ref())).ok()
    }

    /// Why the last fetch gave no value, if it did not.
    pub fn error(&self) -> Option<FetchError<E>>
    where
        E: Clone,
    {
        self.register();
        self.error.try_get().ok().flatten()
    }

    /// Fetches the current key again.
    pub fn refetch(&self) {
        let _ = self.again.try_update(|n| *n += 1);
    }

    /// The loading flag as a signal.
    pub fn loading_signal(&self) -> Signal<bool> {
        self.loading
    }

    /// The value as a signal.
    pub fn value_signal(&self) -> Signal<Option<T>> {
        self.value
    }

    /// The error as a signal.
    pub fn error_signal(&self) -> Signal<Option<FetchError<E>>> {
        self.error
    }

    /// The resource's id, unique in the process.
    pub fn id(&self) -> u64 {
        self.id
    }
}
