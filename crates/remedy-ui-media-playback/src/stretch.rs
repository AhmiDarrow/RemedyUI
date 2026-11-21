//! Changing playback speed without changing pitch: a WSOLA time stretcher (waveform-similarity
//! overlap-add). Sound is cut into short blocks that overlap; each block is taken from where
//! the media should be at this speed, nudged (within a few milliseconds) to where its start
//! best matches how the sound just played would have continued, and cross-faded in. Played
//! faster, blocks are taken further apart and parts of the sound are skipped; slower, parts
//! repeat. Waveforms are not resampled, so voices keep their pitch.
//!
//! Streaming and bounded: it holds a few blocks of input, does a fixed amount of work per
//! block, and runs on the presenting thread.

use std::collections::VecDeque;

/// The slowest playback speed.
pub const MIN_SPEED: f64 = 0.25;
/// The fastest playback speed.
pub const MAX_SPEED: f64 = 4.0;

/// The speeds the player's speed button steps through.
pub const SPEEDS: [f64; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// `speed` as playback takes it: within [`MIN_SPEED`]..=[`MAX_SPEED`], and 1 when it is not
/// a number.
pub fn clamp_speed(speed: f64) -> f64 {
    if speed.is_finite() { speed.clamp(MIN_SPEED, MAX_SPEED) } else { 1.0 }
}

/// The time stretcher for interleaved sound of `channels` channels.
pub(crate) struct Stretcher {
    channels: usize,
    /// Frames each block adds to the output (half a block: blocks overlap by half).
    hop: usize,
    /// How far a block's start may move to match the sound before it, in frames.
    tolerance: usize,
    /// Input not yet used, interleaved; frame 0 is input frame `origin`.
    input: VecDeque<f32>,
    origin: u64,
    /// Where the next block should start at this speed, in input frames (fractional).
    ideal: f64,
    /// How the last block went on: the `hop` frames after it (interleaved), what the next
    /// block cross-fades from and is matched against. Empty before the first block.
    tail: Vec<f32>,
    /// The input frame `tail` starts at.
    tail_at: u64,
    /// The cross-fade's weights for the incoming block, `hop` of them.
    fade: Vec<f32>,
    /// Where the sound ends, in input frames since the last reset, once no more follows
    /// ([`finish`](Self::finish)): silence is held after it so the last blocks can form.
    end: Option<u64>,
}

impl Stretcher {
    /// A stretcher for `channels` channels at `sample_rate` frames per second: blocks of about
    /// 40 ms overlapping by half, each free to move 12 ms to match.
    pub(crate) fn new(channels: u16, sample_rate: u32) -> Self {
        let rate = f64::from(sample_rate.max(1));
        let hop = ((rate * 0.020) as usize).max(16);
        let tolerance = ((rate * 0.012) as usize).max(4);
        let fade = (0..hop)
            .map(|i| {
                // A raised cosine: the two weights at each frame add to one.
                let x = (i as f32 + 0.5) / hop as f32;
                0.5 - 0.5 * (std::f32::consts::PI * x).cos()
            })
            .collect();
        Stretcher {
            channels: usize::from(channels.max(1)),
            hop,
            tolerance,
            input: VecDeque::new(),
            origin: 0,
            ideal: 0.0,
            tail: Vec::new(),
            tail_at: 0,
            fade,
            end: None,
        }
    }

    /// Forgets everything (a seek, a gap in the sound).
    pub(crate) fn reset(&mut self) {
        self.input.clear();
        self.origin = 0;
        self.ideal = 0.0;
        self.tail.clear();
        self.tail_at = 0;
        self.end = None;
    }

    /// No more input follows: what is held is played to its end. Silence is added after it
    /// so the last blocks can form, and [`step`](Self::step) stops once the output has
    /// reached the end of the real sound.
    pub(crate) fn finish(&mut self) {
        if self.end.is_some() {
            return;
        }
        self.end = Some(self.input_end());
        let pad = (self.tolerance + 2 * self.hop + 1) * self.channels;
        self.input.extend(std::iter::repeat_n(0.0, pad));
    }

    /// True once [`finish`](Self::finish)ed and played to the end of the sound.
    pub(crate) fn done(&self) -> bool {
        self.end.is_some_and(|e| self.ideal >= e as f64)
    }

    /// Frames of output each block adds.
    pub(crate) fn hop(&self) -> usize {
        self.hop
    }

    /// Input frames held, not yet used.
    pub(crate) fn held(&self) -> usize {
        self.input.len() / self.channels
    }

    /// Where the input given so far ends, in frames since the last [`reset`](Self::reset).
    pub(crate) fn input_end(&self) -> u64 {
        self.origin + self.held() as u64
    }

    /// Where the next output block starts in the input, in frames since the last
    /// [`reset`](Self::reset): what it plays from, for the clock.
    pub(crate) fn position(&self) -> f64 {
        self.ideal
    }

    /// Adds interleaved input.
    pub(crate) fn push(&mut self, samples: &[f32]) {
        self.input.extend(samples);
    }

    fn frame(&self, at: u64, ch: usize) -> f32 {
        let i = at.saturating_sub(self.origin) as usize * self.channels + ch;
        self.input.get(i).copied().unwrap_or(0.0)
    }

    /// One block of `hop` output frames at `speed`, appended to `out`, or `None` when more
    /// input is needed first.
    pub(crate) fn step(&mut self, speed: f64, out: &mut Vec<f32>) -> Option<()> {
        let speed = clamp_speed(speed);
        if self.done() {
            return None;
        }
        let (hop, ch) = (self.hop, self.channels);
        let ideal = self.ideal.round().max(0.0) as u64;
        // Matched against the tail: the block may start anywhere in ideal ± tolerance, and
        // needs a hop past its start for the cross-fade and another for the next tail.
        let need = ideal + self.tolerance as u64 + 2 * hop as u64;
        if self.origin + self.held() as u64 <= need {
            return None;
        }
        let start = if self.tail.is_empty() { ideal } else { self.best_start(ideal) };
        // Cross-fade from the tail into the block (the very first block plays as it is).
        for i in 0..hop {
            let w = self.fade.get(i).copied().unwrap_or(1.0);
            for c in 0..ch {
                let new = self.frame(start + i as u64, c);
                let old = if self.tail.is_empty() {
                    new
                } else {
                    self.tail.get(i * ch + c).copied().unwrap_or(0.0)
                };
                out.push(old * (1.0 - w) + new * w);
            }
        }
        // How this block goes on, for the next cross-fade.
        self.tail.clear();
        self.tail_at = start + hop as u64;
        for i in 0..hop as u64 {
            for c in 0..ch {
                self.tail.push(self.frame(self.tail_at + i, c));
            }
        }
        self.ideal += hop as f64 * speed;
        // Input no later block can reach is let go.
        let keep_from = (self.ideal.floor() as u64).saturating_sub(self.tolerance as u64).min(self.tail_at);
        if keep_from > self.origin {
            let drop = ((keep_from - self.origin) as usize * ch).min(self.input.len());
            self.input.drain(..drop);
            self.origin = keep_from;
        }
        Some(())
    }

    /// The start within `ideal` ± tolerance whose sound best continues the tail: the largest
    /// normalised correlation, on the channels' mix. A coarse search every fourth frame on
    /// every fourth sample, then a fine one around the best.
    fn best_start(&self, ideal: u64) -> u64 {
        let lo = ideal.saturating_sub(self.tolerance as u64).max(self.origin);
        let hi = ideal + self.tolerance as u64;
        let ch = self.channels;
        let tail_mix: Vec<f32> = (0..self.hop)
            .map(|i| (0..ch).map(|c| self.tail.get(i * ch + c).copied().unwrap_or(0.0)).sum())
            .collect();
        let score = |s: u64, stride: usize| -> f32 {
            let (mut dot, mut energy) = (0.0f32, 1e-9f32);
            for i in (0..self.hop).step_by(stride) {
                let x: f32 = (0..ch).map(|c| self.frame(s + i as u64, c)).sum();
                dot += x * tail_mix.get(i).copied().unwrap_or(0.0);
                energy += x * x;
            }
            dot / energy.sqrt()
        };
        let best_of = |from: u64, to: u64, step: usize, stride: usize| -> u64 {
            let mut best = (f32::MIN, from);
            for s in (from..=to).step_by(step) {
                let v = score(s, stride);
                if v > best.0 {
                    best = (v, s);
                }
            }
            best.1
        };
        let coarse = best_of(lo, hi, 4, 4);
        best_of(coarse.saturating_sub(3).max(lo), (coarse + 3).min(hi), 1, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// `seconds` of a sine of `hz` on each channel (`hz[c]`), interleaved.
    fn sine(hz: &[f32], seconds: f32) -> Vec<f32> {
        let n = (RATE as f32 * seconds) as usize;
        (0..n)
            .flat_map(|i| {
                hz.iter().map(move |f| (2.0 * std::f32::consts::PI * f * i as f32 / RATE as f32).sin() * 0.5)
            })
            .collect()
    }

    /// Everything the stretcher makes of `input` at `speed`, fed in blocks of 1000 frames.
    fn stretch(input: &[f32], channels: u16, speed: f64) -> Vec<f32> {
        let mut s = Stretcher::new(channels, RATE);
        let mut out = Vec::new();
        for chunk in input.chunks(1000 * usize::from(channels)) {
            s.push(chunk);
            while s.step(speed, &mut out).is_some() {}
        }
        out
    }

    /// The frequency of channel `c` of `x` by its upward zero crossings.
    fn frequency(x: &[f32], channels: usize, c: usize) -> f32 {
        let s: Vec<f32> = x.iter().skip(c).step_by(channels).copied().collect();
        let ups = s.windows(2).filter(|w| matches!(w, [a, b] if *a < 0.0 && *b >= 0.0)).count();
        ups as f32 * RATE as f32 / s.len() as f32
    }

    #[test]
    fn speed_changes_length_not_pitch() {
        let input = sine(&[440.0], 2.0);
        for speed in [0.5, 0.75, 1.25, 1.5, 2.0, 3.0] {
            let out = stretch(&input, 1, speed);
            let want = input.len() as f64 / speed;
            let got = out.len() as f64;
            // All but the last few blocks the input could not yet fill.
            assert!(got <= want && got > want - 6.0 * f64::from(RATE) * 0.02, "{speed}: {got} of {want}");
            let f = frequency(&out, 1, 0);
            assert!((f - 440.0).abs() < 6.0, "{speed}: the pitch stays 440 Hz, got {f}");
        }
    }

    #[test]
    fn channels_stay_apart_and_the_sound_is_smooth() {
        let input = sine(&[440.0, 660.0], 1.0);
        for speed in [0.6, 1.7] {
            let out = stretch(&input, 2, speed);
            assert!((frequency(&out, 2, 0) - 440.0).abs() < 8.0, "{speed}: left");
            assert!((frequency(&out, 2, 1) - 660.0).abs() < 10.0, "{speed}: right");
            // A 660 Hz sine at 0.5 moves at most 0.13 a sample: no clicks between blocks.
            let left: Vec<f32> = out.iter().step_by(2).copied().collect();
            let jump = left
                .windows(2)
                .map(|w| match w {
                    [a, b] => (b - a).abs(),
                    _ => 0.0,
                })
                .fold(0.0, f32::max);
            assert!(jump < 0.2, "{speed}: no click (largest step {jump})");
        }
    }

    #[test]
    fn its_position_follows_the_speed_and_it_holds_little() {
        let mut s = Stretcher::new(1, RATE);
        let mut out = Vec::new();
        let input = sine(&[300.0], 3.0);
        for chunk in input.chunks(800) {
            s.push(chunk);
            while s.step(2.0, &mut out).is_some() {}
            assert!(s.held() < RATE as usize / 5, "bounded input held: {}", s.held());
        }
        // Each output frame stands for two input frames.
        let ratio = s.position() / out.len() as f64;
        assert!((ratio - 2.0).abs() < 1e-9, "{ratio}");
        s.reset();
        assert_eq!((s.held(), s.position()), (0, 0.0));
    }

    #[test]
    fn the_end_of_the_sound_is_played_too() {
        let input = sine(&[440.0], 1.0);
        for speed in [0.5, 1.5, 2.0] {
            let mut s = Stretcher::new(1, RATE);
            let mut out = Vec::new();
            s.push(&input);
            while s.step(speed, &mut out).is_some() {}
            let before = out.len();
            s.finish();
            while s.step(speed, &mut out).is_some() {}
            assert!(s.done());
            let want = input.len() as f64 / speed;
            let got = out.len() as f64;
            // All of it, to within the last block.
            assert!(
                got >= want - 1.0 && got < want + s.hop() as f64,
                "{speed}: {got} of {want} ({before} before)"
            );
            // Finishing again, or stepping on, adds nothing.
            s.finish();
            assert!(s.step(speed, &mut out).is_none());
        }
    }

    #[test]
    fn strange_speeds_are_bounded() {
        assert_eq!(clamp_speed(f64::NAN), 1.0);
        assert_eq!(clamp_speed(1000.0), MAX_SPEED);
        assert_eq!(clamp_speed(0.0), MIN_SPEED);
        assert_eq!(clamp_speed(-2.0), MIN_SPEED);
        let out = stretch(&sine(&[200.0], 0.5), 1, f64::INFINITY);
        assert!(!out.is_empty() && out.iter().all(|v| v.is_finite()));
    }
}
