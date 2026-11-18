//! The emulator: a VT and xterm screen model driven by the `vte` parser. It holds the
//! primary and alternate screens, the scrollback, the cursor, scroll regions, tab stops,
//! modes, character sets, colors, hyperlinks and the replies the program asked for.
//!
//! It has no UI and no threads: [`Emulator::feed`] takes the program's bytes and changes the
//! screen. Untrusted input stays bounded: parameters are clamped to the screen, strings (OSC
//! titles, links, clipboard writes) to a limit sized from the machine, and the scrollback to
//! a budget sized from the machine; malformed sequences are dropped.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthChar;

use crate::cell::{Attrs, Cell, Line, TermColor, Underline};

/// How much the scrollback keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scrollback {
    /// A memory budget sized from the machine: 1/256 of its memory (128 MB on a 32 GB
    /// machine), or 64 MB when the platform does not say.
    #[default]
    Auto,
    /// At most this many lines.
    Lines(usize),
    /// At most this many bytes of lines.
    Bytes(usize),
    /// No scrollback: lines leaving the top are gone.
    Off,
}

impl Scrollback {
    /// The (lines, bytes) budget this choice means on this machine.
    fn budget(self) -> (usize, usize) {
        match self {
            Scrollback::Auto => {
                let bytes =
                    machine_memory().map_or(64 << 20, |m| usize::try_from(m / 256).unwrap_or(usize::MAX));
                (usize::MAX, bytes)
            }
            Scrollback::Lines(n) => (n, usize::MAX),
            Scrollback::Bytes(b) => (usize::MAX, b),
            Scrollback::Off => (0, 0),
        }
    }
}

/// The machine's memory, when the platform said.
fn machine_memory() -> Option<u64> {
    remedy_ui_view::device::total_memory()
}

/// The longest string a program may send in one sequence (a title, a link, a clipboard
/// write) by default: 1/2048 of the machine's memory (16 MB on 32 GB), at least 1 MB; 4 MB
/// when the platform does not say. Longer strings are dropped whole.
pub fn default_string_limit() -> usize {
    machine_memory().map_or(4 << 20, |m| usize::try_from(m / 2048).unwrap_or(usize::MAX).max(1 << 20))
}

/// The cursor's shape (DECSCUSR).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// A filled cell.
    #[default]
    Block,
    /// A line under the cell.
    Underline,
    /// A bar at the cell's start.
    Bar,
}

/// Which pointer events the program asked for (DECSET 9, 1000, 1002, 1003).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseMode {
    /// None: the pointer selects text.
    #[default]
    Off,
    /// Presses only (X10).
    X10,
    /// Presses and releases.
    Press,
    /// Presses, releases and moves while a button is down.
    Drag,
    /// Every move.
    Motion,
}

/// How pointer reports are encoded (DECSET 1005, 1006).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MouseEncoding {
    /// Bytes offset by 32 (positions up to 223).
    #[default]
    Default,
    /// UTF-8 encoded positions.
    Utf8,
    /// SGR: `CSI < b ; x ; y M/m`, any position, releases say which button.
    Sgr,
}

/// The modes a program set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Modes {
    /// Cursor keys send application sequences (DECCKM).
    pub app_cursor: bool,
    /// The keypad sends application sequences (DECKPAM).
    pub app_keypad: bool,
    /// Printing inserts instead of overwriting (IRM).
    pub insert: bool,
    /// Line feed also returns the carriage, and Enter sends CR LF (LNM).
    pub newline: bool,
    /// Cursor addressing is relative to the scroll region (DECOM).
    pub origin: bool,
    /// Text wraps at the right edge (DECAWM).
    pub autowrap: bool,
    /// The whole screen is shown in reverse video (DECSCNM).
    pub reverse: bool,
    /// The cursor shows (DECTCEM).
    pub cursor_visible: bool,
    /// The cursor blinks (DECSET 12, or a blinking DECSCUSR shape).
    pub cursor_blink: bool,
    /// Pointer reporting.
    pub mouse: MouseMode,
    /// Pointer report encoding.
    pub mouse_encoding: MouseEncoding,
    /// Focus in and out are reported (DECSET 1004).
    pub focus_events: bool,
    /// Pastes are bracketed (DECSET 2004).
    pub bracketed_paste: bool,
    /// Output is synchronized: the screen is shown when the program ends the update
    /// (DECSET 2026).
    pub synchronized: bool,
    /// The wheel on the alternate screen sends cursor keys (DECSET 1007).
    pub alt_scroll: bool,
    /// The alternate screen is showing.
    pub alt_screen: bool,
    /// Left and right margins can be set (DECLRMM, DECSET 69).
    pub lr_margins: bool,
    /// The program may switch between 80 and 132 columns (DECSET 40).
    pub allow_columns: bool,
    /// The kitty keyboard protocol's enhancement flags in force (0: legacy keys): 1
    /// disambiguate, 2 report event types, 4 report alternate keys, 8 report every key as an
    /// escape code, 16 report associated text.
    pub kitty_flags: u8,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            app_cursor: false,
            app_keypad: false,
            insert: false,
            newline: false,
            origin: false,
            autowrap: true,
            reverse: false,
            cursor_visible: true,
            cursor_blink: false,
            mouse: MouseMode::Off,
            mouse_encoding: MouseEncoding::Default,
            focus_events: false,
            bracketed_paste: false,
            synchronized: false,
            alt_scroll: true,
            alt_screen: false,
            lr_margins: false,
            allow_columns: false,
            kitty_flags: 0,
        }
    }
}

/// Where the cursor is and how it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorState {
    /// Screen row, from 0.
    pub row: usize,
    /// Screen column, from 0.
    pub col: usize,
    /// Shown (DECTCEM).
    pub visible: bool,
    /// Its shape.
    pub shape: CursorShape,
    /// It blinks.
    pub blink: bool,
}

/// A character set (SCS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Charset {
    #[default]
    Ascii,
    /// DEC Special Graphics: line drawing.
    DecGraphics,
    /// United Kingdom: `#` is `£`.
    Uk,
}

impl Charset {
    fn map(self, c: char) -> char {
        match self {
            Charset::Ascii => c,
            Charset::Uk => {
                if c == '#' {
                    '£'
                } else {
                    c
                }
            }
            Charset::DecGraphics => match c {
                '_' => ' ',
                '`' => '◆',
                'a' => '▒',
                'b' => '␉',
                'c' => '␌',
                'd' => '␍',
                'e' => '␊',
                'f' => '°',
                'g' => '±',
                'h' => '␤',
                'i' => '␋',
                'j' => '┘',
                'k' => '┐',
                'l' => '┌',
                'm' => '└',
                'n' => '┼',
                'o' => '⎺',
                'p' => '⎻',
                'q' => '─',
                'r' => '⎼',
                's' => '⎽',
                't' => '├',
                'u' => '┤',
                'v' => '┴',
                'w' => '┬',
                'x' => '│',
                'y' => '≤',
                'z' => '≥',
                '{' => 'π',
                '|' => '≠',
                '}' => '£',
                '~' => '·',
                other => other,
            },
        }
    }
}

/// The colors new cells are written with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pen {
    fg: TermColor,
    bg: TermColor,
    ul: TermColor,
    attrs: Attrs,
    link: u32,
}

impl Pen {
    fn cell(&self, ch: char) -> Cell {
        Cell { ch, fg: self.fg, bg: self.bg, underline_color: self.ul, attrs: self.attrs, link: self.link }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    row: usize,
    col: usize,
    pen: Pen,
    /// The last column was written with autowrap on: the next character wraps first.
    pending: bool,
}

/// What DECSC saves.
#[derive(Clone, Copy, Debug, Default)]
struct Saved {
    cursor: Cursor,
    charsets: [Charset; 4],
    gl: usize,
    origin: bool,
    autowrap: bool,
}

/// An RGB triple.
pub(crate) type Rgb = (u8, u8, u8);

/// The colors queries are answered with: the theme's, as the UI resolved them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Colors {
    pub(crate) palette: Vec<Rgb>,
    pub(crate) fg: Rgb,
    pub(crate) bg: Rgb,
    pub(crate) cursor: Rgb,
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            palette: (0..=255u8).map(xterm_color).collect(),
            fg: (229, 229, 229),
            bg: (0, 0, 0),
            cursor: (229, 229, 229),
        }
    }
}

/// xterm's color for index `i` (the 16 ANSI colors, the cube, the grays).
pub(crate) fn xterm_color(i: u8) -> Rgb {
    const ANSI: [Rgb; 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match i {
        0..=15 => ANSI.get(usize::from(i)).copied().unwrap_or((0, 0, 0)),
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    }
}

/// A hyperlink a program declared (OSC 8).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Link {
    id: Option<String>,
    uri: String,
}

/// The program's hyperlinks, by the id cells carry.
#[derive(Debug, Default)]
struct Links {
    /// Index `n - 1` holds link `n`.
    list: Vec<Option<Link>>,
    lookup: HashMap<(Option<String>, String), u32>,
    /// Links alive after the last sweep: the table is swept again when it doubles.
    live_at_sweep: usize,
}

/// What the program asked of the terminal since the UI last looked.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
    /// Bytes to send back to the program (answers to queries).
    pub(crate) replies: Vec<u8>,
    /// Bells rung.
    pub(crate) bells: u32,
    /// The title changed.
    pub(crate) title: bool,
    /// The program asked to put this on the clipboard (OSC 52); the app decides.
    pub(crate) clipboard: Option<String>,
    /// The palette or default colors changed (OSC 4, 10, 11, 12, 104, 110–112).
    pub(crate) colors: bool,
    /// The working directory changed (OSC 7).
    pub(crate) cwd: bool,
    /// The program switched between 80 and 132 columns (DECCOLM): the view's column count
    /// should follow.
    pub(crate) columns: Option<usize>,
}

/// Where the UI should look: which screen rows changed, and how the scrollback moved.
#[derive(Debug, Default)]
pub(crate) struct Damage {
    /// Screen rows that changed.
    pub(crate) rows: Vec<bool>,
    /// Everything changed (resize, reset, screen switch).
    pub(crate) all: bool,
}

/// The screen model and everything the program set.
pub(crate) struct State {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) grid: Vec<Line>,
    /// The screen not showing (the primary while the alternate shows, and the reverse).
    other: Vec<Line>,
    pub(crate) history: VecDeque<Line>,
    history_bytes: usize,
    /// Lines dropped from the top of the scrollback, ever.
    pub(crate) dropped: u64,
    scrollback: Scrollback,
    budget: (usize, usize),
    cur: Cursor,
    saved: [Saved; 2],
    top: usize,
    bottom: usize,
    /// Left and right margins (DECSLRM): columns `left..right`.
    left: usize,
    right: usize,
    tabs: Vec<bool>,
    pub(crate) modes: Modes,
    pub(crate) shape: CursorShape,
    charsets: [Charset; 4],
    gl: usize,
    single_shift: Option<usize>,
    pub(crate) title: String,
    titles: VecDeque<String>,
    links: Links,
    pub(crate) colors: Colors,
    /// Colors the program changed: palette entries, and fg, bg, cursor (256, 257, 258).
    pub(crate) overrides: HashMap<u16, Rgb>,
    pub(crate) out: Outbox,
    pub(crate) damage: Damage,
    last_char: Option<char>,
    pub(crate) string_limit: usize,
    /// The synchronized update began here.
    pub(crate) sync_since: Option<Instant>,
    /// Pixel size of a cell, as the UI last laid it out (for size reports).
    pub(crate) cell_px: (u32, u32),
    /// A Perform callback ran during the current piece of input.
    acted: bool,
    /// Cell storage of lines the scrollback dropped, reused for new lines.
    spare: Vec<Vec<Cell>>,
    /// The parser is being forced out of an over-long string: what it dispatches is dropped.
    aborting: bool,
    /// The kitty keyboard protocol's flag stacks, for the primary and alternate screens.
    kitty: [Vec<u8>; 2],
    /// The device control string being received.
    dcs: Option<Dcs>,
    /// The working directory the shell reported (OSC 7).
    pub(crate) cwd: Option<String>,
    /// The machine memory the scrollback's automatic budget was sized from.
    budget_memory: Option<u64>,
    /// Columns the program asked for (DECCOLM), overriding the view's.
    pub(crate) forced_cols: Option<usize>,
}

/// A device control string being received.
enum Dcs {
    /// DECRQSS: the setting asked about.
    Request(Vec<u8>),
    /// Sixel graphics.
    Sixel(Box<crate::sixel::Decoder>),
    /// Anything else: parsed and dropped.
    Ignore,
}

impl State {
    fn new(cols: usize, rows: usize) -> State {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let scrollback = Scrollback::Auto;
        State {
            cols,
            rows,
            grid: (0..rows).map(|_| Line::new(cols, Cell::BLANK)).collect(),
            other: (0..rows).map(|_| Line::new(cols, Cell::BLANK)).collect(),
            history: VecDeque::new(),
            history_bytes: 0,
            dropped: 0,
            scrollback,
            budget: scrollback.budget(),
            cur: Cursor::default(),
            saved: [Saved { autowrap: true, ..Saved::default() }; 2],
            top: 0,
            bottom: rows,
            left: 0,
            right: cols,
            tabs: default_tabs(cols),
            modes: Modes::default(),
            shape: CursorShape::Block,
            charsets: [Charset::Ascii; 4],
            gl: 0,
            single_shift: None,
            title: String::new(),
            titles: VecDeque::new(),
            links: Links::default(),
            colors: Colors::default(),
            overrides: HashMap::new(),
            out: Outbox::default(),
            damage: Damage { rows: vec![true; rows], all: true },
            last_char: None,
            string_limit: default_string_limit(),
            sync_since: None,
            cell_px: (0, 0),
            acted: false,
            spare: Vec::new(),
            aborting: false,
            kitty: [Vec::new(), Vec::new()],
            dcs: None,
            cwd: None,
            budget_memory: machine_memory(),
            forced_cols: None,
        }
    }

    // ----- Damage -----

    fn mark(&mut self, row: usize) {
        if let Some(d) = self.damage.rows.get_mut(row) {
            *d = true;
        }
    }

    fn mark_range(&mut self, from: usize, to: usize) {
        for d in self.damage.rows.iter_mut().take(to).skip(from) {
            *d = true;
        }
    }

    fn mark_all(&mut self) {
        self.damage.all = true;
        self.damage.rows.iter_mut().for_each(|d| *d = true);
    }

    // ----- Cells -----

    fn line_mut(&mut self, row: usize) -> Option<&mut Line> {
        self.grid.get_mut(row)
    }

    fn blank_line(&mut self) -> Line {
        let fill = Cell::blank(self.cur.pen.bg);
        // A line the scrollback let go of is reused: no allocation per line of output.
        match self.spare.pop() {
            Some(mut cells) => {
                cells.clear();
                cells.resize(self.cols, fill);
                Line { cells, ..Line::default() }
            }
            None => Line::new(self.cols, fill),
        }
    }

    // ----- Scrolling -----

    /// Adds a line to the scrollback, then drops the oldest lines past the budget.
    fn push_history(&mut self, mut line: Line) {
        let (max_lines, max_bytes) = self.budget;
        if max_lines == 0 || max_bytes == 0 {
            self.dropped += 1;
            return;
        }
        if !line.wrapped {
            let used = line.used();
            line.cells.truncate(used);
            line.clear_marks(used, usize::MAX);
        }
        // Mostly empty lines give their room back; fuller ones keep it, for reuse.
        if line.cells.len() * 2 < line.cells.capacity() {
            line.cells.shrink_to_fit();
        }
        self.history_bytes += line.cost();
        self.history.push_back(line);
        self.enforce_budget();
    }

    fn enforce_budget(&mut self) {
        let (max_lines, max_bytes) = self.budget;
        while self.history.len() > max_lines || (self.history_bytes > max_bytes && !self.history.is_empty()) {
            if let Some(l) = self.history.pop_front() {
                self.history_bytes = self.history_bytes.saturating_sub(l.cost());
                self.dropped += 1;
                // Kept for the next blank line: at most a screenful of them.
                if self.spare.len() < self.rows {
                    self.spare.push(l.cells);
                }
            }
        }
    }

    /// Scrolls the region `top..bottom` up by `n`: lines leave at its top (into the
    /// scrollback when the region is the whole primary screen) and blanks come in at its
    /// bottom.
    fn scroll_up_in(&mut self, top: usize, bottom: usize, n: usize, keep: bool) {
        let bottom = bottom.min(self.rows);
        if top >= bottom {
            return;
        }
        let n = n.min(bottom - top);
        if self.partial_margins() {
            self.scroll_band(top, bottom, n, true);
            return;
        }
        let to_history = keep && top == 0 && bottom == self.rows && !self.modes.alt_screen;
        let blank = self.blank_line();
        let gone: Vec<Line> = self.grid.splice(top..top + n, std::iter::empty()).collect();
        let at = bottom - n;
        self.grid.splice(at..at, std::iter::repeat_n(blank, n));
        if to_history {
            for l in gone {
                self.push_history(l);
            }
            // Lines kept their place in the whole text: only the new ones changed.
            self.damage.rows.drain(0..n.min(self.damage.rows.len()));
            self.damage.rows.extend(std::iter::repeat_n(true, n));
        } else {
            self.mark_range(top, bottom);
        }
    }

    /// Scrolls the region `top..bottom` down by `n`: blanks come in at its top.
    fn scroll_down_in(&mut self, top: usize, bottom: usize, n: usize) {
        let bottom = bottom.min(self.rows);
        if top >= bottom {
            return;
        }
        let n = n.min(bottom - top);
        if self.partial_margins() {
            self.scroll_band(top, bottom, n, false);
            return;
        }
        let blank = self.blank_line();
        self.grid.splice(bottom - n..bottom, std::iter::empty());
        self.grid.splice(top..top, std::iter::repeat_n(blank, n));
        self.mark_range(top, bottom);
    }

    /// Left and right margins narrower than the screen are in force.
    fn partial_margins(&self) -> bool {
        self.left > 0 || self.right < self.cols
    }

    /// Scrolls only columns `left..right` of rows `top..bottom` by `n`, up or down: the
    /// rest of each row stays (DECLRMM). Nothing goes to the scrollback.
    fn scroll_band(&mut self, top: usize, bottom: usize, n: usize, up: bool) {
        let (l, r) = (self.left, self.right.min(self.cols));
        let blank = Cell::blank(self.cur.pen.bg);
        let rows: Vec<usize> = if up { (top..bottom).collect() } else { (top..bottom).rev().collect() };
        for row in rows {
            let src = if up {
                row.checked_add(n).filter(|s| *s < bottom)
            } else {
                row.checked_sub(n).filter(|s| *s >= top)
            };
            let (cells, marks): (Vec<Cell>, Vec<(u32, Box<str>)>) = match src.and_then(|s| self.grid.get(s)) {
                Some(line) => (
                    line.cells.get(l..r).map(<[Cell]>::to_vec).unwrap_or_default(),
                    line.marks
                        .iter()
                        .filter(|(c, _)| (*c as usize) >= l && (*c as usize) < r)
                        .cloned()
                        .collect(),
                ),
                None => (vec![blank; r.saturating_sub(l)], Vec::new()),
            };
            if let Some(line) = self.grid.get_mut(row) {
                line.clear_marks(l, r);
                if let Some(dst) = line.cells.get_mut(l..r) {
                    for (d, c) in dst.iter_mut().zip(cells) {
                        *d = c;
                    }
                }
                line.marks.extend(marks);
                line.fix_wide();
            }
        }
        self.mark_range(top, bottom);
    }

    fn linefeed(&mut self) {
        self.cur.pending = false;
        if self.cur.row + 1 == self.bottom {
            self.scroll_up_in(self.top, self.bottom, 1, true);
        } else if self.cur.row + 1 < self.rows {
            self.cur.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        self.cur.pending = false;
        if self.cur.row == self.top {
            self.scroll_down_in(self.top, self.bottom, 1);
        } else if self.cur.row > 0 {
            self.cur.row -= 1;
        }
    }

    // ----- Printing -----

    fn print_char(&mut self, c: char) {
        let set = self.single_shift.take().unwrap_or(self.gl);
        let c =
            if (c as u32) < 0x80 { self.charsets.get(set).copied().unwrap_or_default().map(c) } else { c };
        // A character after a zero-width joiner, or a regional indicator after another,
        // belongs to the grapheme before it (family emoji, flags): it joins that cell.
        if self.joins_previous(c) {
            self.combine(c);
            return;
        }
        let width = match c.width() {
            Some(w) => w,
            None => return,
        };
        if width == 0 {
            self.combine(c);
            return;
        }
        self.last_char = Some(c);
        self.put(c, width.min(2));
    }

    /// The cell the last character went into, when it is just before the cursor.
    fn previous_cell(&self) -> Option<(usize, usize)> {
        if !self.cur.pending && self.cur.col == 0 {
            return None;
        }
        let col = if self.cur.pending { self.cur.col } else { self.cur.col - 1 };
        let line = self.grid.get(self.cur.row)?;
        let col = match line.cells.get(col) {
            Some(cell) if cell.attrs.contains(Attrs::WIDE_SPACER) => col.checked_sub(1)?,
            Some(_) => col,
            None => return None,
        };
        Some((self.cur.row, col))
    }

    fn joins_previous(&self, c: char) -> bool {
        let Some((row, col)) = self.previous_cell() else { return false };
        let Some(line) = self.grid.get(row) else { return false };
        let marks = line.marks_at(col);
        if marks.is_some_and(|m| m.ends_with('\u{200D}')) {
            return true;
        }
        let ri = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
        // Two regional indicators make one flag; a third starts the next.
        ri(c) && marks.is_none() && line.cells.get(col).is_some_and(|cell| ri(cell.ch))
    }

    /// Adds a combining mark to the character before the cursor.
    fn combine(&mut self, c: char) {
        let col = if self.cur.pending { self.cur.col } else { self.cur.col.saturating_sub(1) };
        if !self.cur.pending && self.cur.col == 0 {
            return;
        }
        let row = self.cur.row;
        let Some(line) = self.grid.get_mut(row) else { return };
        let col = match line.cells.get(col) {
            Some(cell) if cell.attrs.contains(Attrs::WIDE_SPACER) => col.saturating_sub(1),
            Some(_) => col,
            None => return,
        };
        let key = u32::try_from(col).unwrap_or(u32::MAX);
        match line.marks.iter_mut().find(|(at, _)| *at == key) {
            Some((_, m)) => {
                // UAX #15's stream-safe text format allows 30 marks in a row; more are
                // dropped (a stack of marks is a known way to hang text renderers).
                if m.chars().count() < 30 {
                    let mut s = m.to_string();
                    s.push(c);
                    *m = s.into_boxed_str();
                }
            }
            None => line.marks.push((key, c.to_string().into_boxed_str())),
        }
        self.mark(row);
    }

    fn put(&mut self, c: char, width: usize) {
        // Text wraps at the right margin while the cursor is inside the margins.
        let cols =
            if self.cur.col >= self.left && self.cur.col < self.right { self.right } else { self.cols };
        if self.cur.pending && self.modes.autowrap {
            if let Some(l) = self.line_mut(self.cur.row) {
                l.wrapped = true;
            }
            self.carriage_return();
            self.linefeed();
        }
        if width == 2 && self.cur.col + 1 >= cols {
            if self.modes.autowrap && cols >= 2 {
                // It does not fit: a filler here, and the character starts the next line.
                let row = self.cur.row;
                let pen = self.cur.pen;
                if let Some(l) = self.line_mut(row) {
                    if let Some(cell) = l.cells.get_mut(cols - 1) {
                        *cell = Cell { attrs: Attrs::WIDE_FILLER, ..Cell::blank(pen.bg) };
                    }
                    l.wrapped = true;
                    l.fix_wide();
                }
                self.mark(row);
                self.carriage_return();
                self.linefeed();
            } else {
                // No room and no wrapping: a wide character cannot show.
                return;
            }
        }
        let row = self.cur.row;
        let col = self.cur.col;
        let pen = self.cur.pen;
        let insert = self.modes.insert;
        let Some(line) = self.grid.get_mut(row) else { return };
        if insert {
            // Inserting shifts what follows up to the right edge (the margin), no further.
            let at = col.min(line.cells.len());
            let edge = cols.min(line.cells.len());
            for _ in 0..width {
                if at < edge {
                    line.cells.insert(at, Cell::blank(pen.bg));
                    line.cells.remove(edge);
                }
            }
            if !line.marks.is_empty() {
                for (m, _) in &mut line.marks {
                    if (*m as usize) >= col {
                        *m = m.saturating_add(u32::try_from(width).unwrap_or(0));
                    }
                }
                line.clear_marks(cols, usize::MAX);
            }
        }
        line.clear_marks(col, col + width);
        let mut first = pen.cell(c);
        if width == 2 {
            first.attrs = first.attrs.with(Attrs::WIDE, true);
        }
        if let Some(cell) = line.cells.get_mut(col) {
            *cell = first;
        }
        if width == 2
            && let Some(cell) = line.cells.get_mut(col + 1)
        {
            *cell = Cell { ch: ' ', attrs: pen.attrs.with(Attrs::WIDE_SPACER, true), ..pen.cell(' ') };
        }
        // Writing over half of a wide character blanks its other half.
        let before =
            col.checked_sub(1).and_then(|p| line.cells.get(p)).is_some_and(|p| p.attrs.contains(Attrs::WIDE));
        if before || line.cells.get(col + width).is_some_and(|n| n.attrs.contains(Attrs::WIDE_SPACER)) {
            line.fix_wide();
        }
        self.mark(row);
        let next = col + width;
        if next >= cols {
            self.cur.col = cols - 1;
            self.cur.pending = self.modes.autowrap;
        } else {
            self.cur.col = next;
        }
    }

    fn carriage_return(&mut self) {
        // To the left margin when the cursor is at or right of it, else to column 0.
        self.cur.col = if self.cur.col >= self.left { self.left } else { 0 };
        self.cur.pending = false;
    }

    // ----- Cursor movement -----

    fn goto(&mut self, row: usize, col: usize) {
        let (lo, hi) = if self.modes.origin { (self.top, self.bottom) } else { (0, self.rows) };
        let (cl, cr) = if self.modes.origin { (self.left, self.right) } else { (0, self.cols) };
        self.cur.row = (lo + row).min(hi.saturating_sub(1)).max(lo);
        self.cur.col = cl.saturating_add(col).min(cr.saturating_sub(1)).max(cl);
        self.cur.pending = false;
    }

    /// Moves to column `col` of the line (CHA, HPA): inside the margins in origin mode.
    fn goto_col(&mut self, col: usize) {
        let (cl, cr) = if self.modes.origin { (self.left, self.right) } else { (0, self.cols) };
        self.cur.col = cl.saturating_add(col).min(cr.saturating_sub(1)).max(cl);
        self.cur.pending = false;
    }

    fn goto_row(&mut self, row: usize) {
        let col = self.cur.col;
        self.goto(row, col);
    }

    fn up(&mut self, n: usize) {
        let lo = if self.cur.row >= self.top { self.top } else { 0 };
        self.cur.row = self.cur.row.saturating_sub(n).max(lo);
        self.cur.pending = false;
    }

    fn down(&mut self, n: usize) {
        let hi = if self.cur.row < self.bottom { self.bottom } else { self.rows };
        self.cur.row = self.cur.row.saturating_add(n).min(hi - 1);
        self.cur.pending = false;
    }

    fn forward(&mut self, n: usize) {
        // Inside the margins the cursor stops at the right one.
        let hi = if self.cur.col < self.right { self.right } else { self.cols };
        self.cur.col = self.cur.col.saturating_add(n).min(hi - 1);
        self.cur.pending = false;
    }

    fn backward(&mut self, n: usize) {
        let lo = if self.cur.col >= self.left { self.left } else { 0 };
        self.cur.col = self.cur.col.saturating_sub(n).max(lo);
        self.cur.pending = false;
    }

    fn tab(&mut self, n: usize) {
        let edge = if self.cur.col < self.right { self.right } else { self.cols };
        for _ in 0..n {
            let next = (self.cur.col + 1..edge).find(|c| self.tabs.get(*c).copied().unwrap_or(false));
            self.cur.col = next.unwrap_or(edge - 1);
            if next.is_none() {
                break;
            }
        }
        self.cur.pending = false;
    }

    fn back_tab(&mut self, n: usize) {
        for _ in 0..n {
            let prev = (0..self.cur.col).rev().find(|c| self.tabs.get(*c).copied().unwrap_or(false));
            self.cur.col = prev.unwrap_or(0);
            if prev.is_none() {
                break;
            }
        }
        self.cur.pending = false;
    }

    // ----- Erasing and editing -----

    /// Blanks columns `from..to` of `row` with the current background.
    fn erase_cells(&mut self, row: usize, from: usize, to: usize) {
        let bg = self.cur.pen.bg;
        if let Some(line) = self.grid.get_mut(row) {
            let to = to.min(line.cells.len());
            for c in line.cells.iter_mut().take(to).skip(from) {
                *c = Cell::blank(bg);
            }
            line.clear_marks(from, to);
            line.fix_wide();
            if to >= line.cells.len() {
                line.wrapped = false;
            }
            line.pictures.retain(|p| p.col < from || p.col >= to);
            if from == 0 && to >= line.cells.len() {
                line.prompt = 0;
                line.exit = None;
            }
        }
        self.mark(row);
    }

    fn erase_line(&mut self, mode: usize) {
        let (row, col) = (self.cur.row, self.cur.col);
        match mode {
            0 => self.erase_cells(row, col, self.cols),
            1 => self.erase_cells(row, 0, col + 1),
            2 => self.erase_cells(row, 0, self.cols),
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: usize) {
        let (row, col) = (self.cur.row, self.cur.col);
        match mode {
            0 => {
                self.erase_cells(row, col, self.cols);
                for r in row + 1..self.rows {
                    self.erase_cells(r, 0, self.cols);
                }
            }
            1 => {
                for r in 0..row {
                    self.erase_cells(r, 0, self.cols);
                }
                self.erase_cells(row, 0, col + 1);
            }
            2 => {
                for r in 0..self.rows {
                    self.erase_cells(r, 0, self.cols);
                }
            }
            3 => {
                // xterm: erase the scrollback. Its lines count as dropped, so a line number
                // never names two different lines.
                self.dropped += self.history.len() as u64;
                self.history.clear();
                self.history_bytes = 0;
                self.mark_all();
            }
            _ => {}
        }
    }

    fn insert_chars(&mut self, n: usize) {
        // What shifts stops at the right margin when the cursor is inside the margins.
        let edge =
            if self.cur.col >= self.left && self.cur.col < self.right { self.right } else { self.cols };
        if self.cur.col >= edge {
            return;
        }
        let (row, col, cols, bg) = (self.cur.row, self.cur.col, edge, self.cur.pen.bg);
        if let Some(line) = self.grid.get_mut(row) {
            let n = n.min(cols - col);
            for _ in 0..n {
                line.cells.insert(col, Cell::blank(bg));
                line.cells.remove(cols);
            }
            if !line.marks.is_empty() {
                // Marks pushed past the edge go; those between move along.
                line.clear_marks(cols - n, cols);
                let shift = u32::try_from(n).unwrap_or(u32::MAX);
                for (m, _) in &mut line.marks {
                    if (*m as usize) >= col && (*m as usize) < cols {
                        *m = m.saturating_add(shift);
                    }
                }
            }
            line.fix_wide();
            line.wrapped = false;
        }
        self.cur.pending = false;
        self.mark(row);
    }

    fn delete_chars(&mut self, n: usize) {
        let edge =
            if self.cur.col >= self.left && self.cur.col < self.right { self.right } else { self.cols };
        if self.cur.col >= edge {
            return;
        }
        let (row, col, cols, bg) = (self.cur.row, self.cur.col, edge, self.cur.pen.bg);
        if let Some(line) = self.grid.get_mut(row) {
            let n = n.min(cols - col);
            line.clear_marks(col, col + n);
            line.cells.drain(col..(col + n).min(line.cells.len()));
            for _ in 0..n {
                line.cells.insert(cols - n, Cell::blank(bg));
            }
            if !line.marks.is_empty() {
                let shift = u32::try_from(n).unwrap_or(u32::MAX);
                for (m, _) in &mut line.marks {
                    if (*m as usize) >= col + n && (*m as usize) < cols {
                        *m -= shift;
                    }
                }
            }
            line.fix_wide();
            line.wrapped = false;
        }
        self.cur.pending = false;
        self.mark(row);
    }

    fn insert_lines(&mut self, n: usize) {
        if self.cur.row < self.top
            || self.cur.row >= self.bottom
            || self.cur.col < self.left
            || self.cur.col >= self.right
        {
            return;
        }
        self.scroll_down_in(self.cur.row, self.bottom, n);
        self.cur.col = self.left;
        self.cur.pending = false;
    }

    fn delete_lines(&mut self, n: usize) {
        if self.cur.row < self.top
            || self.cur.row >= self.bottom
            || self.cur.col < self.left
            || self.cur.col >= self.right
        {
            return;
        }
        // Lines deleted inside the screen never go to the scrollback.
        self.scroll_up_in(self.cur.row, self.bottom, n, false);
        self.cur.col = self.left;
        self.cur.pending = false;
    }

    fn repeat(&mut self, n: usize) {
        let Some(c) = self.last_char else { return };
        // Beyond a screenful, repeating only pushes copies of the same line into the
        // scrollback: a ten-byte sequence must not cost millions of cells.
        let n = n.min(self.rows.saturating_mul(self.cols));
        for _ in 0..n {
            self.print_char(c);
        }
    }

    fn screen_alignment(&mut self) {
        let e = Cell { ch: 'E', ..Cell::BLANK };
        for line in &mut self.grid {
            line.cells.iter_mut().for_each(|c| *c = e);
            line.marks.clear();
            line.wrapped = false;
        }
        self.top = 0;
        self.bottom = self.rows;
        self.left = 0;
        self.right = self.cols;
        self.cur = Cursor { pen: self.cur.pen, ..Cursor::default() };
        self.mark_all();
    }

    // ----- Screens and saving -----

    fn save_cursor(&mut self) {
        let i = usize::from(self.modes.alt_screen);
        if let Some(s) = self.saved.get_mut(i) {
            *s = Saved {
                cursor: self.cur,
                charsets: self.charsets,
                gl: self.gl,
                origin: self.modes.origin,
                autowrap: self.modes.autowrap,
            };
        }
    }

    fn restore_cursor(&mut self) {
        let i = usize::from(self.modes.alt_screen);
        let s = self.saved.get(i).copied().unwrap_or_default();
        self.cur = s.cursor;
        self.cur.row = self.cur.row.min(self.rows - 1);
        self.cur.col = self.cur.col.min(self.cols - 1);
        self.charsets = s.charsets;
        self.gl = s.gl;
        self.modes.origin = s.origin;
        self.modes.autowrap = s.autowrap;
    }

    fn set_alt_screen(&mut self, on: bool, clear: bool) {
        if self.modes.alt_screen == on {
            if on && clear {
                for r in 0..self.rows {
                    self.erase_cells(r, 0, self.cols);
                }
            }
            return;
        }
        std::mem::swap(&mut self.grid, &mut self.other);
        self.modes.alt_screen = on;
        // Each screen has its own keyboard flags.
        self.modes.kitty_flags = self.kitty_stack().last().copied().unwrap_or(0);
        if on && clear {
            let blank = self.blank_line();
            self.grid.iter_mut().for_each(|l| *l = blank.clone());
        }
        self.top = 0;
        self.bottom = self.rows;
        self.mark_all();
    }

    fn reset(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        let history = std::mem::take(&mut self.history);
        let (bytes, dropped, scrollback, limit, colors, cell_px) = (
            self.history_bytes,
            self.dropped,
            self.scrollback,
            self.string_limit,
            self.colors.clone(),
            self.cell_px,
        );
        let out = std::mem::take(&mut self.out);
        *self = State::new(cols, rows);
        self.history = history;
        self.history_bytes = bytes;
        self.dropped = dropped;
        self.scrollback = scrollback;
        self.budget = scrollback.budget();
        self.string_limit = limit;
        self.colors = colors;
        self.cell_px = cell_px;
        self.out = out;
        self.out.title = true;
        self.out.colors = true;
    }

    fn soft_reset(&mut self) {
        self.modes.cursor_visible = true;
        self.modes.insert = false;
        self.modes.origin = false;
        self.modes.autowrap = true;
        self.modes.app_cursor = false;
        self.modes.app_keypad = false;
        self.top = 0;
        self.bottom = self.rows;
        self.left = 0;
        self.right = self.cols;
        self.cur.pen = Pen::default();
        self.charsets = [Charset::Ascii; 4];
        self.gl = 0;
        self.saved = [Saved { autowrap: true, ..Saved::default() }; 2];
    }

    // ----- Replies -----

    fn reply(&mut self, bytes: &[u8]) {
        // A program that floods queries without reading the answers gets no more of them.
        if self.out.replies.len() + bytes.len() <= self.string_limit {
            self.out.replies.extend_from_slice(bytes);
        }
    }

    // ----- Resizing -----

    pub(crate) fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        let alt = self.modes.alt_screen;
        // The primary screen reflows: wrapped lines join and wrap again at the new width.
        let (primary, cursor) = if alt {
            let s = self.saved.first().copied().unwrap_or_default().cursor;
            (std::mem::take(&mut self.other), (s.row, s.col))
        } else {
            (std::mem::take(&mut self.grid), (self.cur.row, self.cur.col))
        };
        let history = std::mem::take(&mut self.history);
        self.history_bytes = 0;
        let (screen, history, (crow, ccol)) = reflow(history, primary, cursor, cols, rows);
        for l in history {
            self.history_bytes += l.cost();
            self.history.push_back(l);
        }
        self.cols = cols;
        self.rows = rows;
        self.enforce_budget();
        if alt {
            self.other = screen;
            if let Some(s) = self.saved.get_mut(0) {
                s.cursor.row = crow;
                s.cursor.col = ccol;
                s.cursor.pending = false;
            }
            // The alternate screen is cut or padded, never reflowed.
            let blank = Line::new(cols, Cell::BLANK);
            self.grid.resize(rows, blank);
            for l in &mut self.grid {
                l.set_len(cols);
                l.wrapped = false;
            }
            self.cur.row = self.cur.row.min(rows - 1);
            self.cur.col = self.cur.col.min(cols - 1);
        } else {
            self.grid = screen;
            self.cur.row = crow;
            self.cur.col = ccol;
            let blank = Line::new(cols, Cell::BLANK);
            self.other.resize(rows, blank);
            for l in &mut self.other {
                l.set_len(cols);
            }
        }
        self.cur.pending = false;
        if let Some(s) = self.saved.get_mut(1) {
            s.cursor.row = s.cursor.row.min(rows - 1);
            s.cursor.col = s.cursor.col.min(cols - 1);
        }
        self.top = 0;
        self.bottom = rows;
        self.left = 0;
        self.right = cols;
        let mut tabs = default_tabs(cols);
        for (t, old) in tabs.iter_mut().zip(self.tabs.iter()) {
            *t = *old;
        }
        self.tabs = tabs;
        self.damage.rows = vec![true; rows];
        self.mark_all();
    }

    pub(crate) fn set_scrollback(&mut self, s: Scrollback) {
        self.scrollback = s;
        self.budget = s.budget();
        let before = self.history.len();
        self.enforce_budget();
        if self.history.len() != before {
            self.mark_all();
        }
    }

    // ----- Links -----

    fn link_id(&mut self, id: Option<String>, uri: String) -> u32 {
        let key = (id.clone(), uri.clone());
        if let Some(n) = self.links.lookup.get(&key) {
            return *n;
        }
        if self.links.list.len() >= 2 * self.links.live_at_sweep + 64 {
            self.sweep_links();
        }
        self.links.list.push(Some(Link { id, uri }));
        let n = u32::try_from(self.links.list.len()).unwrap_or(u32::MAX);
        self.links.lookup.insert(key, n);
        n
    }

    /// Forgets links no cell points at any more (they scrolled away or were overwritten),
    /// renumbering the rest.
    fn sweep_links(&mut self) {
        let mut used = vec![false; self.links.list.len() + 1];
        let current = self.cur.pen.link;
        let mut note = |id: u32| {
            if let Some(u) = used.get_mut(id as usize) {
                *u = true;
            }
        };
        note(current);
        for l in self.history.iter().chain(self.grid.iter()).chain(self.other.iter()) {
            for c in &l.cells {
                if c.link != 0 {
                    note(c.link);
                }
            }
        }
        let mut map = vec![0u32; self.links.list.len() + 1];
        let mut list = Vec::new();
        for (i, link) in self.links.list.iter_mut().enumerate() {
            if used.get(i + 1).copied().unwrap_or(false)
                && let Some(l) = link.take()
            {
                list.push(Some(l));
                if let Some(m) = map.get_mut(i + 1) {
                    *m = u32::try_from(list.len()).unwrap_or(u32::MAX);
                }
            }
        }
        let remap = |id: &mut u32| {
            if *id != 0 {
                *id = map.get(*id as usize).copied().unwrap_or(0);
            }
        };
        for l in self.history.iter_mut().chain(self.grid.iter_mut()).chain(self.other.iter_mut()) {
            for c in &mut l.cells {
                remap(&mut c.link);
            }
        }
        remap(&mut self.cur.pen.link);
        for s in &mut self.saved {
            remap(&mut s.cursor.pen.link);
        }
        self.links.lookup = list
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.as_ref().map(|l| ((l.id.clone(), l.uri.clone()), i as u32 + 1)))
            .collect();
        self.links.live_at_sweep = list.len();
        self.links.list = list;
    }

    pub(crate) fn link_uri(&self, id: u32) -> Option<&str> {
        let i = (id as usize).checked_sub(1)?;
        self.links.list.get(i)?.as_ref().map(|l| l.uri.as_str())
    }

    // ----- Colors -----

    pub(crate) fn color_of(&self, slot: u16) -> Rgb {
        if let Some(c) = self.overrides.get(&slot) {
            return *c;
        }
        match slot {
            256 => self.colors.fg,
            257 => self.colors.bg,
            258 => self.colors.cursor,
            i => self.colors.palette.get(usize::from(i)).copied().unwrap_or((0, 0, 0)),
        }
    }

    fn color_reply(&mut self, prefix: &str, slot: u16, bell: bool) {
        let (r, g, b) = self.color_of(slot);
        let end = if bell { "\x07" } else { "\x1b\\" };
        let s = format!("\x1b]{prefix};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}{end}");
        self.reply(s.as_bytes());
    }

    // ----- SGR -----

    fn sgr(&mut self, params: &vte::Params) {
        let mut it = params.iter().peekable();
        if params.is_empty() {
            self.cur.pen = Pen { link: self.cur.pen.link, ..Pen::default() };
            return;
        }
        while let Some(p) = it.next() {
            let first = p.first().copied().unwrap_or(0);
            let pen = &mut self.cur.pen;
            match first {
                0 => *pen = Pen { link: pen.link, ..Pen::default() },
                1 => pen.attrs = pen.attrs.with(Attrs::BOLD, true),
                2 => pen.attrs = pen.attrs.with(Attrs::DIM, true),
                3 => pen.attrs = pen.attrs.with(Attrs::ITALIC, true),
                4 => {
                    let u = match p.get(1) {
                        None => Underline::Single,
                        Some(0) => Underline::None,
                        Some(1) => Underline::Single,
                        Some(2) => Underline::Double,
                        Some(3) => Underline::Curly,
                        Some(4) => Underline::Dotted,
                        Some(5) => Underline::Dashed,
                        Some(_) => pen.attrs.underline(),
                    };
                    pen.attrs = pen.attrs.with_underline(u);
                }
                5 | 6 => pen.attrs = pen.attrs.with(Attrs::BLINK, true),
                7 => pen.attrs = pen.attrs.with(Attrs::INVERSE, true),
                8 => pen.attrs = pen.attrs.with(Attrs::HIDDEN, true),
                9 => pen.attrs = pen.attrs.with(Attrs::STRIKE, true),
                21 => pen.attrs = pen.attrs.with_underline(Underline::Double),
                22 => pen.attrs = pen.attrs.with(Attrs::BOLD, false).with(Attrs::DIM, false),
                23 => pen.attrs = pen.attrs.with(Attrs::ITALIC, false),
                24 => pen.attrs = pen.attrs.with_underline(Underline::None),
                25 => pen.attrs = pen.attrs.with(Attrs::BLINK, false),
                27 => pen.attrs = pen.attrs.with(Attrs::INVERSE, false),
                28 => pen.attrs = pen.attrs.with(Attrs::HIDDEN, false),
                29 => pen.attrs = pen.attrs.with(Attrs::STRIKE, false),
                30..=37 => pen.fg = TermColor::Indexed((first - 30) as u8),
                39 => pen.fg = TermColor::Default,
                40..=47 => pen.bg = TermColor::Indexed((first - 40) as u8),
                49 => pen.bg = TermColor::Default,
                53 => pen.attrs = pen.attrs.with(Attrs::OVERLINE, true),
                55 => pen.attrs = pen.attrs.with(Attrs::OVERLINE, false),
                59 => pen.ul = TermColor::Default,
                90..=97 => pen.fg = TermColor::Indexed((first - 90 + 8) as u8),
                100..=107 => pen.bg = TermColor::Indexed((first - 100 + 8) as u8),
                38 | 48 | 58 => {
                    let color = if p.len() > 1 {
                        // Colon form: 38:5:n, 38:2:r:g:b or 38:2::r:g:b.
                        extended_color(p.get(1..).unwrap_or_default())
                    } else {
                        // Semicolon form: the color's parameters follow as their own.
                        let kind = it.peek().and_then(|n| n.first().copied());
                        match kind {
                            Some(5) => {
                                let _ = it.next();
                                it.next()
                                    .and_then(|n| n.first().copied())
                                    .and_then(|n| u8::try_from(n).ok())
                                    .map(TermColor::Indexed)
                            }
                            Some(2) => {
                                let _ = it.next();
                                let mut v = [0u16; 3];
                                let mut ok = true;
                                for slot in &mut v {
                                    match it.next().and_then(|n| n.first().copied()) {
                                        Some(x) => *slot = x,
                                        None => ok = false,
                                    }
                                }
                                let [r, g, b] = v;
                                (ok && r < 256 && g < 256 && b < 256)
                                    .then_some(TermColor::Rgb(r as u8, g as u8, b as u8))
                            }
                            _ => None,
                        }
                    };
                    if let Some(c) = color {
                        let pen = &mut self.cur.pen;
                        match first {
                            38 => pen.fg = c,
                            48 => pen.bg = c,
                            _ => pen.ul = c,
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // ----- Modes -----

    fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.modes.app_cursor = on,
            5 => {
                if self.modes.reverse != on {
                    self.modes.reverse = on;
                    self.mark_all();
                }
            }
            6 => {
                self.modes.origin = on;
                self.goto(0, 0);
            }
            7 => self.modes.autowrap = on,
            12 => self.modes.cursor_blink = on,
            25 => self.modes.cursor_visible = on,
            9 => self.modes.mouse = if on { MouseMode::X10 } else { MouseMode::Off },
            1000 => self.modes.mouse = if on { MouseMode::Press } else { MouseMode::Off },
            1002 => self.modes.mouse = if on { MouseMode::Drag } else { MouseMode::Off },
            1003 => self.modes.mouse = if on { MouseMode::Motion } else { MouseMode::Off },
            1004 => self.modes.focus_events = on,
            1005 => {
                self.modes.mouse_encoding = if on { MouseEncoding::Utf8 } else { MouseEncoding::Default };
            }
            1006 => {
                self.modes.mouse_encoding = if on { MouseEncoding::Sgr } else { MouseEncoding::Default };
            }
            1007 => self.modes.alt_scroll = on,
            40 => self.modes.allow_columns = on,
            3 if self.modes.allow_columns => self.switch_columns(if on { 132 } else { 80 }),
            69 => {
                self.modes.lr_margins = on;
                if !on {
                    self.left = 0;
                    self.right = self.cols;
                }
            }
            47 | 1047 => self.set_alt_screen(on, on && mode == 1047),
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if on {
                    self.save_cursor();
                    self.set_alt_screen(true, true);
                } else {
                    self.set_alt_screen(false, false);
                    self.restore_cursor();
                }
            }
            2004 => self.modes.bracketed_paste = on,
            2026 => {
                self.modes.synchronized = on;
                self.sync_since = on.then(Instant::now);
                if !on {
                    self.mark_all();
                }
            }
            _ => {}
        }
    }

    /// DECRQM's answer for a private mode: 1 set, 2 reset, 0 not known.
    fn private_mode_state(&self, mode: u16) -> u8 {
        let m = &self.modes;
        let v = match mode {
            1 => m.app_cursor,
            5 => m.reverse,
            6 => m.origin,
            7 => m.autowrap,
            12 => m.cursor_blink,
            25 => m.cursor_visible,
            9 => m.mouse == MouseMode::X10,
            1000 => m.mouse == MouseMode::Press,
            1002 => m.mouse == MouseMode::Drag,
            1003 => m.mouse == MouseMode::Motion,
            1004 => m.focus_events,
            1005 => m.mouse_encoding == MouseEncoding::Utf8,
            1006 => m.mouse_encoding == MouseEncoding::Sgr,
            1007 => m.alt_scroll,
            3 => self.cols == 132,
            40 => m.allow_columns,
            69 => m.lr_margins,
            47 | 1047 | 1049 => m.alt_screen,
            2004 => m.bracketed_paste,
            2026 => m.synchronized,
            _ => return 0,
        };
        if v { 1 } else { 2 }
    }

    /// A cell's size in pixels as the UI laid it out, or a nominal 10 × 20 before it has.
    fn cell_px_or_nominal(&self) -> (u32, u32) {
        match self.cell_px {
            (0, _) | (_, 0) => (10, 20),
            p => p,
        }
    }

    /// Shows a sixel picture of `w` × `h` pixels at the cursor: it covers the cells it
    /// needs, and the cursor ends on the line below it (scrolling as text would).
    fn place_picture(&mut self, w: u32, h: u32, rgba: Vec<u8>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let (cw, ch) = self.cell_px_or_nominal();
        let cols_needed = (w.div_ceil(cw) as usize).max(1);
        let bands = (h.div_ceil(ch) as usize).max(1);
        let picture = std::sync::Arc::new(crate::cell::Picture {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            width: w,
            height: h,
            rgba: rgba.into(),
        });
        let col = self.cur.col;
        let bg = self.cur.pen.bg;
        for band in 0..bands {
            let row = self.cur.row;
            if let Some(line) = self.grid.get_mut(row) {
                let to = (col + cols_needed).min(line.cells.len());
                for c in line.cells.iter_mut().take(to).skip(col) {
                    *c = Cell::blank(bg);
                }
                line.clear_marks(col, to);
                line.fix_wide();
                line.pictures.retain(|p| p.col < col || p.col >= to);
                line.pictures.push(crate::cell::PicturePiece {
                    col,
                    band,
                    bands,
                    cell_px: (cw, ch),
                    picture: picture.clone(),
                });
            }
            self.mark(row);
            self.linefeed();
        }
        self.cur.col = col.min(self.cols - 1);
    }

    /// Answers DECRQSS: the current value of the setting `name` names.
    fn answer_setting(&mut self, name: &[u8]) {
        let value = match name {
            b"m" => Some(format!("{}m", self.sgr_string())),
            b"r" => Some(format!("{};{}r", self.top + 1, self.bottom)),
            b"s" => Some(format!("{};{}s", self.left + 1, self.right)),
            b" q" => {
                let n = match (self.shape, self.modes.cursor_blink) {
                    (CursorShape::Block, true) => 1,
                    (CursorShape::Block, false) => 2,
                    (CursorShape::Underline, true) => 3,
                    (CursorShape::Underline, false) => 4,
                    (CursorShape::Bar, true) => 5,
                    (CursorShape::Bar, false) => 6,
                };
                Some(format!("{n} q"))
            }
            b"\"p" => Some("62;1\"p".to_owned()),
            b"\"q" => Some("0\"q".to_owned()),
            _ => None,
        };
        let s = match value {
            Some(v) => format!("\x1bP1$r{v}\x1b\\"),
            None => "\x1bP0$r\x1b\\".to_owned(),
        };
        self.reply(s.as_bytes());
    }

    /// The pen as SGR parameters (for DECRQSS).
    fn sgr_string(&self) -> String {
        let pen = &self.cur.pen;
        let mut parts = vec!["0".to_owned()];
        let a = pen.attrs;
        for (flag, code) in [
            (Attrs::BOLD, "1"),
            (Attrs::DIM, "2"),
            (Attrs::ITALIC, "3"),
            (Attrs::BLINK, "5"),
            (Attrs::INVERSE, "7"),
            (Attrs::HIDDEN, "8"),
            (Attrs::STRIKE, "9"),
            (Attrs::OVERLINE, "53"),
        ] {
            if a.contains(flag) {
                parts.push(code.to_owned());
            }
        }
        match a.underline() {
            Underline::None => {}
            Underline::Single => parts.push("4".into()),
            Underline::Double => parts.push("21".into()),
            Underline::Curly => parts.push("4:3".into()),
            Underline::Dotted => parts.push("4:4".into()),
            Underline::Dashed => parts.push("4:5".into()),
        }
        let color = |c: TermColor, base: u8, bright: u8, ext: u8| -> Option<String> {
            match c {
                TermColor::Default => None,
                TermColor::Indexed(i) if i < 8 => Some(format!("{}", base + i)),
                TermColor::Indexed(i) if i < 16 => Some(format!("{}", bright + i - 8)),
                TermColor::Indexed(i) => Some(format!("{ext};5;{i}")),
                TermColor::Rgb(r, g, b) => Some(format!("{ext};2;{r};{g};{b}")),
            }
        };
        parts.extend(color(pen.fg, 30, 90, 38));
        parts.extend(color(pen.bg, 40, 100, 48));
        if let Some(u) = match pen.ul {
            TermColor::Default => None,
            TermColor::Indexed(i) => Some(format!("58;5;{i}")),
            TermColor::Rgb(r, g, b) => Some(format!("58;2;{r};{g};{b}")),
        } {
            parts.push(u);
        }
        parts.join(";")
    }

    /// DECCOLM: 80 or 132 columns. The screen clears, the margins reset and the cursor goes
    /// home; the view is asked to show that many columns.
    fn switch_columns(&mut self, cols: usize) {
        let rows = self.rows;
        self.resize(cols, rows);
        self.forced_cols = Some(cols);
        self.out.columns = Some(cols);
        for r in 0..self.rows {
            self.erase_cells(r, 0, self.cols);
        }
        self.top = 0;
        self.bottom = self.rows;
        self.left = 0;
        self.right = self.cols;
        self.goto(0, 0);
    }

    /// The kitty keyboard flag stack of the screen showing.
    fn kitty_stack(&mut self) -> &mut Vec<u8> {
        let i = usize::from(self.modes.alt_screen);
        let [a, b] = &mut self.kitty;
        if i == 0 { a } else { b }
    }

    fn push_kitty(&mut self, flags: u8) {
        let limit = self.string_limit;
        let stack = self.kitty_stack();
        stack.push(flags & 31);
        // The protocol bounds the stack, dropping the oldest entries; ours holds what fits
        // in the string limit.
        if stack.len() > limit {
            stack.remove(0);
        }
        self.modes.kitty_flags = flags & 31;
    }

    fn pop_kitty(&mut self, n: usize) {
        let stack = self.kitty_stack();
        let keep = stack.len().saturating_sub(n);
        stack.truncate(keep);
        self.modes.kitty_flags = self.kitty_stack().last().copied().unwrap_or(0);
    }

    /// Replaces the top of the stack (`None`: empties it).
    fn set_kitty_top(&mut self, flags: Option<u8>) {
        let stack = self.kitty_stack();
        match flags {
            Some(f) => match stack.last_mut() {
                Some(top) => *top = f,
                None => stack.push(f),
            },
            None => stack.clear(),
        }
        self.modes.kitty_flags = self.kitty_stack().last().copied().unwrap_or(0);
    }

    /// The machine's memory changed (or became known): the automatic scrollback budget
    /// follows it.
    pub(crate) fn follow_machine_memory(&mut self) {
        let now = machine_memory();
        if now != self.budget_memory {
            self.budget_memory = now;
            self.budget = self.scrollback.budget();
            self.enforce_budget();
            self.mark_all();
        }
    }

    /// Lets go of scrollback when the system is short of memory: the older half when
    /// memory is short, all of it when it is critical.
    pub(crate) fn relieve(&mut self, level: remedy_ui_view::device::MemoryPressure) {
        use remedy_ui_view::device::MemoryPressure;
        let drop = match level {
            MemoryPressure::Normal => 0,
            MemoryPressure::Moderate => self.history.len() / 2,
            MemoryPressure::Critical => self.history.len(),
        };
        if drop == 0 {
            return;
        }
        for _ in 0..drop {
            if let Some(l) = self.history.pop_front() {
                self.history_bytes = self.history_bytes.saturating_sub(l.cost());
                self.dropped += 1;
            }
        }
        self.spare = Vec::new();
        self.history.shrink_to_fit();
        self.mark_all();
    }

    /// The cursor's row and column.
    pub(crate) fn cursor_pos(&self) -> (usize, usize) {
        (self.cur.row, self.cur.col)
    }

    fn set_title(&mut self, s: String) {
        self.title = s;
        self.out.title = true;
    }
}

/// How many cells `c` takes: 0 (combining), 1 or 2; 0 for control characters.
pub(crate) fn char_width(c: char) -> usize {
    c.width().unwrap_or(0).min(2)
}

/// `%XX` escapes decoded (a file URI's path); invalid escapes stay as they are.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |o: usize| b.get(i + o).and_then(|h| char::from(*h).to_digit(16));
        match (c, hex(1), hex(2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).chars().filter(|c| !c.is_control()).collect()
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c % 8 == 0 && c != 0).collect()
}

/// A color in SGR's colon form after its 38/48/58: `5:n`, `2:r:g:b` or `2:cs:r:g:b`.
fn extended_color(p: &[u16]) -> Option<TermColor> {
    match p {
        [5, n, ..] => u8::try_from(*n).ok().map(TermColor::Indexed),
        [2, r, g, b] | [2, _, r, g, b, ..] => {
            let (r, g, b) = (u8::try_from(*r).ok()?, u8::try_from(*g).ok()?, u8::try_from(*b).ok()?);
            Some(TermColor::Rgb(r, g, b))
        }
        _ => None,
    }
}

/// Text a program sent, cleaned for showing: control characters (which could fake a
/// prompt in a title bar or hide part of a link) are dropped.
fn clean(s: &[u8]) -> String {
    String::from_utf8_lossy(s).chars().filter(|c| !c.is_control()).collect()
}

/// A color spec from OSC 4/10/11/12: `rgb:r/g/b` (1–4 hex digits each) or `#rgb` forms.
fn parse_color(s: &[u8]) -> Option<Rgb> {
    let s = std::str::from_utf8(s).ok()?;
    let scale = |h: &str| -> Option<u8> {
        if h.is_empty() || h.len() > 4 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        let max = (1u32 << (4 * h.len())) - 1;
        u8::try_from(v * 255 / max).ok()
    };
    if let Some(rest) = s.strip_prefix("rgb:") {
        let mut parts = rest.split('/');
        let r = scale(parts.next()?)?;
        let g = scale(parts.next()?)?;
        let b = scale(parts.next()?)?;
        return parts.next().is_none().then_some((r, g, b));
    }
    let hex = s.strip_prefix('#')?;
    if hex.is_empty() || hex.len() % 3 != 0 || hex.len() > 12 {
        return None;
    }
    let n = hex.len() / 3;
    let r = scale(hex.get(0..n)?)?;
    let g = scale(hex.get(n..2 * n)?)?;
    let b = scale(hex.get(2 * n..3 * n)?)?;
    Some((r, g, b))
}

/// Reflows the primary screen and its scrollback to `cols` columns and `rows` rows: lines
/// that wrapped join and wrap again at the new width. Returns the screen, the new
/// scrollback, and where the cursor lands.
fn reflow(
    history: VecDeque<Line>,
    screen: Vec<Line>,
    cursor: (usize, usize),
    cols: usize,
    rows: usize,
) -> (Vec<Line>, Vec<Line>, (usize, usize)) {
    let cursor_line = history.len() + cursor.0;
    let mut lines: Vec<Line> = history.into_iter().chain(screen).collect();
    // Blank lines below the cursor carry nothing: they are not pushed into the scrollback.
    while lines.len() > cursor_line + 1 && lines.last().is_some_and(|l| l.used() == 0 && !l.wrapped) {
        lines.pop();
    }
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    let mut new_cursor = (0usize, 0usize);
    let mut iter = lines.into_iter().enumerate().peekable();
    while iter.peek().is_some() {
        // One logical line: physical lines joined while they wrap.
        let mut cells: Vec<Cell> = Vec::new();
        let mut marks: Vec<(usize, Box<str>)> = Vec::new();
        let mut cursor_at: Option<usize> = None;
        // The shell's marks and the pictures stay with the logical line's first line.
        let mut prompt = 0u8;
        let mut exit = None;
        let mut pictures = Vec::new();
        for (i, line) in iter.by_ref() {
            prompt |= line.prompt;
            exit = exit.or(line.exit);
            pictures.extend(line.pictures.iter().filter(|p| p.col < cols).cloned());
            let keep = if line.wrapped { line.cells.len() } else { line.used() };
            let keep = if i == cursor_line {
                keep.max(cursor.1 + 1).min(line.cells.len().max(cursor.1 + 1))
            } else {
                keep
            };
            let mut map = vec![0usize; line.cells.len() + 1];
            for (c, cell) in line.cells.iter().enumerate().take(keep) {
                if let Some(m) = map.get_mut(c) {
                    *m = cells.len();
                }
                if cell.attrs.contains(Attrs::WIDE_FILLER) {
                    continue;
                }
                cells.push(*cell);
            }
            if i == cursor_line {
                let base = map.get(cursor.1).copied().unwrap_or(cells.len());
                cursor_at = Some(if cursor.1 >= line.cells.len() { cells.len() } else { base });
                // A cursor past the text sits on blanks that must stay.
                while cells.len() <= cursor_at.unwrap_or(0) {
                    cells.push(Cell::BLANK);
                }
            }
            for (c, m) in line.marks {
                if let Some(at) = map.get(c as usize) {
                    marks.push((*at, m));
                }
            }
            if !line.wrapped {
                break;
            }
        }
        // Wrap it again at the new width.
        let start = out.len();
        let carried = (prompt, exit, pictures);
        let mut line = Line::new(0, Cell::BLANK);
        let mut placed: Vec<(usize, usize)> = Vec::with_capacity(cells.len());
        for cell in cells {
            if cell.attrs.contains(Attrs::WIDE_SPACER) {
                if let Some(&(l, c)) = placed.last() {
                    placed.push((l, c + 1));
                }
                if line.cells.len() < cols {
                    line.cells.push(cell);
                }
                continue;
            }
            let w = if cell.attrs.contains(Attrs::WIDE) { 2 } else { 1 };
            if line.cells.len() + w > cols {
                if w == 2 && line.cells.len() < cols {
                    line.cells.push(Cell { attrs: Attrs::WIDE_FILLER, ..Cell::BLANK });
                }
                line.wrapped = true;
                out.push(std::mem::replace(&mut line, Line::new(0, Cell::BLANK)));
            }
            placed.push((out.len(), line.cells.len()));
            line.cells.push(cell);
        }
        out.push(line);
        for (at, m) in marks {
            if let Some(&(l, c)) = placed.get(at)
                && let Some(line) = out.get_mut(l)
            {
                line.marks.push((u32::try_from(c).unwrap_or(u32::MAX), m));
            }
        }
        if let Some(at) = cursor_at {
            new_cursor = placed.get(at).copied().unwrap_or((out.len() - 1, 0));
        }
        if out.len() == start {
            out.push(Line::new(0, Cell::BLANK));
        }
        if let Some(first) = out.get_mut(start) {
            (first.prompt, first.exit, first.pictures) = carried;
        }
    }
    if out.is_empty() {
        out.push(Line::new(0, Cell::BLANK));
    }
    let mut screen_start = out.len().saturating_sub(rows);
    if new_cursor.0 < screen_start {
        screen_start = new_cursor.0;
        out.truncate(screen_start + rows);
    }
    let mut screen: Vec<Line> = out.split_off(screen_start);
    let history = out;
    for l in &mut screen {
        l.set_len(cols);
    }
    while screen.len() < rows {
        screen.push(Line::new(cols, Cell::BLANK));
    }
    let crow = new_cursor.0.saturating_sub(screen_start).min(rows - 1);
    let ccol = new_cursor.1.min(cols - 1);
    (screen, history, (crow, ccol))
}

impl vte::Perform for State {
    fn print(&mut self, c: char) {
        self.acted = true;
        // The common case, kept short: printable ASCII in the ASCII set, overwriting a
        // plain cell that is not the last on its line.
        if (' '..='~').contains(&c)
            && self.single_shift.is_none()
            && !self.cur.pending
            && !self.modes.insert
            && self.cur.col + 1 < self.right
            && self.charsets.get(self.gl) == Some(&Charset::Ascii)
        {
            let (row, col, pen) = (self.cur.row, self.cur.col, self.cur.pen);
            if let Some(line) = self.grid.get_mut(row)
                && line.marks.is_empty()
                && let Some(cell) = line.cells.get_mut(col)
                && !cell.attrs.contains(Attrs::WIDE)
                && !cell.attrs.contains(Attrs::WIDE_SPACER)
            {
                *cell = pen.cell(c);
                self.cur.col = col + 1;
                self.last_char = Some(c);
                if let Some(d) = self.damage.rows.get_mut(row) {
                    *d = true;
                }
                return;
            }
        }
        self.print_char(c);
    }

    fn execute(&mut self, byte: u8) {
        self.acted = true;
        if self.aborting {
            return;
        }
        match byte {
            0x07 => self.out.bells = self.out.bells.saturating_add(1),
            0x08 => self.backward(1),
            0x09 => self.tab(1),
            0x0A..=0x0C => {
                if self.modes.newline {
                    self.carriage_return();
                }
                self.linefeed();
            }
            0x0D => self.carriage_return(),
            0x0E => self.gl = 1,
            0x0F => self.gl = 0,
            _ => {}
        }
    }

    fn hook(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        self.acted = true;
        self.dcs = Some(match (intermediates, action) {
            _ if ignore || self.aborting => Dcs::Ignore,
            ([b'$'], 'q') => Dcs::Request(Vec::new()),
            ([], 'q') => {
                // P2 says what unset pixels are: 1 transparent, else the background.
                let p2 = params.iter().nth(1).and_then(|p| p.first().copied()).unwrap_or(0);
                // The picture may be as large as the screen in pixels, never larger.
                let (cw, ch) = self.cell_px_or_nominal();
                let max_w = self.cols.saturating_mul(cw as usize);
                let max_h = self.rows.saturating_mul(ch as usize);
                Dcs::Sixel(Box::new(crate::sixel::Decoder::new(max_w, max_h, p2)))
            }
            _ => Dcs::Ignore,
        });
    }

    fn put(&mut self, byte: u8) {
        self.acted = true;
        match &mut self.dcs {
            Some(Dcs::Request(buf)) => {
                // A setting's name is a few bytes; a longer one names no setting (and is
                // answered as such), and its bytes are not kept.
                if buf.len() < 8 {
                    buf.push(byte);
                }
            }
            Some(Dcs::Sixel(d)) => d.put(byte),
            _ => {}
        }
    }

    fn unhook(&mut self) {
        self.acted = true;
        match self.dcs.take() {
            Some(Dcs::Request(name)) if !self.aborting => self.answer_setting(&name),
            Some(Dcs::Sixel(d)) if !self.aborting => {
                if let Some((w, h, rgba)) = d.finish() {
                    self.place_picture(w, h, rgba);
                }
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        self.acted = true;
        if self.aborting {
            return;
        }
        let total: usize = params.iter().map(|p| p.len() + 1).sum();
        if total > self.string_limit {
            return;
        }
        let Some(kind) = params.first().and_then(|k| std::str::from_utf8(k).ok()) else { return };
        let rest = |from: usize| params.get(from..).unwrap_or_default().join(&b';');
        match kind {
            "0" | "2" => self.set_title(clean(&rest(1))),
            "8" => {
                let uri = clean(&rest(2));
                if uri.is_empty() {
                    self.cur.pen.link = 0;
                    return;
                }
                let id = params.get(1).and_then(|p| {
                    std::str::from_utf8(p)
                        .ok()?
                        .split(':')
                        .find_map(|kv| kv.strip_prefix("id=").map(str::to_owned))
                });
                self.cur.pen.link = self.link_id(id, uri);
            }
            "4" => {
                let mut pairs = params.get(1..).unwrap_or_default().chunks(2);
                for pair in pairs.by_ref() {
                    let [index, spec] = pair else { break };
                    let Some(i) = std::str::from_utf8(index).ok().and_then(|s| s.parse::<u8>().ok()) else {
                        continue;
                    };
                    if *spec == b"?" {
                        self.color_reply(&format!("4;{i}"), u16::from(i), bell_terminated);
                    } else if let Some(c) = parse_color(spec) {
                        self.overrides.insert(u16::from(i), c);
                        self.out.colors = true;
                        self.mark_all();
                    }
                }
            }
            "10" | "11" | "12" => {
                let slot = match kind {
                    "10" => 256,
                    "11" => 257,
                    _ => 258,
                };
                // Each following parameter is the next of the three colors.
                for (k, spec) in params.get(1..).unwrap_or_default().iter().enumerate() {
                    let Some(s) = u16::try_from(k).ok().map(|k| slot + k).filter(|s| *s <= 258) else {
                        break;
                    };
                    if *spec == b"?" {
                        let name = (s - 256 + 10).to_string();
                        self.color_reply(&name, s, bell_terminated);
                    } else if let Some(c) = parse_color(spec) {
                        self.overrides.insert(s, c);
                        self.out.colors = true;
                        self.mark_all();
                    }
                }
            }
            "104" => {
                if params.len() <= 1 || params.get(1).is_some_and(|p| p.is_empty()) {
                    self.overrides.retain(|k, _| *k >= 256);
                } else {
                    for p in params.get(1..).unwrap_or_default() {
                        if let Some(i) = std::str::from_utf8(p).ok().and_then(|s| s.parse::<u16>().ok()) {
                            self.overrides.remove(&i);
                        }
                    }
                }
                self.out.colors = true;
                self.mark_all();
            }
            "110" | "111" | "112" => {
                let slot = match kind {
                    "110" => 256,
                    "111" => 257,
                    _ => 258,
                };
                self.overrides.remove(&slot);
                self.out.colors = true;
                self.mark_all();
            }
            "7" => {
                // file://host/path, with the path percent-encoded.
                let uri = clean(&rest(1));
                let path =
                    uri.strip_prefix("file://").map(|r| r.find('/').map_or("", |i| r.get(i..).unwrap_or("")));
                let cwd = path.map(percent_decode).filter(|p| !p.is_empty());
                if cwd != self.cwd {
                    self.cwd = cwd;
                    self.out.cwd = true;
                }
            }
            "133" => {
                let kind = params.get(1).and_then(|p| p.first().copied());
                let row = self.cur.row;
                let exit = params
                    .get(2)
                    .and_then(|p| std::str::from_utf8(p).ok())
                    .and_then(|s| s.parse::<i32>().ok());
                if let Some(line) = self.grid.get_mut(row) {
                    match kind {
                        Some(b'A') => line.prompt |= crate::cell::MARK_PROMPT,
                        Some(b'B') => line.prompt |= crate::cell::MARK_COMMAND,
                        Some(b'C') => line.prompt |= crate::cell::MARK_OUTPUT,
                        Some(b'D') => {
                            line.prompt |= crate::cell::MARK_DONE;
                            line.exit = exit;
                        }
                        _ => {}
                    }
                }
                self.mark(row);
            }
            "52" => {
                let data = rest(2);
                if data == b"?" {
                    // Programs never read the clipboard.
                    return;
                }
                use base64::Engine as _;
                let cleaned: Vec<u8> = data.into_iter().filter(|b| !b.is_ascii_whitespace()).collect();
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&cleaned) {
                    self.out.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        self.acted = true;
        if ignore || self.aborting {
            return;
        }
        let mut a = [0u16; 32];
        let mut n = 0;
        for p in params.iter() {
            if let Some(slot) = a.get_mut(n) {
                *slot = p.first().copied().unwrap_or(0);
            }
            n += 1;
        }
        // A parameter, with 0 or missing meaning `default`.
        let arg = |i: usize, default: usize| -> usize {
            match a.get(i).copied().unwrap_or(0) {
                0 => default,
                v => usize::from(v),
            }
        };
        let raw = |i: usize| -> u16 { a.get(i).copied().unwrap_or(0) };
        match (intermediates, action) {
            ([], '@') => self.insert_chars(arg(0, 1)),
            ([], 'A') => self.up(arg(0, 1)),
            ([], 'B' | 'e') => self.down(arg(0, 1)),
            ([], 'C' | 'a') => self.forward(arg(0, 1)),
            ([], 'D') => self.backward(arg(0, 1)),
            ([], 'E') => {
                self.down(arg(0, 1));
                self.carriage_return();
            }
            ([], 'F') => {
                self.up(arg(0, 1));
                self.carriage_return();
            }
            ([], 'G' | '`') => self.goto_col(arg(0, 1) - 1),
            ([], 'H' | 'f') => self.goto(arg(0, 1) - 1, arg(1, 1) - 1),
            ([], 'I') => self.tab(arg(0, 1)),
            ([] | [b'?'], 'J') => self.erase_display(usize::from(raw(0))),
            ([] | [b'?'], 'K') => self.erase_line(usize::from(raw(0))),
            ([], 'L') => self.insert_lines(arg(0, 1)),
            ([], 'M') => self.delete_lines(arg(0, 1)),
            ([], 'P') => self.delete_chars(arg(0, 1)),
            ([], 'S') => self.scroll_up_in(self.top, self.bottom, arg(0, 1), true),
            ([], 'T') if n <= 1 => self.scroll_down_in(self.top, self.bottom, arg(0, 1)),
            ([], 'X') => {
                let (row, col) = (self.cur.row, self.cur.col);
                let to = col.saturating_add(arg(0, 1)).min(self.cols);
                // Erasing characters does not end the line's wrap.
                let wrapped = self.grid.get(row).is_some_and(|l| l.wrapped);
                self.erase_cells(row, col, to);
                if let Some(l) = self.grid.get_mut(row) {
                    l.wrapped = wrapped;
                }
            }
            ([], 'Z') => self.back_tab(arg(0, 1)),
            ([], 'b') => self.repeat(arg(0, 1)),
            ([], 'c') if raw(0) == 0 => self.reply(b"\x1b[?62;22c"),
            ([b'>'], 'c') if raw(0) == 0 => self.reply(b"\x1b[>1;10;0c"),
            ([], 'd') => self.goto_row(arg(0, 1) - 1),
            ([], 'g') => match raw(0) {
                0 => {
                    if let Some(t) = self.tabs.get_mut(self.cur.col) {
                        *t = false;
                    }
                }
                3 => self.tabs.iter_mut().for_each(|t| *t = false),
                _ => {}
            },
            ([], 'h' | 'l') => {
                let on = action == 'h';
                for &m in a.iter().take(n) {
                    match m {
                        4 => self.modes.insert = on,
                        20 => self.modes.newline = on,
                        _ => {}
                    }
                }
            }
            ([b'?'], 'h' | 'l') => {
                let on = action == 'h';
                for &m in a.iter().take(n) {
                    self.set_private_mode(m, on);
                }
            }
            ([], 'm') => self.sgr(params),
            ([], 'n') => match raw(0) {
                5 => self.reply(b"\x1b[0n"),
                6 => {
                    let row = if self.modes.origin { self.cur.row - self.top } else { self.cur.row };
                    let s = format!("\x1b[{};{}R", row + 1, self.cur.col + 1);
                    self.reply(s.as_bytes());
                }
                _ => {}
            },
            ([b'?'], 'n') if raw(0) == 6 => {
                let row = if self.modes.origin { self.cur.row - self.top } else { self.cur.row };
                let s = format!("\x1b[?{};{}R", row + 1, self.cur.col + 1);
                self.reply(s.as_bytes());
            }
            ([], 'r') => {
                let top = arg(0, 1) - 1;
                let bottom = arg(1, self.rows).min(self.rows);
                if top + 1 < bottom {
                    self.top = top;
                    self.bottom = bottom;
                    self.goto(0, 0);
                }
            }
            // With left and right margins on, CSI s sets them (DECSLRM); otherwise it saves
            // the cursor (SCOSC).
            ([], 's') if self.modes.lr_margins => {
                let left = arg(0, 1) - 1;
                let right = arg(1, self.cols).min(self.cols);
                if left + 1 < right {
                    self.left = left;
                    self.right = right;
                    self.goto(0, 0);
                }
            }
            ([], 's') => self.save_cursor(),
            ([], 'u') => self.restore_cursor(),
            // The kitty keyboard protocol: push, pop, set and query the enhancement flags.
            ([b'>'], 'u') => self.push_kitty(u8::try_from(raw(0)).unwrap_or(0)),
            ([b'<'], 'u') => self.pop_kitty(arg(0, 1)),
            ([b'='], 'u') => {
                let flags = u8::try_from(raw(0)).unwrap_or(0) & 31;
                let now = self.modes.kitty_flags;
                let next = match arg(1, 1) {
                    2 => now | flags,
                    3 => now & !flags,
                    _ => flags,
                };
                self.set_kitty_top(Some(next));
            }
            ([b'?'], 'u') => {
                let s = format!("\x1b[?{}u", self.modes.kitty_flags);
                self.reply(s.as_bytes());
            }
            ([], 't') => match raw(0) {
                14 => {
                    let (w, h) = self.cell_px;
                    let s = format!("\x1b[4;{};{}t", h as usize * self.rows, w as usize * self.cols);
                    self.reply(s.as_bytes());
                }
                16 => {
                    let (w, h) = self.cell_px;
                    let s = format!("\x1b[6;{h};{w}t");
                    self.reply(s.as_bytes());
                }
                18 => {
                    let s = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.reply(s.as_bytes());
                }
                22 => {
                    let t = self.title.clone();
                    let total: usize = self.titles.iter().map(String::len).sum::<usize>() + t.len();
                    self.titles.push_back(t);
                    // The stack keeps what fits in the string limit, newest first.
                    let mut total = total;
                    while total > self.string_limit {
                        match self.titles.pop_front() {
                            Some(old) => total -= old.len(),
                            None => break,
                        }
                    }
                }
                23 => {
                    if let Some(t) = self.titles.pop_back() {
                        self.set_title(t);
                    }
                }
                _ => {}
            },
            ([b' '], 'q') => {
                let (shape, blink) = match raw(0) {
                    0 | 1 => (CursorShape::Block, true),
                    2 => (CursorShape::Block, false),
                    3 => (CursorShape::Underline, true),
                    4 => (CursorShape::Underline, false),
                    5 => (CursorShape::Bar, true),
                    6 => (CursorShape::Bar, false),
                    _ => return,
                };
                self.shape = shape;
                self.modes.cursor_blink = blink;
                self.mark(self.cur.row);
            }
            ([b'!'], 'p') => self.soft_reset(),
            ([b'?', b'$'], 'p') => {
                let m = raw(0);
                let s = format!("\x1b[?{m};{}$y", self.private_mode_state(m));
                self.reply(s.as_bytes());
            }
            ([b'$'], 'p') => {
                let m = raw(0);
                let v = match m {
                    4 => {
                        if self.modes.insert {
                            1
                        } else {
                            2
                        }
                    }
                    20 => {
                        if self.modes.newline {
                            1
                        } else {
                            2
                        }
                    }
                    _ => 0,
                };
                let s = format!("\x1b[{m};{v}$y");
                self.reply(s.as_bytes());
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.acted = true;
        if ignore || self.aborting {
            return;
        }
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([b'#'], b'8') => self.screen_alignment(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.carriage_return();
                self.linefeed();
            }
            ([], b'H') => {
                if let Some(t) = self.tabs.get_mut(self.cur.col) {
                    *t = true;
                }
            }
            ([], b'M') => self.reverse_index(),
            ([], b'N') => self.single_shift = Some(2),
            ([], b'O') => self.single_shift = Some(3),
            ([], b'Z') => self.reply(b"\x1b[?62;22c"),
            ([], b'c') => self.reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([slot @ (b'(' | b')' | b'*' | b'+')], set) => {
                let i = usize::from(*slot - b'(');
                let cs = match set {
                    b'0' => Charset::DecGraphics,
                    b'A' => Charset::Uk,
                    _ => Charset::Ascii,
                };
                if let Some(c) = self.charsets.get_mut(i) {
                    *c = cs;
                }
            }
            _ => {}
        }
    }
}

/// A terminal emulator with no UI: feed it a program's output and read the screen.
/// [`terminal`](crate::terminal) puts one on screen; this type is also useful alone (tests,
/// recordings, agents reading a session).
pub struct Emulator {
    parser: vte::Parser,
    pub(crate) state: State,
    /// Bytes of input in a row with no action: vte is inside a string it buffers (OSC).
    silent: usize,
    /// Dropping the rest of a string that grew past the limit, up to its terminator.
    discarding: bool,
}

impl std::fmt::Debug for Emulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emulator").field("cols", &self.state.cols).field("rows", &self.state.rows).finish()
    }
}

/// How much input is parsed between checks for an over-long string: a tuning constant (a
/// string is cut within this many bytes of its limit).
const GUARD_STEP: usize = 4096;

/// How long a synchronized update may hold the screen before it is shown anyway: a
/// program that never ends one must not freeze the terminal (xterm and kitty use similar).
pub(crate) const SYNC_TIMEOUT: Duration = Duration::from_millis(150);

impl Emulator {
    /// An emulator of `cols` × `rows` cells (each at least 1).
    pub fn new(cols: usize, rows: usize) -> Emulator {
        Emulator { parser: vte::Parser::new(), state: State::new(cols, rows), silent: 0, discarding: false }
    }

    /// Parses `bytes` from the program and updates the screen.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.state.follow_machine_memory();
        let mut rest = bytes;
        while !rest.is_empty() {
            if self.discarding {
                // Skip to the end of the string that was too long: BEL, or ESC (ESC \ is ST).
                match rest.iter().position(|b| *b == 0x07 || *b == 0x1b || *b == 0x18 || *b == 0x1a) {
                    Some(at) => {
                        self.discarding = false;
                        let b = rest.get(at).copied().unwrap_or(0);
                        rest = rest.get(at + 1..).unwrap_or_default();
                        if b == 0x1b && rest.first() == Some(&b'\\') {
                            rest = rest.get(1..).unwrap_or_default();
                        }
                    }
                    None => return,
                }
                continue;
            }
            let take = rest.len().min(GUARD_STEP);
            let (piece, tail) = rest.split_at(take);
            rest = tail;
            self.state.acted = false;
            self.parser.advance(&mut self.state, piece);
            if self.state.acted {
                self.silent = 0;
            } else {
                self.silent += piece.len();
            }
            if self.silent > self.state.string_limit {
                // Cut the string: CAN returns the parser to ground; what it dispatches on the
                // way out is ignored, and the rest of the string is skipped.
                self.state.aborting = true;
                self.parser.advance(&mut self.state, &[0x18]);
                self.state.aborting = false;
                self.silent = 0;
                self.discarding = true;
                // The parser's string buffer is let go too.
                self.parser = vte::Parser::new();
            }
        }
    }

    /// Changes the size, reflowing the primary screen and the scrollback.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.state.resize(cols, rows);
    }

    /// Columns and rows.
    pub fn size(&self) -> (usize, usize) {
        (self.state.cols, self.state.rows)
    }

    /// Screen line `row`.
    pub fn line(&self, row: usize) -> Option<&Line> {
        self.state.grid.get(row)
    }

    /// Cell (`row`, `col`) of the screen.
    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.state.grid.get(row)?.cells.get(col)
    }

    /// The screen's text, one line per row, trailing blanks left out.
    pub fn screen_text(&self) -> String {
        self.state.grid.iter().map(Line::text).collect::<Vec<_>>().join("\n")
    }

    /// Lines in the scrollback.
    pub fn history_len(&self) -> usize {
        self.state.history.len()
    }

    /// Scrollback line `i` (0 is the oldest kept).
    pub fn history_line(&self, i: usize) -> Option<&Line> {
        self.state.history.get(i)
    }

    /// Bytes the scrollback holds.
    pub fn history_bytes(&self) -> usize {
        self.state.history_bytes
    }

    /// The cursor.
    pub fn cursor(&self) -> CursorState {
        let s = &self.state;
        CursorState {
            row: s.cur.row,
            col: s.cur.col,
            visible: s.modes.cursor_visible,
            shape: s.shape,
            blink: s.modes.cursor_blink,
        }
    }

    /// The modes the program set.
    pub fn modes(&self) -> Modes {
        self.state.modes
    }

    /// The window title the program set (OSC 0 or 2).
    pub fn title(&self) -> &str {
        &self.state.title
    }

    /// The URI of hyperlink `id` (a cell's [`Cell::link`]).
    pub fn link(&self, id: u32) -> Option<&str> {
        self.state.link_uri(id)
    }

    /// Takes the bytes the program is owed: answers to its queries (device attributes,
    /// cursor position, colors, modes). The app writes them to the program.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.state.out.replies)
    }

    /// Takes what the program asked to put on the clipboard (OSC 52), if it did.
    pub fn take_clipboard_request(&mut self) -> Option<String> {
        self.state.out.clipboard.take()
    }

    /// Sets how much scrollback to keep (dropping the oldest lines past it).
    pub fn set_scrollback(&mut self, s: Scrollback) {
        self.state.set_scrollback(s);
    }

    /// Sets the longest string a program may send in one sequence.
    pub fn set_string_limit(&mut self, bytes: usize) {
        self.state.string_limit = bytes.max(1);
    }

    /// The working directory the shell reported (OSC 7), as a path.
    pub fn cwd(&self) -> Option<&str> {
        self.state.cwd.as_deref()
    }

    /// The column count the program switched to (DECCOLM, when DECSET 40 allows it), which
    /// a view should show instead of what fits.
    pub fn forced_columns(&self) -> Option<usize> {
        self.state.forced_cols
    }

    /// Gives scrollback back under memory pressure: the older half when memory is short,
    /// all of it when critical.
    pub fn relieve(&mut self, level: remedy_ui_view::device::MemoryPressure) {
        self.state.relieve(level);
    }

    /// The screen line keys of the shell's marks (OSC 133): (line in the whole text, counting
    /// dropped lines, and its marks), oldest first.
    pub fn prompt_lines(&self) -> Vec<(u64, Vec<crate::cell::PromptMark>)> {
        let s = &self.state;
        let base = s.dropped;
        let history = if s.modes.alt_screen { None } else { Some(&s.history) };
        let mut out = Vec::new();
        for (i, line) in history.into_iter().flatten().enumerate() {
            if line.prompt != 0 {
                out.push((base + i as u64, line.prompt_marks()));
            }
        }
        if !s.modes.alt_screen {
            let start = base + s.history.len() as u64;
            for (r, line) in s.grid.iter().enumerate() {
                if line.prompt != 0 {
                    out.push((start + r as u64, line.prompt_marks()));
                }
            }
        }
        out
    }

    /// Lines dropped from the top of the scrollback so far.
    pub fn dropped_lines(&self) -> u64 {
        self.state.dropped
    }
}
