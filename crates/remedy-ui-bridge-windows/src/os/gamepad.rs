//! XInput: the Xbox-compatible pads Windows reads.

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};

/// How many pads XInput serves.
pub(crate) const SLOTS: u32 = 4;

/// Pad `slot`'s buttons (`XINPUT_GAMEPAD_*`) and left stick, or `None` when none is plugged
/// into that slot.
pub(crate) fn state(slot: u32) -> Option<(u16, i16, i16)> {
    let mut s = XINPUT_STATE::default();
    // SAFETY: `s` is a valid XINPUT_STATE for XInput to fill.
    let r = unsafe { XInputGetState(slot, &raw mut s) };
    (r == ERROR_SUCCESS).then_some((s.Gamepad.wButtons, s.Gamepad.sThumbLX, s.Gamepad.sThumbLY))
}
