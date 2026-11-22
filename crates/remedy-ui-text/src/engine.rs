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
#[derive(Clone)]
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
    /// Families added as script fallbacks, which go before the system's, in order.
    own_ids: Vec<parley::fontique::FamilyId>,
    /// Each script (and language) given fallbacks: its own families, and the system's.
    own_keys: Vec<OwnFallback>,
    /// Faces added when text first needs them ([`FontCollection::add_on_demand`]).
    on_demand: Vec<OnDemand>,
}

/// A script and language, the bundled families registered for it, and the system's.
type OwnFallback = (
    parley::fontique::Script,
    Option<parley::fontique::Language>,
    Vec<parley::fontique::FamilyId>,
    Vec<parley::fontique::FamilyId>,
);

/// Faces a collection adds when text first needs them.
#[derive(Clone)]
struct OnDemand {
    needed: fn(char) -> bool,
    load: Arc<dyn Fn() -> Vec<FontFile> + Send + Sync>,
    fallbacks: Vec<(String, Option<String>, String)>,
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
            own_ids: Vec::new(),
            own_keys: Vec::new(),
            on_demand: Vec::new(),
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

    /// Makes `family` a fallback for text in `script` (an ISO 15924 code such as `Arab`,
    /// `Deva`, `Beng`, `Hani`, `Hira`, `Kana`), after the style's own families: what the
    /// collection draws an Arabic word with when the interface face has no Arabic. With
    /// `locale` (`ja`, `zh-CN`), the fallback is only for text in that language, which is
    /// how Japanese and Chinese text pick their own forms of the characters they share;
    /// without, it is the script's default. Fallbacks added here are tried in the order they
    /// were added, before any the system lists for the script (with `system-fonts`): a face
    /// the app ships is the one it was checked with. Returns false for a family the collection does not have, or a script or locale the
    /// fallback table does not know.
    ///
    /// ```
    /// # use remedy_ui_text::FontCollection;
    /// let mut fonts = FontCollection::new();
    /// assert!(!fonts.add_script_fallback("Arab", None, "Noto Sans Arabic"), "not added yet");
    /// ```
    pub fn add_script_fallback(&mut self, script: &str, locale: Option<&str>, family: &str) -> bool {
        let Ok(script) = parley::fontique::Script::parse(script) else { return false };
        let Some(id) = self.fcx.collection.family_id(family) else { return false };
        let language = match locale.map(parley::fontique::Language::parse) {
            Some(Ok(l)) => Some(l),
            Some(Err(_)) => return false,
            None => None,
        };
        let key = parley::fontique::FallbackKey::new(script, language.as_ref());
        if !self.own_ids.contains(&id) {
            self.own_ids.push(id);
        }
        match self.own_keys.iter_mut().find(|k| k.0 == script && k.1 == language) {
            Some(k) => {
                if !k.2.contains(&id) {
                    k.2.push(id);
                }
            }
            None => {
                // What the system lists for the script, kept after the bundled faces.
                let system: Vec<_> = self
                    .fcx
                    .collection
                    .fallback_families(key)
                    .filter(|f| !self.own_ids.contains(f))
                    .collect();
                self.own_keys.push((script, language, vec![id], system));
            }
        }
        // Every script's list: its own bundled faces, then every other bundled fallback face
        // (punctuation and symbols shared by scripts ride with whatever run they are in: a
        // Japanese bracket beside an Arabic name finds the Japanese face), then the system's.
        // A face the app ships is the one it was checked with.
        let mut ok = true;
        for (script, language, mine, system) in &self.own_keys {
            let key = parley::fontique::FallbackKey::new(*script, language.as_ref());
            let list: Vec<_> = mine
                .iter()
                .chain(self.own_ids.iter().filter(|f| !mine.contains(f)))
                .chain(system.iter())
                .copied()
                .collect();
            ok &= self.fcx.collection.set_fallbacks(key, list.into_iter());
        }
        ok
    }

    /// Adds faces only when text first needs them: the first paragraph with a character
    /// `needed` says yes to loads them (`load`, which should share one unpacked copy per
    /// process, so later windows only register it) and makes them fallbacks for scripts as
    /// [`FontCollection::add_script_fallback`] does, in `fallbacks`' order. For large faces
    /// most windows never use: an app that never shows Chinese never unpacks a Chinese font.
    pub fn add_on_demand<'a>(
        &mut self,
        needed: fn(char) -> bool,
        load: impl Fn() -> Vec<FontFile> + Send + Sync + 'static,
        fallbacks: impl IntoIterator<Item = (&'a str, Option<&'a str>, &'a str)>,
    ) {
        self.on_demand.push(OnDemand {
            needed,
            load: Arc::new(load),
            fallbacks: fallbacks
                .into_iter()
                .map(|(s, l, f)| (s.to_owned(), l.map(str::to_owned), f.to_owned()))
                .collect(),
        });
    }

    /// Loads the on-demand faces `text` needs. Returns whether any were added.
    pub(crate) fn load_needed(&mut self, text: &str) -> bool {
        if self.on_demand.is_empty() {
            return false;
        }
        let (now, later): (Vec<OnDemand>, Vec<OnDemand>) =
            std::mem::take(&mut self.on_demand).into_iter().partition(|d| text.chars().any(d.needed));
        self.on_demand = later;
        for d in &now {
            for file in (d.load)() {
                let _ = self.add_file(&file);
            }
            for (script, locale, family) in &d.fallbacks {
                let _ = self.add_script_fallback(script, locale.as_deref(), family);
            }
        }
        !now.is_empty()
    }

    /// Whether faces are waiting to be loaded on demand ([`FontCollection::add_on_demand`]).
    pub fn has_on_demand(&self) -> bool {
        !self.on_demand.is_empty()
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
    /// The language text is in, unless a span says otherwise ([`TextEngine::set_locale`]).
    language: Option<parley::fontique::Language>,
    /// The casing rules of the locale ([`TextTransform`](crate::TextTransform)).
    case_lang: crate::casing::Lang,
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
    /// At most this many lines (CSS `line-clamp`): text that would take more ends the last
    /// one with "…" ([`TextEngine::layout`]). `None` for no limit.
    pub max_lines: Option<usize>,
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

    /// Sets `max_lines`: at most this many lines, the last ending in "…" when the text is cut.
    pub fn max_lines(mut self, value: usize) -> Self {
        self.max_lines = Some(value);
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
            max_lines: None,
        }
    }
}

impl TextEngine {
    /// An engine using `fonts`.
    pub fn new(fonts: FontCollection) -> Self {
        TextEngine {
            fonts,
            lcx: LayoutContext::new(),
            language: None,
            case_lang: crate::casing::Lang::Default,
        }
    }

    /// The language text is in (a BCP 47 tag such as `ja` or `zh-Hans`; empty or invalid
    /// for none): it chooses between fonts for characters several languages share (Japanese
    /// and Chinese forms of the same character) and language-specific shaping (Urdu's and
    /// Persian's forms of Arabic letters, Serbian Cyrillic). A window sets it from its locale.
    /// Returns whether it changed (paragraphs laid out before should be laid out again).
    pub fn set_locale(&mut self, locale: &str) -> bool {
        let language = parley::fontique::Language::parse(locale).ok();
        let case_lang = crate::casing::Lang::for_locale(locale);
        let changed = language != self.language || case_lang != self.case_lang;
        self.language = language;
        self.case_lang = case_lang;
        changed
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
    ///
    /// A huge paragraph that wraps (at least [`LAZY_MIN_BYTES`](crate::LAZY_MIN_BYTES) and a
    /// finite `max_width`) is laid out lazily: only the lines asked for (those a view shows,
    /// and those its caret, selection and pointer reach) are shaped, a piece at a time, with
    /// exactly the lines, glyphs and carets a whole layout gives. See [`Paragraph`].
    ///
    /// A [`TextTransform`](crate::TextTransform) in `base` or a span is applied here: the
    /// cased text is shaped, and the paragraph's byte offsets (hit tests, carets, selections,
    /// lines and runs) are those of `text` as written.
    ///
    /// With [`ParagraphOptions::max_lines`], text that would take more lines is cut at the
    /// last grapheme boundary that leaves room for "…" on the last line. The "…" follows the
    /// text in logical order, so it lands on the line's visual end in either direction; the
    /// paragraph's offsets stay those of `text` (the cut is a prefix of it).
    pub fn layout(&mut self, text: &RichText, base: &TextStyle, options: ParagraphOptions) -> Paragraph {
        if let Some(n) = options.max_lines {
            return self.clamped(text, base, ParagraphOptions { max_lines: None, ..options }, n.max(1));
        }
        if let Some((cased, map)) = crate::casing::apply(text, base, self.case_lang) {
            let mut p = self.layout_uncased(&cased, base, options);
            p.case = Some(map);
            return p;
        }
        self.layout_uncased(text, base, options)
    }

    /// `text` in at most `n` lines, the last ending in "…" when it is cut.
    fn clamped(
        &mut self,
        text: &RichText,
        base: &TextStyle,
        options: ParagraphOptions,
        n: usize,
    ) -> Paragraph {
        let s = text.as_str();
        // The start of `text` up to `end` (white space after it dropped when `trim`), then
        // `suffix`, keeping the spans and objects there.
        let head = |end: usize, trim: bool, suffix: &str| {
            let kept = s.get(..end).unwrap_or(s);
            let kept = if trim { kept.trim_end() } else { kept };
            let keep = kept.len();
            let mut r = RichText::plain(format!("{kept}{suffix}"));
            for span in text.spans() {
                let (a, b) = (span.range.start.min(keep), span.range.end.min(keep));
                if a < b {
                    r.add_span(a..b, span.patch.clone());
                }
            }
            r.set_objects(text.objects().iter().filter(|(i, _)| *i < keep).copied().collect());
            r
        };
        let cut = |end: usize| head(end, true, "\u{2026}");
        // Lines break greedily, so the lines of a start of the text that reaches past line
        // n + 1 are the whole text's up to there: lay out starts growing fourfold, not the
        // whole of a long text.
        let mut budget = 256 * (n + 2);
        let (full, lines) = loop {
            if budget >= s.len() {
                let p = self.layout(text, base, options);
                let l = p.lines();
                break (p, l);
            }
            let end = (0..=budget).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
            let p = self.layout(&head(end, false, ""), base, options);
            let l = p.lines();
            if l.len() > n + 1 {
                break (p, l);
            }
            budget = budget.saturating_mul(4);
        };
        let Some(last) = lines.get(n - 1).filter(|_| lines.len() > n) else { return full };
        // Grapheme boundaries on the last visible line: a cut never splits a cluster. Text
        // past that line cannot fit beside the "…" (it was pushed to the next line already).
        let line = last.range.start..last.range.end.min(s.len());
        let ends: Vec<usize> = icu_segmenter::GraphemeClusterSegmenter::new()
            .segment_str(s.get(line.clone()).unwrap_or(""))
            .map(|b| b + line.start)
            .filter(|&b| b > line.start)
            .collect();
        // Binary search for the last end that still fits in `n` lines.
        let (mut lo, mut hi) = (0usize, ends.len());
        let mut best = None;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let Some(&end) = ends.get(mid) else { break };
            let p = self.layout(&cut(end), base, options);
            if p.lines().len() <= n {
                best = Some(p);
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        best.unwrap_or_else(|| self.layout(&cut(line.start), base, options))
    }

    fn layout_uncased(&mut self, text: &RichText, base: &TextStyle, options: ParagraphOptions) -> Paragraph {
        self.fonts.load_needed(text.as_str());
        if crate::lazy::eligible(text, &options) {
            return Paragraph::lazy(crate::lazy::Lazy::new(self.snapshot(), text, base, options));
        }
        Paragraph::whole(self.layout_piece(text, base, options, 0.0))
    }

    /// Lays out `text` whole, however long it is: every line shaped now, as
    /// [`TextEngine::layout`] does for paragraphs of ordinary length.
    pub fn layout_whole(
        &mut self,
        text: &RichText,
        base: &TextStyle,
        options: ParagraphOptions,
    ) -> Paragraph {
        let cased = crate::casing::apply(text, base, self.case_lang);
        let shown = cased.as_ref().map_or(text, |(c, _)| c);
        self.fonts.load_needed(shown.as_str());
        let mut p = Paragraph::whole(self.layout_piece(shown, base, options, 0.0));
        p.case = cased.map(|(_, m)| m);
        p
    }

    /// A copy of the engine for one lazy paragraph to lay its pieces out with later: the same
    /// fonts (sharing their data) and language, its own scratch space.
    fn snapshot(&self) -> TextEngine {
        TextEngine {
            fonts: self.fonts.clone(),
            lcx: LayoutContext::new(),
            language: self.language,
            case_lang: self.case_lang,
        }
    }

    /// Shapes and lays out all of `text` (fonts already loaded for it), its first line's top
    /// at `y0` device pixels before rounding (where the lines before it would leave it).
    pub(crate) fn layout_piece(
        &mut self,
        text: &RichText,
        base: &TextStyle,
        options: ParagraphOptions,
        y0: f64,
    ) -> Piece {
        let segments = flatten(text, base);
        let first = self.build(text, base, &segments, options, &[], y0);
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
        self.build(text, base, &segments, options, &overrides, y0)
    }

    /// Lays out `text`, the edited text of `before` (laid out with the same style and
    /// options). Gives the same paragraph as [`TextEngine::layout`]; for a huge paragraph laid
    /// out lazily, the lines before the edit and those after it (once their line breaks agree
    /// again) are kept rather than laid out again, so typing costs about one piece.
    pub fn layout_after_edit(
        &mut self,
        before: &Paragraph,
        text: &RichText,
        base: &TextStyle,
        options: ParagraphOptions,
    ) -> Paragraph {
        if before.case.is_none()
            && crate::casing::apply(text, base, self.case_lang).is_none()
            && let Some(p) = crate::lazy::Lazy::after_edit(&before.inner, text, base, options)
        {
            return Paragraph::lazy(p);
        }
        self.layout(text, base, options)
    }

    /// Shapes and lays out once; `families` replaces the family list on its ranges.
    fn build(
        &mut self,
        text: &RichText,
        base: &TextStyle,
        segments: &[Segment],
        options: ParagraphOptions,
        families: &[(Range<usize>, Vec<FontFamilyName<'static>>)],
        y0: f64,
    ) -> Piece {
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
        b.push_default(StyleProperty::Locale(self.language));
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
        break_lines(&mut layout, options.max_width.map(|w| w * scale), y0);
        Piece { layout, run_styles, align: alignment, wrap: options.max_width, scale, y0 }
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
    /// Each glyph's advance in logical pixels, parallel to `glyphs`, as the shaper gave it
    /// (letter spacing included). A mark's is usually zero; its `x` carries its offset.
    /// Empty, with `clusters`, in the rare run whose shaped cluster is split between styles.
    pub advances: Vec<f32>,
    /// The byte offset into the paragraph's text where each glyph's cluster starts, parallel
    /// to `glyphs`: the glyphs of a ligature or of a letter with its marks share one, as in
    /// HarfBuzz's cluster values.
    pub clusters: Vec<usize>,
    /// The underline to draw under the run, from the paragraph's top-left, if it has one.
    pub underline: Option<Rect>,
    /// The line through the run, if it has one.
    pub strikethrough: Option<Rect>,
}

/// One laid-out piece of text: a whole paragraph, or one piece of a lazy one (in the piece's
/// own coordinates: byte 0 is its first byte, y 0 the top of its first line).
pub(crate) struct Piece {
    /// Shaped at `scale`: its sizes and positions are in device pixels.
    layout: Layout<u32>,
    run_styles: Vec<RunStyle>,
    align: Alignment,
    /// The wrap width lines were aligned within, in logical pixels (`None`: aligned within
    /// the longest line).
    wrap: Option<f32>,
    /// Device pixels per logical pixel the layout was shaped at.
    pub(crate) scale: f32,
    /// Where its first line's top is, in device pixels before rounding.
    y0: f64,
}

/// Breaks `layout`'s lines at `max_advance`, its first line's top at `y0`: parley rounds each
/// line's top from the sum of the unrounded heights above it, so a piece laid out from the
/// sum the lines before it reach is placed exactly where a whole layout places it.
fn break_lines(layout: &mut Layout<u32>, max_advance: Option<f32>, y0: f64) {
    let mut lines = layout.break_lines();
    lines.state_mut().set_line_y(y0);
    lines.break_remaining(max_advance.unwrap_or(f32::MAX));
}

/// A usable device scale: positive and finite, else 1.
fn device_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
}

impl std::fmt::Debug for Paragraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paragraph")
            .field("width", &self.width())
            .field("height", &self.height())
            .field("lazy", &matches!(self.inner, Inner::Lazy(_)))
            .finish()
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

impl Piece {
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
        break_lines(&mut self.layout, max_width.map(|w| w * self.scale), self.y0);
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
        (0..self.layout.len()).filter_map(|i| self.line(i)).collect()
    }

    /// Glyph runs in drawing order, with positions from the paragraph's top-left.
    pub fn glyph_runs(&self) -> Vec<GlyphRunInfo> {
        self.runs_of(0..self.layout.len())
    }

    /// Line `i`'s height before rounding, in device pixels: what the next line's top is
    /// reached by.
    pub(crate) fn line_height(&self, i: usize) -> f32 {
        self.layout.get(i).map_or(0.0, |l| l.metrics().line_height)
    }

    /// What line `i` adds to the paragraph's width, in device pixels: as parley measures the
    /// widest line (its hanging white space left out).
    pub(crate) fn line_extent(&self, i: usize) -> f32 {
        self.layout.get(i).map_or(0.0, |l| {
            let m = l.metrics();
            m.inline_min_coord + m.advance + m.indent.max(0.0) - m.hanging_advance
        })
    }

    /// The number of lines.
    pub(crate) fn line_count(&self) -> usize {
        self.layout.len()
    }

    /// Line `i`, if there is one.
    pub(crate) fn line(&self, i: usize) -> Option<LineInfo> {
        let line = self.layout.get(i)?;
        let m = line.metrics();
        let s = self.scale;
        Some(LineInfo {
            range: line.text_range(),
            baseline: m.baseline / s,
            top: m.block_min_coord / s,
            bottom: m.block_max_coord / s,
            width: m.advance / s,
            left: m.offset / s,
        })
    }

    /// The glyph runs of lines `lines`, in drawing order.
    pub(crate) fn runs_of(&self, lines: Range<usize>) -> Vec<GlyphRunInfo> {
        let s = self.scale;
        let mut out = Vec::new();
        for line in self.layout.lines().skip(lines.start).take(lines.len()) {
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
                let glyphs: Vec<PositionedGlyph> = run
                    .positioned_glyphs()
                    .map(|g| PositionedGlyph { id: g.id, x: g.x / s, y: g.y / s })
                    .collect();
                // Clusters in visual order carry their glyphs, in the same order the run
                // positions them; only those of this glyph run's text belong to it.
                let mut advances = Vec::with_capacity(glyphs.len());
                let mut clusters = Vec::with_capacity(glyphs.len());
                for c in r.visual_clusters() {
                    let start = c.text_range().start;
                    if !range.contains(&start) {
                        continue;
                    }
                    for g in c.glyphs() {
                        advances.push(g.advance / s);
                        clusters.push(start);
                    }
                }
                if advances.len() != glyphs.len() {
                    // A cluster split across styles; the per-glyph detail is not reliable.
                    advances.clear();
                    clusters.clear();
                }
                out.push(GlyphRunInfo {
                    font: RunFont { data: font.data.clone(), index: font.index },
                    size: r.font_size() / s,
                    color: rs.color,
                    link: rs.link,
                    rtl: r.is_rtl(),
                    range,
                    glyphs,
                    advances,
                    clusters,
                    underline,
                    strikethrough,
                });
            }
        }
        out
    }

    /// Inline objects with their boxes, by the ids the caller gave them.
    pub fn inline_objects(&self) -> Vec<(u64, Rect)> {
        self.objects_of(0..self.layout.len())
    }

    /// The inline objects on lines `lines`.
    pub(crate) fn objects_of(&self, lines: Range<usize>) -> Vec<(u64, Rect)> {
        let mut out = Vec::new();
        for line in self.layout.lines().skip(lines.start).take(lines.len()) {
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

/// A laid-out paragraph: lines, glyph runs, inline objects, and caret geometry, all in
/// logical pixels whatever scale it was shaped at.
///
/// A huge paragraph that wraps is laid out lazily ([`TextEngine::layout`]): its lines are
/// shaped a piece at a time, when something asks for them. Asking for the lines, glyphs or
/// objects in a band ([`Paragraph::lines_in`], [`Paragraph::glyph_runs_in`],
/// [`Paragraph::inline_objects_in`]), or for a caret, selection, hit test or movement, lays
/// out the pieces it reaches and no more; [`Paragraph::lines`], [`Paragraph::glyph_runs`]
/// and [`Paragraph::inline_objects`] lay out everything. Whatever has been laid out is
/// exactly what a whole layout ([`TextEngine::layout_whole`]) gives. Until the last line has
/// been laid out, [`Paragraph::height`] adds an estimate for the lines not laid out yet
/// (from the lines that have been), and [`Paragraph::width`] is the widest line so far;
/// [`Paragraph::is_complete`] says whether they are exact.
pub struct Paragraph {
    pub(crate) inner: Inner,
    /// Set when the text was cased before shaping: maps the shaped text's offsets to the
    /// text as written.
    pub(crate) case: Option<crate::casing::CaseMap>,
}

pub(crate) enum Inner {
    Whole(Piece),
    Lazy(crate::lazy::Lazy),
}

impl Paragraph {
    pub(crate) fn whole(piece: Piece) -> Self {
        Paragraph { inner: Inner::Whole(piece), case: None }
    }

    pub(crate) fn lazy(lazy: crate::lazy::Lazy) -> Self {
        Paragraph { inner: Inner::Lazy(lazy), case: None }
    }

    /// Lays the paragraph out again at a new width, without shaping again (a lazy paragraph
    /// starts over at the new width, shaping only what is asked for again).
    pub fn rewrap(&mut self, max_width: Option<f32>) {
        match &mut self.inner {
            Inner::Whole(p) => p.rewrap(max_width),
            Inner::Lazy(l) => {
                if let Some(whole) = l.rewrap(max_width) {
                    self.inner = Inner::Whole(whole);
                }
            }
        }
    }

    /// True when it is laid out lazily, a piece at a time.
    pub fn is_lazy(&self) -> bool {
        matches!(self.inner, Inner::Lazy(_))
    }

    /// True when every line has been laid out, so [`Paragraph::height`] and
    /// [`Paragraph::width`] are exact (always, for a paragraph laid out whole).
    pub fn is_complete(&self) -> bool {
        match &self.inner {
            Inner::Whole(_) => true,
            Inner::Lazy(l) => l.is_complete(),
        }
    }

    /// Where to draw the paragraph inside a box `box_width` wide so it sits as aligned.
    /// Wrapped paragraphs are already aligned within their wrap width (0); one laid out
    /// without one is only as wide as its longest line, so it moves to the box's right edge
    /// or center as its alignment says.
    pub fn x_offset(&self, box_width: f32) -> f32 {
        match &self.inner {
            Inner::Whole(p) => p.x_offset(box_width),
            // Lazy paragraphs always wrap.
            Inner::Lazy(_) => 0.0,
        }
    }

    /// Width of the widest line (of those laid out so far, for a lazy paragraph).
    pub fn width(&self) -> f32 {
        match &self.inner {
            Inner::Whole(p) => p.width(),
            Inner::Lazy(l) => l.width(),
        }
    }

    /// Total height (for a lazy paragraph not yet complete, the lines laid out and an estimate
    /// for the rest).
    pub fn height(&self) -> f32 {
        match &self.inner {
            Inner::Whole(p) => p.height(),
            Inner::Lazy(l) => l.height(),
        }
    }

    /// True when the paragraph's base direction is right to left.
    pub fn is_rtl(&self) -> bool {
        match &self.inner {
            Inner::Whole(p) => p.is_rtl(),
            Inner::Lazy(l) => l.is_rtl(),
        }
    }

    /// The first line's baseline, from the top.
    pub fn first_baseline(&self) -> Option<f32> {
        match &self.inner {
            Inner::Whole(p) => p.first_baseline(),
            Inner::Lazy(l) => l.first_baseline(),
        }
    }

    /// The lines, top to bottom (all of them: a lazy paragraph lays out the rest).
    pub fn lines(&self) -> Vec<LineInfo> {
        let lines = match &self.inner {
            Inner::Whole(p) => p.lines(),
            Inner::Lazy(l) => l.lines_in(f32::NEG_INFINITY..f32::INFINITY),
        };
        self.uncase_lines(lines)
    }

    /// The lines that reach into `ys` (from the paragraph's top), top to bottom.
    pub fn lines_in(&self, ys: Range<f32>) -> Vec<LineInfo> {
        let lines = match &self.inner {
            Inner::Whole(p) => p.lines().into_iter().filter(|l| reaches(l, &ys)).collect(),
            Inner::Lazy(l) => l.lines_in(ys),
        };
        self.uncase_lines(lines)
    }

    /// Glyph runs in drawing order, with positions from the paragraph's top-left (all of
    /// them: a lazy paragraph lays out the rest).
    pub fn glyph_runs(&self) -> Vec<GlyphRunInfo> {
        let runs = match &self.inner {
            Inner::Whole(p) => p.glyph_runs(),
            Inner::Lazy(l) => l.glyph_runs_in(f32::NEG_INFINITY..f32::INFINITY),
        };
        self.uncase_runs(runs)
    }

    /// The glyph runs of the lines that reach into `ys` (from the paragraph's top), in
    /// drawing order: what a view draws for the part of the paragraph it shows.
    pub fn glyph_runs_in(&self, ys: Range<f32>) -> Vec<GlyphRunInfo> {
        let runs = match &self.inner {
            Inner::Whole(p) => {
                let lines = p.lines();
                let first = lines.iter().position(|l| reaches(l, &ys));
                let last = lines.iter().rposition(|l| reaches(l, &ys));
                match (first, last) {
                    (Some(a), Some(b)) => p.runs_of(a..b + 1),
                    _ => Vec::new(),
                }
            }
            Inner::Lazy(l) => l.glyph_runs_in(ys),
        };
        self.uncase_runs(runs)
    }

    /// Inline objects with their boxes, by the ids the caller gave them (all of them: a lazy
    /// paragraph lays out the rest).
    pub fn inline_objects(&self) -> Vec<(u64, Rect)> {
        self.inline_objects_in(f32::NEG_INFINITY..f32::INFINITY)
    }

    /// The inline objects on the lines that reach into `ys`.
    pub fn inline_objects_in(&self, ys: Range<f32>) -> Vec<(u64, Rect)> {
        match &self.inner {
            Inner::Whole(p) => {
                if ys.start == f32::NEG_INFINITY && ys.end == f32::INFINITY {
                    return p.inline_objects();
                }
                let lines = p.lines();
                let first = lines.iter().position(|l| reaches(l, &ys));
                let last = lines.iter().rposition(|l| reaches(l, &ys));
                match (first, last) {
                    (Some(a), Some(b)) => p.objects_of(a..b + 1),
                    _ => Vec::new(),
                }
            }
            Inner::Lazy(l) => l.objects_in(ys),
        }
    }

    /// The text position nearest `point` (from the paragraph's top-left).
    pub fn hit_test(&self, point: Point) -> TextPosition {
        let pos = match &self.inner {
            Inner::Whole(p) => p.hit_test(point),
            Inner::Lazy(l) => l.hit_test(point),
        };
        self.uncase_pos(pos)
    }

    /// The link under `point`, if the point is on linked text.
    pub fn link_at(&self, point: Point) -> Option<u64> {
        match &self.inner {
            Inner::Whole(p) => p.link_at(point),
            Inner::Lazy(l) => l.link_at(point),
        }
    }

    /// Rectangles covering the text of link `link`, one or more per line it is on, in visual
    /// order; empty when no text carries it.
    pub fn link_rects(&self, link: u64) -> Vec<Rect> {
        match &self.inner {
            Inner::Whole(p) => p.link_rects(link),
            Inner::Lazy(l) => l.link_rects(link),
        }
    }

    /// The caret's rectangle at `pos`, `width` logical pixels wide.
    pub fn caret(&self, pos: TextPosition, width: f32) -> Rect {
        let pos = self.case_pos(pos);
        match &self.inner {
            Inner::Whole(p) => p.caret(pos, width),
            Inner::Lazy(l) => l.caret(pos, width),
        }
    }

    /// Rectangles covering `range`, one or more per line, in visual order.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let range = match &self.case {
            Some(m) => m.to_cased(range.start)..m.to_cased(range.end),
            None => range,
        };
        match &self.inner {
            Inner::Whole(p) => p.selection_rects(range),
            Inner::Lazy(l) => l.selection_rects(range),
        }
    }

    /// The word around `point`, as a byte range.
    pub fn word_at(&self, point: Point) -> Range<usize> {
        let r = match &self.inner {
            Inner::Whole(p) => p.word_at(point),
            Inner::Lazy(l) => l.word_at(point),
        };
        match &self.case {
            Some(m) => m.to_original(r.start)..m.to_original(r.end),
            None => r,
        }
    }

    /// The position one character to the visual right (or left) of `pos`, as the arrow keys
    /// move in mixed-direction text.
    pub fn move_visual(&self, pos: TextPosition, rightward: bool) -> TextPosition {
        let pos = self.case_pos(pos);
        let moved = match &self.inner {
            Inner::Whole(p) => p.move_visual(pos, rightward),
            Inner::Lazy(l) => l.moved(pos, |p, at| p.move_visual(at, rightward)),
        };
        self.uncase_pos(moved)
    }

    /// The position one word to the visual right (or left) of `pos`.
    pub fn move_word(&self, pos: TextPosition, rightward: bool) -> TextPosition {
        let pos = self.case_pos(pos);
        let moved = match &self.inner {
            Inner::Whole(p) => p.move_word(pos, rightward),
            Inner::Lazy(l) => l.moved(pos, |p, at| p.move_word(at, rightward)),
        };
        self.uncase_pos(moved)
    }

    fn case_pos(&self, pos: TextPosition) -> TextPosition {
        match &self.case {
            Some(m) => TextPosition { index: m.to_cased(pos.index), ..pos },
            None => pos,
        }
    }

    fn uncase_pos(&self, pos: TextPosition) -> TextPosition {
        match &self.case {
            Some(m) => TextPosition { index: m.to_original(pos.index), ..pos },
            None => pos,
        }
    }

    fn uncase_lines(&self, mut lines: Vec<LineInfo>) -> Vec<LineInfo> {
        if let Some(m) = &self.case {
            for l in &mut lines {
                l.range = m.to_original(l.range.start)..m.to_original(l.range.end);
            }
        }
        lines
    }

    fn uncase_runs(&self, mut runs: Vec<GlyphRunInfo>) -> Vec<GlyphRunInfo> {
        if let Some(m) = &self.case {
            for r in &mut runs {
                r.range = m.to_original(r.range.start)..m.to_original(r.range.end);
                for c in &mut r.clusters {
                    *c = m.to_original(*c);
                }
            }
        }
        runs
    }
}

/// True when `line` reaches into the band `ys`.
fn reaches(line: &LineInfo, ys: &Range<f32>) -> bool {
    line.bottom > ys.start && line.top < ys.end
}
