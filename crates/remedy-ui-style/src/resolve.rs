//! Resolving an element's style: widget default, theme entry, element properties.

use std::collections::HashMap;

use remedy_ui_core::{Color, ColorMatrix, Insets, TextTransform, Vec2};

use crate::states::States;

/// A soft shadow: what a box casts (CSS `box-shadow`) or, as a filter, what the shape of an
/// element's drawing casts (CSS `filter: drop-shadow()`). Lengths are logical pixels.
///
/// ```
/// use remedy_ui_core::Color;
/// use remedy_ui_style::Shadow;
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
/// use remedy_ui_style::StylePatch;
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
    /// Text and icon color.
    pub foreground: Option<Color>,
    /// Border color.
    pub border: Option<Color>,
    /// Border width.
    pub border_width: Option<f32>,
    /// Corner radius.
    pub radius: Option<f32>,
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
    /// At most this many lines of text, the last ending in "…" when cut (CSS
    /// `line-clamp`); 0 for no limit. Not inherited.
    pub max_lines: Option<u32>,
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

    /// Sets the corner radius.
    pub fn radius(mut self, value: f32) -> Self {
        self.radius = Some(value);
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

    /// Sets the most lines text takes (0: no limit).
    pub fn max_lines(mut self, value: u32) -> Self {
        self.max_lines = Some(value);
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

    /// The engine's starting style under `theme`, before any rule: transparent boxes, body
    /// text in the theme's text color, and the theme's focus ring for keyboard focus (as
    /// `:focus-visible` does on the web) unless a rule shows focus another way.
    pub fn defaults(theme: &crate::Theme) -> StylePatch {
        StylePatch {
            background: Some(Color::TRANSPARENT),
            foreground: Some(theme.derive().text),
            border: Some(Color::TRANSPARENT),
            border_width: Some(0.0),
            radius: Some(0.0),
            padding: Some(Insets::ZERO),
            text_size: Some(theme.type_scale.body),
            weight: Some(400.0),
            opacity: Some(1.0),
            mono: Some(false),
            heading: Some(false),
            letter_spacing: Some(LetterSpacing::Px(0.0)),
            text_transform: Some(TextTransform::None),
            max_lines: Some(0),
            outline: Some(theme.derive().focus),
            outline_width: Some(2.0),
            outline_offset: Some(2.0),
            shadow: Some(Shadow::NONE),
            blur: Some(0.0),
            drop_shadow: Some(Shadow::NONE),
            backdrop_blur: Some(0.0),
            color_matrix: Some(ColorMatrix::IDENTITY),
        }
    }

    /// `other`'s set fields over this patch's.
    pub fn merge(&self, other: &StylePatch) -> StylePatch {
        StylePatch {
            background: other.background.or(self.background),
            foreground: other.foreground.or(self.foreground),
            border: other.border.or(self.border),
            border_width: other.border_width.or(self.border_width),
            radius: other.radius.or(self.radius),
            padding: other.padding.or(self.padding),
            text_size: other.text_size.or(self.text_size),
            weight: other.weight.or(self.weight),
            opacity: other.opacity.or(self.opacity),
            mono: other.mono.or(self.mono),
            heading: other.heading.or(self.heading),
            letter_spacing: other.letter_spacing.or(self.letter_spacing),
            text_transform: other.text_transform.or(self.text_transform),
            max_lines: other.max_lines.or(self.max_lines),
            outline: other.outline.or(self.outline),
            outline_width: other.outline_width.or(self.outline_width),
            outline_offset: other.outline_offset.or(self.outline_offset),
            shadow: other.shadow.or(self.shadow),
            blur: other.blur.or(self.blur),
            drop_shadow: other.drop_shadow.or(self.drop_shadow),
            backdrop_blur: other.backdrop_blur.or(self.backdrop_blur),
            color_matrix: other.color_matrix.or(self.color_matrix),
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
    /// use remedy_ui_style::LetterSpacing;
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
    /// Text and icon color.
    pub foreground: Color,
    /// Border color.
    pub border: Color,
    /// Border width.
    pub border_width: f32,
    /// Corner radius.
    pub radius: f32,
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
    /// The most lines text takes (0: no limit).
    pub max_lines: u32,
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
}

impl ResolvedStyle {
    fn from_patch(p: &StylePatch) -> ResolvedStyle {
        ResolvedStyle {
            background: p.background.unwrap_or(Color::TRANSPARENT),
            foreground: p.foreground.unwrap_or(Color::BLACK),
            border: p.border.unwrap_or(Color::TRANSPARENT),
            border_width: p.border_width.unwrap_or(0.0),
            radius: p.radius.unwrap_or(0.0),
            padding: p.padding.unwrap_or(Insets::ZERO),
            text_size: p.text_size.unwrap_or(14.0),
            weight: p.weight.unwrap_or(400.0),
            opacity: p.opacity.unwrap_or(1.0),
            mono: p.mono.unwrap_or(false),
            heading: p.heading.unwrap_or(false),
            letter_spacing: p.letter_spacing.unwrap_or(LetterSpacing::Px(0.0)),
            text_transform: p.text_transform.unwrap_or_default(),
            max_lines: p.max_lines.unwrap_or(0),
            outline: p.outline.unwrap_or(Color::TRANSPARENT),
            outline_width: p.outline_width.unwrap_or(0.0),
            outline_offset: p.outline_offset.unwrap_or(0.0),
            shadow: p.shadow.unwrap_or(Shadow::NONE),
            blur: p.blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0),
            drop_shadow: p.drop_shadow.unwrap_or(Shadow::NONE),
            backdrop_blur: p.backdrop_blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0),
            color_matrix: p.color_matrix.map_or(ColorMatrix::IDENTITY, |m| m.limited()),
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
