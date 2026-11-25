//! Laziness inside one huge paragraph.
//!
//! A paragraph of [`LAZY_MIN_BYTES`] or more that wraps is laid out a piece at a time, each
//! piece a few kilobytes of text laid out by parley on its own, and only when something asks
//! for its lines. The result is the same as laying the paragraph out whole: the same line
//! breaks, glyphs and positions, hit tests, carets and selections. What makes that hold:
//!
//! - **Pieces start where a line starts.** Lines are broken greedily, so the lines after a
//!   line start depend only on the text from there on. The first piece starts at the
//!   paragraph's start; each later one starts at a line start the piece before it found
//!   (a *checkpoint*), so a later part is laid out from the checkpoint before it rather than
//!   from the paragraph's start.
//! - **A piece's ends are not trusted.** A piece ends in the middle of the paragraph, so its
//!   last line is broken by the end of its text and the one before may be too: the last
//!   [`DROP`] lines of a piece are dropped. Its first lines lack what comes before them
//!   (the shaper's context, the line breaking rules' context): the next piece starts
//!   [`OVERLAP`] lines before the end of what the previous piece keeps, and the two must lay
//!   those lines out identically (line breaks, metrics, every glyph and its position) before
//!   the later piece's lines are used. Where they differ, the earlier piece is laid out again
//!   larger, taking the seam inside it, up to the whole paragraph.
//! - **Bidi is resolved per paragraph** (UAX #9 P1), not per line: a number's level can
//!   depend on the last strong letter before it however far back, a bracket's on its pair
//!   however far ahead. A piece laid out alone resolves its own levels, so its levels must
//!   be shown to be the paragraph's. Two ways:
//!   - Left-to-right text with no right-to-left letters, Arabic numbers or explicit
//!     embeddings, overrides or isolates before a piece's end has every level 0 there,
//!     whatever follows (in its own piece too). The paragraph is scanned for such characters
//!     only as far as it is laid out.
//!   - Otherwise the levels of the whole paragraph are resolved once (parley's own analysis,
//!     linear, about a sixth of the cost of laying the text out) and each piece's levels
//!     must match them on the lines it keeps; where they do not (a seam inside a stretch of
//!     text against the paragraph's direction, say), the piece grows until they do. So a
//!     paragraph with right-to-left text pays one bidi pass over all of it before its first
//!     lines, and again after each edit, and a run of text whose levels depend on context
//!     the piece does not have is laid out in one piece.
//! - **The base direction is the paragraph's**, found as parley finds it (the first strong
//!   character, outside isolates) and given to every piece.
//! - Alignment is per line within the wrap width, so pieces align alike. A paragraph that
//!   does not wrap is one line and is laid out whole.
//!
//! Heights below the last line laid out are estimated from the lines laid out so far.
//! Pieces far from what was asked for last are dropped and laid out again (to the same
//! lines) when asked for, so a paragraph shown end to end holds only the pieces near where it
//! was looked at. After an edit, the pieces before it are kept, and laying out from the
//! checkpoint before it stops as soon as a line start after the edit is the start of a piece
//! laid out before the edit and the two agree on that piece's first lines: the pieces after
//! it are kept too, moved by the edit's length and height.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use icu_properties::props::BidiClass;
use parley_engine::{Analysis, AnalysisDataSources, AnalysisOptions, Analyzer, BaseDirection};
use remedy_ui_core::{Point, Rect};

use crate::engine::{
    Affinity, GlyphRunInfo, Inner, LineInfo, ParagraphOptions, Piece, TextEngine, TextPosition,
};
use crate::rich::RichText;
use crate::style::{Direction, TextStyle};

/// About how much text one piece of a lazy paragraph is laid out with, by default
/// ([`LazyLimits::chunk_bytes`]).
pub const LAZY_CHUNK_BYTES: usize = 8 * 1024;

/// The shortest paragraph laid out lazily by default ([`LazyLimits::min_bytes`]): shorter
/// ones are laid out whole, as before, in little more time than one piece takes.
pub const LAZY_MIN_BYTES: usize = 32 * 1024;

/// Pieces of one lazy paragraph kept laid out at once by default
/// ([`LazyLimits::kept_pieces`]).
pub const LAZY_KEPT_PIECES: usize = 48;

/// Lines a piece shares with the piece before it, which both must lay out alike.
const OVERLAP: usize = 2;

/// Lines at the end of a piece that are not kept: the end of its text breaks them.
const DROP: usize = 2;

/// How huge paragraphs are laid out a piece at a time, for every paragraph laid out from now
/// on ([`set_lazy_limits`]). The defaults are the `LAZY_*` constants.
///
/// `min_bytes` is where laying out a piece at a time starts paying: `usize::MAX` lays out
/// every paragraph whole, which costs time and memory with its length. `chunk_bytes` is
/// about how much text a piece holds (at least 1): larger pieces shape more than a view shows,
/// smaller ones more often. `kept_pieces` bounds the memory a paragraph keeps laid out
/// (pieces past it are laid out again when asked for); raising it trades memory for less
/// shaping when a view moves back over text it showed.
///
/// ```
/// use remedy_ui_text::{LazyLimits, set_lazy_limits};
/// set_lazy_limits(LazyLimits::default().kept_pieces(128));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LazyLimits {
    /// The shortest paragraph laid out a piece at a time, in bytes. Default
    /// [`LAZY_MIN_BYTES`] (32 KiB).
    pub min_bytes: usize,
    /// About how much text one piece is laid out with, in bytes. Default
    /// [`LAZY_CHUNK_BYTES`] (8 KiB).
    pub chunk_bytes: usize,
    /// Pieces of one paragraph kept laid out at once. Default [`LAZY_KEPT_PIECES`] (48).
    pub kept_pieces: usize,
}

impl LazyLimits {
    /// The defaults, as a constant.
    pub const DEFAULT: LazyLimits = LazyLimits {
        min_bytes: LAZY_MIN_BYTES,
        chunk_bytes: LAZY_CHUNK_BYTES,
        kept_pieces: LAZY_KEPT_PIECES,
    };

    /// Sets [`min_bytes`](Self::min_bytes).
    pub fn min_bytes(mut self, bytes: usize) -> Self {
        self.min_bytes = bytes;
        self
    }

    /// Sets [`chunk_bytes`](Self::chunk_bytes).
    pub fn chunk_bytes(mut self, bytes: usize) -> Self {
        self.chunk_bytes = bytes;
        self
    }

    /// Sets [`kept_pieces`](Self::kept_pieces).
    pub fn kept_pieces(mut self, pieces: usize) -> Self {
        self.kept_pieces = pieces;
        self
    }
}

impl Default for LazyLimits {
    fn default() -> Self {
        LazyLimits::DEFAULT
    }
}

static LIMITS: std::sync::RwLock<LazyLimits> = std::sync::RwLock::new(LazyLimits::DEFAULT);

/// Sets how every paragraph laid out from now on is laid out a piece at a time; a paragraph
/// already laid out keeps the limits it started with.
pub fn set_lazy_limits(limits: LazyLimits) {
    *LIMITS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = limits;
}

/// The limits paragraphs are laid out a piece at a time with now ([`set_lazy_limits`]).
pub fn lazy_limits() -> LazyLimits {
    *LIMITS.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How far past the start of a piece laid out before an edit a new piece reaches, so the
/// two can be compared.
const REUSE_MARGIN: usize = 2 * 1024;

/// True when `text` laid out with `options` is laid out lazily.
pub(crate) fn eligible(text: &RichText, options: &ParagraphOptions) -> bool {
    text.len() >= lazy_limits().min_bytes && options.max_width.is_some_and(|w| w.is_finite() && w > 0.0)
}

/// One piece of the paragraph and the lines it keeps.
#[derive(Clone)]
struct Chunk {
    /// Where its text starts (a line start of the paragraph).
    start: usize,
    /// Where its text ends.
    end: usize,
    /// The first line it keeps, in its piece.
    first: usize,
    /// How many lines it keeps.
    count: usize,
    /// The text of the lines it keeps.
    kept: Range<usize>,
    /// The first byte at which an edit leaves its kept lines as they are.
    guard: usize,
    /// The top of its first kept line and the bottom of its last, in the paragraph.
    top: f64,
    bottom: f64,
    /// Where its piece's first line's top is, in device pixels before rounding: the sum of
    /// the heights of the lines before it, as parley sums them.
    y0: f64,
    /// What to add to its piece's y to place it in the paragraph: 0, or a whole number of
    /// device pixels for a piece laid out before an edit moved it.
    dy: f64,
    /// Its widest kept line, in device pixels as parley measures it.
    width: f32,
    /// Where its last kept line's bottom is, in device pixels before rounding.
    y_end: f64,
    piece: Option<Rc<Piece>>,
    used: u64,
}

impl Chunk {
    fn lines(&self) -> Range<usize> {
        self.first..self.first + self.count
    }

    /// The same chunk `delta` bytes and `dy` pixels (`device` device pixels) further on.
    fn shifted(mut self, delta: isize, dy: f64, device: f64) -> Chunk {
        let m = |x: usize| x.saturating_add_signed(delta);
        self.start = m(self.start);
        self.end = m(self.end);
        self.kept = m(self.kept.start)..m(self.kept.end);
        self.guard = m(self.guard);
        self.y0 += device;
        self.y_end += device;
        self.top += dy;
        self.bottom += dy;
        self.dy += dy;
        self
    }
}

/// Where the paragraph's bidi levels are known.
#[derive(Clone)]
struct Bidi {
    /// The paragraph's base direction is right to left.
    rtl: bool,
    /// No character before this needs bidi resolution (and the base is left to right).
    quiet_to: usize,
    /// The character at `quiet_to` needs it.
    loud: bool,
    /// The whole paragraph's levels, by byte, once resolved.
    levels: Option<Rc<Vec<u8>>>,
}

/// What an edit left of the pieces laid out before it.
struct Reuse {
    /// The pieces after the edit, as they were.
    chunks: Vec<Chunk>,
    /// Bytes the edit added (negative: removed).
    delta: isize,
    /// Where the edit ended in the text before it.
    old_end: usize,
    /// The paragraph was laid out to its end before the edit.
    done: bool,
    /// The levels before the edit (`None`: the text there was quiet).
    levels: Option<Rc<Vec<u8>>>,
    /// Text before this was quiet before the edit.
    quiet_to: usize,
}

/// A paragraph laid out lazily.
pub(crate) struct Lazy {
    st: RefCell<State>,
}

struct State {
    engine: Rc<RefCell<TextEngine>>,
    text: Rc<RichText>,
    base: TextStyle,
    /// The options, with the base direction made explicit.
    options: ParagraphOptions,
    asked: ParagraphOptions,
    bidi: Bidi,
    chunks: Vec<Chunk>,
    done: bool,
    clock: u64,
    reuse: Option<Reuse>,
    analyzer: Analyzer,
    analysis: Analysis,
    /// Piece size and pieces kept, as they were when the paragraph was laid out.
    chunk_bytes: usize,
    kept_pieces: usize,
}

/// Classes that make parley resolve bidi at all (its `BIDI_MASK`).
fn is_loud(class: BidiClass) -> bool {
    matches!(
        class,
        BidiClass::RightToLeft
            | BidiClass::ArabicLetter
            | BidiClass::ArabicNumber
            | BidiClass::LeftToRightEmbedding
            | BidiClass::RightToLeftEmbedding
            | BidiClass::LeftToRightOverride
            | BidiClass::RightToLeftOverride
            | BidiClass::LeftToRightIsolate
            | BidiClass::RightToLeftIsolate
            | BidiClass::FirstStrongIsolate
    )
}

fn bidi_class(c: char) -> BidiClass {
    parley_data::Properties::get(c).bidi_class()
}

/// The base direction parley's analysis gives `text` (UAX #9 P2 and P3: the first strong
/// character outside isolates; isolates end at paragraph separators): right to left?
fn base_direction(text: &str) -> bool {
    let mut isolates = 0u32;
    for c in text.chars() {
        // ASCII digits, punctuation and spaces are never strong, isolates or separators.
        if c.is_ascii() && !c.is_ascii_alphabetic() && !c.is_ascii_control() {
            continue;
        }
        match bidi_class(c) {
            BidiClass::RightToLeftIsolate | BidiClass::LeftToRightIsolate | BidiClass::FirstStrongIsolate => {
                isolates += 1;
            }
            BidiClass::PopDirectionalIsolate if isolates > 0 => isolates -= 1,
            BidiClass::ParagraphSeparator => isolates = 0,
            class @ (BidiClass::LeftToRight | BidiClass::RightToLeft | BidiClass::ArabicLetter)
                if isolates == 0 =>
            {
                return class != BidiClass::LeftToRight;
            }
            _ => {}
        }
    }
    false
}

/// True when line `ai` of `a` (whose text starts at byte `a0`) and line `bi` of `b` (at
/// `b0`) are the same line: the same text, metrics and glyphs at the same places.
fn same_line(a: &Piece, ai: usize, a0: usize, b: &Piece, bi: usize, b0: usize) -> bool {
    let (Some(la), Some(lb)) = (a.line(ai), b.line(bi)) else { return false };
    let near = |x: f32, y: f32| (x - y).abs() <= 1e-3;
    let near_rect = |x: Option<Rect>, y: Option<Rect>| match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            near(x.min.x, y.min.x)
                && near(x.max.x, y.max.x)
                && near(x.min.y - la.top, y.min.y - lb.top)
                && near(x.max.y - la.top, y.max.y - lb.top)
        }
        _ => false,
    };
    if la.range.start + a0 != lb.range.start + b0
        || la.range.end + a0 != lb.range.end + b0
        || !near(la.bottom - la.top, lb.bottom - lb.top)
        || !near(la.baseline - la.top, lb.baseline - lb.top)
        || !near(la.width, lb.width)
        || !near(la.left, lb.left)
    {
        return false;
    }
    let (ra, rb) = (a.runs_of(ai..ai + 1), b.runs_of(bi..bi + 1));
    let (oa, ob) = (a.objects_of(ai..ai + 1), b.objects_of(bi..bi + 1));
    ra.len() == rb.len()
        && oa.len() == ob.len()
        && oa.iter().zip(&ob).all(|((ia, x), (ib, y))| {
            ia == ib
                && near(x.min.x, y.min.x)
                && near(x.max.x, y.max.x)
                && near(x.min.y - la.top, y.min.y - lb.top)
        })
        && ra.iter().zip(&rb).all(|(x, y)| {
            x.font.id() == y.font.id()
                && x.font.index() == y.font.index()
                && x.size == y.size
                && x.rtl == y.rtl
                && x.color == y.color
                && x.link == y.link
                && x.range.start + a0 == y.range.start + b0
                && x.range.end + a0 == y.range.end + b0
                && x.glyphs.len() == y.glyphs.len()
                && x.glyphs
                    .iter()
                    .zip(&y.glyphs)
                    .all(|(g, h)| g.id == h.id && near(g.x, h.x) && near(g.y - la.top, h.y - lb.top))
                && x.advances.len() == y.advances.len()
                && x.advances.iter().zip(&y.advances).all(|(p, q)| near(*p, *q))
                && x.clusters.len() == y.clusters.len()
                && x.clusters.iter().zip(&y.clusters).all(|(p, q)| p + a0 == q + b0)
                && near_rect(x.underline, y.underline)
                && near_rect(x.strikethrough, y.strikethrough)
                && match (&x.background, &y.background) {
                    (None, None) => true,
                    (Some(p), Some(q)) => {
                        p.color == q.color
                            && near(p.radius, q.radius)
                            && near_rect(Some(p.rect), Some(q.rect))
                    }
                    _ => false,
                }
        })
}

fn shift_rect(r: Rect, dy: f32) -> Rect {
    Rect::new(r.min.x, r.min.y + dy, r.max.x, r.max.y + dy)
}

fn shift_line(mut l: LineInfo, at: usize, dy: f32) -> LineInfo {
    l.range = l.range.start + at..l.range.end + at;
    l.top += dy;
    l.bottom += dy;
    l.baseline += dy;
    l
}

fn shift_run(mut r: GlyphRunInfo, at: usize, dy: f32) -> GlyphRunInfo {
    r.range = r.range.start + at..r.range.end + at;
    for g in &mut r.glyphs {
        g.y += dy;
    }
    for c in &mut r.clusters {
        *c += at;
    }
    r.underline = r.underline.map(|u| shift_rect(u, dy));
    r.strikethrough = r.strikethrough.map(|u| shift_rect(u, dy));
    if let Some(bg) = r.background.as_mut() {
        bg.rect = shift_rect(bg.rect, dy);
    }
    r
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// How many bytes `a` and `b` start with alike (compared a block at a time).
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + BLOCK <= n && a.get(i..i + BLOCK) == b.get(i..i + BLOCK) {
        i += BLOCK;
    }
    i + a
        .get(i..n)
        .unwrap_or_default()
        .iter()
        .zip(b.get(i..n).unwrap_or_default())
        .take_while(|(x, y)| x == y)
        .count()
}

/// How many bytes `a` and `b` end with alike.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 4096;
    let n = a.len().min(b.len());
    let (ea, eb) = (a.len(), b.len());
    let mut i = 0;
    while i + BLOCK <= n && a.get(ea - i - BLOCK..ea - i) == b.get(eb - i - BLOCK..eb - i) {
        i += BLOCK;
    }
    let (ra, rb) = (a.get(..ea - i).unwrap_or_default(), b.get(..eb - i).unwrap_or_default());
    i + ra.iter().rev().zip(rb.iter().rev()).take(n - i).take_while(|(x, y)| x == y).count()
}

/// Why a piece could not be used where it was laid out.
struct Mismatch;

impl State {
    fn new(
        engine: Rc<RefCell<TextEngine>>,
        text: Rc<RichText>,
        base: TextStyle,
        asked: ParagraphOptions,
    ) -> Self {
        let rtl = match asked.direction {
            Direction::Auto => base_direction(text.as_str()),
            Direction::Ltr => false,
            Direction::Rtl => true,
        };
        let limits = lazy_limits();
        let mut options = asked;
        options.direction = if rtl { Direction::Rtl } else { Direction::Ltr };
        State {
            engine,
            text,
            base,
            options,
            asked,
            bidi: Bidi { rtl, quiet_to: 0, loud: rtl, levels: None },
            chunks: Vec::new(),
            done: false,
            clock: 0,
            reuse: None,
            analyzer: Analyzer::new(),
            analysis: Analysis::new(),
            chunk_bytes: limits.chunk_bytes.max(1),
            kept_pieces: limits.kept_pieces,
        }
    }

    fn len(&self) -> usize {
        self.text.len()
    }

    /// Device pixels per logical pixel.
    fn scale(&self) -> f32 {
        let s = self.options.scale;
        if s.is_finite() && s > 0.0 { s } else { 1.0 }
    }

    /// Lays out the text from `start` to `end` on its own, its first line's top at `y0`.
    fn lay(&mut self, start: usize, end: usize, y0: f64) -> Rc<Piece> {
        let sub = self.text.slice(start..end);
        Rc::new(self.engine.borrow_mut().layout_piece(&sub, &self.base, self.options, y0))
    }

    /// Chunk `k`'s piece, laid out again if it was dropped.
    fn piece(&mut self, k: usize) -> Option<Rc<Piece>> {
        self.clock += 1;
        let clock = self.clock;
        let c = self.chunks.get(k)?;
        if let Some(p) = &c.piece {
            let p = p.clone();
            if let Some(c) = self.chunks.get_mut(k) {
                c.used = clock;
            }
            return Some(p);
        }
        let (start, end, y0) = (c.start, c.end, c.y0);
        let p = self.lay(start, end, y0);
        if let Some(c) = self.chunks.get_mut(k) {
            c.piece = Some(p.clone());
            c.used = clock;
            c.dy = 0.0;
        }
        self.evict(k);
        Some(p)
    }

    /// Drops the least recently used pieces past [`LazyLimits::kept_pieces`], never chunk `keep` or the
    /// last chunk (the next piece starts from it).
    fn evict(&mut self, keep: usize) {
        let last = self.chunks.len().saturating_sub(1);
        loop {
            let live = self.chunks.iter().filter(|c| c.piece.is_some()).count();
            if live <= self.kept_pieces {
                return;
            }
            let oldest = self
                .chunks
                .iter()
                .enumerate()
                .filter(|(i, c)| c.piece.is_some() && *i != keep && *i != last)
                .min_by_key(|(_, c)| c.used)
                .map(|(i, _)| i);
            match oldest.and_then(|i| self.chunks.get_mut(i)) {
                Some(c) => c.piece = None,
                None => return,
            }
        }
    }

    /// Scans for characters that need bidi resolution as far as `to`.
    fn scan_quiet(&mut self, to: usize) {
        let b = &mut self.bidi;
        if b.loud || b.quiet_to >= to {
            return;
        }
        let s = self.text.as_str();
        let to = to.min(s.len());
        let bytes = s.as_bytes();
        let mut i = b.quiet_to;
        while i < to {
            let byte = bytes.get(i).copied().unwrap_or(0);
            if byte < 0x80 {
                i += 1;
                continue;
            }
            let c = s.get(i..).and_then(|t| t.chars().next()).unwrap_or(' ');
            if is_loud(bidi_class(c)) {
                b.loud = true;
                break;
            }
            i += c.len_utf8();
        }
        b.quiet_to = i.min(s.len());
    }

    /// The whole paragraph's levels, by byte.
    fn levels(&mut self) -> Rc<Vec<u8>> {
        if let Some(l) = &self.bidi.levels {
            return l.clone();
        }
        let l = Rc::new(self.resolve(0..self.len()));
        self.bidi.levels = Some(l.clone());
        l
    }

    /// The levels parley resolves for the text in `range` laid out on its own, by byte.
    fn resolve(&mut self, range: Range<usize>) -> Vec<u8> {
        let s = self.text.as_str().get(range).unwrap_or_default();
        let options = AnalysisOptions {
            base_direction: if self.bidi.rtl { BaseDirection::Rtl } else { BaseDirection::Ltr },
            ..AnalysisOptions::default()
        };
        self.analyzer.analyze(s, &options, &mut self.analysis, &AnalysisDataSources::default());
        let levels = self.analysis.bidi_levels();
        let mut out = vec![0u8; s.len()];
        if levels.is_empty() {
            return out;
        }
        for ((i, c), level) in s.char_indices().zip(levels) {
            if let Some(b) = out.get_mut(i..i + c.len_utf8()) {
                b.fill(level.to_u8());
            }
        }
        out
    }

    /// True when the piece laid out from `start` to `end` has the paragraph's levels on
    /// `kept`.
    fn bidi_ok(&mut self, start: usize, end: usize, kept: &Range<usize>) -> bool {
        self.scan_quiet(end);
        if !self.bidi.rtl && self.bidi.quiet_to >= end {
            return true;
        }
        let whole = self.levels();
        let mine = self.resolve(start..end);
        let (a, b) = (kept.start.max(start), kept.end.min(end));
        whole.get(a..b) == mine.get(a - start..b - start)
    }

    /// Where the next piece starts, the line of the last chunk's piece it starts at, and
    /// that line's top in device pixels before rounding.
    fn seam(&mut self) -> Option<(usize, Rc<Piece>, usize, f64)> {
        let k = self.chunks.len().checked_sub(1)?;
        let piece = self.piece(k)?;
        let c = self.chunks.get(k)?;
        // A whole paragraph of one line kept after an edit at its end has fewer lines than
        // the overlap: the seam is then its first line (and the piece is laid out again).
        let line = (c.first + c.count).saturating_sub(OVERLAP);
        let y = (0..line).fold(c.y0, |y, i| y + f64::from(piece.line_height(i)));
        Some((c.start + piece.line(line)?.range.start, piece, line, y))
    }

    /// Lays out a piece from `start` (with at least `min` bytes), after the last chunk if
    /// there is one.
    fn try_chunk(
        &mut self,
        start: usize,
        y0: f64,
        prev: Option<(Rc<Piece>, usize, usize)>,
        min: usize,
    ) -> Result<Chunk, Mismatch> {
        let len = self.len();
        let mut size = min.max(self.chunk_bytes);
        let first = if prev.is_some() { OVERLAP } else { 0 };
        loop {
            let end =
                if start + size >= len { len } else { floor_boundary(self.text.as_str(), start + size) };
            let piece = self.lay(start, end, y0);
            let n = piece.line_count();
            let last = end == len;
            let kept_end = if last { n } else { n.saturating_sub(DROP) };
            if kept_end <= first || (!last && kept_end < first + OVERLAP + 1) {
                if last {
                    return Err(Mismatch);
                }
                size *= 2;
                continue;
            }
            if let Some((pp, line, at)) = &prev
                && !(0..OVERLAP).all(|i| same_line(pp, line + i, *at, &piece, i, start))
            {
                return Err(Mismatch);
            }
            let line_start = |i: usize| piece.line(i).map(|l| start + l.range.start);
            let kept = line_start(first).unwrap_or(start)..if last {
                len
            } else {
                line_start(kept_end).unwrap_or(end)
            };
            // Laid out whole, the piece is the paragraph.
            if !(start == 0 && last) && !self.bidi_ok(start, end, &kept) {
                // The end of the text may be what changed the levels: grow the piece, and
                // past four times its size take the seam inside the piece before.
                if last || (prev.is_some() && size >= 4 * min.max(self.chunk_bytes)) {
                    return Err(Mismatch);
                }
                size *= 2;
                continue;
            }
            let (Some(lf), Some(ll)) = (piece.line(first), piece.line(kept_end - 1)) else {
                return Err(Mismatch);
            };
            let top = f64::from(lf.top);
            let dy = 0.0;
            let width = (first..kept_end).map(|i| piece.line_extent(i)).fold(0.0, f32::max);
            let y_end = (0..kept_end).fold(y0, |y, i| y + f64::from(piece.line_height(i)));
            let guard = if last { len } else { line_start(kept_end + 1).unwrap_or(end) };
            self.clock += 1;
            return Ok(Chunk {
                start,
                end,
                first,
                count: kept_end - first,
                kept,
                guard,
                top,
                bottom: dy + f64::from(ll.bottom),
                y0,
                dy,
                width,
                y_end,
                piece: Some(piece),
                used: self.clock,
            });
        }
    }

    /// Lays out the next piece. False when the paragraph is laid out to its end.
    fn extend(&mut self) -> bool {
        if self.done {
            return false;
        }
        let mut min = self.chunk_bytes;
        loop {
            let seam = self.seam();
            let (start, y0) = seam.as_ref().map_or((0, 0.0), |s| (s.0, s.3));
            let prev_start = self.chunks.last().map_or(0, |c| c.start);
            let prev = seam.map(|(_, piece, line, _)| (piece, line, prev_start));
            let want = self.reuse_target(start).map_or(min, |t| min.max(t - start + REUSE_MARGIN));
            match self.try_chunk(start, y0, prev, want) {
                Ok(mut c) => {
                    // An edit's guard for the chunk before: past the line after its last.
                    if let Some(k) = self.chunks.len().checked_sub(1)
                        && let Some(p) = c.piece.clone()
                    {
                        let g = p.line(OVERLAP + 1).map_or(c.end, |l| c.start + l.range.start);
                        if let Some(prev) = self.chunks.get_mut(k) {
                            prev.guard = g;
                        }
                    }
                    let tail = self.reuse_into(&mut c);
                    let last = c.kept.end == self.len();
                    self.chunks.push(c);
                    match tail {
                        Some((tail, done)) => {
                            self.chunks.extend(tail);
                            self.done = done;
                        }
                        None => self.done = last,
                    }
                    let k = self.chunks.len() - 1;
                    self.evict(k);
                    return true;
                }
                Err(Mismatch) => match self.chunks.pop() {
                    // Take the seam inside a larger piece.
                    Some(popped) => min = (popped.end - popped.start) * 2,
                    None => min *= 2,
                },
            }
        }
    }

    /// Where the next piece laid out before the edit starts (now), if a piece from `start`
    /// should reach it.
    fn reuse_target(&mut self, start: usize) -> Option<usize> {
        let r = self.reuse.as_mut()?;
        r.chunks.retain(|c| {
            c.first == OVERLAP && c.start >= r.old_end && c.start.saturating_add_signed(r.delta) > start
        });
        let c = r.chunks.first()?;
        let t = c.start.saturating_add_signed(r.delta);
        (t - start <= 8 * self.chunk_bytes).then_some(t)
    }

    /// If new chunk `c` reaches the start of a piece laid out before the edit with the same
    /// lines, ends `c` there and returns the chunks from that piece on, moved, and whether
    /// they reach the end.
    fn reuse_into(&mut self, c: &mut Chunk) -> Option<(Vec<Chunk>, bool)> {
        let piece = c.piece.clone()?;
        let n = piece.line_count();
        let last = c.end == self.len();
        let limit = if last { n } else { c.first + c.count };
        let r = self.reuse.as_ref()?;
        let (delta, done) = (r.delta, r.done);
        let (ci, cand) = r.chunks.iter().enumerate().find_map(|(i, o)| {
            let t = o.start.saturating_add_signed(delta);
            (t >= c.kept.start && t < c.end).then(|| (i, o.clone()))
        })?;
        let t = cand.start.saturating_add_signed(delta);
        let i = (c.first..limit).find(|&i| piece.line(i).is_some_and(|l| c.start + l.range.start == t))?;
        if i + OVERLAP > limit {
            return None;
        }
        // The old piece's first line is new line `i`: where it is now, before rounding. The
        // old piece's lines move by whole device pixels only if that moves by a whole number.
        let y_new = (0..i).fold(c.y0, |y, j| y + f64::from(piece.line_height(j)));
        let device = y_new - cand.y0;
        if device.fract() != 0.0 {
            return None;
        }
        let old = match &cand.piece {
            Some(p) => p.clone(),
            None => self.lay(t, cand.end.saturating_add_signed(delta), cand.y0),
        };
        if !(0..OVERLAP).all(|j| same_line(&piece, i + j, c.start, &old, j, t)) {
            return None;
        }
        // The levels the old pieces were laid out with must be the levels now.
        let old_levels = self.reuse.as_ref().and_then(|r| r.levels.clone());
        let old_quiet = self.reuse.as_ref().map_or(0, |r| r.quiet_to);
        let tail: Vec<Chunk> =
            self.reuse.as_ref().map(|r| r.chunks.get(ci..).unwrap_or_default().to_vec())?;
        let reach = tail.last().map_or(t, |o| o.end.saturating_add_signed(delta));
        self.scan_quiet(reach);
        if self.bidi.rtl || self.bidi.quiet_to < reach {
            let now = self.levels();
            let from = cand.start;
            let to = tail.last().map_or(from, |o| o.end);
            let same = (from..to).all(|x| {
                let was = if x < old_quiet {
                    0
                } else {
                    old_levels.as_ref().and_then(|l| l.get(x).copied()).unwrap_or(u8::MAX)
                };
                now.get(x.saturating_add_signed(delta)).copied() == Some(was)
            });
            if !same {
                self.reuse = None;
                return None;
            }
        }
        let scale = f64::from(piece.scale);
        let dy = device / scale;
        // `c` now keeps its lines up to where the old piece's kept lines begin.
        let kept_end = i + OVERLAP;
        c.count = kept_end - c.first;
        c.kept.end = cand.kept.start.saturating_add_signed(delta);
        c.guard = old.line(OVERLAP + 1).map_or(cand.kept.start, |l| t + l.range.start);
        if let Some(l) = piece.line(kept_end - 1) {
            c.bottom = c.dy + f64::from(l.bottom);
        }
        c.width = (c.first..kept_end).map(|i| piece.line_extent(i)).fold(0.0, f32::max);
        c.y_end = (0..kept_end).fold(c.y0, |y, i| y + f64::from(piece.line_height(i)));
        self.reuse = None;
        Some((tail.into_iter().map(|o| o.shifted(delta, dy, device)).collect(), done))
    }

    /// Lays out pieces until the lines reach down past `y`.
    fn reach_y(&mut self, y: f32) {
        while !self.done && self.chunks.last().is_none_or(|c| c.bottom <= f64::from(y)) {
            self.extend();
        }
    }

    /// Lays out pieces until a chunk keeps the line `index` is on (a line start belongs to
    /// the line it starts).
    fn reach_index(&mut self, index: usize) {
        while !self.done && self.chunks.last().is_none_or(|c| c.kept.end <= index) {
            self.extend();
        }
    }

    /// The chunk that keeps the line `index` is on.
    fn chunk_at_index(&mut self, index: usize) -> usize {
        self.reach_index(index);
        self.chunks.partition_point(|c| c.kept.start <= index).saturating_sub(1)
    }

    /// The chunk that keeps the line at `y`.
    fn chunk_at_y(&mut self, y: f32) -> usize {
        self.reach_y(y);
        self.chunks.partition_point(|c| c.top <= f64::from(y)).saturating_sub(1)
    }

    /// A chunk whose piece has the line `index` is on and the lines on both sides of it laid
    /// out as the paragraph has them: the chunk after, for the last line a chunk keeps (it is
    /// that chunk's shared first lines).
    fn chunk_around(&mut self, pos: TextPosition) -> usize {
        // A position after a line's last character, at the next line's start, is on the line
        // it ends.
        let index = match pos.affinity {
            Affinity::Upstream => pos.index.saturating_sub(1),
            Affinity::Downstream => pos.index,
        };
        let k = self.chunk_at_index(index);
        let Some(piece) = self.piece(k) else { return k };
        let Some(c) = self.chunks.get(k) else { return k };
        let last_line = piece.line(c.first + c.count - 1).map_or(usize::MAX, |l| c.start + l.range.start);
        if index >= last_line && !(self.done && k + 1 == self.chunks.len()) {
            if k + 1 == self.chunks.len() {
                self.extend();
            }
            if k + 1 < self.chunks.len() {
                return k + 1;
            }
        }
        k
    }

    fn height(&self) -> f64 {
        let Some(last) = self.chunks.last() else {
            // Nothing laid out: one line per wrap width's worth of text, at the style's size.
            let w = f64::from(self.options.max_width.unwrap_or(1.0).max(1.0));
            let per_line = (w / f64::from(self.base.size.max(1.0) * 0.5)).max(1.0);
            return (self.len() as f64 / per_line).ceil() * f64::from(self.base.size) * 1.25;
        };
        if self.done {
            return f64::from(last.y_end as f32 / self.scale());
        }
        let lines: usize = self.chunks.iter().map(|c| c.count).sum();
        let bytes = last.kept.end.max(1) as f64;
        let per_line = bytes / lines.max(1) as f64;
        let line_height = last.bottom / lines.max(1) as f64;
        last.bottom + (self.len() - last.kept.end) as f64 / per_line.max(1.0) * line_height
    }
}

impl Lazy {
    pub(crate) fn new(
        engine: TextEngine,
        text: &RichText,
        base: &TextStyle,
        options: ParagraphOptions,
    ) -> Self {
        let st = State::new(Rc::new(RefCell::new(engine)), Rc::new(text.clone()), base.clone(), options);
        Lazy { st: RefCell::new(st) }
    }

    /// The paragraph `before` laid out lazily with `base` and `options`, edited to `text`:
    /// a lazy paragraph keeping the pieces the edit left as they were, or `None` when
    /// `before` is not such a paragraph.
    pub(crate) fn after_edit(
        before: &Inner,
        text: &RichText,
        base: &TextStyle,
        options: ParagraphOptions,
    ) -> Option<Lazy> {
        let Inner::Lazy(before) = before else { return None };
        if !eligible(text, &options) {
            return None;
        }
        let old = before.st.borrow();
        if old.asked != options || old.base != *base {
            return None;
        }
        let (a, b) = (old.text.as_str(), text.as_str());
        let prefix = floor_boundary(a, common_prefix(a.as_bytes(), b.as_bytes()));
        let room = a.len().min(b.len()) - prefix;
        let suffix = common_suffix(a.as_bytes(), b.as_bytes()).min(room);
        let (old_end, new_end) = (a.len() - suffix, b.len() - suffix);
        let delta = b.len() as isize - a.len() as isize;
        // Spans and objects must be the old ones, moved by the edit.
        let moved = |x: usize| {
            if x <= prefix {
                Some(x)
            } else if x >= old_end {
                Some(x.saturating_add_signed(delta))
            } else {
                None
            }
        };
        let spans_same = old.text.spans().len() == text.spans().len()
            && old.text.spans().iter().zip(text.spans()).all(|(o, n)| {
                o.patch == n.patch
                    && moved(o.range.start) == Some(n.range.start)
                    && moved(o.range.end) == Some(n.range.end)
            });
        let objects_same = old.text.objects().len() == text.objects().len()
            && old
                .text
                .objects()
                .iter()
                .zip(text.objects())
                .all(|((at, o), (bt, n))| o == n && moved(*at) == Some(*bt));
        let mut st = State::new(old.engine.clone(), Rc::new(text.clone()), base.clone(), options);
        // Faces the inserted text needs: when any load, everything is laid out again.
        let inserted = b.get(prefix..new_end).unwrap_or_default();
        let fonts_same = !st.engine.borrow_mut().fonts().load_needed(inserted);
        if !(spans_same && objects_same && fonts_same) || st.bidi.rtl != old.bidi.rtl {
            return Some(Lazy { st: RefCell::new(st) });
        }
        // What is known quiet carries over: the text before the edit, and after it when the
        // edit and all before it were quiet.
        if old.bidi.quiet_to < prefix || (old.bidi.quiet_to == prefix && old.bidi.loud) {
            st.bidi.quiet_to = old.bidi.quiet_to;
            st.bidi.loud = old.bidi.loud;
        } else {
            st.bidi.quiet_to = prefix;
            st.scan_quiet(new_end);
            if !st.bidi.loud && st.bidi.quiet_to >= new_end && old.bidi.quiet_to >= old_end {
                st.bidi.quiet_to = old.bidi.quiet_to.saturating_add_signed(delta);
                st.bidi.loud = old.bidi.loud;
            }
        }
        // Where an edit could change the lines kept before it: its start, or where it
        // changed a level.
        // Text before the first character needing bidi resolution has level 0 whatever
        // follows; past it, the levels now must be those the pieces were laid out with.
        let mut from = prefix;
        st.scan_quiet(prefix);
        if st.bidi.rtl || st.bidi.quiet_to < prefix {
            let now = st.levels();
            let was = |x: usize| {
                if x < old.bidi.quiet_to && !old.bidi.rtl {
                    Some(0)
                } else {
                    old.bidi.levels.as_ref().and_then(|l| l.get(x).copied())
                }
            };
            let quiet = if st.bidi.rtl { 0 } else { st.bidi.quiet_to };
            if let Some(m) = (quiet..prefix).find(|&x| now.get(x).copied() != was(x)) {
                from = m;
            }
        }
        let keep = old.chunks.partition_point(|c| c.guard <= from);
        st.chunks = old.chunks.get(..keep).unwrap_or_default().to_vec();
        for c in &mut st.chunks {
            // Pieces reaching past the edit are laid out again only to the edit, should they
            // be dropped: their kept lines end before it.
            if c.end > prefix {
                c.end = prefix;
            }
        }
        let tail = old.chunks.get(keep..).unwrap_or_default().to_vec();
        st.reuse = Some(Reuse {
            chunks: tail,
            delta,
            old_end,
            done: old.done,
            levels: old.bidi.levels.clone(),
            quiet_to: if old.bidi.rtl { 0 } else { old.bidi.quiet_to },
        });
        drop(old);
        Some(Lazy { st: RefCell::new(st) })
    }

    /// The paragraph laid out again at a new width: lazily, or whole when it no longer wraps.
    pub(crate) fn rewrap(&mut self, max_width: Option<f32>) -> Option<Piece> {
        let st = self.st.get_mut();
        let mut options = st.asked;
        options.max_width = max_width;
        if !eligible(&st.text, &options) {
            let text = st.text.clone();
            let base = st.base.clone();
            return Some(st.engine.borrow_mut().layout_piece(&text, &base, options, 0.0));
        }
        *st = State::new(st.engine.clone(), st.text.clone(), st.base.clone(), options);
        None
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.st.borrow().done
    }

    pub(crate) fn is_rtl(&self) -> bool {
        self.st.borrow().bidi.rtl
    }

    pub(crate) fn width(&self) -> f32 {
        let st = self.st.borrow();
        st.chunks.iter().map(|c| c.width).fold(0.0, f32::max) / st.scale()
    }

    pub(crate) fn height(&self) -> f32 {
        self.st.borrow().height() as f32
    }

    pub(crate) fn first_baseline(&self) -> Option<f32> {
        let mut st = self.st.borrow_mut();
        st.reach_y(0.0);
        let p = st.piece(0)?;
        let c = st.chunks.first()?;
        Some((c.dy + f64::from(p.line(0)?.baseline)) as f32)
    }

    /// Lays out what reaches into `ys` and calls `f` with each chunk's piece, the chunk and
    /// the band of its lines that reach into it.
    fn each_in<T>(
        &self,
        ys: Range<f32>,
        mut f: impl FnMut(&Piece, &Chunk, Range<usize>, &mut Vec<T>),
    ) -> Vec<T> {
        let mut st = self.st.borrow_mut();
        st.reach_y(ys.end);
        let mut out = Vec::new();
        let from = st.chunks.partition_point(|c| c.bottom <= f64::from(ys.start));
        let to = st.chunks.partition_point(|c| c.top < f64::from(ys.end));
        for k in from..to {
            let Some(piece) = st.piece(k) else { continue };
            let Some(c) = st.chunks.get(k) else { continue };
            let (a, b) = (f64::from(ys.start) - c.dy, f64::from(ys.end) - c.dy);
            let lines: Vec<usize> = c
                .lines()
                .filter(|&i| piece.line(i).is_some_and(|l| f64::from(l.bottom) > a && f64::from(l.top) < b))
                .collect();
            if let (Some(&first), Some(&last)) = (lines.first(), lines.last()) {
                f(&piece, c, first..last + 1, &mut out);
            }
        }
        out
    }

    pub(crate) fn lines_in(&self, ys: Range<f32>) -> Vec<LineInfo> {
        self.each_in(ys, |p, c, lines, out| {
            out.extend(lines.filter_map(|i| p.line(i)).map(|l| shift_line(l, c.start, c.dy as f32)));
        })
    }

    pub(crate) fn glyph_runs_in(&self, ys: Range<f32>) -> Vec<GlyphRunInfo> {
        self.each_in(ys, |p, c, lines, out| {
            out.extend(p.runs_of(lines).into_iter().map(|r| shift_run(r, c.start, c.dy as f32)));
        })
    }

    pub(crate) fn objects_in(&self, ys: Range<f32>) -> Vec<(u64, Rect)> {
        self.each_in(ys, |p, c, lines, out| {
            out.extend(p.objects_of(lines).into_iter().map(|(id, r)| (id, shift_rect(r, c.dy as f32))));
        })
    }

    /// The chunk for a point, its piece, and the point in the piece's coordinates.
    fn at_point(&self, point: Point) -> Option<(Rc<Piece>, usize, Point)> {
        let mut st = self.st.borrow_mut();
        let k = st.chunk_at_y(point.y);
        let piece = st.piece(k)?;
        let c = st.chunks.get(k)?;
        Some((piece, c.start, Point::new(point.x, (f64::from(point.y) - c.dy) as f32)))
    }

    pub(crate) fn hit_test(&self, point: Point) -> TextPosition {
        match self.at_point(point) {
            Some((p, at, local)) => {
                let mut pos = p.hit_test(local);
                pos.index += at;
                pos
            }
            None => TextPosition::default(),
        }
    }

    pub(crate) fn link_at(&self, point: Point) -> Option<u64> {
        let (p, _, local) = self.at_point(point)?;
        p.link_at(local)
    }

    pub(crate) fn word_at(&self, point: Point) -> Range<usize> {
        let mut st = self.st.borrow_mut();
        let k = st.chunk_at_y(point.y);
        let len = st.len();
        let Some(c) = st.chunks.get(k).cloned() else { return 0..0 };
        // A word broken over lines can reach past the piece: lay out a wider one, from an
        // earlier checkpoint or further on, until it holds the whole word.
        let (mut from, mut end) = (k, c.end);
        loop {
            let Some(f) = st.chunks.get(from).cloned() else { return 0..0 };
            let (piece, local) = if from == k && end == c.end {
                let Some(p) = st.piece(k) else { return 0..0 };
                (p, Point::new(point.x, (f64::from(point.y) - c.dy) as f32))
            } else {
                (st.lay(f.start, end, f.y0), point)
            };
            let w = piece.word_at(local);
            if w.start == 0 && f.start > 0 && from > 0 {
                from -= 1;
            } else if f.start + w.end >= end && end < len {
                end = floor_boundary(st.text.as_str(), end + (end - f.start).max(st.chunk_bytes));
            } else {
                return w.start + f.start..w.end + f.start;
            }
        }
    }

    pub(crate) fn link_rects(&self, link: u64) -> Vec<Rect> {
        // The text the link's spans cover (as the runs carrying it would).
        let range = {
            let st = self.st.borrow();
            let spans: Vec<Range<usize>> = st
                .text
                .spans()
                .iter()
                .filter(|s| s.patch.link == Some(link))
                .map(|s| s.range.clone())
                .collect();
            match (spans.iter().map(|r| r.start).min(), spans.iter().map(|r| r.end).max()) {
                (Some(a), Some(b)) => a..b,
                _ => return Vec::new(),
            }
        };
        self.selection_rects(range)
    }

    pub(crate) fn caret(&self, pos: TextPosition, width: f32) -> Rect {
        let mut st = self.st.borrow_mut();
        let k = st.chunk_at_index(pos.index);
        let (Some(p), Some(c)) = (st.piece(k), st.chunks.get(k)) else { return Rect::ZERO };
        let local = TextPosition { index: pos.index.saturating_sub(c.start), affinity: pos.affinity };
        shift_rect(p.caret(local, width), c.dy as f32)
    }

    pub(crate) fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut st = self.st.borrow_mut();
        st.reach_index(range.end);
        let from = st.chunk_at_index(range.start);
        let to = st.chunk_at_index(range.end);
        let mut out = Vec::new();
        for k in from..=to {
            let Some(p) = st.piece(k) else { continue };
            let Some(c) = st.chunks.get(k) else { continue };
            if k != from && range.end <= c.kept.start && !range.is_empty() {
                continue;
            }
            let (Some(lf), Some(ll)) = (p.line(c.first), p.line(c.first + c.count - 1)) else { continue };
            let (a, b) =
                (range.start.clamp(c.start, c.end) - c.start, range.end.clamp(c.start, c.end) - c.start);
            for r in p.selection_rects(a..b) {
                let mid = (r.min.y + r.max.y) / 2.0;
                if mid >= lf.top && mid < ll.bottom {
                    out.push(shift_rect(r, c.dy as f32));
                }
            }
        }
        out
    }

    /// A movement from `pos` done by `f` in the piece that has `pos`'s line and both its
    /// neighbors as the paragraph has them.
    pub(crate) fn moved(
        &self,
        pos: TextPosition,
        f: impl Fn(&Piece, TextPosition) -> TextPosition,
    ) -> TextPosition {
        let mut st = self.st.borrow_mut();
        let k = st.chunk_around(pos);
        let (Some(p), Some(c)) = (st.piece(k), st.chunks.get(k)) else { return pos };
        let mut out =
            f(&p, TextPosition { index: pos.index.saturating_sub(c.start), affinity: pos.affinity });
        out.index += c.start;
        out
    }
}
