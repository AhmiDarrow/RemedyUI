//! Text styles.

use remedy_ui_core::Color;

/// How tall each line is.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LineHeight {
    /// The font's own line spacing (ascent + descent + line gap).
    FontMetrics,
    /// A multiple of the font size, as CSS `line-height: 1.33`.
    Relative(f32),
    /// An exact height in logical pixels.
    Absolute(f32),
}

/// Horizontal alignment of each line within the paragraph's width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Align {
    /// Left in left-to-right text, right in right-to-left text.
    #[default]
    Start,
    /// Right in left-to-right text, left in right-to-left text.
    End,
    /// Centered.
    Center,
    /// Stretched to both edges, except the last line of each paragraph.
    Justify,
}

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

/// How letters are cased when drawn (CSS `text-transform`). Copying, selection and screen
/// readers keep the text as written. Casing follows the text engine's locale: Turkish and
/// Azerbaijani dotted and dotless i, German ß as SS in capitals, Greek final sigma.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
// Exhaustive on purpose: the cases CSS has.
pub enum TextTransform {
    /// As written.
    #[default]
    None,
    /// All capitals.
    Upper,
    /// All small letters.
    Lower,
    /// The first letter of each word a capital (title case), the rest as written.
    Capitalize,
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
        }
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
        s.features.extend(self.features.iter().copied());
        s
    }
}
