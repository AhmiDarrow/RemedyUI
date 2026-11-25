//! RemedyUI motion: easing, tweens, springs, interruption and the frame clock.
//!
//! Every animation samples a time given to it by the window's frame clock (the predicted
//! presentation time of the frame being built), never wall time, so animations stay in step
//! and tests can drive them with a fake clock. Springs are solved in closed form, so they are
//! exact at 60, 120 or 240 Hz. Retargeting an animation mid-flight starts from the value and
//! velocity it has, so motion never jumps. See `docs/design/06-input-and-motion.md`.
//!
//! ```
//! use remedy_ui_core::motion::*;
//! use std::time::Duration;
//!
//! let mut a = Animation::new(0.0f32);
//! a.animate_to(100.0, Motion::Spring(Spring::smooth(Duration::from_millis(300))), Duration::ZERO);
//! let mid = a.sample(Duration::from_millis(150));
//! assert!(mid > 10.0 && mid < 100.0);
//! assert!((a.sample(Duration::from_secs(2)) - 100.0).abs() < 0.01);
//! assert!(a.is_settled(Duration::from_secs(2)));
//! ```

mod animatable;
mod clock;
mod curve;
mod spring;

pub use animatable::Animatable;
pub use clock::FrameClock;
pub use curve::{CubicBezier, Easing};
pub use spring::Spring;

use std::time::Duration;

/// How a value moves to a new target.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Motion {
    /// At once.
    Instant,
    /// Along an easing curve over a duration.
    Tween {
        /// How long the move takes.
        duration: Duration,
        /// The curve.
        easing: Easing,
    },
    /// As a spring.
    Spring(Spring),
}

#[derive(Clone, Debug)]
enum State {
    Still,
    Tween { from: Vec<f32>, start: Duration, duration: Duration, easing: Easing },
    Spring { spring: Spring, from: Vec<f32>, velocity: Vec<f32>, start: Duration },
}

/// One animated value of type `T`.
#[derive(Clone, Debug)]
pub struct Animation<T: Animatable> {
    target: T,
    state: State,
    reduced_motion: bool,
}

impl<T: Animatable> Animation<T> {
    /// A value at rest.
    pub fn new(value: T) -> Self {
        Animation { target: value, state: State::Still, reduced_motion: false }
    }

    /// When on, every move is instant (or the app's chosen cross-fade happens elsewhere):
    /// the OS or app asked for reduced motion.
    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
    }

    /// The target the value is moving to.
    pub fn target(&self) -> &T {
        &self.target
    }

    /// Moves to `target` from wherever the value is at `now`, keeping its velocity for
    /// springs, so an interrupted animation turns smoothly.
    pub fn animate_to(&mut self, target: T, motion: Motion, now: Duration) {
        let current = self.sample_vec(now);
        let velocity = self.velocity_vec(now);
        self.target = target;
        self.state = match (motion, self.reduced_motion) {
            (_, true) | (Motion::Instant, _) => State::Still,
            (Motion::Tween { duration, easing }, _) if !duration.is_zero() => {
                State::Tween { from: current, start: now, duration, easing }
            }
            (Motion::Tween { .. }, _) => State::Still,
            (Motion::Spring(spring), _) => State::Spring { spring, from: current, velocity, start: now },
        };
    }

    /// Jumps to `value` with no animation.
    pub fn set(&mut self, value: T) {
        self.target = value;
        self.state = State::Still;
    }

    fn sample_vec(&self, now: Duration) -> Vec<f32> {
        let target = self.target.to_components();
        match &self.state {
            State::Still => target,
            State::Tween { from, start, duration, easing } => {
                let t = (now.saturating_sub(*start).as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
                let e = easing.apply(t);
                from.iter().zip(&target).map(|(a, b)| a + (b - a) * e).collect()
            }
            State::Spring { spring, from, velocity, start } => {
                let t = now.saturating_sub(*start).as_secs_f32();
                from.iter()
                    .zip(&target)
                    .zip(velocity)
                    .map(|((a, b), v)| {
                        let (x, _) = spring.displacement(a - b, *v, t);
                        b + x
                    })
                    .collect()
            }
        }
    }

    fn velocity_vec(&self, now: Duration) -> Vec<f32> {
        let target = self.target.to_components();
        match &self.state {
            State::Still => vec![0.0; target.len()],
            State::Tween { from, start, duration, easing } => {
                let secs = duration.as_secs_f32();
                let t = (now.saturating_sub(*start).as_secs_f32() / secs).clamp(0.0, 1.0);
                if t >= 1.0 {
                    return vec![0.0; target.len()];
                }
                let slope = easing.slope(t) / secs;
                from.iter().zip(&target).map(|(a, b)| (b - a) * slope).collect()
            }
            State::Spring { spring, from, velocity, start } => {
                let t = now.saturating_sub(*start).as_secs_f32();
                from.iter()
                    .zip(&target)
                    .zip(velocity)
                    .map(|((a, b), v)| spring.displacement(a - b, *v, t).1)
                    .collect()
            }
        }
    }

    /// The value at `now`. At rest, and once a tween has run its course, that is the target
    /// itself, exactly: not a round trip through the components (colors move in Oklab, whose
    /// round trip is a few millionths off).
    pub fn sample(&self, now: Duration) -> T {
        match &self.state {
            State::Still => return self.target.clone(),
            State::Tween { start, duration, .. } if now.saturating_sub(*start) >= *duration => {
                return self.target.clone();
            }
            _ => {}
        }
        T::from_components(&self.sample_vec(now))
    }

    /// The value's rate of change at `now`, per second, per component.
    pub fn velocity(&self, now: Duration) -> Vec<f32> {
        self.velocity_vec(now)
    }

    /// True when the value has reached its target and stopped (within a hundredth of a
    /// unit), so the window can stop asking for frames.
    pub fn is_settled(&self, now: Duration) -> bool {
        match &self.state {
            State::Still => true,
            State::Tween { start, duration, .. } => now.saturating_sub(*start) >= *duration,
            State::Spring { .. } => {
                let target = self.target.to_components();
                let x = self.sample_vec(now);
                let v = self.velocity_vec(now);
                x.iter().zip(&target).all(|(a, b)| (a - b).abs() < 0.01) && v.iter().all(|s| s.abs() < 0.01)
            }
        }
    }
}
