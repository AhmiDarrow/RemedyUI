//! Text styles.

use remedy_ui_core::Color;
pub use remedy_ui_core::{LineHeight, TextTransform};

/// Horizontal alignment of each line within the paragraph's width: the same type as style
/// sheets' [`TextAlign`](remedy_ui_core::TextAlign).
pub use remedy_ui_core::TextAlign as Align;

/// The direction of the paragraph as a whole. Runs inside it still follow the Unicode
/// bidirectional algorithm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// Exhaustive on purpose: text runs left to right or right to left.
pub enum Direction {
    /// Taken from the first strong character (the Unicode default).
    #[default]
    Auto,
    /// Left to right.
    Ltr,
    /// Right to left.
    Rtl,
}

/// An OpenType feature setting, such as `tnum` (tabular figures) at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Feature {
    /// The four-letter tag.
    pub tag: [u8; 4],
    /// The value; 1 turns most features on, 0 off.
    pub value: u16,
}

impl Feature {
    /// Tabular (same-width) figures.
    pub const TABULAR_FIGURES: Feature = Feature { tag: *b"tnum", value: 1 };
}

/// A complete text style.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextStyle {
    /// Families in fallback order. The first that covers a character draws it. CSS's
    /// generic names (`monospace`, `serif`, `sans-serif`, `system-ui`, `ui-monospace`) stand
    /// for the system's choice, when the collection reaches the system's fonts.
    pub families: Vec<String>,
    /// Font size in logical pixels.
    pub size: f32,
    /// Weight, 100 to 900 (400 regular, 700 bold).
    pub weight: f32,
    /// Italic.
    pub italic: bool,
    /// Text color.
    pub color: Color,
    /// Extra space between letters, in logical pixels.
    pub letter_spacing: f32,
    /// How letters are cased when drawn.
    pub transform: TextTransform,
    /// Line height.
    pub line_height: LineHeight,
    /// Underline.
    pub underline: bool,
    /// Line through the middle.
    pub strikethrough: bool,
    /// OpenType features.
    pub features: Vec<Feature>,
    /// Filled background behind the whole paragraph. A span's own background replaces it
    /// on that span.
    pub background: Option<SpanBackground>,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            families: vec!["Remedy Sans UI".into()],
            size: 14.0,
            weight: 400.0,
            italic: false,
            color: Color::BLACK,
            letter_spacing: 0.0,
            transform: TextTransform::None,
            line_height: LineHeight::Relative(1.33),
            underline: false,
            strikethrough: false,
            features: Vec::new(),
            background: None,
        }
    }
}

/// A length in logical pixels. Negative, NaN and infinite values become zero.
pub(crate) fn finite_px(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

/// A filled background painted behind a span, the way an inline code chip is drawn.
///
/// Horizontal padding is laid out: the span is wider by twice [`Self::padding_inline`], and
/// the fill covers that room, so the chip does not sit on the words beside it. Vertical
/// padding is painted above the ascent and below the descent and does not change the line
/// height.
///
/// ```
/// use remedy_ui_core::Color;
/// use remedy_ui_text::{SpanBackground, TextStyle, TextStylePatch};
///
/// let chip = SpanBackground::chip(Color::rgb8(40, 40, 48), 14.0);
/// assert_eq!(chip.padding_inline, 14.0 * 0.4);
/// assert_eq!(chip.padding_block, 14.0 * 0.12);
/// assert_eq!(chip.radius, 4.0);
/// let patch = TextStylePatch::background(chip);
/// assert_eq!(patch.apply(&TextStyle::default()).background, Some(chip));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SpanBackground {
    /// Fill color.
    pub color: Color,
    /// Extra room on the inline start and on the inline end, in logical pixels.
    pub padding_inline: f32,
    /// Extra room above the ascent and below the descent, in logical pixels.
    pub padding_block: f32,
    /// Corner radius, in logical pixels.
    pub radius: f32,
}

impl SpanBackground {
    /// A flat fill, no padding, square corners.
    pub fn new(color: Color) -> Self {
        SpanBackground { color, padding_inline: 0.0, padding_block: 0.0, radius: 0.0 }
    }

    /// The compact chip Remedy Desktop draws around inline code, at font size `size`:
    /// `0.12em` above and below, `0.4em` on each side, corners of 4.
    pub fn chip(color: Color, size: f32) -> Self {
        let size = finite_px(size);
        SpanBackground { color, padding_inline: size * 0.4, padding_block: size * 0.12, radius: 4.0 }
    }

    /// Sets the inline padding (logical pixels on the start and on the end).
    pub fn padding_inline(mut self, v: f32) -> Self {
        self.padding_inline = finite_px(v);
        self
    }

    /// Sets the block padding (logical pixels above the ascent and below the descent).
    pub fn padding_block(mut self, v: f32) -> Self {
        self.padding_block = finite_px(v);
        self
    }

    /// Sets both paddings: `inline` on the start and end, `block` above and below.
    pub fn padding(mut self, inline: f32, block: f32) -> Self {
        self.padding_inline = finite_px(inline);
        self.padding_block = finite_px(block);
        self
    }

    /// Sets the corner radius, in logical pixels.
    pub fn radius(mut self, v: f32) -> Self {
        self.radius = finite_px(v);
        self
    }
}

/// Changes a span makes to the style around it. Unset fields are inherited.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TextStylePatch {
    /// Families in fallback order.
    pub families: Option<Vec<String>>,
    /// Font size.
    pub size: Option<f32>,
    /// Weight.
    pub weight: Option<f32>,
    /// Italic.
    pub italic: Option<bool>,
    /// Color.
    pub color: Option<Color>,
    /// Letter spacing.
    pub letter_spacing: Option<f32>,
    /// Casing.
    pub transform: Option<TextTransform>,
    /// Underline.
    pub underline: Option<bool>,
    /// Strikethrough.
    pub strikethrough: Option<bool>,
    /// Features added to the inherited ones.
    pub features: Vec<Feature>,
    /// Marks the span as a link with this id; hit tests report it.
    pub link: Option<u64>,
    /// Filled background behind the span. `None` keeps the background around it.
    pub background: Option<SpanBackground>,
}

impl TextStylePatch {
    /// Bold (weight 700).
    pub fn bold() -> Self {
        TextStylePatch { weight: Some(700.0), ..Self::default() }
    }

    /// Italic.
    pub fn italic() -> Self {
        TextStylePatch { italic: Some(true), ..Self::default() }
    }

    /// A color.
    pub fn color(c: Color) -> Self {
        TextStylePatch { color: Some(c), ..Self::default() }
    }

    /// A link with an id the app knows, underlined.
    pub fn link(id: u64) -> Self {
        TextStylePatch { link: Some(id), underline: Some(true), ..Self::default() }
    }

    /// Letters cased by `t` when drawn ([`TextTransform`]).
    pub fn transform(t: TextTransform) -> Self {
        TextStylePatch { transform: Some(t), ..Self::default() }
    }

    /// Strikethrough.
    pub fn strikethrough() -> Self {
        TextStylePatch { strikethrough: Some(true), ..Self::default() }
    }

    /// A filled background behind the span ([`SpanBackground`]), such as an inline code chip.
    pub fn background(bg: SpanBackground) -> Self {
        TextStylePatch { background: Some(bg), ..Self::default() }
    }

    /// Applies this patch over `base`.
    pub fn apply(&self, base: &TextStyle) -> TextStyle {
        let mut s = base.clone();
        if let Some(f) = &self.families {
            s.families = f.clone();
        }
        if let Some(v) = self.size {
            s.size = v;
        }
        if let Some(v) = self.weight {
            s.weight = v;
        }
        if let Some(v) = self.italic {
            s.italic = v;
        }
        if let Some(v) = self.color {
            s.color = v;
        }
        if let Some(v) = self.letter_spacing {
            s.letter_spacing = v;
        }
        if let Some(v) = self.transform {
            s.transform = v;
        }
        if let Some(v) = self.underline {
            s.underline = v;
        }
        if let Some(v) = self.strikethrough {
            s.strikethrough = v;
        }
        if let Some(v) = self.background {
            s.background = Some(v);
        }
        s.features.extend(self.features.iter().copied());
        s
    }
}
