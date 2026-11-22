//! Text settings shared by the text engine and style sheets.

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

/// How tall each line of text is (CSS `line-height`); inherited.
#[derive(Clone, Copy, Debug, PartialEq)]
// Exhaustive on purpose: CSS `normal`, a number and a length (a percentage is a number).
pub enum LineHeight {
    /// The font's own line spacing (ascent + descent + line gap).
    FontMetrics,
    /// A multiple of the font size, as CSS `line-height: 1.33`.
    Relative(f32),
    /// An exact height in logical pixels.
    Absolute(f32),
}
