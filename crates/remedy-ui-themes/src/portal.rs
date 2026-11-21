//! Portal's own theme set, token for token as Portal's page has it (`web/src/themes.ts` and the
//! `:root` of `web/src/styles.css`): its default gold "Portal" palette, and the menu's others,
//! whose colourblind ones (Azure, Skyline, Lagoon) differ from the shared palettes'. Portal's
//! page sets each in Remedy Sans at 16 px.

use remedy_ui_core::{Color, Insets};
use remedy_ui_style::{States, StylePatch, StyleSheet, Theme};

/// One Portal palette: `(id, name, [bg, raised, well, line, accent, text, muted, error, track])`,
/// as `themes.ts` writes them (`--track` is the line, except in Portal's own palette).
pub type PortalPalette = (&'static str, &'static str, [&'static str; 9]);

/// Portal's palettes, its default first, in `themes.ts` order.
pub const PORTAL_PALETTES: &[PortalPalette] = &[
    (
        "portal",
        "Portal",
        ["#14120e", "#211e18", "#1a1713", "#3a3428", "#e2b15a", "#f4efe4", "#b3a894", "#f08a74", "#2c281f"],
    ),
    (
        "dark",
        "Dark",
        ["#0c0c14", "#14141f", "#1c1c2a", "#2a2a3d", "#8b5cf6", "#f0f0f5", "#a8a8c0", "#f87171", "#2a2a3d"],
    ),
    (
        "neutral",
        "Neutral Dark",
        ["#0f1115", "#161a20", "#1e2430", "#2e3644", "#94a3b8", "#e2e8f0", "#94a3b8", "#f87171", "#2e3644"],
    ),
    (
        "light",
        "Light",
        ["#f4f4f8", "#ffffff", "#e8e8ef", "#a8a8b8", "#7c3aed", "#14141f", "#3d3d52", "#b91c1c", "#a8a8b8"],
    ),
    (
        "forest",
        "Dark Forest",
        ["#0a0e0b", "#121812", "#1a221c", "#2a352c", "#4d7a5a", "#e6ebe7", "#9aa89e", "#b87a7a", "#2a352c"],
    ),
    (
        "green",
        "Emerald",
        ["#0a1410", "#0f1f18", "#163026", "#1f4a38", "#34d399", "#d8ebe2", "#8fb8a4", "#fca5a5", "#1f4a38"],
    ),
    (
        "purple",
        "Amethyst",
        ["#100a16", "#1a1224", "#261a36", "#3d2a55", "#c084fc", "#e4dff0", "#b0a0c8", "#fca5a5", "#3d2a55"],
    ),
    (
        "alien",
        "Dark Purple",
        ["#07040f", "#0f081a", "#1a0f2e", "#3b1d66", "#b026ff", "#f2e8ff", "#c4a0e8", "#ff6b9d", "#3b1d66"],
    ),
    (
        "orange",
        "Amber",
        ["#14100a", "#1f1810", "#2e2418", "#4a3a24", "#fb923c", "#efe6da", "#c4a88a", "#fca5a5", "#4a3a24"],
    ),
    (
        "cyan",
        "Ocean",
        ["#0a1216", "#0f1c22", "#163038", "#1e4a55", "#22d3ee", "#d8eef2", "#8fb8c2", "#fca5a5", "#1e4a55"],
    ),
    (
        "deutan",
        "Azure",
        ["#0b0f14", "#121821", "#1a2330", "#34465a", "#56b4e9", "#e9f0f6", "#a9b9c7", "#f5a13c", "#34465a"],
    ),
    (
        "protan",
        "Skyline",
        ["#0d1013", "#151a1f", "#1e252c", "#384551", "#6cb6ff", "#eef1f4", "#adb8c2", "#ffc233", "#384551"],
    ),
    (
        "tritan",
        "Lagoon",
        ["#12100f", "#1b1817", "#262120", "#453b39", "#2fd0c0", "#f3efee", "#c0b6b3", "#ff6f7d", "#453b39"],
    ),
];

/// The theme for Portal's palette `id` (`portal`, `light`, `deutan`, …), set as Portal's page
/// is: Remedy Sans at 16 px (small text 0.88 rem), headings in Ahmi Pixel (Portal's default
/// heading face), 36 px controls, Portal's radii (8 px small, 12 px for the stage, 18 px for
/// the dock), and the player's tokens with Portal's values: the colors `track`,
/// `player-loaded` (the text mixed 28% into the track), `player-thumb`, `player-thumb-ring`,
/// `player-track-full` and `player-loaded-full` (full screen), `player-big-play-hover-text`,
/// and the sliders' `slider-track`, `slider-fill`, `slider-thumb`; and the sizes in
/// [`Theme::metrics`] the player and slider document. Use it with [`portal_sheet`].
/// `None` for an id Portal does not have.
pub fn portal_theme(id: &str) -> Option<Theme> {
    let (_, name, v) = PORTAL_PALETTES.iter().find(|p| p.0 == id)?;
    let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::BLACK);
    let mut t = Theme::dark_forest();
    t.name = (*name).to_owned();
    let c = &mut t.colors;
    c.bg = hex(v[0]);
    c.panel = hex(v[1]);
    c.card = hex(v[2]);
    c.line = Some(hex(v[3]));
    c.accent = hex(v[4]);
    c.text = hex(v[5]);
    c.on_accent = hex(v[0]);
    c.muted = hex(v[6]);
    c.danger = hex(v[7]);
    let track = hex(v[8]);
    c.custom.insert("track".into(), track);
    // CSS color-mix(in srgb, text 28%, track).
    let (text, k) = (hex(v[5]), 0.28);
    let loaded = Color::new(
        track.r + (text.r - track.r) * k,
        track.g + (text.g - track.g) * k,
        track.b + (text.b - track.b) * k,
        1.0,
    );
    c.custom.insert("player-loaded".into(), loaded);
    c.custom.insert("player-thumb".into(), hex(v[4]));
    // `.seek-bar::-webkit-slider-thumb`: a 2 px ring of the background at 65%.
    c.custom.insert("player-thumb-ring".into(), hex(v[0]).with_alpha(0.65));
    // `.player-stage.is-full .seek-bar`.
    c.custom.insert("player-track-full".into(), Color::WHITE.with_alpha(0.22));
    c.custom.insert("player-loaded-full".into(), Color::WHITE.with_alpha(0.42));
    // `.stage-play:hover`: the glyph in the background's color.
    c.custom.insert("player-big-play-hover-text".into(), hex(v[0]));
    // `.volume-bar`: the text's fill and thumb on the track.
    c.custom.insert("slider-track".into(), track);
    c.custom.insert("slider-fill".into(), text);
    c.custom.insert("slider-thumb".into(), text);
    t.font_families = vec!["Remedy Sans".into(), "Remedy Sans UI".into()];
    t.mono_families = t.font_families.clone();
    t.heading_families = vec!["Ahmi Pixel".into(), "Remedy Sans".into()];
    // Portal's sizes (rem at 16 px), from styles.css.
    for (name, px) in [
        ("player-glyph", 20.8),           // .icon-btn svg: 1.3rem
        ("player-glyph-full", 24.8),      // .is-full .icon-btn svg: 1.55rem
        ("player-gap", 1.6),              // .chrome-row gap: 0.1rem
        ("player-rule", 1.0),             // .chrome border-top
        ("player-seek-height", 21.6),     // .seek-bar: 1.35rem
        ("player-seek-track", 4.0),       // the track
        ("player-seek-track-hover", 6.0), // :hover track
        ("player-thumb", 13.0 * 0.85),    // --scrub-thumb at scale(0.85)
        ("player-thumb-hover", 13.0),     // :hover scale(1)
        ("player-thumb-ring", 2.0),       // the thumb's box-shadow ring
        ("player-big-play", 68.0),        // .stage-play: 4.25rem
        ("player-big-play-glyph", 32.0),  // .stage-play svg: 2rem
        ("player-big-play-nudge", 3.2),   // margin-left: 0.2rem
        ("player-volume-width", 84.0),    // .volume:hover .volume-bar: 5.25rem
        ("player-volume-height", 24.0),   // .volume-bar: 1.5rem
        ("slider-thumb", 12.0),           // .volume-bar thumb
        ("slider-track", 4.0),            // .volume-bar track
        ("player-full-pad-top", 48.0),    // .is-full > .chrome: 3rem 1.4rem 0.9rem
        ("player-full-pad-bottom", 14.4),
        ("player-full-pad-side", 22.4),
    ] {
        t.metrics.insert(name.into(), px);
    }
    t.type_scale.small = 14.08;
    t.type_scale.body = 16.0;
    t.spacing.control_height = 36.0;
    t.radii.small = 8.0;
    t.radii.medium = 12.0;
    t.radii.large = 18.0;
    Some(t)
}

/// The style sheet for a Portal theme: [`sheet`](crate::sheet) with Portal's player on top
/// (`.chrome`, `.icon-btn`, `.clock`, `.stage-top` in Portal's styles.css): the bar on the
/// raised surface under a hairline, bare round glyph buttons a control high with the text
/// tinted 13% under the pointer and the accent while on, the time muted at 0.88 rem with the
/// elapsed part in the text color, and in full screen (the `full` variant) larger buttons, a
/// 0.95 rem time and no surface under the bar; the full-screen title in the heading face at
/// 1.3 rem.
pub fn portal_sheet(t: &Theme) -> StyleSheet {
    let mut s = crate::sheet(t);
    let c = &t.colors;
    let u = t.spacing.unit;
    let line = c.line.unwrap_or(c.muted);
    let ch = t.spacing.control_height;
    let glyph = t.metric("player-glyph", ch);
    // .is-full .icon-btn: 2.7rem.
    let full = ch * 1.2;
    let full_glyph = t.metric("player-glyph-full", glyph);
    let clear = Color::TRANSPARENT;
    let bare = |size: f32, glyph: f32| {
        StylePatch::default()
            .background(clear)
            .border(clear)
            .border_width(0.0)
            .foreground(c.text)
            .radius(size / 2.0)
            .padding(Insets::all(((size - glyph) / 2.0).max(0.0)))
    };
    let mut rule = |w: &'static str, v: Option<&'static str>, st: States, p: StylePatch| s.rule(w, v, st, p);
    rule("player_row", None, States::NONE, StylePatch::default().background(clear).padding(Insets::ZERO));
    let pad = Insets { top: u * 0.6, bottom: u * 1.6, start: u * 2.0, end: u * 2.0 };
    rule("player_bar", None, States::NONE, StylePatch::default().background(c.panel).padding(pad));
    rule(
        "player_bar",
        Some("full"),
        States::NONE,
        StylePatch::default().background(clear).padding(Insets::ZERO),
    );
    rule("player_rule", None, States::NONE, StylePatch::default().background(line));
    rule("player_rule", Some("full"), States::NONE, StylePatch::default().background(clear));
    rule("player_button", None, States::NONE, bare(ch, glyph));
    rule("player_button", Some("full"), States::NONE, bare(full, full_glyph));
    for v in [None, Some("full")] {
        rule("player_button", v, States::HOVER, StylePatch::default().background(c.text.with_alpha(0.13)));
        rule("player_button", v, States::PRESSED, StylePatch::default().background(c.text.with_alpha(0.2)));
        rule(
            "player_button",
            v,
            States::CHECKED,
            StylePatch::default().background(clear).border(clear).foreground(c.accent),
        );
    }
    let clock = |size: f32, color: Color| {
        StylePatch::default().foreground(color).text_size(size).padding(Insets {
            top: 0.0,
            bottom: 0.0,
            start: u * 1.6,
            end: u * 1.6,
        })
    };
    rule("player_clock", None, States::NONE, clock(t.type_scale.small, c.muted));
    // .is-full .clock: 0.95rem, the text at 72%.
    rule("player_clock", Some("full"), States::NONE, clock(15.2, c.text.with_alpha(0.72)));
    rule("player_clock_now", None, States::NONE, StylePatch::default().foreground(c.text));
    rule("player_seek", None, States::DISABLED, StylePatch::default().opacity(0.45));
    // .scrub-tip: the accent on the well, 0.82rem, lifted.
    rule(
        "player_tip",
        None,
        States::NONE,
        StylePatch::default()
            .foreground(c.accent)
            .text_size(13.12)
            .padding(Insets { top: 1.92, bottom: 1.92, start: 7.2, end: 7.2 })
            .shadow(remedy_ui_style::Shadow::new(0.0, 8.0, 18.0, Color::BLACK.with_alpha(0.4))),
    );
    // .is-full .stage-top: the heading face at 1.3rem.
    rule(
        "player_title_text",
        None,
        States::NONE,
        StylePatch::default().heading(true).text_size(20.8).foreground(c.text),
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values as Portal's `themes.ts` and `styles.css` have them, read back off the theme.
    #[test]
    fn portal_themes_match_portals_tokens() {
        let hex = |s: &str| Color::from_hex(s).unwrap_or(Color::BLACK);
        let t = portal_theme("portal").unwrap_or_else(|| unreachable!());
        assert_eq!(t.colors.bg, hex("#14120e"));
        assert_eq!(t.colors.panel, hex("#211e18"));
        assert_eq!(t.colors.accent, hex("#e2b15a"));
        assert_eq!(t.colors.custom.get("track"), Some(&hex("#2c281f")));
        let azure = portal_theme("deutan").unwrap_or_else(|| unreachable!());
        assert_eq!(azure.name, "Azure");
        assert_eq!(azure.colors.accent, hex("#56b4e9"));
        assert_eq!(azure.colors.danger, hex("#f5a13c"));
        assert_eq!(azure.colors.line, Some(hex("#34465a")));
        assert_eq!(portal_theme("protan").map(|t| t.colors.accent), Some(hex("#6cb6ff")));
        assert_eq!(portal_theme("tritan").map(|t| t.colors.accent), Some(hex("#2fd0c0")));
        let light = portal_theme("light").unwrap_or_else(|| unreachable!());
        assert_eq!((light.colors.panel, light.colors.muted), (hex("#ffffff"), hex("#3d3d52")));
        assert_eq!(PORTAL_PALETTES.len(), 13);
        assert!(portal_theme("hearth").is_none());
    }
}
