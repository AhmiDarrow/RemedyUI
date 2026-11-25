//! The tab staff: tablature for any number of strings, with measures, rhythm and techniques.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_core::input::{Buttons, Key, PointerPhase};
use remedy_ui_core::layout::Length;
use remedy_ui_engine::paint::{Brush, Item, Path, Stroke, StrokeCap, StrokeJoin};
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, PaintCx, TextPaint, Ui};

use crate::glyphs::{Arc, NoteValue, Staff, draw_arc, draw_beam, draw_dot, draw_flags, draw_rest, line};
use crate::layout::{beam_runs, break_systems, ends, step_event, step_measure, stretch};
use crate::note_staff::beat_quarters;
use crate::pitch::{GUITAR, Pitch};
use crate::tab_edit::{TabEditing, TabEditor};

/// A way of playing a note, marked on the tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Technique {
    /// Plain.
    #[default]
    None,
    /// Hammer-on to the next note on this string (an arc and "h").
    HammerOn,
    /// Pull-off to the next note on this string (an arc and "p").
    PullOff,
    /// Slide up to the next note on this string (a rising line).
    SlideUp,
    /// Slide down to the next note on this string (a falling line).
    SlideDown,
    /// Bend up by this many half steps (2 is a whole step).
    Bend(u8),
    /// Vibrato (a wavy line).
    Vibrato,
    /// Natural harmonic (the fret in angle brackets).
    Harmonic,
    /// Palm muting (P.M. with a dashed line to the next note).
    PalmMute,
    /// A dead, muted note (an x in place of the fret).
    Dead,
    /// Tied to the next note on this string (an arc).
    Tie,
}

impl Technique {
    /// How it is said aloud in English ([`Technique::spoken_in`] for another language).
    pub fn spoken(self) -> String {
        self.spoken_in("en")
    }

    /// How it is said aloud in `locale` (empty for a plain note).
    pub fn spoken_in(self, locale: &str) -> String {
        let id = match self {
            Technique::None => return String::new(),
            Technique::HammerOn => "hammer-on",
            Technique::PullOff => "pull-off",
            Technique::SlideUp => "slide-up",
            Technique::SlideDown => "slide-down",
            Technique::Bend(1) => "bend-half",
            Technique::Bend(2) => "bend-whole",
            Technique::Bend(n) => {
                return crate::say(locale, "technique-bend", &[("steps", (f64::from(n) / 2.0).into())]);
            }
            Technique::Vibrato => "vibrato",
            Technique::Harmonic => "harmonic",
            Technique::PalmMute => "palm-mute",
            Technique::Dead => "dead",
            Technique::Tie => "tied",
        };
        crate::say(locale, "technique", &[("technique", id.into())])
    }

    /// Whether the technique runs on to the next note on the same string.
    pub fn connects(self) -> bool {
        matches!(
            self,
            Technique::HammerOn
                | Technique::PullOff
                | Technique::SlideUp
                | Technique::SlideDown
                | Technique::Tie
        )
    }
}

/// The amount of a bend as written: "1/2", "full", "1 1/2", "2".
pub fn bend_text(half_steps: u8) -> String {
    match half_steps {
        0 => String::new(),
        1 => "1/2".into(),
        2 => "full".into(),
        n if n % 2 == 0 => (n / 2).to_string(),
        n => format!("{} 1/2", n / 2),
    }
}

/// One note on a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabNote {
    /// The string, counted from the lowest one (0 is the bottom line of the tab).
    pub string: usize,
    /// The fret; 0 is the open string.
    pub fret: u32,
    /// How it is played.
    pub technique: Technique,
}

impl TabNote {
    /// A plain note.
    pub fn new(string: usize, fret: u32) -> TabNote {
        TabNote { string, fret, technique: Technique::None }
    }

    /// The same note played with `technique`.
    pub fn with(mut self, technique: Technique) -> TabNote {
        self.technique = technique;
        self
    }
}

/// Notes that sound together, or a rest.
#[derive(Clone, Debug, PartialEq)]
pub struct TabEvent {
    /// The notes; empty for a rest.
    pub notes: Vec<TabNote>,
    /// How long, as a note value.
    pub value: NoteValue,
    /// Dots.
    pub dots: u8,
    /// Its length in quarter notes when that is not a note value with dots (a transcription
    /// in free time, a tuplet): it takes the place of `value` and `dots` for timing and
    /// spacing, and the event is drawn without a rhythm stem.
    pub length: Option<f32>,
}

impl TabEvent {
    /// An event of `notes`.
    pub fn new(notes: Vec<TabNote>, value: NoteValue) -> TabEvent {
        TabEvent { notes, value, dots: 0, length: None }
    }

    /// A rest.
    pub fn rest(value: NoteValue) -> TabEvent {
        TabEvent { notes: Vec::new(), value, dots: 0, length: None }
    }

    /// An event `quarters` quarter notes long, whatever that is (0.05, 1.37): notes that
    /// sound together, or with no notes a silence of that length. Events follow one another,
    /// so a note's start is the lengths before it.
    pub fn free(notes: Vec<TabNote>, quarters: f32) -> TabEvent {
        let length = if quarters.is_finite() { quarters.max(0.0) } else { 0.0 };
        TabEvent { notes, value: NoteValue::Quarter, dots: 0, length: Some(length) }
    }

    /// The same event with dots.
    pub fn dotted(mut self, dots: u8) -> TabEvent {
        self.dots = dots;
        self
    }

    /// Whether it is a rest.
    pub fn is_rest(&self) -> bool {
        self.notes.is_empty()
    }

    /// Length in quarter notes.
    pub fn quarters(&self) -> f32 {
        self.length.unwrap_or_else(|| self.value.quarters(self.dots))
    }
}

/// A measure of tab. A time signature here changes the meter from this measure on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabMeasure {
    /// The events in order.
    pub events: Vec<TabEvent>,
    /// A new time signature (beats, beat note).
    pub time: Option<(u8, u8)>,
}

impl TabMeasure {
    /// A measure of `events`.
    pub fn new(events: Vec<TabEvent>) -> TabMeasure {
        TabMeasure { events, time: None }
    }
}

/// A place in the tab: a measure, an event and a string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TabCursor {
    /// The measure.
    pub measure: usize,
    /// The event in it.
    pub event: usize,
    /// The string (0 is the lowest).
    pub string: usize,
}

/// How a tab's measures are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TabFlow {
    /// Lines (systems) that wrap to the width, one under the other.
    #[default]
    Wrapped,
    /// One line as long as the piece: the tab is as wide as its measures, for a container
    /// that scrolls sideways (`El::scroll().scroll_x()`).
    Row,
}

/// How one note is marked ([`TabDecor::notes`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoteMark {
    /// Its number's colour in place of the text colour (a low-confidence note, a wrong one).
    pub color: Option<Color>,
    /// Draw it faint.
    pub dim: bool,
    /// What a tooltip says when the pointer rests on it; empty for none.
    pub tip: String,
}

/// What an app marks on a tab over its notes ([`TabView::decor`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabDecor {
    /// Marks on single notes, by measure, event and string.
    pub notes: HashMap<TabCursor, NoteMark>,
    /// The columns in play (events counted from 0 through the piece): the notes of every
    /// other column are drawn faint. `None`: all of them.
    pub active: Option<Range<usize>>,
}

/// What a tab shows and follows beyond its notes ([`tab_staff_view`]).
#[derive(Clone, Copy, Default)]
#[non_exhaustive]
pub struct TabView {
    /// The play position in quarter notes from the start of the piece (an audio clock's
    /// beat): the column sounding then is lit from top to bottom. `None` inside: stopped.
    pub playhead: Option<Signal<Option<f32>>>,
    /// Scroll the lit column into view as the playhead moves.
    pub follow: bool,
    /// Colours, faint notes and tooltips.
    pub decor: Option<Signal<TabDecor>>,
}

impl TabView {
    /// Lights the column at `playhead`.
    pub fn playhead(mut self, playhead: Signal<Option<f32>>) -> Self {
        self.playhead = Some(playhead);
        self
    }

    /// Keeps the lit column in view.
    pub fn follow(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }

    /// Marks notes as `decor` says.
    pub fn decor(mut self, decor: Signal<TabDecor>) -> Self {
        self.decor = Some(decor);
        self
    }
}

/// One column of a laid-out tab: an event and where it is ([`tab_columns`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TabColumn {
    /// Its measure.
    pub measure: usize,
    /// Its event in the measure.
    pub event: usize,
    /// When it starts, in quarter notes from the start of the piece.
    pub start: f32,
    /// How long it lasts, in quarter notes.
    pub length: f32,
    /// The column from above the top line to below the bottom one, in the tab's own
    /// coordinates.
    pub rect: Rect,
}

/// The line gap a tab uses in `ui`'s theme.
fn gap_of(ui: &Ui, options: &TabOptions) -> f32 {
    options.line_gap.unwrap_or(ui.theme().spacing.unit * 3.5).max(6.0)
}

/// The columns of `measures` as a [`tab_staff`] `width` wide lays them out, in order: for a
/// playhead drawn by the app, hit testing, or lining something else up with the events.
pub fn tab_columns(ui: &Ui, measures: &[TabMeasure], options: &TabOptions, width: f32) -> Vec<TabColumn> {
    let gap = gap_of(ui, options);
    columns(&build(measures, options, width, gap), measures, options)
}

fn columns(lay: &TabLayout, measures: &[TabMeasure], o: &TabOptions) -> Vec<TabColumn> {
    let strings = o.tuning.len().max(1);
    let mut starts = Vec::with_capacity(measures.len());
    let mut t = 0.0f32;
    for m in measures {
        starts.push(t);
        t += m.events.iter().map(TabEvent::quarters).sum::<f32>();
    }
    let mut out = Vec::new();
    for sys in &lay.systems {
        let (top, bottom) =
            (sys.top_line - lay.gap * 0.5, sys.top_line + (strings - 1) as f32 * lay.gap + lay.gap * 0.5);
        for mb in &sys.measures {
            let Some(m) = measures.get(mb.index) else { continue };
            let mut start = starts.get(mb.index).copied().unwrap_or(0.0);
            for (event, (e, col)) in m.events.iter().zip(&mb.cols).enumerate() {
                let length = e.quarters();
                out.push(TabColumn {
                    measure: mb.index,
                    event,
                    start,
                    length,
                    rect: Rect::new(col.left, top, col.right, bottom),
                });
                start += length;
            }
        }
    }
    out
}

/// The column sounding at `beat`: the last one that has started.
fn column_at(cols: &[TabColumn], beat: f32) -> Option<&TabColumn> {
    cols.iter().rev().find(|c| c.start <= beat).or(cols.first()).filter(|_| beat.is_finite())
}

/// How a [`tab_staff`] looks.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TabOptions {
    /// The open strings as MIDI notes, lowest first. The number of lines is the length.
    pub tuning: Vec<i32>,
    /// Show the open string names at the start of each line.
    pub string_names: bool,
    /// How the measures are laid out.
    pub flow: TabFlow,
    /// Number the columns (the events, counted from 1 through the piece) above the lines.
    pub column_numbers: bool,
    /// Show rhythm stems, flags and beams under the lines.
    pub rhythm: bool,
    /// The time signature the piece starts in.
    pub time: Option<(u8, u8)>,
    /// The distance between lines. `None` takes three and a half theme units.
    pub line_gap: Option<f32>,
    /// Spell black keys with flats in string names.
    pub flats: bool,
}

impl TabOptions {
    /// How the measures are laid out.
    pub fn flow(mut self, value: TabFlow) -> Self {
        self.flow = value;
        self
    }

    /// Numbers the columns above the lines.
    pub fn column_numbers(mut self, value: bool) -> Self {
        self.column_numbers = value;
        self
    }

    /// Sets `tuning`: the open strings as MIDI notes, lowest first.
    pub fn tuning(mut self, value: Vec<i32>) -> Self {
        self.tuning = value;
        self
    }

    /// Sets `string_names`: show the open string names at the start of each line.
    pub fn string_names(mut self, value: bool) -> Self {
        self.string_names = value;
        self
    }

    /// Sets `rhythm`: show rhythm stems, flags and beams under the lines.
    pub fn rhythm(mut self, value: bool) -> Self {
        self.rhythm = value;
        self
    }

    /// Sets `time`: the time signature the piece starts in.
    pub fn time(mut self, value: (u8, u8)) -> Self {
        self.time = Some(value);
        self
    }

    /// Sets `line_gap`: the distance between lines.
    pub fn line_gap(mut self, value: f32) -> Self {
        self.line_gap = Some(value);
        self
    }

    /// Sets `flats`: spell black keys with flats in string names.
    pub fn flats(mut self, value: bool) -> Self {
        self.flats = value;
        self
    }
}

impl Default for TabOptions {
    fn default() -> Self {
        TabOptions {
            tuning: GUITAR.to_vec(),
            flow: TabFlow::Wrapped,
            column_numbers: false,
            string_names: false,
            rhythm: true,
            time: Some((4, 4)),
            line_gap: None,
            flats: false,
        }
    }
}

/// How a note's number is written: the fret, `x` for dead notes, `<12>` for harmonics.
pub fn fret_text(n: &TabNote) -> String {
    match n.technique {
        Technique::Dead => "x".into(),
        Technique::Harmonic => format!("<{}>", n.fret),
        _ => n.fret.to_string(),
    }
}

/// How an event is said aloud on `string` in English: "string 3: fret 5, hammer-on", or
/// "string 3: no note" ([`describe_tab_note_in`] for another language).
pub fn describe_tab_note(e: &TabEvent, string: usize, strings: usize) -> String {
    describe_tab_note_in(e, string, strings, "en")
}

/// How an event is said aloud on `string` in `locale`.
pub fn describe_tab_note_in(e: &TabEvent, string: usize, strings: usize, locale: &str) -> String {
    use crate::note_staff::{dots_id, value_id};
    let number = ("string", strings.saturating_sub(string).into());
    if e.is_rest() {
        let args = [("value", value_id(e.value).into()), ("dots", dots_id(e.dots).into())];
        return crate::say(locale, "event-rest", &args);
    }
    match e.notes.iter().find(|n| n.string == string) {
        Some(n) => {
            let note = if n.technique == Technique::Dead {
                crate::say(locale, "tab-note-dead", &[number])
            } else if n.fret == 0 {
                crate::say(locale, "tab-note-open", &[number])
            } else {
                crate::say(locale, "tab-note-fret", &[number, ("fret", n.fret.into())])
            };
            match n.technique {
                Technique::None | Technique::Dead => note,
                t => crate::say(
                    locale,
                    "tab-note-technique",
                    &[("note", note.into()), ("technique", t.spoken_in(locale).into())],
                ),
            }
        }
        None => crate::say(locale, "tab-no-note", &[number]),
    }
}

// --- layout ---

struct Col {
    x: f32,
    left: f32,
    right: f32,
}

struct MeasureBox {
    index: usize,
    x1: f32,
    cols: Vec<Col>,
    time_x: Option<f32>,
}

struct SystemBox {
    measures: Vec<MeasureBox>,
    /// The y of the top line.
    top_line: f32,
    top: f32,
    height: f32,
    x_end: f32,
}

struct TabLayout {
    systems: Vec<SystemBox>,
    height: f32,
    /// How wide the systems are, margins included.
    width: f32,
    gap: f32,
    margin: f32,
}

fn header_w(gap: f32, names: bool) -> f32 {
    gap * if names { 4.8 } else { 3.2 }
}

fn event_w(e: &TabEvent, gap: f32) -> f32 {
    let digits = e.notes.iter().map(|n| fret_text(n).chars().count()).max().unwrap_or(1) as f32;
    let room = gap * (0.85 + 0.6 * digits);
    let dots = if e.dots > 0 { gap * 0.7 } else { 0.0 };
    (gap * (0.9 + 2.0 * e.quarters().sqrt()) + dots).max(room + gap * 0.8)
}

fn build(measures: &[TabMeasure], o: &TabOptions, width: f32, gap: f32) -> TabLayout {
    let margin = gap * 0.5;
    let strings = o.tuning.len().max(1);
    let header = header_w(gap, o.string_names);
    let first_time = measures.first().and_then(|m| m.time).or(o.time);
    let first_extra = if first_time.is_some() { gap * 2.6 } else { 0.0 };
    let lead = gap * 0.8;
    let trail = gap * 0.8;
    let natural: Vec<(f32, Vec<f32>)> = measures
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let ws: Vec<f32> = m.events.iter().map(|e| event_w(e, gap)).collect();
            let time = if i > 0 && m.time.is_some() { gap * 2.6 } else { 0.0 };
            ((time + lead + ws.iter().sum::<f32>() + trail).max(gap * 4.0), ws)
        })
        .collect();
    let widths: Vec<f32> = natural.iter().map(|(w, _)| *w).collect();
    let avail = match o.flow {
        TabFlow::Wrapped => (width - 2.0 * margin).max(gap * 10.0),
        TabFlow::Row => f32::INFINITY,
    };
    let ranges = break_systems(&widths, header, first_extra, avail);
    let n_systems = ranges.len();
    let above = gap * if o.column_numbers { 3.6 } else { 2.4 };
    let below = if o.rhythm { gap * 3.4 } else { gap * 1.0 };
    let mut y = margin;
    let mut systems = Vec::new();
    for (si, range) in ranges.into_iter().enumerate() {
        let extra = if si == 0 { first_extra } else { 0.0 };
        let used: f32 = header + extra + widths.get(range.clone()).unwrap_or(&[]).iter().sum::<f32>();
        let k = stretch(used, header + extra, avail, si + 1 == n_systems);
        let mut x = margin + header + extra;
        let mut boxes = Vec::new();
        for mi in range {
            let (w, ws) = natural.get(mi).cloned().unwrap_or_default();
            let time_w =
                if mi > 0 && measures.get(mi).is_some_and(|m| m.time.is_some()) { gap * 2.6 } else { 0.0 };
            let x1 = x + w * k;
            let inner = w - time_w - lead - trail;
            let scale = if inner > 0.0 { (w * k - time_w - lead - trail) / inner } else { 1.0 };
            let mut ex = x + time_w + lead;
            let mut cols = Vec::new();
            for ew in ws {
                let ew = ew * scale;
                cols.push(Col { x: ex + ew / 2.0, left: ex, right: ex + ew });
                ex += ew;
            }
            boxes.push(MeasureBox { index: mi, x1, cols, time_x: (time_w > 0.0).then_some(x + gap * 0.3) });
            x = x1;
        }
        let top = y;
        let top_line = top + above;
        let height = above + (strings - 1) as f32 * gap + below;
        systems.push(SystemBox { measures: boxes, top_line, top, height, x_end: x });
        y += height + gap * 0.8;
    }
    let wide = systems.iter().map(|s: &SystemBox| s.x_end).fold(0.0f32, f32::max) + margin;
    TabLayout { systems, height: (y - gap * 0.8 + margin).max(gap * 8.0), width: wide, gap, margin }
}

fn string_y(sys: &SystemBox, gap: f32, strings: usize, string: usize) -> f32 {
    sys.top_line + (strings - 1 - string.min(strings - 1)) as f32 * gap
}

#[derive(Clone, Default)]
struct Marks {
    cursor: Option<TabCursor>,
    picked: Option<TabCursor>,
    /// The column the playhead is in.
    playing: Option<(usize, usize)>,
    decor: TabDecor,
}

fn paint(cx: &mut PaintCx<'_>, lay: &TabLayout, measures: &[TabMeasure], o: &TabOptions, marks: Marks) {
    let gap = lay.gap;
    let strings = o.tuning.len().max(1);
    let d = cx.theme.derive();
    let c = cx.theme.colors.clone();
    let ink = cx.style.foreground;
    let faint = ink.with_alpha(0.6);
    let num = TextPaint { size: gap * 0.95, weight: 600.0, color: ink, mono: true };
    let small = TextPaint { size: gap * 0.75, weight: 400.0, color: d.muted, mono: false };
    let mut time = o.time;
    let mut times: Vec<(u8, u8)> = Vec::new();
    for m in measures {
        if m.time.is_some() {
            time = m.time;
        }
        times.push(time.unwrap_or((4, 4)));
    }
    let char_w = gap * 0.6;
    // The number of each measure's first column, counted through the piece.
    let mut first_col = Vec::with_capacity(measures.len());
    let mut count = 0usize;
    for m in measures {
        first_col.push(count);
        count += m.events.len();
    }
    let col_of = |measure: usize, event: usize| first_col.get(measure).copied().unwrap_or(0) + event;
    for sys in &lay.systems {
        let x0 = lay.margin;
        let ys: Vec<f32> = (0..strings).map(|s| string_y(sys, gap, strings, s)).collect();
        let y_of = |s: usize| ys.get(s).copied().unwrap_or(sys.top_line);
        let y_top = y_of(strings - 1);
        let y_bottom = y_of(0);
        // Interaction highlights under everything.
        for mb in &sys.measures {
            for (ei, col) in mb.cols.iter().enumerate() {
                let here = |m: Option<TabCursor>| m.filter(|t| t.measure == mb.index && t.event == ei);
                if marks.playing == Some((mb.index, ei)) {
                    cx.list.fill_rect(
                        Rect::new(col.left, y_top - gap * 0.5, col.right, y_bottom + gap * 0.5),
                        Brush::Solid(c.accent.with_alpha(0.28)),
                    );
                }
                if let Some(t) = here(marks.picked) {
                    let y = y_of(t.string);
                    cx.list.fill_rounded(
                        Rect::new(col.x - gap * 0.7, y - gap * 0.55, col.x + gap * 0.7, y + gap * 0.55),
                        CornerRadii::all(gap * 0.4),
                        Brush::Solid(c.accent.with_alpha(0.35)),
                    );
                }
                if let Some(t) = here(marks.cursor) {
                    let y = y_of(t.string);
                    cx.list.fill_rect(
                        Rect::new(col.left, y_top - gap * 0.5, col.right, y_bottom + gap * 0.5),
                        Brush::Solid(d.hover),
                    );
                    cx.list.border(
                        Rect::new(col.x - gap * 0.75, y - gap * 0.6, col.x + gap * 0.75, y + gap * 0.6),
                        CornerRadii::all(gap * 0.4),
                        2.0,
                        d.focus,
                    );
                }
            }
        }
        // The lines, broken where numbers sit.
        for s in 0..strings {
            let y = y_of(s);
            let mut holes: Vec<(f32, f32)> = Vec::new();
            for mb in &sys.measures {
                let Some(m) = measures.get(mb.index) else { continue };
                for (e, col) in m.events.iter().zip(&mb.cols) {
                    for n in e.notes.iter().filter(|n| n.string == s) {
                        let w = fret_text(n).chars().count() as f32 * char_w;
                        holes.push((col.x - w / 2.0 - 2.0, col.x + w / 2.0 + 2.0));
                    }
                }
            }
            holes.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut x = x0;
            for (a, b) in holes {
                if a > x {
                    line(cx.list, x, a, y, 1.0, faint);
                }
                x = x.max(b);
            }
            line(cx.list, x, sys.x_end, y, 1.0, faint);
        }
        // TAB at the start, spelled down the lines, and the string names.
        let mid = (y_top + y_bottom) / 2.0;
        let tab_x = x0 + if o.string_names { gap * 2.9 } else { gap * 1.0 };
        let tab_style = TextPaint { size: gap * 1.15, weight: 700.0, color: ink, mono: false };
        for (i, ch) in ["T", "A", "B"].iter().enumerate() {
            cx.text(
                ch,
                Point::new(tab_x, mid + (i as f32 - 1.0) * gap * 1.2),
                Vec2::new(0.5, 0.5),
                tab_style,
            );
        }
        if o.string_names {
            for s in 0..strings {
                if let Some(m) = o.tuning.get(s) {
                    cx.text(
                        &Pitch::from_midi(*m, o.flats).letter_name(),
                        Point::new(x0 + gap * 0.2, y_of(s)),
                        Vec2::new(0.0, 0.5),
                        small,
                    );
                }
            }
        }
        // The first time signature.
        if sys.measures.first().is_some_and(|m| m.index == 0)
            && let Some(t) = measures.first().and_then(|m| m.time).or(o.time)
        {
            let tx = x0 + header_w(gap, o.string_names);
            time_sig(cx, t, tx, mid, gap, ink);
        }
        for mb in &sys.measures {
            let Some(m) = measures.get(mb.index) else { continue };
            if let (Some(x), Some(t)) = (mb.time_x, m.time) {
                time_sig(cx, t, x, mid, gap, ink);
            }
            // Numbers.
            for (ei, (e, col)) in m.events.iter().zip(&mb.cols).enumerate() {
                let column = col_of(mb.index, ei);
                let outside = marks.decor.active.as_ref().is_some_and(|r| !r.contains(&column));
                if o.column_numbers {
                    let mut style = small;
                    if outside {
                        style.color = style.color.with_alpha(style.color.a * 0.4);
                    }
                    cx.text(
                        &(column + 1).to_string(),
                        Point::new(col.x, sys.top + gap * 0.6),
                        Vec2::new(0.5, 0.5),
                        style,
                    );
                }
                for n in &e.notes {
                    let mark =
                        marks.decor.notes.get(&TabCursor { measure: mb.index, event: ei, string: n.string });
                    let mut style = num;
                    if let Some(color) = mark.and_then(|m| m.color) {
                        style.color = color;
                    }
                    if outside || mark.is_some_and(|m| m.dim) {
                        style.color = style.color.with_alpha(style.color.a * 0.35);
                    }
                    cx.text(&fret_text(n), Point::new(col.x, y_of(n.string)), Vec2::new(0.5, 0.5), style);
                }
            }
            // Techniques.
            for (ei, (e, col)) in m.events.iter().zip(&mb.cols).enumerate() {
                for n in &e.notes {
                    technique(cx, &TechCtx { lay, sys, ys: &ys, measures, strings, ink, small }, mb, ei, n);
                }
                let _ = col;
            }
            // Rhythm.
            let rhythm_top = y_bottom + gap * 0.8;
            let stem_len = gap * 1.9;
            if o.rhythm {
                let this_time = times.get(mb.index).copied().unwrap_or((4, 4));
                // A free-time event breaks a beam as a rest does.
                let items: Vec<(f32, u8, bool)> = m
                    .events
                    .iter()
                    .map(|e| (e.quarters(), e.value.flags(), e.is_rest() || e.length.is_some()))
                    .collect();
                let groups = beam_runs(&items, beat_quarters(this_time));
                let glyph_staff = Staff { bottom: (y_top + y_bottom) / 2.0 + gap * 1.5, space: gap * 0.75 };
                for (ei, (e, col)) in m.events.iter().zip(&mb.cols).enumerate() {
                    // Free time has no note value to draw.
                    if e.length.is_some() {
                        continue;
                    }
                    if e.is_rest() {
                        draw_rest(cx.list, e.value, col.x, glyph_staff, ink);
                        continue;
                    }
                    if e.value.has_stem() {
                        cx.list.fill_rect(
                            Rect::new(col.x - 0.6, rhythm_top, col.x + 0.6, rhythm_top + stem_len),
                            Brush::Solid(ink),
                        );
                        if e.value.flags() > 0 && !groups.iter().any(|g: &Range<usize>| g.contains(&ei)) {
                            draw_flags(
                                cx.list,
                                col.x,
                                rhythm_top + stem_len,
                                false,
                                e.value.flags(),
                                gap,
                                ink,
                            );
                        }
                    } else {
                        // A whole note has no stem: a small circle stands for it.
                        cx.list.border(
                            Rect::new(
                                col.x - gap * 0.25,
                                rhythm_top,
                                col.x + gap * 0.25,
                                rhythm_top + gap * 0.5,
                            ),
                            CornerRadii::all(gap * 0.25),
                            1.2,
                            ink,
                        );
                    }
                    if e.dots > 0 {
                        for k in 0..e.dots {
                            draw_dot(
                                cx.list,
                                Point::new(
                                    col.x + gap * (0.5 + 0.4 * f32::from(k)),
                                    rhythm_top + stem_len * 0.7,
                                ),
                                gap,
                                ink,
                            );
                        }
                    }
                }
                for gr in &groups {
                    let thick = gap * 0.35;
                    let cols: Vec<(usize, f32)> =
                        gr.clone().filter_map(|i| Some((i, mb.cols.get(i)?.x))).collect();
                    for level in 1..=3u8 {
                        let y = rhythm_top + stem_len - f32::from(level - 1) * gap * 0.6 - thick;
                        let mut i = 0;
                        while i < cols.len() {
                            let has = |k: usize| {
                                cols.get(k)
                                    .and_then(|(ei, _)| m.events.get(*ei))
                                    .is_some_and(|e| e.value.flags() >= level)
                            };
                            if !has(i) {
                                i += 1;
                                continue;
                            }
                            let mut j = i;
                            while has(j + 1) {
                                j += 1;
                            }
                            let (xa, xb) =
                                (cols.get(i).map_or(0.0, |c| c.1), cols.get(j).map_or(0.0, |c| c.1));
                            if i == j {
                                let toward = if i == 0 { 1.0 } else { -1.0 };
                                draw_beam(
                                    cx.list,
                                    xa.min(xa + toward * gap * 0.8),
                                    y,
                                    xa.max(xa + toward * gap * 0.8),
                                    y,
                                    thick,
                                    ink,
                                );
                            } else {
                                draw_beam(cx.list, xa, y, xb, y, thick, ink);
                            }
                            i = j + 1;
                        }
                    }
                }
            }
            // The bar line.
            let last = mb.index + 1 == measures.len();
            line(cx.list, mb.x1 - 0.5, mb.x1 + 0.5, (y_top + y_bottom) / 2.0, y_bottom - y_top, ink);
            if last {
                cx.list.fill_rect(Rect::new(mb.x1 + 2.0, y_top, mb.x1 + 5.0, y_bottom), Brush::Solid(ink));
            }
        }
        // The opening bar line.
        line(cx.list, x0, x0 + 1.0, (y_top + y_bottom) / 2.0, y_bottom - y_top, ink);
    }
}

fn time_sig(cx: &mut PaintCx<'_>, t: (u8, u8), x: f32, mid: f32, gap: f32, ink: Color) {
    let style = TextPaint { size: gap * 1.5, weight: 700.0, color: ink, mono: false };
    cx.text(&t.0.to_string(), Point::new(x, mid - gap * 0.9), Vec2::new(0.0, 0.5), style);
    cx.text(&t.1.to_string(), Point::new(x, mid + gap * 0.9), Vec2::new(0.0, 0.5), style);
}

/// The next note on `string` after event `ei` of measure `mb`, within the same line.
fn next_on_string(
    measures: &[TabMeasure],
    sys: &SystemBox,
    mb: &MeasureBox,
    ei: usize,
    string: usize,
) -> Option<(f32, u32)> {
    let mut first = true;
    for other in sys.measures.iter().skip_while(|m| m.index != mb.index) {
        let m = measures.get(other.index)?;
        let from = if first { ei + 1 } else { 0 };
        first = false;
        for (k, (e, col)) in m.events.iter().zip(&other.cols).enumerate().skip(from) {
            let _ = k;
            if let Some(n) = e.notes.iter().find(|n| n.string == string) {
                return Some((col.x, n.fret));
            }
        }
    }
    None
}

/// What the technique marks need to know about the line they are on.
struct TechCtx<'a> {
    lay: &'a TabLayout,
    sys: &'a SystemBox,
    ys: &'a [f32],
    measures: &'a [TabMeasure],
    strings: usize,
    ink: Color,
    small: TextPaint,
}

fn technique(cx: &mut PaintCx<'_>, m: &TechCtx<'_>, mb: &MeasureBox, ei: usize, n: &TabNote) {
    let TechCtx { lay, sys, ys, measures, strings, ink, small } = *m;

    let gap = lay.gap;
    let Some(col) = mb.cols.get(ei) else { return };
    let y = ys.get(n.string).copied().unwrap_or(sys.top_line);
    let w = fret_text(n).chars().count() as f32 * gap * 0.6;
    let top_line = ys.get(strings - 1).copied().unwrap_or(sys.top_line);
    let end_x = next_on_string(measures, sys, mb, ei, n.string).map_or(sys.x_end - gap * 0.3, |(x, _)| x);
    let next_w = next_on_string(measures, sys, mb, ei, n.string)
        .map_or(gap * 0.6, |(_, f)| f.to_string().chars().count() as f32 * gap * 0.6);
    let from = col.x + w / 2.0 + 2.0;
    let to = (end_x - next_w / 2.0 - 2.0).max(from + gap * 0.5);
    let mut stroke = Stroke::new(1.2);
    stroke.cap = StrokeCap::Round;
    stroke.join = StrokeJoin::Round;
    let mut path = Path::new();
    match n.technique {
        Technique::HammerOn | Technique::PullOff | Technique::Tie => {
            draw_arc(
                cx.list,
                Arc { x0: from, x1: to, y: y - gap * 0.45, up: true, height: gap * 0.55, width: 1.2 },
                ink,
            );
            let letter = match n.technique {
                Technique::HammerOn => "h",
                Technique::PullOff => "p",
                _ => "",
            };
            if !letter.is_empty() {
                cx.text(letter, Point::new((from + to) / 2.0, y - gap * 1.25), Vec2::new(0.5, 1.0), small);
            }
        }
        Technique::SlideUp | Technique::SlideDown => {
            let (a, b) = if n.technique == Technique::SlideUp {
                (gap * 0.3, -gap * 0.3)
            } else {
                (-gap * 0.3, gap * 0.3)
            };
            path.move_to(Point::new(from, y + a));
            path.line_to(Point::new(to, y + b));
            cx.list.push(Item::StrokePath { path, stroke, brush: Brush::Solid(ink) });
        }
        Technique::Bend(h) if h > 0 => {
            let x = col.x + w / 2.0 + 1.0;
            let tip = Point::new(x + gap * 1.1, y - gap * 1.3);
            path.move_to(Point::new(x, y - gap * 0.2));
            path.cubic_to(
                Point::new(x + gap * 0.9, y - gap * 0.1),
                Point::new(x + gap * 1.1, y - gap * 0.6),
                tip,
            );
            path.move_to(Point::new(tip.x - gap * 0.3, tip.y + gap * 0.35));
            path.line_to(tip);
            path.line_to(Point::new(tip.x + gap * 0.3, tip.y + gap * 0.35));
            cx.list.push(Item::StrokePath { path, stroke, brush: Brush::Solid(ink) });
            cx.text(&bend_text(h), Point::new(tip.x, tip.y - gap * 0.15), Vec2::new(0.5, 1.0), small);
        }
        Technique::Vibrato => {
            let (a, b) = (col.x - gap * 0.6, end_x.min(col.x + gap * 3.0).max(col.x + gap * 1.2));
            let yy = top_line - gap * 0.9;
            path.move_to(Point::new(a, yy));
            let mut x = a;
            let mut up = false;
            while x < b {
                x += gap * 0.3;
                path.line_to(Point::new(x, yy + if up { 0.0 } else { -gap * 0.3 }));
                up = !up;
            }
            cx.list.push(Item::StrokePath { path, stroke, brush: Brush::Solid(ink) });
        }
        Technique::PalmMute => {
            let yy = top_line - gap * 1.1;
            cx.text("P.M.", Point::new(col.x - gap * 0.5, yy), Vec2::new(0.0, 0.5), small);
            let mut s = Stroke::new(1.0);
            s.dashes = vec![gap * 0.35, gap * 0.35];
            path.move_to(Point::new(col.x + gap * 1.9, yy));
            path.line_to(Point::new(end_x.max(col.x + gap * 2.2), yy));
            cx.list.push(Item::StrokePath { path, stroke: s, brush: Brush::Solid(ink) });
        }
        Technique::Harmonic | Technique::Dead | Technique::None | Technique::Bend(_) => {}
    }
}

/// A tab staff for the measures in `measures`: one line for each string of the tuning, fret
/// numbers where notes fall, the rhythm under the lines (stems, flags, beams and rests) and
/// techniques marked on the notes: hammer-ons and pull-offs with an arc and a letter, slides,
/// bends, vibrato, harmonics, palm muting, dead notes and ties. Measures wrap onto lines to the
/// width given.
///
/// `selected` is the place the user picked. The tab takes the keyboard as one stop: Left and
/// Right move the cursor between events (across measures), Up and Down between strings, Page
/// Up and Down between measures, Home and End to the ends of the measure (with Ctrl, of the
/// piece), Enter or Space picks, Escape clears. Screen readers hear the measure, the beat, the
/// string and the fret with its technique. Tab, like notation, reads left to right in every
/// window.
#[track_caller]
pub fn tab_staff(
    ui: &Ui,
    label: impl Into<String>,
    measures: Signal<Vec<TabMeasure>>,
    selected: Signal<Option<TabCursor>>,
    options: TabOptions,
) -> El {
    tab_staff_with(ui, label.into(), measures, selected, options, None, TabView::default())
}

/// A [`tab_staff`] that also shows what `view` says: a playhead that lights the column
/// sounding now (and keeps it in view), and the app's colours, faint notes and tooltips.
#[track_caller]
pub fn tab_staff_view(
    ui: &Ui,
    label: impl Into<String>,
    measures: Signal<Vec<TabMeasure>>,
    selected: Signal<Option<TabCursor>>,
    options: TabOptions,
    view: TabView,
) -> El {
    tab_staff_with(ui, label.into(), measures, selected, options, None, view)
}

/// A [`tab_staff_editor`] that also shows what `view` says, as [`tab_staff_view`] does.
#[track_caller]
pub fn tab_staff_editor_view(
    ui: &Ui,
    label: impl Into<String>,
    measures: Signal<Vec<TabMeasure>>,
    selected: Signal<Option<TabCursor>>,
    options: TabOptions,
    editing: TabEditing,
    view: TabView,
) -> El {
    tab_staff_with(ui, label.into(), measures, selected, options, Some(editing), view)
}

/// A tab people can edit: everything [`tab_staff`] does, plus keys and the pointer change the
/// measures signal (which stays the app's to own), and each change is recorded in the window's
/// undo history (`Ui::record_change`, so Ctrl+Z and Ctrl+Y take it back and make it again) and
/// reported to `editing.on_edit`.
///
/// Keys: the digits type a fret on the cursor's string (a second digit soon after makes
/// two), Delete removes the note (Shift+Delete the whole event, Ctrl+Delete the measure),
/// Alt+Up and Alt+Down raise and lower the fret, `H` `P` `/` `\` `B` `V` `X` `O` `M` `T` mark hammer-on,
/// pull-off, slides, bend, vibrato, dead note, harmonic, palm mute and tie (the same key again
/// clears it, `N` clears any), `[` and `]` shorten and lengthen, `.` cycles dots, `R` makes a rest, Insert
/// adds a rest after the event and Ctrl+Enter adds a measure. Double-clicking an empty place
/// puts an open string note there and double-clicking a note removes it.
#[track_caller]
pub fn tab_staff_editor(
    ui: &Ui,
    label: impl Into<String>,
    measures: Signal<Vec<TabMeasure>>,
    selected: Signal<Option<TabCursor>>,
    options: TabOptions,
    editing: TabEditing,
) -> El {
    tab_staff_with(ui, label.into(), measures, selected, options, Some(editing), TabView::default())
}

fn tab_staff_with(
    ui: &Ui,
    label: String,
    measures: Signal<Vec<TabMeasure>>,
    selected: Signal<Option<TabCursor>>,
    options: TabOptions,
    editing: Option<TabEditing>,
    view: TabView,
) -> El {
    let gap = gap_of(ui, &options);
    let row = options.flow == TabFlow::Row;
    let decor_of = move || view.decor.map(|d| d.get()).unwrap_or_default();
    let strings = options.tuning.len().max(1);
    let start = TabCursor { measure: 0, event: 0, string: strings - 1 };
    let cursor = Signal::new(selected.get_untracked().ok().flatten().unwrap_or(start));
    let focused = Signal::new(false);
    let width = Rc::new(Cell::new(0.0f32));
    let options = Rc::new(options);

    let (po, pw) = (options.clone(), width.clone());
    let el = ui.el("tab_staff").height(gap * 9.0);
    let el = if row { el } else { el.fill_width() };
    let el = el.paint(move |cx| {
        let ms = measures.get();
        let lay = build(&ms, &po, cx.size.width.max(pw.get()), gap);
        let playing = view
            .playhead
            .and_then(|p| p.get())
            .and_then(|beat| column_at(&columns(&lay, &ms, &po), beat).map(|c| (c.measure, c.event)));
        let marks = Marks {
            cursor: focused.get().then(|| cursor.get()),
            picked: selected.get(),
            playing,
            decor: decor_of(),
        };
        paint(cx, &lay, &ms, &po, marks);
    });

    let (so, sw, sel) = (options.clone(), width.clone(), el.clone());
    let el = el.on_resize(move |size| {
        sw.set(size.width);
        let ms = measures.get_untracked().unwrap_or_default();
        let lay = build(&ms, &so, size.width, gap);
        let _ = sel.clone().layout(|l| {
            l.height = Length::Px(lay.height);
            if row {
                l.width = Length::Px(lay.width);
            }
        });
    });
    let (bo, bw, bel) = (options.clone(), width.clone(), el.clone());
    let el = el.bind(move || {
        let ms = measures.get();
        let w = bw.get();
        if w > 0.0 || row {
            let lay = build(&ms, &bo, w, gap);
            let _ = bel.clone().layout(|l| {
                l.height = Length::Px(lay.height);
                if row {
                    l.width = Length::Px(lay.width);
                }
            });
        }
        bel.repaint();
    });
    // The playhead and the app's marks redraw the tab; a column newly lit is brought into
    // view.
    let (fo, fw, fel) = (options.clone(), width.clone(), el.clone());
    let lit = Cell::new(None);
    let el = el.bind(move || {
        let _ = view.decor.map(|d| d.with(|_| ()));
        let beat = view.playhead.and_then(|p| p.get());
        fel.repaint();
        let Some(beat) = beat else { return lit.set(None) };
        if !view.follow {
            return;
        }
        let ms = measures.get_untracked().unwrap_or_default();
        let lay = build(&ms, &fo, fw.get(), gap);
        let cols = columns(&lay, &ms, &fo);
        let Some(col) = column_at(&cols, beat) else { return };
        if lit.replace(Some((col.measure, col.event))) != Some((col.measure, col.event)) {
            // A little of what comes next shows too.
            let r = col.rect;
            fel.scroll_rect_into_view(Rect::new(
                r.min.x - gap,
                r.min.y - gap * 2.4,
                r.max.x + gap * 6.0,
                r.max.y + gap * 3.4,
            ));
        }
    });
    // A note's tooltip, where the app gave it one.
    let tips = view.decor.map(|decor| (decor, remedy_ui_widgets_core::PartTooltip::new(&el)));

    let hit = {
        let (pel, pw, po) = (el.clone(), width.clone(), options.clone());
        move |p: Point| -> Option<TabCursor> {
            let b = pel.bounds()?;
            let ms = measures.get_untracked().ok()?;
            let lay = build(&ms, &po, pw.get().max(b.width()), gap);
            let (x, y) = (p.x - b.min.x, p.y - b.min.y);
            let sys = lay.systems.iter().find(|s| y >= s.top && y <= s.top + s.height).or_else(|| {
                lay.systems.iter().min_by(|a, b| {
                    (a.top + a.height / 2.0 - y).abs().total_cmp(&(b.top + b.height / 2.0 - y).abs())
                })
            })?;
            let mut best: Option<(usize, usize, f32)> = None;
            for mb in &sys.measures {
                for (ei, col) in mb.cols.iter().enumerate() {
                    let d = (col.x - x).abs();
                    if best.is_none_or(|(_, _, bd)| d < bd) {
                        best = Some((mb.index, ei, d));
                    }
                }
            }
            let (measure, event, _) = best?;
            let row = ((y - sys.top_line) / gap).round().clamp(0.0, (strings - 1) as f32) as usize;
            Some(TabCursor { measure, event, string: strings - 1 - row })
        }
    };
    let counts = move || -> Vec<usize> {
        measures.get_untracked().map(|ms| ms.iter().map(|m| m.events.len()).collect()).unwrap_or_default()
    };
    let down = Rc::new(Cell::new(false));
    let (tip_el, tip_width, tip_options) = (el.clone(), width.clone(), options.clone());
    let editor = editing.map(|e| TabEditor::new(el.ui().clone(), measures, cursor, e.on_edit));
    let editable = editor.is_some();
    let key_editor = editor.clone();
    let pointer_editor = editor;
    let (sem_label, u) = (label.clone(), ui.clone());
    let sem_options = options.clone();
    el.focusable(true)
        .on_focus_change(move |f| focused.set_if_changed(f))
        .on_pointer(move |e| {
            if let Some((decor, tip)) = &tips {
                let part =
                    hit(e.position).filter(|_| e.phase == PointerPhase::Move && !down.get()).and_then(|c| {
                        let text = decor.get_untracked().ok()?.notes.get(&c)?.tip.clone();
                        let (b, w) = (tip_el.bounds()?, tip_width.get());
                        let ms = measures.get_untracked().ok()?;
                        let lay = build(&ms, &tip_options, w.max(b.width()), gap);
                        let col = columns(&lay, &ms, &tip_options)
                            .into_iter()
                            .find(|k| (k.measure, k.event) == (c.measure, c.event))?;
                        let sys =
                            lay.systems.iter().find(|s| s.measures.iter().any(|m| m.index == c.measure))?;
                        let y = b.min.y + string_y(sys, gap, strings, c.string);
                        let near = (e.position.y - y).abs() <= gap * 0.6
                            && (e.position.x - b.min.x - (col.rect.min.x + col.rect.max.x) / 2.0).abs()
                                <= gap;
                        near.then(|| {
                            (
                                Rect::new(
                                    b.min.x + col.rect.min.x,
                                    y - gap * 0.5,
                                    b.min.x + col.rect.max.x,
                                    y + gap * 0.5,
                                ),
                                text,
                            )
                        })
                    });
                tip.point(part);
            }
            match e.phase {
                PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => down.set(true),
                PointerPhase::Move if down.get() => {}
                PointerPhase::Up | PointerPhase::Cancel => return down.replace(false),
                _ => return false,
            }
            if let Some(c) = hit(e.position) {
                selected.set_if_changed(Some(c));
                cursor.set_if_changed(c);
                if let Some(ed) = &pointer_editor
                    && e.phase == PointerPhase::Down
                    && ed.ui.click_count() == 2
                {
                    ed.toggle_note();
                }
            }
            true
        })
        .on_key(move |e| {
            if !e.pressed {
                return false;
            }
            if let Some(ed) = &key_editor
                && ed.key(e)
            {
                return true;
            }
            let cs = counts();
            let cur = cursor.get_untracked().unwrap_or(start);
            let at = (cur.measure, cur.event);
            let moved =
                |p: Option<(usize, usize)>| p.map(|(m, ev)| TabCursor { measure: m, event: ev, ..cur });
            let next = match e.key {
                Key::ArrowRight => moved(step_event(&cs, at, true)),
                Key::ArrowLeft => moved(step_event(&cs, at, false)),
                Key::ArrowUp => Some(TabCursor { string: (cur.string + 1).min(strings - 1), ..cur }),
                Key::ArrowDown => Some(TabCursor { string: cur.string.saturating_sub(1), ..cur }),
                Key::PageDown => moved(step_measure(&cs, at, true)),
                Key::PageUp => moved(step_measure(&cs, at, false)),
                Key::Home if e.modifiers.ctrl => moved(ends(&cs).map(|(f, _)| f)),
                Key::End if e.modifiers.ctrl => moved(ends(&cs).map(|(_, l)| l)),
                Key::Home => Some(TabCursor { event: 0, ..cur }),
                Key::End => Some(TabCursor {
                    event: cs.get(cur.measure).copied().unwrap_or(1).saturating_sub(1),
                    ..cur
                }),
                Key::Enter | Key::Space => {
                    selected.set_if_changed(Some(cur));
                    return true;
                }
                Key::Escape => {
                    if selected.get_untracked().ok().flatten().is_none() {
                        return false;
                    }
                    selected.set(None);
                    return true;
                }
                _ => return false,
            };
            if let Some(n) = next {
                cursor.set_if_changed(n);
            }
            true
        })
        .semantics(move || {
            let ms = measures.get();
            let cur = cursor.get();
            let picked = selected.get();
            let locale = u.locale();
            let mut value = String::new();
            if let Some(ev) = ms.get(cur.measure).and_then(|m| m.events.get(cur.event)) {
                let beat = beat_in(&ms, sem_options.time, cur.measure, cur.event);
                value = crate::say(
                    &locale,
                    "tab-cursor",
                    &[
                        ("measure", (cur.measure + 1).into()),
                        ("beat", crate::note_staff::beat_arg(beat)),
                        ("note", describe_tab_note_in(ev, cur.string, strings, &locale).into()),
                    ],
                );
                if picked == Some(cur) {
                    value = crate::say(&locale, "cursor-picked", &[("what", value.into())]);
                }
            }
            let mut said = crate::say(
                &locale,
                "tab-summary",
                &[("strings", strings.into()), ("measures", ms.len().into())],
            );
            if editable {
                said = crate::say(&locale, "tab-editable", &[("summary", said.into())]);
            }
            Semantics::new(Role::Grid)
                .label(sem_label.clone())
                .value(value)
                .description(said)
                .action(Action::Focus)
        })
}

/// The beat an event starts on, as text ("1", "2.5").
pub fn beat_in(
    measures: &[TabMeasure],
    start_time: Option<(u8, u8)>,
    measure: usize,
    event: usize,
) -> String {
    let mut time = start_time;
    for m in measures.iter().take(measure + 1) {
        if m.time.is_some() {
            time = m.time;
        }
    }
    let beat = beat_quarters(time.unwrap_or((4, 4)));
    let before: f32 = measures
        .get(measure)
        .map(|m| m.events.iter().take(event).map(TabEvent::quarters).sum())
        .unwrap_or(0.0);
    let s = format!("{:.2}", before / beat + 1.0);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}
