//! What the engine knows about the machine it runs on, for packs that size themselves from it
//! (caches, decoded frames, script limits): its memory, and when the system runs short.
//!
//! The app's runner fills these in from its host; packs read them and listen.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

pub use remedy_ui_core::host::MemoryPressure;

/// The machine's physical memory in bytes (0: not known).
static TOTAL_MEMORY: AtomicU64 = AtomicU64::new(0);

/// Someone listening for memory pressure, and whether what it listens for is still there.
#[derive(Clone)]
struct Listener {
    alive: Rc<dyn Fn() -> bool>,
    call: Rc<dyn Fn(MemoryPressure)>,
}

thread_local! {
    /// Who wants to hear that memory is short, on this (the UI) thread.
    static PRESSURE: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

/// The machine's physical memory in bytes, when the platform said.
pub fn total_memory() -> Option<u64> {
    match TOTAL_MEMORY.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

/// Records the machine's physical memory (the runner does, from its host).
pub fn set_total_memory(bytes: Option<u64>) {
    TOTAL_MEMORY.store(bytes.unwrap_or(0), Ordering::Relaxed);
}

/// Calls `f` whenever the system says memory is short, for as long as the app runs. Packs
/// with caches register once and let go of what they can rebuild. Something that comes and
/// goes (a view, a document) listens with [`on_memory_pressure_for`] instead, or its
/// listener stays behind for good.
pub fn on_memory_pressure(f: impl Fn(MemoryPressure) + 'static) {
    listen(Listener { alive: Rc::new(|| true), call: Rc::new(f) });
}

/// Calls `f` with `owner` whenever the system says memory is short, for as long as `owner`
/// lives: the listener holds it weakly and goes once it is dropped.
pub fn on_memory_pressure_for<T: 'static>(owner: &Rc<T>, f: impl Fn(&Rc<T>, MemoryPressure) + 'static) {
    let (held, weak) = (Rc::downgrade(owner), Rc::downgrade(owner));
    listen(Listener {
        alive: Rc::new(move || held.strong_count() > 0),
        call: Rc::new(move |level| {
            if let Some(owner) = weak.upgrade() {
                f(&owner, level);
            }
        }),
    });
}

/// Takes the listeners whose owner is gone off the list. The caller drops them once the
/// list is no longer borrowed: what a listener captured may register another as it drops.
fn prune() -> Vec<Listener> {
    PRESSURE.with(|p| {
        let mut p = p.borrow_mut();
        let (live, dead) = std::mem::take(&mut *p).into_iter().partition(|l| (l.alive)());
        *p = live;
        dead
    })
}

/// Adds a listener, dropping those whose owner is gone (so the list follows what lives even
/// when memory never runs short).
fn listen(l: Listener) {
    drop(prune());
    PRESSURE.with(|p| p.borrow_mut().push(l));
}

/// Tells everyone listening that memory is short (the runner does, on the host's event).
pub fn memory_pressure(level: MemoryPressure) {
    drop(prune());
    let listeners: Vec<Listener> = PRESSURE.with(|p| p.borrow().clone());
    for l in listeners {
        (l.call)(level);
    }
}

/// How many listeners are registered (tests).
#[doc(hidden)]
pub fn memory_pressure_listeners() -> usize {
    PRESSURE.with(|p| p.borrow().len())
}
