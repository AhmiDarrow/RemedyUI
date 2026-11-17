//! Damped springs, solved in closed form.

use std::time::Duration;

/// A damped spring with unit mass: `x'' = -stiffness·x - damping·x'`, where `x` is the
/// distance from the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    /// Pull toward the target.
    pub stiffness: f32,
    /// Resistance to motion.
    pub damping: f32,
}

impl Spring {
    /// A spring described the way designers think of it: roughly how long it takes to
    /// arrive, and how much it bounces (0 none, toward 1 more). This is the response and
    /// damping-fraction model SwiftUI uses.
    pub fn new(duration: Duration, bounce: f32) -> Self {
        let response = duration.as_secs_f32().max(0.01);
        let zeta = (1.0 - bounce).clamp(0.05, 2.0);
        let stiffness = (2.0 * std::f32::consts::PI / response).powi(2);
        let damping = 4.0 * std::f32::consts::PI * zeta / response;
        Spring { stiffness, damping }
    }

    /// No bounce: critically damped, the fastest settle without overshoot.
    pub fn smooth(duration: Duration) -> Self {
        Self::new(duration, 0.0)
    }

    /// A little bounce, for playful feedback.
    pub fn bouncy(duration: Duration) -> Self {
        Self::new(duration, 0.3)
    }

    /// Distance from the target and velocity at time `t` seconds, starting `x0` from the
    /// target with velocity `v0`.
    pub fn displacement(&self, x0: f32, v0: f32, t: f32) -> (f32, f32) {
        let k = self.stiffness.max(1e-6);
        let c = self.damping.max(0.0);
        let w0 = k.sqrt();
        let zeta = c / (2.0 * w0);
        if (zeta - 1.0).abs() < 1e-4 {
            // Critically damped.
            let b = v0 + w0 * x0;
            let e = (-w0 * t).exp();
            let x = (x0 + b * t) * e;
            let v = (b - w0 * (x0 + b * t)) * e;
            (x, v)
        } else if zeta < 1.0 {
            // Under-damped: decaying oscillation.
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let a = x0;
            let b = (v0 + zeta * w0 * x0) / wd;
            let e = (-zeta * w0 * t).exp();
            let (s, co) = (wd * t).sin_cos();
            let x = e * (a * co + b * s);
            let v = e * ((b * wd - zeta * w0 * a) * co - (a * wd + zeta * w0 * b) * s);
            (x, v)
        } else {
            // Over-damped: two decaying exponentials.
            let r = w0 * (zeta * zeta - 1.0).sqrt();
            let r1 = -zeta * w0 + r;
            let r2 = -zeta * w0 - r;
            let c2 = (v0 - r1 * x0) / (r2 - r1);
            let c1 = x0 - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        }
    }
}
