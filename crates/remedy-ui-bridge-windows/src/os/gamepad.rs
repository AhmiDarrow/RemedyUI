//! The pads Windows reads: XInput's Xbox-compatible slots, and every other controller
//! (PlayStation, Switch Pro, generic HID gamepads) through Windows.Gaming.Input.

use windows::Gaming::Input::{
    GameControllerButtonLabel, GameControllerSwitchPosition, Gamepad, RawGameController,
};
use windows_core::HSTRING;
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

/// The connected controllers XInput does not serve, by their id. Windows.Gaming.Input reads
/// Xbox-class pads as a `Gamepad` too, and XInput already reads those, so they are left out
/// and no pad is reported twice. Without Windows.Gaming.Input (older Windows), none.
pub(crate) fn controllers() -> Vec<(HSTRING, RawGameController)> {
    let Ok(list) = RawGameController::RawGameControllers() else { return Vec::new() };
    let n = list.Size().unwrap_or(0);
    (0..n)
        .filter_map(|i| list.GetAt(i).ok())
        .filter(|c| Gamepad::FromGameController(c).is_err())
        .filter_map(|c| Some((c.NonRoamableId().ok()?, c)))
        .collect()
}

/// A controller read through Windows.Gaming.Input, with room for its readings.
pub(crate) struct Raw {
    controller: RawGameController,
    /// The USB vendor, for controllers whose buttons have no labels.
    vendor: u16,
    /// Each button's label (`None` where the driver gives none).
    labels: Vec<GameControllerButtonLabel>,
    buttons: Vec<bool>,
    switches: Vec<GameControllerSwitchPosition>,
    axes: Vec<f64>,
}

impl Raw {
    pub(crate) fn new(controller: RawGameController) -> Raw {
        let count = |n: windows_core::Result<i32>| n.ok().and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
        let buttons = count(controller.ButtonCount());
        let labels = (0..buttons)
            .map(|i| {
                i32::try_from(i)
                    .ok()
                    .and_then(|i| controller.GetButtonLabel(i).ok())
                    .unwrap_or(GameControllerButtonLabel::None)
            })
            .collect();
        Raw {
            vendor: controller.HardwareVendorId().unwrap_or(0),
            labels,
            buttons: vec![false; buttons],
            switches: vec![GameControllerSwitchPosition::Center; count(controller.SwitchCount())],
            axes: vec![0.0; count(controller.AxisCount())],
            controller,
        }
    }

    /// The vendor, the buttons' labels, the buttons held, the first switch (the d-pad, as a
    /// hat) and the axes (0 to 1), or `None` when the controller could not be read.
    #[allow(clippy::type_complexity, reason = "the reading's parts, as the API gives them")]
    pub(crate) fn read(
        &mut self,
    ) -> Option<(u16, &[GameControllerButtonLabel], &[bool], GameControllerSwitchPosition, &[f64])> {
        self.controller.GetCurrentReading(&mut self.buttons, &mut self.switches, &mut self.axes).ok()?;
        let hat = self.switches.first().copied().unwrap_or(GameControllerSwitchPosition::Center);
        Some((self.vendor, &self.labels, &self.buttons, hat, &self.axes))
    }
}
