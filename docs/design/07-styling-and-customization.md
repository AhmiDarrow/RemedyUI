# 7. Styling, markup and user customization

Two audiences change how a RemedyUI app looks and is arranged:

- **Developers** need a style system that is predictable and fast.
- **Users**, by the code-fluid tenet, should be able to restyle and rearrange the apps they use.

One system serves both: tokens, styles and an optional markup layer. All of them are data that can be edited, and all are safe to load from untrusted sources.

## Tokens

A **theme** is a typed set of design tokens, organized the way modern design systems are (Material 3, Fluent 2, the W3C design-tokens format):

| Group | Examples |
|---|---|
| Color roles | `bg`, `surface`, `surface_raised`, `on_surface`, `muted`, `accent`, `on_accent`, `danger`, `warn`, `ok`, `line`, `focus`, app-defined roles |
| Type scale | `caption`, `body`, `body_strong`, `heading`, `title`, `display`; each a family, size, weight, line height and tracking |
| Spacing | `s1` … `s8` on a base unit (4 dp by default) |
| Shape | Corner radii `r0` … `r4`, stroke widths |
| Elevation | Shadow sets per level, or none |
| Motion | Durations and curves: `quick`, `standard`, `emphasized`; spring presets |
| Density | `compact`, `comfortable`, `spacious`; scales control heights and paddings |
| Assets | Background images, nine-slice frames, icon set |

- **Derived tokens** are computed once per theme change, not per widget: hover, pressed and selected fills from the base colors; contrast-balanced text colors (WCAG 4.5:1 and 7:1, or APCA when the theme asks); the high-contrast variant.
- **Tokens are signals.** Changing one token (a user's accent pick, a skin editor) updates only the elements that read it.
- **System links.** A token can follow the OS: the accent color, light or dark, high contrast and forced colors, text scale.

## Styles

- **A style** is a typed patch of element properties: colors, padding, size, border, corner radius, shadow, text style, transitions.
- **Variants and states.** Each widget declares named variants (`primary`, `quiet`, `danger`) and the states it can be in (`hover`, `pressed`, `focus_visible`, `disabled`, `selected`, `checked`, `invalid`, `dragging`). The theme supplies a style per variant and state.
- **No selector cascade.** There is no CSS-style matching over the tree. That is the slowest and least predictable part of the web. A style comes from exactly three places, in a known order:
  1. the widget's default
  2. the theme's entry for that widget, variant and state
  3. properties the app set on that element
- **Inheritance.** Only text properties are inherited (font, size, color, line height, direction), through context, as in Flutter and CSS.
- **Speed.** Resolved styles are cached by (widget kind, variant, state bits, theme version). Hovering a button is a cache hit, not a computation.

```rust
button(t("Send")).variant(Variant::Primary)   // theme decides the look
button(t("Send")).style(|s| s.radius(Radius::R3).padding_x(Space::S4))   // local override, still token-based
```

## Markup (`.rui`)

Rust builders are the primary API. An **optional markup language** (`.rui`) describes layout and styling as data, the way QML, XAML and Slint markup do. It earns its place three ways:

- **Live editing.** The devtools reload a changed `.rui` file instantly, with signal state kept.
- **Designer-friendly files** that do not need a Rust toolchain to tweak.
- **User customization.** A user can override an app's `.rui` files to rearrange its screens.

```
// settings_voice.rui
column(gap: s3) {
    section(title: @t("Input")) {
        select(items: $input_devices, selected: $input_device)
        slider(value: $input_gain, range: 0..200, suffix: "%", on_commit: !save_gain)
        meter(level: $input_level)
    }
    switch(label: @t("Noise suppression"), checked: $noise_suppression)
}
```

- **Bindings only.** Markup can bind to signals (`$name`) and commands (`!name`) that the Rust side exposes for that file, and use tokens (`s3`) and translations (`@t`). It has no expressions beyond literals and these references. It cannot call arbitrary code, read files, open the network or reach anything not exposed. It is data, so loading a user's or a stranger's `.rui` is safe.
- **Release builds.** `.rui` files are compiled to Rust at build time by the `remedy-ui-rui` build helper, so release builds pay nothing for the markup layer, unless the app enables user overrides. Then the interpreter (a small crate) loads override files at run time.
- **Errors** point at the file, line and column, with the list of exposed names when a reference is wrong. A broken override falls back to the built-in file and reports why.

## User customization: the code-fluid layer

A RemedyUI app can expose, with no extra code beyond turning it on:

| What the user can change | Stored as | Default |
|---|---|---|
| Theme: colors, fonts, sizes, spacing, radii, motion, density | A theme file (W3C design-tokens JSON, or an app format such as FreeSpeak's `.fskin` mapped to tokens) | On |
| Shortcuts | Command id → chord list | On |
| Toolbars and menus: order, visibility, added commands | A list of command ids per slot | On where the app declares slots |
| Screen layouts | `.rui` overrides | Off; the app turns it on |
| Text: every visible string | Message files in the user's override folder | Off; the app turns it on |

- **Where it lives.** Customizations are files in the app's user-config folder, readable and editable by hand and diffable. Each also has an in-app editor built from RemedyUI widgets: a theme editor with live preview and contrast warnings, a shortcut editor with conflict detection, and a toolbar editor using drag and drop.
- **Undoable and resettable.** Every customization change goes through the undo service. "Reset to default" exists per item and for everything. A customization that fails to load (bad file, broken override) is skipped with a visible notice and a one-click reset, so the app always starts.
- **Untrusted by default.** Customizations from another person (a shared theme, a downloaded layout) are parsed as data, with the same safety as anything from the network. Themes may include images and fonts, which go through the media and font safety rules. Shaders and external URLs in themes are refused unless the user turns them on.
- **One format across apps.** All Remedy apps read the same token format, so a theme a user makes for one app can be applied to all of them, and to the RemedyOS desktop.
