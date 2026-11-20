//! RemedyUI text: fonts, shaping, bidi, line breaking, rich paragraphs and caret geometry.
//!
//! Text is shaped by HarfRust and laid out by parley (bidi by UAX #9, line breaking by
//! UAX #14, grapheme-safe carets), behind RemedyUI's own types so the engine underneath can
//! change without breaking apps. See `docs/design/05-text.md`.
//!
//! ```
//! use remedy_ui_text::*;
//!
//! let mut fonts = FontCollection::new();
//! for face in remedy_ui_fonts::all() {
//!     fonts.add_static(face.data)?;
//! }
//! let mut engine = TextEngine::new(fonts);
//! let text = RichText::new().text("Hello, ").styled("world", TextStylePatch::bold());
//! let options = ParagraphOptions::default().max_width(200.0);
//! let para = engine.layout(&text, &TextStyle::default(), options);
//! assert_eq!(para.lines().len(), 1);
//! assert!(para.width() > 0.0 && para.width() <= 200.0);
//! # Ok::<(), FontError>(())
//! ```

mod atlas;
mod code_edit;
mod editing;
mod engine;
mod pseudo;
mod raster;
mod rich;
mod rich_edit;
mod rich_html;
mod safety;
mod style;

/// True for locales written right to left (Arabic, Hebrew, Persian, Urdu, …). A script
/// subtag decides when there is one: `pa-Arab` (Punjabi in Shahmukhi) and `ks-Arab` are right
/// to left, `ug-Latn` and `az-Latn` are not; otherwise the language's usual script does.
///
/// ```
/// use remedy_ui_text::is_rtl_locale;
/// assert!(is_rtl_locale("ar-EG") && is_rtl_locale("he") && is_rtl_locale("pa-Arab-PK"));
/// assert!(!is_rtl_locale("en-US") && !is_rtl_locale("hi") && !is_rtl_locale("ug-Latn"));
/// ```
pub fn is_rtl_locale(locale: &str) -> bool {
    let mut parts = locale.split(['-', '_']);
    let lang = parts.next().unwrap_or("").to_ascii_lowercase();
    // A script subtag is four letters, right after the language.
    if let Some(script) = parts.next().filter(|s| s.len() == 4 && s.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return matches!(
            script.to_ascii_lowercase().as_str(),
            "arab" | "aran" | "hebr" | "syrc" | "thaa" | "nkoo" | "adlm" | "rohg" | "mand" | "samr" | "mend"
        );
    }
    matches!(
        lang.as_str(),
        "ar" | "he"
            | "iw"
            | "fa"
            | "ur"
            | "ps"
            | "sd"
            | "ug"
            | "yi"
            | "dv"
            | "ckb"
            | "syr"
            | "ks"
            | "prs"
            | "arc"
            | "nqo"
            | "lrc"
            | "mzn"
            | "glk"
    )
}

pub use atlas::{AtlasEntry, AtlasStats, GlyphAtlas, GlyphKey, PAGE_SIZE, Page, PageRect};
pub use code_edit::{CodeEditor, Indent};
pub use editing::{
    Composition, SecretEditor, SecretString, TextChange, TextEditor, TextSelection, UNDO_PAUSE_MS,
};
pub use engine::{
    Affinity, DEFAULT_SYMBOL_FALLBACKS, FontCollection, FontError, FontFile, GlyphRunInfo, LineInfo,
    Paragraph, ParagraphOptions, PositionedGlyph, RunFont, TextEngine, TextPosition,
};
pub use pseudo::Pseudo;
pub use raster::{Antialias, GlyphImage, GlyphRasterizer, Hinting, SUBPIXEL_BINS, size_64, subpixel_bin};
pub use rich::{InlineObject, OBJECT_REPLACEMENT, RichText, Span};
pub use rich_edit::{Block, MAX_LEVEL, Mark, RichDoc, RichEditor, RichLook, RichView, SOFT_BREAK};
pub use rich_html::{from_html, to_html};
pub use safety::{Reveal, has_hidden_controls, looks_like, reveal_controls, skeleton};
pub use style::{Align, Direction, Feature, LineHeight, TextStyle, TextStylePatch};
