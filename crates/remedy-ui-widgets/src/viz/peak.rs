//! Peak hold: the marker that stays at the highest recent level, then falls back.

use std::time::Duration;

/// Tracks the highest recent level.
///
/// The peak jumps up with the level and stays for the hold time. After that it falls back
/// toward the level at a steady rate (or drops at once when it should not move).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeakHold {
    hold: Duration,
    fall_per_second: f32,
    peak: f32,
    held_until: Duration,
    last: Duration,
}

impl PeakHold {
    /// A peak at zero that holds for `hold` and then falls `fall_per_second` of the full
    /// scale each second. A rate of zero or less drops it to the level at once.
    pub fn new(hold: Duration, fall_per_second: f32) -> Self {
        PeakHold { hold, fall_per_second, peak: 0.0, held_until: Duration::ZERO, last: Duration::ZERO }
    }

    /// Takes the level at time `now` (any steady clock) and returns the peak.
    pub fn feed(&mut self, level: f32, now: Duration) -> f32 {
        let level = if level.is_finite() { level.clamp(0.0, 1.0) } else { 0.0 };
        if level >= self.peak {
            self.peak = level;
            self.held_until = now + self.hold;
        } else if now > self.held_until {
            if self.fall_per_second > 0.0 {
                let from = self.last.max(self.held_until);
                let fell = now.saturating_sub(from).as_secs_f32() * self.fall_per_second;
                self.peak = (self.peak - fell).max(level);
            } else {
                self.peak = level;
            }
        }
        self.last = now;
        self.peak
    }

    /// Changes how fast the peak falls after the hold (zero or less: it drops at once).
    pub fn set_fall_per_second(&mut self, fall_per_second: f32) {
        self.fall_per_second = fall_per_second;
    }

    /// The peak as of the last [`PeakHold::feed`].
    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// True while the peak is above `level` and so still has something to do: holding, or
    /// falling.
    pub fn is_moving(&self, level: f32) -> bool {
        self.peak > level.clamp(0.0, 1.0)
    }
}
