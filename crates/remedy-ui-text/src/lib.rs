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
//! let text = RichText::new().text("Hello, ").styled("world", StylePatch::bold());
//! let options = ParagraphOptions { max_width: Some(200.0), ..Default::default() };
//! let para = engine.layout(&text, &TextStyle::default(), options);
//! assert_eq!(para.lines().len(), 1);
//! assert!(para.width() > 0.0 && para.width() <= 200.0);
//! # Ok::<(), FontError>(())
//! ```

mod atlas;
mod engine;
mod raster;
mod rich;
mod style;

pub use atlas::{AtlasEntry, AtlasStats, GlyphAtlas, GlyphKey, PAGE_SIZE, Page, PageRect};
pub use engine::{
    Affinity, FontCollection, FontError, GlyphRunInfo, LineInfo, Paragraph, ParagraphOptions,
    PositionedGlyph, RunFont, TextEngine, TextPosition,
};
pub use raster::{Antialias, GlyphImage, GlyphRasterizer, Hinting, SUBPIXEL_BINS, subpixel_bin};
pub use rich::{InlineObject, OBJECT_REPLACEMENT, RichText, Span};
pub use style::{Align, Direction, Feature, LineHeight, StylePatch, TextStyle};
