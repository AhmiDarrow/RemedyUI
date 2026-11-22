//! Gamepads through XInput, read for the shared core's gamepad thread.

use std::time::{Duration, Instant};

use remedy_ui_bridge_winit::gamepad::PadSource;
use remedy_ui_input::{PadButtons, PadState};
use windows_sys::Win32::UI::Input::XboxController::{
    XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT,
    XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
};

use crate::os;

/// How often connected pads are read: XInput has no events, and 8 ms is under a frame at
/// 120 Hz. A sampling rate, not a cap.
const POLL: Duration = Duration::from_millis(8);
/// How often empty slots are looked at for a pad plugged in (XInput's hot-plug): reading an
/// empty slot is slow, so not every poll.
const SCAN: Duration = Duration::from_secs(1);

/// A pad's reading from XInput's buttons and left stick (whose y grows upward).
pub(crate) fn pad_state(buttons: u16, x: i16, y: i16) -> PadState {
    let mut b = PadButtons::NONE;
    for (bit, pad) in [
        (XINPUT_GAMEPAD_DPAD_UP, PadButtons::UP),
        (XINPUT_GAMEPAD_DPAD_DOWN, PadButtons::DOWN),
        (XINPUT_GAMEPAD_DPAD_LEFT, PadButtons::LEFT),
        (XINPUT_GAMEPAD_DPAD_RIGHT, PadButtons::RIGHT),
        (XINPUT_GAMEPAD_A, PadButtons::A),
        (XINPUT_GAMEPAD_B, PadButtons::B),
    ] {
        if buttons & bit != 0 {
            b |= pad;
        }
    }
    let axis = |v: i16| (f32::from(v) / 32767.0).clamp(-1.0, 1.0);
    PadState { buttons: b, stick: [axis(x), -axis(y)] }
}

/// XInput's four slots.
#[derive(Default)]
pub(crate) struct XInput {
    connected: [bool; os::gamepad::SLOTS as usize],
    scanned: Option<Instant>,
}

impl PadSource for XInput {
    fn read(&mut self, wait: Option<Duration>) -> Vec<(usize, Option<PadState>)> {
        let period = if self.connected.iter().any(|c| *c) { POLL } else { SCAN };
        std::thread::sleep(wait.map_or(period, |w| w.min(period)));
        let scan = self.scanned.is_none_or(|t| t.elapsed() >= SCAN);
        if scan {
            self.scanned = Some(Instant::now());
        }
        let mut out = Vec::new();
        for (slot, connected) in self.connected.iter_mut().enumerate() {
            if !*connected && !scan {
                continue;
            }
            let state =
                u32::try_from(slot).ok().and_then(os::gamepad::state).map(|(b, x, y)| pad_state(b, x, y));
            if state.is_some() || *connected {
                out.push((slot, state));
            }
            *connected = state.is_some();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xinput_buttons_and_stick_become_a_reading() {
        let s = pad_state(XINPUT_GAMEPAD_A | XINPUT_GAMEPAD_DPAD_LEFT, 32767, 32767);
        assert_eq!(s.buttons, PadButtons::A | PadButtons::LEFT);
        assert_eq!(s.stick, [1.0, -1.0], "XInput's up is the reading's negative y");
        assert_eq!(pad_state(XINPUT_GAMEPAD_B, i16::MIN, 0).stick, [-1.0, 0.0]);
        assert_eq!(pad_state(0x0010, 0, 0).buttons, PadButtons::NONE, "Start means nothing here");
    }

    #[test]
    fn the_keyboard_repeat_comes_from_control_panel_steps() {
        assert_eq!(os::key_repeat(1, 31), (Duration::from_millis(500), Duration::from_secs_f64(1.0 / 30.0)));
        assert_eq!(os::key_repeat(0, 0), (Duration::from_millis(250), Duration::from_millis(400)));
        assert_eq!(os::key_repeat(9, 99).0, Duration::from_secs(1), "out of range is the slowest step");
    }

    #[test]
    fn reading_the_slots_works_with_or_without_a_pad() {
        // Whatever is plugged in here, every reading is of a real slot.
        let mut x = XInput::default();
        assert!(x.read(Some(Duration::ZERO)).iter().all(|(slot, _)| *slot < 4));
    }
}
