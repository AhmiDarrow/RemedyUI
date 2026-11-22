//! The theme set: real palettes, system following, high contrast, and readable widgets in
//! every theme.

use remedy_ui_core::Color;
use remedy_ui_style::{ResolvedStyle, States, StyleKey, StylePatch, Theme};
use remedy_ui_themes::{PALETTES, ThemeChoice, all, high_contrast, palette, sheet, theme};

fn resolve(t: &Theme, widget: &'static str, variant: &'static str, states: States) -> ResolvedStyle {
    let mut s = sheet(t);
    let base = StylePatch::defaults(t);
    s.resolve(&StyleKey { widget, variant, states }, &base, &StylePatch::default())
}

#[test]
fn every_palette_parses_and_forest_is_the_default_dark_forest() {
    assert_eq!(PALETTES.len(), 12);
    assert_eq!(PALETTES.first().map(|p| p.id), Some("forest"));
    for p in PALETTES {
        for hex in [
            p.bg,
            p.panel,
            p.card,
            p.line,
            p.accent,
            p.text,
            p.muted,
            p.faint,
            p.accent_hover,
            p.ok,
            p.danger,
            p.warn,
        ] {
            assert!(Color::from_hex(hex).is_ok(), "{} {hex}", p.id);
        }
        assert_eq!(theme(p).derive().dark, p.dark, "{} is as dark as it says", p.id);
    }
    let forest = palette("forest").map(theme);
    assert_eq!(
        forest.map(|t| t.colors.clone()).map(|mut c| {
            c.custom.clear();
            c
        }),
        Some(Theme::dark_forest().colors),
        "the same Dark Forest the engine ships"
    );
}


#[test]
fn colorblind_palette_ids_keep_their_short_display_names() {
    for (id, name) in [("deutan", "Deutan"), ("protan", "Protan"), ("tritan", "Tritan")] {
        let p = palette(id).unwrap_or_else(|| unreachable!("palette {id}"));
        assert_eq!(p.id, id, "ids stay stable so saved prefs still match");
        assert_eq!(p.name, name);
    }
}

#[test]
fn system_follows_the_platform_and_unknown_ids_fall_back() {
    assert_eq!(ThemeChoice::System.resolve(Some(false), false).name, "Light");
    assert_eq!(ThemeChoice::System.resolve(Some(true), false).name, "Dark");
    assert_eq!(ThemeChoice::System.resolve(None, false).name, "Dark");
    assert_eq!(ThemeChoice::Palette("nope".into()).resolve(None, false).name, "Dark Forest");
    assert_eq!(ThemeChoice::default().resolve(None, false).name, "Dark Forest");
    let hc = ThemeChoice::Palette("light".into()).resolve(None, true);
    assert!(hc.name.contains("high contrast"));
    assert_eq!(hc.colors.line, Some(hc.colors.text), "borders as strong as text");
}

#[test]
fn widgets_read_well_in_every_theme_and_in_high_contrast() {
    let mut themes = all();
    themes.extend(all().iter().map(high_contrast));
    for t in &themes {
        let bg = t.colors.bg;
        let check = |what: &str, fg: Color, on: Color, min: f32| {
            let ratio = fg.contrast(on);
            assert!(ratio >= min, "{}: {what} {ratio:.2} < {min}", t.name);
        };
        let label = resolve(t, "label", "", States::NONE);
        check("text", label.foreground, bg, 7.0);
        check("muted text", resolve(t, "label", "muted", States::NONE).foreground, bg, 7.0);
        check("error text", resolve(t, "label", "error", States::NONE).foreground, bg, 4.5);
        let primary = resolve(t, "button", "primary", States::NONE);
        check("primary button", primary.foreground, primary.background, 4.5);
        let button = resolve(t, "button", "", States::NONE);
        check("button", button.foreground, button.background, 4.5);
        let hover = resolve(t, "button", "", States::HOVER);
        check("hovered button", hover.foreground, hover.background, 4.5);
        let input = resolve(t, "text_input", "", States::NONE);
        check("input", input.foreground, input.background, 4.5);
        let seg = resolve(t, "segment", "", States::SELECTED);
        check("selected segment", seg.foreground, seg.background, 4.5);
        let danger = resolve(t, "button", "danger_solid", States::NONE);
        check("danger button", danger.foreground, danger.background, 4.5);
        // A focused control is visibly marked: the focus color stands out from its surroundings.
        check("focus ring", t.derive().focus, bg, 3.0);
        assert!(
            resolve(t, "button", "", States::DISABLED).opacity < 1.0,
            "{}: disabled looks disabled",
            t.name
        );
        assert!(resolve(t, "label", "code", States::NONE).mono, "{}: codes are monospace", t.name);
    }
}
