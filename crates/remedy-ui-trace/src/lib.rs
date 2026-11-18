//! Tracing for RemedyUI: every phase of the engine emits a [`tracing`] span, named after the
//! phase, with the target [`TARGET`]. Apps see them in any `tracing` subscriber; the devtools
//! record them into the performance overlay and Perfetto traces
//! (`docs/design/09-performance-and-resources.md`).
//!
//! With no subscriber, or one that is not recording, a span costs one check of its
//! callsite's interest: nothing is allocated or timed.
//!
//! ```
//! let _layout = remedy_ui_trace::phase!("layout").entered();
//! ```
//!
//! Work that is not a span on a thread (the GPU running what the renderer submitted) is
//! reported as a slice with explicit begin and end times on the shared [`now_ns`] clock
//! ([`gpu_slice`]).

use std::sync::OnceLock;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::Instant;
// The page's clock in a browser, where std's panics.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_time::Instant;

pub use tracing;

/// The target of the engine's phase spans.
pub const TARGET: &str = "remedy_ui";

/// The target of GPU work slices ([`gpu_slice`]).
pub const GPU_TARGET: &str = "remedy_ui::gpu";

/// The phases a frame is made of, by span name, in pipeline order.
pub mod phases {
    /// Input dispatched to handlers (pointer, keys, wheel, input method).
    pub const EVENTS: &str = "events";
    /// Effects and bindings re-running after state changed.
    pub const BINDS: &str = "binds";
    /// Style resolution for elements whose state or style changed.
    pub const STYLE: &str = "style";
    /// Layout of dirty subtrees, and element bounds.
    pub const LAYOUT: &str = "layout";
    /// Paint recording: display lists of dirty elements.
    pub const PAINT: &str = "paint";
    /// Assembling compositor layers from paint chunks.
    pub const LAYERS: &str = "layers";
    /// The accessibility tree brought up to date.
    pub const SEMANTICS: &str = "semantics";
    /// The compositor: scroll and animation deltas, damage, the raster plan.
    pub const COMPOSITE: &str = "composite";
    /// Rasterizing tiles (on the CPU, or recording and submitting GPU work).
    pub const RASTER: &str = "raster";
    /// Handing the frame to the platform.
    pub const PRESENT: &str = "present";
    /// One whole frame of one window (the parent of the phases above on its thread).
    pub const FRAME: &str = "frame";
    /// Every phase, in pipeline order.
    pub const ALL: [&str; 10] =
        [EVENTS, BINDS, STYLE, LAYOUT, PAINT, LAYERS, SEMANTICS, COMPOSITE, RASTER, PRESENT];
}

/// A span for one of the engine's phases (or other engine work), at trace level with the
/// target [`TARGET`]: `phase!("layout")`, or with fields, `phase!("frame", ui = id)`.
#[macro_export]
macro_rules! phase {
    ($name:literal $(, $($fields:tt)*)?) => {
        $crate::tracing::trace_span!(target: "remedy_ui", $name $(, $($fields)*)?)
    };
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Nanoseconds on the trace clock: one monotonic clock for every thread of the process,
/// starting at its first use.
pub fn now_ns() -> u64 {
    ns_at(Instant::now())
}

/// `at` on the trace clock ([`now_ns`]); instants before the clock started read 0.
pub fn ns_at(at: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(epoch()).as_nanos()).unwrap_or(u64::MAX)
}

/// True when something records GPU work slices: renderers check this before arranging to
/// learn when submitted work finishes, so nothing is arranged when nobody listens.
pub fn gpu_slices_wanted() -> bool {
    tracing::enabled!(target: "remedy_ui::gpu", tracing::Level::TRACE)
}

/// Reports GPU work that ran from `begin_ns` to `end_ns` on the [`now_ns`] clock. `what`
/// names it (the kind of work a renderer submitted).
pub fn gpu_slice(what: &'static str, begin_ns: u64, end_ns: u64) {
    tracing::trace!(target: "remedy_ui::gpu", begin_ns, end_ns, what);
}

/// Process-wide measures of what the engine holds (cache bytes, entries), for tools such as
/// the devtools' performance overlay. Each owner (one renderer, one compositor) keeps a
/// [`Gauge`](gauges::Gauge) and sets it as its holdings change; [`snapshot`](gauges::snapshot) sums the live
/// gauges of each name. A dropped gauge stops counting.
pub mod gauges {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, Weak};

    /// Every gauge ever made, by name; dead ones are swept as new ones register.
    type Registry = Mutex<Vec<(&'static str, Weak<AtomicU64>)>>;

    fn registry() -> &'static Registry {
        static R: OnceLock<Registry> = OnceLock::new();
        R.get_or_init(Mutex::default)
    }

    /// One owner's measure of `name`.
    #[derive(Debug)]
    pub struct Gauge(Arc<AtomicU64>);

    impl Gauge {
        /// A gauge counting toward `name` (such as `cpu tiles`), starting at 0.
        pub fn new(name: &'static str) -> Self {
            let value = Arc::new(AtomicU64::new(0));
            let mut r = registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            r.retain(|(_, w)| w.strong_count() > 0);
            r.push((name, Arc::downgrade(&value)));
            Gauge(value)
        }

        /// Sets this owner's share.
        pub fn set(&self, value: u64) {
            self.0.store(value, Ordering::Relaxed);
        }

        /// This owner's share.
        pub fn get(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    /// Every name with live gauges and their sum, sorted by name.
    pub fn snapshot() -> Vec<(&'static str, u64)> {
        let r = registry().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out: Vec<(&'static str, u64)> = Vec::new();
        for (name, w) in r.iter() {
            let Some(v) = w.upgrade() else { continue };
            let v = v.load(Ordering::Relaxed);
            match out.iter_mut().find(|(n, _)| n == name) {
                Some((_, sum)) => *sum += v,
                None => out.push((name, v)),
            }
        }
        out.sort_unstable_by_key(|(n, _)| *n);
        out
    }

    /// The sum of the live gauges named `name`.
    pub fn total(name: &str) -> u64 {
        snapshot().into_iter().find(|(n, _)| *n == name).map_or(0, |(_, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauges_sum_their_live_owners() {
        let a = gauges::Gauge::new("test bytes");
        let b = gauges::Gauge::new("test bytes");
        a.set(10);
        b.set(5);
        assert_eq!(gauges::total("test bytes"), 15);
        drop(b);
        assert_eq!(gauges::total("test bytes"), 10);
        drop(a);
        assert_eq!(gauges::total("test bytes"), 0);
    }

    #[test]
    fn the_clock_only_moves_forward() {
        let a = now_ns();
        let b = now_ns();
        assert!(b >= a);
        assert_eq!(ns_at(epoch()), 0);
    }

    #[test]
    fn phases_are_distinct() {
        for (i, a) in phases::ALL.iter().enumerate() {
            for b in phases::ALL.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
        assert!(!phases::ALL.contains(&phases::FRAME));
    }

    #[test]
    fn spans_cost_nothing_without_a_subscriber() {
        let span = phase!("layout");
        assert!(span.is_disabled());
        assert!(!gpu_slices_wanted());
    }
}
