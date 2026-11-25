//! Gamepads through XInput (Xbox-compatible pads) and Windows.Gaming.Input (every other
//! controller: PlayStation, Switch Pro, generic HID gamepads), read for the shared core's
//! gamepad thread.

use std::time::{Duration, Instant};

use crate::bridge_winit::gamepad::PadSource;
use remedy_ui_core::input::{PadButtons, PadState};
use windows::Gaming::Input::{GameControllerButtonLabel as Label, GameControllerSwitchPosition as Hat};
use windows_core::HSTRING;
use windows_sys::Win32::UI::Input::XboxController::{
    XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT,
    XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
};

use crate::bridge_windows::os;

/// How often connected pads are read: neither API has events for readings, and 8 ms is
/// under a frame at 120 Hz. A sampling rate, not a cap.
const POLL: Duration = Duration::from_millis(8);
/// How often pads plugged in or unplugged are looked for (reading an empty XInput slot is
/// slow, so not every poll).
const SCAN: Duration = Duration::from_secs(1);
/// How near its center an axis must come once before it counts: a controller's resting
/// pedal or throttle (an axis that never centers) then never holds a direction.
const CENTERED: f64 = 0.25;

const SONY: u16 = 0x054C;
const NINTENDO: u16 = 0x057E;

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

/// What a button labelled `label` means; `nintendo` swaps the letters, as Nintendo's A is
/// the east face button and B the south one.
fn labelled(label: Label, nintendo: bool) -> PadButtons {
    match label {
        Label::XboxA | Label::Cross => PadButtons::A,
        Label::XboxB | Label::Circle => PadButtons::B,
        Label::LetterA if nintendo => PadButtons::B,
        Label::LetterA => PadButtons::A,
        Label::LetterB if nintendo => PadButtons::A,
        Label::LetterB => PadButtons::B,
        Label::XboxUp | Label::Up => PadButtons::UP,
        Label::XboxDown | Label::Down => PadButtons::DOWN,
        Label::XboxLeft | Label::Left => PadButtons::LEFT,
        Label::XboxRight | Label::Right => PadButtons::RIGHT,
        _ => PadButtons::NONE,
    }
}

/// A Windows.Gaming.Input controller's reading: its buttons by their labels (or, when the
/// driver labels none, the south and east face buttons where the vendor's HID report puts
/// them: Cross and Circle second and third on Sony's, first and second elsewhere), its
/// first switch as the d-pad, and its first two axes (0 to 1, y growing downward) as the
/// stick. `centered` says which axes have been near their center yet; one that has not
/// stays at rest.
pub(crate) fn raw_state(
    vendor: u16,
    labels: &[Label],
    buttons: &[bool],
    hat: Hat,
    axes: &[f64],
    centered: &mut [bool; 2],
) -> PadState {
    let any_label = labels.iter().any(|l| *l != Label::None);
    let (south, east) = if vendor == SONY { (1, 2) } else { (0, 1) };
    let mut b = PadButtons::NONE;
    for (i, _) in buttons.iter().enumerate().filter(|(_, held)| **held) {
        b |= match labels.get(i).copied().unwrap_or(Label::None) {
            Label::None if !any_label && i == south => PadButtons::A,
            Label::None if !any_label && i == east => PadButtons::B,
            label => labelled(label, vendor == NINTENDO),
        };
    }
    b |= match hat {
        Hat::Up => PadButtons::UP,
        Hat::UpRight => PadButtons::UP | PadButtons::RIGHT,
        Hat::Right => PadButtons::RIGHT,
        Hat::DownRight => PadButtons::DOWN | PadButtons::RIGHT,
        Hat::Down => PadButtons::DOWN,
        Hat::DownLeft => PadButtons::DOWN | PadButtons::LEFT,
        Hat::Left => PadButtons::LEFT,
        Hat::UpLeft => PadButtons::UP | PadButtons::LEFT,
        _ => PadButtons::NONE,
    };
    let mut stick = [0.0; 2];
    for ((s, seen), v) in stick.iter_mut().zip(centered.iter_mut()).zip(axes) {
        let v = (v * 2.0 - 1.0).clamp(-1.0, 1.0);
        *seen |= v.abs() <= CENTERED;
        if *seen {
            *s = v as f32;
        }
    }
    PadState { buttons: b, stick }
}

/// A Windows.Gaming.Input controller in its slot.
struct RawPad {
    slot: usize,
    id: HSTRING,
    pad: os::gamepad::Raw,
    centered: [bool; 2],
}

/// Every pad Windows reads: XInput's four slots first, then the other controllers in the
/// slots after them.
#[derive(Default)]
pub(crate) struct WindowsPads {
    connected: [bool; os::gamepad::SLOTS as usize],
    raw: Vec<RawPad>,
    scanned: Option<Instant>,
}

impl WindowsPads {
    /// Takes in the controllers plugged in since the last scan and lets go of the ones gone.
    fn scan(&mut self, out: &mut Vec<(usize, Option<PadState>)>) {
        let found = os::gamepad::controllers();
        self.raw.retain(|p| {
            let kept = found.iter().any(|(id, _)| *id == p.id);
            if !kept {
                out.push((p.slot, None));
            }
            kept
        });
        for (id, c) in found {
            if self.raw.iter().any(|p| p.id == id) {
                continue;
            }
            let first = os::gamepad::SLOTS as usize;
            let slot = (first..).find(|s| self.raw.iter().all(|p| p.slot != *s)).unwrap_or(first);
            self.raw.push(RawPad { slot, id, pad: os::gamepad::Raw::new(c), centered: [false; 2] });
        }
    }
}

impl PadSource for WindowsPads {
    fn read(&mut self, wait: Option<Duration>) -> Vec<(usize, Option<PadState>)> {
        let any = self.connected.iter().any(|c| *c) || !self.raw.is_empty();
        let period = if any { POLL } else { SCAN };
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
        if scan {
            self.scan(&mut out);
        }
        for p in &mut self.raw {
            if let Some((vendor, labels, buttons, hat, axes)) = p.pad.read() {
                out.push((p.slot, Some(raw_state(vendor, labels, buttons, hat, axes, &mut p.centered))));
            }
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

    fn raw(vendor: u16, labels: &[Label], held: &[usize], hat: Hat, axes: &[f64]) -> PadState {
        let n = labels.len().max(held.iter().max().map_or(0, |m| m + 1));
        let buttons: Vec<bool> = (0..n).map(|i| held.contains(&i)).collect();
        raw_state(vendor, labels, &buttons, hat, axes, &mut [true; 2])
    }

    #[test]
    fn labelled_buttons_mean_their_face_position() {
        let ps = [Label::Square, Label::Cross, Label::Circle, Label::Triangle];
        assert_eq!(raw(0, &ps, &[1], Hat::Center, &[]).buttons, PadButtons::A, "Cross");
        assert_eq!(raw(0, &ps, &[2], Hat::Center, &[]).buttons, PadButtons::B, "Circle");
        assert_eq!(raw(0, &ps, &[0, 3], Hat::Center, &[]).buttons, PadButtons::NONE);
        // Nintendo's B is south (Enter) and its A east (back); elsewhere letters are Xbox's.
        let letters = [Label::LetterB, Label::LetterA];
        assert_eq!(raw(NINTENDO, &letters, &[0], Hat::Center, &[]).buttons, PadButtons::A);
        assert_eq!(raw(NINTENDO, &letters, &[1], Hat::Center, &[]).buttons, PadButtons::B);
        assert_eq!(raw(0x1234, &letters, &[0], Hat::Center, &[]).buttons, PadButtons::B);
        let dpad = [Label::Up, Label::Down, Label::Left, Label::Right];
        assert_eq!(raw(0, &dpad, &[1, 3], Hat::Center, &[]).buttons, PadButtons::DOWN | PadButtons::RIGHT);
    }

    #[test]
    fn unlabelled_buttons_follow_the_vendor_layout() {
        let none = [Label::None; 4];
        assert_eq!(raw(SONY, &none, &[1], Hat::Center, &[]).buttons, PadButtons::A, "Cross is second");
        assert_eq!(raw(SONY, &none, &[2], Hat::Center, &[]).buttons, PadButtons::B);
        assert_eq!(raw(SONY, &none, &[0], Hat::Center, &[]).buttons, PadButtons::NONE, "Square");
        assert_eq!(raw(NINTENDO, &none, &[0], Hat::Center, &[]).buttons, PadButtons::A, "B is first");
        assert_eq!(raw(0x0079, &none, &[1], Hat::Center, &[]).buttons, PadButtons::B);
    }

    #[test]
    fn the_hat_is_the_dpad_and_the_first_axes_the_stick() {
        let s = raw(0, &[], &[], Hat::UpLeft, &[1.0, 0.5, 0.0]);
        assert_eq!(s.buttons, PadButtons::UP | PadButtons::LEFT);
        assert_eq!(s.stick, [1.0, 0.0]);
        assert_eq!(raw(0, &[], &[], Hat::Center, &[0.0, 1.0]).stick, [-1.0, 1.0], "HID y grows down");
        assert_eq!(raw(0, &[], &[], Hat::Center, &[]).stick, [0.0, 0.0], "no axes");
    }

    #[test]
    fn an_axis_counts_once_it_has_centered() {
        let mut centered = [false; 2];
        let read = |x: f64, c: &mut [bool; 2]| raw_state(0, &[], &[], Hat::Center, &[x, 0.5], c).stick;
        assert_eq!(read(0.0, &mut centered), [0.0, 0.0], "a pedal at rest holds nothing");
        assert_eq!(centered, [false, true]);
        assert_eq!(read(0.5, &mut centered), [0.0, 0.0]);
        assert_eq!(read(1.0, &mut centered), [1.0, 0.0]);
    }

    #[test]
    fn the_keyboard_repeat_comes_from_control_panel_steps() {
        assert_eq!(os::key_repeat(1, 31), (Duration::from_millis(500), Duration::from_secs_f64(1.0 / 30.0)));
        assert_eq!(os::key_repeat(0, 0), (Duration::from_millis(250), Duration::from_millis(400)));
        assert_eq!(os::key_repeat(9, 99).0, Duration::from_secs(1), "out of range is the slowest step");
    }

    #[test]
    fn reading_the_pads_works_with_or_without_one() {
        // Whatever is plugged in here, readings come in XInput's slots or the ones after,
        // and a second scan finds the same controllers in the same slots.
        let mut p = WindowsPads::default();
        p.read(Some(Duration::ZERO));
        let slots: Vec<usize> = p.raw.iter().map(|r| r.slot).collect();
        assert!(slots.iter().all(|s| *s >= 4), "{slots:?}");
        p.scanned = None;
        p.read(Some(Duration::ZERO));
        assert_eq!(p.raw.iter().map(|r| r.slot).collect::<Vec<_>>(), slots);
    }
}
