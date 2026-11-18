//! The style sheet for RemedyUI's widgets, built only from theme tokens.

use remedy_ui_core::{Color, Insets};
use remedy_ui_style::{States, StylePatch, StyleSheet, Theme, ensure_contrast};

/// Text on a filled color: the theme's text or background, whichever reads better.
fn on(fill: Color, t: &Theme) -> Color {
    ensure_contrast(t.colors.text, fill, 4.5)
}

fn pad(v: f32, h: f32) -> Insets {
    Insets::symmetric(v, h)
}

/// The widget style sheet for `t`. Widgets are named for what they are (`button`,
/// `text_input`, `dialog`, …) and variants for their role (`primary`, `quiet`, `danger`, …).
pub fn sheet(t: &Theme) -> StyleSheet {
    let d = t.derive();
    let c = &t.colors;
    let u = t.spacing.unit;
    let r = &t.radii;
    let line = d.line;
    let danger = ensure_contrast(c.danger, c.bg, 4.5);
    let disabled = StylePatch { opacity: Some(0.45), ..StylePatch::default() };
    let mut s = StyleSheet::new();
    let mut rule =
        |w: &'static str, v: Option<&'static str>, st: States, patch: StylePatch| s.rule(w, v, st, patch);

    // Surfaces.
    rule(
        "window",
        None,
        States::NONE,
        StylePatch { background: Some(c.bg), foreground: Some(d.text), ..Default::default() },
    );
    rule("panel", None, States::NONE, StylePatch { background: Some(c.panel), ..Default::default() });
    rule(
        "card",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.card),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.large),
            padding: Some(Insets::all(u * 4.0)),
            ..Default::default()
        },
    );
    // Boards: areas are cards; while arranging, areas and widgets show hover and selection.
    rule(
        "board_area",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.card),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.large),
            ..Default::default()
        },
    );
    rule(
        "board_area",
        None,
        States::SELECTED,
        StylePatch {
            border: Some(c.accent),
            border_width: Some(t.stroke.max(1.0) * 2.0),
            ..Default::default()
        },
    );
    rule(
        "board_area",
        None,
        States::DROP_TARGET,
        StylePatch { border: Some(c.accent), ..Default::default() },
    );
    rule("board_item", None, States::NONE, StylePatch { radius: Some(r.medium), ..Default::default() });
    rule("board_item", None, States::HOVER, StylePatch { background: Some(d.hover), ..Default::default() });
    rule(
        "board_item",
        None,
        States::SELECTED,
        StylePatch { background: Some(d.selected), ..Default::default() },
    );
    rule("board_item", None, States::DRAGGING, StylePatch { opacity: Some(0.6), ..Default::default() });
    // Code: monospace text, quiet line numbers, added and removed lines tinted.
    rule("code_text", None, States::NONE, StylePatch { mono: Some(true), ..Default::default() });
    rule(
        "code_mark",
        None,
        States::NONE,
        StylePatch { mono: Some(true), foreground: Some(d.muted), ..Default::default() },
    );
    rule(
        "code_gutter",
        None,
        States::NONE,
        StylePatch { mono: Some(true), foreground: Some(d.muted), ..Default::default() },
    );
    rule("code_view", None, States::NONE, StylePatch { background: Some(c.card), ..Default::default() });
    rule(
        "code_line",
        Some("added"),
        States::NONE,
        StylePatch { background: Some(c.card.mix_oklab(c.ok, 0.18)), ..Default::default() },
    );
    rule(
        "code_line",
        Some("removed"),
        States::NONE,
        StylePatch { background: Some(c.card.mix_oklab(c.danger, 0.18)), ..Default::default() },
    );
    // Video: bars where a picture's shape differs from its place are black, as on every screen.
    rule(
        "video_surface",
        None,
        States::NONE,
        StylePatch { background: Some(Color::BLACK), ..Default::default() },
    );
    // Documents: quotes get a bar, code a box, tables lines between cells.
    rule("md_quote_bar", None, States::NONE, StylePatch { background: Some(line), ..Default::default() });
    rule("md_quote_body", None, States::NONE, StylePatch { foreground: Some(d.muted), ..Default::default() });
    rule("md_marker", None, States::NONE, StylePatch { foreground: Some(d.muted), ..Default::default() });
    rule(
        "md_code_scroll",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.card),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.medium),
            ..Default::default()
        },
    );
    rule(
        "md_code",
        None,
        States::NONE,
        StylePatch { mono: Some(true), padding: Some(pad(u * 2.0, u * 3.0)), ..Default::default() },
    );
    rule(
        "md_table",
        None,
        States::NONE,
        StylePatch {
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.small),
            ..Default::default()
        },
    );
    rule(
        "md_cell",
        None,
        States::NONE,
        StylePatch {
            border: Some(line),
            border_width: Some(t.stroke * 0.5),
            padding: Some(pad(u * 1.5, u * 2.5)),
            ..Default::default()
        },
    );
    rule(
        "md_cell",
        Some("head"),
        States::NONE,
        StylePatch { weight: Some(700.0), background: Some(c.card), ..Default::default() },
    );
    // A card that stands out (the user's own message, a highlighted item).
    rule(
        "card",
        Some("accent"),
        States::NONE,
        StylePatch { background: Some(d.selected), border: Some(c.accent), ..Default::default() },
    );
    rule("divider", None, States::NONE, StylePatch { background: Some(line), ..Default::default() });
    rule(
        "scrim",
        None,
        States::NONE,
        StylePatch { background: Some(Color::BLACK.with_alpha(0.5)), ..Default::default() },
    );
    rule(
        "dialog",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.panel),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.large),
            padding: Some(Insets::all(u * 5.0)),
            ..Default::default()
        },
    );

    // Text.
    // Plain labels and headings inherit their text color (white on a primary button, the
    // window's text color elsewhere); variants that mean something set their own.
    rule(
        "label",
        Some("muted"),
        States::NONE,
        StylePatch { foreground: Some(d.muted), ..Default::default() },
    );
    rule(
        "label",
        Some("small"),
        States::NONE,
        StylePatch { foreground: Some(d.muted), text_size: Some(t.type_scale.small), ..Default::default() },
    );
    rule("label", Some("error"), States::NONE, StylePatch { foreground: Some(danger), ..Default::default() });
    rule("label", Some("strong"), States::NONE, StylePatch { weight: Some(700.0), ..Default::default() });
    rule(
        "label",
        Some("code"),
        States::NONE,
        StylePatch {
            mono: Some(true),
            background: Some(c.card),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.small),
            padding: Some(pad(u * 2.0, u * 3.0)),
            ..Default::default()
        },
    );
    rule(
        "heading",
        Some("1"),
        States::NONE,
        StylePatch { text_size: Some(t.type_scale.title + 6.0), weight: Some(700.0), ..Default::default() },
    );
    rule(
        "heading",
        Some("2"),
        States::NONE,
        StylePatch { text_size: Some(t.type_scale.title), weight: Some(650.0), ..Default::default() },
    );
    rule(
        "heading",
        Some("3"),
        States::NONE,
        StylePatch { text_size: Some(t.type_scale.body + 2.0), weight: Some(600.0), ..Default::default() },
    );

    // Buttons.
    let base = StylePatch {
        background: Some(c.card),
        foreground: Some(d.text),
        border: Some(line),
        border_width: Some(t.stroke),
        radius: Some(r.medium),
        padding: Some(pad(u * 1.5, u * 3.0)),
        weight: Some(600.0),
        ..Default::default()
    };
    for w in ["button", "icon_button"] {
        rule(w, None, States::NONE, base.clone());
        rule(w, None, States::HOVER, StylePatch { background: Some(d.hover), ..Default::default() });
        rule(w, None, States::PRESSED, StylePatch { background: Some(d.pressed), ..Default::default() });
        // A button that stays on (bold in a toolbar, a toggle) shows it.
        rule(
            w,
            None,
            States::CHECKED,
            StylePatch { background: Some(d.selected), border: Some(c.accent), ..Default::default() },
        );
        rule(w, None, States::DISABLED, disabled.clone());
        rule(
            w,
            Some("primary"),
            States::NONE,
            StylePatch {
                background: Some(c.accent),
                foreground: Some(d.on_accent),
                border: Some(c.accent),
                ..Default::default()
            },
        );
        rule(
            w,
            Some("primary"),
            States::HOVER,
            StylePatch { background: Some(d.accent_hover), ..Default::default() },
        );
        rule(
            w,
            Some("primary"),
            States::PRESSED,
            StylePatch { background: Some(d.accent_pressed), ..Default::default() },
        );
        rule(
            w,
            Some("quiet"),
            States::NONE,
            StylePatch {
                background: Some(Color::TRANSPARENT),
                border: Some(Color::TRANSPARENT),
                ..Default::default()
            },
        );
        rule(w, Some("quiet"), States::HOVER, StylePatch { background: Some(d.hover), ..Default::default() });
        rule(
            w,
            Some("danger"),
            States::NONE,
            StylePatch {
                foreground: Some(danger),
                border: Some(danger.with_alpha(0.6)),
                ..Default::default()
            },
        );
        rule(
            w,
            Some("danger"),
            States::HOVER,
            StylePatch { background: Some(c.card.mix_oklab(c.danger, 0.18)), ..Default::default() },
        );
        let solid = c.danger;
        rule(
            w,
            Some("danger_solid"),
            States::NONE,
            StylePatch {
                background: Some(solid),
                foreground: Some(on(solid, t)),
                border: Some(solid),
                ..Default::default()
            },
        );
        rule(
            w,
            Some("danger_solid"),
            States::HOVER,
            StylePatch { background: Some(solid.mix_oklab(Color::BLACK, 0.1)), ..Default::default() },
        );
    }
    rule(
        "icon_button",
        None,
        States::NONE,
        StylePatch { padding: Some(Insets::all(u * 1.5)), radius: Some(r.small), ..Default::default() },
    );

    // Inputs.
    for w in ["text_input", "secret_input", "text_area", "search"] {
        rule(
            w,
            None,
            States::NONE,
            StylePatch {
                background: Some(c.panel),
                foreground: Some(d.text),
                border: Some(line),
                border_width: Some(t.stroke),
                radius: Some(r.small),
                padding: Some(pad(u * 1.5, u * 2.0)),
                ..Default::default()
            },
        );
        rule(
            w,
            None,
            States::HOVER,
            StylePatch { border: Some(line.mix_oklab(d.text, 0.25)), ..Default::default() },
        );
        rule(
            w,
            None,
            States::FOCUS,
            StylePatch { border: Some(d.focus), outline_width: Some(0.0), ..Default::default() },
        );
        rule(w, None, States::INVALID, StylePatch { border: Some(danger), ..Default::default() });
        rule(w, None, States::DISABLED, disabled.clone());
    }
    // An input inside another control that already draws the input's frame (search).
    rule(
        "text_input",
        Some("bare"),
        States::NONE,
        StylePatch {
            background: Some(Color::TRANSPARENT),
            border: Some(Color::TRANSPARENT),
            padding: Some(Insets::ZERO),
            ..Default::default()
        },
    );
    rule(
        "text_input",
        Some("bare"),
        States::FOCUS,
        StylePatch { border: Some(Color::TRANSPARENT), outline_width: Some(2.0), ..Default::default() },
    );
    rule(
        "field",
        None,
        States::NONE,
        StylePatch {
            foreground: Some(d.muted),
            text_size: Some(t.type_scale.small),
            weight: Some(600.0),
            ..Default::default()
        },
    );

    // Choices.
    rule(
        "check",
        None,
        States::NONE,
        StylePatch {
            border: Some(line),
            border_width: Some(t.stroke.max(1.5)),
            radius: Some(r.small),
            background: Some(c.panel),
            ..Default::default()
        },
    );
    rule(
        "check",
        None,
        States::CHECKED,
        StylePatch {
            background: Some(c.accent),
            border: Some(c.accent),
            foreground: Some(d.on_accent),
            ..Default::default()
        },
    );
    rule(
        "switch",
        None,
        States::NONE,
        StylePatch {
            background: Some(line),
            radius: Some(999.0),
            foreground: Some(d.text),
            ..Default::default()
        },
    );
    rule(
        "switch",
        None,
        States::CHECKED,
        StylePatch { background: Some(c.accent), foreground: Some(d.on_accent), ..Default::default() },
    );
    for w in ["check", "switch", "radio"] {
        rule(w, None, States::DISABLED, disabled.clone());
    }
    rule(
        "radio",
        None,
        States::NONE,
        StylePatch {
            border: Some(line),
            border_width: Some(t.stroke.max(1.5)),
            radius: Some(999.0),
            background: Some(c.panel),
            foreground: Some(c.accent),
            ..Default::default()
        },
    );
    rule("radio", None, States::CHECKED, StylePatch { border: Some(c.accent), ..Default::default() });
    rule(
        "segmented",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.card),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.medium),
            padding: Some(Insets::all(2.0)),
            ..Default::default()
        },
    );
    rule(
        "segment",
        None,
        States::NONE,
        StylePatch {
            foreground: Some(d.muted),
            radius: Some(r.small),
            padding: Some(pad(u, u * 2.5)),
            weight: Some(600.0),
            ..Default::default()
        },
    );
    rule(
        "segment",
        None,
        States::HOVER,
        StylePatch { foreground: Some(d.text), background: Some(d.hover), ..Default::default() },
    );
    rule(
        "segment",
        None,
        States::SELECTED,
        StylePatch { background: Some(c.accent), foreground: Some(d.on_accent), ..Default::default() },
    );
    rule(
        "chip",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.card),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(999.0),
            padding: Some(pad(u * 0.5, u * 2.0)),
            text_size: Some(t.type_scale.small),
            ..Default::default()
        },
    );
    rule(
        "chip",
        None,
        States::SELECTED,
        StylePatch { background: Some(d.selected), border: Some(c.accent), ..Default::default() },
    );
    rule(
        "swatch",
        None,
        States::NONE,
        StylePatch {
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.small),
            ..Default::default()
        },
    );
    rule("swatch", None, States::HOVER, StylePatch { border: Some(d.text), ..Default::default() });

    // Lists and menus.
    rule(
        "list_item",
        None,
        States::NONE,
        StylePatch { radius: Some(r.small), padding: Some(pad(u * 1.5, u * 2.0)), ..Default::default() },
    );
    rule("list_item", None, States::HOVER, StylePatch { background: Some(d.hover), ..Default::default() });
    rule(
        "list_item",
        None,
        States::PRESSED,
        StylePatch { background: Some(d.pressed), ..Default::default() },
    );
    rule(
        "list_item",
        None,
        States::SELECTED,
        StylePatch { background: Some(d.selected), ..Default::default() },
    );
    rule(
        "menu",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.panel),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.medium),
            padding: Some(Insets::all(u)),
            ..Default::default()
        },
    );
    rule(
        "menu_item",
        None,
        States::NONE,
        StylePatch { radius: Some(r.small), padding: Some(pad(u * 1.5, u * 2.5)), ..Default::default() },
    );
    rule("menu_item", None, States::HOVER, StylePatch { background: Some(d.hover), ..Default::default() });
    rule(
        "menu_item",
        None,
        States::FOCUS_VISIBLE,
        // Menus show the focused item by highlighting it, as every platform does.
        StylePatch { background: Some(d.hover), outline_width: Some(0.0), ..Default::default() },
    );
    rule(
        "menu_item",
        Some("danger"),
        States::NONE,
        StylePatch { foreground: Some(danger), ..Default::default() },
    );

    // Feedback.
    rule(
        "badge",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.accent),
            foreground: Some(d.on_accent),
            radius: Some(999.0),
            padding: Some(pad(u * 0.25, u * 1.5)),
            text_size: Some(t.type_scale.small),
            weight: Some(700.0),
            ..Default::default()
        },
    );
    rule(
        "progress",
        None,
        States::NONE,
        StylePatch {
            background: Some(line),
            foreground: Some(c.accent),
            radius: Some(999.0),
            ..Default::default()
        },
    );
    rule("spinner", None, States::NONE, StylePatch { foreground: Some(c.accent), ..Default::default() });
    // Live audio visuals (the viz pack): the ring, sparkline and bars take the accent; the meter
    // sits on a track and draws its zones from the ok, warn and danger colors.
    for w in ["speaking_ring", "sparkline", "bars"] {
        rule(w, None, States::NONE, StylePatch { foreground: Some(c.accent), ..Default::default() });
    }
    rule(
        "speaking_ring",
        Some("muted"),
        States::NONE,
        StylePatch { foreground: Some(danger), ..Default::default() },
    );
    rule(
        "level_meter",
        None,
        States::NONE,
        StylePatch { background: Some(line), radius: Some(r.small), ..Default::default() },
    );
    rule(
        "meter_threshold",
        None,
        States::NONE,
        StylePatch { foreground: Some(d.text), radius: Some(r.small), ..Default::default() },
    );
    rule(
        "toast",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.card),
            foreground: Some(d.text),
            border: Some(line),
            border_width: Some(t.stroke),
            radius: Some(r.medium),
            padding: Some(pad(u * 2.0, u * 3.0)),
            ..Default::default()
        },
    );
    rule("toast", Some("error"), States::NONE, StylePatch { border: Some(danger), ..Default::default() });
    rule(
        "tooltip",
        None,
        States::NONE,
        StylePatch {
            background: Some(d.text),
            foreground: Some(c.bg),
            radius: Some(r.small),
            padding: Some(pad(u, u * 2.0)),
            text_size: Some(t.type_scale.small),
            ..Default::default()
        },
    );
    rule(
        "empty_state",
        None,
        States::NONE,
        StylePatch { foreground: Some(d.muted), padding: Some(Insets::all(u * 6.0)), ..Default::default() },
    );
    rule(
        "title_bar",
        None,
        States::NONE,
        StylePatch { padding: Some(pad(u, u * 2.0)), ..Default::default() },
    );
    rule(
        "lock_screen",
        None,
        States::NONE,
        StylePatch { background: Some(c.bg), padding: Some(Insets::all(u * 6.0)), ..Default::default() },
    );

    // Navigation.
    rule(
        "tab_strip",
        None,
        States::NONE,
        StylePatch { border: Some(line), border_width: Some(0.0), ..Default::default() },
    );
    rule(
        "tab",
        None,
        States::NONE,
        StylePatch {
            foreground: Some(d.muted),
            radius: Some(r.small),
            padding: Some(pad(u, u * 3.0)),
            ..Default::default()
        },
    );
    rule(
        "tab",
        None,
        States::HOVER,
        StylePatch { foreground: Some(d.text), background: Some(d.hover), ..Default::default() },
    );
    rule(
        "tab",
        None,
        States::SELECTED,
        StylePatch {
            foreground: Some(d.text),
            background: Some(d.selected),
            weight: Some(600.0),
            ..Default::default()
        },
    );
    for w in ["nav_item", "list_item"] {
        rule(w, None, States::PRESSED, StylePatch { background: Some(d.pressed), ..Default::default() });
    }
    rule(
        "nav_item",
        None,
        States::NONE,
        StylePatch {
            foreground: Some(d.muted),
            radius: Some(r.small),
            padding: Some(pad(u * 1.5, u * 2.5)),
            ..Default::default()
        },
    );
    rule(
        "nav_item",
        None,
        States::HOVER,
        StylePatch { foreground: Some(d.text), background: Some(d.hover), ..Default::default() },
    );
    rule(
        "nav_item",
        None,
        States::SELECTED,
        StylePatch {
            foreground: Some(d.text),
            background: Some(d.selected),
            weight: Some(600.0),
            ..Default::default()
        },
    );
    rule(
        "nav_rail",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.panel),
            border: Some(line),
            padding: Some(Insets::all(u)),
            ..Default::default()
        },
    );
    rule(
        "bottom_bar",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.panel),
            border: Some(line),
            border_width: Some(t.stroke),
            padding: Some(Insets::all(u)),
            ..Default::default()
        },
    );
    rule(
        "toolbar",
        None,
        States::NONE,
        StylePatch { background: Some(c.panel), padding: Some(Insets::all(u)), ..Default::default() },
    );
    rule(
        "drawer",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.panel),
            border: Some(line),
            border_width: Some(t.stroke),
            padding: Some(Insets::all(u * 3.0)),
            ..Default::default()
        },
    );

    // Data: tables and trees. The table or tree itself takes focus and shows it on the
    // row the keyboard is on (its cursor), drawn inside the row so scrolling never clips it.
    for w in ["table", "tree", "grid"] {
        rule(w, None, States::NONE, StylePatch { outline_width: Some(0.0), ..Default::default() });
    }
    // Grid tiles: rounded, the cursor's ring drawn inside the tile.
    rule(
        "grid_tile",
        None,
        States::NONE,
        StylePatch { radius: Some(r.medium), outline_offset: Some(-2.0), ..Default::default() },
    );
    rule("grid_tile", None, States::HOVER, StylePatch { background: Some(d.hover), ..Default::default() });
    rule(
        "grid_tile",
        None,
        States::SELECTED,
        StylePatch { background: Some(d.selected), ..Default::default() },
    );
    rule("grid_tile", None, States::DRAGGING, StylePatch { opacity: Some(0.6), ..Default::default() });
    // The rubber band: the accent, see-through, with a solid edge.
    rule(
        "grid_band",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.accent.with_alpha(0.18)),
            border: Some(c.accent),
            border_width: Some(t.stroke.max(1.0)),
            ..Default::default()
        },
    );
    rule(
        "table_header_bar",
        None,
        States::NONE,
        StylePatch {
            background: Some(c.panel),
            border: Some(line),
            border_width: Some(0.0),
            ..Default::default()
        },
    );
    rule(
        "table_heading",
        None,
        States::NONE,
        StylePatch { foreground: Some(d.muted), outline_offset: Some(-2.0), ..Default::default() },
    );
    rule(
        "table_heading",
        None,
        States::HOVER,
        StylePatch { foreground: Some(d.text), background: Some(d.hover), ..Default::default() },
    );
    rule("column_grip", None, States::HOVER, StylePatch { background: Some(d.focus), ..Default::default() });
    for w in ["table_row", "tree_item"] {
        rule(w, None, States::NONE, StylePatch { outline_offset: Some(-2.0), ..Default::default() });
        rule(w, None, States::HOVER, StylePatch { background: Some(d.hover), ..Default::default() });
        rule(w, None, States::SELECTED, StylePatch { background: Some(d.selected), ..Default::default() });
        rule(w, None, States::DRAGGING, StylePatch { opacity: Some(0.6), ..Default::default() });
    }
    // A row marked to move (cut).
    rule("tree_item", Some("cut"), States::NONE, StylePatch { opacity: Some(0.55), ..Default::default() });
    rule(
        "property_group",
        None,
        States::NONE,
        StylePatch {
            foreground: Some(d.muted),
            weight: Some(600.0),
            padding: Some(pad(u, 0.0)),
            ..Default::default()
        },
    );
    // An image still loading shows a placeholder the size it will be.
    rule(
        "image",
        Some("loading"),
        States::NONE,
        StylePatch { background: Some(d.hover), radius: Some(r.small), ..Default::default() },
    );
    drop(rule);
    s
}
