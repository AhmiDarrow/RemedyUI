//! Reaching the UI thread from other threads.
//!
//! Network, audio, decoder and file threads never touch signals directly. They hold a
//! [`Waker`] and post closures; the UI thread runs everything posted since its last frame in
//! one batch, so many posts cost one frame.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::error::ReactiveError;
use crate::runtime::with_rt;

type Post = Box<dyn FnOnce() + Send>;
type WakeFn = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub(crate) struct PostQueue {
    queue: Mutex<VecDeque<Post>>,
    wake: Mutex<Option<WakeFn>>,
    closed: AtomicBool,
}

/// Locks a mutex. A panic while holding these locks cannot leave the data half-changed
/// (pushes and swaps only), so a poisoned lock is used as it is.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PostQueue {
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        lock(&self.queue).clear();
    }
}

/// The UI thread's runtime has ended; the post was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed;

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the UI thread this waker posts to has shut down")
    }
}

impl std::error::Error for Closed {}

/// A handle any thread can use to run closures on a UI thread. `Send`, `Sync` and cheap to
/// clone.
#[derive(Clone)]
pub struct Waker {
    queue: Arc<PostQueue>,
}

impl std::fmt::Debug for Waker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waker").field("closed", &self.is_closed()).finish()
    }
}

impl Waker {
    /// Queues `f` to run on the UI thread in the next frame, and wakes the UI thread if it
    /// was idle. Posts from one thread run in the order they were posted.
    pub fn post(&self, f: impl FnOnce() + Send + 'static) -> Result<(), Closed> {
        if self.is_closed() {
            return Err(Closed);
        }
        let was_empty = {
            let mut q = lock(&self.queue.queue);
            let was_empty = q.is_empty();
            q.push_back(Box::new(f));
            was_empty
        };
        if was_empty {
            let wake = lock(&self.queue.wake).clone();
            if let Some(wake) = wake {
                wake();
            }
        }
        Ok(())
    }

    /// True once the UI thread's runtime has ended.
    pub fn is_closed(&self) -> bool {
        self.queue.closed.load(Ordering::Acquire)
    }
}

/// A waker for this thread's runtime.
pub fn waker() -> Result<Waker, ReactiveError> {
    with_rt(|rt| Waker { queue: rt.posts.clone() })
}

/// Sets what a post does to wake this thread (a bridge sets this to wake its event loop).
/// It is called from the posting thread, only when the queue goes from empty to non-empty.
pub fn set_wake_handler(f: impl Fn() + Send + Sync + 'static) -> Result<(), ReactiveError> {
    with_rt(|rt| *lock(&rt.posts.wake) = Some(Arc::new(f)))
}

/// True when posts are waiting.
pub fn has_posted() -> bool {
    with_rt(|rt| !lock(&rt.posts.queue).is_empty()).unwrap_or(false)
}

/// What [`run_posted`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PostReport {
    /// Closures that ran to completion.
    pub ran: usize,
    /// Closures that panicked. Each panic was contained; the rest still ran.
    pub panicked: usize,
}

/// Runs every closure posted so far, in one batch. A panicking closure is contained and
/// counted; the others still run. Closures posted while these run wait for the next call.
pub fn run_posted() -> PostReport {
    let posts: Vec<Post> = match with_rt(|rt| std::mem::take(&mut *lock(&rt.posts.queue))) {
        Ok(q) => q.into_iter().collect(),
        Err(_) => return PostReport::default(),
    };
    let mut report = PostReport::default();
    crate::batch(|| {
        for post in posts {
            match catch_unwind(AssertUnwindSafe(post)) {
                Ok(()) => report.ran += 1,
                Err(_) => report.panicked += 1,
            }
        }
    });
    report
}
