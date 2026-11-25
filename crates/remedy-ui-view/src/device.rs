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

/// Someone listening for memory pressure.
type Listener = Rc<dyn Fn(MemoryPressure)>;

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
/// with caches register once and let go of what they can rebuild.
pub fn on_memory_pressure(f: impl Fn(MemoryPressure) + 'static) {
    PRESSURE.with(|p| p.borrow_mut().push(Rc::new(f)));
}

/// Tells everyone listening that memory is short (the runner does, on the host's event).
pub fn memory_pressure(level: MemoryPressure) {
    let listeners: Vec<Listener> = PRESSURE.with(|p| p.borrow().clone());
    for f in listeners {
        f(level);
    }
}
