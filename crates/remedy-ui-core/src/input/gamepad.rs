//! Gamepads as a TV remote: the d-pad and the left stick arrive as arrow keys, A as Enter and
//! B as [`Key::Back`], and a held direction repeats as a held arrow key does. Bridges read the
//! pads (the Gamepad API, XInput, evdev) and hand each one's state to [`GamepadKeys`], which
//! turns it into key events; the app tunes it with [`GamepadTuning`].

use std::collections::BTreeMap;
use std::time::Duration;

use crate::input::{Key, KeyEvent};

/// The buttons a gamepad moves about an app with, as bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PadButtons(pub u8);

impl PadButtons {
    /// Nothing held.
    pub const NONE: PadButtons = PadButtons(0);
    /// The d-pad's up.
    pub const UP: PadButtons = PadButtons(1);
    /// The d-pad's down.
    pub const DOWN: PadButtons = PadButtons(2);
    /// The d-pad's left.
    pub const LEFT: PadButtons = PadButtons(4);
    /// The d-pad's right.
    pub const RIGHT: PadButtons = PadButtons(8);
    /// A (the bottom face button, Cross on a PlayStation pad): Enter.
    pub const A: PadButtons = PadButtons(16);
    /// B (the right face button, Circle): back.
    pub const B: PadButtons = PadButtons(32);

    /// True when every button of `other` is held here.
    pub fn contains(self, other: PadButtons) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for PadButtons {
    type Output = PadButtons;
    fn bitor(self, rhs: PadButtons) -> PadButtons {
        PadButtons(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for PadButtons {
    fn bitor_assign(&mut self, rhs: PadButtons) {
        self.0 |= rhs.0;
    }
}

/// What a pad has held at one reading.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// The buttons held.
    pub buttons: PadButtons,
    /// The left stick, each axis from -1 to 1: x grows to the right, y grows downward.
    pub stick: [f32; 2],
}

/// How gamepads move focus ([`GamepadKeys`]); every value has a default the app may change.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GamepadTuning {
    /// How long a direction is held before it repeats; `None` (the default) follows the
    /// system's keyboard repeat delay.
    pub repeat_delay: Option<Duration>,
    /// The time between repeats while it stays held; `None` (the default) follows the
    /// system's keyboard repeat rate. Repeats come at most once per reading of the pad.
    pub repeat_interval: Option<Duration>,
    /// How far, from 0 to 1, the stick must lean before it counts as a direction: 0.5 by
    /// default, so a resting stick's drift never moves focus.
    pub dead_zone: f32,
}

impl Default for GamepadTuning {
    fn default() -> Self {
        GamepadTuning { repeat_delay: None, repeat_interval: None, dead_zone: 0.5 }
    }
}

impl GamepadTuning {
    /// With its own repeat delay instead of the system keyboard's.
    pub fn with_repeat_delay(mut self, delay: Duration) -> Self {
        self.repeat_delay = Some(delay);
        self
    }

    /// With its own repeat interval instead of the system keyboard's.
    pub fn with_repeat_interval(mut self, interval: Duration) -> Self {
        self.repeat_interval = Some(interval);
        self
    }

    /// With a stick dead zone of `dead_zone` (0 to 1).
    pub fn with_dead_zone(mut self, dead_zone: f32) -> Self {
        self.dead_zone = dead_zone;
        self
    }
}

/// The keys the buttons mean, in the order their events are sent.
const KEYS: [(PadButtons, Key); 6] = [
    (PadButtons::UP, Key::ArrowUp),
    (PadButtons::DOWN, Key::ArrowDown),
    (PadButtons::LEFT, Key::ArrowLeft),
    (PadButtons::RIGHT, Key::ArrowRight),
    (PadButtons::A, Key::Enter),
    (PadButtons::B, Key::Back),
];

const ARROWS: u8 = PadButtons::UP.0 | PadButtons::DOWN.0 | PadButtons::LEFT.0 | PadButtons::RIGHT.0;

#[derive(Clone, Copy, Debug, Default)]
struct Pad {
    held: PadButtons,
    /// The direction that repeats (the last one pressed, as on a keyboard) and when next.
    repeat: Option<(PadButtons, Duration)>,
}

/// Turns pad readings into key events: presses and releases as buttons change, and repeats
/// of the last direction pressed while it stays held.
#[derive(Clone, Debug)]
pub struct GamepadKeys {
    tuning: GamepadTuning,
    /// The system keyboard's repeat delay and interval, for what the tuning leaves to it.
    system: (Duration, Duration),
    /// By the platform's pad number, which need not be small.
    pads: BTreeMap<usize, Pad>,
}

impl Default for GamepadKeys {
    fn default() -> Self {
        GamepadKeys::new()
    }
}

impl GamepadKeys {
    /// No pads yet, the default tuning, and a 500 ms delay and 33 ms interval until the
    /// system's keyboard repeat is known.
    pub fn new() -> Self {
        GamepadKeys {
            tuning: GamepadTuning::default(),
            system: (Duration::from_millis(500), Duration::from_millis(33)),
            pads: BTreeMap::new(),
        }
    }

    /// Changes the tuning.
    pub fn set_tuning(&mut self, tuning: GamepadTuning) {
        self.tuning = tuning;
    }

    /// The system keyboard's repeat delay and interval (`SystemSettings`), which the tuning
    /// follows unless it sets its own.
    pub fn set_system_repeat(&mut self, delay: Duration, interval: Duration) {
        self.system = (delay, interval);
    }

    fn delay(&self) -> Duration {
        self.tuning.repeat_delay.unwrap_or(self.system.0)
    }

    fn interval(&self) -> Duration {
        self.tuning.repeat_interval.unwrap_or(self.system.1)
    }

    /// The buttons `state` holds, with the stick as the d-pad: past the dead zone it leans
    /// one way, along whichever axis it leans further.
    pub fn buttons(&self, state: &PadState) -> PadButtons {
        let [x, y] = state.stick;
        let dz = self.tuning.dead_zone;
        let mut b = state.buttons;
        if x.abs() >= y.abs() && x.abs() >= dz && x != 0.0 {
            b |= if x > 0.0 { PadButtons::RIGHT } else { PadButtons::LEFT };
        } else if y.abs() >= dz && y != 0.0 {
            b |= if y > 0.0 { PadButtons::DOWN } else { PadButtons::UP };
        }
        b
    }

    /// Pad `pad` now reads `state` (`None`: it was unplugged, and what it held is let go).
    /// Returns the presses and releases that makes.
    pub fn update(&mut self, pad: usize, state: Option<&PadState>, now: Duration) -> Vec<KeyEvent> {
        let held = state.map_or(PadButtons::NONE, |s| self.buttons(s));
        let delay = self.delay();
        let mut p = self.pads.get(&pad).copied().unwrap_or_default();
        let mut out = Vec::new();
        for (bit, key) in KEYS {
            let was = p.held.contains(bit);
            let is = held.contains(bit);
            if was == is {
                continue;
            }
            out.push(KeyEvent::press(key).with_pressed(is));
            if is && bit.0 & ARROWS != 0 {
                p.repeat = Some((bit, now.saturating_add(delay)));
            } else if !is && p.repeat.is_some_and(|(r, _)| r == bit) {
                p.repeat = None;
            }
        }
        p.held = held;
        if state.is_some() {
            self.pads.insert(pad, p);
        } else {
            self.pads.remove(&pad);
        }
        out
    }

    /// The repeats due at `now`: one per pad whose direction is still held past its time.
    pub fn tick(&mut self, now: Duration) -> Vec<KeyEvent> {
        let interval = self.interval();
        let mut out = Vec::new();
        for p in self.pads.values_mut() {
            let Some((bit, due)) = p.repeat else { continue };
            if due > now {
                continue;
            }
            if let Some((_, key)) = KEYS.iter().find(|(b, _)| *b == bit) {
                out.push(KeyEvent::press(key.clone()).with_repeat(true));
            }
            // Late readings (a busy machine) resume the rhythm from now, not in a burst.
            let next = due.saturating_add(interval);
            p.repeat = Some((bit, if next > now { next } else { now.saturating_add(interval) }));
        }
        out
    }

    /// When the next repeat is due, if a direction is held.
    pub fn next_due(&self) -> Option<Duration> {
        self.pads.values().filter_map(|p| p.repeat.map(|(_, at)| at)).min()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "a missing event fails the test as it should")]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn pressed(b: PadButtons) -> PadState {
        PadState { buttons: b, stick: [0.0, 0.0] }
    }

    #[test]
    fn buttons_press_and_release_their_keys() {
        let mut k = GamepadKeys::new();
        let e = k.update(0, Some(&pressed(PadButtons::A | PadButtons::RIGHT)), ms(0));
        assert_eq!(
            e.iter().map(|e| (e.key.clone(), e.pressed)).collect::<Vec<_>>(),
            [(Key::ArrowRight, true), (Key::Enter, true)]
        );
        assert!(k.update(0, Some(&pressed(PadButtons::A | PadButtons::RIGHT)), ms(10)).is_empty());
        let e = k.update(0, Some(&pressed(PadButtons::NONE)), ms(20));
        assert!(e.iter().all(|e| !e.pressed) && e.len() == 2);
        let e = k.update(0, Some(&pressed(PadButtons::B)), ms(30));
        assert_eq!(e[0].key, Key::Back);
    }

    #[test]
    fn a_held_direction_repeats_after_the_delay_at_the_interval() {
        let mut k = GamepadKeys::new();
        k.set_system_repeat(ms(400), ms(50));
        k.update(0, Some(&pressed(PadButtons::DOWN)), ms(0));
        assert_eq!(k.next_due(), Some(ms(400)));
        assert!(k.tick(ms(399)).is_empty());
        let r = k.tick(ms(400));
        assert_eq!((r[0].key.clone(), r[0].pressed, r[0].repeat), (Key::ArrowDown, true, true));
        assert!(k.tick(ms(420)).is_empty());
        assert_eq!(k.tick(ms(450)).len(), 1);
        // A late reading does not burst.
        assert_eq!(k.tick(ms(900)).len(), 1);
        assert_eq!(k.next_due(), Some(ms(950)));
        k.update(0, Some(&pressed(PadButtons::NONE)), ms(910));
        assert_eq!(k.next_due(), None);
        assert!(k.tick(ms(2000)).is_empty());
    }

    #[test]
    fn the_app_tuning_wins_over_the_system_and_buttons_do_not_repeat() {
        let mut k = GamepadKeys::new();
        k.set_system_repeat(ms(400), ms(50));
        k.set_tuning(GamepadTuning::default().with_repeat_delay(ms(100)).with_repeat_interval(ms(10)));
        k.update(0, Some(&pressed(PadButtons::A)), ms(0));
        assert_eq!(k.next_due(), None, "A does not repeat");
        k.update(0, Some(&pressed(PadButtons::A | PadButtons::LEFT)), ms(0));
        assert_eq!(k.next_due(), Some(ms(100)));
        k.tick(ms(100));
        assert_eq!(k.next_due(), Some(ms(110)));
    }

    #[test]
    fn a_delay_without_end_never_repeats() {
        let mut k = GamepadKeys::new();
        k.set_tuning(GamepadTuning::default().with_repeat_delay(Duration::MAX));
        k.update(0, Some(&pressed(PadButtons::DOWN)), ms(10));
        assert!(k.tick(ms(1_000_000)).is_empty());
    }

    #[test]
    fn the_last_direction_pressed_repeats() {
        let mut k = GamepadKeys::new();
        k.update(0, Some(&pressed(PadButtons::UP)), ms(0));
        k.update(0, Some(&pressed(PadButtons::UP | PadButtons::RIGHT)), ms(100));
        assert_eq!(k.tick(ms(600))[0].key, Key::ArrowRight);
        // Letting go of up leaves right repeating; letting go of right stops it.
        k.update(0, Some(&pressed(PadButtons::RIGHT)), ms(610));
        assert!(k.next_due().is_some());
        k.update(0, Some(&pressed(PadButtons::NONE)), ms(620));
        assert_eq!(k.next_due(), None);
    }

    #[test]
    fn the_stick_leans_one_way_past_the_dead_zone() {
        let mut k = GamepadKeys::new();
        let s = |x, y| PadState { buttons: PadButtons::NONE, stick: [x, y] };
        assert_eq!(k.buttons(&s(0.3, -0.2)), PadButtons::NONE, "drift");
        assert_eq!(k.buttons(&s(0.9, 0.6)), PadButtons::RIGHT, "the stronger axis");
        assert_eq!(k.buttons(&s(-0.2, -0.7)), PadButtons::UP);
        k.set_tuning(GamepadTuning::default().with_dead_zone(0.1));
        assert_eq!(k.buttons(&s(0.0, 0.2)), PadButtons::DOWN);
        k.set_tuning(GamepadTuning::default().with_dead_zone(0.0));
        assert_eq!(k.buttons(&s(0.0, 0.0)), PadButtons::NONE, "a centered stick holds nothing");
    }

    #[test]
    fn an_unplugged_pad_lets_go_and_pads_are_apart() {
        let mut k = GamepadKeys::new();
        k.update(0, Some(&pressed(PadButtons::LEFT)), ms(0));
        k.update(2, Some(&pressed(PadButtons::LEFT)), ms(0));
        assert!(k.update(2, Some(&pressed(PadButtons::LEFT)), ms(5)).is_empty());
        let e = k.update(0, None, ms(10));
        assert_eq!((e[0].key.clone(), e[0].pressed), (Key::ArrowLeft, false));
        assert_eq!(k.tick(ms(500)).len(), 1, "pad 2 still repeats");
        // The platform's number for a pad may be anything.
        let e = k.update(usize::MAX, Some(&pressed(PadButtons::A)), ms(510));
        assert_eq!((e.len(), e[0].key.clone()), (1, Key::Enter));
    }
}
