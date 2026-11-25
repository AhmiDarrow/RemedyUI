//! How many steps a millisecond buys on this device: what turns a time budget into a step
//! budget ([`Limits::operations_at`](crate::script::Limits::operations_at)).
//!
//! Each engine keeps one [`StepRate`] for the process. The first time the engine is used it
//! calibrates: a fixed small script runs for a few milliseconds away from the UI thread
//! ([`remedy_ui_core::exec::spawn`]: a thread natively, a cooperative slice between frames in
//! a browser page) and the steps it took over the time it took is the rate. After that,
//! every call long enough to time well is a measurement too, so the rate follows the device
//! when it slows down (thermal throttling, a power-saving mode) or speeds up. Until the
//! first measurement no step budget is made from a guess ([`Limits::step_budget`]
//! (crate::script::Limits::step_budget)): only the time limit applies, so fair work is never stopped
//! by a rate the device beats. The engine's fallback rate only spaces its clock checks then.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// The shortest measurement used on any clock: a page's clock can be as coarse as a
/// millisecond. [`min_sample`] lengthens it where the thread clock is coarser.
pub const MIN_SAMPLE: Duration = Duration::from_millis(2);

/// The shortest measurement this device times well: [`MIN_SAMPLE`], or four ticks of the
/// thread clock where that is coarser (Windows' thread clock moves in about 15.6 ms steps,
/// which would make a few milliseconds read as nothing or as a whole tick). `None` until the
/// clock's resolution is known: it is measured by calibration, away from the UI thread.
pub fn min_sample() -> Option<Duration> {
    remedy_ui_core::time::ThreadTime::resolution_if_known().map(|r| MIN_SAMPLE.max(r * 4))
}

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

    /// Whether a call that took `steps` really ran for `time` at this device's speed, rather than
    /// waiting: on a virtual machine the host can take the processor away from the guest, and
    /// the guest's own thread clock counts that time as run. A call that made a quarter or less
    /// of the steps the measured rate allows for `time` was mostly not running, so the time is
    /// not held against it (its step budget still bounds it); a script that does run, an
    /// endless loop included, makes its steps at full speed and is stopped. A quarter leaves
    /// room for the rate falling behind the device (throttling) between measurements. Before
    /// the rate is measured there is nothing to compare with, and the time decides alone.
    pub fn ran_for(&self, steps: u64, time: Duration) -> bool {
        let Some(rate) = self.measured() else { return true };
        let expected = rate * time.as_secs_f64() * 1000.0;
        steps as f64 >= expected / 4.0
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
    /// too short to time well ([`min_sample`]), before the clock's resolution is known, or with
    /// no steps.
    pub fn record(&self, steps: u64, elapsed: Duration) -> bool {
        let Some(min) = min_sample() else { return false };
        if steps == 0 || elapsed < min {
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
/// at least twice [`min_sample`], and returns that round's steps and time: a calibration
/// run long enough for this device's clock whatever its speed. `round` returns how long the
/// steps took, or `None` when they could not run. Measures the thread clock's resolution
/// first if it is not known yet (call it away from the UI thread).
pub fn calibrate_with(start: u64, mut round: impl FnMut(u64) -> Option<Duration>) -> Option<(u64, Duration)> {
    let min = MIN_SAMPLE.max(remedy_ui_core::time::ThreadTime::resolution() * 4);
    let mut steps = start.max(1);
    // Doubling from any start reaches a few milliseconds long before 2^40 steps.
    for _ in 0..40 {
        let took = round(steps)?;
        if took >= min * 2 {
            return Some((steps, took));
        }
        // Aim straight for the target when the round says how far off it is.
        let factor = if took.is_zero() {
            2.0
        } else {
            ((min * 3).as_secs_f64() / took.as_secs_f64()).clamp(2.0, 64.0)
        };
        steps = ((steps as f64) * factor) as u64;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::Limits;

    /// The shortest sample this machine times well, with the clock's resolution measured.
    fn sample() -> Duration {
        let _ = remedy_ui_core::time::ThreadTime::resolution();
        min_sample().unwrap_or(MIN_SAMPLE)
    }

    #[test]
    fn a_measurement_sets_the_rate_and_later_ones_follow_it() {
        let t = sample();
        let ms = t.as_secs_f64() * 1000.0;
        let r = StepRate::new(1000);
        assert_eq!((r.per_ms(), r.is_calibrated()), (1000, false));
        assert!(!r.record(1_000_000, t / 4), "too short to time on this clock");
        assert!(r.record((100_000.0 * ms) as u64, t));
        assert!((99_000..=101_000).contains(&r.per_ms()), "{}", r.per_ms());
        // The device slows down: the rate follows within a few calls.
        for _ in 0..12 {
            r.record((50_000.0 * ms) as u64, t);
        }
        let after = r.per_ms();
        assert!((50_000..52_000).contains(&after), "{after}");
    }

    #[test]
    fn a_slower_machine_gets_smaller_step_budgets() {
        let t = sample();
        let (fast, slow) = (StepRate::new(1000), StepRate::new(1000));
        fast.record(20_000_000, t);
        // The same steps taking twice as long: a machine half as fast.
        slow.record(20_000_000, t * 2);
        let limits = Limits::default();
        let (f, s) = (limits.step_budget(&fast), limits.step_budget(&slow));
        assert!(f.abs_diff(2 * s) <= 2, "{f} {s}");
        assert!(s > 0);
    }

    #[test]
    fn time_counts_only_when_the_call_ran_at_the_devices_speed() {
        let t = sample();
        let r = StepRate::new(1000);
        assert!(r.ran_for(0, Duration::from_millis(8)), "unmeasured: the time decides");
        r.record((10_000.0 * t.as_secs_f64() * 1000.0) as u64, t);
        // 10,000 steps a millisecond: 8 ms of running is 80,000 steps.
        assert!(r.ran_for(80_000, Duration::from_millis(8)), "ran at full speed");
        assert!(r.ran_for(25_000, Duration::from_millis(8)), "throttled, still running");
        assert!(!r.ran_for(1_000, Duration::from_millis(8)), "mostly waiting: the time was taken");
    }

    #[test]
    fn an_unmeasured_device_is_held_by_time_alone() {
        let r = StepRate::new(1000);
        assert_eq!(Limits::default().step_budget(&r), u64::MAX, "no budget from a guessed rate");
        assert_eq!(Limits::default().operations(Some(5)).step_budget(&r), 5, "a set budget holds");
    }

    #[test]
    fn calibration_grows_its_run_until_it_times_well() {
        let t = sample();
        // A device doing 1,000 steps a millisecond.
        let r = calibrate_with(10, |steps| Some(Duration::from_micros(steps)));
        let Some((steps, took)) = r else { unreachable!("calibrates") };
        assert!(took >= t * 2 && took <= Duration::from_secs(2), "{took:?}");
        let s = StepRate::new(1);
        assert!(s.record(steps, took));
        assert_eq!(s.per_ms(), 1000);
        assert_eq!(calibrate_with(10, |_| None), None);
    }
}
