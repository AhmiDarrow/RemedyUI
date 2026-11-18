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
    fn clocks_move_forward() {
        let a = Instant::now();
        let b = Instant::now();
        assert!(b >= a);
        assert!(unix_seconds() > 1_700_000_000, "the wall clock reads after 2023");
    }
}
