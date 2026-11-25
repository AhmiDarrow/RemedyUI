//! The frame clock.

use std::time::Duration;

/// A window's frame clock. The bridge reports each vertical blank's timestamp and the
/// display's frame interval; animations sample the predicted presentation time of the frame
/// being built, so motion matches what reaches the screen. Animations register deadlines while
/// they run, and a clock with no deadlines asks for no frames.
#[derive(Clone, Debug)]
pub struct FrameClock {
    last_vblank: Duration,
    interval: Duration,
    deadlines: Vec<Duration>,
}

impl FrameClock {
    /// A clock for a display refreshing every `interval` (16.7 ms at 60 Hz).
    pub fn new(interval: Duration) -> Self {
        FrameClock { last_vblank: Duration::ZERO, interval, deadlines: Vec::new() }
    }

    /// Records a vertical blank at `at`, and the display's current interval (variable
    /// refresh and monitor changes update it).
    pub fn vblank(&mut self, at: Duration, interval: Duration) {
        self.last_vblank = at;
        if !interval.is_zero() {
            self.interval = interval;
        }
    }

    /// When the frame being built will be shown: one interval after the last blank.
    pub fn presentation_time(&self) -> Duration {
        self.last_vblank + self.interval
    }

    /// The display's frame interval.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Asks for a frame at or after `at` (an animation step, a tooltip delay).
    pub fn request(&mut self, at: Duration) {
        self.deadlines.push(at);
    }

    /// The earliest pending request, if any: what the event loop should wait for.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.deadlines.iter().copied().min()
    }

    /// Removes requests due by `now` and says whether any were.
    pub fn take_due(&mut self, now: Duration) -> bool {
        let before = self.deadlines.len();
        self.deadlines.retain(|d| *d > now);
        self.deadlines.len() != before
    }

    /// True when nothing is waiting: the window can sleep.
    pub fn is_idle(&self) -> bool {
        self.deadlines.is_empty()
    }
}
