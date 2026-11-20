//! Fonts, the layout engine and laid-out paragraphs.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use std::collections::HashMap;

use parley::fontique::{Attributes, Blob, Collection, CollectionOptions, FontInfoOverride, SourceCache};
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

/// The families a [`FontCollection`] reaches for a symbol that the chosen font and its
/// script's fallback fonts lack, in order: the bundled Remedy Symbols (`remedy-ui-fonts`),
/// then each platform's symbol, math and emoji fonts, then the system's emoji family. Families
/// the collection does not have are skipped.
///
/// Font fallback goes by script, and symbols such as ✦ ◆ ⚠ ☰ ⌕ ⇄ belong to no script
/// (Unicode's Common script): they take the script of the text around them, whose fallback
/// fonts rarely have them. These families are where they live.
pub const DEFAULT_SYMBOL_FALLBACKS: &[&str] = &[
    "Remedy Symbols",
    // Windows.
    "Segoe UI Symbol",
    "Segoe UI Emoji",
    "Cambria Math",
    // macOS and iOS.
    "Apple Symbols",
    "Apple Color Emoji",
    // Linux and Android.
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "Noto Sans Math",
    "DejaVu Sans",
    "Symbola",
    "Noto Color Emoji",
    // The system's own emoji family.
    "emoji",
];

/// The fonts a [`TextEngine`] can use. Starts empty; apps add their bundled fonts (for
/// example from `remedy-ui-fonts`, or their brand's own as TrueType, OpenType, WOFF or
/// WOFF2) and, with the `system-fonts` feature, may let fallback reach the system's fonts.
///
/// A web font often comes as several files for one family, weight and style, each covering
/// some scripts (Inter's Latin, Cyrillic, Greek and Vietnamese subsets, which CSS joins with
/// `unicode-range`). Added here, they join the same way: text in that family takes each
/// character from whichever part covers it, before falling back to another family.
///
/// When a character still has no glyph after the style's families and its script's
/// fallback fonts (a symbol from Unicode's Common script, typically), layout tries the
/// [symbol fallbacks](Self::set_symbol_fallbacks) for that character.
pub struct FontCollection {
    fcx: FontContext,
    /// The faces added by the app, by family (lowercase): what a further part of a family is
    /// told apart by.
    own: HashMap<String, Vec<Attributes>>,
    /// Further parts of a family (lowercase name) for faces it already had, each registered
    /// under a hidden name of its own and tried, in order, right after the family itself.
    parts: HashMap<String, Vec<String>>,
    /// The families tried for a character no other font had a glyph for.
    symbols: Vec<String>,
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
        FontCollection::over(collection)
    }

    fn over(collection: Collection) -> Self {
        FontCollection {
            fcx: FontContext { collection, source_cache: SourceCache::default() },
            own: HashMap::new(),
            parts: HashMap::new(),
            symbols: DEFAULT_SYMBOL_FALLBACKS.iter().map(|f| (*f).to_string()).collect(),
        }
    }

    /// A collection that falls back to the system's fonts after the ones added to it.
    #[cfg(feature = "system-fonts")]
    pub fn with_system_fonts() -> Self {
        let collection = Collection::new(CollectionOptions { shared: false, system_fonts: true });
        FontCollection::over(collection)
    }

    /// The families tried, in order, for a character that has no glyph in the style's
    /// families or its script's fallback fonts. Starts as [`DEFAULT_SYMBOL_FALLBACKS`].
    pub fn symbol_fallbacks(&self) -> &[String] {
        &self.symbols
    }

    /// Sets the families tried for a character that has no glyph in the style's families or
    /// its script's fallback fonts: family names, or CSS generic names such as `emoji`.
    /// An empty list turns this fallback off, leaving such characters as the font's
    /// missing-glyph box.
    ///
    /// ```
    /// use remedy_ui_text::FontCollection;
    ///
    /// let mut fonts = FontCollection::new();
    /// fonts.set_symbol_fallbacks(["My Icons", "Remedy Symbols"]);
    /// assert_eq!(fonts.symbol_fallbacks(), ["My Icons", "Remedy Symbols"]);
    /// ```
    pub fn set_symbol_fallbacks<I, S>(&mut self, families: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.symbols = families.into_iter().map(Into::into).collect();
    }

    /// Adds every face in a font file (a single font or a collection, TrueType or OpenType,
    /// plain or wrapped as WOFF or WOFF2) from static bytes, returning the family names it
    /// contained. Bytes that are not a font add nothing and return an error.
    pub fn add_static(&mut self, data: &'static [u8]) -> Result<Vec<String>, FontError> {
        match unwrap_web_font(data)? {
            Some(sfnt) => self.add(Blob::new(Arc::new(sfnt))),
            None => self.add(Blob::new(Arc::new(data))),
        }
    }

    /// Adds the faces of a font file read once ([`FontFile`]), sharing its bytes: for fonts
    /// added to every window.
    pub fn add_file(&mut self, file: &FontFile) -> Result<Vec<String>, FontError> {
        self.add(file.data.clone())
    }

    /// Adds every face in a font file from owned bytes (a user's font file), in any format
    /// [`FontCollection::add_static`] takes.
    pub fn add_owned(&mut self, data: Vec<u8>) -> Result<Vec<String>, FontError> {
        match unwrap_web_font(&data)? {
            Some(sfnt) => self.add(Blob::new(Arc::new(sfnt))),
            None => self.add(Blob::new(Arc::new(data))),
        }
    }

    fn add(&mut self, blob: Blob<u8>) -> Result<Vec<String>, FontError> {
        // What the file holds, read on a scratch collection first, so a further part of a
        // family already here (another script's subset) can be told from a new face.
        let mut probe = Collection::new(CollectionOptions { shared: false, system_fonts: false });
        let mut names: Vec<String> = Vec::new();
        let mut faces: Vec<(String, Attributes)> = Vec::new();
        for (family, fonts) in probe.register_fonts(blob.clone(), None) {
            let Some(name) = probe.family_name(family).map(str::to_string) else { continue };
            for f in fonts {
                faces.push((name.clone(), Attributes::new(f.width(), f.style(), f.weight())));
            }
            if !names.contains(&name) {
                names.push(name);
            }
        }
        if faces.is_empty() {
            return Err(FontError("the data is not a font this engine can read".into()));
        }
        let taken = |own: &HashMap<String, Vec<Attributes>>, family: &str| {
            let had = own.get(&family.to_lowercase());
            faces.iter().any(|(_, a)| had.is_some_and(|h| h.contains(a)))
        };
        let target = match names.as_slice() {
            [name] if taken(&self.own, name) => {
                let key = name.to_lowercase();
                let parts = self.parts.get(&key).cloned().unwrap_or_default();
                let free = parts.iter().find(|p| !taken(&self.own, p)).cloned();
                Some(free.unwrap_or_else(|| {
                    // A name no font gives itself: the family's name, a private-use mark and
                    // the part's number.
                    let alias = format!("{name}\u{E000}{}", parts.len() + 1);
                    self.parts.entry(key).or_default().push(alias.clone());
                    alias
                }))
            }
            _ => None,
        };
        let over = target
            .as_deref()
            .map(|family| FontInfoOverride { family_name: Some(family), ..Default::default() });
        if self.fcx.collection.register_fonts(blob, over).is_empty() {
            return Err(FontError("the data is not a font this engine can read".into()));
        }
        for (name, attrs) in faces {
            let family = target.clone().unwrap_or(name);
            self.own.entry(family.to_lowercase()).or_default().push(attrs);
        }
        Ok(names)
    }

    /// Every family in `families` followed by its further parts, as text styles ask for it.
    fn with_parts<'a>(&'a self, families: &'a [String]) -> impl Iterator<Item = &'a str> + 'a {
        families.iter().flat_map(move |f| {
            let parts = if self.parts.is_empty() { None } else { self.parts.get(&f.to_lowercase()) };
            std::iter::once(f.as_str()).chain(parts.into_iter().flatten().map(String::as_str))
        })
    }

    /// Every family name the collection can use.
    pub fn families(&mut self) -> Vec<String> {
        let parts = &self.parts;
        self.fcx
            .collection
            .family_names()
            .filter(|n| !parts.values().flatten().any(|p| p == n))
            .map(str::to_string)
            .collect()
    }
}

/// A font file the app ships (its brand's typeface), read and checked once: TrueType or
/// OpenType, plain or wrapped as WOFF or WOFF2 (unwrapped here, once). Cloning it shares the
/// bytes, so every window can add it ([`FontCollection::add_file`]) without copying or
/// unpacking it again.
#[derive(Clone)]
pub struct FontFile {
    data: Blob<u8>,
    families: Vec<String>,
}

impl std::fmt::Debug for FontFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontFile").field("families", &self.families).field("bytes", &self.data.len()).finish()
    }
}

impl FontFile {
    /// Reads a font file from static bytes (`include_bytes!`). Bytes that are not a font this
    /// engine can read are an error.
    pub fn new(data: &'static [u8]) -> Result<Self, FontError> {
        match unwrap_web_font(data)? {
            Some(sfnt) => Self::check(Blob::new(Arc::new(sfnt))),
            None => Self::check(Blob::new(Arc::new(data))),
        }
    }

    /// Reads a font file from owned bytes.
    pub fn from_vec(data: Vec<u8>) -> Result<Self, FontError> {
        match unwrap_web_font(&data)? {
            Some(sfnt) => Self::check(Blob::new(Arc::new(sfnt))),
            None => Self::check(Blob::new(Arc::new(data))),
        }
    }

    fn check(data: Blob<u8>) -> Result<Self, FontError> {
        let families = FontCollection::new().add(data.clone())?;
        Ok(FontFile { data, families })
    }

    /// The family names its faces belong to.
    pub fn families(&self) -> &[String] {
        &self.families
    }
}

/// The TrueType or OpenType font inside a WOFF or WOFF2 file, or `None` for any other data.
/// The sizes a WOFF file gives are checked against what its compression could produce
/// before anything is unpacked, so a small file cannot claim gigabytes.
fn unwrap_web_font(data: &[u8]) -> Result<Option<Vec<u8>>, FontError> {
    let bad = |what: &str| FontError(format!("the WOFF data is malformed: {what}"));
    match data.get(..4) {
        Some(b"wOFF") => {
            plausible_woff(data).map_err(bad)?;
            wuff::decompress_woff1(data).map(Some).map_err(|e| bad(&format!("{e:?}")))
        }
        // WOFF2's decoder checks its sizes against the data and its compression itself.
        Some(b"wOF2") => wuff::decompress_woff2(data).map(Some).map_err(|e| bad(&format!("{e:?}"))),
        _ => Ok(None),
    }
}

/// Checks a WOFF 1.0 table directory: every table inside the file, no table larger after
/// unpacking than zlib could make it (1032 to 1), and none stored larger than unpacked.
fn plausible_woff(data: &[u8]) -> Result<(), &'static str> {
    /// zlib's largest expansion: a deflate stream cannot unpack to more than this many
    /// bytes per byte.
    const DEFLATE_MAX_RATIO: u64 = 1032;
    let u16_at =
        |at: usize| data.get(at..).and_then(|b| b.first_chunk::<2>()).map(|b| u16::from_be_bytes(*b));
    let u32_at =
        |at: usize| data.get(at..).and_then(|b| b.first_chunk::<4>()).map(|b| u32::from_be_bytes(*b));
    let tables = usize::from(u16_at(12).ok_or("too short for its header")?);
    for t in 0..tables {
        let entry = 44 + t * 20;
        let (Some(offset), Some(stored), Some(unpacked)) =
            (u32_at(entry + 4), u32_at(entry + 8), u32_at(entry + 12))
        else {
            return Err("its table directory runs past the end");
        };
        let (offset, stored, unpacked) = (u64::from(offset), u64::from(stored), u64::from(unpacked));
        if offset + stored > data.len() as u64 {
            return Err("a table runs past the end");
        }
        if stored > unpacked {
            return Err("a table is stored larger than it unpacks");
        }
        if unpacked > stored.saturating_mul(DEFLATE_MAX_RATIO) {
            return Err("a table claims more than its compressed data can hold");
        }
    }
    Ok(())
}

impl FontCollection {
    /// The family names a style's stack resolves to, further parts included.
    fn stack(&self, families: &[String]) -> Vec<FontFamilyName<'static>> {
        // CSS's generic names (`monospace`, `serif`, `sans-serif`, `system-ui`,
        // `ui-monospace`, …) stand for the system's own choice; anything else names a family.
        self.with_parts(families)
            .map(|f| match parley::fontique::GenericFamily::parse(f) {
                Some(g) => FontFamilyName::Generic(g),
                None => FontFamilyName::Named(Cow::Owned(f.to_owned())),
            })
            .collect()
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
#[non_exhaustive]
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
    /// Device pixels per logical pixel. The paragraph is shaped at this scale (hinting and
    /// rounding see device pixels) but every size and position it reports stays in logical
    /// pixels; the scale changes only how they fall on the device's pixel grid.
    pub scale: f32,
    /// Round line metrics to whole device pixels, so hinted text sits on the pixel grid.
    pub snap: bool,
}

impl ParagraphOptions {
    /// Sets `max_width`: wrap width in logical pixels; `None` keeps each paragraph on one line.
    pub fn max_width(mut self, value: f32) -> Self {
        self.max_width = Some(value);
        self
    }

    /// Sets `align`: line alignment.
    pub fn align(mut self, value: Align) -> Self {
        self.align = value;
        self
    }

    /// Sets `direction`: paragraph direction.
    pub fn direction(mut self, value: Direction) -> Self {
        self.direction = value;
        self
    }

    /// Sets `container`: the direction of the UI around the paragraph.
    pub fn container(mut self, value: Direction) -> Self {
        self.container = value;
        self
    }

    /// Sets `scale`: device pixels per logical pixel.
    pub fn scale(mut self, value: f32) -> Self {
        self.scale = value;
        self
    }

    /// Sets `snap`: round line metrics to whole device pixels, so hinted text sits on the pixel grid.
    pub fn snap(mut self, value: bool) -> Self {
        self.snap = value;
        self
    }
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
    ///
    /// Characters that come out without a glyph (a symbol none of the style's families or
    /// its script's fallback fonts have) are laid out once more with the collection's
    /// [symbol fallbacks](FontCollection::set_symbol_fallbacks) after the style's families,
    /// for those characters only.
    pub fn layout(&mut self, text: &RichText, base: &TextStyle, options: ParagraphOptions) -> Paragraph {
        let segments = flatten(text, base);
        let first = self.build(text, base, &segments, options, &[]);
        if self.fonts.symbols.is_empty() {
            return first;
        }
        let missing = first.missing_glyphs(text.as_str());
        if missing.is_empty() {
            return first;
        }
        let symbols = family_names(self.fonts.symbols.iter().map(String::as_str));
        let overrides: Vec<(Range<usize>, Vec<FontFamilyName<'static>>)> = missing
            .into_iter()
            .map(|range| {
                let style =
                    segments.iter().find(|s| s.range.contains(&range.start)).map_or(base, |s| &s.style);
                let mut families = self.fonts.stack(&style.families);
                for f in &symbols {
                    if !families.contains(f) {
                        families.push(f.clone());
                    }
                }
                (range, families)
            })
            .collect();
        self.build(text, base, &segments, options, &overrides)
    }

    /// Shapes and lays out once; `families` replaces the family list on its ranges.
    fn build(
        &mut self,
        text: &RichText,
        base: &TextStyle,
        segments: &[Segment],
        options: ParagraphOptions,
        families: &[(Range<usize>, Vec<FontFamilyName<'static>>)],
    ) -> Paragraph {
        let mut run_styles = Vec::with_capacity(segments.len() + 1);
        let content = text.as_str();
        let base_stack = self.fonts.stack(&base.families);
        let stacks: Vec<_> = segments.iter().map(|seg| self.fonts.stack(&seg.style.families)).collect();
        // Parley shapes at the device scale: its sizes and positions come out in device
        // pixels, and the paragraph turns them back into logical pixels at its edges.
        let scale = device_scale(options.scale);
        let mut b = self.lcx.ranged_builder(&mut self.fonts.fcx, content, scale, options.snap);
        b.set_base_direction(match options.direction {
            Direction::Auto => BaseDirection::Auto,
            Direction::Ltr => BaseDirection::Ltr,
            Direction::Rtl => BaseDirection::Rtl,
        });
        push_style(&mut b, base, base_stack, None, 0);
        run_styles.push(RunStyle { color: base.color, link: None, range: 0..content.len() });
        for ((i, seg), stack) in segments.iter().enumerate().zip(stacks) {
            let brush = u32::try_from(i + 1).unwrap_or(u32::MAX);
            push_style(&mut b, &seg.style, stack, Some(seg.range.clone()), brush);
            run_styles.push(RunStyle { color: seg.style.color, link: seg.link, range: seg.range.clone() });
        }
        // Pushed after the spans, so they win on their ranges; every other property stays.
        for (range, list) in families {
            b.push(StyleProperty::FontFamily(FontFamily::List(Cow::Owned(list.clone()))), range.clone());
        }
        for &(at, obj) in text.objects() {
            b.push_inline_box(InlineBox {
                id: obj.id,
                kind: InlineBoxKind::InFlow,
                index: at,
                width: obj.width * scale,
                height: obj.height * scale,
                baseline: obj.baseline.map(|b| b * scale),
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
        layout.break_all_lines(options.max_width.map(|w| w * scale));
        Paragraph { layout, run_styles, align: alignment, wrap: options.max_width, scale }
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

/// Family names for parley. CSS's generic names (`monospace`, `serif`, `sans-serif`,
/// `system-ui`, `ui-monospace`, `emoji`, …) stand for the system's own choice; anything else
/// names a family.
fn family_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<FontFamilyName<'static>> {
    names
        .map(|f| match parley::fontique::GenericFamily::parse(f) {
            Some(g) => FontFamilyName::Generic(g),
            None => FontFamilyName::Named(Cow::Owned(f.to_string())),
        })
        .collect()
}

/// True for a character that is drawn: not white space, a control or a default-ignorable
/// code point (joiners, variation selectors, bidi controls, tags…), which fonts need not map.
fn is_drawn(c: char) -> bool {
    !(c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '\u{AD}'
                | '\u{34F}'
                | '\u{61C}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFF8}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        ))
}

fn push_style(
    b: &mut parley::RangedBuilder<'_, u32>,
    s: &TextStyle,
    families: Vec<FontFamilyName<'static>>,
    range: Option<Range<usize>>,
    brush: u32,
) {
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
        // A word longer than its line (an invite, a link, a key) breaks onto the next line
        // instead of running past the box it is in; text with spaces wraps as before.
        StyleProperty::OverflowWrap(parley::OverflowWrap::BreakWord),
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
// Exhaustive on purpose: a caret sits before or after a line break, nothing else.
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
#[non_exhaustive]
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
#[non_exhaustive]
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

/// A laid-out paragraph: lines, glyph runs, inline objects, and caret geometry, all in
/// logical pixels whatever scale it was shaped at.
pub struct Paragraph {
    /// Shaped at `scale`: its sizes and positions are in device pixels.
    layout: Layout<u32>,
    run_styles: Vec<RunStyle>,
    align: Alignment,
    /// The wrap width lines were aligned within, in logical pixels (`None`: aligned within
    /// the longest line).
    wrap: Option<f32>,
    /// Device pixels per logical pixel the layout was shaped at.
    scale: f32,
}

/// A usable device scale: positive and finite, else 1.
fn device_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
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

/// A box of the layout (device pixels at `scale`) in logical pixels.
fn bbox(b: parley::BoundingBox, scale: f32) -> Rect {
    let s = f64::from(scale);
    Rect::new((b.x0 / s) as f32, (b.y0 / s) as f32, (b.x1 / s) as f32, (b.y1 / s) as f32)
}

impl Paragraph {
    /// The byte ranges of `text`'s clusters (this paragraph's text) that are drawn but came
    /// out with the font's missing-glyph box (glyph 0), in order.
    fn missing_glyphs(&self, text: &str) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for line in self.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                for cluster in run.run().clusters() {
                    let range = cluster.text_range();
                    if cluster.glyphs().any(|g| g.id == 0)
                        && text.get(range.clone()).is_some_and(|s| s.chars().any(is_drawn))
                    {
                        out.push(range);
                    }
                }
            }
        }
        // A run broken across lines is visited once per line.
        out.sort_by_key(|r| (r.start, r.end));
        out.dedup();
        out
    }

    /// Lays the paragraph out again at a new width, without shaping again.
    pub fn rewrap(&mut self, max_width: Option<f32>) {
        self.layout.set_alignment(self.align, AlignmentOptions::default());
        self.layout.break_all_lines(max_width.map(|w| w * self.scale));
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
        self.layout.width() / self.scale
    }

    /// Total height.
    pub fn height(&self) -> f32 {
        self.layout.height() / self.scale
    }

    /// True when the paragraph's base direction is right to left.
    pub fn is_rtl(&self) -> bool {
        self.layout.is_rtl()
    }

    /// The first line's baseline, from the top.
    pub fn first_baseline(&self) -> Option<f32> {
        self.layout.first_baseline().map(|b| b / self.scale)
    }

    /// The lines, top to bottom.
    pub fn lines(&self) -> Vec<LineInfo> {
        self.layout
            .lines()
            .map(|line| {
                let m = line.metrics();
                let s = self.scale;
                LineInfo {
                    range: line.text_range(),
                    baseline: m.baseline / s,
                    top: m.block_min_coord / s,
                    bottom: m.block_max_coord / s,
                    width: m.advance / s,
                    left: m.offset / s,
                }
            })
            .collect()
    }

    /// Glyph runs in drawing order, with positions from the paragraph's top-left.
    pub fn glyph_runs(&self) -> Vec<GlyphRunInfo> {
        let s = self.scale;
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
                // the size asks (at least a device pixel), placed from the span's metrics.
                // Everything here is in the layout's device pixels until the run is made.
                let size = r.font_size();
                let metrics = run.span_metrics();
                let thick = (size / 14.0).max(1.0);
                let (x, baseline, width) = (run.offset(), run.baseline(), run.advance());
                let bar = |offset: f32, d: &parley::layout::Decoration<u32>| {
                    let t = d.size.unwrap_or(thick);
                    let y = baseline - d.offset.unwrap_or(offset);
                    Rect::new(x / s, (y - t / 2.0) / s, (x + width) / s, (y + t / 2.0) / s)
                };
                let below = -(metrics.descent.abs() * 0.4).max(thick);
                let middle = if metrics.x_height > 0.0 { metrics.x_height / 2.0 } else { size * 0.25 };
                let underline = style.underline.as_ref().map(|d| bar(below, d));
                let strikethrough = style.strikethrough.as_ref().map(|d| bar(middle, d));
                out.push(GlyphRunInfo {
                    font: RunFont { data: font.data.clone(), index: font.index },
                    size: r.font_size() / s,
                    color: rs.color,
                    link: rs.link,
                    rtl: r.is_rtl(),
                    range,
                    glyphs: run
                        .positioned_glyphs()
                        .map(|g| PositionedGlyph { id: g.id, x: g.x / s, y: g.y / s })
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
                    let s = self.scale;
                    out.push((b.id, Rect::new(b.x / s, b.y / s, (b.x + b.width) / s, (b.y + b.height) / s)));
                }
            }
        }
        out
    }

    /// The text position nearest `point` (from the paragraph's top-left).
    pub fn hit_test(&self, point: Point) -> TextPosition {
        let c = Cursor::from_point(&self.layout, point.x * self.scale, point.y * self.scale);
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
        bbox(c.geometry(&self.layout, width * self.scale), self.scale)
    }

    /// Rectangles covering `range`, one or more per line, in visual order.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let anchor = Cursor::from_byte_index(&self.layout, range.start, PAffinity::Downstream);
        let focus = Cursor::from_byte_index(&self.layout, range.end, PAffinity::Upstream);
        Selection::new(anchor, focus)
            .geometry(&self.layout)
            .into_iter()
            .map(|(b, _)| bbox(b, self.scale))
            .collect()
    }

    /// The word around `point`, as a byte range.
    pub fn word_at(&self, point: Point) -> Range<usize> {
        Selection::word_from_point(&self.layout, point.x * self.scale, point.y * self.scale).text_range()
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
