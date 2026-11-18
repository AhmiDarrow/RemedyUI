//! The Remedy font families, as data for RemedyUI's text engine.
//!
//! Each family is a Cargo feature, so an app carries only the faces it uses. Remedy Sans UI
//! is on by default. The fonts are the hinted TrueType builds from their own repositories and
//! are licensed under the SIL Open Font License 1.1 (see `fonts/OFL-*.txt`); this crate's
//! code is MIT OR Apache-2.0.
//!
//! | Feature | Family | Faces |
//! |---|---|---|
//! | `remedy-sans-ui` (default) | Remedy Sans UI: the interface face | Light, Semilight, Regular, Semibold, Bold |
//! | `remedy-sans` | Remedy Sans: the RemedyOS display face | Regular, Bold |
//! | `ahmi-print` | AhmiPrint: hand-printed, with same-width figures | Regular, Bold |
//! | `ahmi-pixel` | Ahmi Pixel: pixel face | Regular, Bold |
//!
//! ```
//! let faces = remedy_ui_fonts::all();
//! let regular = faces.iter().find(|f| f.family == "Remedy Sans UI" && f.weight == 400);
//! assert!(regular.is_some_and(|f| !f.data.is_empty()));
//! ```

/// Upright or italic. The Remedy families are upright today; the field is here so a future
/// italic face needs no API change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Style {
    /// Upright.
    Normal,
    /// Italic.
    Italic,
}

/// One font face: a family at one weight and style.
#[derive(Clone, Copy, Debug)]
pub struct Face {
    /// The typographic family name, as the font's own name table gives it.
    pub family: &'static str,
    /// The weight class (300 Light … 700 Bold).
    pub weight: u16,
    /// Upright or italic.
    pub style: Style,
    /// The TrueType file.
    pub data: &'static [u8],
}

macro_rules! face {
    ($family:literal, $weight:literal, $file:literal) => {
        Face {
            family: $family,
            weight: $weight,
            style: Style::Normal,
            data: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/", $file)),
        }
    };
}

/// Remedy Sans UI, the interface face, in five weights.
#[cfg(feature = "remedy-sans-ui")]
pub const REMEDY_SANS_UI: &[Face] = &[
    face!("Remedy Sans UI", 300, "RemedySansUI-Light.ttf"),
    face!("Remedy Sans UI", 350, "RemedySansUI-Semilight.ttf"),
    face!("Remedy Sans UI", 400, "RemedySansUI-Regular.ttf"),
    face!("Remedy Sans UI", 600, "RemedySansUI-Semibold.ttf"),
    face!("Remedy Sans UI", 700, "RemedySansUI-Bold.ttf"),
];

/// Remedy Sans, the RemedyOS display face.
#[cfg(feature = "remedy-sans")]
pub const REMEDY_SANS: &[Face] =
    &[face!("Remedy Sans", 400, "RemedySans-Regular.ttf"), face!("Remedy Sans", 700, "RemedySans-Bold.ttf")];

/// AhmiPrint, the hand-printed face with same-width figures.
#[cfg(feature = "ahmi-print")]
pub const AHMI_PRINT: &[Face] =
    &[face!("AhmiPrint", 400, "AhmiPrint-Regular.ttf"), face!("AhmiPrint", 700, "AhmiPrint-Bold.ttf")];

/// Ahmi Pixel, the pixel face.
#[cfg(feature = "ahmi-pixel")]
pub const AHMI_PIXEL: &[Face] =
    &[face!("Ahmi Pixel", 400, "AhmiPixel-Regular.ttf"), face!("Ahmi Pixel", 700, "AhmiPixel-Bold.ttf")];

/// Every face compiled into this build (the enabled features).
pub fn all() -> Vec<&'static Face> {
    let mut out: Vec<&'static Face> = Vec::new();
    #[cfg(feature = "remedy-sans-ui")]
    out.extend(REMEDY_SANS_UI);
    #[cfg(feature = "remedy-sans")]
    out.extend(REMEDY_SANS);
    #[cfg(feature = "ahmi-print")]
    out.extend(AHMI_PRINT);
    #[cfg(feature = "ahmi-pixel")]
    out.extend(AHMI_PIXEL);
    out
}
