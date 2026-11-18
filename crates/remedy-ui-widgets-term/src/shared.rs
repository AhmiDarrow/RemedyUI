//! The emulator shared between the thread feeding it and the UI thread showing it.
//!
//! The app reads its PTY on a thread of its own and hands the bytes to a [`Feeder`]. Parsing
//! happens right there, on the feeding thread, in slices short enough that the UI thread
//! never waits long for the lock. The feeder then wakes the UI thread (once, however many
//! slices arrive before it runs), which takes only what changed: the rows that changed
//! under the cursor, how far the scrollback moved, replies, the title.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use remedy_ui_reactive::{Signal, Waker};

use crate::term::Emulator;

/// How much input is parsed per hold of the lock: a tuning constant. Parsing 16 KiB takes
/// a fraction of a millisecond, so the UI thread never waits longer than that to read the
/// screen.
const FEED_SLICE: usize = 16 * 1024;

/// What a feed does to reach the UI thread: the waker, and the signal its update reads.
type Notify = (Waker, Signal<u64>);

pub(crate) struct Shared {
    pub(crate) emu: Mutex<Emulator>,
    /// A wake-up is on its way to the UI thread.
    posted: AtomicBool,
    notify: Mutex<Option<Notify>>,
    /// The UI thread is waiting for the lock: the feeder lets it in before taking the next
    /// slice (a plain mutex would let a busy feeder take it again and again first).
    ui_waiting: AtomicUsize,
}

/// Locks a mutex. The emulator's state stays whole if a feeding thread panicked mid-parse
/// (each action leaves the screen consistent), so a poisoned lock is used as it is.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub(crate) fn new(emu: Emulator, notify: Option<Notify>) -> Arc<Shared> {
        Arc::new(Shared {
            emu: Mutex::new(emu),
            posted: AtomicBool::new(false),
            notify: Mutex::new(notify),
            ui_waiting: AtomicUsize::new(0),
        })
    }

    /// The emulator, for the UI thread: it goes ahead of the feeder's next slice.
    pub(crate) fn ui_lock(&self) -> MutexGuard<'_, Emulator> {
        self.ui_waiting.fetch_add(1, Ordering::AcqRel);
        let g = lock(&self.emu);
        self.ui_waiting.fetch_sub(1, Ordering::AcqRel);
        g
    }

    /// The emulator, for a feeding thread: after the UI thread, when it is waiting.
    fn feeder_lock(&self) -> MutexGuard<'_, Emulator> {
        while self.ui_waiting.load(Ordering::Acquire) > 0 {
            std::thread::yield_now();
        }
        lock(&self.emu)
    }

    /// Wakes the UI thread to take what changed, unless a wake-up is already on its way.
    pub(crate) fn wake(self: &Arc<Self>) {
        if self.posted.swap(true, Ordering::AcqRel) {
            return;
        }
        let notify = lock(&self.notify).clone();
        let Some((waker, tick)) = notify else {
            self.posted.store(false, Ordering::Release);
            return;
        };
        let me = self.clone();
        let posted = waker.post(move || {
            me.posted.store(false, Ordering::Release);
            let _ = tick.try_update(|t| *t = t.wrapping_add(1));
        });
        if posted.is_err() {
            // The UI thread has shut down: nothing will show this.
            self.posted.store(false, Ordering::Release);
        }
    }
}

/// Feeds a terminal the program's output from any thread. `Send`, `Sync` and cheap to clone.
///
/// ```no_run
/// # fn demo(term: &remedy_ui_widgets_term::Terminal, mut pty: impl std::io::Read + Send + 'static) {
/// let feeder = term.feeder();
/// std::thread::spawn(move || {
///     let mut buf = [0u8; 65536];
///     while let Ok(n) = pty.read(&mut buf) {
///         if n == 0 {
///             break;
///         }
///         feeder.feed(buf.get(..n).unwrap_or_default());
///     }
/// });
/// # }
/// ```
#[derive(Clone)]
pub struct Feeder {
    pub(crate) shared: Arc<Shared>,
}

impl std::fmt::Debug for Feeder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feeder").finish_non_exhaustive()
    }
}

impl Feeder {
    /// Parses `bytes` from the program into the screen, here on the calling thread, and wakes
    /// the UI thread to show the change.
    pub fn feed(&self, bytes: &[u8]) {
        for slice in bytes.chunks(FEED_SLICE) {
            self.shared.feeder_lock().feed(slice);
        }
        self.shared.wake();
    }
}
