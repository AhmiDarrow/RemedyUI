//! Velocity tracking and the gesture arena.

use std::time::Duration;

use remedy_ui_core::{Point, Vec2};

use crate::events::{PointerEvent, PointerKind, PointerPhase};

/// How far back samples count for velocity.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

/// Estimates a pointer's velocity from its recent positions (least squares over the last
/// 100 ms), the way flings and springs need it.
#[derive(Clone, Debug, Default)]
pub struct VelocityTracker {
    samples: Vec<(Point, Duration)>,
}

impl VelocityTracker {
    /// An empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a position at a time.
    pub fn add(&mut self, p: Point, t: Duration) {
        self.samples.push((p, t));
        let cutoff = t.saturating_sub(VELOCITY_WINDOW);
        self.samples.retain(|(_, ts)| *ts >= cutoff);
    }

    /// Adds an event's coalesced history and its own position.
    pub fn add_event(&mut self, e: &PointerEvent) {
        for (p, t) in &e.coalesced {
            self.add(*p, *t);
        }
        self.add(e.position, e.time);
    }

    /// Velocity in logical pixels per second.
    pub fn velocity(&self) -> Vec2 {
        let n = self.samples.len();
        if n < 2 {
            return Vec2::ZERO;
        }
        let t0 = self.samples.first().map(|(_, t)| *t).unwrap_or_default();
        let ts: Vec<f32> = self.samples.iter().map(|(_, t)| t.saturating_sub(t0).as_secs_f32()).collect();
        let mean_t = ts.iter().sum::<f32>() / n as f32;
        let var: f32 = ts.iter().map(|t| (t - mean_t).powi(2)).sum();
        if var <= f32::EPSILON {
            return Vec2::ZERO;
        }
        let slope = |coord: fn(&Point) -> f32| {
            let vals: Vec<f32> = self.samples.iter().map(|(p, _)| coord(p)).collect();
            let mean = vals.iter().sum::<f32>() / n as f32;
            ts.iter().zip(&vals).map(|(t, v)| (t - mean_t) * (v - mean)).sum::<f32>() / var
        };
        Vec2::new(slope(|p| p.x), slope(|p| p.y))
    }
}

/// Platform thresholds for telling gestures apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureSettings {
    /// How far a finger may move before a tap becomes a drag.
    pub touch_slop: f32,
    /// How far a mouse may move before a click becomes a drag.
    pub mouse_slop: f32,
    /// How long a press must last to be a long press.
    pub long_press: Duration,
    /// Longest gap between taps of a double tap.
    pub double_tap: Duration,
}

impl Default for GestureSettings {
    fn default() -> Self {
        GestureSettings {
            touch_slop: 8.0,
            mouse_slop: 3.0,
            long_press: Duration::from_millis(500),
            double_tap: Duration::from_millis(300),
        }
    }
}

/// What a recognizer wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// Undecided: keep watching.
    Possible,
    /// This is my gesture.
    Accept,
    /// Not my gesture.
    Reject,
}

/// The kinds of gesture the built-in recognizer tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    /// A press and release without moving past the slop.
    Tap,
    /// A press held past the long-press delay without moving.
    LongPress,
    /// A drag in any direction.
    Pan,
    /// A drag that started mostly horizontally.
    HorizontalPan,
    /// A drag that started mostly vertically.
    VerticalPan,
}

/// One recognizer in an arena: which gesture it wants, and its claim on the pointer so far.
#[derive(Clone, Debug)]
pub struct Recognizer {
    /// An id the owner chose (the element that registered it).
    pub owner: u64,
    /// The gesture it recognizes.
    pub kind: GestureKind,
    claim: Claim,
}

impl Recognizer {
    /// A recognizer for `kind`, owned by `owner`.
    pub fn new(owner: u64, kind: GestureKind) -> Self {
        Recognizer { owner, kind, claim: Claim::Possible }
    }
}

/// The arena for one pointer: every recognizer on the hit path joins when the pointer goes
/// down, and they compete until one wins. A swipe on a row and the list's scroll never both
/// fire; a button inside a draggable card still clicks.
#[derive(Clone, Debug)]
pub struct GestureArena {
    settings: GestureSettings,
    recognizers: Vec<Recognizer>,
    start: Option<(Point, Duration, PointerKind)>,
    winner: Option<usize>,
}

impl GestureArena {
    /// An arena with the platform's thresholds.
    pub fn new(settings: GestureSettings) -> Self {
        GestureArena { settings, recognizers: Vec::new(), start: None, winner: None }
    }

    /// Adds a recognizer, innermost element first: on a tie, the earliest added wins.
    pub fn join(&mut self, r: Recognizer) {
        self.recognizers.push(r);
    }

    /// The winning recognizer, once decided.
    pub fn winner(&self) -> Option<&Recognizer> {
        self.winner.and_then(|i| self.recognizers.get(i))
    }

    /// Feeds an event; returns the winner when this event decided it.
    pub fn handle(&mut self, e: &PointerEvent) -> Option<&Recognizer> {
        if self.winner.is_some() {
            return None;
        }
        match e.phase {
            PointerPhase::Down => {
                self.start = Some((e.position, e.time, e.kind));
            }
            PointerPhase::Move => {
                let (start, _, kind) = self.start?;
                let d = e.position - start;
                let slop = if kind == PointerKind::Touch {
                    self.settings.touch_slop
                } else {
                    self.settings.mouse_slop
                };
                if d.length() > slop {
                    let horizontal = d.x.abs() > d.y.abs();
                    for r in &mut self.recognizers {
                        r.claim = match r.kind {
                            GestureKind::Tap | GestureKind::LongPress => Claim::Reject,
                            GestureKind::Pan => Claim::Accept,
                            GestureKind::HorizontalPan if horizontal => Claim::Accept,
                            GestureKind::VerticalPan if !horizontal => Claim::Accept,
                            GestureKind::HorizontalPan | GestureKind::VerticalPan => Claim::Reject,
                        };
                    }
                } else if e.time.saturating_sub(self.start.map(|s| s.1).unwrap_or_default())
                    >= self.settings.long_press
                {
                    for r in &mut self.recognizers {
                        r.claim = if r.kind == GestureKind::LongPress { Claim::Accept } else { r.claim };
                    }
                }
            }
            PointerPhase::Up => {
                for r in &mut self.recognizers {
                    r.claim = match r.kind {
                        GestureKind::Tap if r.claim == Claim::Possible => Claim::Accept,
                        _ if r.claim == Claim::Possible => Claim::Reject,
                        _ => r.claim,
                    };
                }
            }
            PointerPhase::Cancel => {
                for r in &mut self.recognizers {
                    r.claim = Claim::Reject;
                }
            }
        }
        self.decide()
    }

    /// Checks the long-press timer at `now` (no event needed).
    pub fn tick(&mut self, now: Duration) -> Option<&Recognizer> {
        if self.winner.is_some() {
            return None;
        }
        let (_, t0, _) = self.start?;
        if now.saturating_sub(t0) >= self.settings.long_press {
            for r in &mut self.recognizers {
                if r.kind == GestureKind::LongPress && r.claim == Claim::Possible {
                    r.claim = Claim::Accept;
                }
            }
        }
        self.decide()
    }

    fn decide(&mut self) -> Option<&Recognizer> {
        let first_accept = self.recognizers.iter().position(|r| r.claim == Claim::Accept)?;
        self.winner = Some(first_accept);
        self.recognizers.get(first_accept)
    }
}
