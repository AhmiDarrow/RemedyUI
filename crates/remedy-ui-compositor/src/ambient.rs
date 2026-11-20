//! Ambient loops: endless transform and opacity motion the compositor runs on its own, with
//! no UI-thread work per frame (a lighthouse beam sweeping, a glow breathing, a slow drift).
//!
//! The UI thread describes a loop once ([`AmbientLoop`], on a [`Layer`](crate::Layer)); the
//! compositor samples it at each frame's time and only at the window's ambient rate
//! ([`Compositor::set_ambient`](crate::Compositor::set_ambient)). While the rate is `None`
//! (the window is hidden, ambient motion is off) the loops hold where they are and ask for no
//! frames; under reduced motion they rest at their start.

use std::time::Duration;

use remedy_ui_core::{Point, Vec2};
use remedy_ui_paint::Affine;

/// How a loop moves through its period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Wave {
    /// From `from` to `to`, then straight back to `from`: a turn that keeps turning.
    Saw,
    /// From `from` to `to` and back again, eased at both ends: a breath, a sway, a pulse.
    Sine,
}

/// What a loop moves.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LoopMotion {
    /// Turns about `center` (the element's own coordinates), in radians, clockwise.
    Rotate {
        /// The pivot.
        center: Point,
        /// The angle at the start.
        from: f32,
        /// The angle at the far end.
        to: f32,
    },
    /// Moves, in logical pixels.
    Translate {
        /// The offset at the start.
        from: Vec2,
        /// The offset at the far end.
        to: Vec2,
    },
    /// Scales about `center`.
    Scale {
        /// The pivot.
        center: Point,
        /// The scale at the start.
        from: f32,
        /// The scale at the far end.
        to: f32,
    },
    /// Fades (a multiplier on the layer's opacity, 0 to 1).
    Opacity {
        /// The multiplier at the start.
        from: f32,
        /// The multiplier at the far end.
        to: f32,
    },
}

/// An endless loop the compositor runs: what moves, its period, its wave and where in the
/// period it starts.
///
/// ```
/// use std::time::Duration;
/// use remedy_ui_compositor::AmbientLoop;
/// use remedy_ui_core::Point;
///
/// // A lighthouse beam: one full turn every 8 seconds about its pivot.
/// let beam = AmbientLoop::spin(Point::new(40.0, 300.0), Duration::from_secs(8));
/// // A glow breathing between 60 % and full every 3 seconds, a third of the way in.
/// let glow = AmbientLoop::pulse(0.6, 1.0, Duration::from_secs(3)).phase(1.0 / 3.0);
/// assert_eq!(beam.progress(Duration::from_secs(2)), 0.25);
/// assert!(glow.progress(Duration::ZERO) > 0.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientLoop {
    /// What moves.
    pub motion: LoopMotion,
    /// One full cycle.
    pub period: Duration,
    /// How it moves through the cycle.
    pub wave: Wave,
    /// Where in the cycle it starts, 0 to 1 (offset loops so a row of lights does not pulse
    /// in step).
    pub phase: f32,
}

impl AmbientLoop {
    /// `motion` over `period`, as a [`Wave::Sine`] from its start.
    pub fn new(motion: LoopMotion, period: Duration) -> Self {
        AmbientLoop { motion, period, wave: Wave::Sine, phase: 0.0 }
    }

    /// A full clockwise turn about `center` every `period` (a beam, a fan, a mill).
    pub fn spin(center: Point, period: Duration) -> Self {
        AmbientLoop::new(LoopMotion::Rotate { center, from: 0.0, to: std::f32::consts::TAU }, period).wave(Wave::Saw)
    }

    /// Swings about `center` between `from` and `to` radians and back every `period` (a
    /// searchlight, a pendulum, a swaying lantern).
    pub fn sway(center: Point, from: f32, to: f32, period: Duration) -> Self {
        AmbientLoop::new(LoopMotion::Rotate { center, from, to }, period)
    }

    /// Drifts by `by` logical pixels and back every `period` (clouds, parallax, water).
    pub fn drift(by: Vec2, period: Duration) -> Self {
        AmbientLoop::new(LoopMotion::Translate { from: Vec2::ZERO, to: by }, period)
    }

    /// Fades between `from` and `to` opacity and back every `period` (a glow, a pulse).
    pub fn pulse(from: f32, to: f32, period: Duration) -> Self {
        AmbientLoop::new(LoopMotion::Opacity { from, to }, period)
    }

    /// Scales about `center` between `from` and `to` and back every `period` (breathing).
    pub fn breathe(center: Point, from: f32, to: f32, period: Duration) -> Self {
        AmbientLoop::new(LoopMotion::Scale { center, from, to }, period)
    }

    /// Starts `phase` (0 to 1) of the way into the cycle.
    pub fn phase(mut self, phase: f32) -> Self {
        self.phase = if phase.is_finite() { phase.rem_euclid(1.0) } else { 0.0 };
        self
    }

    /// Moves with `wave`.
    pub fn wave(mut self, wave: Wave) -> Self {
        self.wave = wave;
        self
    }

    /// Where the loop is at loop time `t`, from 0 (`from`) to 1 (`to`).
    pub fn progress(&self, t: Duration) -> f32 {
        let period = self.period.as_secs_f64();
        if period <= 0.0 || !period.is_finite() {
            return 0.0;
        }
        let cycle = (t.as_secs_f64() / period + f64::from(self.phase)).rem_euclid(1.0) as f32;
        match self.wave {
            Wave::Saw => cycle,
            Wave::Sine => 0.5 - 0.5 * (cycle * std::f32::consts::TAU).cos(),
        }
    }

    /// The transform and opacity multiplier `loops` give together at loop time `t`.
    pub fn sample(loops: &[AmbientLoop], t: Duration) -> (Affine, f32) {
        let mut affine = Affine::IDENTITY;
        let mut opacity = 1.0f32;
        for l in loops {
            let p = l.progress(t);
            let lerp = |a: f32, b: f32| a + (b - a) * p;
            let about = |c: Point, m: Affine| {
                Affine::translate(Vec2::new(-c.x, -c.y)).then(m).then(Affine::translate(c.to_vec2()))
            };
            match l.motion {
                LoopMotion::Rotate { center, from, to } => {
                    affine = affine.then(about(center, Affine::rotate(lerp(from, to))));
                }
                LoopMotion::Translate { from, to } => {
                    affine = affine.then(Affine::translate(Vec2::new(lerp(from.x, to.x), lerp(from.y, to.y))));
                }
                LoopMotion::Scale { center, from, to } => {
                    let s = lerp(from, to);
                    affine = affine.then(about(center, Affine::scale(s, s)));
                }
                LoopMotion::Opacity { from, to } => opacity *= lerp(from, to).clamp(0.0, 1.0),
            }
        }
        (affine, opacity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saw_turns_and_sine_comes_back() {
        let spin = AmbientLoop::spin(Point::ZERO, Duration::from_secs(4));
        assert_eq!(spin.progress(Duration::from_secs(1)), 0.25);
        assert_eq!(spin.progress(Duration::from_secs(4)), 0.0);
        let pulse = AmbientLoop::pulse(0.0, 1.0, Duration::from_secs(2));
        assert!(pulse.progress(Duration::ZERO).abs() < 1e-6);
        assert!((pulse.progress(Duration::from_secs(1)) - 1.0).abs() < 1e-6);
        assert!(pulse.progress(Duration::from_secs(2)).abs() < 1e-6);
    }

    #[test]
    fn phase_offsets_and_bad_periods_rest() {
        let a = AmbientLoop::pulse(0.0, 1.0, Duration::from_secs(2)).phase(0.5);
        assert!((a.progress(Duration::ZERO) - 1.0).abs() < 1e-6);
        let z = AmbientLoop::pulse(0.0, 1.0, Duration::ZERO);
        assert_eq!(z.progress(Duration::from_secs(3)), 0.0);
        assert_eq!(AmbientLoop::pulse(0.0, 1.0, Duration::from_secs(1)).phase(f32::NAN).phase, 0.0);
    }

    #[test]
    fn loops_compose() {
        let loops = [
            AmbientLoop::spin(Point::new(10.0, 10.0), Duration::from_secs(4)),
            AmbientLoop::pulse(0.5, 0.5, Duration::from_secs(1)),
        ];
        let (m, o) = AmbientLoop::sample(&loops, Duration::from_secs(1));
        // A quarter turn about (10, 10) takes (20, 10) to (10, 20).
        let p = m.apply(Point::new(20.0, 10.0));
        assert!((p.x - 10.0).abs() < 1e-4 && (p.y - 20.0).abs() < 1e-4, "{p:?}");
        assert_eq!(o, 0.5);
    }
}
