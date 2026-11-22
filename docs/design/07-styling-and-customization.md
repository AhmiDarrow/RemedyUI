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
| Spacing | The spacing scale ([13](13-design-language.md#spacing-scale)) on a base unit (4 dp by default): half steps `s0_5` … `s3_5`, `s1` … `s8`, layout steps `s10` … `s24` |
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
- **Gradient backgrounds.** `background_layers` stacks gradients over the background colour, the first on top, as CSS `background-image`: linear at an angle, radial as a circle or an axis-aligned ellipse (sized by radii or CSS's `closest-side`, `closest-corner`, `farthest-side`, `farthest-corner`) at any position, and conic. Stops are colours or theme roles with an alpha (`color-mix(in srgb, var(--gold) 16%, transparent)`), placed as CSS places them. `Gradient::parse` reads CSS gradient text, so a web app's backgrounds carry over as written. They paint as display-list brushes (`Brush::Linear`, `Radial`, `Elliptical`, `Conic`), the same on both renderers.
- **Inheritance.** Only text properties are inherited (font, size, color, line height, direction), through context, as in Flutter and CSS.
- **Speed.** Resolved styles are cached by (widget kind, variant, state bits, theme version). Hovering a button is a cache hit, not a computation.

```rust
button(ui, t("Send"), send).variant("primary")   // theme decides the look
button(ui, t("Send"), send).style(Style::new().radius(Radius::R3).padding_x(Space::S4))   // local override, still token-based
```

- **Typed tokens in code.** `Space::S0`…`S8`, `Radius::R0`…`R4` (R4 fully rounded), `TextRole` (caption, body, body strong, heading, title, display) and `ColorRole` (the color roles, and `Custom("name")` for an app's own) are what `.gap()`, `.width()`, `.height()`, `.padding()` and `Style` take, beside plain numbers and colors. They resolve through the window's theme, and again whenever it changes. Padding set on only some sides keeps the theme's padding on the others.
- **Why `Style::new()` and not a closure.** Every builder property takes a constant, a signal or a closure (see [12](12-developer-experience.md)), and a closure always means "re-read this when its signals change". A style builder written as a closure would read as a reactive style, so token styles start from `Style::new()`; `.style(move || ...)` is the reactive form, layered over the constant ones.

## Markup (`.rui`)

Rust builders are the primary API. An **optional markup language** (`.rui`) describes layout and styling as data, the way QML, XAML and Slint markup do. It earns its place three ways:

- **Live editing.** The devtools reload a changed `.rui` file instantly, with signal state kept.
- **Designer-friendly files** that do not need a Rust toolchain to tweak.
- **User customization.** A user can override an app's `.rui` files to rearrange its screens.

```
// settings_voice.rui
column(gap: s3) {
    section(title: @t("Input")) {
        select(label: @t("Device"), items: $input_devices, selected: $input_device)
        slider(label: @t("Gain"), value: $input_gain, range: 0..200, step: 1)
        meter(level: $input_level)   // the app's own element, registered with the markup
    }
    switch(label: @t("Noise suppression"), checked: $noise_suppression)
    button(label: @t("Save"), on_press: !save_gain, variant: primary)
}
```

- **Bindings only.** Markup can bind to signals (`$name`) and commands (`!name`) that the Rust side exposes for that file, and use tokens (`s3`) and translations (`@t`). It has no expressions beyond literals and these references. It cannot call arbitrary code, read files, open the network or reach anything not exposed. It is data, so loading a user's or a stranger's `.rui` is safe.
- **Release builds.** `.rui` files are compiled to Rust at build time by the `remedy-ui-rui` build helper, so release builds pay nothing for the markup layer, unless the app enables user overrides. Then the interpreter (a small crate) loads override files at run time.
- **Errors** point at the file, line and column, with the list of exposed names when a reference is wrong. A broken override falls back to the built-in file and reports why.

### Scripts inside markup and the app's own elements

Markup has no logic, and some presentations need it: seats placed on an orbit, which window of a scene is lit. For those an app lets a sandboxed script (`remedy-ui-script`, any engine the app accepts) describe part of the screen, and places it like any element:

- **The element protocol, version 2** (`remedy_ui_script::PROTOCOL`). Scripts describe columns, rows, text, buttons, links, progress, dividers, **pictures** (`{ "kind": "image", "src", "alt", "width", "height" }`, resolved by the app) and **the app's own elements**: any kind the app offers with `describe_with` (a scene, a sprite, a part the app draws itself). An app element's properties named `on` or `on_...` are handlers; everything else is data for the app's builder. A kind the app does not offer is refused, so a script never shows half a widget, and a description holds at most 4096 handlers (`Limits::handlers`; pictures up to `Limits::image_side`, gaps up to `Limits::gap`, Rhai expressions nested up to `Limits::expr_depth`, all the app's to raise).
- **Mounting.** `remedy_ui::board::mount_script(ui, widget, settings, host, hooks)` renders a loaded script anywhere: in a board slot, in a screen built in Rust, or as an element the app registers with the markup (`Widgets::register("script", ...)`). `ScriptHooks` carries what the script reaches out through: links (the app asks first), pictures, the app's element builder, and where failures go. The script renders again when the app's `host` data changes and after each of its handlers; a failed render keeps the last good picture and is reported, so the app can count failures and set a script aside.
- **Handlers with values.** An app element's handler can hand the script a value (where a knob was turned to, which station a dial chose) through `ScriptWidget::act_with`; engines that take none run `act`.
- **Third-party engines.** `ScriptEngine` is a trait any crate implements for its language (`Language::new("lua")`) and adds to `Engines`; RemedyUI ships Rhai and WebAssembly, and FreeSpeak adds Lua 5.4 over its own sandbox.

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
