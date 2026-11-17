//! Scroll physics: flings that decay, and overscroll that springs back.

/// Per-platform feel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollPhysics {
    /// How quickly a fling slows: the fraction of velocity left after one second (0..1).
    pub decay_per_second: f32,
    /// Below this speed (px/s) a fling stops.
    pub stop_speed: f32,
    /// How far past the edge content may be pulled, as a fraction of the drag (0 for none).
    pub overscroll_resistance: f32,
}

impl ScrollPhysics {
    /// Android and Windows touch: fling decay, no rubber band.
    pub const CLAMPED: ScrollPhysics =
        ScrollPhysics { decay_per_second: 0.06, stop_speed: 20.0, overscroll_resistance: 0.0 };
    /// Apple platforms: fling decay with rubber banding at the edges.
    pub const BOUNCING: ScrollPhysics =
        ScrollPhysics { decay_per_second: 0.06, stop_speed: 20.0, overscroll_resistance: 0.45 };
}

/// One scroll axis with momentum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollAxis {
    /// Current offset.
    pub offset: f32,
    /// Largest offset (content minus viewport).
    pub max: f32,
    /// Current velocity in px/s.
    pub velocity: f32,
    physics: ScrollPhysics,
}

impl ScrollAxis {
    /// A resting axis.
    pub fn new(max: f32, physics: ScrollPhysics) -> Self {
        ScrollAxis { offset: 0.0, max: max.max(0.0), velocity: 0.0, physics }
    }

    /// Moves by a drag or wheel delta. Past the edges, bouncing physics give resistance;
    /// clamped physics stop at the edge.
    pub fn drag(&mut self, delta: f32) {
        self.velocity = 0.0;
        let next = self.offset + delta;
        self.offset = if next < 0.0 || next > self.max {
            if self.physics.overscroll_resistance > 0.0 {
                let past = if next < 0.0 { next } else { next - self.max };
                let edge = if next < 0.0 { 0.0 } else { self.max };
                edge + past * self.physics.overscroll_resistance
            } else {
                next.clamp(0.0, self.max)
            }
        } else {
            next
        };
    }

    /// Starts a fling with the release velocity.
    pub fn fling(&mut self, velocity: f32) {
        self.velocity = velocity;
    }

    /// Advances `dt` seconds. Returns true while moving (the window should keep animating).
    pub fn step(&mut self, dt: f32) -> bool {
        // Back inside the edges after an overscroll: spring back quickly.
        if self.offset < 0.0 || self.offset > self.max {
            let edge = self.offset.clamp(0.0, self.max);
            let back = 1.0 - (-dt * 12.0).exp();
            self.offset += (edge - self.offset) * back;
            self.velocity = 0.0;
            if (self.offset - edge).abs() < 0.5 {
                self.offset = edge;
                return false;
            }
            return true;
        }
        if self.velocity.abs() < self.physics.stop_speed {
            self.velocity = 0.0;
            return false;
        }
        // Exponential decay: v(t) = v0·k^t, distance = v0·(k^t − 1)/ln k.
        let k = self.physics.decay_per_second.clamp(1e-4, 0.999);
        let ln_k = k.ln();
        let v0 = self.velocity;
        let v1 = v0 * k.powf(dt);
        self.offset += (v1 - v0) / ln_k;
        self.velocity = v1;
        if self.offset < 0.0 || self.offset > self.max {
            if self.physics.overscroll_resistance == 0.0 {
                self.offset = self.offset.clamp(0.0, self.max);
                self.velocity = 0.0;
                return false;
            }
            self.velocity *= self.physics.overscroll_resistance;
        }
        true
    }
}
