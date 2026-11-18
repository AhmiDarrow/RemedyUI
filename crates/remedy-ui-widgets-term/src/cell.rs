//! What the screen is made of: cells with a character, colors and attributes, and lines of
//! cells.

/// A cell's color as the program chose it: the terminal's default, one of the 256 indexed
/// colors (0 to 15 are the theme's `term-0` to `term-15`), or exact RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TermColor {
    /// The terminal's default foreground or background.
    #[default]
    Default,
    /// An indexed color: 0–15 the theme's ANSI colors, 16–231 the 6×6×6 cube, 232–255 grays.
    Indexed(u8),
    /// True color.
    Rgb(u8, u8, u8),
}

/// How a cell is underlined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    /// Not underlined.
    #[default]
    None,
    /// One straight line.
    Single,
    /// Two lines.
    Double,
    /// A wavy line (spelling and diagnostics in editors).
    Curly,
    /// Dots.
    Dotted,
    /// Dashes.
    Dashed,
}

/// A cell's attributes, as bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs(u16);

impl Attrs {
    /// Bold.
    pub const BOLD: Attrs = Attrs(1);
    /// Dim (faint).
    pub const DIM: Attrs = Attrs(1 << 1);
    /// Italic.
    pub const ITALIC: Attrs = Attrs(1 << 2);
    /// Blinking.
    pub const BLINK: Attrs = Attrs(1 << 3);
    /// Foreground and background swapped.
    pub const INVERSE: Attrs = Attrs(1 << 4);
    /// Hidden (drawn in the background color).
    pub const HIDDEN: Attrs = Attrs(1 << 5);
    /// A line through.
    pub const STRIKE: Attrs = Attrs(1 << 6);
    /// A line over.
    pub const OVERLINE: Attrs = Attrs(1 << 7);
    /// The first half of a character two cells wide.
    pub const WIDE: Attrs = Attrs(1 << 8);
    /// The second half of a character two cells wide (shows nothing of its own).
    pub const WIDE_SPACER: Attrs = Attrs(1 << 9);
    /// A blank left at the end of a line because a wide character did not fit there; it is
    /// not part of the text.
    pub const WIDE_FILLER: Attrs = Attrs(1 << 10);
    const UNDERLINE_SHIFT: u16 = 12;
    const UNDERLINE_MASK: u16 = 0b111 << 12;

    /// No attributes.
    pub const NONE: Attrs = Attrs(0);

    /// True when every attribute in `other` is set.
    pub fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }

    /// These attributes with `other` set or cleared.
    pub fn with(self, other: Attrs, on: bool) -> Attrs {
        if on { Attrs(self.0 | other.0) } else { Attrs(self.0 & !other.0) }
    }

    /// The underline style.
    pub fn underline(self) -> Underline {
        match (self.0 & Self::UNDERLINE_MASK) >> Self::UNDERLINE_SHIFT {
            1 => Underline::Single,
            2 => Underline::Double,
            3 => Underline::Curly,
            4 => Underline::Dotted,
            5 => Underline::Dashed,
            _ => Underline::None,
        }
    }

    /// These attributes with underline style `u`.
    pub fn with_underline(self, u: Underline) -> Attrs {
        let v: u16 = match u {
            Underline::None => 0,
            Underline::Single => 1,
            Underline::Double => 2,
            Underline::Curly => 3,
            Underline::Dotted => 4,
            Underline::Dashed => 5,
        };
        Attrs((self.0 & !Self::UNDERLINE_MASK) | (v << Self::UNDERLINE_SHIFT))
    }
}

/// One cell of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The character (a space when empty). Combining marks on it are kept by its line.
    pub ch: char,
    /// Foreground.
    pub fg: TermColor,
    /// Background.
    pub bg: TermColor,
    /// The underline's own color (SGR 58); `Default` draws it in the foreground.
    pub underline_color: TermColor,
    /// Bold, italic, underline style, …
    pub attrs: Attrs,
    /// The OSC 8 hyperlink the cell belongs to (0 for none).
    pub link: u32,
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

impl Cell {
    /// An empty cell with default colors.
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: TermColor::Default,
        bg: TermColor::Default,
        underline_color: TermColor::Default,
        attrs: Attrs::NONE,
        link: 0,
    };

    /// An empty cell with background `bg` (what erasing leaves, under the current colors).
    pub fn blank(bg: TermColor) -> Cell {
        Cell { bg, ..Cell::BLANK }
    }

    /// True for an empty cell with default colors and no attributes.
    pub fn is_blank(&self) -> bool {
        *self == Cell::BLANK
    }

    /// True when the cell shows nothing but its background (a space, the second half of a
    /// wide character, a filler).
    pub fn is_empty_text(&self) -> bool {
        self.ch == ' ' || self.attrs.contains(Attrs::WIDE_SPACER) || self.attrs.contains(Attrs::WIDE_FILLER)
    }
}

/// A picture a program drew in the terminal (sixel graphics): premultiplied RGBA8 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// A stable id for caches.
    pub id: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Premultiplied RGBA8, row by row.
    pub rgba: std::sync::Arc<[u8]>,
}

/// The piece of a picture one line shows: the picture's `band`th row of cells, from column
/// `col`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PicturePiece {
    /// The first column it covers.
    pub col: usize,
    /// Which row of cells of the picture this is (0 at its top).
    pub band: usize,
    /// How many rows of cells the whole picture covers.
    pub bands: usize,
    /// A cell's size in pixels when the picture was drawn: the picture is shown at that scale.
    pub cell_px: (u32, u32),
    /// The picture.
    pub picture: std::sync::Arc<Picture>,
}

/// A shell's mark on a line (OSC 133, FinalTerm's semantic prompts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptMark {
    /// A prompt starts here (`A`).
    Prompt,
    /// The command people type starts here (`B`).
    Command,
    /// The command's output starts here (`C`).
    Output,
    /// The command finished here (`D`), with its exit status when the shell said.
    Done(Option<i32>),
}

pub(crate) const MARK_PROMPT: u8 = 1;
pub(crate) const MARK_COMMAND: u8 = 2;
pub(crate) const MARK_OUTPUT: u8 = 4;
pub(crate) const MARK_DONE: u8 = 8;

/// Combining marks on one cell (accents, variation selectors, joiners), by column.
pub(crate) type Marks = Vec<(u32, Box<str>)>;

/// One line: its cells, whether it runs on into the next line (a soft wrap), and the
/// combining marks on its cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub(crate) cells: Vec<Cell>,
    pub(crate) wrapped: bool,
    pub(crate) marks: Marks,
    /// The shell's marks on the line (`MARK_*` bits) and the exit status a `D` gave.
    pub(crate) prompt: u8,
    pub(crate) exit: Option<i32>,
    /// Pieces of pictures shown on the line.
    pub(crate) pictures: Vec<PicturePiece>,
}

impl Line {
    /// A line of `cols` cells like `fill`.
    pub(crate) fn new(cols: usize, fill: Cell) -> Line {
        Line { cells: vec![fill; cols], ..Line::default() }
    }

    /// The shell's marks on this line (OSC 133), in the order they happen.
    pub fn prompt_marks(&self) -> Vec<PromptMark> {
        let mut out = Vec::new();
        let has = |b: u8| self.prompt & b != 0;
        if has(MARK_DONE) {
            out.push(PromptMark::Done(self.exit));
        }
        if has(MARK_PROMPT) {
            out.push(PromptMark::Prompt);
        }
        if has(MARK_COMMAND) {
            out.push(PromptMark::Command);
        }
        if has(MARK_OUTPUT) {
            out.push(PromptMark::Output);
        }
        out
    }

    /// The pieces of pictures on this line.
    pub fn pictures(&self) -> &[PicturePiece] {
        &self.pictures
    }

    /// The cells (a line in the scrollback may be shorter than the screen: blank cells at its
    /// end are not kept).
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// True when the line continues on the next one: the text ran past the edge and wrapped.
    pub fn is_wrapped(&self) -> bool {
        self.wrapped
    }

    /// The text of cell `col`: its character and any combining marks on it.
    pub fn grapheme(&self, col: usize) -> Option<String> {
        let c = self.cells.get(col)?;
        let mut s = String::new();
        s.push(c.ch);
        if let Some((_, m)) = self.marks.iter().find(|(at, _)| *at as usize == col) {
            s.push_str(m);
        }
        Some(s)
    }

    /// The combining marks on cell `col`, if any.
    pub(crate) fn marks_at(&self, col: usize) -> Option<&str> {
        if self.marks.is_empty() {
            return None;
        }
        self.marks.iter().find(|(at, _)| *at as usize == col).map(|(_, m)| &**m)
    }

    /// The line's text: each cell's character (wide characters once), with the blank cells at
    /// its end left out.
    pub fn text(&self) -> String {
        self.text_range(0, self.cells.len(), true)
    }

    /// The text of columns `from..to`; `trim` leaves out trailing blanks.
    pub(crate) fn text_range(&self, from: usize, to: usize, trim: bool) -> String {
        let mut s = String::new();
        let to = to.min(self.cells.len());
        for (i, c) in self.cells.iter().enumerate().take(to).skip(from) {
            if c.attrs.contains(Attrs::WIDE_SPACER) || c.attrs.contains(Attrs::WIDE_FILLER) {
                continue;
            }
            s.push(c.ch);
            if let Some(m) = self.marks_at(i) {
                s.push_str(m);
            }
        }
        if trim {
            let t = s.trim_end_matches(' ').len();
            s.truncate(t);
        }
        s
    }

    /// Bytes this line keeps in memory (for the scrollback's budget).
    pub(crate) fn cost(&self) -> usize {
        std::mem::size_of::<Line>()
            + self.cells.capacity() * std::mem::size_of::<Cell>()
            + self.marks.iter().map(|(_, m)| m.len() + std::mem::size_of::<(u32, Box<str>)>()).sum::<usize>()
            + self
                .pictures
                .iter()
                .map(|p| p.picture.rgba.len() / p.bands.max(1) + std::mem::size_of::<PicturePiece>())
                .sum::<usize>()
    }

    /// Drops the combining marks of columns `from..to`.
    pub(crate) fn clear_marks(&mut self, from: usize, to: usize) {
        if !self.marks.is_empty() {
            self.marks.retain(|(c, _)| (*c as usize) < from || (*c as usize) >= to);
        }
    }

    /// Repairs wide characters cut in half by an edit: a first half with no second half, or
    /// a second half with no first, becomes a blank.
    pub(crate) fn fix_wide(&mut self) {
        let n = self.cells.len();
        for i in 0..n {
            let (wide, spacer) = match self.cells.get(i) {
                Some(c) => (c.attrs.contains(Attrs::WIDE), c.attrs.contains(Attrs::WIDE_SPACER)),
                None => continue,
            };
            if wide {
                let ok = self.cells.get(i + 1).is_some_and(|n| n.attrs.contains(Attrs::WIDE_SPACER));
                if !ok && let Some(c) = self.cells.get_mut(i) {
                    *c = Cell::blank(c.bg);
                }
            } else if spacer {
                let ok = i > 0 && self.cells.get(i - 1).is_some_and(|p| p.attrs.contains(Attrs::WIDE));
                if !ok && let Some(c) = self.cells.get_mut(i) {
                    *c = Cell::blank(c.bg);
                }
            }
        }
    }

    /// Makes the line exactly `cols` cells long, cutting or padding with blanks.
    pub(crate) fn set_len(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::BLANK);
        self.clear_marks(cols, usize::MAX);
        self.fix_wide();
    }

    /// The columns holding something other than default blanks: up to the last such cell.
    pub(crate) fn used(&self) -> usize {
        self.cells.iter().rposition(|c| !c.is_blank()).map_or(0, |i| i + 1)
    }
}
