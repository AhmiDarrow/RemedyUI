# Theming and design tokens

A look in RemedyUI comes from two things:

- a [`Theme`](crate::style::Theme): typed design tokens (color roles, the type scale, spacing,
  radii, motion, density, stroke and font families);
- a [`StyleSheet`](crate::style::StyleSheet): rules that turn tokens into styles for each
  widget name, variant and state.

A style comes from exactly three places, in order: the widget's defaults, the sheet's rules
for that widget, variant and states, and properties set on the element itself. There is no
selector cascade, so what an element looks like never depends on where it sits. Resolved
styles are cached, and a theme change restyles only what it has to.

Every Remedy app shares one theme set ([`themes`](crate::themes), feature `widgets`), so a
person's theme follows them from app to app.

## The tokens

```
use remedy_ui::style::Theme;
use remedy_ui::themes;

let theme: Theme = themes::ThemeChoice::default().resolve(None, false);
assert_eq!(theme.name, "Dark Forest");

let c = &theme.colors;
let _surfaces = (c.bg, c.panel, c.card);
let _text = (c.text, c.muted);
let _roles = (c.accent, c.on_accent, c.danger, c.warn, c.ok);

// Spacing is a grid unit; `s(n)` is n units.
assert_eq!(theme.spacing.s(3), theme.spacing.unit * 3.0);
let _sizes = (theme.type_scale.small, theme.type_scale.body, theme.type_scale.title);
let _corners = (theme.radii.small, theme.radii.medium, theme.radii.large);
```

Some colors are worked out from the rest by [`Theme::derive`](crate::style::Theme::derive):
hover, pressed and selected fills, the focus ring, and text colors balanced so they meet WCAG
contrast on the background. Widgets read these instead of mixing colors themselves, which is
why every widget in every Remedy palette passes contrast checks.

```
use remedy_ui::themes;

for theme in themes::all() {
    let d = theme.derive();
    assert!(d.text.contrast(theme.colors.bg) >= 7.0, "{}", theme.name);
    assert!(d.muted.contrast(theme.colors.bg) >= 4.5, "{}", theme.name);
}
```

## The Remedy palettes

[`themes::PALETTES`](crate::themes::PALETTES) holds the twelve Remedy palettes, Dark Forest
first, including variants for deuteranopia, protanopia and tritanopia. People choose one with
a [`ThemeChoice`](crate::themes::ThemeChoice): a palette by id, or `System`, which follows
the platform between the Dark and Light palettes.
[`themes::high_contrast`](crate::themes::high_contrast) gives any theme's high-contrast
form, for the system setting.

```
use remedy_ui::themes::{self, ThemeChoice};

assert_eq!(themes::PALETTES.len(), 12);

let system = ThemeChoice::System;
let light = system.resolve(Some(false), false);
let dark = system.resolve(Some(true), false);
assert!(!light.derive().dark && dark.derive().dark);

let high = ThemeChoice::Palette("forest".into()).resolve(None, true);
assert!(high.name.ends_with("(high contrast)"));
```

An app starts styled with the first palette and the widget sheet
([`AppConfig::new`](crate::AppConfig::new)). To follow the system, listen with
[`App::on_appearance`](crate::App::on_appearance) and apply the theme with
[`App::set_theme`](crate::App::set_theme), which restyles every window:

```
# use remedy_ui::AppConfig;
# use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::themes::ThemeChoice;
use remedy_ui::{App, WindowSpec};

let choice = Signal::new(ThemeChoice::System);
let app = move |app: &App| {
    let a = app.clone();
    app.on_appearance(move |prefers_dark, high_contrast| {
        a.set_theme(choice.get_untracked().unwrap_or_default().resolve(prefers_dark, high_contrast));
    });
    app.open(WindowSpec::new("Themed"), |_, _| {});
};
# let mut test = TestApp::new(AppConfig::new("Themed"), app);
# let Some(ui) = test.ui() else { unreachable!("the window is open") };
# assert!(!ui.theme().name.is_empty());
```

## Typed tokens

Tokens name a place in the design system instead of a number, so autocomplete shows the
design system and a typo fails to compile. They resolve through the window's theme, and again
whenever the theme changes:

| Token | What it names |
|---|---|
| [`Space`](crate::style::Space) (`S0` to `S8`) | Steps of the spacing unit, for gaps, padding and sizes |
| [`Radius`](crate::style::Radius) (`R0` to `R4`) | Corner radii |
| [`TextRole`](crate::style::TextRole) | Text sizes and weights: `Caption`, `Body`, `BodyStrong`, `Heading`, `Title`, `Display` |
| [`ColorRole`](crate::style::ColorRole) | Color roles: `Bg`, `Panel`, `Card`, `Text`, `Muted`, `Accent`, `Danger`, ... and the theme's custom colors |

`gap`, `width`, `height` and `padding` take a [`Dp`](crate::style::Dp) (a token or plain
pixels) or a [`Pad`](crate::style::Pad); [`El::style`](crate::El::style) takes a
[`Style`](crate::style::Style), a style patch whose values may be tokens:

```
use remedy_ui::layout::Align;
use remedy_ui::style::{ColorRole, Pad, Radius, Space, States, Style, TextRole};
use remedy_ui::testing::TestApp;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let app = TestApp::window(move |ui, _| {
    let badge = w::text(ui, "New")
        .style(Style::new().background(ColorRole::Accent).foreground(ColorRole::OnAccent).radius(Radius::R2).text(TextRole::Caption))
        .padding(Pad::xy(Space::S2, Space::S1));
    let _ = s.set(badge.clone());
    let row = w::row(ui).gap(Space::S3).align(Align::Center).child(w::text(ui, "Inbox")).child(badge);
    ui.root().child(row);
});
let (Some(ui), Some(badge)) = (app.ui(), slot.get()) else { unreachable!("built") };
let theme = ui.theme();
let Some(style) = badge.style_in(States::NONE) else { unreachable!("styled") };
assert_eq!(style.background, theme.colors.accent);
assert_eq!(style.radius, theme.radii.medium);
```

Use plain numbers where a size belongs to the content rather than the design system (an
avatar's size, a column of a known width); `Dp::from(24.0)` and `24.0` both work.

## Variants

Widgets are named for what they are (`button`, `text_input`, `card`, `dialog`) and variants
for their role. A primary action is a `button` with the `primary` variant:

```
use remedy_ui::testing::TestApp;
use remedy_ui::style::States;
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let save = Rc::new(OnceCell::new());
let s = save.clone();
let mut app = TestApp::window(move |ui, _| {
    let button = w::button(ui, "Save", || {}).variant("primary");
    let _ = s.set(button.clone());
    ui.root().child(w::row(ui).gap(8.0).child(w::button(ui, "Cancel", || {})).child(button));
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
let Some(style) = save.get().and_then(|b| b.style_in(States::NONE)) else { unreachable!("styled") };
assert_eq!(style.background, ui.theme().colors.accent, "primary buttons are filled with the accent");
```

[`El::style_in`](crate::El::style_in) gives the style an element would have in any states,
which is how tests check that focus, hover and selection are visible.

## Your own rules

An app adds its own widget names and rules on top of the widget sheet. The sheet is a
function of the theme, so it is rebuilt from the new tokens whenever the theme changes, and
rules written in tokens follow every palette. Later rules win over earlier ones, and rules
that need more states win over rules that need fewer.

```
# use remedy_ui::testing::TestApp;
use remedy_ui::core::Insets;
use remedy_ui::style::{States, StylePatch, StyleSheet, Theme};
use remedy_ui::{App, AppConfig, WindowSpec, themes};
use std::cell::OnceCell;
use std::rc::Rc;

/// The widget sheet plus the app's `tag` widget, in tokens only.
fn sheet(t: &Theme) -> StyleSheet {
    let mut s = themes::sheet(t);
    let d = t.derive();
    let tag = StylePatch::new()
        .background(t.colors.card)
        .foreground(d.muted)
        .radius(t.radii.small)
        .padding(Insets::symmetric(t.spacing.s(1), t.spacing.s(2)));
    s.rule("tag", None, States::NONE, tag);
    s.rule("tag", Some("urgent"), States::NONE, StylePatch::new().background(t.colors.danger));
    s.rule("tag", None, States::HOVER, StylePatch::new().background(d.hover));
    s
}

let theme = themes::ThemeChoice::default().resolve(None, false);
let config = AppConfig::new("Tags").theme(theme, sheet);
let tag = Rc::new(OnceCell::new());
let t = tag.clone();
let app = move |app: &App| {
    app.open(WindowSpec::new("Tags"), move |ui, _| {
        let urgent = ui.el("tag").variant("urgent").text("Overdue");
        let _ = t.set(urgent.clone());
        ui.root().child(ui.el("tag").text("Later")).child(urgent);
    });
};
# let mut test = TestApp::new(config, app);
# let Some(ui) = test.ui() else { unreachable!("the window is open") };
let Some(urgent) = tag.get() else { unreachable!("built") };
let danger = ui.theme().colors.danger;
assert_eq!(urgent.style_in(States::NONE).map(|s| s.background), Some(danger));
// A hover rule for the widget beats the variant rule, because it needs more states.
let hover = ui.theme().derive().hover;
assert_eq!(urgent.style_in(States::HOVER).map(|s| s.background), Some(hover));
```

The states a rule can ask for are the flags of [`States`](crate::style::States): `HOVER`,
`PRESSED`, `FOCUS`, `FOCUS_VISIBLE`, `DISABLED`, `SELECTED`, `CHECKED`, `INVALID`,
`DRAGGING` and `DROP_TARGET`. The view sets them; app code sets its own with
[`El::state`](crate::El::state).

## One element's own style

[`El::style`](crate::El::style) sets properties on one element, over whatever the sheet
says; given a closure, it follows signals. Use it for what belongs to
the content, not the look: a user's chosen note color, a heading made larger in one place.
Name values by their token, not as constants, so the element still fits when the theme
changes:

```
use remedy_ui::testing::TestApp;
use remedy_ui::reactive::Signal;
use remedy_ui::style::{ColorRole, States, Style};
use remedy_ui::widgets as w;
use std::cell::OnceCell;
use std::rc::Rc;

let warning = Signal::new(false);
let slot = Rc::new(OnceCell::new());
let s = slot.clone();
let mut app = TestApp::window(move |ui, _| {
    let status = w::text(ui, "Battery").style(move || {
        let color = if warning.get() { ColorRole::Warn } else { ColorRole::Text };
        let style = Style::new().foreground(color);
        if warning.get() { style.weight(600.0) } else { style }
    });
    let _ = s.set(status.clone());
    ui.root().child(status);
});
let Some(ui) = app.ui() else { unreachable!("the window is open") };
warning.set(true);
let Some(status) = slot.get() else { unreachable!("built") };
assert_eq!(status.style_in(States::NONE).map(|s| s.foreground), Some(ui.theme().colors.warn));
```

## Painting with tokens

A custom painter ([`El::paint`](crate::El::paint), [`widgets::canvas`](crate::widgets::canvas))
gets the theme in [`PaintCx::theme`](crate::PaintCx::theme) and the element's resolved style in
[`PaintCx::style`](crate::PaintCx::style). Draw with those, never with fixed colors: the
painter runs again when the theme changes.

## People's own themes

Themes load and save as W3C design-tokens files with
[`themes::from_tokens`](crate::themes::from_tokens) and
[`themes::to_tokens`](crate::themes::to_tokens), so people can make their own and apps can map
their skin formats onto the shared set. See [`crate::guide::customization`].
