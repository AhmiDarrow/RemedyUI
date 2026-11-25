//! What the app supplies: its pages, as a [`PageSource`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use remedy_ui_core::{Point, Rect, Size};
use remedy_ui_engine::paint::Image;

/// A document's pages, supplied by the app. The engine never parses a document: a PDF app
/// wraps its PDF library here, an e-book app its layout engine.
///
/// The view calls [`render`](Self::render) and [`text`](Self::text) on worker threads, never
/// on the UI thread, so an implementation must be `Send + Sync` and may take its time.
/// [`page_count`](Self::page_count) and [`page_size`](Self::page_size) are called on the UI
/// thread while laying out and must be quick (read them from the document's page tree).
pub trait PageSource: Send + Sync + 'static {
    /// How many pages there are.
    fn page_count(&self) -> usize;

    /// A page's size in points (1/72 inch), before any rotation the view applies.
    fn page_size(&self, page: usize) -> Size;

    /// Draws part of a page: the area `request.region` (points, the page's own unrotated
    /// space) at `request.scale` pixels per point, into an image exactly
    /// `request.width` × `request.height` pixels whose top-left pixel is `region.min`.
    ///
    /// Return [`RenderError::Cancelled`] early when `cancel` says the tile is no longer
    /// wanted (it scrolled away, or the zoom moved on); long renders should look at it now
    /// and then.
    fn render(&self, request: &RenderRequest, cancel: &Cancel) -> Result<Image, RenderError>;

    /// The page's text, as runs placed on the page, for selection, copying, search and screen
    /// readers. `None` when the page has no text layer (a scan without OCR).
    fn text(&self, page: usize) -> Option<PageText> {
        let _ = page;
        None
    }
}

/// One tile of one page to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RenderRequest {
    /// The page.
    pub page: usize,
    /// The area of the page to draw, in points, in the page's unrotated space.
    pub region: Rect,
    /// Pixels per point.
    pub scale: f32,
    /// Width of the image to return, in pixels.
    pub width: u32,
    /// Height of the image to return, in pixels.
    pub height: u32,
}

impl RenderRequest {
    /// The transform from page points to this tile's pixels: subtract `region.min`, then
    /// scale. Sources that draw with an affine (most renderers) use it directly.
    pub fn page_to_pixels(&self) -> [f32; 6] {
        let s = self.scale;
        [s, 0.0, 0.0, s, -self.region.min.x * s, -self.region.min.y * s]
    }
}

/// Tells a render that its result is no longer wanted.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A token nobody has cancelled.
    pub fn new() -> Self {
        Cancel::default()
    }

    /// True once the result is no longer wanted.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Withdraws the request.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Why a page could not be drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderError {
    /// The request was withdrawn before it finished.
    Cancelled,
    /// The source failed, and why (shown to the user on the page).
    Failed(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Cancelled => f.write_str("the page render was cancelled"),
            RenderError::Failed(why) => write!(f, "the page could not be drawn: {why}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// A page's text: runs of text placed on the page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    /// The runs in reading order.
    pub runs: Vec<TextRun>,
}

/// A run of text on a page (a line or a part of one, in one direction).
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// The text.
    pub text: String,
    /// Its box on the page, in points (unrotated page space).
    pub bounds: Rect,
    /// One box per character of `text` (by `chars()`), in points. Empty: the characters
    /// share `bounds` evenly, which suits monospaced runs and is close for others.
    pub chars: Vec<Rect>,
}

impl TextRun {
    /// A run whose characters share its box evenly.
    pub fn new(text: impl Into<String>, bounds: Rect) -> Self {
        TextRun { text: text.into(), bounds, chars: Vec::new() }
    }

    /// Number of characters.
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// True for a run with no characters.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The box of character `i`.
    pub fn char_rect(&self, i: usize) -> Rect {
        if let Some(r) = self.chars.get(i) {
            return *r;
        }
        let n = self.len().max(1) as f32;
        let w = self.bounds.width() / n;
        let x = self.bounds.min.x + w * i as f32;
        Rect::new(x, self.bounds.min.y, x + w, self.bounds.max.y)
    }

    /// The character boundary nearest `p` along the run: 0 before the first character,
    /// `len()` after the last.
    pub fn boundary_at(&self, p: Point) -> usize {
        let n = self.len();
        (0..n).find(|&i| p.x < self.char_rect(i).center().x).unwrap_or(n)
    }

    /// Characters `range` as a string.
    pub fn slice(&self, range: std::ops::Range<usize>) -> String {
        self.text.chars().skip(range.start).take(range.end.saturating_sub(range.start)).collect()
    }
}
