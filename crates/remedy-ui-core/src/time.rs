//! Clocks that work on every platform the engine runs on.
//!
//! `std::time::Instant::now()` and `SystemTime::now()` panic on `wasm32-unknown-unknown`,
//! where the browser owns the clocks. The engine reads time through these types instead:
//! the standard library's own on native platforms, and the page's monotonic clock
//! (`performance.now()`) and wall clock (`Date.now()`) in a browser, with the same API.

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// The processor time the calling thread has used: what a time budget for work on this thread
/// (a script's call, a calibration run) should count, because it leaves out the time the
/// system gave other threads and processes. Measured against the wall clock, a busy machine
/// would make fair work look slow and stop it.
///
/// Off the web it reads the system's per-thread clock (`GetThreadTimes` on Windows,
/// `CLOCK_THREAD_CPUTIME_ID` elsewhere). A browser page has no such clock, so there it is the
/// page's monotonic clock, which is what a page's single thread can know; and wherever the
/// system refuses the thread clock it falls back to the monotonic clock too, saying so with
/// [`ThreadTime::is_processor_time`].
#[derive(Clone, Copy, Debug)]
pub struct ThreadTime {
    at: ThreadReading,
}

#[derive(Clone, Copy, Debug)]
enum ThreadReading {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    Processor(std::time::Duration),
    Wall(Instant),
}

impl ThreadTime {
    /// Now, on the calling thread's clock.
    pub fn now() -> ThreadTime {
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        if let Ok(t) = cpu_time::ThreadTime::try_now() {
            return ThreadTime { at: ThreadReading::Processor(t.as_duration()) };
        }
        ThreadTime { at: ThreadReading::Wall(Instant::now()) }
    }

    /// The thread's time since `self` was read (on the same thread).
    pub fn elapsed(&self) -> std::time::Duration {
        match self.at {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            ThreadReading::Processor(then) => match cpu_time::ThreadTime::try_now() {
                Ok(now) => now.as_duration().saturating_sub(then),
                // The thread clock failed after working: count nothing rather than guess.
                Err(_) => std::time::Duration::ZERO,
            },
            ThreadReading::Wall(then) => then.elapsed(),
        }
    }

    /// How finely the thread clock moves on this system: about 15.6 ms on Windows, a
    /// microsecond or less elsewhere, the monotonic clock's step where there is no thread
    /// clock. Measured once per process by watching the clock tick while this thread works
    /// (up to a couple of ticks of processor time), so call it away from the UI thread the
    /// first time; [`resolution_if_known`](Self::resolution_if_known) never measures.
    pub fn resolution() -> std::time::Duration {
        *RESOLUTION.get_or_init(measure_resolution)
    }

    /// The thread clock's resolution if it was measured already.
    pub fn resolution_if_known() -> Option<std::time::Duration> {
        RESOLUTION.get().copied()
    }

    /// Whether this reading is the thread's processor time (rather than the monotonic clock
    /// a browser page or a refusing system leaves).
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub fn is_processor_time(&self) -> bool {
        matches!(self.at, ThreadReading::Processor(_))
    }

    /// Whether this reading is the thread's processor time: never in a browser page, which
    /// has no thread clock.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn is_processor_time(&self) -> bool {
        false
    }
}

static RESOLUTION: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();

/// Watches the thread clock change while this thread works: the smallest step it takes, over
/// three steps, is its resolution. Bounded by the monotonic clock: a clock that does not move
/// within a quarter of a second of work is treated as moving in steps that long.
fn measure_resolution() -> std::time::Duration {
    let first = ThreadTime::now();
    if !first.is_processor_time() {
        return std::time::Duration::from_micros(1);
    }
    let limit = Instant::now() + std::time::Duration::from_millis(250);
    let mut last = first.elapsed();
    let mut smallest: Option<std::time::Duration> = None;
    let mut steps = 0;
    let mut x = 1u64;
    while steps < 3 && Instant::now() < limit {
        x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
        let now = first.elapsed();
        if now > last {
            let step = now - last;
            smallest = Some(smallest.map_or(step, |s| s.min(step)));
            last = now;
            steps += 1;
        }
    }
    smallest.unwrap_or(std::time::Duration::from_millis(250))
}

/// Times work done on the calling thread, fairly on a busy machine and on a coarse thread
/// clock: it reads both the thread's processor time ([`ThreadTime`]) and the monotonic clock.
/// Each is wrong in one direction only: time the system gives other work inflates the
/// monotonic reading, and a thread clock that ticks coarsely (about 15.6 ms on Windows)
/// inflates the processor reading when a tick lands. So work is [`over`](Self::over) a budget
/// only when both say so, and the work done is the smaller of the two.
///
/// The system charges a whole step of the thread clock to the thread that is running when
/// the clock ticks, however briefly it ran, and on a busy machine that is also the moment the
/// thread is set aside. So a limit ([`over`](Self::over), [`proven`](Self::proven)) leaves one
/// step out of the thread reading: a reading of one step is no proof of work, and a reading of
/// two is a step's worth. A measurement ([`work`](Self::work)) keeps the reading as it is.
#[derive(Clone, Copy, Debug)]
pub struct WorkClock {
    thread: ThreadTime,
    wall: Instant,
}

/// One step of the thread clock: its measured resolution, or the system's usual tick (15.625 ms
/// on Windows, nothing to speak of elsewhere) until it is measured.
fn thread_step() -> std::time::Duration {
    let unmeasured = if cfg!(windows) { 15_625 } else { 0 };
    ThreadTime::resolution_if_known().unwrap_or(std::time::Duration::from_micros(unmeasured))
}

impl WorkClock {
    /// The processor time this thread is known to have used since the start.
    fn thread_work(&self) -> std::time::Duration {
        self.thread.elapsed().saturating_sub(thread_step())
    }

    /// Starts timing, now, on this thread.
    pub fn start() -> WorkClock {
        WorkClock { thread: ThreadTime::now(), wall: Instant::now() }
    }

    /// The work done since the start: the smaller of the thread's processor time and the
    /// monotonic time (just the monotonic time where there is no thread clock).
    pub fn work(&self) -> std::time::Duration {
        let wall = self.wall.elapsed();
        if self.thread.is_processor_time() { self.thread.elapsed().min(wall) } else { wall }
    }

    /// The work certainly done since the start, for holding work to a limit: as
    /// [`work`](Self::work), less the one step a thread reading can overstate by.
    pub fn proven(&self) -> std::time::Duration {
        let wall = self.wall.elapsed();
        if self.thread.is_processor_time() { self.thread_work().min(wall) } else { wall }
    }

    /// Whether the work since the start has gone past `budget`: both clocks past it (just the
    /// monotonic clock where there is no thread clock).
    pub fn over(&self, budget: std::time::Duration) -> bool {
        let wall = self.wall.elapsed();
        if wall <= budget {
            return false;
        }
        !self.thread.is_processor_time() || self.thread_work() > budget
    }
}

/// Seconds since 1970-01-01T00:00:00Z, or 0 when the clock reads before it.
pub fn unix_seconds() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => -(i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thread_clock_counts_this_threads_work_not_its_sleep() {
        let t = ThreadTime::now();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let slept = t.elapsed();
        if t.is_processor_time() {
            assert!(
                slept < std::time::Duration::from_millis(15),
                "sleeping uses no processor time: {slept:?}"
            );
        }
        let t = ThreadTime::now();
        let start = Instant::now();
        let mut x = 0u64;
        while start.elapsed() < std::time::Duration::from_millis(30) {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        }
        std::hint::black_box(x);
        assert!(t.elapsed() >= std::time::Duration::from_millis(5), "working uses it: {:?}", t.elapsed());
    }

    #[test]
    fn the_thread_clock_knows_its_resolution() {
        let r = ThreadTime::resolution();
        assert!(r > std::time::Duration::ZERO && r <= std::time::Duration::from_millis(250), "{r:?}");
        assert_eq!(ThreadTime::resolution_if_known(), Some(r));
    }

    #[test]
    fn work_is_over_only_when_both_clocks_agree() {
        let c = WorkClock::start();
        std::thread::sleep(std::time::Duration::from_millis(30));
        if ThreadTime::now().is_processor_time() {
            assert!(!c.over(std::time::Duration::from_millis(10)), "sleeping is no work");
            assert!(c.work() < std::time::Duration::from_millis(16), "{:?}", c.work());
        }
        let c = WorkClock::start();
        let start = Instant::now();
        let mut x = 0u64;
        while start.elapsed() < std::time::Duration::from_millis(60) {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        }
        std::hint::black_box(x);
        assert!(c.over(std::time::Duration::from_millis(5)), "working past a budget is over it");
        assert!(c.work() <= start.elapsed() + std::time::Duration::from_millis(1));
        assert!(c.proven() <= c.work(), "a limit never counts more than was measured");
    }

    #[test]
    fn clocks_move_forward() {
        let a = Instant::now();
        let b = Instant::now();
        assert!(b >= a);
        assert!(unix_seconds() > 1_700_000_000, "the wall clock reads after 2023");
    }
}
