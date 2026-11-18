//! How many steps a millisecond buys on this device: what turns a time budget into a step
//! budget ([`Limits::operations_at`](crate::Limits::operations_at)).
//!
//! Each engine keeps one [`StepRate`] for the process. The first time the engine is used it
//! calibrates: a fixed small script runs for a few milliseconds away from the UI thread
//! ([`remedy_ui_core::exec::spawn`]: a thread natively, a cooperative slice between frames in
//! a browser page) and the steps it took over the time it took is the rate. After that,
//! every call long enough to time well is a measurement too, so the rate follows the device
//! when it slows down (thermal throttling, a power-saving mode) or speeds up. Until the
//! first measurement the engine's documented fallback applies: a low rate, so a step budget
//! errs short rather than long.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// A measurement shorter than this is not used: the clock's resolution (as coarse as a
/// millisecond in some browsers) would decide it.
pub const MIN_SAMPLE: Duration = Duration::from_millis(2);

/// How much of the way a new measurement moves the rate: a quarter, so a sustained change
/// shows within a few calls and one odd call does not swing it.
const FOLLOW: f64 = 0.25;

/// Steps a millisecond, for one engine, measured on this device.
#[derive(Debug)]
pub struct StepRate {
    /// The measured rate, as `f64` bits; 0 before the first measurement.
    measured: AtomicU64,
    fallback: u64,
    started: AtomicBool,
}

impl StepRate {
    /// A rate not yet measured, at `fallback` steps a millisecond until it is.
    pub const fn new(fallback: u64) -> StepRate {
        StepRate { measured: AtomicU64::new(0), fallback, started: AtomicBool::new(false) }
    }

    /// Steps a millisecond: the measured rate, or the fallback before the first measurement.
    pub fn per_ms(&self) -> u64 {
        match self.measured() {
            Some(r) => (r.round() as u64).max(1),
            None => self.fallback,
        }
    }

    /// The rate used before the first measurement.
    pub fn fallback(&self) -> u64 {
        self.fallback
    }

    /// Whether the rate has been measured on this device.
    pub fn is_calibrated(&self) -> bool {
        self.measured().is_some()
    }

    fn measured(&self) -> Option<f64> {
        let bits = self.measured.load(Ordering::Relaxed);
        (bits != 0).then(|| f64::from_bits(bits))
    }

    /// Records that `steps` took `elapsed`. The first measurement sets the rate; later ones
    /// move it a quarter of the way. Returns false (and changes nothing) for a measurement
    /// too short to time well ([`MIN_SAMPLE`]) or with no steps.
    pub fn record(&self, steps: u64, elapsed: Duration) -> bool {
        if steps == 0 || elapsed < MIN_SAMPLE {
            return false;
        }
        let sample = steps as f64 / (elapsed.as_secs_f64() * 1000.0);
        if !sample.is_finite() || sample <= 0.0 {
            return false;
        }
        let mut bits = self.measured.load(Ordering::Relaxed);
        loop {
            let next =
                if bits == 0 { sample } else { f64::from_bits(bits) * (1.0 - FOLLOW) + sample * FOLLOW };
            match self.measured.compare_exchange_weak(
                bits,
                next.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(now) => bits = now,
            }
        }
    }

    /// Calibrates once per process: runs `measure` (which returns the steps a fixed small
    /// run took and how long it took) away from the caller, and records it. Later calls do
    /// nothing. Where work cannot be started away from the caller, it is measured here.
    pub fn calibrate_once(&'static self, name: &str, measure: fn() -> Option<(u64, Duration)>) {
        if self.started.swap(true, Ordering::Relaxed) {
            return;
        }
        let run = move || {
            if let Some((steps, elapsed)) = measure() {
                let _ = self.record(steps, elapsed);
            }
        };
        if remedy_ui_core::exec::spawn(name, run).is_err()
            && let Some((steps, elapsed)) = measure()
        {
            let _ = self.record(steps, elapsed);
        }
    }
}

/// Runs `round(steps)` with growing step counts, starting at `start`, until one round takes
/// at least twice [`MIN_SAMPLE`], and returns that round's steps and time: a calibration
/// run of a few milliseconds whatever the device's speed. `round` returns how long the
/// steps took, or `None` when they could not run.
pub fn calibrate_with(start: u64, mut round: impl FnMut(u64) -> Option<Duration>) -> Option<(u64, Duration)> {
    let mut steps = start.max(1);
    // Doubling from any start reaches a few milliseconds long before 2^40 steps.
    for _ in 0..40 {
        let took = round(steps)?;
        if took >= MIN_SAMPLE * 2 {
            return Some((steps, took));
        }
        // Aim straight for the target when the round says how far off it is.
        let factor = if took.is_zero() {
            2.0
        } else {
            ((MIN_SAMPLE * 3).as_secs_f64() / took.as_secs_f64()).clamp(2.0, 64.0)
        };
        steps = ((steps as f64) * factor) as u64;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;

    #[test]
    fn a_measurement_sets_the_rate_and_later_ones_follow_it() {
        let r = StepRate::new(1000);
        assert_eq!((r.per_ms(), r.is_calibrated()), (1000, false));
        assert!(!r.record(1_000_000, Duration::from_micros(500)), "too short to time");
        assert!(r.record(1_000_000, Duration::from_millis(10)));
        assert_eq!(r.per_ms(), 100_000);
        // The device slows down: the rate follows within a few calls.
        for _ in 0..12 {
            r.record(500_000, Duration::from_millis(10));
        }
        let after = r.per_ms();
        assert!((50_000..52_000).contains(&after), "{after}");
    }

    #[test]
    fn a_slower_machine_gets_smaller_step_budgets() {
        let (fast, slow) = (StepRate::new(1000), StepRate::new(1000));
        fast.record(2_000_000, Duration::from_millis(10));
        // The same steps taking twice as long: a machine half as fast.
        slow.record(2_000_000, Duration::from_millis(20));
        let limits = Limits::default();
        let (f, s) = (limits.operations_at(fast.per_ms()), limits.operations_at(slow.per_ms()));
        assert_eq!(f, 2 * s);
        assert!(s > 0);
    }

    #[test]
    fn calibration_grows_its_run_until_it_times_well() {
        // A device doing 1,000 steps a millisecond.
        let r = calibrate_with(10, |steps| Some(Duration::from_micros(steps)));
        let Some((steps, took)) = r else { unreachable!("calibrates") };
        assert!(took >= MIN_SAMPLE * 2 && took <= Duration::from_millis(500), "{took:?}");
        let s = StepRate::new(1);
        assert!(s.record(steps, took));
        assert_eq!(s.per_ms(), 1000);
        assert_eq!(calibrate_with(10, |_| None), None);
    }
}
