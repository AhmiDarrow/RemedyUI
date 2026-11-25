//! Themes, contrast and resolution.

use remedy_ui_core::style::*;
use remedy_ui_core::{Color, Insets};

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
    for accent in [d.accent_hover, d.accent_pressed] {
        assert!(d.on_accent.contrast(accent) >= CONTRAST_AA, "a hovered or pressed accent stays readable");
    }
    for surface in [theme.colors.bg, theme.colors.panel, theme.colors.card] {
        assert!(d.muted.contrast(surface) >= CONTRAST_AAA, "secondary text reads easily on every surface");
        assert!(d.text.contrast(surface) >= CONTRAST_AAA);
    }
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

#[test]
fn tokens_resolve_through_whatever_theme_is_shown() {
    let mut theme = Theme::dark_forest();
    let style = Style::new()
        .radius(Radius::R3)
        .padding_x(Space::S4)
        .background(ColorRole::Accent)
        .border_width(Dp::Stroke)
        .text(TextRole::Title);
    let p = style.resolve_over(&theme, Insets { top: 7.0, end: 1.0, bottom: 7.0, start: 1.0 });
    assert_eq!(p.radius, Some(theme.radii.large));
    assert_eq!(p.background, Some(theme.colors.accent));
    assert_eq!(p.border_width, Some(theme.stroke));
    assert_eq!(p.text_size, Some(TextRole::Title.size(&theme)));
    assert_eq!(p.weight, Some(600.0));
    let pad = p.padding.unwrap_or(Insets::ZERO);
    assert_eq!((pad.start, pad.end), (theme.spacing.s(4), theme.spacing.s(4)));
    assert_eq!((pad.top, pad.bottom), (7.0, 7.0), "unset sides keep the widget's own padding");
    assert!(style.has_partial_padding());
    assert!(!style.is_plain());

    // Another theme: the same tokens, other numbers.
    theme.spacing.unit = 8.0;
    theme.radii.large = 20.0;
    theme.colors.accent = Color::rgb8(200, 30, 30);
    let p = style.resolve(&theme);
    assert_eq!(p.radius, Some(20.0));
    assert_eq!(p.padding.map(|i| i.start), Some(32.0));
    assert_eq!(p.background, Some(Color::rgb8(200, 30, 30)));

    // Plain values still work, and a later patch wins property by property.
    let plain = Style::from(StylePatch::new().radius(5.0));
    assert!(plain.is_plain());
    let merged =
        style.merge(&plain).merge(&Style::new().padding(Pad { top: Some(Dp::Px(2.0)), ..Pad::default() }));
    let p = merged.resolve(&theme);
    assert_eq!(p.radius, Some(5.0));
    assert_eq!(p.padding.map(|i| (i.top, i.start)), Some((2.0, 32.0)));
    assert_eq!(Radius::R0.px(&theme), 0.0);
    assert_eq!(Radius::R4.px(&theme), RADIUS_FULL);
    assert_eq!(Space::S0.px(&theme), 0.0);
    // The scale: half steps below four units, whole steps to eight, layout steps beyond.
    let units: Vec<f32> = Space::ALL.iter().map(|s| s.units()).collect();
    assert!(units.windows(2).all(|w| w.first() < w.get(1)), "smallest first: {units:?}");
    assert_eq!(Space::S0_5.px(&theme), theme.spacing.unit / 2.0);
    assert_eq!(Space::S3_5.px(&theme), theme.spacing.unit * 3.5);
    assert_eq!(Dp::from(Space::S12).resolve(&theme), theme.spacing.unit * 12.0);
    assert!(Space::ALL.iter().all(|s| Space::from_name(s.name()) == Some(*s)));
    assert_eq!(Space::from_name("s9"), None);
    let mut roomy = theme.clone();
    roomy.spacing.unit *= 2.0;
    assert_eq!(Space::S1_5.px(&roomy), Space::S1_5.px(&theme) * 2.0, "steps follow the theme's unit");
    assert_eq!(ColorRole::Custom("brand").color(&theme), Color::TRANSPARENT);
    theme.colors.custom.insert("brand".into(), Color::WHITE);
    assert_eq!(ColorRef::from(ColorRole::Custom("brand")).resolve(&theme), Color::WHITE);
}

#[test]
fn line_height_resolves_merges_and_is_inherited() {
    let theme = Theme::dark_forest();
    let mut sheet = StyleSheet::new();
    sheet.rule("label", None, States::NONE, StylePatch::default().line_height(LineHeight::Relative(1.5)));
    let key = StyleKey { widget: "label", variant: "", states: States::NONE };
    let defaults = StylePatch::defaults(&theme);
    // Today's default stays.
    let plain = sheet.resolve(
        &StyleKey { widget: "box", variant: "", states: States::NONE },
        &defaults,
        &StylePatch::default(),
    );
    assert_eq!(plain.line_height, LineHeight::Relative(1.33));
    // A rule sets it, a local patch wins over the rule.
    assert_eq!(sheet.resolve(&key, &defaults, &StylePatch::default()).line_height, LineHeight::Relative(1.5));
    let local = StylePatch::default().line_height(LineHeight::Absolute(20.0));
    let own = sheet.resolve(&key, &defaults, &local);
    assert_eq!(own.line_height, LineHeight::Absolute(20.0));
    // Children inherit it, as CSS does.
    assert_eq!(own.inherited_by_children(&defaults).line_height, Some(LineHeight::Absolute(20.0)));
    // The token style carries it into a patch, and merging keeps the later one.
    let tok = Style::default().line_height(LineHeight::FontMetrics);
    assert_eq!(tok.resolve(&theme).line_height, Some(LineHeight::FontMetrics));
    let merged = tok.merge(&Style::default().line_height(LineHeight::Relative(1.2)));
    assert_eq!(merged.line_height, Some(LineHeight::Relative(1.2)));
    assert_eq!(StylePatch::default().merge(&local).line_height, Some(LineHeight::Absolute(20.0)));
}
