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

/// True for locales written right to left (Arabic, Hebrew, Persian, Urdu, …).
pub fn is_rtl_locale(locale: &str) -> bool {
    let lang = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    matches!(
        lang.as_str(),
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "dv" | "ckb" | "syr"
    )
}

pub use atlas::{AtlasEntry, AtlasStats, GlyphAtlas, GlyphKey, PAGE_SIZE, Page, PageRect};
pub use code_edit::{CodeEditor, Indent};
pub use editing::{
    Composition, SecretEditor, SecretString, TextChange, TextEditor, TextSelection, UNDO_PAUSE_MS,
};
pub use engine::{
    Affinity, FontCollection, FontError, GlyphRunInfo, LineInfo, Paragraph, ParagraphOptions,
    PositionedGlyph, RunFont, TextEngine, TextPosition,
};
pub use pseudo::Pseudo;
pub use raster::{Antialias, GlyphImage, GlyphRasterizer, Hinting, SUBPIXEL_BINS, size_64, subpixel_bin};
pub use rich::{InlineObject, OBJECT_REPLACEMENT, RichText, Span};
pub use rich_edit::{Block, MAX_LEVEL, Mark, RichDoc, RichEditor, RichLook, RichView, SOFT_BREAK};
pub use rich_html::{from_html, to_html};
pub use safety::{Reveal, has_hidden_controls, looks_like, reveal_controls, skeleton};
pub use style::{Align, Direction, Feature, LineHeight, TextStyle, TextStylePatch};
