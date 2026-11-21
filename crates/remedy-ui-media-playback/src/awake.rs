//! The screen held on while video plays ([`Player::set_keep_awake`](crate::Player::set_keep_awake)):
//! the desktop services' display inhibit with the `keep-awake` feature, nothing without it.

/// Holds the screen on, saying `reason` where the system lists what keeps it awake; `None`
/// where the system refuses or the `keep-awake` feature is off.
#[cfg(feature = "keep-awake")]
pub(crate) fn platform_hold(reason: &str) -> Option<Box<dyn std::any::Any>> {
    use remedy_ui_services_desktop::power::{Inhibit, inhibit};
    let app = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "RemedyUI".to_owned());
    // Counted with the platform objects, so the leak tests see a hold left behind.
    inhibit(Inhibit::Display, &app, reason)
        .ok()
        .map(|h| Box::new((h, crate::os::Live::new())) as Box<dyn std::any::Any>)
}

#[cfg(not(feature = "keep-awake"))]
pub(crate) fn platform_hold(_reason: &str) -> Option<Box<dyn std::any::Any>> {
    None
}

#[cfg(all(test, windows, feature = "keep-awake"))]
mod tests {
    /// Smoke test: the system's hold is a power request on Windows, taken and let go.
    #[test]
    fn windows_holds_the_display_with_a_power_request() {
        let hold = super::platform_hold("Playing video");
        assert!(hold.is_some(), "PowerCreateRequest with a display request");
        drop(hold);
        assert!(super::platform_hold("Playing video").is_some(), "taken again after release");
    }
}
