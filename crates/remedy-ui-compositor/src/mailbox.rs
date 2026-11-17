//! The single-slot mailbox commits travel through.

use std::sync::{Mutex, MutexGuard};

/// A single-slot handoff between threads. A newer value replaces one the receiver has not
/// taken yet, so the compositor always draws the latest commit and the UI thread never waits.
#[derive(Debug, Default)]
pub struct Mailbox<T> {
    slot: Mutex<Option<T>>,
}

impl<T> Mailbox<T> {
    /// An empty mailbox.
    pub fn new() -> Self {
        Mailbox { slot: Mutex::new(None) }
    }

    fn lock(&self) -> MutexGuard<'_, Option<T>> {
        // A panic while holding the lock cannot leave the slot half-written: it only ever
        // holds a whole value or nothing, so the poisoned guard is still sound.
        self.slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Posts a value. Returns true when it replaced one that was never taken.
    pub fn post(&self, value: T) -> bool {
        self.lock().replace(value).is_some()
    }

    /// Takes the waiting value, if any.
    pub fn take(&self) -> Option<T> {
        self.lock().take()
    }

    /// True when a value is waiting.
    pub fn is_pending(&self) -> bool {
        self.lock().is_some()
    }
}
