//! What a Linux gamepad's evdev events mean: its d-pad (buttons or a hat), left stick, A and B
//! as one [`PadState`]. Plain data, so it is tested on every platform.

use remedy_ui_core::input::{PadButtons, PadState};

pub(crate) const EV_SYN: u16 = 0x00;
pub(crate) const EV_KEY: u16 = 0x01;
pub(crate) const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
/// A (`BTN_A`): every pad the kernel calls a gamepad has it.
pub(crate) const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;
pub(crate) const ABS_X: u16 = 0x00;
pub(crate) const ABS_Y: u16 = 0x01;
pub(crate) const ABS_HAT0X: u16 = 0x10;
pub(crate) const ABS_HAT0Y: u16 = 0x11;
/// The axes a pad's reading uses, in [`Pad::resync`]'s order.
pub(crate) const AXES: [u16; 4] = [ABS_X, ABS_Y, ABS_HAT0X, ABS_HAT0Y];
/// Bytes in a key bitmask (`KEY_MAX` is 0x2ff).
pub(crate) const KEY_BYTES: usize = 0x300 / 8;

const BUTTONS: [(u16, PadButtons); 6] = [
    (BTN_SOUTH, PadButtons::A),
    (BTN_EAST, PadButtons::B),
    (BTN_DPAD_UP, PadButtons::UP),
    (BTN_DPAD_DOWN, PadButtons::DOWN),
    (BTN_DPAD_LEFT, PadButtons::LEFT),
    (BTN_DPAD_RIGHT, PadButtons::RIGHT),
];

fn bit(bits: &[u8], code: u16) -> bool {
    bits.get(usize::from(code / 8)).is_some_and(|b| b & (1 << (code % 8)) != 0)
}

/// True for a device whose keys include a gamepad's A (`EVIOCGBIT(EV_KEY)`).
pub(crate) fn is_gamepad(key_bits: &[u8]) -> bool {
    bit(key_bits, BTN_SOUTH)
}

/// One pad's state, built from its events.
#[derive(Clone, Debug, Default)]
pub(crate) struct Pad {
    keys: PadButtons,
    hat: [i32; 2],
    stick: [f32; 2],
    /// Each stick axis's range (`EVIOCGABS`); `None` when the pad has no such axis.
    range: [Option<(i32, i32)>; 2],
    changed: bool,
    /// Events were dropped: what comes before the next report is incomplete.
    dropped: bool,
}

impl Pad {
    /// A pad whose stick axes span `range` (x, then y).
    pub(crate) fn new(range: [Option<(i32, i32)>; 2]) -> Pad {
        Pad { range, ..Pad::default() }
    }

    /// The reading now.
    pub(crate) fn state(&self) -> PadState {
        let mut b = self.keys;
        match self.hat[0] {
            x if x < 0 => b |= PadButtons::LEFT,
            x if x > 0 => b |= PadButtons::RIGHT,
            _ => {}
        }
        match self.hat[1] {
            y if y < 0 => b |= PadButtons::UP,
            y if y > 0 => b |= PadButtons::DOWN,
            _ => {}
        }
        PadState { buttons: b, stick: self.stick }
    }

    fn axis(&mut self, i: usize, value: i32) {
        let Some(Some((min, max))) = self.range.get(i) else { return };
        if max <= min {
            return;
        }
        let mid = (f64::from(*min) + f64::from(*max)) / 2.0;
        let half = (f64::from(*max) - f64::from(*min)) / 2.0;
        if let Some(s) = self.stick.get_mut(i) {
            *s = ((f64::from(value) - mid) / half).clamp(-1.0, 1.0) as f32;
        }
    }

    fn set(&mut self, code: u16, value: i32) {
        match code {
            ABS_X => self.axis(0, value),
            ABS_Y => self.axis(1, value),
            ABS_HAT0X => self.hat[0] = value,
            ABS_HAT0Y => self.hat[1] = value,
            _ => {}
        }
    }

    /// Takes one event, and says whether a report just ended that changed the reading, or
    /// that events were dropped and the caller should [`Pad::resync`] from the device.
    pub(crate) fn event(&mut self, ty: u16, code: u16, value: i32) -> Event {
        match (ty, code) {
            (EV_SYN, SYN_DROPPED) => {
                self.dropped = true;
                Event::None
            }
            (EV_SYN, SYN_REPORT) if self.dropped => {
                self.dropped = false;
                Event::Resync
            }
            (EV_SYN, SYN_REPORT) => {
                if std::mem::take(&mut self.changed) {
                    Event::Changed
                } else {
                    Event::None
                }
            }
            _ if self.dropped => Event::None,
            (EV_KEY, _) => {
                if let Some((_, b)) = BUTTONS.iter().find(|(c, _)| *c == code) {
                    // 2 is the kernel's own autorepeat: still held.
                    self.keys = if value != 0 { self.keys | *b } else { PadButtons(self.keys.0 & !b.0) };
                    self.changed = true;
                }
                Event::None
            }
            (EV_ABS, _) => {
                self.set(code, value);
                self.changed = true;
                Event::None
            }
            _ => Event::None,
        }
    }

    /// Replaces the state with the device's own: `key_state` from `EVIOCGKEY` and the
    /// [`AXES`]' values from `EVIOCGABS` (after dropped events, and when a pad is opened with
    /// something held).
    pub(crate) fn resync(&mut self, key_state: &[u8], axes: [Option<i32>; 4]) {
        self.keys = PadButtons::NONE;
        for (code, b) in BUTTONS {
            if bit(key_state, code) {
                self.keys |= b;
            }
        }
        for (code, v) in AXES.iter().zip(axes) {
            match v {
                Some(v) => self.set(*code, v),
                None if *code == ABS_HAT0X => self.hat[0] = 0,
                None if *code == ABS_HAT0Y => self.hat[1] = 0,
                None => {}
            }
        }
        self.changed = false;
    }
}

/// What an event did ([`Pad::event`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    None,
    /// A report ended with the reading changed.
    Changed,
    /// Events were lost; read the state from the device.
    Resync,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(codes: &[u16]) -> Vec<u8> {
        let mut b = vec![0u8; KEY_BYTES];
        for c in codes {
            if let Some(x) = b.get_mut(usize::from(c / 8)) {
                *x |= 1 << (c % 8);
            }
        }
        b
    }

    #[test]
    fn a_gamepad_has_a_south_button() {
        assert!(is_gamepad(&bits(&[BTN_SOUTH, BTN_EAST])));
        assert!(!is_gamepad(&bits(&[30, 31])), "a keyboard");
        assert!(!is_gamepad(&[]));
    }

    #[test]
    fn buttons_hats_and_the_stick_make_a_reading_at_each_report() {
        let mut p = Pad::new([Some((-32768, 32767)), Some((0, 255))]);
        assert_eq!(p.event(EV_KEY, BTN_SOUTH, 1), Event::None);
        assert_eq!(p.event(EV_SYN, SYN_REPORT, 0), Event::Changed);
        assert_eq!(p.state().buttons, PadButtons::A);
        assert_eq!(p.event(EV_SYN, SYN_REPORT, 0), Event::None, "nothing new");
        p.event(EV_KEY, BTN_SOUTH, 0);
        p.event(EV_ABS, ABS_HAT0X, 1);
        p.event(EV_ABS, ABS_HAT0Y, -1);
        p.event(EV_ABS, ABS_Y, 255);
        p.event(EV_ABS, ABS_X, -32768);
        assert_eq!(p.event(EV_SYN, SYN_REPORT, 0), Event::Changed);
        let s = p.state();
        assert_eq!(s.buttons, PadButtons::RIGHT | PadButtons::UP);
        assert_eq!(s.stick, [-1.0, 1.0]);
        p.event(EV_KEY, BTN_DPAD_DOWN, 1);
        p.event(EV_KEY, BTN_EAST, 2);
        p.event(EV_SYN, SYN_REPORT, 0);
        assert!(p.state().buttons.contains(PadButtons::DOWN | PadButtons::B));
        p.event(EV_KEY, 0x13c, 1);
        assert_eq!(p.event(EV_SYN, SYN_REPORT, 0), Event::None, "the mode button means nothing here");
    }

    #[test]
    fn dropped_events_ask_for_the_device_state() {
        let mut p = Pad::new([None, None]);
        p.event(EV_SYN, SYN_DROPPED, 0);
        p.event(EV_KEY, BTN_SOUTH, 1);
        assert_eq!(p.event(EV_SYN, SYN_REPORT, 0), Event::Resync);
        assert_eq!(p.state().buttons, PadButtons::NONE, "what came after the drop is ignored");
        p.resync(&bits(&[BTN_EAST]), [Some(5), None, None, Some(1)]);
        assert_eq!(p.state().buttons, PadButtons::B | PadButtons::DOWN);
        assert_eq!(p.state().stick, [0.0, 0.0], "no stick range");
    }
}
