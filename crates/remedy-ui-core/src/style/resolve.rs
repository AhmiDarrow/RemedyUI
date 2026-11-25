//! Resolving an element's style: widget default, theme entry, element properties.

use std::collections::HashMap;

use crate::{
    Color, ColorMatrix, CornerRadii, Insets, LineHeight, ScrollbarColor, ScrollbarVisibility, ScrollbarWidth,
    TextAlign, TextTransform, Vec2,
};

use crate::style::states::States;

/// How a border is drawn (CSS `border-style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BorderStyle {
    /// One unbroken line.
    #[default]
    Solid,
    /// Dashes three times the border's width long, with gaps as long.
    Dashed,
    /// Dots as wide as the border, a dot's width apart (round from 3 pixels wide).
    Dotted,
}

/// A side of a box, for its own border ([`StylePatch::border_side`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Exhaustive on purpose: a box has four sides.
pub enum BorderEdge {
    /// The top.
    Top,
    /// The end: right in left-to-right layout, left in right-to-left.
    End,
    /// The bottom.
    Bottom,
    /// The start: left in left-to-right layout, right in right-to-left.
    Start,
}

impl BorderEdge {
    fn index(self) -> usize {
        match self {
            BorderEdge::Top => 0,
            BorderEdge::End => 1,
            BorderEdge::Bottom => 2,
            BorderEdge::Start => 3,
        }
    }
}

/// One side's border (CSS `border-top` and the like): width, color and line style.
///
/// ```
/// use remedy_ui_core::Color;
/// use remedy_ui_core::style::{BorderEdge, BorderSide, BorderStyle, StylePatch};
/// let rule = BorderSide::new(1.0, Color::rgb8(60, 60, 60)).style(BorderStyle::Dashed);
/// let p = StylePatch::default().border_side(BorderEdge::Bottom, rule);
/// assert_eq!(p.border_sides[2], Some(rule));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderSide {
    /// Width in logical pixels; 0 draws none.
    pub width: f32,
    /// Color.
    pub color: Color,
    /// Line style.
    pub style: BorderStyle,
}

impl BorderSide {
    /// A solid side `width` wide in `color`.
    pub fn new(width: f32, color: Color) -> Self {
        BorderSide { width, color, style: BorderStyle::Solid }
    }

    /// The side drawn dashed or dotted.
    pub fn style(mut self, style: BorderStyle) -> Self {
        self.style = style;
        self
    }
}

/// A soft shadow: what a box casts (CSS `box-shadow`) or, as a filter, what the shape of an
/// element's drawing casts (CSS `filter: drop-shadow()`). Lengths are logical pixels.
///
/// ```
/// use remedy_ui_core::Color;
/// use remedy_ui_core::style::Shadow;
/// let s = Shadow::new(0.0, 4.0, 12.0, Color::rgba8(0, 0, 0, 90)).spread(1.0);
/// assert!(s.is_visible());
/// assert!(!Shadow::NONE.is_visible());
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// How far it falls.
    pub offset: Vec2,
    /// The blur radius, as CSS gives it (twice the Gaussian's standard deviation).
    pub blur: f32,
    /// How much the box grows before it blurs (box shadows only).
    pub spread: f32,
    /// Its color; transparent draws nothing.
    pub color: Color,
}

impl Shadow {
    /// No shadow: what a rule sets to take one away.
    pub const NONE: Shadow = Shadow { offset: Vec2::ZERO, blur: 0.0, spread: 0.0, color: Color::TRANSPARENT };

    /// A shadow moved by (`x`, `y`), blurred by `blur`, in `color`.
    pub fn new(x: f32, y: f32, blur: f32, color: Color) -> Shadow {
        Shadow { offset: Vec2::new(x, y), blur, spread: 0.0, color }
    }

    /// The same shadow grown by `spread` before blurring.
    pub fn spread(mut self, spread: f32) -> Shadow {
        self.spread = spread;
        self
    }

    /// True when it draws anything.
    pub fn is_visible(&self) -> bool {
        self.color.a > 0.0
    }
}

/// Visual properties a style sets. Every field is optional so a patch changes only what it
/// names.
///
/// Build one from [`StylePatch::new`] with a method per property; properties are added over
/// time, so a patch is never written as a struct literal outside this crate:
///
/// ```
/// use remedy_ui_core::Color;
/// use remedy_ui_core::style::StylePatch;
///
/// let patch = StylePatch::new().background(Color::rgb8(30, 60, 90)).radius(6.0).text_size(15.0);
/// assert_eq!(patch.radius, Some(6.0));
/// assert_eq!(patch.foreground, None);
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct StylePatch {
    /// Background fill.
    pub background: Option<Color>,
    /// Gradient layers over the background fill, the first on top (CSS `background-image`);
    /// an empty list for none.
    pub background_layers: Option<Vec<crate::style::Gradient>>,
    /// Text and icon color.
    pub foreground: Option<Color>,
    /// Border color.
    pub border: Option<Color>,
    /// Border width.
    pub border_width: Option<f32>,
    /// Border line style: solid, dashed or dotted.
    pub border_style: Option<BorderStyle>,
    /// Each side's own border, top, end, bottom and start: a side set here takes the place
    /// of `border`, `border_width` and `border_style` on that side.
    pub border_sides: [Option<BorderSide>; 4],
    /// Corner radius.
    pub radius: Option<f32>,
    /// Each corner's radius (CSS's `border-radius: 16px 16px 0 0`): set, it takes the place
    /// of `radius`; a `radius` set over it (a later patch's) takes its place again.
    pub corners: Option<CornerRadii>,
    /// Padding.
    pub padding: Option<Insets>,
    /// Text size.
    pub text_size: Option<f32>,
    /// Text weight.
    pub weight: Option<f32>,
    /// Opacity, 0 to 1.
    pub opacity: Option<f32>,
    /// Text in the theme's monospace families (codes, keys, numbers that must line up).
    pub mono: Option<bool>,
    /// Text in the theme's heading families (titles and headings); monospace wins.
    pub heading: Option<bool>,
    /// Extra space after each letter (CSS `letter-spacing`).
    pub letter_spacing: Option<LetterSpacing>,
    /// How letters are cased when drawn (CSS `text-transform`); inherited.
    pub text_transform: Option<TextTransform>,
    /// How tall each line of text is (CSS `line-height`); inherited.
    pub line_height: Option<LineHeight>,
    /// At most this many lines of text, the last ending in "…" when cut (CSS
    /// `line-clamp`); 0 for no limit. Not inherited.
    pub max_lines: Option<u32>,
    /// Where each line of text sits across its box (CSS `text-align`); inherited.
    pub text_align: Option<TextAlign>,
    /// The focus ring's color, drawn while the element has keyboard focus.
    pub outline: Option<Color>,
    /// The focus ring's width; 0 for elements that show focus another way.
    pub outline_width: Option<f32>,
    /// The gap between the element's edge and its focus ring; negative draws it inside.
    pub outline_offset: Option<f32>,
    /// The shadow the element's box casts (CSS `box-shadow`); [`Shadow::NONE`] for none.
    pub shadow: Option<Shadow>,
    /// Blurs the element and what it draws inside it (CSS `filter: blur()`): the Gaussian's
    /// standard deviation in logical pixels; 0 for none.
    pub blur: Option<f32>,
    /// A shadow of the shape of everything the element draws, text and icons included (CSS
    /// `filter: drop-shadow()`); [`Shadow::NONE`] for none. Its spread is not used.
    pub drop_shadow: Option<Shadow>,
    /// Frosted glass: blurs what the window shows behind the element's box (CSS
    /// `backdrop-filter: blur()`), the standard deviation in logical pixels; 0 for none. The
    /// element gets a compositor layer of its own.
    pub backdrop_blur: Option<f32>,
    /// A colour matrix on everything the element draws (CSS `saturate()`, `hue-rotate()`,
    /// `grayscale()`, ...); [`ColorMatrix::IDENTITY`] for none.
    pub color_matrix: Option<ColorMatrix>,
    /// Sticks the element within its nearest scroll container as that scrolls (CSS
    /// `position: sticky` with its insets); [`StickyInsets::NONE`] for not sticky.
    pub sticky: Option<StickyInsets>,
    /// How thick a scroll container's scroll bars are (CSS `scrollbar-width`).
    pub scrollbar_width: Option<ScrollbarWidth>,
    /// The colors of its scroll bars (CSS `scrollbar-color`), inherited; `None` all the way
    /// up for the platform's own.
    pub scrollbar_color: Option<ScrollbarColor>,
    /// When its scroll bars show: overlaid while scrolling, always, or never.
    pub scrollbar_visibility: Option<ScrollbarVisibility>,
}

/// The edges a sticky element keeps to in its nearest scroll container's viewport (CSS
/// `position: sticky` and `top`, `bottom`, `inset-inline-start`, `inset-inline-end`): each set
/// side is how close it keeps to that edge, and a side left `None` does not stick. Start and
/// end follow the parent's text direction. It is laid out in the flow, moved by the
/// compositor as the container scrolls, and never leaves its parent's content box.
///
/// ```
/// use remedy_ui_core::style::StickyInsets;
/// let header = StickyInsets::top(0.0);
/// assert_eq!((header.top, header.bottom), (Some(0.0), None));
/// assert!(!StickyInsets::NONE.sticks());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StickyInsets {
    /// Distance kept from the top edge.
    pub top: Option<f32>,
    /// Distance kept from the inline end edge (right in left-to-right text).
    pub end: Option<f32>,
    /// Distance kept from the bottom edge.
    pub bottom: Option<f32>,
    /// Distance kept from the inline start edge (left in left-to-right text).
    pub start: Option<f32>,
}

impl StickyInsets {
    /// Not sticky.
    pub const NONE: StickyInsets = StickyInsets { top: None, end: None, bottom: None, start: None };

    /// Sticks `v` below the top edge (a header).
    pub const fn top(v: f32) -> Self {
        StickyInsets { top: Some(v), ..Self::NONE }
    }

    /// Sticks `v` above the bottom edge (a footer, an action bar).
    pub const fn bottom(v: f32) -> Self {
        StickyInsets { bottom: Some(v), ..Self::NONE }
    }

    /// Sticks `v` from the inline start edge (a first column).
    pub const fn start(v: f32) -> Self {
        StickyInsets { start: Some(v), ..Self::NONE }
    }

    /// Sticks `v` from the inline end edge.
    pub const fn end(v: f32) -> Self {
        StickyInsets { end: Some(v), ..Self::NONE }
    }

    /// True when some side sticks (a finite distance).
    pub fn sticks(&self) -> bool {
        [self.top, self.end, self.bottom, self.start].iter().any(|v| v.is_some_and(f32::is_finite))
    }
}

impl StylePatch {
    /// A patch that sets nothing.
    pub fn new() -> Self {
        StylePatch::default()
    }

    /// Sets the background fill.
    pub fn background(mut self, value: Color) -> Self {
        self.background = Some(value);
        self
    }

    /// Sets the gradient layers drawn over the background fill, the first on top.
    pub fn background_layers(mut self, layers: impl IntoIterator<Item = crate::style::Gradient>) -> Self {
        self.background_layers = Some(layers.into_iter().collect());
        self
    }

    /// Sets the text and icon color.
    pub fn foreground(mut self, value: Color) -> Self {
        self.foreground = Some(value);
        self
    }

    /// Sets the border color.
    pub fn border(mut self, value: Color) -> Self {
        self.border = Some(value);
        self
    }

    /// Sets the border width.
    pub fn border_width(mut self, value: f32) -> Self {
        self.border_width = Some(value);
        self
    }

    /// Sets the border's line style (CSS `border-style`).
    pub fn border_style(mut self, value: BorderStyle) -> Self {
        self.border_style = Some(value);
        self
    }

    /// Sets one side's border (CSS `border-top`, `border-inline-end` and the like), in place
    /// of the box's border on that side.
    pub fn border_side(mut self, edge: BorderEdge, side: BorderSide) -> Self {
        if let Some(slot) = self.border_sides.get_mut(edge.index()) {
            *slot = Some(side);
        }
        self
    }

    /// Sets the corner radius: every corner's.
    pub fn radius(mut self, value: f32) -> Self {
        self.radius = Some(value);
        self.corners = None;
        self
    }

    /// Sets each corner's radius, as CSS's four-value `border-radius`: a surface round at
    /// the top only is `CornerRadii { top_start: 16.0, top_end: 16.0, ..CornerRadii::ZERO }`.
    /// `radius` becomes the largest of them.
    pub fn corners(mut self, value: CornerRadii) -> Self {
        self.radius = Some(value.top_start.max(value.top_end).max(value.bottom_end).max(value.bottom_start));
        self.corners = Some(value);
        self
    }

    /// Sets the padding.
    pub fn padding(mut self, value: Insets) -> Self {
        self.padding = Some(value);
        self
    }

    /// Sets the text size.
    pub fn text_size(mut self, value: f32) -> Self {
        self.text_size = Some(value);
        self
    }

    /// Sets the text weight.
    pub fn weight(mut self, value: f32) -> Self {
        self.weight = Some(value);
        self
    }

    /// Sets the opacity, 0 to 1.
    pub fn opacity(mut self, value: f32) -> Self {
        self.opacity = Some(value);
        self
    }

    /// Sets whether text uses the theme's heading families.
    pub fn heading(mut self, value: bool) -> Self {
        self.heading = Some(value);
        self
    }

    /// Sets the extra space after each letter.
    pub fn letter_spacing(mut self, value: LetterSpacing) -> Self {
        self.letter_spacing = Some(value);
        self
    }

    /// Sets how letters are cased when drawn.
    pub fn text_transform(mut self, value: TextTransform) -> Self {
        self.text_transform = Some(value);
        self
    }

    /// Sets how tall each line of text is.
    pub fn line_height(mut self, value: LineHeight) -> Self {
        self.line_height = Some(value);
        self
    }

    /// Sets the most lines text takes (0: no limit).
    pub fn max_lines(mut self, value: u32) -> Self {
        self.max_lines = Some(value);
        self
    }

    /// Sets where each line of text sits across its box (CSS `text-align`).
    pub fn text_align(mut self, value: TextAlign) -> Self {
        self.text_align = Some(value);
        self
    }

    /// Sets whether text uses the theme's monospace families.
    pub fn mono(mut self, value: bool) -> Self {
        self.mono = Some(value);
        self
    }

    /// Sets the focus ring's color.
    pub fn outline(mut self, value: Color) -> Self {
        self.outline = Some(value);
        self
    }

    /// Sets the focus ring's width.
    pub fn outline_width(mut self, value: f32) -> Self {
        self.outline_width = Some(value);
        self
    }

    /// Sets the gap between the element's edge and its focus ring.
    pub fn outline_offset(mut self, value: f32) -> Self {
        self.outline_offset = Some(value);
        self
    }

    /// Sets the shadow the box casts.
    pub fn shadow(mut self, value: Shadow) -> Self {
        self.shadow = Some(value);
        self
    }

    /// Sets the blur of the element and its content.
    pub fn blur(mut self, value: f32) -> Self {
        self.blur = Some(value);
        self
    }

    /// Sets the shadow of the shape of what the element draws.
    pub fn drop_shadow(mut self, value: Shadow) -> Self {
        self.drop_shadow = Some(value);
        self
    }

    /// Sets the blur of what lies behind the element (frosted glass).
    pub fn backdrop_blur(mut self, value: f32) -> Self {
        self.backdrop_blur = Some(value);
        self
    }

    /// Sets the colour matrix on everything the element draws.
    pub fn color_matrix(mut self, value: ColorMatrix) -> Self {
        self.color_matrix = Some(value);
        self
    }

    /// Sticks the element within its nearest scroll container ([`StickyInsets`]).
    pub fn sticky(mut self, value: StickyInsets) -> Self {
        self.sticky = Some(value);
        self
    }

    /// Sets how thick a scroll container's scroll bars are.
    pub fn scrollbar_width(mut self, value: ScrollbarWidth) -> Self {
        self.scrollbar_width = Some(value);
        self
    }

    /// Sets the colors of a scroll container's scroll bars (inherited).
    pub fn scrollbar_color(mut self, value: ScrollbarColor) -> Self {
        self.scrollbar_color = Some(value);
        self
    }

    /// Sets when a scroll container's scroll bars show.
    pub fn scrollbar_visibility(mut self, value: ScrollbarVisibility) -> Self {
        self.scrollbar_visibility = Some(value);
        self
    }

    /// The engine's starting style under `theme`, before any rule: transparent boxes, body
    /// text in the theme's text color, and the theme's focus ring for keyboard focus (as
    /// `:focus-visible` does on the web) unless a rule shows focus another way.
    pub fn defaults(theme: &crate::style::Theme) -> StylePatch {
        StylePatch {
            background: Some(Color::TRANSPARENT),
            background_layers: Some(Vec::new()),
            foreground: Some(theme.derive().text),
            border: Some(Color::TRANSPARENT),
            border_width: Some(0.0),
            border_style: Some(BorderStyle::Solid),
            border_sides: [None; 4],
            radius: Some(0.0),
            corners: None,
            padding: Some(Insets::ZERO),
            text_size: Some(theme.type_scale.body),
            weight: Some(400.0),
            opacity: Some(1.0),
            mono: Some(false),
            heading: Some(false),
            letter_spacing: Some(LetterSpacing::Px(0.0)),
            text_transform: Some(TextTransform::None),
            line_height: Some(LineHeight::Relative(1.33)),
            max_lines: Some(0),
            text_align: Some(TextAlign::Start),
            outline: Some(theme.derive().focus),
            outline_width: Some(2.0),
            outline_offset: Some(2.0),
            shadow: Some(Shadow::NONE),
            blur: Some(0.0),
            drop_shadow: Some(Shadow::NONE),
            backdrop_blur: Some(0.0),
            color_matrix: Some(ColorMatrix::IDENTITY),
            sticky: Some(StickyInsets::NONE),
            scrollbar_width: Some(ScrollbarWidth::Auto),
            // The platform's colors, from the theme, unless a rule says otherwise.
            scrollbar_color: None,
            scrollbar_visibility: Some(ScrollbarVisibility::Auto),
        }
    }

    /// `other`'s set fields over this patch's.
    pub fn merge(&self, other: &StylePatch) -> StylePatch {
        StylePatch {
            background: other.background.or(self.background),
            background_layers: other.background_layers.clone().or_else(|| self.background_layers.clone()),
            foreground: other.foreground.or(self.foreground),
            border: other.border.or(self.border),
            border_width: other.border_width.or(self.border_width),
            border_style: other.border_style.or(self.border_style),
            border_sides: std::array::from_fn(|i| {
                other.border_sides.get(i).copied().flatten().or(self.border_sides.get(i).copied().flatten())
            }),
            radius: other.radius.or(self.radius),
            // A radius alone over corners rounds every corner alike again.
            corners: match (other.corners, other.radius) {
                (Some(c), _) => Some(c),
                (None, Some(_)) => None,
                (None, None) => self.corners,
            },
            padding: other.padding.or(self.padding),
            text_size: other.text_size.or(self.text_size),
            weight: other.weight.or(self.weight),
            opacity: other.opacity.or(self.opacity),
            mono: other.mono.or(self.mono),
            heading: other.heading.or(self.heading),
            letter_spacing: other.letter_spacing.or(self.letter_spacing),
            text_transform: other.text_transform.or(self.text_transform),
            line_height: other.line_height.or(self.line_height),
            max_lines: other.max_lines.or(self.max_lines),
            text_align: other.text_align.or(self.text_align),
            outline: other.outline.or(self.outline),
            outline_width: other.outline_width.or(self.outline_width),
            outline_offset: other.outline_offset.or(self.outline_offset),
            shadow: other.shadow.or(self.shadow),
            blur: other.blur.or(self.blur),
            drop_shadow: other.drop_shadow.or(self.drop_shadow),
            backdrop_blur: other.backdrop_blur.or(self.backdrop_blur),
            color_matrix: other.color_matrix.or(self.color_matrix),
            sticky: other.sticky.or(self.sticky),
            scrollbar_width: other.scrollbar_width.or(self.scrollbar_width),
            scrollbar_color: other.scrollbar_color.or(self.scrollbar_color),
            scrollbar_visibility: other.scrollbar_visibility.or(self.scrollbar_visibility),
        }
    }
}

/// Extra space after each letter, as CSS's `letter-spacing`: in logical pixels, or in ems
/// (of the text's own size, so it scales with it). Text layout adds it after every cluster
/// (a letter with its marks, a ligature), never inside one, in either direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LetterSpacing {
    /// Logical pixels.
    Px(f32),
    /// Ems: times the text size.
    Em(f32),
}

impl LetterSpacing {
    /// In logical pixels, for text `size` pixels high.
    ///
    /// ```
    /// use remedy_ui_core::style::LetterSpacing;
    ///
    /// assert_eq!(LetterSpacing::Em(0.08).px(12.0), 0.96);
    /// assert_eq!(LetterSpacing::Px(2.0).px(30.0), 2.0);
    /// ```
    pub fn px(self, size: f32) -> f32 {
        match self {
            LetterSpacing::Px(v) => v,
            LetterSpacing::Em(v) => v * size,
        }
    }
}

/// A complete style, every property decided.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle {
    /// Background fill.
    pub background: Color,
    /// Gradient layers over the background fill, the first on top.
    pub background_layers: Vec<crate::style::Gradient>,
    /// Text and icon color.
    pub foreground: Color,
    /// Border color.
    pub border: Color,
    /// Border width.
    pub border_width: f32,
    /// Border line style.
    pub border_style: BorderStyle,
    /// Sides with borders of their own, top, end, bottom and start
    /// ([`border_on`](Self::border_on) gives each side's border either way).
    pub border_sides: [Option<BorderSide>; 4],
    /// Corner radius (the largest corner's when they differ).
    pub radius: f32,
    /// Each corner's radius: `radius` at every corner unless the style set them one by one.
    pub corners: CornerRadii,
    /// Padding.
    pub padding: Insets,
    /// Text size.
    pub text_size: f32,
    /// Text weight.
    pub weight: f32,
    /// Opacity.
    pub opacity: f32,
    /// Monospace text.
    pub mono: bool,
    /// Text in the theme's heading families.
    pub heading: bool,
    /// Extra space after each letter.
    pub letter_spacing: LetterSpacing,
    /// How letters are cased when drawn.
    pub text_transform: TextTransform,
    /// How tall each line of text is.
    pub line_height: LineHeight,
    /// The most lines text takes (0: no limit).
    pub max_lines: u32,
    /// Where each line of text sits across its box.
    pub text_align: TextAlign,
    /// The focus ring's color.
    pub outline: Color,
    /// The focus ring's width (0: none).
    pub outline_width: f32,
    /// The focus ring's gap from the edge (negative: inside).
    pub outline_offset: f32,
    /// The shadow the box casts ([`Shadow::NONE`]: none).
    pub shadow: Shadow,
    /// The blur of the element and its content, a standard deviation (0: none).
    pub blur: f32,
    /// The shadow of the shape of what the element draws ([`Shadow::NONE`]: none).
    pub drop_shadow: Shadow,
    /// The blur of what lies behind the element, a standard deviation (0: none).
    pub backdrop_blur: f32,
    /// The colour matrix on everything the element draws ([`ColorMatrix::IDENTITY`]: none).
    pub color_matrix: ColorMatrix,
    /// The edges it sticks to ([`StickyInsets::NONE`]: not sticky).
    pub sticky: StickyInsets,
    /// How thick its scroll bars are, when it scrolls.
    pub scrollbar_width: ScrollbarWidth,
    /// The colors of its scroll bars (`None`: the platform's, from the theme).
    pub scrollbar_color: Option<ScrollbarColor>,
    /// When its scroll bars show.
    pub scrollbar_visibility: ScrollbarVisibility,
}

impl ResolvedStyle {
    fn from_patch(p: &StylePatch) -> ResolvedStyle {
        ResolvedStyle {
            background: p.background.unwrap_or(Color::TRANSPARENT),
            background_layers: p.background_layers.clone().unwrap_or_default(),
            foreground: p.foreground.unwrap_or(Color::BLACK),
            border: p.border.unwrap_or(Color::TRANSPARENT),
            border_width: p.border_width.unwrap_or(0.0),
            border_style: p.border_style.unwrap_or_default(),
            border_sides: p.border_sides,
            radius: p.radius.unwrap_or(0.0),
            corners: p.corners.unwrap_or(CornerRadii::all(p.radius.unwrap_or(0.0))),
            padding: p.padding.unwrap_or(Insets::ZERO),
            text_size: p.text_size.unwrap_or(14.0),
            weight: p.weight.unwrap_or(400.0),
            opacity: p.opacity.unwrap_or(1.0),
            mono: p.mono.unwrap_or(false),
            heading: p.heading.unwrap_or(false),
            letter_spacing: p.letter_spacing.unwrap_or(LetterSpacing::Px(0.0)),
            text_transform: p.text_transform.unwrap_or_default(),
            line_height: p.line_height.unwrap_or(LineHeight::Relative(1.33)),
            max_lines: p.max_lines.unwrap_or(0),
            text_align: p.text_align.unwrap_or_default(),
            outline: p.outline.unwrap_or(Color::TRANSPARENT),
            outline_width: p.outline_width.unwrap_or(0.0),
            outline_offset: p.outline_offset.unwrap_or(0.0),
            shadow: p.shadow.unwrap_or(Shadow::NONE),
            blur: p.blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0),
            drop_shadow: p.drop_shadow.unwrap_or(Shadow::NONE),
            backdrop_blur: p.backdrop_blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0),
            color_matrix: p.color_matrix.map_or(ColorMatrix::IDENTITY, |m| m.limited()),
            sticky: p.sticky.unwrap_or(StickyInsets::NONE),
            scrollbar_width: p.scrollbar_width.unwrap_or_default(),
            scrollbar_color: p.scrollbar_color,
            scrollbar_visibility: p.scrollbar_visibility.unwrap_or_default(),
        }
    }

    /// True when keyboard focus on an element styled `self` draws a ring.
    pub fn shows_focus_ring(&self) -> bool {
        self.outline_width > 0.0 && self.outline.a > 0.0
    }
}

/// What a resolved style is cached by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StyleKey {
    /// The widget kind, such as `"button"`.
    pub widget: &'static str,
    /// The variant, such as `"primary"`.
    pub variant: &'static str,
    /// The element's states.
    pub states: States,
}

#[derive(Clone, Debug)]
struct Rule {
    widget: &'static str,
    variant: Option<&'static str>,
    states: States,
    patch: StylePatch,
}

/// The theme's style rules for widgets, and a cache of resolved styles. Changing the sheet or
/// the theme bumps its version and empties the cache.
#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    rules: Vec<Rule>,
    version: u64,
    cache: HashMap<StyleKey, StylePatch>,
    hits: u64,
    misses: u64,
}

impl ResolvedStyle {
    /// The border on `edge`: the side's own, else the box's.
    pub fn border_on(&self, edge: BorderEdge) -> BorderSide {
        self.border_sides.get(edge.index()).copied().flatten().unwrap_or(BorderSide {
            width: self.border_width,
            color: self.border,
            style: self.border_style,
        })
    }

    /// True when a side has a border of its own.
    pub fn has_border_sides(&self) -> bool {
        self.border_sides.iter().any(Option::is_some)
    }

    /// How far the border reaches in on each side (layout and content sit inside it).
    pub fn border_insets(&self) -> Insets {
        let w = |e| self.border_on(e).width.max(0.0);
        Insets {
            top: w(BorderEdge::Top),
            end: w(BorderEdge::End),
            bottom: w(BorderEdge::Bottom),
            start: w(BorderEdge::Start),
        }
    }

    /// Padding and border together: where the content starts on each side.
    pub fn inner_insets(&self) -> Insets {
        let b = self.border_insets();
        let p = self.padding;
        Insets {
            top: p.top + b.top,
            end: p.end + b.end,
            bottom: p.bottom + b.bottom,
            start: p.start + b.start,
        }
    }

    /// The defaults for a child of an element styled `self`: text properties (color, size,
    /// weight, monospace) are inherited, as in CSS; box properties (background, border,
    /// radius, padding, opacity, focus ring) start fresh from `base`.
    pub fn inherited_by_children(&self, base: &StylePatch) -> StylePatch {
        StylePatch {
            foreground: Some(self.foreground),
            text_size: Some(self.text_size),
            weight: Some(self.weight),
            mono: Some(self.mono),
            heading: Some(self.heading),
            letter_spacing: Some(self.letter_spacing),
            text_transform: Some(self.text_transform),
            line_height: Some(self.line_height),
            text_align: Some(self.text_align),
            // As CSS `scrollbar-color`, the bars' colors are inherited.
            scrollbar_color: self.scrollbar_color.or(base.scrollbar_color),
            ..base.clone()
        }
    }
}

impl StyleSheet {
    /// An empty sheet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a rule: `patch` applies to `widget` (and only its `variant` if given) when the
    /// element has at least `states`. Later rules win over earlier ones, and rules that need
    /// more states win over rules that need fewer.
    pub fn rule(
        &mut self,
        widget: &'static str,
        variant: Option<&'static str>,
        states: States,
        patch: StylePatch,
    ) {
        self.rules.push(Rule { widget, variant, states, patch });
        self.invalidate();
    }

    /// Empties the cache (call after the theme changes).
    pub fn invalidate(&mut self) {
        self.version += 1;
        self.cache.clear();
    }

    /// The sheet's version; it changes whenever rules or the theme change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The style for `key`, starting from `defaults` (what the element inherits and the
    /// engine's defaults), then the sheet's rules for the key, then `local` (properties set on
    /// the element). The rules' combined patch is cached per key, so elements with different
    /// inherited values share it.
    pub fn resolve(&mut self, key: &StyleKey, defaults: &StylePatch, local: &StylePatch) -> ResolvedStyle {
        let rules = match self.cache.get(key) {
            Some(p) => {
                self.hits += 1;
                p.clone()
            }
            None => {
                self.misses += 1;
                let mut matching: Vec<&Rule> = self
                    .rules
                    .iter()
                    .filter(|r| r.widget == key.widget)
                    .filter(|r| r.variant.is_none_or(|v| v == key.variant))
                    .filter(|r| key.states.contains(r.states))
                    .collect();
                // Fewer required states first, so more specific rules apply last; stable sort
                // keeps declaration order among equals.
                matching.sort_by_key(|r| r.states.bits().count_ones() + u32::from(r.variant.is_some()));
                let patch = matching.iter().fold(StylePatch::default(), |acc, r| acc.merge(&r.patch));
                self.cache.insert(key.clone(), patch.clone());
                patch
            }
        };
        ResolvedStyle::from_patch(&defaults.merge(&rules).merge(local))
    }

    /// (cache hits, cache misses), for tests and the performance overlay.
    pub fn cache_stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}
