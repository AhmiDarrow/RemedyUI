//! Portal's own theme set, token for token as Portal's page has it (`web/src/themes.ts` and the
//! `:root` of `web/src/styles.css`): its default gold "Portal" palette, and the menu's others,
//! whose colourblind ones (Azure, Skyline, Lagoon) differ from the shared palettes'. Portal's
//! page sets each in Remedy Sans at 16 px.

use remedy_ui_core::Color;
use remedy_ui_style::Theme;

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
/// is: Remedy Sans at 16 px (clock and small text 0.88 rem), 36 px controls, Portal's radii
/// (8 px small, 12 px for the stage, 18 px for the dock), and the seek bar's tokens: `track`
/// under it, `player-loaded` for what has loaded (the text mixed 28% into the track) and
/// `player-thumb` (the accent). `None` for an id Portal does not have.
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
    t.font_families = vec!["Remedy Sans".into(), "Remedy Sans UI".into()];
    t.mono_families = t.font_families.clone();
    t.type_scale.small = 14.08;
    t.type_scale.body = 16.0;
    t.spacing.control_height = 36.0;
    t.radii.small = 8.0;
    t.radii.medium = 12.0;
    t.radii.large = 18.0;
    Some(t)
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
