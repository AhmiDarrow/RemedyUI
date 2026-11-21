//! Rich text: a string, styled spans and inline objects.

use std::ops::Range;

use crate::style::TextStylePatch;

/// The character that stands for an inline object in the text (U+FFFC).
pub const OBJECT_REPLACEMENT: char = '\u{FFFC}';

/// Something laid out inline with the text, like a character: an emoji image, a mention pill,
/// a custom widget. It takes its size from the caller and wraps with the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineObject {
    /// The caller's id for the object, reported back with its position.
    pub id: u64,
    /// Width in logical pixels.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
    /// Distance from the object's top to its baseline; `None` puts its bottom on the text's
    /// baseline.
    pub baseline: Option<f32>,
}

/// A styled span.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// Byte range in the text.
    pub range: Range<usize>,
    /// The style changes for that range.
    pub patch: TextStylePatch,
}

/// A paragraph's content: text, styled spans (which may nest and overlap; later spans win)
/// and inline objects.
///
/// ```
/// use remedy_ui_text::{RichText, TextStylePatch};
/// let msg = RichText::new()
///     .text("Meet at ")
///     .styled("10:30", TextStylePatch::bold())
///     .text(" in ")
///     .styled("#general", TextStylePatch::link(7));
/// assert_eq!(msg.as_str(), "Meet at 10:30 in #general");
/// assert_eq!(msg.spans().len(), 2);
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichText {
    text: String,
    spans: Vec<Span>,
    objects: Vec<(usize, InlineObject)>,
}

impl RichText {
    /// Empty rich text.
    pub fn new() -> Self {
        Self::default()
    }

    /// Plain text with no spans.
    pub fn plain(text: impl Into<String>) -> Self {
        RichText { text: text.into(), ..Self::default() }
    }

    /// Appends unstyled text.
    pub fn text(mut self, s: &str) -> Self {
        self.push_text(s);
        self
    }

    /// Appends text with a style patch.
    pub fn styled(mut self, s: &str, patch: TextStylePatch) -> Self {
        self.push_styled(s, patch);
        self
    }

    /// Appends an inline object.
    pub fn object(mut self, object: InlineObject) -> Self {
        self.push_object(object);
        self
    }

    /// Appends unstyled text in place.
    pub fn push_text(&mut self, s: &str) {
        self.text.push_str(s);
    }

    /// Appends styled text in place.
    pub fn push_styled(&mut self, s: &str, patch: TextStylePatch) {
        let start = self.text.len();
        self.text.push_str(s);
        self.spans.push(Span { range: start..self.text.len(), patch });
    }

    /// Styles a byte range of the existing text. Ranges that are out of bounds or that cut
    /// through a character are clamped to the nearest character boundaries inside the text.
    pub fn add_span(&mut self, range: Range<usize>, patch: TextStylePatch) {
        let start = self.floor_boundary(range.start);
        let end = self.floor_boundary(range.end.max(range.start));
        if start < end {
            self.spans.push(Span { range: start..end, patch });
        }
    }

    /// Appends an inline object in place.
    pub fn push_object(&mut self, object: InlineObject) {
        let at = self.text.len();
        self.text.push(OBJECT_REPLACEMENT);
        self.objects.push((at, object));
    }

    /// Replaces the inline objects (each at a U+FFFC already in the text).
    pub(crate) fn set_objects(&mut self, objects: Vec<(usize, InlineObject)>) {
        self.objects = objects;
    }

    /// The text, with U+FFFC where objects are.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The spans, in the order they apply.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Inline objects with their byte offsets.
    pub fn objects(&self) -> &[(usize, InlineObject)] {
        &self.objects
    }

    /// The length of the text in bytes.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// True when there is no text and no object.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The part of the text in `range` (on character boundaries), with the spans and objects
    /// in it, moved to start at 0.
    pub(crate) fn slice(&self, range: Range<usize>) -> RichText {
        let text = self.text.get(range.clone()).unwrap_or_default().to_owned();
        let spans = self
            .spans
            .iter()
            .filter_map(|s| {
                let (a, b) = (s.range.start.max(range.start), s.range.end.min(range.end));
                (a < b).then(|| Span { range: a - range.start..b - range.start, patch: s.patch.clone() })
            })
            .collect();
        let objects = self
            .objects
            .iter()
            .filter(|(at, _)| range.contains(at))
            .map(|&(at, o)| (at - range.start, o))
            .collect();
        RichText { text, spans, objects }
    }

    fn floor_boundary(&self, i: usize) -> usize {
        let mut i = i.min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

impl From<&str> for RichText {
    fn from(s: &str) -> Self {
        RichText::plain(s)
    }
}

impl From<String> for RichText {
    fn from(s: String) -> Self {
        RichText::plain(s)
    }
}
