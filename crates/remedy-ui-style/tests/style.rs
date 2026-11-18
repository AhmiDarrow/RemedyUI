//! Themes, contrast and resolution.

use remedy_ui_core::{Color, Insets};
use remedy_ui_style::*;

#[test]
fn ensure_contrast_moves_only_as_far_as_needed() {
    let bg = Color::from_hex("#141413").unwrap_or(Color::BLACK);
    let dim = Color::from_hex("#4A4A44").unwrap_or(Color::BLACK);
    let fixed = ensure_contrast(dim, bg, CONTRAST_AA);
    let ratio = fixed.contrast(bg);
    assert!(ratio >= CONTRAST_AA, "{ratio}");
    assert!(ratio < CONTRAST_AA + 0.2, "not pushed further than needed: {ratio}");
    // Already fine: unchanged.
    assert_eq!(ensure_contrast(Color::WHITE, bg, CONTRAST_AA), Color::WHITE);
    // Light backgrounds darken.
    let on_light = ensure_contrast(Color::rgb8(200, 200, 200), Color::WHITE, CONTRAST_AA);
    assert!(on_light.luminance() < Color::rgb8(200, 200, 200).luminance());
    assert!(on_light.contrast(Color::WHITE) >= CONTRAST_AA);
}

#[test]
fn ensure_contrast_keeps_the_hue() {
    let bg = Color::rgb8(20, 20, 20);
    let green = Color::rgb8(40, 90, 50);
    let fixed = ensure_contrast(green, bg, CONTRAST_AAA);
    let [_, a0, b0, _] = green.to_oklab();
    let [_, a1, b1, _] = fixed.to_oklab();
    let hue = |a: f32, b: f32| b.atan2(a);
    assert!((hue(a0, b0) - hue(a1, b1)).abs() < 0.15);
}

#[test]
fn derived_tokens_follow_the_skin_rules() {
    let mut theme = Theme::dark_forest();
    let d = theme.derive();
    assert!(d.dark);
    assert_eq!(d.sizes, [11.0, 12.0, 13.0, 15.0, 18.0, 24.0], "six sizes from three");
    assert!(d.text.contrast(theme.colors.bg) >= CONTRAST_AAA);
    assert!(d.on_accent.contrast(theme.colors.accent) >= CONTRAST_AA);
    assert_ne!(d.hover, theme.colors.card);
    assert_eq!(d.control_height, 28.0);
    theme.density = Density::Compact;
    assert_eq!(theme.derive().control_height, 24.0);
    assert_eq!(theme.spacing.snap(10.0), 12.0, "spacing rounds to the 4 dp grid like skins");
    assert_eq!(theme.spacing.s(4), 16.0);
}

#[test]
fn rules_apply_by_widget_variant_and_state() {
    let mut sheet = StyleSheet::new();
    let accent = Color::rgb8(90, 140, 126);
    let hover = Color::rgb8(100, 150, 136);
    sheet.rule("button", None, States::NONE, StylePatch::default().radius(3.0));
    sheet.rule("button", Some("primary"), States::NONE, StylePatch::default().background(accent));
    sheet.rule("button", Some("primary"), States::HOVER, StylePatch::default().background(hover));
    sheet.rule("button", None, States::DISABLED, StylePatch::default().opacity(0.4));

    let defaults = StylePatch::default().padding(Insets::symmetric(6.0, 12.0));
    let none = StylePatch::default();
    let key = |states| StyleKey { widget: "button", variant: "primary", states };

    let plain = sheet.resolve(&key(States::NONE), &defaults, &none);
    assert_eq!((plain.background, plain.radius, plain.padding), (accent, 3.0, Insets::symmetric(6.0, 12.0)));
    let hovered = sheet.resolve(&key(States::HOVER | States::FOCUS), &defaults, &none);
    assert_eq!(hovered.background, hover, "a state rule beats the plain one");
    let disabled = sheet.resolve(&key(States::DISABLED), &defaults, &none);
    assert_eq!((disabled.opacity, disabled.background), (0.4, accent));
    // The element's own properties come last.
    let local = StylePatch::default().radius(10.0);
    assert_eq!(sheet.resolve(&key(States::NONE), &defaults, &local).radius, 10.0);
    // A quiet button gets none of the primary rules.
    let quiet = sheet.resolve(
        &StyleKey { widget: "button", variant: "quiet", states: States::HOVER },
        &defaults,
        &none,
    );
    assert_eq!(quiet.background, Color::TRANSPARENT);
}

#[test]
fn hovering_is_a_cache_hit() {
    let mut sheet = StyleSheet::new();
    sheet.rule("button", None, States::HOVER, StylePatch::default().background(Color::WHITE));
    let key = StyleKey { widget: "button", variant: "", states: States::HOVER };
    let none = StylePatch::default();
    for _ in 0..1000 {
        sheet.resolve(&key, &none, &none);
    }
    assert_eq!(sheet.cache_stats(), (999, 1));
    let v = sheet.version();
    sheet.invalidate();
    assert!(sheet.version() > v);
    sheet.resolve(&key, &none, &none);
    assert_eq!(sheet.cache_stats().1, 2, "a theme change re-resolves");
}

#[test]
fn states_combine_and_print() {
    let s = States::HOVER | States::FOCUS_VISIBLE;
    assert!(s.contains(States::HOVER) && !s.contains(States::PRESSED));
    assert_eq!(format!("{:?}", s.with(States::HOVER, false)), "States(FOCUS_VISIBLE)");
}
