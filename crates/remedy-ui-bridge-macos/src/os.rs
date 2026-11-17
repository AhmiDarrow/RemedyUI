//! AppKit calls: the only unsafe code in the macOS bridge.

pub(crate) mod tray;

use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSColor, NSColorSpace, NSEvent,
    NSPasteboard, NSPasteboardTypeString, NSView, NSWindowSharingType, NSWorkspace,
};
use objc2_foundation::{NSArray, NSLocale, NSString};
use remedy_ui_core::Color;
use remedy_ui_host::Unsupported;

/// The pasteboard's plain text.
pub(crate) fn clipboard_text() -> Option<String> {
    let pb = NSPasteboard::generalPasteboard();
    // SAFETY: NSPasteboardTypeString is an immutable framework constant.
    let ty = unsafe { NSPasteboardTypeString };
    pb.stringForType(ty).map(|s| s.to_string())
}

/// Writes plain text. Sensitive text also carries `org.nspasteboard.ConcealedType` and
/// `TransientType`, the shared convention clipboard managers honor to skip history.
pub(crate) fn set_clipboard_text(text: &str, sensitive: bool) -> Result<(), Unsupported> {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    // SAFETY: NSPasteboardTypeString is an immutable framework constant.
    let ty = unsafe { NSPasteboardTypeString };
    if !pb.setString_forType(&NSString::from_str(text), ty) {
        return Err(Unsupported::new("the clipboard", "the pasteboard refused the text"));
    }
    if sensitive {
        let empty = NSString::from_str("");
        for marker in ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"] {
            pb.setString_forType(&empty, &NSString::from_str(marker));
        }
    }
    Ok(())
}

/// Empties the pasteboard.
pub(crate) fn clear_clipboard() {
    NSPasteboard::generalPasteboard().clearContents();
}

/// Keeps a window out of screenshots and screen sharing (`NSWindowSharingNone`), or not.
pub(crate) fn set_capture_protected(
    ns_view: std::ptr::NonNull<std::ffi::c_void>,
    on: bool,
) -> Result<(), Unsupported> {
    // SAFETY: winit's AppKit handle points to the window's live content NSView, used on the
    // main thread where the event loop runs.
    let view: &NSView = unsafe { ns_view.cast::<NSView>().as_ref() };
    let window =
        view.window().ok_or_else(|| Unsupported::new("capture protection", "the view has no window"))?;
    window.setSharingType(if on { NSWindowSharingType::None } else { NSWindowSharingType::ReadOnly });
    Ok(())
}

/// System settings as macOS reports them.
pub(crate) struct Settings {
    pub(crate) dark: Option<bool>,
    pub(crate) accent: Option<Color>,
    pub(crate) high_contrast: bool,
    pub(crate) reduced_motion: bool,
    pub(crate) reduced_transparency: bool,
    pub(crate) locale: Option<String>,
    pub(crate) double_click: Duration,
}

pub(crate) fn settings() -> Settings {
    let dark = MainThreadMarker::new().and_then(|mtm| {
        let app = NSApplication::sharedApplication(mtm);
        let appearance = app.effectiveAppearance();
        // SAFETY: both names are immutable framework constants.
        let names = unsafe { NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]) };
        let best = appearance.bestMatchFromAppearancesWithNames(&names)?;
        // SAFETY: as above.
        Some(&*best == unsafe { NSAppearanceNameDarkAqua })
    });
    let accent =
        NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()).map(|c| {
            Color::new(c.redComponent() as f32, c.greenComponent() as f32, c.blueComponent() as f32, 1.0)
        });
    let ws = NSWorkspace::sharedWorkspace();
    let locale = NSLocale::currentLocale().localeIdentifier().to_string().replace('_', "-");
    Settings {
        dark,
        accent,
        high_contrast: ws.accessibilityDisplayShouldIncreaseContrast(),
        reduced_motion: ws.accessibilityDisplayShouldReduceMotion(),
        reduced_transparency: ws.accessibilityDisplayShouldReduceTransparency(),
        locale: (!locale.is_empty()).then_some(locale),
        double_click: Duration::from_secs_f64(NSEvent::doubleClickInterval().max(0.0)),
    }
}
