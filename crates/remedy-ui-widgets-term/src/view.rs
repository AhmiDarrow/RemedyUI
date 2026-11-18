//! The terminal element: the emulator's screen and scrollback on a virtual list of rows,
//! drawn cell by cell from the theme's colors; keys, pastes, pointer reports and focus sent
//! to the program; selection and copying; links opened through the app; the input method's
//! composition at the cursor; and the screen and new output for screen readers.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use remedy_ui_a11y::{Live, Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Size, Vec2};
use remedy_ui_host::ImeEvent;
use remedy_ui_input::{Buttons, Key, KeyEvent, Modifiers, Platform, PointerEvent, PointerPhase, WheelEvent};
use remedy_ui_layout::Length;
use remedy_ui_paint::{Brush, DisplayList, GlyphRun, Item, Path, Stroke};
use remedy_ui_reactive::Signal;
use remedy_ui_style::Theme;
use remedy_ui_text::{Antialias, Hinting, ParagraphOptions, RichText, RunFont, TextStyle};
use remedy_ui_view::{El, PaintCx, TextInput, Ui};

use crate::boxdraw;
use crate::cell::{Attrs, Cell, Line, Underline};
use crate::keys::{self, MouseAction, MouseButton};
use crate::palette::{self, Palette};
use crate::shared::{Feeder, Shared};
use crate::term::{CursorShape, CursorState, Emulator, Modes, MouseMode, SYNC_TIMEOUT, Scrollback};

/// How often screen readers hear new output at most: a tuning constant (output faster than
/// this is gathered and read in one go, so a busy build does not flood speech).
const ANNOUNCE_EVERY: Duration = Duration::from_millis(500);

/// The cursor's blink half-period, xterm's default.
const BLINK: Duration = Duration::from_millis(530);

/// Rows built beyond each edge of the view, so a small scroll shows rows already painted.
const PREFETCH_ROWS: usize = 4;

/// Keys for the alternate screen's rows in the line cache (the primary's use line numbers).
const ALT: u64 = 1 << 63;

/// How often a drag past the view's edge scrolls it: a tuning constant (the speed grows
/// with how far past the edge the pointer is).
const AUTOSCROLL_EVERY: Duration = Duration::from_millis(50);

/// A line rewritten in place (a progress bar) is read again at most every this many
/// announcement rounds (four rounds of 500 ms: every two seconds).
const REANNOUNCE_ROUNDS: u32 = 4;

/// How a terminal is set up.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalOptions {
    /// What screen readers call it ("Terminal", "Build output").
    pub label: String,
    /// How much scrollback to keep.
    pub scrollback: Scrollback,
    /// The longest string a program may send in one sequence; `None` sizes it from the
    /// machine ([`default_string_limit`](crate::default_string_limit)).
    pub string_limit: Option<usize>,
    /// Font families in fallback order; `None` uses the system's monospace font
    /// (`ui-monospace`, then `monospace`) and then the theme's monospace families. A face that
    /// is not monospace still lines up: each character is centered in its cells, and one
    /// wider than its cells is drawn smaller.
    pub families: Option<Vec<String>>,
    /// Characters that count as part of a word when double-clicking, besides letters and
    /// digits (so a path or a URL selects whole).
    pub word_chars: String,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        TerminalOptions {
            label: "Terminal".into(),
            scrollback: Scrollback::Auto,
            string_limit: None,
            families: None,
            word_chars: "-_.~/:@?&=%+#".into(),
        }
    }
}

/// The terminal's size, as the app passes it to its PTY.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridSize {
    /// Columns.
    pub cols: usize,
    /// Rows.
    pub rows: usize,
    /// A cell's width in device pixels.
    pub cell_width: u32,
    /// A cell's height in device pixels.
    pub cell_height: u32,
}

/// The cursor as a row draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CursorDraw {
    col: usize,
    shape: CursorShape,
    /// The terminal has focus: a block cursor is filled, else outlined.
    focused: bool,
}

/// What one row shows.
#[derive(Clone, Debug, Default)]
struct RowView {
    line: Option<Rc<Line>>,
    cursor: Option<CursorDraw>,
    /// Selected columns, `start..end` (`end` past the line for the rest of it).
    sel: Option<(usize, usize)>,
    hover_link: u32,
    /// The input method's composition and the column it shows at.
    preedit: Option<(usize, String)>,
    /// The look's generation: a new font or palette repaints.
    generation: u64,
}

impl PartialEq for RowView {
    fn eq(&self, o: &Self) -> bool {
        let same_line = match (&self.line, &o.line) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_line
            && self.cursor == o.cursor
            && self.sel == o.sel
            && self.hover_link == o.hover_link
            && self.preedit == o.preedit
            && self.generation == o.generation
    }
}

/// Glyph ids with their offsets from the cell's origin on the baseline.
type Placed = Vec<(u32, f32, f32)>;

/// Program color changes: palette slot (256 fg, 257 bg, 258 cursor) and RGB.
type ColorChanges = Vec<(u16, (u8, u8, u8))>;

/// Glyphs for one cell's text, shaped once.
struct Shaped {
    runs: Vec<(RunFont, f32, Placed)>,
    width: f32,
}

/// Shaped cell texts, by text and bold/italic. Cleared when the font changes, and when it
/// holds several screenfuls of different texts (sized from the screen, not fixed). Also the
/// pictures programs drew, as images for the renderer, while lines still show them.
#[derive(Default)]
struct GlyphCache {
    map: HashMap<(Box<str>, u8), Rc<Shaped>>,
    limit: usize,
    pictures: HashMap<u64, remedy_ui_paint::Image>,
}

/// How the terminal is drawn: cell metrics and colors.
struct Look {
    cell_w: f32,
    line_h: f32,
    baseline: f32,
    size: f32,
    weight: f32,
    families: Vec<String>,
    scale: f32,
    measured: bool,
    /// The font gives every character the same advance: characters sit at their cell's
    /// start. Otherwise each is centered in its cells.
    monospace: bool,
    /// The theme colors the palette was made from, and the program's color changes.
    theme_colors: Option<remedy_ui_style::ColorRoles>,
    overrides: Vec<(u16, (u8, u8, u8))>,
    palette: Option<Rc<Palette>>,
    /// The palette changed and the emulator should answer color queries with it.
    palette_for_emulator: bool,
    reverse: bool,
    generation: u64,
}

impl Look {
    /// The palette under `theme` with the program's changes, made again when either changed.
    fn palette(&mut self, theme: &Theme) -> Rc<Palette> {
        let theme_changed = self.theme_colors.as_ref() != Some(&theme.colors);
        match &self.palette {
            Some(p) if !theme_changed => p.clone(),
            _ => {
                let fg = palette::foreground(theme);
                let p = Rc::new(Palette::new(theme, fg, palette::background(theme), &self.overrides));
                if theme_changed {
                    self.theme_colors = Some(theme.colors.clone());
                    self.palette_for_emulator = true;
                }
                self.palette = Some(p.clone());
                p
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelMode {
    Cell,
    Word,
    Line,
}

/// A selection: where it started and where it is now, as (line key, column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sel {
    anchor: (u64, usize),
    head: (u64, usize),
    mode: SelMode,
}

struct Row {
    el: El,
    view: Signal<RowView>,
}

type Output = Rc<dyn Fn(&[u8])>;
type LinkFn = dyn Fn(&str);
type ConsentFn = dyn Fn(&str) -> bool;

#[derive(Default)]
struct Callbacks {
    output: Option<Output>,
    resize: Option<Rc<dyn Fn(GridSize)>>,
    link: Option<Rc<LinkFn>>,
    clipboard: Option<Rc<ConsentFn>>,
    bell: Option<Rc<dyn Fn()>>,
}

/// The UI thread's view of the emulator.
struct St {
    cols: usize,
    rows: usize,
    alt: bool,
    history: usize,
    dropped: u64,
    cache: HashMap<u64, Rc<Line>>,
    shown: BTreeMap<usize, Row>,
    follow: bool,
    /// How far the view could scroll at the last layout.
    max_scroll: f32,
    view: Size,
    cursor: CursorState,
    modes: Modes,
    sel: Option<Sel>,
    selecting: bool,
    hover_link: u32,
    preedit: Option<String>,
    focused: bool,
    wheel: f32,
    mouse_down: Option<MouseButton>,
    last_report: Option<(usize, usize)>,
    /// Where the output read to screen readers ends: line key and column.
    spoken_to: Option<(u64, usize)>,
    a11y_pending: bool,
    /// The cursor's line as screen readers last heard it, and announcement rounds since a
    /// line was last read again.
    line_said: Option<(u64, String)>,
    rounds_since_reread: u32,
    blink_on: bool,
    blink_armed: bool,
    sync_waiting: bool,
    /// The columns the program switched to (DECCOLM), shown instead of what fits.
    forced_cols: Option<usize>,
    /// A drag past the view's edge: lines to scroll per tick (negative up), and the pointer.
    autoscroll: Option<(f32, Point)>,
    autoscroll_armed: bool,
    /// The last text selected here: what the middle button pastes where the platform has
    /// no primary selection.
    last_selection: Option<String>,
}

struct Inner {
    ui: Ui,
    shared: Arc<Shared>,
    outer: El,
    root: El,
    scroller: El,
    top: El,
    rows_box: El,
    bottom: El,
    options: TerminalOptions,
    st: RefCell<St>,
    cb: RefCell<Callbacks>,
    look: Rc<RefCell<Look>>,
    glyphs: Rc<RefCell<GlyphCache>>,
    title: Signal<String>,
    cwd: Signal<Option<String>>,
    screen_text: Signal<String>,
    announce: Signal<String>,
}

/// A terminal on screen. Cloning gives another handle to the same terminal.
///
/// The app owns the PTY: it hands the program's output to [`Terminal::feeder`] (from any
/// thread) and writes what [`Terminal::on_output`] gives it (keys, pastes, pointer reports,
/// answers to queries) to the program. When the terminal's size changes,
/// [`Terminal::on_resize`] tells it the new rows and columns for the PTY.
#[derive(Clone)]
pub struct Terminal(Rc<Inner>);

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.0.st.borrow();
        f.debug_struct("Terminal").field("cols", &st.cols).field("rows", &st.rows).finish_non_exhaustive()
    }
}

/// A terminal element. Put [`Terminal::element`] in the tree; feed it with
/// [`Terminal::feeder`].
#[track_caller]
pub fn terminal(ui: &Ui, options: TerminalOptions) -> Terminal {
    let mut emu = Emulator::new(80, 24);
    emu.set_scrollback(options.scrollback);
    if let Some(l) = options.string_limit {
        emu.set_string_limit(l);
    }
    emu.state.colors = theme_colors(&ui.theme());
    let tick = Signal::new(0u64);
    let notify = remedy_ui_reactive::waker().ok().map(|w| (w, tick));
    let shared = Shared::new(emu, notify);
    let measure = ui.el("terminal_measure").text(RichText::plain("0000000000")).nowrap().data_text();
    let measure_box = ui
        .el("terminal_measure_box")
        .height(0.0)
        .clip()
        .layout(|l| l.shrink = 0.0)
        .semantics(Semantics::new(Role::Group).hidden(true))
        .child(measure.clone());
    let top = ui.el("terminal_space").layout(|l| l.shrink = 0.0);
    let rows_box = ui.el("terminal_rows").column().fill_width().layout(|l| l.shrink = 0.0);
    let bottom = ui.el("terminal_space").layout(|l| l.shrink = 0.0);
    let scroller = ui
        .el("terminal_scroll")
        .scroll()
        .column()
        .layout(|l| {
            l.grow = 1.0;
            l.min_height = Length::Px(0.0);
        })
        .child(top.clone())
        .child(rows_box.clone())
        .child(bottom.clone());
    // The terminal takes the room it is given, never the height of its scrollback.
    let fill = |l: &mut remedy_ui_layout::LayoutStyle| {
        l.grow = 1.0;
        l.min_height = Length::Px(0.0);
    };
    let root =
        ui.el("terminal").column().fill_width().layout(fill).child(measure_box).child(scroller.clone());
    let announcer = ui.el("terminal_announcer").height(0.0).layout(|l| l.shrink = 0.0);
    let outer =
        ui.el("terminal_box").column().fill_width().layout(fill).child(root.clone()).child(announcer.clone());
    let look = Look {
        cell_w: 0.0,
        line_h: 0.0,
        baseline: 0.0,
        size: 0.0,
        weight: 400.0,
        families: Vec::new(),
        scale: 1.0,
        measured: false,
        monospace: false,
        theme_colors: None,
        overrides: Vec::new(),
        palette: None,
        palette_for_emulator: false,
        reverse: false,
        generation: 0,
    };
    let st = St {
        cols: 80,
        rows: 24,
        alt: false,
        history: 0,
        dropped: 0,
        cache: HashMap::new(),
        shown: BTreeMap::new(),
        follow: true,
        max_scroll: 0.0,
        view: Size::ZERO,
        cursor: CursorState { row: 0, col: 0, visible: true, shape: CursorShape::Block, blink: false },
        modes: Modes::default(),
        sel: None,
        selecting: false,
        hover_link: 0,
        preedit: None,
        focused: false,
        wheel: 0.0,
        mouse_down: None,
        last_report: None,
        spoken_to: None,
        a11y_pending: false,
        line_said: None,
        rounds_since_reread: REANNOUNCE_ROUNDS,
        blink_on: true,
        blink_armed: false,
        sync_waiting: false,
        forced_cols: None,
        autoscroll: None,
        autoscroll_armed: false,
        last_selection: None,
    };
    let inner = Inner {
        ui: ui.clone(),
        shared,
        outer,
        root: root.clone(),
        scroller: scroller.clone(),
        top,
        rows_box,
        bottom,
        options,
        st: RefCell::new(st),
        cb: RefCell::new(Callbacks::default()),
        look: Rc::new(RefCell::new(look)),
        glyphs: Rc::new(RefCell::new(GlyphCache::default())),
        title: Signal::new(String::new()),
        cwd: Signal::new(None),
        screen_text: Signal::new(String::new()),
        announce: Signal::new(String::new()),
    };
    let t = Terminal(Rc::new(inner));
    t.wire(root, scroller, measure, announcer, tick);
    // Short of memory: the scrollback gives back what it can. The listener holds the
    // terminal weakly, so a closed terminal is not kept alive by it.
    let weak = Rc::downgrade(&t.0);
    remedy_ui_view::device::on_memory_pressure(move |level| {
        if let Some(inner) = weak.upgrade() {
            inner.shared.ui_lock().relieve(level);
            Terminal(inner).sync();
        }
    });
    t
}

impl Terminal {
    fn wire(&self, root: El, scroller: El, measure: El, announcer: El, tick: Signal<u64>) {
        let ui = &self.0.ui;
        let label = self.0.options.label.clone();
        let (title, screen) = (self.0.title, self.0.screen_text);
        let t = self.clone();
        let _ = measure.on_resize(move |_| {
            t.measure();
        });
        let t = self.clone();
        // Wider than the view when the program asks for 132 columns: it scrolls sideways.
        let scroller = scroller.scroll_x();
        let _ = scroller.clone().on_resize(move |s| t.viewport(s));
        let t = self.clone();
        let _ = scroller.on_scroll(move |o| t.scrolled(o.y));
        let announce = self.0.announce;
        let _ = announcer
            .semantics_dyn(move || Semantics::new(Role::Status).live(Live::Polite).label(announce.get()));
        let (t1, t2, t3, t4, t5, t6) =
            (self.clone(), self.clone(), self.clone(), self.clone(), self.clone(), self.clone());
        let caret = move || t5.caret_rect();
        let _ = root
            .keyboard_alternative("Ctrl+Shift+A selects everything and Ctrl+Shift+C copies it")
            .on_key(move |e| t1.key(e))
            .on_pointer(move |e| t2.pointer(e))
            .on_wheel(move |e| t3.wheel(e))
            .on_focus_change(move |f| t4.focus(f))
            .on_text_input(caret, move |i| t6.text_input(i))
            .semantics_dyn(move || {
                let name = title.with(|t| if t.is_empty() { label.clone() } else { format!("{label}: {t}") });
                Semantics::new(Role::MultilineTextInput).label(name).value(screen.get())
            });
        let t = self.clone();
        let _ = self.0.outer.clone().bind(move || {
            let _ = tick.get();
            t.sync();
        });
        let _ = ui;
    }

    /// The element to put in the tree.
    pub fn element(&self) -> El {
        self.0.outer.clone()
    }

    /// The focusable terminal inside [`Terminal::element`] (to focus it, or test it).
    pub fn input_element(&self) -> El {
        self.0.root.clone()
    }

    /// A handle that feeds the program's output in, from any thread.
    pub fn feeder(&self) -> Feeder {
        Feeder { shared: self.0.shared.clone() }
    }

    /// Calls `f` with bytes for the program: typed keys, pastes, pointer reports, focus
    /// changes and answers to its queries. The app writes them to the PTY.
    pub fn on_output(&self, f: impl Fn(&[u8]) + 'static) {
        self.0.cb.borrow_mut().output = Some(Rc::new(f));
    }

    /// Calls `f` when the terminal's rows or columns change, for the app to resize the PTY.
    pub fn on_resize(&self, f: impl Fn(GridSize) + 'static) {
        self.0.cb.borrow_mut().resize = Some(Rc::new(f));
    }

    /// Calls `f` with a hyperlink's address when someone opens it: Ctrl+click (Command+click
    /// on Apple platforms) on a link the program printed (OSC 8). Links never open by
    /// themselves; without this, they do nothing.
    pub fn on_link(&self, f: impl Fn(&str) + 'static) {
        self.0.cb.borrow_mut().link = Some(Rc::new(f));
    }

    /// Decides whether the program may put text on the clipboard (OSC 52): `f` gets the text
    /// and returns true to allow it (asking the person first, or by a setting). Without this,
    /// programs cannot write the clipboard. Programs can never read it.
    pub fn on_clipboard_write(&self, f: impl Fn(&str) -> bool + 'static) {
        self.0.cb.borrow_mut().clipboard = Some(Rc::new(f));
    }

    /// Calls `f` when the program rings the bell.
    pub fn on_bell(&self, f: impl Fn() + 'static) {
        self.0.cb.borrow_mut().bell = Some(Rc::new(f));
    }

    /// The title the program set (OSC 0 or 2).
    pub fn title(&self) -> Signal<String> {
        self.0.title
    }

    /// The current size.
    pub fn size(&self) -> GridSize {
        let st = self.0.st.borrow();
        let look = self.0.look.borrow();
        GridSize {
            cols: st.cols,
            rows: st.rows,
            cell_width: (look.cell_w * look.scale).round() as u32,
            cell_height: (look.line_h * look.scale).round() as u32,
        }
    }

    /// Reads the emulator: the screen, the scrollback, modes (for tests, recordings and
    /// agents reading the session).
    pub fn with_emulator<R>(&self, f: impl FnOnce(&Emulator) -> R) -> R {
        f(&self.0.shared.ui_lock())
    }

    /// Sets how much scrollback to keep.
    pub fn set_scrollback(&self, s: Scrollback) {
        self.0.shared.ui_lock().set_scrollback(s);
        self.sync();
    }

    /// Sends `text` to the program as a paste (bracketed when it asked for that).
    pub fn paste(&self, text: &str) {
        let bracketed = self.0.st.borrow().modes.bracketed_paste;
        self.emit(&keys::encode_paste(text, bracketed));
        self.to_bottom();
    }

    /// The selected text, wrapped lines joined and trailing blanks left out.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.0.st.borrow().sel?;
        let ((sk, sc), (ek, ec)) = self.sel_bounds(sel);
        let emu = self.0.shared.ui_lock();
        let mut out = String::new();
        for k in sk..=ek {
            let Some(line) = line_by_key(&emu, k) else { continue };
            let from = if k == sk { sc } else { 0 };
            let to = if k == ek { ec.saturating_add(1) } else { line.cells.len() };
            let joined = line.wrapped && k != ek;
            out.push_str(&line.text_range(from, to, !joined));
            if k != ek && !line.wrapped {
                out.push('\n');
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// Copies the selection to the clipboard. False when nothing is selected.
    pub fn copy_selection(&self) -> bool {
        match self.selected_text() {
            Some(t) => {
                self.0.ui.copy_text(t);
                true
            }
            None => false,
        }
    }

    /// Selects everything: the scrollback and the screen.
    pub fn select_all(&self) {
        {
            let mut st = self.0.st.borrow_mut();
            let total = total(&st);
            if total == 0 {
                return;
            }
            let first = key_of(&st, 0);
            let last = key_of(&st, total - 1);
            st.sel = Some(Sel { anchor: (first, 0), head: (last, usize::MAX), mode: SelMode::Cell });
        }
        self.refresh_views();
    }

    /// Clears the selection.
    pub fn clear_selection(&self) {
        let had = self.0.st.borrow_mut().sel.take().is_some();
        if had {
            self.refresh_views();
        }
    }

    /// Scrolls to the newest output and keeps following it.
    pub fn scroll_to_bottom(&self) {
        self.to_bottom();
    }

    /// The working directory the shell reported (OSC 7), as a path; reading it in a binding
    /// follows its changes.
    pub fn cwd(&self) -> Signal<Option<String>> {
        self.0.cwd
    }

    /// True when the font in use gives every character the same width (a real monospace
    /// face); false when characters are centered in their cells to line up.
    pub fn monospace(&self) -> bool {
        self.0.look.borrow().monospace
    }

    /// Scrolls to the shell's previous prompt above the view's top (`older`), or the next one
    /// below it, marked with OSC 133. False when there is none that way.
    pub fn jump_to_prompt(&self, older: bool) -> bool {
        let prompts: Vec<u64> = self.with_emulator(|e| {
            e.prompt_lines()
                .into_iter()
                .filter(|(_, m)| m.contains(&crate::cell::PromptMark::Prompt))
                .map(|(k, _)| k)
                .collect()
        });
        let lh = self.0.look.borrow().line_h;
        if lh <= 0.0 {
            return false;
        }
        let id = self.0.scroller.id();
        // From where the view is: following the output, from the cursor's line (so the
        // previous prompt is the last command's); scrolled back, from the line at the top.
        let (dropped, top_key) = {
            let st = self.0.st.borrow();
            let y = self.0.ui.scroll_offset(id).y;
            let from = if st.follow {
                st.dropped + (st.history + st.cursor.row) as u64
            } else {
                st.dropped + (y / lh).round() as u64
            };
            (st.dropped, from)
        };
        let target = if older {
            prompts.iter().rev().find(|k| **k < top_key).copied()
        } else {
            prompts.iter().find(|k| **k > top_key).copied()
        };
        let Some(k) = target else { return false };
        let y = ((k - dropped) as f32 * lh).min(self.0.st.borrow().max_scroll);
        self.0.st.borrow_mut().follow = false;
        self.0.scroller.scroll_to(Vec2::new(0.0, y));
        self.0.ui.set_scroll_offset(id, Vec2::new(0.0, y));
        true
    }

    /// Selects the output of the last command the shell marked (OSC 133 `C`, up to the next
    /// prompt). False when no command output is marked.
    pub fn select_command_output(&self) -> bool {
        let (marks, end) = self.with_emulator(|e| {
            let s = &e.state;
            let cursor = s.dropped + (s.history.len() + s.cursor_pos().0) as u64;
            (e.prompt_lines(), cursor)
        });
        let Some(start) =
            marks.iter().rev().find(|(_, m)| m.contains(&crate::cell::PromptMark::Output)).map(|(k, _)| *k)
        else {
            return false;
        };
        let next_prompt = marks
            .iter()
            .find(|(k, m)| *k > start && m.contains(&crate::cell::PromptMark::Prompt))
            .map(|(k, _)| *k);
        let last = next_prompt.map_or(end, |p| p.saturating_sub(1)).max(start);
        self.0.st.borrow_mut().sel =
            Some(Sel { anchor: (start, 0), head: (last, usize::MAX - 1), mode: SelMode::Cell });
        self.refresh_views();
        true
    }

    /// Takes what the feeder parsed since the last look and shows it. This happens by
    /// itself after each feed; call it to show output at once (tests, a feed on the UI
    /// thread before a frame).
    pub fn sync(&self) {
        let ui = self.0.ui.clone();
        // What changed, taken under the lock; everything else happens after it.
        let taken = {
            let mut emu = self.0.shared.ui_lock();
            let s = &mut emu.state;
            if s.modes.synchronized
                && let Some(since) = s.sync_since
                && since.elapsed() < SYNC_TIMEOUT
            {
                let wait = SYNC_TIMEOUT.saturating_sub(since.elapsed());
                drop(emu);
                let already = std::mem::replace(&mut self.0.st.borrow_mut().sync_waiting, true);
                if !already {
                    let t = self.clone();
                    let _ = ui.after(wait, move || {
                        t.0.st.borrow_mut().sync_waiting = false;
                        t.sync();
                    });
                }
                return;
            }
            // The theme changed since the emulator last heard: color queries answer with it.
            if std::mem::take(&mut self.0.look.borrow_mut().palette_for_emulator) {
                s.colors = theme_colors(&ui.theme());
            }
            let rows = s.rows;
            let damage =
                std::mem::replace(&mut s.damage, crate::term::Damage { rows: vec![false; rows], all: false });
            Taken {
                cols: s.cols,
                rows,
                alt: s.modes.alt_screen,
                history: s.history.len(),
                dropped: s.dropped,
                cursor: CursorState {
                    row: emu_cursor(s).0,
                    col: emu_cursor(s).1,
                    visible: s.modes.cursor_visible,
                    shape: s.shape,
                    blink: s.modes.cursor_blink,
                },
                modes: s.modes,
                damage,
                title: std::mem::take(&mut s.out.title).then(|| s.title.clone()),
                replies: std::mem::take(&mut s.out.replies),
                bells: std::mem::take(&mut s.out.bells),
                clipboard: s.out.clipboard.take(),
                colors: std::mem::take(&mut s.out.colors).then(|| {
                    let mut v: Vec<(u16, (u8, u8, u8))> = s.overrides.iter().map(|(k, c)| (*k, *c)).collect();
                    v.sort_unstable();
                    v
                }),
                cwd: std::mem::take(&mut s.out.cwd).then(|| s.cwd.clone()),
                columns: s.out.columns.take().is_some(),
                forced: s.forced_cols,
            }
        };
        let mut reset = false;
        // Scrolled back while the oldest lines leave the scrollback: the view stays on the
        // lines it shows, moving up as many lines as left.
        let anchor = {
            let st = self.0.st.borrow();
            let gone = taken.dropped.saturating_sub(st.dropped);
            (!st.follow && !st.alt && !taken.alt && gone > 0).then_some(gone)
        };
        if let Some(gone) = anchor {
            let lh = self.0.look.borrow().line_h;
            let id = self.0.scroller.id();
            let y = (ui.scroll_offset(id).y - gone as f32 * lh).max(0.0);
            self.0.scroller.scroll_to(Vec2::new(0.0, y));
            ui.set_scroll_offset(id, Vec2::new(0.0, y));
        }
        {
            let mut st = self.0.st.borrow_mut();
            let geometry = (taken.cols, taken.rows, taken.alt) != (st.cols, st.rows, st.alt)
                || taken.history < st.history && taken.dropped == st.dropped;
            if taken.damage.all || geometry {
                st.cache.clear();
                reset = true;
                if geometry {
                    st.sel = None;
                }
            } else {
                let base = if taken.alt { ALT } else { taken.dropped + taken.history as u64 };
                for (r, d) in taken.damage.rows.iter().enumerate() {
                    if *d {
                        st.cache.remove(&(base + r as u64));
                    }
                }
            }
            // The selection's lines left the scrollback: it goes too.
            if let Some(s) = st.sel
                && !taken.alt
                && s.anchor.0.min(s.head.0) < taken.dropped
            {
                st.sel = None;
            }
            st.cols = taken.cols;
            st.rows = taken.rows;
            st.alt = taken.alt;
            st.history = taken.history;
            st.dropped = taken.dropped;
            st.cursor = taken.cursor;
            st.modes = taken.modes;
        }
        {
            let mut look = self.0.look.borrow_mut();
            if look.reverse != taken.modes.reverse {
                look.reverse = taken.modes.reverse;
                look.generation += 1;
            }
            if let Some(o) = taken.colors {
                look.overrides = o;
                look.palette = None;
                look.generation += 1;
            }
        }
        if reset {
            self.0.look.borrow_mut().generation += 1;
        }
        self.layout_rows();
        self.arm_blink();
        self.schedule_a11y();
        if let Some(t) = taken.title {
            let _ = self.0.title.try_set(t);
        }
        if let Some(c) = taken.cwd {
            let _ = self.0.cwd.try_set(c);
        }
        let forced_changed =
            std::mem::replace(&mut self.0.st.borrow_mut().forced_cols, taken.forced) != taken.forced;
        if forced_changed || taken.columns {
            // The program switched columns: the grid shows them, and the app's PTY hears.
            let view = self.0.st.borrow().view;
            self.apply_columns();
            self.viewport(view);
            let f = self.0.cb.borrow().resize.clone();
            if let Some(f) = f {
                f(self.size());
            }
        }
        if !taken.replies.is_empty() {
            self.emit(&taken.replies);
        }
        if taken.bells > 0 {
            let bell = self.0.cb.borrow().bell.clone();
            if let Some(b) = bell {
                b();
            }
        }
        if let Some(text) = taken.clipboard {
            let consent = self.0.cb.borrow().clipboard.clone();
            if consent.is_some_and(|f| f(&text)) {
                ui.copy_text(text);
            }
        }
    }

    // ----- Layout -----

    /// Measures a cell in the terminal's font. True when the metrics changed.
    fn measure(&self) -> bool {
        let ui = &self.0.ui;
        let Some(style) = self.0.root.style_in(self.0.root.states()) else { return false };
        let families = self.0.options.families.clone().unwrap_or_else(|| {
            // The system's monospace face first; the theme's own after it, for systems (and
            // collections) without one.
            let mut f = vec!["ui-monospace".to_owned(), "monospace".to_owned()];
            f.extend(ui.theme().mono_families.iter().cloned());
            f
        });
        let scale = ui.scale();
        {
            let look = self.0.look.borrow();
            if look.measured
                && look.size == style.text_size
                && look.weight == style.weight
                && look.families == families
                && look.scale == scale
            {
                return false;
            }
        }
        let text_style = TextStyle {
            families: families.clone(),
            size: style.text_size,
            weight: style.weight,
            ..TextStyle::default()
        };
        let options = ParagraphOptions { scale, ..ParagraphOptions::default() };
        let Some(p) = ui.layout_text(&RichText::plain("0000000000"), &text_style, options) else {
            return false;
        };
        // Monospace when narrow and wide letters take the same room as figures.
        let width_of =
            |s: &str| ui.layout_text(&RichText::plain(s.to_owned()), &text_style, options).map(|q| q.width());
        let zeros = p.width();
        let monospace = [width_of("iiiiiiiiii"), width_of("WWWWWWWWWW"), width_of("..........")]
            .iter()
            .all(|w| w.is_some_and(|w| (w - zeros).abs() <= zeros * 0.01));
        let snap_up = |v: f32| (v * scale).ceil().max(1.0) / scale;
        let cell_w = snap_up(p.width() / 10.0);
        let line_h = snap_up(p.height());
        let baseline = p.first_baseline().unwrap_or(line_h * 0.8);
        {
            let mut look = self.0.look.borrow_mut();
            look.cell_w = cell_w;
            look.line_h = line_h;
            look.baseline = baseline;
            look.size = style.text_size;
            look.weight = style.weight;
            look.families = families;
            look.scale = scale;
            look.measured = true;
            look.monospace = monospace;
            look.generation += 1;
        }
        let mut g = self.0.glyphs.borrow_mut();
        g.map.clear();
        drop(g);
        let shown: Vec<El> = self.0.st.borrow().shown.values().map(|r| r.el.clone()).collect();
        for el in shown {
            let _ = el.height(line_h);
        }
        let view = self.0.st.borrow().view;
        if view != Size::ZERO {
            self.viewport(view);
        }
        true
    }

    /// The scroll view's size changed: the grid gets the rows and columns that fit.
    fn viewport(&self, size: Size) {
        self.0.st.borrow_mut().view = size;
        let (cw, lh, scale) = {
            let look = self.0.look.borrow();
            (look.cell_w, look.line_h, look.scale)
        };
        if cw <= 0.0 || lh <= 0.0 {
            if self.measure() {
                return;
            }
            self.layout_rows();
            return;
        }
        let forced = self.0.st.borrow().forced_cols;
        let cols = forced.unwrap_or(((size.width / cw).floor() as usize).max(1));
        let rows = ((size.height / lh).floor() as usize).max(1);
        let changed = {
            let st = self.0.st.borrow();
            (st.cols, st.rows) != (cols, rows)
        };
        if changed {
            {
                let mut emu = self.0.shared.ui_lock();
                emu.resize(cols, rows);
                emu.state.cell_px = ((cw * scale).round() as u32, (lh * scale).round() as u32);
            }
            self.0.glyphs.borrow_mut().limit = (cols * rows * 4).max(4096);
            self.sync();
            let f = self.0.cb.borrow().resize.clone();
            if let Some(f) = f {
                f(self.size());
            }
        } else {
            self.layout_rows();
        }
    }

    /// Rows as wide as the columns the program asked for (DECCOLM), or as the view.
    fn apply_columns(&self) {
        let forced = self.0.st.borrow().forced_cols;
        let cw = self.0.look.borrow().cell_w;
        let _ = self.0.rows_box.clone().layout(|l| {
            l.width = match forced {
                Some(c) if cw > 0.0 => Length::Px(c as f32 * cw),
                _ => Length::Auto,
            };
            l.min_width = l.width;
        });
    }

    /// Builds the rows in view (and a few around it), drops the rest, and gives each row
    /// what it shows.
    fn layout_rows(&self) {
        let (lh, cw) = {
            let look = self.0.look.borrow();
            (look.line_h, look.cell_w)
        };
        if lh <= 0.0 || cw <= 0.0 {
            return;
        }
        let ui = self.0.ui.clone();
        let offset = ui.scroll_offset(self.0.scroller.id()).y;
        let (range, total, follow, view_h) = {
            let mut st = self.0.st.borrow_mut();
            let total = total(&st);
            let max = (total as f32 * lh - st.view.height).max(0.0);
            st.max_scroll = max;
            let y = if st.follow { max } else { offset.min(max) };
            let first = ((y / lh).floor() as usize).saturating_sub(PREFETCH_ROWS);
            let last = (((y + st.view.height) / lh).ceil() as usize + PREFETCH_ROWS).min(total);
            (first.min(last)..last, total, st.follow, st.view.height)
        };
        let _ = self.0.top.clone().layout(|l| l.height = Length::Px(range.start as f32 * lh));
        let _ = self.0.bottom.clone().layout(|l| l.height = Length::Px((total - range.end) as f32 * lh));
        if follow {
            let max = (total as f32 * lh - view_h).max(0.0);
            if (offset - max).abs() > 0.5 {
                self.0.scroller.scroll_to(Vec2::new(0.0, max));
            }
        }
        // Rows leaving the range go; rows entering it are built.
        let mut gone = Vec::new();
        let mut make = Vec::new();
        {
            let mut st = self.0.st.borrow_mut();
            let keys: Vec<usize> = st.shown.keys().copied().collect();
            for i in keys {
                if !range.contains(&i)
                    && let Some(r) = st.shown.remove(&i)
                {
                    gone.push(r.el);
                }
            }
            for i in range.clone() {
                if !st.shown.contains_key(&i) {
                    make.push(i);
                }
            }
        }
        for el in gone {
            el.remove();
        }
        let first_shown = self.0.st.borrow().shown.keys().next().copied();
        let mut before = 0;
        for i in make {
            let row = self.make_row();
            match first_shown {
                Some(f) if i < f => {
                    self.0.rows_box.insert_child(before, &row.el);
                    before += 1;
                }
                _ => {
                    let _ = self.0.rows_box.clone().child(row.el.clone());
                }
            }
            self.0.st.borrow_mut().shown.insert(i, row);
        }
        // Lines the rows need and the cache lacks, read under one lock.
        let need: Vec<u64> = {
            let st = self.0.st.borrow();
            range.clone().map(|i| key_of(&st, i)).filter(|k| !st.cache.contains_key(k)).collect()
        };
        if !need.is_empty() {
            let lines: Vec<(u64, Line)> = {
                let emu = self.0.shared.ui_lock();
                need.iter().filter_map(|k| line_by_key(&emu, *k).map(|l| (*k, l.clone()))).collect()
            };
            let mut st = self.0.st.borrow_mut();
            for (k, l) in lines {
                st.cache.insert(k, Rc::new(l));
            }
        }
        {
            let mut st = self.0.st.borrow_mut();
            let keep: std::collections::HashSet<u64> = range.clone().map(|i| key_of(&st, i)).collect();
            st.cache.retain(|k, _| keep.contains(k));
        }
        self.refresh_views();
    }

    fn make_row(&self) -> Row {
        let view = Signal::new(RowView::default());
        let (look, glyphs) = (self.0.look.clone(), self.0.glyphs.clone());
        let lh = self.0.look.borrow().line_h;
        let el = self.0.ui.el("terminal_row").height(lh).fill_width().layout(|l| l.shrink = 0.0).paint(
            move |cx| {
                let v = view.get();
                paint_row(cx, &v, &mut look.borrow_mut(), &mut glyphs.borrow_mut());
            },
        );
        Row { el, view }
    }

    /// Gives every shown row what it shows now; rows whose view did not change keep their
    /// paint.
    fn refresh_views(&self) {
        let st = self.0.st.borrow();
        let generation = self.0.look.borrow().generation;
        let screen_first = total(&st) - st.rows;
        let bounds = st.sel.map(|s| self.sel_bounds_with(&st, s));
        for (i, row) in &st.shown {
            let key = key_of(&st, *i);
            let line = st.cache.get(&key).cloned();
            let cursor_row = screen_first + st.cursor.row;
            let cursor = (*i == cursor_row && st.cursor.visible && (st.blink_on || !st.focused))
                .then_some(CursorDraw { col: st.cursor.col, shape: st.cursor.shape, focused: st.focused });
            let sel = bounds.and_then(|((sk, sc), (ek, ec))| {
                (key >= sk && key <= ek).then(|| {
                    let from = if key == sk { sc } else { 0 };
                    let to = if key == ek { ec.saturating_add(1) } else { usize::MAX };
                    (from, to)
                })
            });
            let preedit =
                if *i == cursor_row { st.preedit.clone().map(|p| (st.cursor.col, p)) } else { None };
            let v = RowView { line, cursor, sel, hover_link: st.hover_link, preedit, generation };
            if row.view.with_untracked(|old| *old != v).unwrap_or(true) {
                let _ = row.view.try_set(v);
            }
        }
    }

    /// The view scrolled (people scrolling, or the scroll this terminal asked for): at the
    /// end it follows new output; scrolled up, it stays where it is.
    fn scrolled(&self, y: f32) {
        let lh = self.0.look.borrow().line_h;
        {
            let mut st = self.0.st.borrow_mut();
            st.follow = y >= st.max_scroll - lh * 0.5;
        }
        self.layout_rows();
    }

    fn to_bottom(&self) {
        self.0.st.borrow_mut().follow = true;
        self.layout_rows();
    }

    // ----- Output -----

    fn emit(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let f = self.0.cb.borrow().output.clone();
        if let Some(f) = f {
            f(bytes);
        }
    }

    // ----- Keys -----

    fn key(&self, e: &KeyEvent) -> bool {
        if e.composing {
            return false;
        }
        if !e.pressed {
            // Releases go to the program only when it asked for them (kitty protocol).
            let modes = self.0.st.borrow().modes;
            return match keys::encode_key(e, &modes) {
                Some(b) => {
                    self.emit(&b);
                    true
                }
                None => false,
            };
        }
        let ui = &self.0.ui;
        let apple = ui.platform() == Platform::Apple;
        let m = e.modifiers;
        let ch = |c: &str| matches!(&e.key, Key::Character(k) if k.eq_ignore_ascii_case(c));
        let cmd_shift = if apple { m.meta && !m.ctrl } else { m.ctrl && m.shift && !m.alt };
        // The terminal's own shortcuts: copy, paste, select all, scroll the scrollback.
        if (cmd_shift && ch("c")) || (!apple && m.ctrl && !m.shift && e.key == Key::Named("Insert".into())) {
            self.copy_selection();
            return true;
        }
        if (cmd_shift && ch("v")) || (!apple && m.shift && !m.ctrl && e.key == Key::Named("Insert".into())) {
            ui.request_paste();
            return true;
        }
        if cmd_shift && ch("a") {
            self.select_all();
            return true;
        }
        let alt = self.0.st.borrow().alt;
        // Between the shell's prompts: Ctrl+Shift+Up and Down (Command+Up and Down).
        let prompt_jump = if apple { m.meta && !m.ctrl && !m.shift } else { m.ctrl && m.shift && !m.alt };
        if prompt_jump && !alt {
            match e.key {
                Key::ArrowUp => return self.jump_to_prompt(true),
                Key::ArrowDown => return self.jump_to_prompt(false),
                _ => {}
            }
        }
        if m.shift && !m.ctrl && !m.alt && !m.meta && !alt {
            let page = self.0.st.borrow().rows.saturating_sub(1).max(1) as f32;
            let lines = match e.key {
                Key::PageUp => Some(-page),
                Key::PageDown => Some(page),
                Key::Home => Some(f32::MIN),
                Key::End => Some(f32::MAX),
                _ => None,
            };
            if let Some(l) = lines {
                self.scroll_lines(l);
                return true;
            }
        }
        let modes = self.0.st.borrow().modes;
        let Some(bytes) = keys::encode_key(e, &modes) else { return false };
        self.emit(&bytes);
        let had_sel = self.0.st.borrow_mut().sel.take().is_some();
        self.0.st.borrow_mut().follow = true;
        if had_sel {
            self.refresh_views();
        }
        self.layout_rows();
        true
    }

    fn scroll_lines(&self, lines: f32) {
        let lh = self.0.look.borrow().line_h;
        let ui = &self.0.ui;
        let now = ui.scroll_offset(self.0.scroller.id()).y;
        let (total, view_h) = {
            let st = self.0.st.borrow();
            (total(&st), st.view.height)
        };
        let max = (total as f32 * lh - view_h).max(0.0);
        let y = if lines == f32::MIN {
            0.0
        } else if lines == f32::MAX {
            max
        } else {
            (now + lines * lh).clamp(0.0, max)
        };
        self.0.st.borrow_mut().follow = y >= max - 0.5;
        self.0.scroller.scroll_to(Vec2::new(0.0, y));
        ui.set_scroll_offset(self.0.scroller.id(), Vec2::new(0.0, y));
    }

    // ----- Text input -----

    fn caret_rect(&self) -> Rect {
        let (Some(rb), Some(sb)) = (self.0.root.bounds(), self.0.scroller.bounds()) else {
            return Rect::ZERO;
        };
        let look = self.0.look.borrow();
        let st = self.0.st.borrow();
        let offset = self.0.ui.scroll_offset(self.0.scroller.id()).y;
        let line = (total(&st) - st.rows + st.cursor.row) as f32;
        let x = sb.min.x - rb.min.x + st.cursor.col as f32 * look.cell_w;
        let y = sb.min.y - rb.min.y + line * look.line_h - offset;
        Rect::new(x, y, x + look.cell_w, y + look.line_h)
    }

    fn text_input(&self, input: &TextInput) {
        match input {
            TextInput::Paste(text) => self.paste(text),
            TextInput::Ime(ImeEvent::Commit(text)) => {
                self.0.st.borrow_mut().preedit = None;
                self.emit(text.as_bytes());
                self.to_bottom();
            }
            TextInput::Ime(ImeEvent::Preedit { text, .. }) => {
                self.0.st.borrow_mut().preedit = (!text.is_empty()).then(|| text.clone());
                self.refresh_views();
            }
            TextInput::Ime(ImeEvent::Disabled) => {
                self.0.st.borrow_mut().preedit = None;
                self.refresh_views();
            }
            TextInput::Ime(ImeEvent::Enabled | ImeEvent::DeleteSurrounding { .. }) => {}
        }
    }

    // ----- Focus and the cursor -----

    fn focus(&self, focused: bool) {
        self.0.st.borrow_mut().focused = focused;
        if focused {
            self.0.ui.clear_text_selection();
        }
        let report = self.0.st.borrow().modes.focus_events;
        if report {
            self.emit(keys::encode_focus(focused));
        }
        self.0.st.borrow_mut().blink_on = true;
        self.arm_blink();
        self.refresh_views();
    }

    fn arm_blink(&self) {
        let ui = self.0.ui.clone();
        let want = {
            let st = self.0.st.borrow();
            st.cursor.blink && st.focused && st.cursor.visible && !ui.reduced_motion()
        };
        let mut st = self.0.st.borrow_mut();
        if !want {
            if !st.blink_on {
                st.blink_on = true;
                drop(st);
                self.refresh_views();
            }
            return;
        }
        if st.blink_armed {
            return;
        }
        st.blink_armed = true;
        drop(st);
        let t = self.clone();
        let _ = ui.after(BLINK, move || {
            {
                let mut st = t.0.st.borrow_mut();
                st.blink_armed = false;
                st.blink_on = !st.blink_on;
            }
            t.refresh_views();
            t.arm_blink();
        });
    }

    // ----- Screen readers -----

    fn schedule_a11y(&self) {
        if std::mem::replace(&mut self.0.st.borrow_mut().a11y_pending, true) {
            return;
        }
        let t = self.clone();
        let _ = self.0.ui.after(ANNOUNCE_EVERY, move || t.refresh_a11y());
    }

    /// Tells screen readers the screen as it is now, and reads out the output since the last
    /// time: the text from where the cursor was then to where it is now (at most a screenful).
    /// Output that only rewrites what was there (a progress bar) is not read again.
    fn refresh_a11y(&self) {
        let last = {
            let mut st = self.0.st.borrow_mut();
            st.a11y_pending = false;
            st.spoken_to
        };
        let (screen, news, now, line_now) = {
            let emu = self.0.shared.ui_lock();
            let s = &emu.state;
            let screen = emu.screen_text().trim_end_matches('\n').to_owned();
            let (row, col) = s.cursor_pos();
            if s.modes.alt_screen {
                // Full-screen programs redraw: the screen is there to read, nothing is
                // announced.
                (screen, String::new(), None, None)
            } else {
                let cursor_key = s.dropped + (s.history.len() + row) as u64;
                let line_now =
                    line_by_key(&emu, cursor_key).map(|l| (cursor_key, l.text().trim().to_owned()));
                let now = (s.dropped + (s.history.len() + row) as u64, col);
                let top = (now.0 + 1).saturating_sub(s.rows as u64).max(s.dropped);
                let from = match last {
                    Some(l) if l <= now && l.0 >= top => l,
                    Some(l) if l.0 < top => (top, 0),
                    // The cursor went back: only its line is new.
                    Some(_) => (now.0, 0),
                    None => (top, 0),
                };
                let mut parts = Vec::new();
                for k in from.0..=now.0 {
                    let Some(line) = line_by_key(&emu, k) else { continue };
                    let a = if k == from.0 { from.1 } else { 0 };
                    let b = if k == now.0 { now.1.max(line.used()) } else { line.cells.len() };
                    let t = line.text_range(a, b, true);
                    if !t.trim().is_empty() {
                        parts.push(t.trim().to_owned());
                    }
                }
                (
                    screen,
                    parts.join("\n"),
                    Some((now.0, now.1.max(line_by_key(&emu, now.0).map_or(0, Line::used)))),
                    line_now,
                )
            }
        };
        // A line rewritten in place (a progress bar, a spinner with a status) is read again,
        // at most every few rounds; the latest state is read when the wait is over.
        let mut news = news;
        let mut again = false;
        {
            let mut st = self.0.st.borrow_mut();
            st.spoken_to = now;
            st.rounds_since_reread = st.rounds_since_reread.saturating_add(1);
            match (&line_now, &st.line_said) {
                (Some((k, text)), Some((said_k, said))) if news.is_empty() && k == said_k && text != said => {
                    if text.is_empty() {
                        st.line_said = line_now.clone();
                    } else if st.rounds_since_reread >= REANNOUNCE_ROUNDS {
                        news = text.clone();
                        st.line_said = line_now.clone();
                        st.rounds_since_reread = 0;
                    } else {
                        again = true;
                    }
                }
                _ => st.line_said = line_now.clone(),
            }
        }
        if again {
            self.schedule_a11y();
        }
        if self.0.screen_text.with_untracked(|s| *s != screen).unwrap_or(true) {
            let _ = self.0.screen_text.try_set(screen);
        }
        if !news.is_empty() {
            let _ = self.0.announce.try_set(news);
        }
    }

    // ----- Pointer -----

    /// The display line and column under window point `p`.
    fn cell_at(&self, p: Point) -> Option<(usize, usize)> {
        let b = self.0.scroller.bounds()?;
        let look = self.0.look.borrow();
        if look.line_h <= 0.0 || look.cell_w <= 0.0 {
            return None;
        }
        let st = self.0.st.borrow();
        let total = total(&st);
        if total == 0 {
            return None;
        }
        let offset = self.0.ui.scroll_offset(self.0.scroller.id()).y;
        let y = (p.y - b.min.y + offset).max(0.0);
        let i = ((y / look.line_h).floor() as usize).min(total - 1);
        let col = (((p.x - b.min.x) / look.cell_w).floor().max(0.0) as usize).min(st.cols - 1);
        Some((i, col))
    }

    fn reporting(&self, mods: Modifiers) -> bool {
        // Shift keeps the pointer for selecting, as in every terminal.
        self.0.st.borrow().modes.mouse != MouseMode::Off && !mods.shift
    }

    fn report(&self, button: MouseButton, action: MouseAction, i: usize, col: usize, mods: Modifiers) {
        let (bytes, row) = {
            let st = self.0.st.borrow();
            let row = i.saturating_sub(total(&st) - st.rows).min(st.rows - 1);
            (keys::encode_mouse(button, action, col, row, mods, &st.modes), row)
        };
        self.0.st.borrow_mut().last_report = Some((row, col));
        if let Some(b) = bytes {
            self.emit(&b);
        }
    }

    fn pointer(&self, e: &PointerEvent) -> bool {
        let Some((i, col)) = self.cell_at(e.position) else { return false };
        let ui = self.0.ui.clone();
        let apple = ui.platform() == Platform::Apple;
        let link_mod = if apple { e.modifiers.meta } else { e.modifiers.ctrl };
        match e.phase {
            PointerPhase::Down => {
                ui.clear_text_selection();
                let button = if e.buttons.contains(Buttons::PRIMARY) {
                    MouseButton::Left
                } else if e.buttons.contains(Buttons::SECONDARY) {
                    MouseButton::Right
                } else if e.buttons.contains(Buttons::MIDDLE) {
                    MouseButton::Middle
                } else {
                    return false;
                };
                if button == MouseButton::Left && link_mod {
                    let link = self.link_at(i, col);
                    if link != 0 {
                        let uri = self.0.shared.ui_lock().link(link).map(str::to_owned);
                        let open = self.0.cb.borrow().link.clone();
                        if let (Some(uri), Some(open)) = (uri, open) {
                            open(&uri);
                        }
                        return true;
                    }
                }
                if self.reporting(e.modifiers) {
                    self.0.st.borrow_mut().mouse_down = Some(button);
                    self.report(button, MouseAction::Press, i, col, e.modifiers);
                    return true;
                }
                if button == MouseButton::Middle {
                    // The middle button pastes the primary selection (X11 and Wayland), or
                    // what was last selected here where the platform has none.
                    let fallback = self.0.st.borrow().last_selection.clone();
                    ui.request_paste_primary(fallback);
                    return true;
                }
                if button != MouseButton::Left {
                    return false;
                }
                let key = key_of(&self.0.st.borrow(), i);
                let mode = match ui.click_count() {
                    1 => SelMode::Cell,
                    2 => SelMode::Word,
                    _ => SelMode::Line,
                };
                {
                    let mut st = self.0.st.borrow_mut();
                    st.sel = match st.sel {
                        // Shift+click moves the selection's end.
                        Some(s) if e.modifiers.shift => Some(Sel { head: (key, col), ..s }),
                        _ => Some(Sel { anchor: (key, col), head: (key, col), mode }),
                    };
                    st.selecting = true;
                }
                self.refresh_views();
                true
            }
            PointerPhase::Move => {
                let down = self.0.st.borrow().mouse_down;
                if self.reporting(e.modifiers)
                    && (down.is_some() || self.0.st.borrow().modes.mouse == MouseMode::Motion)
                {
                    let row = {
                        let st = self.0.st.borrow();
                        i.saturating_sub(total(&st) - st.rows)
                    };
                    if self.0.st.borrow().last_report != Some((row, col)) {
                        self.report(
                            down.unwrap_or(MouseButton::None),
                            MouseAction::Move,
                            i,
                            col,
                            e.modifiers,
                        );
                    }
                    return true;
                }
                let selecting = self.0.st.borrow().selecting && e.buttons.contains(Buttons::PRIMARY);
                if selecting {
                    let key = key_of(&self.0.st.borrow(), i);
                    {
                        let mut st = self.0.st.borrow_mut();
                        if let Some(s) = st.sel.as_mut() {
                            s.head = (key, col);
                        }
                    }
                    self.refresh_views();
                    // Past the view's top or bottom edge: it scrolls on its own, faster the
                    // farther the pointer is.
                    let lh = self.0.look.borrow().line_h;
                    let speed = self.0.scroller.bounds().and_then(|b| {
                        let past = if e.position.y < b.min.y {
                            e.position.y - b.min.y
                        } else if e.position.y > b.max.y {
                            e.position.y - b.max.y
                        } else {
                            return None;
                        };
                        (lh > 0.0).then(|| (past / lh).abs().ceil().max(1.0).copysign(past))
                    });
                    self.0.st.borrow_mut().autoscroll = speed.map(|s| (s, e.position));
                    if speed.is_some() {
                        self.arm_autoscroll();
                    }
                    return true;
                }
                let link = self.link_at(i, col);
                let changed = {
                    let mut st = self.0.st.borrow_mut();
                    let changed = st.hover_link != link;
                    st.hover_link = link;
                    changed
                };
                if changed {
                    self.refresh_views();
                }
                false
            }
            PointerPhase::Up => {
                let down = self.0.st.borrow_mut().mouse_down.take();
                if let Some(b) = down {
                    self.report(b, MouseAction::Release, i, col, e.modifiers);
                    return true;
                }
                let mut st = self.0.st.borrow_mut();
                let was = st.selecting;
                st.selecting = false;
                st.autoscroll = None;
                // A press and release on one cell, without dragging, selects nothing.
                if let Some(s) = st.sel
                    && s.mode == SelMode::Cell
                    && s.anchor == s.head
                {
                    st.sel = None;
                    drop(st);
                    self.refresh_views();
                } else {
                    drop(st);
                    // What is selected becomes the primary selection (X11 and Wayland), and
                    // what the middle button pastes here.
                    if was && let Some(text) = self.selected_text() {
                        ui.set_primary_selection(text.clone());
                        self.0.st.borrow_mut().last_selection = Some(text);
                    }
                }
                was
            }
            PointerPhase::Cancel => {
                let mut st = self.0.st.borrow_mut();
                st.selecting = false;
                st.mouse_down = None;
                st.autoscroll = None;
                false
            }
        }
    }

    /// Keeps scrolling while a selection is dragged past the view's edge: each tick scrolls
    /// by the speed the pointer's distance set, and moves the selection's end with it.
    fn arm_autoscroll(&self) {
        if std::mem::replace(&mut self.0.st.borrow_mut().autoscroll_armed, true) {
            return;
        }
        let t = self.clone();
        let _ = self.0.ui.after(AUTOSCROLL_EVERY, move || {
            t.0.st.borrow_mut().autoscroll_armed = false;
            let step = {
                let st = t.0.st.borrow();
                st.autoscroll.filter(|_| st.selecting)
            };
            let Some((lines, at)) = step else { return };
            t.scroll_lines(lines);
            // The selection's end follows to the line now at the edge.
            if let Some(b) = t.0.scroller.bounds() {
                let y = at.y.clamp(b.min.y + 0.5, b.max.y - 0.5);
                if let Some((i, col)) = t.cell_at(Point::new(at.x, y)) {
                    let key = key_of(&t.0.st.borrow(), i);
                    if let Some(s) = t.0.st.borrow_mut().sel.as_mut() {
                        s.head = (key, col);
                    }
                    t.refresh_views();
                }
            }
            t.arm_autoscroll();
        });
    }

    fn link_at(&self, i: usize, col: usize) -> u32 {
        let st = self.0.st.borrow();
        let key = key_of(&st, i);
        st.cache.get(&key).and_then(|l| l.cells.get(col)).map_or(0, |c| c.link)
    }

    fn wheel(&self, e: &WheelEvent) -> bool {
        let lh = self.0.look.borrow().line_h;
        if lh <= 0.0 {
            return false;
        }
        let (alt, modes) = {
            let st = self.0.st.borrow();
            (st.alt, st.modes)
        };
        let reporting = modes.mouse != MouseMode::Off;
        let arrows = alt && modes.alt_scroll;
        if !reporting && !arrows {
            // The scrollback scrolls (the compositor moves it).
            return false;
        }
        let lines = {
            let mut st = self.0.st.borrow_mut();
            st.wheel += e.delta.y / lh;
            let whole = st.wheel.trunc();
            st.wheel -= whole;
            whole as i64
        };
        if lines == 0 {
            return true;
        }
        let up = lines < 0;
        if reporting {
            if let Some((i, col)) = self.cell_at(e.position) {
                let b = if up { MouseButton::WheelUp } else { MouseButton::WheelDown };
                for _ in 0..lines.unsigned_abs() {
                    self.report(b, MouseAction::Press, i, col, Modifiers::NONE);
                }
            }
        } else {
            let seq: &[u8] = match (up, modes.app_cursor) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let n = usize::try_from(lines.unsigned_abs()).unwrap_or(0);
            self.emit(&seq.repeat(n));
        }
        true
    }

    // ----- Selection -----

    fn sel_bounds(&self, s: Sel) -> ((u64, usize), (u64, usize)) {
        let st = self.0.st.borrow();
        self.sel_bounds_with(&st, s)
    }

    /// The selection's first and last cells (inclusive), widened to words or lines.
    fn sel_bounds_with(&self, _st: &St, s: Sel) -> ((u64, usize), (u64, usize)) {
        let (mut a, mut b) = if s.anchor <= s.head { (s.anchor, s.head) } else { (s.head, s.anchor) };
        match s.mode {
            SelMode::Cell => {}
            SelMode::Word => {
                let emu = self.0.shared.ui_lock();
                let word = |c: &Cell| {
                    c.ch.is_alphanumeric()
                        || c.attrs.contains(Attrs::WIDE_SPACER)
                        || (!c.ch.is_ascii() && !c.ch.is_whitespace())
                        || self.0.options.word_chars.contains(c.ch)
                };
                if let Some(line) = line_by_key(&emu, a.0)
                    && line.cells.get(a.1).is_some_and(word)
                {
                    while a.1 > 0 && line.cells.get(a.1 - 1).is_some_and(word) {
                        a.1 -= 1;
                    }
                }
                if let Some(line) = line_by_key(&emu, b.0)
                    && line.cells.get(b.1).is_some_and(word)
                {
                    while line.cells.get(b.1 + 1).is_some_and(word) {
                        b.1 += 1;
                    }
                }
            }
            SelMode::Line => {
                let emu = self.0.shared.ui_lock();
                // The whole logical line: back while the line before wrapped into this one,
                // on while this one wraps.
                while a.0 > 0 && line_by_key(&emu, a.0 - 1).is_some_and(|l| l.wrapped) {
                    a.0 -= 1;
                }
                while line_by_key(&emu, b.0).is_some_and(|l| l.wrapped)
                    && line_by_key(&emu, b.0 + 1).is_some()
                {
                    b.0 += 1;
                }
                a.1 = 0;
                b.1 = usize::MAX - 1;
            }
        }
        (a, b)
    }
}

/// What [`Terminal::sync`] takes from the emulator under its lock.
struct Taken {
    cols: usize,
    rows: usize,
    alt: bool,
    history: usize,
    dropped: u64,
    cursor: CursorState,
    modes: Modes,
    damage: crate::term::Damage,
    title: Option<String>,
    replies: Vec<u8>,
    bells: u32,
    clipboard: Option<String>,
    colors: Option<ColorChanges>,
    cwd: Option<Option<String>>,
    columns: bool,
    forced: Option<usize>,
}

/// The theme's terminal colors, before any change the program made, as the emulator answers
/// color queries with them.
fn theme_colors(theme: &Theme) -> crate::term::Colors {
    Palette::new(theme, palette::foreground(theme), palette::background(theme), &[]).to_rgb()
}

fn emu_cursor(s: &crate::term::State) -> (usize, usize) {
    s.cursor_pos()
}

/// Lines the view can scroll through: the scrollback and the screen (the screen alone on the
/// alternate screen).
fn total(st: &St) -> usize {
    if st.alt { st.rows } else { st.history + st.rows }
}

/// The cache key of display line `i`.
fn key_of(st: &St, i: usize) -> u64 {
    if st.alt { ALT | i as u64 } else { st.dropped + i as u64 }
}

/// The line a key names, as the emulator holds it now.
fn line_by_key(emu: &Emulator, key: u64) -> Option<&Line> {
    let s = &emu.state;
    if key & ALT != 0 {
        return if s.modes.alt_screen { s.grid.get((key & !ALT) as usize) } else { None };
    }
    let i = usize::try_from(key.checked_sub(s.dropped)?).ok()?;
    if s.modes.alt_screen {
        return s.history.get(i);
    }
    match i.checked_sub(s.history.len()) {
        None => s.history.get(i),
        Some(r) => s.grid.get(r),
    }
}

// ----- Painting -----

fn shaped(
    cx: &mut PaintCx<'_>,
    look: &Look,
    cache: &mut GlyphCache,
    text: &str,
    bold: bool,
    italic: bool,
) -> Rc<Shaped> {
    let style = u8::from(bold) | (u8::from(italic) << 1);
    if let Some(s) = cache.map.get(&(Box::from(text), style)) {
        return s.clone();
    }
    if cache.map.len() >= cache.limit.max(4096) {
        cache.map.clear();
    }
    let ts = TextStyle {
        families: look.families.clone(),
        size: look.size,
        weight: if bold { 700.0 } else { look.weight },
        italic,
        ..TextStyle::default()
    };
    let p = cx.layout_text(
        &RichText::plain(text.to_owned()),
        &ts,
        ParagraphOptions { scale: look.scale, ..ParagraphOptions::default() },
    );
    let base = p.first_baseline().unwrap_or(0.0);
    let runs = p
        .glyph_runs()
        .into_iter()
        .map(|r| (r.font, r.size, r.glyphs.iter().map(|g| (g.id, g.x, g.y - base)).collect()))
        .collect();
    let s = Rc::new(Shaped { runs, width: p.width() });
    cache.map.insert((Box::from(text), style), s.clone());
    s
}

/// The colors cell `c` is drawn in: foreground, background.
fn cell_colors(c: &Cell, pal: &Palette, dfg: Color, dbg: Color) -> (Color, Color) {
    let mut fg = pal.resolve(c.fg, dfg);
    let mut bg = pal.resolve(c.bg, dbg);
    if c.attrs.contains(Attrs::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if c.attrs.contains(Attrs::DIM) {
        fg = fg.mix_oklab(bg, 0.45);
    }
    if c.attrs.contains(Attrs::HIDDEN) {
        fg = bg;
    }
    (fg, bg)
}

fn push_glyphs(
    list: &mut DisplayList,
    batch: &mut Option<GlyphRun>,
    font: &RunFont,
    size: f32,
    color: Color,
    glyphs: impl Iterator<Item = (u32, Point)>,
) {
    let same = batch.as_ref().is_some_and(|b| {
        b.font.id() == font.id() && b.font.index() == font.index() && b.size == size && b.color == color
    });
    if !same && let Some(b) = batch.take() {
        list.push(Item::Glyphs(b));
    }
    let b = batch.get_or_insert_with(|| GlyphRun {
        font: font.clone(),
        size,
        color,
        glyphs: Vec::new(),
        hinting: Hinting::Full,
        antialias: Antialias::Grayscale,
    });
    b.glyphs.extend(glyphs);
}

fn decorate(list: &mut DisplayList, kind: Underline, x: f32, w: f32, y: f32, t: f32, color: Color) {
    let brush = Brush::Solid(color);
    match kind {
        Underline::None => {}
        Underline::Single => list.fill_rect(Rect::new(x, y, x + w, y + t), brush),
        Underline::Double => {
            list.fill_rect(Rect::new(x, y, x + w, y + t), brush.clone());
            list.fill_rect(Rect::new(x, y + t * 2.0, x + w, y + t * 3.0), brush);
        }
        Underline::Curly => {
            let mut path = Path::new();
            let amp = t * 1.5;
            let step = (t * 3.0).max(2.0);
            let mut px = x;
            path.move_to(Point::new(px, y + t));
            let mut up = true;
            while px < x + w {
                let nx = (px + step).min(x + w);
                let cy = if up { y + t - amp } else { y + t + amp };
                path.quad_to(Point::new((px + nx) * 0.5, cy), Point::new(nx, y + t));
                px = nx;
                up = !up;
            }
            list.push(Item::StrokePath { path, stroke: Stroke::new(t), brush });
        }
        Underline::Dotted | Underline::Dashed => {
            let mut path = Path::new();
            path.move_to(Point::new(x, y + t * 0.5)).line_to(Point::new(x + w, y + t * 0.5));
            let dashes = if kind == Underline::Dotted { vec![t, t] } else { vec![t * 3.0, t * 2.0] };
            let stroke = Stroke { dashes, ..Stroke::new(t) };
            list.push(Item::StrokePath { path, stroke, brush });
        }
    }
}

/// Draws one row: backgrounds, selection, the cursor, glyphs placed in their cells, lines
/// under and through, and the input method's composition at the cursor.
fn paint_row(cx: &mut PaintCx<'_>, v: &RowView, look: &mut Look, cache: &mut GlyphCache) {
    let (cw, lh, base, scale) = (look.cell_w, look.line_h, look.baseline, look.scale.max(0.1));
    if cw <= 0.0 || lh <= 0.0 {
        return;
    }
    let theme: &Theme = cx.theme;
    let pal = look.palette(theme);
    let (dfg, dbg) = if look.reverse { (pal.bg, pal.fg) } else { (pal.fg, pal.bg) };
    let width = cx.size.width;
    if look.reverse {
        cx.list.fill_rect(Rect::new(0.0, 0.0, width, lh), Brush::Solid(dbg));
    }
    let empty: Vec<Cell> = Vec::new();
    let line = v.line.as_deref();
    let cells = line.map_or(&empty, |l| &l.cells);
    let cols_fit = ((width / cw).floor() as usize).max(1);
    let n = cells.len().max(v.cursor.map_or(0, |c| c.col + 1)).min(cols_fit.max(cells.len()));
    let cell_of = |col: usize| cells.get(col).copied().unwrap_or(Cell::BLANK);
    let selected = |col: usize| v.sel.is_some_and(|(a, b)| col >= a && col < b);
    let block = v.cursor.filter(|c| c.shape == CursorShape::Block && c.focused).map(|c| c.col);
    // Each cell's colors, with the selection and a focused block cursor applied.
    let colors: Vec<(Color, Color)> = (0..n)
        .map(|col| {
            let c = cell_of(col);
            let (mut fg, mut bg) = cell_colors(&c, &pal, dfg, dbg);
            if selected(col) {
                bg = pal.selection.over(bg);
            }
            let under_cursor =
                block.is_some_and(|b| b == col || (b + 1 == col && c.attrs.contains(Attrs::WIDE_SPACER)));
            if under_cursor {
                bg = pal.cursor;
                fg = pal.cursor_text;
            }
            (fg, bg)
        })
        .collect();
    // Backgrounds, as runs of one color.
    let mut col = 0;
    while col < n {
        let bg = colors.get(col).map_or(dbg, |c| c.1);
        let mut end = col + 1;
        while end < n && colors.get(end).is_some_and(|c| c.1 == bg) {
            end += 1;
        }
        if bg != dbg {
            cx.list.fill_rect(Rect::new(col as f32 * cw, 0.0, end as f32 * cw, lh), Brush::Solid(bg));
        }
        col = end;
    }
    // The selection runs on to the edge when it goes on to the next line.
    if let Some((a, b)) = v.sel
        && b == usize::MAX
    {
        let from = a.max(n) as f32 * cw;
        if from < width {
            cx.list.fill_rect(Rect::new(from, 0.0, width, lh), Brush::Solid(pal.selection.over(dbg)));
        }
    }
    // An unfocused block, and the underline and bar cursors.
    if let Some(c) = v.cursor {
        let x = c.col as f32 * cw;
        let wide = cells.get(c.col).is_some_and(|x| x.attrs.contains(Attrs::WIDE));
        let w = if wide { cw * 2.0 } else { cw };
        let t = (scale.recip()).max((cw / 8.0 * scale).round() / scale);
        match c.shape {
            CursorShape::Block if !c.focused => {
                cx.list.border(Rect::new(x, 0.0, x + w, lh), CornerRadii::ZERO, t, pal.cursor);
            }
            CursorShape::Block => {}
            CursorShape::Underline => {
                cx.list.fill_rect(Rect::new(x, lh - t * 2.0, x + w, lh), Brush::Solid(pal.cursor))
            }
            CursorShape::Bar => {
                cx.list.fill_rect(Rect::new(x, 0.0, x + t * 2.0, lh), Brush::Solid(pal.cursor))
            }
        }
    }
    // Pictures programs drew (sixel): the band of each that this row shows.
    if let Some(l) = line
        && !l.pictures.is_empty()
    {
        // Pictures no line shows any more are let go (only this cache still holds them).
        cache.pictures.retain(|_, img| std::sync::Arc::strong_count(&img.pixels) > 1);
        cx.list.push_clip(Rect::new(0.0, 0.0, width, lh), CornerRadii::ZERO);
        for piece in &l.pictures {
            let pic = &piece.picture;
            let image = cache
                .pictures
                .entry(pic.id)
                .or_insert_with(|| remedy_ui_paint::Image::new(pic.width, pic.height, pic.rgba.clone()))
                .clone();
            let (pw, ph) = (piece.cell_px.0.max(1) as f32, piece.cell_px.1.max(1) as f32);
            let (w, h) = (pic.width as f32 * cw / pw, pic.height as f32 * lh / ph);
            let x = piece.col as f32 * cw;
            let y = -(piece.band as f32) * lh;
            cx.list.push(Item::Image {
                image,
                rect: Rect::new(x, y, x + w, y + h),
                radii: CornerRadii::ZERO,
                sampling: remedy_ui_paint::Sampling::Nearest,
            });
        }
        cx.list.pop_clip();
    }
    // Glyphs, each placed in its cell.
    let thin = (look.size / 14.0 * scale).round().max(1.0) / scale;
    let mut batch: Option<GlyphRun> = None;
    for col in 0..n.min(cells.len()) {
        let c = cell_of(col);
        if c.is_empty_text() && line.and_then(|l| l.marks_at(col)).is_none() {
            continue;
        }
        let (fg, _) = colors.get(col).copied().unwrap_or((dfg, dbg));
        if c.attrs.contains(Attrs::HIDDEN) {
            continue;
        }
        let span = if c.attrs.contains(Attrs::WIDE) { 2.0 } else { 1.0 };
        let x = col as f32 * cw;
        if line.and_then(|l| l.marks_at(col)).is_none() {
            let rect = Rect::new(x, 0.0, x + cw * span, lh);
            if let Some(b) = batch.take() {
                cx.list.push(Item::Glyphs(b));
            }
            if boxdraw::draw(cx.list, c.ch, rect, fg, scale) {
                continue;
            }
        }
        let text = line.and_then(|l| l.grapheme(col)).unwrap_or_else(|| c.ch.to_string());
        let s =
            shaped(cx, look, cache, &text, c.attrs.contains(Attrs::BOLD), c.attrs.contains(Attrs::ITALIC));
        // Wider than its cells: drawn smaller to fit. With a monospace face a character
        // starts at its cell, as the face lays it out; otherwise it is centered in its cells.
        let room = cw * span;
        let k = if s.width > room && s.width > 0.0 { room / s.width } else { 1.0 };
        let dx = if look.monospace && span == 1.0 && k == 1.0 { x } else { x + (room - s.width * k) * 0.5 };
        for (font, size, glyphs) in &s.runs {
            push_glyphs(
                cx.list,
                &mut batch,
                font,
                size * k,
                fg,
                glyphs.iter().map(|(id, gx, gy)| (*id, Point::new(dx + gx * k, base + gy * k))),
            );
        }
    }
    if let Some(b) = batch.take() {
        cx.list.push(Item::Glyphs(b));
    }
    // Lines under, through and over, and links.
    for col in 0..n.min(cells.len()) {
        let c = cell_of(col);
        if c.attrs.contains(Attrs::WIDE_SPACER) || c.attrs.contains(Attrs::HIDDEN) {
            continue;
        }
        let (fg, _) = colors.get(col).copied().unwrap_or((dfg, dbg));
        let span = if c.attrs.contains(Attrs::WIDE) { 2.0 } else { 1.0 };
        let (x, w) = (col as f32 * cw, cw * span);
        let ul = if c.underline_color == crate::cell::TermColor::Default {
            fg
        } else {
            pal.resolve(c.underline_color, fg)
        };
        let under_y = (base + thin * 1.5).min(lh - thin * 3.0);
        let mut kind = c.attrs.underline();
        if c.link != 0 && kind == Underline::None {
            kind = if v.hover_link == c.link { Underline::Single } else { Underline::Dotted };
        }
        decorate(cx.list, kind, x, w, under_y, thin, ul);
        if c.attrs.contains(Attrs::STRIKE) {
            let y = base - look.size * 0.3;
            cx.list.fill_rect(Rect::new(x, y, x + w, y + thin), Brush::Solid(fg));
        }
        if c.attrs.contains(Attrs::OVERLINE) {
            cx.list.fill_rect(Rect::new(x, 0.0, x + w, thin), Brush::Solid(fg));
        }
    }
    // The input method's composition, at the cursor, underlined.
    if let Some((pcol, text)) = &v.preedit {
        let mut x = *pcol as f32 * cw;
        let start = x;
        let mut batch: Option<GlyphRun> = None;
        let mut glyph_runs: Vec<(f32, Rc<Shaped>, f32)> = Vec::new();
        for ch in text.chars() {
            let w = crate::term::char_width(ch) as f32;
            if w == 0.0 {
                continue;
            }
            let s = shaped(cx, look, cache, &ch.to_string(), false, false);
            glyph_runs.push((x, s, w * cw));
            x += w * cw;
        }
        cx.list.fill_rect(Rect::new(start, 0.0, x, lh), Brush::Solid(dbg));
        for (gx, s, room) in glyph_runs {
            let k = if s.width > room && s.width > 0.0 { room / s.width } else { 1.0 };
            let dx = gx + (room - s.width * k) * 0.5;
            for (font, size, glyphs) in &s.runs {
                push_glyphs(
                    cx.list,
                    &mut batch,
                    font,
                    size * k,
                    dfg,
                    glyphs.iter().map(|(id, ggx, ggy)| (*id, Point::new(dx + ggx * k, base + ggy * k))),
                );
            }
        }
        if let Some(b) = batch.take() {
            cx.list.push(Item::Glyphs(b));
        }
        let y = (base + thin * 1.5).min(lh - thin * 2.0);
        cx.list.fill_rect(Rect::new(start, y, x, y + thin), Brush::Solid(dfg));
    }
}
