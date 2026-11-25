//! How much of an animation is kept decoded: sized from the picture, the shown size and the
//! machine, never a fixed byte limit.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The memory the app lets animation frames use in all (0: not set).
static BUDGET: AtomicU64 = AtomicU64::new(0);

/// How many frames an animation may keep decoded when the app has set no budget. An animation
/// with this many frames or fewer is kept whole and loops with no decoding; a longer one is
/// streamed. Frames are counted, not bytes, so the allowance follows the size they are shown at.
const WHOLE: usize = 64;

/// The fewest frames a stream needs: the one shown, the next, and the first (the poster).
const LEAST: usize = 3;

/// How far ahead of the one shown the next frames are decoded, in playing time: enough to
/// ride out a slow moment of a worker.
const LOOKAHEAD: Duration = Duration::from_millis(250);

/// Tells the animations how much memory, in bytes, their decoded frames may use in all. Each
/// live animation gets an equal share and keeps whole, or streams, by what its frames cost at
/// the size shown. `None` goes back to the default: a thirty-second of the machine's memory
/// (64 MiB to 1 GiB) when the platform says how much it has, and otherwise animations of up
/// to 64 frames kept whole.
pub fn set_memory_budget(bytes: Option<u64>) {
    BUDGET.store(bytes.unwrap_or(0), Ordering::Relaxed);
}

/// The memory decoded frames may use on a machine with `total` bytes when the app has not
/// said: a thirty-second of it, at least 64 MiB and at most 1 GiB.
pub(crate) fn default_budget(total: u64) -> u64 {
    const MIB: u64 = 1024 * 1024;
    (total / 32).clamp(64 * MIB, 1024 * MIB)
}

/// The most decoded frames one of `live` animations may keep when a frame costs `frame_bytes`.
pub(crate) fn resident_frames(frame_bytes: usize, live: usize) -> usize {
    let budget = match BUDGET.load(Ordering::Relaxed) {
        0 => remedy_ui_engine::view::device::total_memory().map_or(0, default_budget),
        set => set,
    };
    match budget {
        0 => WHOLE,
        total => {
            let share = total / live.max(1) as u64;
            usize::try_from(share / frame_bytes.max(1) as u64).unwrap_or(usize::MAX).max(LEAST)
        }
    }
}

/// How many frames to decode ahead of the one shown, for frames that stay `average` each.
pub(crate) fn ahead_frames(average: Duration, limit: usize) -> usize {
    let avg = average.as_micros().max(1);
    let frames = LOOKAHEAD.as_micros().div_ceil(avg);
    usize::try_from(frames).unwrap_or(usize::MAX).clamp(LEAST - 1, limit.max(LEAST) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_follows_the_frame_rate_and_the_allowance() {
        // 250 ms of 20 ms frames.
        assert_eq!(ahead_frames(Duration::from_millis(20), 64), 13);
        // Slow frames: never fewer than the next two.
        assert_eq!(ahead_frames(Duration::from_secs(2), 64), 2);
        // A small allowance holds the window down.
        assert_eq!(ahead_frames(Duration::from_millis(20), 5), 4);
    }

    #[test]
    fn without_a_budget_the_machines_memory_sets_one() {
        const MIB: u64 = 1024 * 1024;
        assert_eq!(default_budget(16 * 1024 * MIB), 512 * MIB, "a thirty-second of 16 GiB");
        assert_eq!(default_budget(512 * MIB), 64 * MIB, "never below 64 MiB");
        assert_eq!(default_budget(256 * 1024 * MIB), 1024 * MIB, "never above 1 GiB");
    }
}
