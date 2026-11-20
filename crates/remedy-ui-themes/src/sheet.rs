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
    let disabled = StylePatch::default().opacity(0.45);
    let mut s = StyleSheet::new();
    let mut rule =
        |w: &'static str, v: Option<&'static str>, st: States, patch: StylePatch| s.rule(w, v, st, patch);

    // Surfaces.
    rule("window", None, States::NONE, StylePatch::default().background(c.bg).foreground(d.text));
    rule("panel", None, States::NONE, StylePatch::default().background(c.panel));
    rule(
        "card",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.card)
            .border(line)
            .border_width(t.stroke)
            .radius(r.large)
            .padding(Insets::all(u * 4.0)),
    );
    // Boards: areas are cards; while arranging, areas and widgets show hover and selection.
    rule(
        "board_area",
        None,
        States::NONE,
        StylePatch::default().background(c.card).border(line).border_width(t.stroke).radius(r.large),
    );
    rule(
        "board_area",
        None,
        States::SELECTED,
        StylePatch::default().border(c.accent).border_width(t.stroke.max(1.0) * 2.0),
    );
    rule("board_area", None, States::DROP_TARGET, StylePatch::default().border(c.accent));
    rule("board_item", None, States::NONE, StylePatch::default().radius(r.medium));
    rule("board_item", None, States::HOVER, StylePatch::default().background(d.hover));
    rule("board_item", None, States::SELECTED, StylePatch::default().background(d.selected));
    rule("board_item", None, States::DRAGGING, StylePatch::default().opacity(0.6));
    // Code: monospace text, quiet line numbers, added and removed lines tinted.
    rule("code_text", None, States::NONE, StylePatch::default().mono(true));
    rule("code_input", None, States::NONE, StylePatch::default().mono(true));
    rule("code_mark", None, States::NONE, StylePatch::default().mono(true).foreground(d.muted));
    rule("code_gutter", None, States::NONE, StylePatch::default().mono(true).foreground(d.muted));
    rule("code_view", None, States::NONE, StylePatch::default().background(c.card));
    rule(
        "code_line",
        Some("added"),
        States::NONE,
        StylePatch::default().background(c.card.mix_oklab(c.ok, 0.18)),
    );
    rule(
        "code_line",
        Some("removed"),
        States::NONE,
        StylePatch::default().background(c.card.mix_oklab(c.danger, 0.18)),
    );
    // Terminals: monospace on the terminal's own background (the `term-bg` and `term-fg`
    // tokens, else the window's), with a focus border; cells take their colors from the
    // `term-0` … `term-15` tokens (remedy-ui-widgets-term).
    let term_bg = c.custom.get("term-bg").copied().unwrap_or(c.bg);
    let term_fg = c.custom.get("term-fg").copied().unwrap_or(d.text);
    rule(
        "terminal",
        None,
        States::NONE,
        StylePatch::default()
            .mono(true)
            .background(term_bg)
            .foreground(term_fg)
            .border(line)
            .border_width(t.stroke)
            .padding(Insets::all(u * 2.0)),
    );
    rule("terminal", None, States::FOCUS, StylePatch::default().border(d.focus).outline_width(0.0));
    rule("terminal_measure", None, States::NONE, StylePatch::default().mono(true));
    // Video: bars where a picture's shape differs from its place are black, as on every screen.
    rule("video_surface", None, States::NONE, StylePatch::default().background(Color::BLACK));
    // Captions over a picture: light text on a dark box, readable over any frame, as on
    // every screen. The player's stage is black like a video surface's bars.
    rule(
        "player_caption",
        None,
        States::NONE,
        StylePatch::default()
            .background(Color::BLACK.with_alpha(0.8))
            .foreground(Color::WHITE)
            .radius(r.small),
    );
    rule("player_stage", None, States::NONE, StylePatch::default().background(Color::BLACK));
    // Documents: quotes get a bar, code a box, tables lines between cells.
    rule("md_quote_bar", None, States::NONE, StylePatch::default().background(line));
    rule("md_quote_body", None, States::NONE, StylePatch::default().foreground(d.muted));
    rule("md_marker", None, States::NONE, StylePatch::default().foreground(d.muted));
    rule(
        "md_code_scroll",
        None,
        States::NONE,
        StylePatch::default().background(c.card).border(line).border_width(t.stroke).radius(r.medium),
    );
    rule("md_code", None, States::NONE, StylePatch::default().mono(true).padding(pad(u * 2.0, u * 3.0)));
    rule(
        "md_table",
        None,
        States::NONE,
        StylePatch::default().border(line).border_width(t.stroke).radius(r.small),
    );
    rule(
        "md_cell",
        None,
        States::NONE,
        StylePatch::default().border(line).border_width(t.stroke * 0.5).padding(pad(u * 1.5, u * 2.5)),
    );
    rule("md_cell", Some("head"), States::NONE, StylePatch::default().weight(700.0).background(c.card));
    // A card that stands out (the user's own message, a highlighted item).
    rule("card", Some("accent"), States::NONE, StylePatch::default().background(d.selected).border(c.accent));
    rule("divider", None, States::NONE, StylePatch::default().background(line));
    rule("scrim", None, States::NONE, StylePatch::default().background(Color::BLACK.with_alpha(0.5)));
    rule(
        "dialog",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.panel)
            .border(line)
            .border_width(t.stroke)
            .radius(r.large)
            .padding(Insets::all(u * 5.0)),
    );

    // Text.
    // Plain labels and headings inherit their text color (white on a primary button, the
    // window's text color elsewhere); variants that mean something set their own.
    rule("label", Some("muted"), States::NONE, StylePatch::default().foreground(d.muted));
    rule(
        "label",
        Some("small"),
        States::NONE,
        StylePatch::default().foreground(d.muted).text_size(t.type_scale.small),
    );
    rule("label", Some("error"), States::NONE, StylePatch::default().foreground(danger));
    rule("label", Some("strong"), States::NONE, StylePatch::default().weight(700.0));
    rule(
        "label",
        Some("code"),
        States::NONE,
        StylePatch::default()
            .mono(true)
            .background(c.card)
            .border(line)
            .border_width(t.stroke)
            .radius(r.small)
            .padding(pad(u * 2.0, u * 3.0)),
    );
    rule(
        "heading",
        Some("1"),
        States::NONE,
        StylePatch::default().text_size(t.type_scale.title + 6.0).weight(700.0),
    );
    rule(
        "heading",
        Some("2"),
        States::NONE,
        StylePatch::default().text_size(t.type_scale.title).weight(650.0),
    );
    rule(
        "heading",
        Some("3"),
        States::NONE,
        StylePatch::default().text_size(t.type_scale.body + 2.0).weight(600.0),
    );

    // Buttons.
    let base = StylePatch::default()
        .background(c.card)
        .foreground(d.text)
        .border(line)
        .border_width(t.stroke)
        .radius(r.medium)
        .padding(pad(u * 1.5, u * 3.0))
        .weight(600.0);
    for w in ["button", "icon_button"] {
        rule(w, None, States::NONE, base.clone());
        rule(w, None, States::HOVER, StylePatch::default().background(d.hover));
        rule(w, None, States::PRESSED, StylePatch::default().background(d.pressed));
        // A button that stays on (bold in a toolbar, a toggle) shows it.
        rule(w, None, States::CHECKED, StylePatch::default().background(d.selected).border(c.accent));
        rule(w, None, States::DISABLED, disabled.clone());
        rule(
            w,
            Some("primary"),
            States::NONE,
            StylePatch::default().background(c.accent).foreground(d.on_accent).border(c.accent),
        );
        rule(w, Some("primary"), States::HOVER, StylePatch::default().background(d.accent_hover));
        rule(w, Some("primary"), States::PRESSED, StylePatch::default().background(d.accent_pressed));
        rule(
            w,
            Some("quiet"),
            States::NONE,
            StylePatch::default().background(Color::TRANSPARENT).border(Color::TRANSPARENT),
        );
        rule(w, Some("quiet"), States::HOVER, StylePatch::default().background(d.hover));
        rule(
            w,
            Some("danger"),
            States::NONE,
            StylePatch::default().foreground(danger).border(danger.with_alpha(0.6)),
        );
        rule(
            w,
            Some("danger"),
            States::HOVER,
            StylePatch::default().background(c.card.mix_oklab(c.danger, 0.18)),
        );
        let solid = c.danger;
        rule(
            w,
            Some("danger_solid"),
            States::NONE,
            StylePatch::default().background(solid).foreground(on(solid, t)).border(solid),
        );
        rule(
            w,
            Some("danger_solid"),
            States::HOVER,
            StylePatch::default().background(solid.mix_oklab(Color::BLACK, 0.1)),
        );
    }
    rule(
        "icon_button",
        None,
        States::NONE,
        StylePatch::default().padding(Insets::all(u * 1.5)).radius(r.small),
    );

    // Inputs.
    for w in ["text_input", "secret_input", "text_area", "search"] {
        rule(
            w,
            None,
            States::NONE,
            StylePatch::default()
                .background(c.panel)
                .foreground(d.text)
                .border(line)
                .border_width(t.stroke)
                .radius(r.small)
                .padding(pad(u * 1.5, u * 2.0)),
        );
        rule(w, None, States::HOVER, StylePatch::default().border(line.mix_oklab(d.text, 0.25)));
        rule(w, None, States::FOCUS, StylePatch::default().border(d.focus).outline_width(0.0));
        rule(w, None, States::INVALID, StylePatch::default().border(danger));
        rule(w, None, States::DISABLED, disabled.clone());
    }
    // An input inside another control that already draws the input's frame (search).
    rule(
        "text_input",
        Some("bare"),
        States::NONE,
        StylePatch::default().background(Color::TRANSPARENT).border(Color::TRANSPARENT).padding(Insets::ZERO),
    );
    rule(
        "text_input",
        Some("bare"),
        States::FOCUS,
        StylePatch::default().border(Color::TRANSPARENT).outline_width(2.0),
    );
    rule(
        "field",
        None,
        States::NONE,
        StylePatch::default().foreground(d.muted).text_size(t.type_scale.small).weight(600.0),
    );

    // Choices.
    rule(
        "check",
        None,
        States::NONE,
        StylePatch::default()
            .border(line)
            .border_width(t.stroke.max(1.5))
            .radius(r.small)
            .background(c.panel),
    );
    rule(
        "check",
        None,
        States::CHECKED,
        StylePatch::default().background(c.accent).border(c.accent).foreground(d.on_accent),
    );
    rule(
        "switch",
        None,
        States::NONE,
        StylePatch::default().background(line).radius(999.0).foreground(d.text),
    );
    rule("switch", None, States::CHECKED, StylePatch::default().background(c.accent).foreground(d.on_accent));
    for w in ["check", "switch", "radio"] {
        rule(w, None, States::DISABLED, disabled.clone());
    }
    rule(
        "radio",
        None,
        States::NONE,
        StylePatch::default()
            .border(line)
            .border_width(t.stroke.max(1.5))
            .radius(999.0)
            .background(c.panel)
            .foreground(c.accent),
    );
    rule("radio", None, States::CHECKED, StylePatch::default().border(c.accent));
    rule(
        "segmented",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.card)
            .border(line)
            .border_width(t.stroke)
            .radius(r.medium)
            .padding(Insets::all(2.0)),
    );
    rule(
        "segment",
        None,
        States::NONE,
        StylePatch::default().foreground(d.muted).radius(r.small).padding(pad(u, u * 2.5)).weight(600.0),
    );
    rule("segment", None, States::HOVER, StylePatch::default().foreground(d.text).background(d.hover));
    rule(
        "segment",
        None,
        States::SELECTED,
        StylePatch::default().background(c.accent).foreground(d.on_accent),
    );
    rule(
        "chip",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.card)
            .border(line)
            .border_width(t.stroke)
            .radius(999.0)
            .padding(pad(u * 0.5, u * 2.0))
            .text_size(t.type_scale.small),
    );
    rule("chip", None, States::SELECTED, StylePatch::default().background(d.selected).border(c.accent));
    rule(
        "swatch",
        None,
        States::NONE,
        StylePatch::default().border(line).border_width(t.stroke).radius(r.small),
    );
    rule("swatch", None, States::HOVER, StylePatch::default().border(d.text));

    // Lists and menus.
    rule(
        "list_item",
        None,
        States::NONE,
        StylePatch::default().radius(r.small).padding(pad(u * 1.5, u * 2.0)),
    );
    rule("list_item", None, States::HOVER, StylePatch::default().background(d.hover));
    rule("list_item", None, States::PRESSED, StylePatch::default().background(d.pressed));
    rule("list_item", None, States::SELECTED, StylePatch::default().background(d.selected));
    rule(
        "menu",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.panel)
            .border(line)
            .border_width(t.stroke)
            .radius(r.medium)
            .padding(Insets::all(u)),
    );
    rule(
        "menu_item",
        None,
        States::NONE,
        StylePatch::default().radius(r.small).padding(pad(u * 1.5, u * 2.5)),
    );
    // Menus show the focused item by highlighting it, as every platform does; the pointer
    // resting on an item focuses it, so the highlight follows the pointer and the keyboard
    // alike, and there is only ever one.
    for st in [States::FOCUS, States::FOCUS_VISIBLE] {
        rule("menu_item", None, st, StylePatch::default().background(d.hover).outline_width(0.0));
    }
    // An item whose submenu is open stays marked while focus is in the submenu.
    rule("menu_item", None, States::SELECTED, StylePatch::default().background(d.hover));
    rule("menu_item", None, States::DISABLED, disabled.clone());
    rule("menu_item", Some("danger"), States::NONE, StylePatch::default().foreground(danger));
    rule(
        "menu_shortcut",
        None,
        States::NONE,
        StylePatch::default().foreground(d.muted).text_size(t.type_scale.small),
    );
    rule("menu_separator", None, States::NONE, StylePatch::default().background(line));
    rule("menu_bar", None, States::NONE, StylePatch::default().background(c.panel).padding(pad(u * 0.5, u)));
    rule("menu_bar_item", None, States::NONE, StylePatch::default().radius(r.small).padding(pad(u, u * 2.5)));
    rule("menu_bar_item", None, States::HOVER, StylePatch::default().background(d.hover));
    // The title the keyboard is on is highlighted; Tab-style keyboard focus also gets the ring,
    // which tells a focused open title from an open one.
    rule("menu_bar_item", None, States::FOCUS, StylePatch::default().background(d.hover));
    rule("menu_bar_item", None, States::SELECTED, StylePatch::default().background(d.selected));
    rule("menu_bar_item", None, States::DISABLED, disabled.clone());

    // Feedback.
    rule(
        "badge",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.accent)
            .foreground(d.on_accent)
            .radius(999.0)
            .padding(pad(u * 0.25, u * 1.5))
            .text_size(t.type_scale.small)
            .weight(700.0),
    );
    // A status light's color while on; its variant says what kind of status it is.
    rule("lamp", None, States::NONE, StylePatch::default().foreground(c.ok));
    rule("lamp", Some("ok"), States::NONE, StylePatch::default().foreground(c.ok));
    rule("lamp", Some("accent"), States::NONE, StylePatch::default().foreground(c.accent));
    rule("lamp", Some("warn"), States::NONE, StylePatch::default().foreground(c.warn));
    rule("lamp", Some("danger"), States::NONE, StylePatch::default().foreground(c.danger));
    rule(
        "progress",
        None,
        States::NONE,
        StylePatch::default().background(line).foreground(c.accent).radius(999.0),
    );
    rule("spinner", None, States::NONE, StylePatch::default().foreground(c.accent));
    // Live audio visuals (the viz pack): the ring, sparkline and bars take the accent; the meter
    // sits on a track and draws its zones from the ok, warn and danger colors.
    for w in ["speaking_ring", "sparkline", "bars"] {
        rule(w, None, States::NONE, StylePatch::default().foreground(c.accent));
    }
    rule("speaking_ring", Some("muted"), States::NONE, StylePatch::default().foreground(danger));
    rule("level_meter", None, States::NONE, StylePatch::default().background(line).radius(r.small));
    rule("meter_threshold", None, States::NONE, StylePatch::default().foreground(d.text).radius(r.small));
    // Music (the music pack): notation and neck diagrams are drawn in the text color; the
    // neck itself is a card-colored surface.
    for w in ["fretboard", "tab_staff", "note_staff", "chord_diagram"] {
        rule(w, None, States::NONE, StylePatch::default().foreground(d.text).radius(r.small));
    }
    // Extras: the star color, the carousel's frame and dots, the wizard's markers and the swatches.
    rule("rating", None, States::NONE, StylePatch::default().foreground(c.warn));
    rule("qr_code", None, States::NONE, StylePatch::default().radius(r.small));
    rule("carousel_viewport", None, States::NONE, StylePatch::default().radius(r.medium));
    for w in ["carousel_dot_target", "step_marker_cell"] {
        rule(w, None, States::NONE, StylePatch::default().radius(r.small));
        rule(w, None, States::HOVER, StylePatch::default().background(d.hover));
    }
    rule(
        "swatch_cell",
        None,
        States::NONE,
        StylePatch::default().radius(r.small).border(line).border_width(t.stroke),
    );
    rule("swatch_cell", None, States::HOVER, StylePatch::default().border(d.text));
    rule(
        "toast",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.card)
            .foreground(d.text)
            .border(line)
            .border_width(t.stroke)
            .radius(r.medium)
            .padding(pad(u * 2.0, u * 3.0)),
    );
    rule("toast", Some("error"), States::NONE, StylePatch::default().border(danger));
    rule(
        "tooltip",
        None,
        States::NONE,
        StylePatch::default()
            .background(d.text)
            .foreground(c.bg)
            .radius(r.small)
            .padding(pad(u, u * 2.0))
            .text_size(t.type_scale.small),
    );
    rule(
        "empty_state",
        None,
        States::NONE,
        StylePatch::default().foreground(d.muted).padding(Insets::all(u * 6.0)),
    );
    rule("title_bar", None, States::NONE, StylePatch::default().padding(pad(u, u * 2.0)));
    rule(
        "lock_screen",
        None,
        States::NONE,
        StylePatch::default().background(c.bg).padding(Insets::all(u * 6.0)),
    );

    // Navigation.
    rule("tab_strip", None, States::NONE, StylePatch::default().border(line).border_width(0.0));
    rule(
        "tab",
        None,
        States::NONE,
        StylePatch::default().foreground(d.muted).radius(r.small).padding(pad(u, u * 3.0)),
    );
    rule("tab", None, States::HOVER, StylePatch::default().foreground(d.text).background(d.hover));
    rule(
        "tab",
        None,
        States::SELECTED,
        StylePatch::default().foreground(d.text).background(d.selected).weight(600.0),
    );
    for w in ["nav_item", "list_item"] {
        rule(w, None, States::PRESSED, StylePatch::default().background(d.pressed));
    }
    rule(
        "nav_item",
        None,
        States::NONE,
        StylePatch::default().foreground(d.muted).radius(r.small).padding(pad(u * 1.5, u * 2.5)),
    );
    rule("nav_item", None, States::HOVER, StylePatch::default().foreground(d.text).background(d.hover));
    rule(
        "nav_item",
        None,
        States::SELECTED,
        StylePatch::default().foreground(d.text).background(d.selected).weight(600.0),
    );
    rule(
        "nav_rail",
        None,
        States::NONE,
        StylePatch::default().background(c.panel).border(line).padding(Insets::all(u)),
    );
    rule(
        "bottom_bar",
        None,
        States::NONE,
        StylePatch::default().background(c.panel).border(line).border_width(t.stroke).padding(Insets::all(u)),
    );
    rule("toolbar", None, States::NONE, StylePatch::default().background(c.panel).padding(Insets::all(u)));
    rule(
        "drawer",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.panel)
            .border(line)
            .border_width(t.stroke)
            .padding(Insets::all(u * 3.0)),
    );

    // Data: tables and trees. The table or tree itself takes focus and shows it on the
    // row the keyboard is on (its cursor), drawn inside the row so scrolling never clips it.
    for w in ["table", "tree", "grid"] {
        rule(w, None, States::NONE, StylePatch::default().outline_width(0.0));
    }
    // Grid tiles: rounded, the cursor's ring drawn inside the tile.
    rule("grid_tile", None, States::NONE, StylePatch::default().radius(r.medium).outline_offset(-2.0));
    rule("grid_tile", None, States::HOVER, StylePatch::default().background(d.hover));
    rule("grid_tile", None, States::SELECTED, StylePatch::default().background(d.selected));
    rule("grid_tile", None, States::DRAGGING, StylePatch::default().opacity(0.6));
    // The rubber band: the accent, see-through, with a solid edge.
    rule(
        "grid_band",
        None,
        States::NONE,
        StylePatch::default()
            .background(c.accent.with_alpha(0.18))
            .border(c.accent)
            .border_width(t.stroke.max(1.0)),
    );
    rule(
        "table_header_bar",
        None,
        States::NONE,
        StylePatch::default().background(c.panel).border(line).border_width(0.0),
    );
    rule("table_heading", None, States::NONE, StylePatch::default().foreground(d.muted).outline_offset(-2.0));
    rule("table_heading", None, States::HOVER, StylePatch::default().foreground(d.text).background(d.hover));
    rule("column_grip", None, States::HOVER, StylePatch::default().background(d.focus));
    for w in ["table_row", "tree_item"] {
        rule(w, None, States::NONE, StylePatch::default().outline_offset(-2.0));
        rule(w, None, States::HOVER, StylePatch::default().background(d.hover));
        rule(w, None, States::SELECTED, StylePatch::default().background(d.selected));
        rule(w, None, States::DRAGGING, StylePatch::default().opacity(0.6));
    }
    // A row marked to move (cut).
    rule("tree_item", Some("cut"), States::NONE, StylePatch::default().opacity(0.55));
    rule(
        "property_group",
        None,
        States::NONE,
        StylePatch::default().foreground(d.muted).weight(600.0).padding(pad(u, 0.0)),
    );
    // An image still loading shows a placeholder the size it will be.
    rule("image", Some("loading"), States::NONE, StylePatch::default().background(d.hover).radius(r.small));
    drop(rule);
    s
}
