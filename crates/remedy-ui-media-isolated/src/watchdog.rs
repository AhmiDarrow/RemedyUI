//! One thread that stops helpers past their time (and, where the system cannot limit a
//! process's memory, past their memory). A request arms a watch before it sends and disarms
//! it after the reply; stopping happens under the same lock as disarming, so once a watch is
//! disarmed its helper is never stopped by it (a reaped process id is never signalled).
//!
//! Time is the helper's processor time, so a busy machine that leaves a decoder waiting for
//! a turn never gets it stopped; a wall-clock deadline far beyond that stops a helper that
//! blocks instead of working.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::os::Killer;

/// What a watch did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fired {
    No,
    /// The helper ran past its time and was stopped.
    Time,
    /// It used more memory than allowed and was stopped.
    Memory,
}

/// What a watch enforces.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Limits {
    /// The helper's total processor time past which it is stopped, and how often to look.
    pub cpu: Option<(Duration, Duration)>,
    /// When it is stopped whatever it is doing.
    pub deadline: Option<Instant>,
    /// Its memory past which it is stopped (where the system does not enforce a limit).
    pub memory: Option<u64>,
}

struct Watch {
    limits: Limits,
    killer: Killer,
    fired: Arc<AtomicU8>,
}

#[derive(Default)]
struct State {
    next: u64,
    watches: HashMap<u64, Watch>,
    started: bool,
}

struct Dog {
    state: Mutex<State>,
    changed: Condvar,
}

fn dog() -> &'static Dog {
    static D: OnceLock<Dog> = OnceLock::new();
    D.get_or_init(|| Dog { state: Mutex::new(State::default()), changed: Condvar::new() })
}

/// How often memory is looked at while a polled watch is armed: often enough that a helper
/// allocating as fast as memory can be written gets little past its limit.
const MEMORY_POLL: Duration = Duration::from_millis(1);

fn check(w: &Watch, now: Instant) -> u8 {
    let l = &w.limits;
    if l.deadline.is_some_and(|t| t <= now) {
        return 1;
    }
    if let Some((limit, _)) = l.cpu
        && w.killer.cpu_time().is_some_and(|t| t > limit)
    {
        return 1;
    }
    if let Some(limit) = l.memory
        && w.killer.memory().is_some_and(|m| m > limit)
    {
        return 2;
    }
    0
}

fn run(d: &'static Dog) {
    let mut st = d.state.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        let now = Instant::now();
        let mut wait: Option<Duration> = None;
        for w in st.watches.values() {
            if w.fired.load(Ordering::Relaxed) != 0 {
                continue;
            }
            let fired = check(w, now);
            if fired != 0 {
                w.killer.kill();
                w.fired.store(fired, Ordering::Relaxed);
                continue;
            }
            let l = &w.limits;
            let next = [
                l.deadline.map(|t| t.saturating_duration_since(now)),
                l.cpu.map(|(_, every)| every),
                l.memory.map(|_| MEMORY_POLL),
            ];
            for t in next.into_iter().flatten() {
                wait = Some(wait.map_or(t, |x| x.min(t)));
            }
        }
        st = match wait {
            Some(t) => d.changed.wait_timeout(st, t).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0),
            None => d.changed.wait(st).unwrap_or_else(PoisonError::into_inner),
        };
    }
}

/// A watch on one request.
pub(crate) struct Armed {
    id: u64,
    fired: Arc<AtomicU8>,
}

/// Watches the helper `killer` stops. `None` when the watchdog thread cannot be started
/// (the request must not run unwatched).
pub(crate) fn arm(limits: Limits, killer: Killer) -> Option<Armed> {
    let d = dog();
    let fired = Arc::new(AtomicU8::new(0));
    let mut st = d.state.lock().unwrap_or_else(PoisonError::into_inner);
    if !st.started {
        st.started =
            std::thread::Builder::new().name("remedy-decoder-watchdog".into()).spawn(move || run(d)).is_ok();
        if !st.started {
            return None;
        }
    }
    st.next += 1;
    let id = st.next;
    st.watches.insert(id, Watch { limits, killer, fired: fired.clone() });
    d.changed.notify_all();
    Some(Armed { id, fired })
}

impl Armed {
    /// Ends the watch; what it did.
    pub(crate) fn disarm(self) -> Fired {
        let d = dog();
        let mut st = d.state.lock().unwrap_or_else(PoisonError::into_inner);
        st.watches.remove(&self.id);
        d.changed.notify_all();
        match self.fired.load(Ordering::Relaxed) {
            1 => Fired::Time,
            2 => Fired::Memory,
            _ => Fired::No,
        }
    }
}
