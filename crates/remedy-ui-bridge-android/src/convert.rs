//! Android's values in RemedyUI's terms. Platform-neutral, so it is tested on any machine.

use std::time::Duration;

use remedy_ui_core::Color;
use remedy_ui_host::{ForegroundKind, MemoryPressure, NotificationResponse, SystemSettings, TrayImage};
use remedy_ui_input::Platform;

/// `ComponentCallbacks2.onTrimMemory` levels.
pub mod trim {
    /// Running, and the system is beginning to run low.
    pub const RUNNING_MODERATE: i32 = 5;
    /// Running, and the system is low.
    pub const RUNNING_LOW: i32 = 10;
    /// Running, and the system will soon kill background processes.
    pub const RUNNING_CRITICAL: i32 = 15;
    /// The app's interface is no longer seen.
    pub const UI_HIDDEN: i32 = 20;
    /// In the background, on the list of processes to kill.
    pub const BACKGROUND: i32 = 40;
    /// In the background, around the middle of that list.
    pub const MODERATE: i32 = 60;
    /// In the background, next to be killed.
    pub const COMPLETE: i32 = 80;
}

/// What an `onTrimMemory` level asks of the engine. Levels Android may add later count as
/// the nearest level below them.
pub fn trim_pressure(level: i32) -> Option<MemoryPressure> {
    Some(match level {
        l if l >= trim::MODERATE => MemoryPressure::Critical,
        // Hidden or in the background with memory to spare: what is cheap to rebuild goes.
        l if l >= trim::UI_HIDDEN => MemoryPressure::Moderate,
        l if l >= trim::RUNNING_CRITICAL => MemoryPressure::Critical,
        l if l >= trim::RUNNING_MODERATE => MemoryPressure::Moderate,
        _ => return None,
    })
}

/// What the bridge read from Android's configuration and settings.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct AndroidSettings {
    /// `Configuration.uiMode` night bits: `Some(true)` for night, `None` when undefined.
    pub night: Option<bool>,
    /// `Configuration.fontScale`.
    pub font_scale: Option<f32>,
    /// The first of `Configuration.getLocales()`, as a BCP 47 tag.
    pub locale: Option<String>,
    /// All of `Configuration.getLocales()`, `LocaleList.toLanguageTags()`: BCP 47 tags,
    /// comma-separated, most preferred first.
    pub locales: Option<String>,
    /// `Settings.Global.ANIMATOR_DURATION_SCALE`.
    pub animator_scale: Option<f32>,
    /// High-contrast text is on.
    pub high_contrast_text: Option<bool>,
    /// The system accent (Material You's `system_accent1_500`, Android 12 and later), ARGB.
    pub accent: Option<i32>,
    /// `ViewConfiguration.getDoubleTapTimeout()`, in milliseconds.
    pub double_tap_ms: Option<i32>,
    /// `Configuration.uiMode` type bits say television (Android TV, Google TV, Fire TV).
    pub television: bool,
}

/// The caret blink interval of Android's own text fields (`Editor`'s fixed 500 ms).
pub const CARET_BLINK: Duration = Duration::from_millis(500);

impl AndroidSettings {
    /// As RemedyUI's system settings; what Android did not say keeps the default.
    pub fn to_settings(&self) -> SystemSettings {
        let base = SystemSettings::default();
        let mut s = SystemSettings::default();
        s.platform = Platform::Standard;
        s.dark = self.night;
        s.accent = self.accent.map(argb);
        s.high_contrast = self.high_contrast_text.unwrap_or(false);
        s.text_scale = self.font_scale.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(base.text_scale);
        // Animations off (scale 0) is how Android users ask for less motion; the
        // accessibility "remove animations" switch sets the same value.
        s.reduced_motion = self.animator_scale.is_some_and(|s| s == 0.0);
        // Android has no reduced-transparency setting.
        s.reduced_transparency = false;
        s.locale = self.locale.as_deref().and_then(bcp47).unwrap_or(base.locale);
        s.locales = self.locales.as_deref().unwrap_or_default().split(',').filter_map(bcp47).collect();
        if s.locales.is_empty() {
            s.locales = vec![s.locale.clone()];
        }
        s.double_click = self
            .double_tap_ms
            .and_then(|ms| u64::try_from(ms).ok())
            .filter(|ms| *ms > 0)
            .map_or(base.double_click, Duration::from_millis);
        s.caret_blink = Some(CARET_BLINK);
        // A TV is driven by its remote's d-pad.
        s.spatial_navigation = self.television;
        s
    }
}

/// An ARGB color int as a color.
pub fn argb(c: i32) -> Color {
    let [a, r, g, b] = c.to_be_bytes();
    Color::rgba8(r, g, b, a)
}

/// A field's purpose as the Java half's `TextInput.PURPOSE_*` code.
pub fn purpose_code(p: remedy_ui_host::InputPurpose) -> i32 {
    use remedy_ui_host::InputPurpose as P;
    match p {
        P::Digits => 1,
        P::Decimal => 2,
        P::Url => 3,
        P::Email => 4,
        P::Search => 5,
        P::Username => 6,
        P::CurrentPassword => 7,
        P::NewPassword => 8,
        P::OneTimeCode => 9,
        _ => 0,
    }
}

/// A field's text and its selection in UTF-8 bytes, as Java sees them: the text and the
/// selection in UTF-16 units. Offsets inside a character move to its start.
pub fn utf16_selection(text: &str, selection: std::ops::Range<usize>) -> (String, (i32, i32)) {
    let units = |byte: usize| -> i32 {
        let mut b = byte.min(text.len());
        while !text.is_char_boundary(b) {
            b -= 1;
        }
        let n: usize = text.get(..b).map_or(0, |s| s.chars().map(char::len_utf16).sum());
        i32::try_from(n).unwrap_or(i32::MAX)
    };
    (text.to_owned(), (units(selection.start), units(selection.end)))
}

/// A locale tag from `Locale.toLanguageTag()`, or `None` for "und" (no locale).
pub fn bcp47(tag: &str) -> Option<String> {
    let t = tag.trim();
    (!t.is_empty() && !t.eq_ignore_ascii_case("und")).then(|| t.to_owned())
}

/// `ServiceInfo.FOREGROUND_SERVICE_TYPE_*` for the kinds of foreground work: the bits the
/// manifest declares and `startForeground` holds. `None` for a kind this bridge does not know.
pub fn foreground_types(kinds: &[ForegroundKind]) -> Option<i32> {
    kinds.iter().try_fold(0, |bits, k| {
        Some(
            bits | match k {
                ForegroundKind::DataSync => 1,
                ForegroundKind::MediaPlayback => 2,
                ForegroundKind::Camera => 64,
                ForegroundKind::Microphone => 128,
                _ => return None,
            },
        )
    })
}

/// A notification picture as Android's bitmap pixels: non-premultiplied ARGB, and its side.
/// `None` when the picture's bytes do not match its size.
pub fn argb_pixels(image: &TrayImage) -> Option<(Vec<i32>, i32)> {
    let side = usize::try_from(image.size).ok()?;
    if side == 0 || image.rgba.len() != side.checked_mul(side)?.checked_mul(4)? {
        return None;
    }
    let pixels = image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let [r, g, b, a] = p.map(u32::from);
            // Premultiplied to straight: each channel over alpha, rounded, at most 255.
            let straight = |c: u32| (c * 255 + a / 2).checked_div(a).unwrap_or(0).min(255);
            let argb = (a << 24) | (straight(r) << 16) | (straight(g) << 8) | straight(b);
            i32::from_ne_bytes(argb.to_ne_bytes())
        })
        .collect();
    Some((pixels, i32::try_from(side).ok()?))
}

/// What the Java half's notification report means: 0 pressed, 1 a button (`action`, its id),
/// 2 dismissed.
pub fn notification_response(kind: i32, action: Option<String>) -> Option<NotificationResponse> {
    match kind {
        0 => Some(NotificationResponse::Activated),
        1 => action.map(NotificationResponse::Action),
        2 => Some(NotificationResponse::Dismissed),
        _ => None,
    }
}

/// A Java exception's text as the reason a call failed: the message, without the exception's
/// class name when the Java half threw it to explain itself.
pub fn java_reason(text: &str) -> String {
    for class in ["java.lang.IllegalStateException: ", "java.lang.IllegalArgumentException: "] {
        if let Some(rest) = text.strip_prefix(class) {
            return rest.to_owned();
        }
    }
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_kinds_are_androids_service_types() {
        assert_eq!(foreground_types(&[]), Some(0));
        assert_eq!(foreground_types(&[ForegroundKind::Microphone, ForegroundKind::MediaPlayback]), Some(130));
        assert_eq!(
            foreground_types(&[ForegroundKind::Camera, ForegroundKind::DataSync, ForegroundKind::Camera]),
            Some(65)
        );
    }

    #[test]
    fn notification_pictures_become_straight_argb() {
        let image =
            TrayImage { size: 2, rgba: vec![255, 0, 0, 255, 64, 32, 0, 128, 0, 0, 0, 0, 10, 10, 10, 10] };
        let (px, side) = argb_pixels(&image).unwrap_or_default();
        assert_eq!(side, 2);
        let u: Vec<u32> = px.iter().map(|p| u32::from_ne_bytes(p.to_ne_bytes())).collect();
        assert_eq!(u, vec![0xffff_0000, 0x8080_4000, 0, 0x0aff_ffff]);
        assert_eq!(argb_pixels(&TrayImage { size: 3, rgba: vec![0; 4] }), None, "bytes must match the size");
        assert_eq!(argb_pixels(&TrayImage { size: 0, rgba: Vec::new() }), None);
    }

    #[test]
    fn notification_reports_decode() {
        assert_eq!(notification_response(0, None), Some(NotificationResponse::Activated));
        assert_eq!(
            notification_response(1, Some("reply".into())),
            Some(NotificationResponse::Action("reply".into()))
        );
        assert_eq!(notification_response(1, None), None, "a button names itself");
        assert_eq!(notification_response(2, None), Some(NotificationResponse::Dismissed));
        assert_eq!(notification_response(9, None), None);
        assert_eq!(
            java_reason("java.lang.IllegalStateException: notifications are off"),
            "notifications are off"
        );
        assert_eq!(
            java_reason("android.app.ForegroundServiceStartNotAllowedException: no"),
            "android.app.ForegroundServiceStartNotAllowedException: no"
        );
    }

    #[test]
    fn trim_levels_ask_for_the_right_pressure() {
        assert_eq!(trim_pressure(0), None);
        assert_eq!(trim_pressure(trim::RUNNING_MODERATE), Some(MemoryPressure::Moderate));
        assert_eq!(trim_pressure(trim::RUNNING_LOW), Some(MemoryPressure::Moderate));
        assert_eq!(trim_pressure(trim::RUNNING_CRITICAL), Some(MemoryPressure::Critical));
        assert_eq!(trim_pressure(trim::UI_HIDDEN), Some(MemoryPressure::Moderate));
        assert_eq!(trim_pressure(trim::BACKGROUND), Some(MemoryPressure::Moderate));
        assert_eq!(trim_pressure(trim::MODERATE), Some(MemoryPressure::Critical));
        assert_eq!(trim_pressure(trim::COMPLETE), Some(MemoryPressure::Critical));
        assert_eq!(
            trim_pressure(90),
            Some(MemoryPressure::Critical),
            "a future level counts as the one below"
        );
    }

    #[test]
    fn settings_follow_android() {
        let s = AndroidSettings {
            night: Some(true),
            font_scale: Some(1.3),
            locale: Some("de-DE".into()),
            locales: Some("de-DE,fr-CH,und,en".into()),
            animator_scale: Some(0.0),
            high_contrast_text: Some(true),
            accent: Some(0xFF33_66CC_u32 as i32),
            double_tap_ms: Some(300),
            television: true,
        }
        .to_settings();
        assert_eq!(s.dark, Some(true));
        assert_eq!(s.text_scale, 1.3);
        assert_eq!(s.locale, "de-DE");
        assert_eq!(s.locales, ["de-DE", "fr-CH", "en"], "the locale list in order, \"und\" dropped");
        assert!(s.reduced_motion);
        assert!(s.high_contrast);
        assert_eq!(s.accent, Some(Color::rgba8(0x33, 0x66, 0xCC, 0xFF)));
        assert_eq!(s.double_click, Duration::from_millis(300));
        assert_eq!(s.caret_blink, Some(CARET_BLINK));
        assert!(s.spatial_navigation, "a TV moves focus with its d-pad");
    }

    #[test]
    fn selections_count_utf16_units() {
        // "a" 1 byte/1 unit, "é" 2/1, "日" 3/1, "😀" 4/2.
        let t = "aé日😀b";
        assert_eq!(utf16_selection(t, 0..0).1, (0, 0));
        assert_eq!(utf16_selection(t, 1..3).1, (1, 2));
        assert_eq!(utf16_selection(t, 6..10).1, (3, 5), "the emoji is two units");
        assert_eq!(utf16_selection(t, 11..11).1, (6, 6));
        assert_eq!(utf16_selection(t, 2..8).1, (1, 3), "inside a character moves to its start");
        assert_eq!(utf16_selection(t, 99..99).1, (6, 6), "past the end is the end");
    }

    #[test]
    fn what_android_does_not_say_keeps_the_default() {
        let s =
            AndroidSettings { animator_scale: Some(0.5), locale: Some("und".into()), ..Default::default() }
                .to_settings();
        let d = SystemSettings::default();
        assert_eq!(s.dark, None);
        assert!(!s.reduced_motion, "slower animations are still animations");
        assert_eq!(s.locale, d.locale);
        assert_eq!(s.locales, std::slice::from_ref(&d.locale), "no list: the one locale");
        let one = AndroidSettings { locale: Some("ja-JP".into()), ..Default::default() }.to_settings();
        assert_eq!(one.locales, ["ja-JP"]);
        assert_eq!(s.text_scale, d.text_scale);
        assert_eq!(s.double_click, d.double_click);
        assert!(!s.spatial_navigation);
    }
}
