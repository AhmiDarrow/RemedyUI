//! Secrets on screen: the logic the engine uses for idle locking and sensitive copies.
//!
//! - [`IdleTimer`]: an idle signal fed only by real user input. The runner calls
//!   [`IdleTimer::input`] for pointer, key, wheel and input-method events and nothing else, so
//!   timers, animations and programmatic changes never keep an app unlocked.
//! - [`SensitiveCopy`]: remembers a copied secret by a keyed hash (never the text) so the
//!   clipboard is cleared after its time only if it still holds that secret, and never wipes
//!   something the user copied since.
//!
//! The zeroizing secret buffer and `secret_input` live in `text` and the view; capture
//! protection is a window option every bridge implements. See
//! `docs/design/08-security-and-privacy.md`.

use std::time::Duration;

use zeroize::Zeroize;

/// Locks after a stretch with no real user input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IdleTimer {
    timeout: Option<Duration>,
    last_input: Duration,
}

impl IdleTimer {
    /// A timer that fires after `timeout` without input (`None`: never), counting from `now`.
    pub fn new(timeout: Option<Duration>, now: Duration) -> Self {
        IdleTimer { timeout, last_input: now }
    }

    /// Changes the timeout; the idle stretch so far still counts.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }

    /// Real user input happened at `now`.
    pub fn input(&mut self, now: Duration) {
        self.last_input = self.last_input.max(now);
    }

    /// When the timer will fire if no input comes first.
    pub fn deadline(&self) -> Option<Duration> {
        self.timeout.map(|t| self.last_input + t)
    }

    /// True once the timeout has passed with no input.
    pub fn is_idle(&self, now: Duration) -> bool {
        self.deadline().is_some_and(|d| now >= d)
    }
}

/// A process-random key so stored fingerprints say nothing about the text across runs.
fn key() -> (u64, u64) {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    let s = RandomState::new();
    (s.hash_one(0x5ec_u64), s.hash_one(0x5ec_u64 ^ u64::MAX))
}

/// A keyed 128-bit fingerprint (two independent SipHash-1-3 lanes from the standard library).
fn fingerprint(text: &str, k: (u64, u64)) -> (u64, u64) {
    use std::hash::{BuildHasher, Hasher};
    let lane = |seed: u64| {
        let mut h = std::hash::BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default()
            .build_hasher();
        h.write_u64(seed);
        h.write(text.as_bytes());
        h.finish()
    };
    (lane(k.0), lane(k.1))
}

/// A secret put on the clipboard, remembered by fingerprint only.
#[derive(Debug, PartialEq, Eq)]
pub struct SensitiveCopy {
    key: (u64, u64),
    print: (u64, u64),
    clear_at: Duration,
}

impl SensitiveCopy {
    /// Remembers `text` (copied at `now`) to clear after `keep`. `text` is not kept.
    pub fn new(text: &str, now: Duration, keep: Duration) -> Self {
        let key = key();
        SensitiveCopy { key, print: fingerprint(text, key), clear_at: now + keep }
    }

    /// When to check the clipboard.
    pub fn clear_at(&self) -> Duration {
        self.clear_at
    }

    /// True when the clipboard still holds the copied secret (so clearing is right). The
    /// read text is wiped from memory here.
    pub fn still_there(&self, mut clipboard: String) -> bool {
        let same = fingerprint(&clipboard, self.key) == self.print;
        clipboard.zeroize();
        same
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: u64) -> Duration {
        Duration::from_secs(v)
    }

    #[test]
    fn idle_counts_from_the_last_real_input() {
        let mut t = IdleTimer::new(Some(s(60)), s(0));
        assert_eq!(t.deadline(), Some(s(60)));
        assert!(!t.is_idle(s(59)));
        t.input(s(30));
        assert!(!t.is_idle(s(60)), "input pushed the deadline");
        assert!(t.is_idle(s(90)));
        t.input(s(10));
        assert_eq!(t.deadline(), Some(s(90)), "an older input never moves it back");
        t.set_timeout(None);
        assert!(!t.is_idle(s(10_000)), "off means never");
        assert_eq!(t.deadline(), None);
    }

    #[test]
    fn a_secret_is_cleared_only_while_it_is_still_on_the_clipboard() {
        let c = SensitiveCopy::new("hunter2-recovery", s(5), s(30));
        assert_eq!(c.clear_at(), s(35));
        assert!(c.still_there("hunter2-recovery".to_owned()));
        assert!(!c.still_there("something the user copied since".to_owned()));
        assert!(!c.still_there(String::new()));
        let printed = format!("{c:?}");
        assert!(!printed.contains("hunter2"), "the text is never kept");
    }
}
