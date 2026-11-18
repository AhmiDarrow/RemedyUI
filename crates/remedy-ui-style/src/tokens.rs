//! Typed tokens: `Space::S3`, `Radius::R2`, `TextRole::Body`, `ColorRole::Accent`.
//!
//! A token names a place in the design system rather than a number, so autocomplete shows
//! the design system and a typo fails to compile. Tokens resolve through the theme each time
//! the theme changes: an element given `Space::S3` gets three spacing units of whatever theme
//! it is shown in. Plain values still work wherever a token does ([`Dp`] and [`ColorRef`]
//! take both).
//!
//! The design's own spelling was `Color::Accent` and `TextStyle::Body`; those names are
//! already the color type (`remedy_ui_core::Color`) and the text engine's style
//! (`remedy_ui_text::TextStyle`), so the tokens are [`ColorRole`] and [`TextRole`].
//!
//! ```
//! use remedy_ui_style::*;
//!
//! let theme = Theme::dark_forest();
//! assert_eq!(Dp::from(Space::S3).resolve(&theme), theme.spacing.unit * 3.0);
//! let patch = Style::new().radius(Radius::R2).padding_x(Space::S4).background(ColorRole::Accent).resolve(&theme);
//! assert_eq!(patch.radius, Some(theme.radii.medium));
//! assert_eq!(patch.background, Some(theme.colors.accent));
//! ```

use remedy_ui_core::{Color, Insets};

use crate::resolve::StylePatch;
use crate::theme::Theme;

/// A step on the spacing scale: `Sn` is `n` spacing units (4 dp each by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[allow(missing_docs, reason = "each step is n units, as the type says")]
pub enum Space {
    S0,
    S1,
    S2,
    S3,
    S4,
    S5,
    S6,
    S7,
    S8,
}

impl Space {
    /// The number of spacing units.
    pub fn units(self) -> u8 {
        match self {
            Space::S0 => 0,
            Space::S1 => 1,
            Space::S2 => 2,
            Space::S3 => 3,
            Space::S4 => 4,
            Space::S5 => 5,
            Space::S6 => 6,
            Space::S7 => 7,
            Space::S8 => 8,
        }
    }

    /// Logical pixels under `theme`.
    pub fn px(self, theme: &Theme) -> f32 {
        theme.spacing.s(self.units())
    }
}

/// The radius a fully rounded corner resolves to. The renderers fit radii to the box as CSS
/// does, so this becomes half the shorter side: a pill or a circle.
pub const RADIUS_FULL: f32 = f32::MAX / 4.0;

/// A corner radius from the shape scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Radius {
    /// Square corners.
    R0,
    /// The theme's small radius (small controls).
    R1,
    /// The theme's medium radius (cards and dialogs).
    R2,
    /// The theme's large radius (sheets and large surfaces).
    R3,
    /// Fully rounded: half the shorter side (pills, round avatars).
    R4,
}

impl Radius {
    /// Logical pixels under `theme`.
    pub fn px(self, theme: &Theme) -> f32 {
        match self {
            Radius::R0 => 0.0,
            Radius::R1 => theme.radii.small,
            Radius::R2 => theme.radii.medium,
            Radius::R3 => theme.radii.large,
            Radius::R4 => RADIUS_FULL,
        }
    }
}

/// A place on the type scale: size and weight together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextRole {
    /// Captions and help text under controls.
    Caption,
    /// Everything people read.
    Body,
    /// Body text that stands out.
    BodyStrong,
    /// Section headings.
    Heading,
    /// Screen titles.
    Title,
    /// Large display text.
    Display,
}

impl TextRole {
    /// Text size in logical pixels under `theme` (from its derived type scale).
    pub fn size(self, theme: &Theme) -> f32 {
        let s = theme.derive().sizes;
        let i = match self {
            TextRole::Caption => 1,
            TextRole::Body | TextRole::BodyStrong => 2,
            TextRole::Heading => 3,
            TextRole::Title => 4,
            TextRole::Display => 5,
        };
        s.get(i).copied().unwrap_or(theme.type_scale.body)
    }

    /// Weight, 100 to 900.
    pub fn weight(self) -> f32 {
        match self {
            TextRole::Caption | TextRole::Body => 400.0,
            TextRole::BodyStrong | TextRole::Heading | TextRole::Title => 600.0,
            TextRole::Display => 700.0,
        }
    }
}

/// A color role of the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorRole {
    /// The window background.
    Bg,
    /// Panels and sidebars.
    Panel,
    /// Cards and raised surfaces.
    Card,
    /// Body text (contrast-balanced against the background).
    Text,
    /// Secondary text (contrast-balanced).
    Muted,
    /// The accent.
    Accent,
    /// Text on the accent (contrast-balanced).
    OnAccent,
    /// Errors and destructive actions.
    Danger,
    /// Warnings.
    Warn,
    /// Success.
    Ok,
    /// Lines and borders.
    Line,
    /// The focus ring.
    Focus,
    /// Selected rows and ranges.
    Selected,
    /// An app's own role from the theme's `custom` colors; transparent when the theme has
    /// none of that name.
    Custom(&'static str),
}

impl ColorRole {
    /// The color under `theme`.
    pub fn color(self, theme: &Theme) -> Color {
        let c = &theme.colors;
        match self {
            ColorRole::Bg => c.bg,
            ColorRole::Panel => c.panel,
            ColorRole::Card => c.card,
            ColorRole::Accent => c.accent,
            ColorRole::Danger => c.danger,
            ColorRole::Warn => c.warn,
            ColorRole::Ok => c.ok,
            ColorRole::Custom(name) => c.custom.get(name).copied().unwrap_or(Color::TRANSPARENT),
            ColorRole::Text => theme.derive().text,
            ColorRole::Muted => theme.derive().muted,
            ColorRole::OnAccent => theme.derive().on_accent,
            ColorRole::Line => theme.derive().line,
            ColorRole::Focus => theme.derive().focus,
            ColorRole::Selected => theme.derive().selected,
        }
    }
}

/// A length in logical pixels, given as a number or as a token the theme resolves.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Dp {
    /// A number of logical pixels.
    Px(f32),
    /// A spacing step.
    Space(Space),
    /// A corner radius.
    Radius(Radius),
    /// The theme's hairline and border width.
    Stroke,
}

impl Dp {
    /// Logical pixels under `theme`.
    pub fn resolve(self, theme: &Theme) -> f32 {
        match self {
            Dp::Px(v) => v,
            Dp::Space(s) => s.px(theme),
            Dp::Radius(r) => r.px(theme),
            Dp::Stroke => theme.stroke,
        }
    }

    /// True when it depends on the theme.
    pub fn is_token(self) -> bool {
        !matches!(self, Dp::Px(_))
    }
}

impl From<f32> for Dp {
    fn from(v: f32) -> Self {
        Dp::Px(v)
    }
}

impl From<Space> for Dp {
    fn from(s: Space) -> Self {
        Dp::Space(s)
    }
}

impl From<Radius> for Dp {
    fn from(r: Radius) -> Self {
        Dp::Radius(r)
    }
}

/// A color, given as a value or as a role the theme resolves.
// Exhaustive on purpose: a color is given as a value or as a role, and nothing else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorRef {
    /// This color.
    Value(Color),
    /// The theme's color for this role.
    Role(ColorRole),
}

impl ColorRef {
    /// The color under `theme`.
    pub fn resolve(self, theme: &Theme) -> Color {
        match self {
            ColorRef::Value(c) => c,
            ColorRef::Role(r) => r.color(theme),
        }
    }
}

impl From<Color> for ColorRef {
    fn from(c: Color) -> Self {
        ColorRef::Value(c)
    }
}

impl From<ColorRole> for ColorRef {
    fn from(r: ColorRole) -> Self {
        ColorRef::Role(r)
    }
}

/// Padding by side, each side a length or a token; a side left unset keeps what the theme
/// gives the widget.
// Exhaustive on purpose: the four sides of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    /// Top.
    pub top: Option<Dp>,
    /// End (right in left-to-right text).
    pub end: Option<Dp>,
    /// Bottom.
    pub bottom: Option<Dp>,
    /// Start (left in left-to-right text).
    pub start: Option<Dp>,
}

impl Pad {
    /// The same on every side.
    pub fn all(v: impl Into<Dp>) -> Self {
        let v = Some(v.into());
        Pad { top: v, end: v, bottom: v, start: v }
    }

    /// `x` at the start and end, `y` at the top and bottom.
    pub fn xy(x: impl Into<Dp>, y: impl Into<Dp>) -> Self {
        let (x, y) = (Some(x.into()), Some(y.into()));
        Pad { top: y, end: x, bottom: y, start: x }
    }

    /// True when every side is set.
    pub fn is_complete(&self) -> bool {
        self.top.is_some() && self.end.is_some() && self.bottom.is_some() && self.start.is_some()
    }

    /// `other`'s set sides over these.
    pub fn merge(&self, other: &Pad) -> Pad {
        Pad {
            top: other.top.or(self.top),
            end: other.end.or(self.end),
            bottom: other.bottom.or(self.bottom),
            start: other.start.or(self.start),
        }
    }

    /// Logical pixels under `theme`, with unset sides taken from `under`.
    pub fn resolve(&self, theme: &Theme, under: Insets) -> Insets {
        let side = |d: Option<Dp>, u: f32| d.map_or(u, |d| d.resolve(theme));
        Insets {
            top: side(self.top, under.top),
            end: side(self.end, under.end),
            bottom: side(self.bottom, under.bottom),
            start: side(self.start, under.start),
        }
    }
}

impl From<Insets> for Pad {
    fn from(i: Insets) -> Self {
        Pad {
            top: Some(Dp::Px(i.top)),
            end: Some(Dp::Px(i.end)),
            bottom: Some(Dp::Px(i.bottom)),
            start: Some(Dp::Px(i.start)),
        }
    }
}

impl From<Space> for Pad {
    fn from(s: Space) -> Self {
        Pad::all(s)
    }
}

impl From<Dp> for Pad {
    fn from(d: Dp) -> Self {
        Pad::all(d)
    }
}

impl From<f32> for Pad {
    fn from(v: f32) -> Self {
        Pad::all(v)
    }
}

/// A style patch in tokens: what [`StylePatch`] holds, with lengths and colors that may be
/// tokens. It resolves through the theme each time the theme changes.
///
/// ```
/// use remedy_ui_style::*;
/// let s = Style::new().radius(Radius::R3).padding_x(Space::S4).text(TextRole::Title);
/// let theme = Theme::dark_forest();
/// let p = s.resolve(&theme);
/// assert_eq!(p.text_size, Some(TextRole::Title.size(&theme)));
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Style {
    /// Background fill.
    pub background: Option<ColorRef>,
    /// Text and icon color.
    pub foreground: Option<ColorRef>,
    /// Border color.
    pub border: Option<ColorRef>,
    /// Border width.
    pub border_width: Option<Dp>,
    /// Corner radius.
    pub radius: Option<Dp>,
    /// Padding, by side.
    pub padding: Option<Pad>,
    /// Text size.
    pub text_size: Option<Dp>,
    /// Text size from the type scale (when `text_size` is not set).
    pub text_role: Option<TextRole>,
    /// Text weight.
    pub weight: Option<f32>,
    /// Opacity, 0 to 1.
    pub opacity: Option<f32>,
    /// Monospace text.
    pub mono: Option<bool>,
    /// Focus ring color.
    pub outline: Option<ColorRef>,
    /// Focus ring width.
    pub outline_width: Option<Dp>,
    /// Gap between the edge and the focus ring.
    pub outline_offset: Option<Dp>,
}

impl Style {
    /// An empty patch.
    pub fn new() -> Self {
        Style::default()
    }

    /// Background fill.
    pub fn background(mut self, c: impl Into<ColorRef>) -> Self {
        self.background = Some(c.into());
        self
    }

    /// Text and icon color.
    pub fn foreground(mut self, c: impl Into<ColorRef>) -> Self {
        self.foreground = Some(c.into());
        self
    }

    /// Border color.
    pub fn border(mut self, c: impl Into<ColorRef>) -> Self {
        self.border = Some(c.into());
        self
    }

    /// Border width.
    pub fn border_width(mut self, w: impl Into<Dp>) -> Self {
        self.border_width = Some(w.into());
        self
    }

    /// Corner radius.
    pub fn radius(mut self, r: impl Into<Dp>) -> Self {
        self.radius = Some(r.into());
        self
    }

    /// Padding on every side, or by side with a [`Pad`].
    pub fn padding(mut self, p: impl Into<Pad>) -> Self {
        let p = p.into();
        self.padding = Some(self.padding.unwrap_or_default().merge(&p));
        self
    }

    /// Padding at the start and end.
    pub fn padding_x(self, v: impl Into<Dp>) -> Self {
        let v = Some(v.into());
        self.padding(Pad { start: v, end: v, ..Pad::default() })
    }

    /// Padding at the top and bottom.
    pub fn padding_y(self, v: impl Into<Dp>) -> Self {
        let v = Some(v.into());
        self.padding(Pad { top: v, bottom: v, ..Pad::default() })
    }

    /// Size and weight from the type scale.
    pub fn text(mut self, role: TextRole) -> Self {
        self.text_role = Some(role);
        self.text_size = None;
        self.weight = Some(role.weight());
        self
    }

    /// Text size.
    pub fn text_size(mut self, s: impl Into<Dp>) -> Self {
        self.text_size = Some(s.into());
        self.text_role = None;
        self
    }

    /// Text weight, 100 to 900.
    pub fn weight(mut self, w: f32) -> Self {
        self.weight = Some(w);
        self
    }

    /// Opacity, 0 to 1.
    pub fn opacity(mut self, o: f32) -> Self {
        self.opacity = Some(o);
        self
    }

    /// Monospace text.
    pub fn mono(mut self, on: bool) -> Self {
        self.mono = Some(on);
        self
    }

    /// Focus ring color.
    pub fn outline(mut self, c: impl Into<ColorRef>) -> Self {
        self.outline = Some(c.into());
        self
    }

    /// Focus ring width.
    pub fn outline_width(mut self, w: impl Into<Dp>) -> Self {
        self.outline_width = Some(w.into());
        self
    }

    /// Gap between the edge and the focus ring.
    pub fn outline_offset(mut self, w: impl Into<Dp>) -> Self {
        self.outline_offset = Some(w.into());
        self
    }

    /// `other`'s set properties over these (padding merges side by side).
    pub fn merge(&self, other: &Style) -> Style {
        let (text_size, text_role) = if other.text_size.is_some() || other.text_role.is_some() {
            (other.text_size, other.text_role)
        } else {
            (self.text_size, self.text_role)
        };
        Style {
            background: other.background.or(self.background),
            foreground: other.foreground.or(self.foreground),
            border: other.border.or(self.border),
            border_width: other.border_width.or(self.border_width),
            radius: other.radius.or(self.radius),
            padding: match (self.padding, other.padding) {
                (Some(a), Some(b)) => Some(a.merge(&b)),
                (a, b) => b.or(a),
            },
            text_size,
            text_role,
            weight: other.weight.or(self.weight),
            opacity: other.opacity.or(self.opacity),
            mono: other.mono.or(self.mono),
            outline: other.outline.or(self.outline),
            outline_width: other.outline_width.or(self.outline_width),
            outline_offset: other.outline_offset.or(self.outline_offset),
        }
    }

    /// True when the padding sets only some sides: the rest come from the widget's own
    /// style, so resolving needs it ([`Style::resolve_over`]).
    pub fn has_partial_padding(&self) -> bool {
        self.padding.is_some_and(|p| !p.is_complete())
    }

    /// True when nothing in it depends on the theme.
    pub fn is_plain(&self) -> bool {
        let dp = |d: Option<Dp>| d.is_none_or(|d| !d.is_token());
        let c = |c: Option<ColorRef>| !matches!(c, Some(ColorRef::Role(_)));
        let pad = self.padding.is_none_or(|p| [p.top, p.end, p.bottom, p.start].into_iter().all(dp));
        c(self.background)
            && c(self.foreground)
            && c(self.border)
            && c(self.outline)
            && dp(self.border_width)
            && dp(self.radius)
            && dp(self.text_size)
            && dp(self.outline_width)
            && dp(self.outline_offset)
            && self.text_role.is_none()
            && pad
    }

    /// The patch under `theme`; padding sides left unset are 0.
    pub fn resolve(&self, theme: &Theme) -> StylePatch {
        self.resolve_over(theme, Insets::ZERO)
    }

    /// The patch under `theme`, padding sides left unset taken from `padding_under` (the
    /// padding the element has without this patch).
    pub fn resolve_over(&self, theme: &Theme, padding_under: Insets) -> StylePatch {
        let dp = |d: Option<Dp>| d.map(|d| d.resolve(theme));
        let c = |c: Option<ColorRef>| c.map(|c| c.resolve(theme));
        StylePatch {
            background: c(self.background),
            foreground: c(self.foreground),
            border: c(self.border),
            border_width: dp(self.border_width),
            radius: dp(self.radius),
            padding: self.padding.map(|p| p.resolve(theme, padding_under)),
            text_size: dp(self.text_size).or_else(|| self.text_role.map(|r| r.size(theme))),
            weight: self.weight,
            opacity: self.opacity,
            mono: self.mono,
            outline: c(self.outline),
            outline_width: dp(self.outline_width),
            outline_offset: dp(self.outline_offset),
        }
    }
}

impl From<StylePatch> for Style {
    fn from(p: StylePatch) -> Self {
        Style {
            background: p.background.map(ColorRef::Value),
            foreground: p.foreground.map(ColorRef::Value),
            border: p.border.map(ColorRef::Value),
            border_width: p.border_width.map(Dp::Px),
            radius: p.radius.map(Dp::Px),
            padding: p.padding.map(Pad::from),
            text_size: p.text_size.map(Dp::Px),
            text_role: None,
            weight: p.weight,
            opacity: p.opacity,
            mono: p.mono,
            outline: p.outline.map(ColorRef::Value),
            outline_width: p.outline_width.map(Dp::Px),
            outline_offset: p.outline_offset.map(Dp::Px),
        }
    }
}
