//! Board lettering: LED dot matrix and split-flap text that stays real text.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_core::a11y::{Role, Semantics};
use remedy_ui_core::{CornerRadii, Rect, Size};
use remedy_ui_core::layout::Length;
use remedy_ui_engine::paint::Brush;
use remedy_ui_core::reactive::untrack;
use remedy_ui_engine::view::{El, IntoProp, PaintCx, RichText, Ui};
use remedy_ui_viz::{Clock, Moved};

use crate::common::Motion;

/// The look of a [`display_text`] board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Board {
    /// Lit letters over a grid of unlit dots, as on an LED sign or a radio's display.
    #[default]
    Led,
    /// Each letter on its own flap, as on a departures board; changed letters flip.
    SplitFlap,
}

impl Board {
    fn variant(self) -> &'static str {
        match self {
            Board::Led => "led",
            Board::SplitFlap => "split_flap",
        }
    }
}

/// When a [`display_text`] scrolls its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Marquee {
    /// Never: text longer than the board is cut off.
    Off,
    /// When the text is longer than the board.
    #[default]
    Overflow,
    /// Always, like a ticker.
    Always,
}

/// How a [`display_text`] looks and moves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DisplayOptions {
    /// The board's look.
    pub board: Board,
    /// When the text scrolls. It never scrolls under reduced motion or while unseen.
    pub marquee: Marquee,
    /// How fast it scrolls, in logical pixels a second. `None` takes ten theme units.
    pub speed: Option<f32>,
    /// Pads the text with spaces to this many characters, so a board keeps its width as its
    /// text changes (a split-flap board shows blank flaps).
    pub cells: Option<usize>,
    /// Where time comes from.
    pub clock: Clock,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        DisplayOptions {
            board: Board::Led,
            marquee: Marquee::Overflow,
            speed: None,
            cells: None,
            clock: Clock::system(),
        }
    }
}

impl DisplayOptions {
    /// Sets `board`: the board's look.
    pub fn board(mut self, value: Board) -> Self {
        self.board = value;
        self
    }

    /// Sets `marquee`: when the text scrolls.
    pub fn marquee(mut self, value: Marquee) -> Self {
        self.marquee = value;
        self
    }

    /// Sets `speed`: logical pixels a second.
    pub fn speed(mut self, value: f32) -> Self {
        self.speed = (value.is_finite() && value > 0.0).then_some(value);
        self
    }

    /// Sets `cells`: pads the text to this many characters.
    pub fn cells(mut self, value: usize) -> Self {
        self.cells = Some(value.min(4096));
        self
    }

    /// Sets `clock`: where time comes from.
    pub fn clock(mut self, value: Clock) -> Self {
        self.clock = value;
        self
    }
}

/// How long the marquee rests with the text's start showing, each time round.
const REST: Duration = Duration::from_millis(1500);

struct State {
    board: Board,
    marquee: Marquee,
    speed: f32,
    /// How far the text has scrolled toward its start side, in logical pixels.
    offset: Cell<f32>,
    rest: Cell<Duration>,
    text_w: Cell<f32>,
    box_w: Cell<f32>,
    pad: f32,
    /// The text: one text element (LED), or a row of flaps (split-flap).
    line: El,
    /// A split-flap board's flaps, one per character, each holding its character as text.
    flaps: RefCell<Vec<(El, El)>>,
    flap_w: f32,
    shown: RefCell<Vec<char>>,
    /// Character positions that changed in the last update, and how far their flip has come
    /// (1 just changed, 0 done).
    flipping: RefCell<Vec<usize>>,
    flip: Cell<f32>,
    flip_time: Duration,
}

impl State {
    fn overflows(&self) -> bool {
        self.text_w.get() > self.box_w.get() + 0.5
    }

    fn scrolls(&self) -> bool {
        match self.marquee {
            Marquee::Off => false,
            Marquee::Overflow => self.overflows(),
            Marquee::Always => self.text_w.get() > 0.0,
        }
    }

    fn place(&self) {
        let off = self.offset.get();
        let _ = self.line.clone().layout(|l| l.margin.start = -off);
    }

    fn repaint_flips(&self) {
        let flaps = self.flaps.borrow();
        for &i in self.flipping.borrow().iter() {
            if let Some((cell, _)) = flaps.get(i) {
                cell.repaint();
            }
        }
    }

    /// Shows `chars` on the flaps: adds or removes flaps to match, and sets each one's letter.
    fn set_flaps(self: &Rc<Self>, chars: &[char]) {
        let ui = self.line.ui().clone();
        let mut flaps = self.flaps.borrow_mut();
        while flaps.len() > chars.len() {
            if let Some((cell, _)) = flaps.pop() {
                cell.remove();
            }
        }
        while flaps.len() < chars.len() {
            let i = flaps.len();
            let letter = ui.el("display_flap_text").nowrap().selectable(true).data_text();
            let s = self.clone();
            let cell = ui
                .el("display_flap")
                .row()
                .justify(remedy_ui_core::layout::Justify::Center)
                .width(self.flap_w)
                .layout(|l| l.shrink = 0.0)
                .paint(move |cx| flap(cx, &s, i))
                .child(letter.clone());
            let _ = self.line.clone().child(cell.clone());
            flaps.push((cell, letter));
        }
        for ((_, letter), c) in flaps.iter().zip(chars) {
            let _ = letter.clone().text(RichText::plain(c.to_string()));
        }
    }
}

/// Board lettering for `text` (a constant, signal or closure): an LED sign or a split-flap
/// departures board ([`DisplayOptions::board`]), at the size and in the colors the theme gives
/// `display_text` (and its `led` and `split_flap` variants), with same-width figures.
///
/// The text stays real text: it is read by assistive technology as one piece, can be selected
/// and copied, and follows text scaling. A split-flap board puts each character on a flap of
/// its own, all the same width, and flips the ones that change. Text longer than the board
/// scrolls past as a marquee (resting at its start each time round) when
/// [`DisplayOptions::marquee`] allows; never under reduced motion or while unseen.
#[track_caller]
pub fn display_text(ui: &Ui, text: impl IntoProp<String>, options: DisplayOptions) -> El {
    let theme = ui.theme();
    let u = theme.spacing.unit;
    let board = options.board;
    let cells = options.cells;
    let line = match board {
        Board::Led => ui.el("display_text_line").nowrap().selectable(true).data_text(),
        Board::SplitFlap => ui.el("display_flaps").row().gap(u * 0.5),
    }
    .layout(|l| l.shrink = 0.0);
    let state = Rc::new(State {
        board,
        marquee: options.marquee,
        speed: options.speed.unwrap_or(u * 10.0),
        offset: Cell::new(0.0),
        rest: Cell::new(Duration::ZERO),
        text_w: Cell::new(0.0),
        box_w: Cell::new(0.0),
        pad: u * 3.0,
        line: line.clone(),
        flaps: RefCell::new(Vec::new()),
        flap_w: (theme.type_scale.title * 0.95).round(),
        shown: RefCell::new(Vec::new()),
        flipping: RefCell::new(Vec::new()),
        flip: Cell::new(0.0),
        flip_time: theme.motion.standard,
    });
    let el = ui
        .el("display_text")
        .variant(board.variant())
        .row()
        .clip()
        .padding(remedy_ui_core::style::Pad::xy(state.pad, u * 2.0))
        .layout(|l| l.min_width = Length::Px(0.0))
        .child(line.clone());

    let s = state.clone();
    let el = el.paint(move |cx| {
        if s.board == Board::Led {
            dots(cx);
        }
    });
    let s = state.clone();
    let (el, motion) = Motion::attach(el, options.clock, move |dt| {
        let mut changed = false;
        let mut pending = false;
        // Flips settle.
        if s.flip.get() > 0.0 {
            let step = dt.as_secs_f32() / s.flip_time.as_secs_f32().max(1e-3);
            s.flip.set((s.flip.get() - step).max(0.0));
            s.repaint_flips();
            pending |= s.flip.get() > 0.0;
        }
        if s.scrolls() {
            pending = true;
            if s.offset.get() == 0.0 && s.rest.get() < REST {
                s.rest.set(s.rest.get() + dt);
            } else {
                s.rest.set(Duration::ZERO);
                let before = s.offset.get();
                let mut off = before + s.speed * dt.as_secs_f32();
                // Gone past the start side: come back in from the end side, then rest at 0.
                if off >= s.text_w.get() {
                    off = -s.box_w.get();
                }
                if before < 0.0 && off >= 0.0 {
                    off = 0.0;
                }
                s.offset.set(off);
                s.place();
                changed = true;
            }
        } else if s.offset.get() != 0.0 {
            s.offset.set(0.0);
            s.place();
            changed = true;
        }
        Moved { changed, pending }
    });
    // Measured sizes decide whether it scrolls.
    let m2 = motion.clone();
    let s = state.clone();
    let _ = line.clone().on_resize(move |size| {
        s.text_w.set(size.width);
        m2.kick();
    });
    let m3 = motion.clone();
    let s = state.clone();
    let el = el.on_resize(move |size: Size| {
        s.box_w.set((size.width - s.pad * 2.0).max(0.0));
        m3.kick();
    });

    // Under reduced motion the text rests at its start.
    let s = state.clone();
    let el = el.bind_named("reduced_motion", move || {
        let ui = s.line.ui();
        let still = ui.reduced_motion() || ui.ambient().is_none();
        if still && (s.offset.get() != 0.0 || s.flip.get() > 0.0) {
            s.offset.set(0.0);
            s.flip.set(0.0);
            s.repaint_flips();
            s.place();
        }
    });

    let s = state;
    line.prop("text", text, move |line, t: String| {
        let mut chars: Vec<char> = t.chars().collect();
        if let Some(n) = cells
            && chars.len() < n
        {
            // No-break spaces: ordinary trailing spaces hang outside the line's width.
            chars.resize(n, '\u{a0}');
        }
        let changed: Vec<usize> = {
            let old = s.shown.borrow();
            (0..chars.len()).filter(|&i| old.get(i) != chars.get(i)).collect()
        };
        let first = s.shown.borrow().is_empty();
        if board == Board::SplitFlap
            && !first
            && !changed.is_empty()
            && !untrack(|| line.ui().reduced_motion())
        {
            *s.flipping.borrow_mut() = changed;
            s.flip.set(1.0);
        }
        *s.shown.borrow_mut() = chars.clone();
        let full: String = chars.iter().collect();
        match board {
            Board::Led => {
                let _ = line.clone().text(RichText::plain(full));
            }
            Board::SplitFlap => {
                s.set_flaps(&chars);
                // Read as one piece of text, not letter by letter.
                let _ = line
                    .clone()
                    .semantics(Semantics::new(Role::Label).label(full.trim_end()).merge_descendants());
            }
        }
        s.offset.set(0.0);
        s.rest.set(Duration::ZERO);
        s.place();
        s.repaint_flips();
        motion.kick();
    });
    el
}

/// Unlit dots behind an LED board's letters.
fn dots(cx: &mut PaintCx<'_>) {
    let size = cx.style.text_size;
    let pitch = (size * 0.18).max(3.0);
    let r = pitch * 0.28;
    let dim = cx.style.foreground.with_alpha(0.10);
    let mut y = pitch / 2.0;
    while y < cx.size.height {
        let mut x = pitch / 2.0;
        while x < cx.size.width {
            cx.list.fill_rounded(
                Rect::new(x - r, y - r, x + r, y + r),
                CornerRadii::all(r),
                Brush::Solid(dim),
            );
            x += pitch;
        }
        y += pitch;
    }
}

/// One split flap: its panel, the seam across its middle, and while it flips, the top half
/// falling over the new letter.
fn flap(cx: &mut PaintCx<'_>, s: &State, i: usize) {
    let panel = cx.style.background;
    let full = Rect::new(0.0, 0.0, cx.size.width, cx.size.height);
    let mid = cx.size.height / 2.0;
    let r = cx.corners();
    cx.list.fill_rounded(full, r, Brush::Solid(panel));
    let f = s.flip.get();
    if f > 0.0 && s.flipping.borrow().contains(&i) {
        let h = mid * f;
        let shade = panel.mix_oklab(remedy_ui_core::Color::BLACK, 0.3);
        cx.list.fill_rect(Rect::new(0.0, mid - h, cx.size.width, mid), Brush::Solid(shade));
    }
    let seam = panel.mix_oklab(remedy_ui_core::Color::BLACK, 0.5);
    cx.list.fill_rect(Rect::new(0.0, mid - 0.5, cx.size.width, mid + 0.5), Brush::Solid(seam));
}
