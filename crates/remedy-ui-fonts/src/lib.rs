//! The Remedy font families and the script fallback faces, as data for RemedyUI's text engine.
//!
//! Each family is a Cargo feature, so an app carries only the faces it uses. Remedy Sans UI
//! and Remedy Symbols are on by default. The fonts are the hinted TrueType builds from their own repositories and
//! are licensed under the SIL Open Font License 1.1 (see `fonts/OFL-*.txt`); this crate's
//! code is MIT OR Apache-2.0.
//!
//! | Feature | Family | Faces |
//! |---|---|---|
//! | `remedy-sans-ui` (default) | Remedy Sans UI: the interface face | Light, Semilight, Regular, Semibold, Bold |
//! | `remedy-sans` | Remedy Sans: the RemedyOS display face | Regular, Bold |
//! | `ahmi-print` | AhmiPrint: hand-printed, with same-width figures | Regular, Bold |
//! | `ahmi-pixel` | Ahmi Pixel: pixel face | Regular, Bold |
//! | `remedy-symbols` (default) | Remedy Symbols: interface symbols the text faces lack, for fallback (9 KB) | Regular |
//! | `noto-arabic` | Noto Sans Arabic: Arabic, Persian and Urdu letters, digits and marks, for fallback | Regular, Bold |
//! | `noto-devanagari` | Noto Sans Devanagari: Hindi, Marathi, Nepali and the other Devanagari languages, for fallback | Regular, Bold |
//! | `noto-bengali` | Noto Sans Bengali: Bengali and Assamese, for fallback | Regular, Bold |
//! | `scripts` | All three script fallbacks above | |
//!
//! Remedy Sans UI writes Latin (with every letter Spanish, French, Portuguese and German
//! need) and Cyrillic. The Noto faces cover the scripts it does not, so an interface in
//! Arabic, Hindi or Bengali never shows empty boxes even where no such font is installed;
//! [`script_fallbacks`] says which face serves which script, for
//! `FontCollection::add_script_fallback` in `remedy-ui-text`. Chinese and Japanese faces
//! are in `remedy-ui-fonts-cjk` (they are larger than everything here together).
//!
//! Remedy Symbols covers arrows (← ↔ ↻ ⇄ ⇒ ⇥ ⟶ …), geometric shapes (■ □ ▲ ▶ ◆ ◇ ◈ ○ ◎ ● …),
//! stars (★ ☆ ✦ ✧), check marks and crosses (✓ ✔ ✕ ✗ ✘), and interface signs (⚠ ☰ ≡ ⌕ ⌘ ⌂
//! ⋮ ⋯ ⚙ ⏸ ⏹ ⏺ ⏏). It has no letters, so it never takes a script's text from that script's
//! own font: the text engine reaches it for symbols the chosen font lacks (see
//! `FontCollection::set_symbol_fallbacks` in `remedy-ui-text`). [`SYMBOL_CHARS`] lists every
//! character it has.
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

#[allow(unused_macros, reason = "unused when no family is enabled")]
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

/// Remedy Symbols, the fallback face for interface symbols.
#[cfg(feature = "remedy-symbols")]
pub const REMEDY_SYMBOLS: &[Face] = &[face!("Remedy Symbols", 400, "RemedySymbols-Regular.ttf")];

/// Every character Remedy Symbols has, in code point order.
#[cfg(feature = "remedy-symbols")]
pub const SYMBOL_CHARS: &str =
    "‣⁃←↑→↓↔↕↖↗↘↙↺↻⇄⇅⇆⇐⇒⇔⇤⇥≡⋮⋯⌂⌕⌘⏏⏴⏵⏶⏷⏸⏹⏺■□▪▫▬▭▲△▴▵▶▷▸▹►▼▽▾▿◀◁◂◃◄◆◇◈◉○◎●◦◯★☆☰⚙⚠✓✔✕✖✗✘✦✧⟵⟶⟷";

/// Noto Sans Arabic: the Arabic script's letters, marks, digits and punctuation (Arabic,
/// Persian, Urdu), for fallback.
#[cfg(feature = "noto-arabic")]
pub const NOTO_SANS_ARABIC: &[Face] = &[
    face!("Noto Sans Arabic", 400, "NotoSansArabic-Regular.ttf"),
    face!("Noto Sans Arabic", 700, "NotoSansArabic-Bold.ttf"),
];

/// Noto Sans Devanagari: Devanagari letters, conjuncts, digits and dandas (Hindi, Marathi,
/// Nepali, Sanskrit), for fallback.
#[cfg(feature = "noto-devanagari")]
pub const NOTO_SANS_DEVANAGARI: &[Face] = &[
    face!("Noto Sans Devanagari", 400, "NotoSansDevanagari-Regular.ttf"),
    face!("Noto Sans Devanagari", 700, "NotoSansDevanagari-Bold.ttf"),
];

/// Noto Sans Bengali: Bengali letters, conjuncts, digits and the taka sign (Bengali,
/// Assamese), for fallback.
#[cfg(feature = "noto-bengali")]
pub const NOTO_SANS_BENGALI: &[Face] = &[
    face!("Noto Sans Bengali", 400, "NotoSansBengali-Regular.ttf"),
    face!("Noto Sans Bengali", 700, "NotoSansBengali-Bold.ttf"),
];

/// A family the text engine falls back to for one writing system, when the style's own
/// families lack it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScriptFallback {
    /// The script, as an ISO 15924 code (`Arab`, `Deva`, `Beng`, `Hani`, `Hira`, `Kana`).
    pub script: &'static str,
    /// Only for text in this language (`ja`, `zh-CN`); `None` for the script's default.
    pub locale: Option<&'static str>,
    /// The family to draw it with.
    pub family: &'static str,
}

/// What each enabled script face is a fallback for, in the order to try them.
#[allow(clippy::vec_init_then_push, reason = "each entry is behind its own feature")]
pub fn script_fallbacks() -> Vec<ScriptFallback> {
    #[allow(unused_mut, reason = "empty when no script face is enabled")]
    let mut out = Vec::new();
    #[cfg(feature = "noto-arabic")]
    out.push(ScriptFallback { script: "Arab", locale: None, family: "Noto Sans Arabic" });
    #[cfg(feature = "noto-devanagari")]
    out.push(ScriptFallback { script: "Deva", locale: None, family: "Noto Sans Devanagari" });
    #[cfg(feature = "noto-bengali")]
    out.push(ScriptFallback { script: "Beng", locale: None, family: "Noto Sans Bengali" });
    out
}

/// True for a face that is only a fallback for scripts the text faces lack (the Noto
/// script faces and Remedy Symbols), not a face to set text in.
pub fn is_fallback_only(family: &str) -> bool {
    family == "Remedy Symbols" || family.starts_with("Noto Sans ")
}

/// Every face compiled into this build (the enabled features).
pub fn all() -> Vec<&'static Face> {
    #[allow(unused_mut, reason = "empty when no family is enabled")]
    let mut out: Vec<&'static Face> = Vec::new();
    #[cfg(feature = "remedy-sans-ui")]
    out.extend(REMEDY_SANS_UI);
    #[cfg(feature = "remedy-sans")]
    out.extend(REMEDY_SANS);
    #[cfg(feature = "ahmi-print")]
    out.extend(AHMI_PRINT);
    #[cfg(feature = "ahmi-pixel")]
    out.extend(AHMI_PIXEL);
    #[cfg(feature = "remedy-symbols")]
    out.extend(REMEDY_SYMBOLS);
    #[cfg(feature = "noto-arabic")]
    out.extend(NOTO_SANS_ARABIC);
    #[cfg(feature = "noto-devanagari")]
    out.extend(NOTO_SANS_DEVANAGARI);
    #[cfg(feature = "noto-bengali")]
    out.extend(NOTO_SANS_BENGALI);
    out
}
