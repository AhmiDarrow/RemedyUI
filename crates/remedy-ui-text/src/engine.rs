//! Fonts, the layout engine and laid-out paragraphs.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::setting::Tag;
use parley::{
    Affinity as PAffinity, Alignment, AlignmentOptions, BaseDirection, Cursor, FontContext, FontFamily,
    FontFamilyName, FontFeature, FontFeatures, FontStyle, FontWeight, InlineBox, InlineBoxKind, Layout,
    LayoutContext, LineHeight as PLineHeight, PositionedLayoutItem, Selection, StyleProperty,
};
use remedy_ui_core::{Color, Point, Rect};

use crate::rich::RichText;
use crate::style::{Align, Direction, LineHeight, TextStyle};

/// A font could not be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontError(pub String);

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FontError {}

/// The fonts a [`TextEngine`] can use. Starts empty; apps add their bundled fonts (for
/// example from `remedy-ui-fonts`) and, with the `system-fonts` feature, may let fallback
/// reach the system's fonts.
pub struct FontCollection {
    fcx: FontContext,
}

impl std::fmt::Debug for FontCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FontCollection")
    }
}

impl Default for FontCollection {
    fn default() -> Self {
        Self::new()
    }
}

impl FontCollection {
    /// A collection with no fonts and no access to system fonts.
    pub fn new() -> Self {
        let collection = Collection::new(CollectionOptions { shared: false, system_fonts: false });
        FontCollection { fcx: FontContext { collection, source_cache: SourceCache::default() } }
    }

    /// A collection that falls back to the system's fonts after the ones added to it.
    #[cfg(feature = "system-fonts")]
    pub fn with_system_fonts() -> Self {
        let collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
        FontCollection { fcx: FontContext { collection, source_cache: SourceCache::default() } }
    }

    /// Adds every face in a font file (a single font or a collection) from static bytes,
    /// returning the family names it contained. Bytes that are not a font add nothing and
    /// return an error.
    pub fn add_static(&mut self, data: &'static [u8]) -> Result<Vec<String>, FontError> {
        self.add(Blob::new(Arc::new(data)))
    }

    /// Adds every face in a font file from owned bytes (a user's font file).
    pub fn add_owned(&mut self, data: Vec<u8>) -> Result<Vec<String>, FontError> {
        self.add(Blob::new(Arc::new(data)))
    }

    fn add(&mut self, blob: Blob<u8>) -> Result<Vec<String>, FontError> {
        let added = self.fcx.collection.register_fonts(blob, None);
        if added.is_empty() {
            return Err(FontError("the data is not a font this engine can read".into()));
        }
        let mut names: Vec<String> = Vec::new();
        for (family, _) in added {
            if let Some(name) = self.fcx.collection.family_name(family) {
                let name = name.to_string();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Ok(names)
    }

    /// Every family name the collection can use.
    pub fn families(&mut self) -> Vec<String> {
        self.fcx.collection.family_names().map(str::to_string).collect()
    }
}

/// Per-run data RemedyUI needs back from layout. Each flattened segment has its own brush
/// id, so a glyph run maps back to exactly one contiguous byte range.
#[derive(Clone, Debug, Default, PartialEq)]
struct RunStyle {
    color: Color,
    link: Option<u64>,
    range: Range<usize>,
}

/// Lays out paragraphs. Keep one per UI thread and reuse it: it holds scratch space and the
/// fonts.
pub struct TextEngine {
    fonts: FontCollection,
    lcx: LayoutContext<u32>,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TextEngine")
    }
}

/// Paragraph settings that apply to the whole paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphOptions {
    /// Wrap width in logical pixels; `None` keeps each paragraph on one line.
    pub max_width: Option<f32>,
    /// Line alignment.
    pub align: Align,
    /// Paragraph direction.
    pub direction: Direction,
    /// The direction of the UI around the paragraph. When set, `Start` and `End` align to
    /// its edges (the start of a right-to-left window is its right edge) whatever the
    /// paragraph's own direction; `Auto` aligns to the paragraph's direction.
    pub container: Direction,
    /// Device pixels per logical pixel. Metrics are rounded for this scale when `snap` is on.
    pub scale: f32,
    /// Round line metrics to whole device pixels, so hinted text sits on the pixel grid.
    pub snap: bool,
}

impl Default for ParagraphOptions {
    fn default() -> Self {
        ParagraphOptions {
            max_width: None,
            align: Align::Start,
            direction: Direction::Auto,
            container: Direction::Auto,
            scale: 1.0,
            snap: true,
        }
    }
}

impl TextEngine {
    /// An engine using `fonts`.
    pub fn new(fonts: FontCollection) -> Self {
        TextEngine { fonts, lcx: LayoutContext::new() }
    }

    /// The engine's fonts, to add more.
    pub fn fonts(&mut self) -> &mut FontCollection {
        &mut self.fonts
    }

    /// Shapes and lays out `text` with `base` style and the spans and objects in `text`.
    pub fn layout(&mut self, text: &RichText, base: &TextStyle, options: ParagraphOptions) -> Paragraph {
        let segments = flatten(text, base);
        let mut run_styles = Vec::with_capacity(segments.len());
        let content = text.as_str();
        let mut b = self.lcx.ranged_builder(&mut self.fonts.fcx, content, options.scale, options.snap);
        b.set_base_direction(match options.direction {
            Direction::Auto => BaseDirection::Auto,
            Direction::Ltr => BaseDirection::Ltr,
            Direction::Rtl => BaseDirection::Rtl,
        });
        push_style(&mut b, base, None, 0);
        run_styles.push(RunStyle { color: base.color, link: None, range: 0..content.len() });
        for (i, seg) in segments.iter().enumerate() {
            let brush = u32::try_from(i + 1).unwrap_or(u32::MAX);
            push_style(&mut b, &seg.style, Some(seg.range.clone()), brush);
            run_styles.push(RunStyle { color: seg.style.color, link: seg.link, range: seg.range.clone() });
        }
        for &(at, obj) in text.objects() {
            b.push_inline_box(InlineBox {
                id: obj.id,
                kind: InlineBoxKind::InFlow,
                index: at,
                width: obj.width,
                height: obj.height,
                baseline: obj.baseline,
                vertical_align: Default::default(),
            });
        }
        let mut layout = b.build(content);
        let alignment = match (options.align, options.container) {
            (Align::Start, Direction::Ltr) | (Align::End, Direction::Rtl) => Alignment::Left,
            (Align::Start, Direction::Rtl) | (Align::End, Direction::Ltr) => Alignment::Right,
            (Align::Start, Direction::Auto) => Alignment::Start,
            (Align::End, Direction::Auto) => Alignment::End,
            (Align::Center, _) => Alignment::Center,
            (Align::Justify, _) => Alignment::Justify,
        };
        // Parley aligns while it breaks lines: set the alignment first.
        layout.set_alignment(alignment, AlignmentOptions::default());
        layout.break_all_lines(options.max_width);
        Paragraph { layout, run_styles, align: alignment, wrap: options.max_width }
    }
}

struct Segment {
    range: Range<usize>,
    style: TextStyle,
    link: Option<u64>,
}

/// Splits the text at every span boundary and resolves each piece's full style. The pieces
/// cover the whole text, so each gets its own brush.
fn flatten(text: &RichText, base: &TextStyle) -> Vec<Segment> {
    let spans = text.spans();
    if spans.is_empty() {
        return Vec::new();
    }
    let mut cuts: Vec<usize> = spans.iter().flat_map(|s| [s.range.start, s.range.end]).collect();
    cuts.push(0);
    cuts.push(text.len());
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        let [start, end] = [w.first().copied().unwrap_or(0), w.get(1).copied().unwrap_or(0)];
        if start >= end {
            continue;
        }
        let covering: Vec<_> =
            spans.iter().filter(|s| s.range.start <= start && s.range.end >= end).collect();
        let mut style = base.clone();
        let mut link = None;
        for s in covering {
            style = s.patch.apply(&style);
            link = s.patch.link.or(link);
        }
        out.push(Segment { range: start..end, style, link });
    }
    out
}

fn push_style(
    b: &mut parley::RangedBuilder<'_, u32>,
    s: &TextStyle,
    range: Option<Range<usize>>,
    brush: u32,
) {
    let families: Vec<FontFamilyName<'static>> =
        s.families.iter().map(|f| FontFamilyName::Named(Cow::Owned(f.clone()))).collect();
    let features: Vec<FontFeature> =
        s.features.iter().map(|f| FontFeature::new(Tag::from_bytes(f.tag), f.value)).collect();
    let line_height = match s.line_height {
        LineHeight::FontMetrics => PLineHeight::MetricsRelative(1.0),
        LineHeight::Relative(v) => PLineHeight::FontSizeRelative(v),
        LineHeight::Absolute(v) => PLineHeight::Absolute(v),
    };
    let props: Vec<StyleProperty<'static, u32>> = vec![
        StyleProperty::FontFamily(FontFamily::List(Cow::Owned(families))),
        StyleProperty::FontSize(s.size),
        StyleProperty::FontWeight(FontWeight::new(s.weight)),
        StyleProperty::FontStyle(if s.italic { FontStyle::Italic } else { FontStyle::Normal }),
        StyleProperty::LetterSpacing(s.letter_spacing),
        StyleProperty::LineHeight(line_height),
        StyleProperty::Underline(s.underline),
        StyleProperty::Strikethrough(s.strikethrough),
        StyleProperty::FontFeatures(FontFeatures::List(Cow::Owned(features))),
        StyleProperty::Brush(brush),
    ];
    for p in props {
        match &range {
            Some(r) => b.push(p, r.clone()),
            None => b.push_default(p),
        }
    }
}

/// Which side of a character boundary a caret belongs to, for boundaries where a line wraps
/// or the direction changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    /// With the character after the boundary.
    #[default]
    Downstream,
    /// With the character before the boundary.
    Upstream,
}

/// A position in the text: a byte index and its affinity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextPosition {
    /// Byte index, always on a character boundary.
    pub index: usize,
    /// Side of the boundary.
    pub affinity: Affinity,
}

/// One laid-out line.
#[derive(Clone, Debug, PartialEq)]
pub struct LineInfo {
    /// The text the line covers.
    pub range: Range<usize>,
    /// Baseline, from the paragraph's top.
    pub baseline: f32,
    /// Top of the line box.
    pub top: f32,
    /// Bottom of the line box.
    pub bottom: f32,
    /// Width of the line's content.
    pub width: f32,
    /// Where the line's content starts, from the paragraph's left edge (its alignment).
    pub left: f32,
}

/// A positioned glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// Glyph id in its font.
    pub id: u32,
    /// Position of the glyph origin (on the baseline), from the paragraph's top-left.
    pub x: f32,
    /// Baseline position.
    pub y: f32,
}

/// The font a run is drawn with: the font file's bytes (shared, not copied), the face
/// index within it, and a stable id for caches.
#[derive(Clone)]
pub struct RunFont {
    data: Blob<u8>,
    index: u32,
}

impl RunFont {
    /// The font file's bytes.
    pub fn bytes(&self) -> &[u8] {
        self.data.data()
    }

    /// The face index within a collection file (0 for a single font).
    pub fn index(&self) -> u32 {
        self.index
    }

    /// An id that is the same for the same font bytes, for glyph caches.
    pub fn id(&self) -> u64 {
        self.data.id()
    }
}

impl std::fmt::Debug for RunFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunFont(id {}, face {}, {} bytes)", self.id(), self.index, self.bytes().len())
    }
}

/// A run of glyphs in one font, size and style.
#[derive(Clone, Debug)]
pub struct GlyphRunInfo {
    /// The font.
    pub font: RunFont,
    /// Font size in logical pixels.
    pub size: f32,
    /// Fill color.
    pub color: Color,
    /// The link this run belongs to, if any.
    pub link: Option<u64>,
    /// True when the run reads right to left.
    pub rtl: bool,
    /// The text the run covers.
    pub range: Range<usize>,
    /// The glyphs.
    pub glyphs: Vec<PositionedGlyph>,
    /// The underline to draw under the run, from the paragraph's top-left, if it has one.
    pub underline: Option<Rect>,
    /// The line through the run, if it has one.
    pub strikethrough: Option<Rect>,
}

/// A laid-out paragraph: lines, glyph runs, inline objects, and caret geometry.
pub struct Paragraph {
    layout: Layout<u32>,
    run_styles: Vec<RunStyle>,
    align: Alignment,
    /// The wrap width lines were aligned within (`None`: aligned within the longest line).
    wrap: Option<f32>,
}

impl std::fmt::Debug for Paragraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paragraph").field("width", &self.width()).field("height", &self.height()).finish()
    }
}

fn to_affinity(a: PAffinity) -> Affinity {
    match a {
        PAffinity::Upstream => Affinity::Upstream,
        PAffinity::Downstream => Affinity::Downstream,
    }
}

fn from_affinity(a: Affinity) -> PAffinity {
    match a {
        Affinity::Upstream => PAffinity::Upstream,
        Affinity::Downstream => PAffinity::Downstream,
    }
}

fn bbox(b: parley::BoundingBox) -> Rect {
    Rect::new(b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32)
}

impl Paragraph {
    /// Lays the paragraph out again at a new width, without shaping again.
    pub fn rewrap(&mut self, max_width: Option<f32>) {
        self.layout.set_alignment(self.align, AlignmentOptions::default());
        self.layout.break_all_lines(max_width);
        self.wrap = max_width;
    }

    /// Where to draw the paragraph inside a box `box_width` wide so it sits as aligned.
    /// Wrapped paragraphs are already aligned within their wrap width (0); one laid out
    /// without one is only as wide as its longest line, so it moves to the box's right edge
    /// or center as its alignment says.
    pub fn x_offset(&self, box_width: f32) -> f32 {
        if self.wrap.is_some_and(f32::is_finite) {
            return 0.0;
        }
        let free = (box_width - self.width()).max(0.0);
        let rtl = self.layout.is_rtl();
        match self.align {
            Alignment::Right => free,
            Alignment::Center => free / 2.0,
            Alignment::Start | Alignment::Justify if rtl => free,
            Alignment::End if !rtl => free,
            _ => 0.0,
        }
    }

    /// Width of the widest line.
    pub fn width(&self) -> f32 {
        self.layout.width()
    }

    /// Total height.
    pub fn height(&self) -> f32 {
        self.layout.height()
    }

    /// True when the paragraph's base direction is right to left.
    pub fn is_rtl(&self) -> bool {
        self.layout.is_rtl()
    }

    /// The first line's baseline, from the top.
    pub fn first_baseline(&self) -> Option<f32> {
        self.layout.first_baseline()
    }

    /// The lines, top to bottom.
    pub fn lines(&self) -> Vec<LineInfo> {
        self.layout
            .lines()
            .map(|line| {
                let m = line.metrics();
                LineInfo {
                    range: line.text_range(),
                    baseline: m.baseline,
                    top: m.block_min_coord,
                    bottom: m.block_max_coord,
                    width: m.advance,
                    left: m.offset,
                }
            })
            .collect()
    }

    /// Glyph runs in drawing order, with positions from the paragraph's top-left.
    pub fn glyph_runs(&self) -> Vec<GlyphRunInfo> {
        let mut out = Vec::new();
        for line in self.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let style = run.style();
                let rs = self.run_styles.get(style.brush as usize).cloned().unwrap_or_default();
                let r = run.run();
                let font = r.font();
                let shaped = r.text_range();
                let range = rs.range.start.max(shaped.start)..rs.range.end.min(shaped.end);
                // Decorations: a line under the text and one through its middle, as thick as
                // the size asks (at least a pixel), placed from the span's metrics.
                let size = r.font_size();
                let metrics = run.span_metrics();
                let thick = (size / 14.0).max(1.0);
                let (x, baseline, width) = (run.offset(), run.baseline(), run.advance());
                let bar = |offset: f32, d: &parley::layout::Decoration<u32>| {
                    let t = d.size.unwrap_or(thick);
                    let y = baseline - d.offset.unwrap_or(offset);
                    Rect::new(x, y - t / 2.0, x + width, y + t / 2.0)
                };
                let below = -(metrics.descent.abs() * 0.4).max(thick);
                let middle = if metrics.x_height > 0.0 { metrics.x_height / 2.0 } else { size * 0.25 };
                let underline = style.underline.as_ref().map(|d| bar(below, d));
                let strikethrough = style.strikethrough.as_ref().map(|d| bar(middle, d));
                out.push(GlyphRunInfo {
                    font: RunFont { data: font.data.clone(), index: font.index },
                    size: r.font_size(),
                    color: rs.color,
                    link: rs.link,
                    rtl: r.is_rtl(),
                    range,
                    glyphs: run
                        .positioned_glyphs()
                        .map(|g| PositionedGlyph { id: g.id, x: g.x, y: g.y })
                        .collect(),
                    underline,
                    strikethrough,
                });
            }
        }
        out
    }

    /// Inline objects with their boxes, by the ids the caller gave them.
    pub fn inline_objects(&self) -> Vec<(u64, Rect)> {
        let mut out = Vec::new();
        for line in self.layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::InlineBox(b) = item {
                    out.push((b.id, Rect::new(b.x, b.y, b.x + b.width, b.y + b.height)));
                }
            }
        }
        out
    }

    /// The text position nearest `point` (from the paragraph's top-left).
    pub fn hit_test(&self, point: Point) -> TextPosition {
        let c = Cursor::from_point(&self.layout, point.x, point.y);
        TextPosition { index: c.index(), affinity: to_affinity(c.affinity()) }
    }

    /// The link under `point`, if the point is on linked text.
    pub fn link_at(&self, point: Point) -> Option<u64> {
        let pos = self.hit_test(point);
        self.glyph_runs()
            .into_iter()
            .filter(|r| r.link.is_some())
            .find(|r| r.range.contains(&pos.index))
            .filter(|r| self.selection_rects(r.range.clone()).iter().any(|rect| rect.contains(point)))
            .and_then(|r| r.link)
    }

    /// Rectangles covering the text of link `link`, one or more per line it is on, in visual
    /// order; empty when no text carries it.
    pub fn link_rects(&self, link: u64) -> Vec<Rect> {
        let runs: Vec<Range<usize>> =
            self.glyph_runs().into_iter().filter(|r| r.link == Some(link)).map(|r| r.range).collect();
        let (Some(start), Some(end)) = (runs.iter().map(|r| r.start).min(), runs.iter().map(|r| r.end).max())
        else {
            return Vec::new();
        };
        self.selection_rects(start..end)
    }

    /// The caret's rectangle at `pos`, `width` logical pixels wide.
    pub fn caret(&self, pos: TextPosition, width: f32) -> Rect {
        let c = Cursor::from_byte_index(&self.layout, pos.index, from_affinity(pos.affinity));
        bbox(c.geometry(&self.layout, width))
    }

    /// Rectangles covering `range`, one or more per line, in visual order.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let anchor = Cursor::from_byte_index(&self.layout, range.start, PAffinity::Downstream);
        let focus = Cursor::from_byte_index(&self.layout, range.end, PAffinity::Upstream);
        Selection::new(anchor, focus).geometry(&self.layout).into_iter().map(|(b, _)| bbox(b)).collect()
    }

    /// The word around `point`, as a byte range.
    pub fn word_at(&self, point: Point) -> Range<usize> {
        Selection::word_from_point(&self.layout, point.x, point.y).text_range()
    }

    /// The position one character to the visual right (or left) of `pos`, as the arrow keys
    /// move in mixed-direction text.
    pub fn move_visual(&self, pos: TextPosition, rightward: bool) -> TextPosition {
        let c = Cursor::from_byte_index(&self.layout, pos.index, from_affinity(pos.affinity));
        let n = if rightward { c.next_visual(&self.layout) } else { c.previous_visual(&self.layout) };
        TextPosition { index: n.index(), affinity: to_affinity(n.affinity()) }
    }

    /// The position one word to the visual right (or left) of `pos`.
    pub fn move_word(&self, pos: TextPosition, rightward: bool) -> TextPosition {
        let c = Cursor::from_byte_index(&self.layout, pos.index, from_affinity(pos.affinity));
        let n =
            if rightward { c.next_visual_word(&self.layout) } else { c.previous_visual_word(&self.layout) };
        TextPosition { index: n.index(), affinity: to_affinity(n.affinity()) }
    }
}
