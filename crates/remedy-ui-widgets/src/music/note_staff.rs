//! The note staff: music in standard notation with clefs, key signatures, notes by pitch with
//! ledger lines and accidentals, durations, rests, beams and ties.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use remedy_ui_core::a11y::{Action, Role, Semantics};
use remedy_ui_core::{Color, CornerRadii, Point, Rect, Vec2};
use remedy_ui_core::input::{Buttons, Key, PointerPhase};
use remedy_ui_core::layout::Length;
use remedy_ui_engine::paint::Brush;
use remedy_ui_core::reactive::Signal;
use remedy_ui_engine::view::{El, TextPaint, Ui};

use crate::music::glyphs::{
    Arc, Clef, NoteValue, Staff, accidental_width, clef_width, draw_accidental, draw_arc, draw_beam,
    draw_clef, draw_dot, draw_flags, draw_head, draw_rest, draw_stem, head_half_width, line, stem_x,
};
use crate::music::layout::{beam_runs, break_systems, ends, step_event, step_measure, stretch};
use crate::music::pitch::{Pitch, Step};
use crate::music::staff_edit::{StaffEditing, StaffEditor};

/// One thing that happens in a measure: a note, a chord or a rest.
#[derive(Clone, Debug, PartialEq)]
pub struct StaffEvent {
    /// The notes sounding together; empty for a rest.
    pub pitches: Vec<Pitch>,
    /// The note value.
    pub value: NoteValue,
    /// Dots after the note (each adds half the length so far).
    pub dots: u8,
    /// Tied to the next event: same pitches, held on.
    pub tie: bool,
}

impl StaffEvent {
    /// A single note.
    pub fn note(pitch: Pitch, value: NoteValue) -> StaffEvent {
        StaffEvent { pitches: vec![pitch], value, dots: 0, tie: false }
    }

    /// Notes played together.
    pub fn chord(pitches: Vec<Pitch>, value: NoteValue) -> StaffEvent {
        StaffEvent { pitches, value, dots: 0, tie: false }
    }

    /// A rest.
    pub fn rest(value: NoteValue) -> StaffEvent {
        StaffEvent { pitches: Vec::new(), value, dots: 0, tie: false }
    }

    /// The same event with `dots` dots.
    pub fn dotted(mut self, dots: u8) -> StaffEvent {
        self.dots = dots;
        self
    }

    /// The same event tied to the next.
    pub fn tied(mut self) -> StaffEvent {
        self.tie = true;
        self
    }

    /// Whether it is a rest.
    pub fn is_rest(&self) -> bool {
        self.pitches.is_empty()
    }

    /// Its length in quarter notes.
    pub fn quarters(&self) -> f32 {
        self.value.quarters(self.dots)
    }
}

/// A measure of events. A time signature here changes the meter from this measure on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StaffMeasure {
    /// What happens, in order.
    pub events: Vec<StaffEvent>,
    /// A new time signature (beats, beat note) starting here.
    pub time: Option<(u8, u8)>,
}

impl StaffMeasure {
    /// A measure of `events`.
    pub fn new(events: Vec<StaffEvent>) -> StaffMeasure {
        StaffMeasure { events, time: None }
    }
}

/// An event picked out by measure and event number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StaffCursor {
    /// The measure.
    pub measure: usize,
    /// The event in it.
    pub event: usize,
}

/// How a [`note_staff`] looks.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NoteStaffOptions {
    /// The clef.
    pub clef: Clef,
    /// The key signature: the number of sharps (positive) or flats (negative), -7 to 7.
    pub key: i8,
    /// The time signature the piece starts in.
    pub time: Option<(u8, u8)>,
    /// The distance between staff lines. `None` takes two and a half theme units.
    pub space: Option<f32>,
}

impl NoteStaffOptions {
    /// Sets `clef`: the clef.
    pub fn clef(mut self, value: Clef) -> Self {
        self.clef = value;
        self
    }

    /// Sets `key`: the key signature: the number of sharps (positive) or flats (negative), -7 to 7.
    pub fn key(mut self, value: i8) -> Self {
        self.key = value;
        self
    }

    /// Sets `time`: the time signature the piece starts in.
    pub fn time(mut self, value: (u8, u8)) -> Self {
        self.time = Some(value);
        self
    }

    /// Sets `space`: the distance between staff lines.
    pub fn space(mut self, value: f32) -> Self {
        self.space = Some(value);
        self
    }
}

impl Default for NoteStaffOptions {
    fn default() -> Self {
        NoteStaffOptions { clef: Clef::Treble, key: 0, time: Some((4, 4)), space: None }
    }
}

/// The position of a pitch on the staff of `clef`: 0 is the bottom line, 1 the space above it,
/// 8 the top line; below 0 and above 8 are the ledger lines.
pub fn staff_position(pitch: Pitch, clef: Clef) -> i32 {
    pitch.diatonic() - clef.bottom_line()
}

/// How the key signature `key` (sharps positive, flats negative) alters a note letter.
pub fn key_alter(step: Step, key: i8) -> i8 {
    const SHARPS: [Step; 7] = [Step::F, Step::C, Step::G, Step::D, Step::A, Step::E, Step::B];
    const FLATS: [Step; 7] = [Step::B, Step::E, Step::A, Step::D, Step::G, Step::C, Step::F];
    let n = usize::from(key.unsigned_abs()).min(7);
    if key > 0 && SHARPS.iter().take(n).any(|s| *s == step) {
        1
    } else if key < 0 && FLATS.iter().take(n).any(|s| *s == step) {
        -1
    } else {
        0
    }
}

/// The staff positions of a key signature's accidentals, in the order they are written.
pub fn key_positions(key: i8, clef: Clef) -> Vec<i32> {
    const SHARPS: [i32; 7] = [8, 5, 9, 6, 3, 7, 4];
    const FLATS: [i32; 7] = [4, 7, 3, 6, 2, 5, 1];
    let table = if key >= 0 { SHARPS } else { FLATS };
    // The treble clef's bottom line is E4; the others shift the pattern by the difference.
    let shift = (4 * 7 + 2 - clef.bottom_line() + 3).rem_euclid(7) - 3;
    table
        .iter()
        .take(usize::from(key.unsigned_abs()).min(7))
        .map(|p| {
            let q = p + shift;
            if q > 9 { q - 7 } else { q }
        })
        .collect()
}

/// Which accidental each note of a measure needs, in the order notes are listed: for each
/// event, for each pitch, `Some(alter)` when a sign must be written (0 is a natural), taking
/// the key signature and signs already written earlier in the measure into account.
pub fn accidentals(measure: &StaffMeasure, key: i8) -> Vec<Vec<Option<i8>>> {
    let mut memory: HashMap<(Step, i8), i8> = HashMap::new();
    measure
        .events
        .iter()
        .map(|e| {
            e.pitches
                .iter()
                .map(|p| {
                    let now =
                        memory.get(&(p.step, p.octave)).copied().unwrap_or_else(|| key_alter(p.step, key));
                    if now == p.alter {
                        None
                    } else {
                        memory.insert((p.step, p.octave), p.alter);
                        Some(p.alter)
                    }
                })
                .collect()
        })
        .collect()
}

/// The staff positions where ledger lines are drawn for a note at `pos`: every even position
/// from the staff's edge out to the note.
pub fn ledger_positions(pos: i32) -> Vec<i32> {
    if pos < 0 {
        let low = if pos % 2 == 0 { pos } else { pos + 1 };
        (low..0).filter(|p| p % 2 == 0).collect()
    } else if pos > 8 {
        let high = if pos % 2 == 0 { pos } else { pos - 1 };
        (10..=high).filter(|p| p % 2 == 0).collect()
    } else {
        Vec::new()
    }
}

/// Whether stems point up: they do for notes below the middle line, and for chords by the
/// note farthest from it.
pub fn stem_up(positions: &[i32]) -> bool {
    let low = positions.iter().copied().min().unwrap_or(4);
    let high = positions.iter().copied().max().unwrap_or(4);
    (4 - low) >= (high - 4)
}

/// The length of a beat in quarter notes for a time signature: a quarter, or three eighths in
/// compound meters such as 6/8.
pub fn beat_quarters(time: (u8, u8)) -> f32 {
    let (num, den) = time;
    if den >= 8 && num >= 6 && num % 3 == 0 { 1.5 } else { 4.0 / f32::from(den.max(1)) }
}

/// Groups events that are joined by beams: runs of notes with flags that fall in the same
/// beat. Returns the event ranges, each at least two events long.
pub fn beam_groups(events: &[StaffEvent], time: (u8, u8)) -> Vec<Range<usize>> {
    let items: Vec<(f32, u8, bool)> =
        events.iter().map(|e| (e.quarters(), e.value.flags(), e.is_rest())).collect();
    beam_runs(&items, beat_quarters(time))
}

/// Where a beam runs over a group of stems.
struct BeamLine {
    group: Range<usize>,
    up: bool,
    /// The y of the beam's outer edge at `x0`.
    y0: f32,
    /// The change in y for each unit of x.
    slope: f32,
    x0: f32,
}

/// The longest a stem may be shorter than the usual: stems reach at least this far (staff
/// spaces) past their outermost head.
const STEM_SPACES: f32 = 3.5;

/// The beam over a group of stems. `points` are each stem's x and the y of its outermost head
/// (the top one for stems up, the bottom for down; y grows downward), `space` is a staff space
/// and `middle` the y of the middle line, which stems always reach.
///
/// The beam follows the contour from the first note to the last, at half the rise between
/// them and never more than one staff space in all (the usual slope limit); it is level when a
/// note inside the group lies beyond both ends (a peak or a valley), when the ends are level,
/// or for a lone note. It then sits as close to the heads as lets every stem keep its
/// length. Returns the beam's y at the first stem and its slope (dy per dx).
pub fn beam_line(points: &[(f32, f32)], up: bool, space: f32, middle: f32) -> (f32, f32) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return (middle, 0.0) };
    let width = last.0 - first.0;
    let dy = last.1 - first.1;
    let inner = points.get(1..points.len().saturating_sub(1)).unwrap_or(&[]);
    let (lo, hi) = (first.1.min(last.1), first.1.max(last.1));
    // Toward the beam is up the page for stems up, down for stems down.
    let peak = inner.iter().any(|p| if up { p.1 < lo } else { p.1 > hi });
    let slope =
        if points.len() < 2 || width <= 0.0 || peak { 0.0 } else { (dy * 0.5).clamp(-space, space) / width };
    let len = STEM_SPACES * space;
    let at = |x: f32| slope * (x - first.0);
    let mut y0 = if up { f32::MAX } else { f32::MIN };
    for (x, y) in points {
        let need = if up { y - len - at(*x) } else { y + len - at(*x) };
        y0 = if up { y0.min(need) } else { y0.max(need) };
    }
    // Stems reach the middle line at least.
    for (x, _) in points {
        let need = middle - at(*x);
        y0 = if up { y0.min(need) } else { y0.max(need) };
    }
    (y0, slope)
}

/// How an event is said aloud in English: "C sharp 4 quarter note", "half rest", "whole chord
/// of C 4, E 4" ([`describe_event_in`] for another language).
pub fn describe_event(e: &StaffEvent) -> String {
    describe_event_in(e, "en")
}

/// How an event is said aloud in `locale`.
pub fn describe_event_in(e: &StaffEvent, locale: &str) -> String {
    let (value, dots) = (value_id(e.value), dots_id(e.dots));
    if e.is_rest() {
        return crate::music::say(locale, "event-rest", &[("value", value.into()), ("dots", dots.into())]);
    }
    let length = crate::music::say(locale, "note-length", &[("value", value.into()), ("dots", dots.into())]);
    if e.pitches.len() == 1 {
        let pitch = e.pitches.first().map(|p| p.spoken_in(locale)).unwrap_or_default();
        let id = if e.tie { "event-note-tied" } else { "event-note" };
        crate::music::say(locale, id, &[("pitch", pitch.into()), ("length", length.into())])
    } else {
        let names: Vec<String> = e.pitches.iter().map(|p| p.spoken_in(locale)).collect();
        let pitches = crate::music::list(locale, &names);
        crate::music::say(locale, "event-chord", &[("length", length.into()), ("pitches", pitches.into())])
    }
}

/// A note value as the catalog's messages select on it.
pub(crate) fn value_id(v: NoteValue) -> &'static str {
    match v {
        NoteValue::Whole => "whole",
        NoteValue::Half => "half",
        NoteValue::Quarter => "quarter",
        NoteValue::Eighth => "eighth",
        NoteValue::Sixteenth => "sixteenth",
        NoteValue::ThirtySecond => "thirty-second",
    }
}

/// A number of dots as the catalog's messages select on it.
pub(crate) fn dots_id(dots: u8) -> &'static str {
    match dots {
        0 => "none",
        1 => "dotted",
        _ => "double-dotted",
    }
}

/// A beat ("1", "2.5") as a number for a message, so it is written the locale's way.
pub(crate) fn beat_arg(beat: String) -> remedy_ui_engine::view::strings::StrArg {
    match beat.parse::<f64>() {
        Ok(b) => b.into(),
        Err(_) => beat.into(),
    }
}

// --- layout ---

struct EventBox {
    /// Where the heads sit (the x of the head center).
    x: f32,
    left: f32,
    right: f32,
}

struct MeasureBox {
    index: usize,
    x1: f32,
    events: Vec<EventBox>,
    time_x: Option<f32>,
}

struct SystemBox {
    measures: Vec<MeasureBox>,
    /// The y of the bottom staff line.
    bottom: f32,
    top: f32,
    height: f32,
    x_start: f32,
    x_end: f32,
}

struct NoteLayout {
    systems: Vec<SystemBox>,
    height: f32,
    space: f32,
    margin: f32,
    first_time: Option<(u8, u8)>,
}

fn event_width(e: &StaffEvent, accs: &[Option<i8>], space: f32) -> (f32, f32) {
    let room = accs.iter().flatten().map(|a| accidental_width(*a) + 0.3).fold(0.0f32, f32::max) * space;
    let base = space * (2.0 + 1.8 * e.quarters().sqrt());
    let dots = if e.dots > 0 { space * 0.8 } else { 0.0 };
    (room, room + base + dots)
}

fn header_width(o: &NoteStaffOptions, space: f32) -> f32 {
    (clef_width(o.clef) + f32::from(o.key.unsigned_abs()) * 1.0 + 1.0) * space
}

fn time_width(space: f32) -> f32 {
    space * 2.4
}

fn extremes(e: &StaffEvent, clef: Clef) -> Option<(i32, i32)> {
    let ps: Vec<i32> = e.pitches.iter().map(|p| staff_position(*p, clef)).collect();
    Some((ps.iter().copied().min()?, ps.iter().copied().max()?))
}

/// The lowest and highest staff positions a system draws, counting stems.
fn system_extent(measures: &[StaffMeasure], range: Range<usize>, clef: Clef) -> (i32, i32) {
    let (mut lo, mut hi) = (0, 8);
    for m in measures.get(range).unwrap_or(&[]) {
        for e in &m.events {
            if let Some((a, b)) = extremes(e, clef) {
                lo = lo.min(a);
                hi = hi.max(b);
                if e.value.has_stem() {
                    let up = stem_up(&e.pitches.iter().map(|p| staff_position(*p, clef)).collect::<Vec<_>>());
                    // Stems are three and a half spaces: seven positions, to the middle line at least,
                    // and a slanted beam may lift them a space more.
                    if up {
                        hi = hi.max(b + 9).max(4);
                    } else {
                        lo = lo.min(a - 9).min(4);
                    }
                }
            }
        }
    }
    (lo, hi)
}

fn build(measures: &[StaffMeasure], o: &NoteStaffOptions, width: f32, space: f32) -> NoteLayout {
    let margin = space * 1.0;
    let header = header_width(o, space);
    let first_time = measures.first().and_then(|m| m.time).or(o.time);
    let first_extra = if first_time.is_some() { time_width(space) } else { 0.0 };
    // Each measure's natural widths.
    let lead = space * 1.0;
    let trail = space * 1.0;
    let natural: Vec<(f32, Vec<(f32, f32)>)> = measures
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let accs = accidentals(m, o.key);
            let evs: Vec<(f32, f32)> =
                m.events.iter().zip(&accs).map(|(e, a)| event_width(e, a, space)).collect();
            let time = if i > 0 && m.time.is_some() { time_width(space) } else { 0.0 };
            let total = time + lead + evs.iter().map(|(_, w)| *w).sum::<f32>() + trail;
            (total.max(space * 4.0), evs)
        })
        .collect();
    let widths: Vec<f32> = natural.iter().map(|(w, _)| *w).collect();
    let avail = (width - 2.0 * margin).max(space * 8.0);
    let ranges = break_systems(&widths, header, first_extra, avail);
    let mut systems = Vec::new();
    let mut y = margin;
    let n_systems = ranges.len();
    for (si, range) in ranges.into_iter().enumerate() {
        let extra = if si == 0 { first_extra } else { 0.0 };
        let used: f32 = header + extra + widths.get(range.clone()).unwrap_or(&[]).iter().sum::<f32>();
        let k = stretch(used, header + extra, avail, si + 1 == n_systems);
        let (lo, hi) = system_extent(measures, range.clone(), o.clef);
        let above = (hi - 8).max(0) as f32 * space / 2.0 + space * 1.0;
        let below = (-lo).max(0) as f32 * space / 2.0 + space * 1.0;
        let top = y;
        let bottom = top + above + 4.0 * space;
        let height = above + 4.0 * space + below;
        let x_start = margin + header + extra;
        let mut x = x_start;
        let mut boxes = Vec::new();
        for mi in range.clone() {
            let (w, evs) = natural.get(mi).cloned().unwrap_or_default();
            let time_w = if mi > 0 && measures.get(mi).is_some_and(|m| m.time.is_some()) {
                time_width(space)
            } else {
                0.0
            };
            let x0 = x;
            let x1 = x0 + w * k;
            let inner = w - time_w - lead - trail;
            let scale = if inner > 0.0 { (w * k - time_w - lead - trail) / inner } else { 1.0 };
            let mut ex = x0 + time_w * 1.0 + lead;
            let mut events = Vec::new();
            for (room, ew) in evs {
                let ew = ew * scale;
                events.push(EventBox { x: ex + room + space * 0.9, left: ex, right: ex + ew });
                ex += ew;
            }
            boxes.push(MeasureBox {
                index: mi,
                x1,
                events,
                time_x: (time_w > 0.0).then_some(x0 + space * 0.4),
            });
            x = x1;
        }
        systems.push(SystemBox { measures: boxes, bottom, top, height, x_start, x_end: x });
        y += height + space * 1.5;
    }
    let height = (y - space * 1.5 + margin).max(space * 10.0);
    NoteLayout { systems, height, space, margin, first_time }
}

/// Everything the painter needs to know about interaction.
#[derive(Clone, Copy, Default)]
struct Marks {
    cursor: Option<StaffCursor>,
    picked: Option<StaffCursor>,
}

struct Palette {
    ink: Color,
    line: Color,
    cursor_fill: Color,
    picked_fill: Color,
    focus: Color,
}

fn paint_systems(
    cx: &mut remedy_ui_engine::view::PaintCx<'_>,
    lay: &NoteLayout,
    measures: &[StaffMeasure],
    o: &NoteStaffOptions,
    marks: Marks,
    pal: &Palette,
) {
    let sp = lay.space;
    let mut time = o.time;
    let mut time_per_measure: Vec<(u8, u8)> = Vec::new();
    for m in measures {
        if m.time.is_some() {
            time = m.time;
        }
        time_per_measure.push(time.unwrap_or((4, 4)));
    }
    let time_text = |cx: &mut remedy_ui_engine::view::PaintCx<'_>, t: (u8, u8), x: f32, staff: Staff| {
        let style = TextPaint { size: sp * 2.7, weight: 700.0, color: pal.ink, mono: false };
        cx.text(&t.0.to_string(), Point::new(x, staff.y(6)), Vec2::new(0.0, 0.5), style);
        cx.text(&t.1.to_string(), Point::new(x, staff.y(2)), Vec2::new(0.0, 0.5), style);
    };
    for sys in &lay.systems {
        let staff = Staff { bottom: sys.bottom, space: sp };
        let x0 = lay.margin;
        // The five lines and the opening barline.
        for i in 0..5 {
            line(cx.list, x0, sys.x_end, staff.y(i * 2), 1.0, pal.line);
        }
        line(cx.list, x0, x0 + 1.0, (staff.y(0) + staff.y(8)) / 2.0, staff.y(0) - staff.y(8), pal.ink);
        // Clef, key signature, the first time signature.
        draw_clef(cx.list, o.clef, x0 + sp * 0.5, staff, pal.ink);
        let mut kx = x0 + (clef_width(o.clef) + 0.2) * sp;
        for p in key_positions(o.key, o.clef) {
            draw_accidental(cx.list, i8::from(o.key > 0) * 2 - 1, kx, staff.y(p), sp, pal.ink);
            kx += sp * 1.0;
        }
        if sys.measures.first().is_some_and(|m| m.index == 0)
            && let Some(t) = lay.first_time
        {
            time_text(cx, t, sys.x_start - time_width(sp) + sp * 0.4, staff);
        }
        for mb in &sys.measures {
            let Some(m) = measures.get(mb.index) else { continue };
            if let (Some(x), Some(t)) = (mb.time_x, m.time) {
                time_text(cx, t, x, staff);
            }
            let this_time = time_per_measure.get(mb.index).copied().unwrap_or((4, 4));
            let accs = accidentals(m, o.key);
            let groups = beam_groups(&m.events, this_time);
            // Interaction highlights under the notes.
            for (ei, eb) in mb.events.iter().enumerate() {
                let here = StaffCursor { measure: mb.index, event: ei };
                let col = Rect::new(eb.left, sys.top, eb.right, sys.top + sys.height);
                if marks.picked == Some(here) {
                    cx.list.fill_rounded(
                        col,
                        CornerRadii::all(cx.theme.radii.small),
                        Brush::Solid(pal.picked_fill),
                    );
                }
                if marks.cursor == Some(here) {
                    cx.list.fill_rounded(
                        col,
                        CornerRadii::all(cx.theme.radii.small),
                        Brush::Solid(pal.cursor_fill),
                    );
                    cx.list.border(col, CornerRadii::all(cx.theme.radii.small), 2.0, pal.focus);
                }
            }
            // Stem directions and beam levels first, since stems of a group reach one beam.
            let mut stem_end: Vec<Option<(bool, f32)>> = vec![None; m.events.len()];
            for (ei, e) in m.events.iter().enumerate() {
                if e.is_rest() || !e.value.has_stem() {
                    continue;
                }
                let ps: Vec<i32> = e.pitches.iter().map(|p| staff_position(*p, o.clef)).collect();
                let up = stem_up(&ps);
                let (lo, hi) = (ps.iter().copied().min().unwrap_or(4), ps.iter().copied().max().unwrap_or(4));
                let end_pos = if up { (hi + 7).max(4) } else { (lo - 7).min(4) };
                if let Some(slot) = stem_end.get_mut(ei) {
                    *slot = Some((up, staff.y(end_pos)));
                }
            }
            let mut lines: Vec<BeamLine> = Vec::new();
            for gr in &groups {
                let dirs: Vec<(bool, i32)> = gr
                    .clone()
                    .filter_map(|i| {
                        let e = m.events.get(i)?;
                        let ps: Vec<i32> = e.pitches.iter().map(|p| staff_position(*p, o.clef)).collect();
                        Some((stem_up(&ps), ps.iter().copied().sum::<i32>() / ps.len().max(1) as i32))
                    })
                    .collect();
                let mean = dirs.iter().map(|(_, p)| *p).sum::<i32>() / dirs.len().max(1) as i32;
                let up = mean < 4 || (mean == 4 && dirs.first().is_some_and(|d| d.0));
                // Each stem's outermost head and where its stem stands; the beam follows them.
                let points: Vec<(f32, f32)> = gr
                    .clone()
                    .filter_map(|i| {
                        let e = m.events.get(i)?;
                        let eb = mb.events.get(i)?;
                        let ys = e.pitches.iter().map(|p| staff.y(staff_position(*p, o.clef)));
                        let y = if up { ys.fold(f32::MAX, f32::min) } else { ys.fold(f32::MIN, f32::max) };
                        Some((stem_x(eb.x, up, sp), y))
                    })
                    .collect();
                let (y0, slope) = beam_line(&points, up, sp, staff.y(4));
                let x0 = points.first().map_or(0.0, |p| p.0);
                for (k, i) in gr.clone().enumerate() {
                    let x = points.get(k).map_or(x0, |p| p.0);
                    if let Some(s) = stem_end.get_mut(i) {
                        *s = Some((up, y0 + slope * (x - x0)));
                    }
                }
                lines.push(BeamLine { group: gr.clone(), up, y0, slope, x0 });
            }
            // The notes.
            for (ei, (e, eb)) in m.events.iter().zip(&mb.events).enumerate() {
                if e.is_rest() {
                    draw_rest(cx.list, e.value, eb.x, staff, pal.ink);
                    if e.dots > 0 {
                        draw_dot(cx.list, Point::new(eb.x + sp * 0.9, staff.y(4) - sp * 0.3), sp, pal.ink);
                    }
                    continue;
                }
                let ps: Vec<(i32, Pitch)> =
                    e.pitches.iter().map(|p| (staff_position(*p, o.clef), *p)).collect();
                let up = stem_end
                    .get(ei)
                    .copied()
                    .flatten()
                    .map_or_else(|| stem_up(&ps.iter().map(|p| p.0).collect::<Vec<_>>()), |s| s.0);
                // Heads a second apart sit on opposite sides of the stem.
                let mut order: Vec<usize> = (0..ps.len()).collect();
                order.sort_by_key(|i| ps.get(*i).map_or(0, |p| p.0));
                if !up {
                    order.reverse();
                }
                let mut shifted = vec![false; ps.len()];
                let mut last: Option<(i32, bool)> = None;
                for i in &order {
                    let pos = ps.get(*i).map_or(0, |p| p.0);
                    let sh = last.is_some_and(|(lp, ls)| (lp - pos).abs() == 1 && !ls);
                    if let Some(s) = shifted.get_mut(*i) {
                        *s = sh;
                    }
                    last = Some((pos, sh));
                }
                let hw = head_half_width(sp);
                let mut ledgers: Vec<i32> = Vec::new();
                for (i, (pos, _)) in ps.iter().enumerate() {
                    ledgers.extend(ledger_positions(*pos));
                    let off = if shifted.get(i).copied().unwrap_or(false) {
                        if up { 2.0 * hw - 1.0 } else { -2.0 * hw + 1.0 }
                    } else {
                        0.0
                    };
                    draw_head(cx.list, e.value, Point::new(eb.x + off, staff.y(*pos)), sp, pal.ink);
                }
                ledgers.sort_unstable();
                ledgers.dedup();
                for p in ledgers {
                    line(cx.list, eb.x - hw * 1.7, eb.x + hw * 1.7, staff.y(p), 1.0, pal.ink);
                }
                // Accidentals sit left of the heads, stepped apart when close together.
                let mut acc_x = eb.x - hw - sp * 0.5;
                let mut last_acc_pos: Option<i32> = None;
                for i in &order {
                    if let (Some(Some(a)), Some((pos, _))) =
                        (accs.get(ei).and_then(|v| v.get(*i)), ps.get(*i))
                    {
                        if last_acc_pos.is_some_and(|lp| (lp - pos).abs() < 6) {
                            acc_x -= sp * 1.0;
                        } else {
                            acc_x = eb.x - hw - sp * 0.5;
                        }
                        draw_accidental(cx.list, *a, acc_x, staff.y(*pos), sp, pal.ink);
                        last_acc_pos = Some(*pos);
                    }
                }
                // Dots go in the space beside a head on a line.
                if e.dots > 0 {
                    for (pos, _) in &ps {
                        let y = if pos % 2 == 0 { staff.y(*pos) - sp * 0.5 } else { staff.y(*pos) };
                        for d in 0..e.dots {
                            draw_dot(
                                cx.list,
                                Point::new(
                                    eb.x + hw
                                        + sp * (0.5 + 0.45 * f32::from(d))
                                        + if shifted.iter().any(|s| *s) && up { 2.0 * hw } else { 0.0 },
                                    y,
                                ),
                                sp,
                                pal.ink,
                            );
                        }
                    }
                }
                // Stem and flags (beams come next).
                if let Some((su, end_y)) = stem_end.get(ei).copied().flatten() {
                    let ys: Vec<f32> = ps.iter().map(|(p, _)| staff.y(*p)).collect();
                    let near = if su {
                        ys.iter().copied().fold(f32::MAX, f32::min)
                    } else {
                        ys.iter().copied().fold(f32::MIN, f32::max)
                    };
                    let far = if su {
                        ys.iter().copied().fold(f32::MIN, f32::max)
                    } else {
                        ys.iter().copied().fold(f32::MAX, f32::min)
                    };
                    let sx = stem_x(eb.x, su, sp);
                    let from = Point::new(eb.x, far);
                    draw_stem(cx.list, from, su, end_y, sp, pal.ink);
                    let beamed = groups.iter().any(|gr| gr.contains(&ei));
                    if e.value.flags() > 0 && !beamed {
                        draw_flags(cx.list, sx, end_y, su, e.value.flags(), sp, pal.ink);
                    }
                    let _ = near;
                }
            }
            // Beams.
            for gr in &groups {
                let thick = sp * 0.5;
                let evs: Vec<(usize, &EventBox, &StaffEvent)> =
                    gr.clone().filter_map(|i| Some((i, mb.events.get(i)?, m.events.get(i)?))).collect();
                let Some(line) = lines.iter().find(|l| l.group == *gr) else { continue };
                let up = line.up;
                let at = |x: f32| line.y0 + line.slope * (x - line.x0);
                let sx = |eb: &EventBox| stem_x(eb.x, up, sp);
                for level in 1..=3u8 {
                    let offset = f32::from(level - 1) * sp * 0.75;
                    let off = if up { offset } else { -offset - thick };
                    // Runs of events that have this many flags.
                    let mut i = 0;
                    while i < evs.len() {
                        let has = evs.get(i).is_some_and(|(_, _, e)| e.value.flags() >= level);
                        if !has {
                            i += 1;
                            continue;
                        }
                        let mut j = i;
                        while evs.get(j + 1).is_some_and(|(_, _, e)| e.value.flags() >= level) {
                            j += 1;
                        }
                        let (Some((_, a, _)), Some((_, b, _))) = (evs.get(i), evs.get(j)) else { break };
                        if i == j {
                            // A lone short note gets a stub toward its neighbor.
                            let toward = if i == 0 { 1.0 } else { -1.0 };
                            let x = sx(a);
                            let (xa, xb) = (x.min(x + toward * sp * 1.0), x.max(x + toward * sp * 1.0));
                            draw_beam(cx.list, xa, at(xa) + off, xb, at(xb) + off, thick, pal.ink);
                        } else {
                            draw_beam(
                                cx.list,
                                sx(a),
                                at(sx(a)) + off,
                                sx(b),
                                at(sx(b)) + off,
                                thick,
                                pal.ink,
                            );
                        }
                        i = j + 1;
                    }
                }
            }
            // Ties: an arc to the next head of the same pitch.
            for (ei, e) in m.events.iter().enumerate() {
                if !e.tie || e.is_rest() {
                    continue;
                }
                let Some(eb) = mb.events.get(ei) else { continue };
                let next = mb.events.get(ei + 1).map(|n| n.x).unwrap_or(mb.x1 - sp * 0.3);
                for p in &e.pitches {
                    let pos = staff_position(*p, o.clef);
                    let ps: Vec<i32> = e.pitches.iter().map(|p| staff_position(*p, o.clef)).collect();
                    let down = stem_up(&ps);
                    let y = staff.y(pos) + if down { sp * 0.55 } else { -sp * 0.55 };
                    let arc = Arc {
                        x0: eb.x + sp * 0.2,
                        x1: next - sp * 0.2,
                        y,
                        up: !down,
                        height: sp * 0.45,
                        width: sp * 0.16,
                    };
                    draw_arc(cx.list, arc, pal.ink);
                }
            }
            // The closing barline: thin, and thick-thin at the very end.
            let last = mb.index + 1 == measures.len();
            let bx = mb.x1;
            line(
                cx.list,
                bx - 0.5,
                bx + 0.5,
                (staff.y(0) + staff.y(8)) / 2.0,
                staff.y(0) - staff.y(8),
                pal.ink,
            );
            if last {
                cx.list
                    .fill_rect(Rect::new(bx + 2.0, staff.y(8), bx + 5.0, staff.y(0)), Brush::Solid(pal.ink));
            }
        }
    }
}

fn palette(cx: &remedy_ui_engine::view::PaintCx<'_>) -> Palette {
    let d = cx.theme.derive();
    let c = &cx.theme.colors;
    Palette {
        ink: cx.style.foreground,
        line: cx.style.foreground.with_alpha(0.6),
        cursor_fill: d.hover,
        picked_fill: c.accent.with_alpha(0.28),
        focus: d.focus,
    }
}

/// A note staff for the measures in `measures`: the clef, key signature and time signature,
/// notes placed by pitch with ledger lines and the accidentals each note needs (counting the
/// key and the signs already written in the measure), note values with beams, dots, ties and
/// rests, wrapped onto lines to the width given. `selected` is the event the user picked.
///
/// The staff takes the keyboard as one stop: Left and Right move the cursor from event to
/// event across measures, Page Up and Page Down to the measure before and after, Home and
/// End to the ends of the measure (with Ctrl, of the piece), Enter or Space picks, Escape
/// clears. Screen readers hear the measure, the beat and the event ("C sharp 4 quarter
/// note"). Notation is never mirrored in right-to-left windows: it reads left to right
/// everywhere.
#[track_caller]
pub fn note_staff(
    ui: &Ui,
    label: impl Into<String>,
    measures: Signal<Vec<StaffMeasure>>,
    selected: Signal<Option<StaffCursor>>,
    options: NoteStaffOptions,
) -> El {
    note_staff_with(ui, label.into(), measures, selected, options, None)
}

/// A note staff people can edit: everything [`note_staff`] does, plus keys and the pointer
/// change the measures signal (which stays the app's to own) and each change is recorded in
/// the window's undo history (`Ui::record_change`, so Ctrl+Z and Ctrl+Y take it back and
/// make it again) and reported to `editing.on_edit`.
///
/// Keys: Up and Down move the note a step (Shift a semitone, Ctrl an octave), the letters
/// A to G set the note (with Shift they add it to a chord), `+` and `-` sharpen and flatten
/// and `=` makes natural, the digits 1 to 6 set the length (whole to thirty-second), `.` cycles
/// dots, `[` and `]` shorten and lengthen, `R` makes a rest, `T` ties to the next, Delete turns
/// a note into a rest (and removes a rest), Shift+Delete removes the event, Insert adds a
/// rest after it, Ctrl+Enter adds a measure and Ctrl+Delete removes one. Double-clicking at a
/// height on the staff puts the note at that pitch. Up and Down are also the screen reader's
/// increment and decrement.
#[track_caller]
pub fn note_staff_editor(
    ui: &Ui,
    label: impl Into<String>,
    measures: Signal<Vec<StaffMeasure>>,
    selected: Signal<Option<StaffCursor>>,
    options: NoteStaffOptions,
    editing: StaffEditing,
) -> El {
    note_staff_with(ui, label.into(), measures, selected, options, Some(editing))
}

fn note_staff_with(
    ui: &Ui,
    label: String,
    measures: Signal<Vec<StaffMeasure>>,
    selected: Signal<Option<StaffCursor>>,
    options: NoteStaffOptions,
    editing: Option<StaffEditing>,
) -> El {
    let unit = ui.theme().spacing.unit;
    let space = options.space.unwrap_or(unit * 2.5).max(4.0);
    let cursor = Signal::new(selected.get_untracked().ok().flatten().unwrap_or_default());
    let focused = Signal::new(false);
    let width = Rc::new(std::cell::Cell::new(0.0f32));

    let paint_width = width.clone();
    let el = ui.el("note_staff").fill_width().height(space * 12.0).paint(move |cx| {
        let ms = measures.get();
        let lay = build(&ms, &options, cx.size.width.max(paint_width.get()), space);
        let pal = palette(cx);
        let marks = Marks { cursor: focused.get().then(|| cursor.get()), picked: selected.get() };
        paint_systems(cx, &lay, &ms, &options, marks, &pal);
    });

    // The height follows the number of lines.
    let size_el = el.clone();
    let size_w = width.clone();
    let el = el.on_resize(move |size| {
        size_w.set(size.width);
        let ms = measures.get_untracked().unwrap_or_default();
        let h = build(&ms, &options, size.width, space).height;
        let _ = size_el.clone().layout(|l| l.height = Length::Px(h));
    });
    let bind_el = el.clone();
    let bind_w = width.clone();
    let el = el.bind(move || {
        let ms = measures.get();
        let w = bind_w.get();
        if w > 0.0 {
            let h = build(&ms, &options, w, space).height;
            let _ = bind_el.clone().layout(|l| l.height = Length::Px(h));
        }
        bind_el.repaint();
    });

    let hit = {
        let (pel, pw) = (el.clone(), width.clone());
        move |p: Point| -> Option<(StaffCursor, i32)> {
            let b = pel.bounds()?;
            let ms = measures.get_untracked().ok()?;
            let lay = build(&ms, &options, pw.get().max(b.width()), space);
            let (x, y) = (p.x - b.min.x, p.y - b.min.y);
            let sys =
                lay.systems.iter().find(|s| y >= s.top - space && y <= s.top + s.height + space).or_else(
                    || {
                        lay.systems.iter().min_by(|a, b| {
                            (a.top + a.height / 2.0 - y).abs().total_cmp(&(b.top + b.height / 2.0 - y).abs())
                        })
                    },
                )?;
            let mut best: Option<(StaffCursor, f32)> = None;
            for mb in &sys.measures {
                for (ei, eb) in mb.events.iter().enumerate() {
                    let d = (eb.x - x).abs();
                    if best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((StaffCursor { measure: mb.index, event: ei }, d));
                    }
                }
            }
            // The staff position under the pointer: 0 is the bottom line, 1 the next space.
            let position = ((sys.bottom - y) / (space / 2.0)).round() as i32;
            best.map(|(c, _)| (c, position))
        }
    };
    let editor =
        editing.map(|e| StaffEditor { ui: el.ui().clone(), measures, cursor, options, on_edit: e.on_edit });
    let editable = editor.is_some();
    let key_editor = editor.clone();
    let pointer_editor = editor;
    let counts = move || -> Vec<usize> {
        measures.get_untracked().map(|ms| ms.iter().map(|m| m.events.len()).collect()).unwrap_or_default()
    };
    let down = Rc::new(std::cell::Cell::new(false));
    let (sem_label, u) = (label.clone(), el.ui().clone());
    el.focusable(true)
        .on_focus_change(move |f| focused.set_if_changed(f))
        .on_pointer(move |e| {
            match e.phase {
                PointerPhase::Down if e.buttons.contains(Buttons::PRIMARY) => down.set(true),
                PointerPhase::Move if down.get() => {}
                PointerPhase::Up | PointerPhase::Cancel => return down.replace(false),
                _ => return false,
            }
            if let Some((c, position)) = hit(e.position) {
                selected.set_if_changed(Some(c));
                cursor.set_if_changed(c);
                // A double-click puts the note at the height clicked.
                if let Some(ed) = &pointer_editor
                    && e.phase == PointerPhase::Down
                    && ed.ui.click_count() == 2
                {
                    ed.pitch_at(position, e.modifiers.shift);
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
            let cur = cursor.get_untracked().unwrap_or_default();
            let at = (cur.measure, cur.event);
            let next = match e.key {
                Key::ArrowRight => step_event(&cs, at, true),
                Key::ArrowLeft => step_event(&cs, at, false),
                Key::PageDown => step_measure(&cs, at, true),
                Key::PageUp => step_measure(&cs, at, false),
                Key::Home if e.modifiers.ctrl => ends(&cs).map(|(f, _)| f),
                Key::End if e.modifiers.ctrl => ends(&cs).map(|(_, l)| l),
                Key::Home => Some((cur.measure, 0)),
                Key::End => Some((cur.measure, cs.get(cur.measure).copied().unwrap_or(1).saturating_sub(1))),
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
            if let Some((m, ev)) = next {
                cursor.set_if_changed(StaffCursor { measure: m, event: ev });
            }
            true
        })
        .semantics(move || {
            let ms = measures.get();
            let cur = cursor.get();
            let picked = selected.get();
            let locale = u.locale();
            let key = match options.key {
                0 => crate::music::say(&locale, "key-no-accidentals", &[]),
                k if k > 0 => crate::music::say(&locale, "key-sharps", &[("count", i32::from(k).into())]),
                k => crate::music::say(&locale, "key-flats", &[("count", u32::from(k.unsigned_abs()).into())]),
            };
            let mut value = String::new();
            if let Some(e) = ms.get(cur.measure).and_then(|m| m.events.get(cur.event)) {
                let beat = beat_of(&ms, options.time, cur);
                value = crate::music::say(
                    &locale,
                    "staff-cursor",
                    &[
                        ("measure", (cur.measure + 1).into()),
                        ("beat", beat_arg(beat)),
                        ("event", describe_event_in(e, &locale).into()),
                    ],
                );
                if picked == Some(cur) {
                    value = crate::music::say(&locale, "cursor-picked", &[("what", value.into())]);
                }
            }
            let clef = match options.clef {
                Clef::Bass => "bass",
                Clef::Alto => "alto",
                Clef::Tenor => "tenor",
                _ => "treble",
            };
            let mut args = vec![("clef", clef.into()), ("key", key.into()), ("measures", ms.len().into())];
            let summary = match options.time {
                Some((beats, unit)) => {
                    args.push(("beats", beats.into()));
                    args.push(("unit", unit.into()));
                    crate::music::say(&locale, "staff-summary-time", &args)
                }
                None => crate::music::say(&locale, "staff-summary", &args),
            };
            let mut said = summary.clone();
            let mut sem =
                Semantics::new(Role::Grid).label(sem_label.clone()).value(value).action(Action::Focus);
            if editable {
                said = crate::music::say(&locale, "staff-editable", &[("summary", summary.into())]);
                sem = sem.action(Action::Increment).action(Action::Decrement);
            }
            sem.description(said)
        })
}

/// The beat an event starts on, as text ("1", "2.5").
pub fn beat_of(measures: &[StaffMeasure], start_time: Option<(u8, u8)>, at: StaffCursor) -> String {
    let mut time = start_time;
    for m in measures.iter().take(at.measure + 1) {
        if m.time.is_some() {
            time = m.time;
        }
    }
    let beat = beat_quarters(time.unwrap_or((4, 4)));
    let before: f32 = measures
        .get(at.measure)
        .map(|m| m.events.iter().take(at.event).map(StaffEvent::quarters).sum())
        .unwrap_or(0.0);
    let b = before / beat + 1.0;
    let s = format!("{b:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}
