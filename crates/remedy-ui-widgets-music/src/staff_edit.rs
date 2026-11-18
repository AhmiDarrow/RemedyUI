//! Editing notation: the changes the keyboard and pointer make, as plain functions on the
//! measures so apps can use them too. Each returns whether anything changed.

use std::rc::Rc;

use remedy_ui_input::Key;
use remedy_ui_reactive::Signal;
use remedy_ui_view::Ui;

use crate::glyphs::{Clef, NoteValue};
use crate::note_staff::{NoteStaffOptions, StaffCursor, StaffEvent, StaffMeasure, key_alter};
use crate::pitch::{Pitch, Step};

fn event_mut(ms: &mut [StaffMeasure], c: StaffCursor) -> Option<&mut StaffEvent> {
    ms.get_mut(c.measure)?.events.get_mut(c.event)
}

/// The pitch at a staff position of `clef` (0 is the bottom line), spelled as the key
/// signature `key` spells that letter.
pub fn pitch_at_position(pos: i32, clef: Clef, key: i8) -> Pitch {
    let d = clef.bottom_line() + pos;
    let step = Step::from_index(d.rem_euclid(7));
    Pitch::new(step, key_alter(step, key), d.div_euclid(7).clamp(-128, 127) as i8)
}

/// `pitch` moved `steps` letters up (or down), spelled as the key spells the new letter.
pub fn step_pitch(pitch: Pitch, steps: i32, key: i8) -> Pitch {
    let d = pitch.diatonic() + steps;
    let step = Step::from_index(d.rem_euclid(7));
    Pitch::new(step, key_alter(step, key), d.div_euclid(7).clamp(-128, 127) as i8)
}

/// `pitch` moved by semitones: up spells with sharps, down with flats, unless the key
/// prefers flats.
pub fn shift_pitch(pitch: Pitch, semitones: i32, key: i8) -> Pitch {
    Pitch::from_midi(pitch.midi() + semitones, key < 0 || (semitones < 0 && key == 0))
}

fn transpose(ms: &mut [StaffMeasure], c: StaffCursor, f: impl Fn(Pitch) -> Pitch) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    if e.pitches.is_empty() {
        return false;
    }
    e.pitches = e.pitches.iter().map(|p| f(*p)).collect();
    true
}

/// Moves every pitch of the event by letters (a step up or down the staff).
pub fn move_steps(ms: &mut [StaffMeasure], c: StaffCursor, steps: i32, key: i8) -> bool {
    transpose(ms, c, |p| step_pitch(p, steps, key))
}

/// Moves every pitch of the event by semitones.
pub fn move_semitones(ms: &mut [StaffMeasure], c: StaffCursor, semitones: i32, key: i8) -> bool {
    transpose(ms, c, |p| shift_pitch(p, semitones, key))
}

/// Moves every pitch of the event by octaves.
pub fn move_octaves(ms: &mut [StaffMeasure], c: StaffCursor, octaves: i32) -> bool {
    transpose(ms, c, |p| Pitch { octave: (i32::from(p.octave) + octaves).clamp(-128, 127) as i8, ..p })
}

/// Sharpens (`+1`) or flattens (`-1`) every pitch of the event; 0 makes them natural.
pub fn alter(ms: &mut [StaffMeasure], c: StaffCursor, delta: i8) -> bool {
    transpose(ms, c, |p| Pitch { alter: if delta == 0 { 0 } else { (p.alter + delta).clamp(-2, 2) }, ..p })
}

fn push_pitch(e: &mut StaffEvent, p: Pitch, add: bool) -> bool {
    if add {
        if e.pitches.contains(&p) {
            return false;
        }
        e.pitches.push(p);
        e.pitches.sort_by_key(|q| q.midi());
        true
    } else if e.pitches.len() == 1 && e.pitches.first() == Some(&p) {
        false
    } else {
        e.pitches = vec![p];
        true
    }
}

/// Puts the letter `step` on the event, in the octave nearest to where the note is (or to the
/// staff's middle line for a rest), spelled by the key. With `add` it joins the notes already
/// there as a chord; otherwise it replaces them. A rest becomes a note.
pub fn set_letter(
    ms: &mut [StaffMeasure],
    c: StaffCursor,
    step: Step,
    clef: Clef,
    key: i8,
    add: bool,
) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    let reference = e.pitches.first().map_or(clef.bottom_line() + 4, |p| p.diatonic());
    let nearest = (-1..=1)
        .map(|o| (reference.div_euclid(7) + o) * 7 + step.index())
        .min_by_key(|d| (d - reference).abs())
        .unwrap_or(reference);
    let p = Pitch::new(step, key_alter(step, key), nearest.div_euclid(7).clamp(-128, 127) as i8);
    push_pitch(e, p, add)
}

/// Puts the pitch at staff `position` on the event (the pointer's way in): replacing its
/// notes, or joining them with `add`. A rest becomes a note.
pub fn set_pitch_at(
    ms: &mut [StaffMeasure],
    c: StaffCursor,
    position: i32,
    clef: Clef,
    key: i8,
    add: bool,
) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    push_pitch(e, pitch_at_position(position, clef, key), add)
}

/// Turns the event into a rest of the same length.
pub fn make_rest(ms: &mut [StaffMeasure], c: StaffCursor) -> bool {
    match event_mut(ms, c) {
        Some(e) if !e.pitches.is_empty() => {
            e.pitches.clear();
            e.tie = false;
            true
        }
        _ => false,
    }
}

/// Ties the event to the next, or unties it. Rests cannot be tied.
pub fn toggle_tie(ms: &mut [StaffMeasure], c: StaffCursor) -> bool {
    match event_mut(ms, c) {
        Some(e) if !e.pitches.is_empty() => {
            e.tie = !e.tie;
            true
        }
        _ => false,
    }
}

/// Sets the event's note value.
pub fn set_value(ms: &mut [StaffMeasure], c: StaffCursor, value: NoteValue) -> bool {
    match event_mut(ms, c) {
        Some(e) if e.value != value => {
            e.value = value;
            true
        }
        _ => false,
    }
}

/// Makes the event a note value longer or shorter.
pub fn change_value(ms: &mut [StaffMeasure], c: StaffCursor, longer: bool) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    match if longer { e.value.longer() } else { e.value.shorter() } {
        Some(v) => {
            e.value = v;
            true
        }
        None => false,
    }
}

/// Cycles the event through no dot, one dot and two dots.
pub fn toggle_dot(ms: &mut [StaffMeasure], c: StaffCursor) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    e.dots = (e.dots + 1) % 3;
    true
}

/// Takes the event out of its measure; returns where the cursor goes.
pub fn remove_event(ms: &mut [StaffMeasure], c: StaffCursor) -> Option<StaffCursor> {
    let m = ms.get_mut(c.measure)?;
    if c.event >= m.events.len() {
        return None;
    }
    m.events.remove(c.event);
    Some(StaffCursor { event: c.event.min(m.events.len().saturating_sub(1)), ..c })
}

/// Adds a rest after the event, as long as it, and moves to it.
pub fn insert_rest_after(ms: &mut [StaffMeasure], c: StaffCursor) -> Option<StaffCursor> {
    let m = ms.get_mut(c.measure)?;
    let (value, dots) = m.events.get(c.event).map_or((NoteValue::Quarter, 0), |e| (e.value, e.dots));
    let at = (c.event + 1).min(m.events.len());
    m.events.insert(at, StaffEvent::rest(value).dotted(dots));
    Some(StaffCursor { event: at, ..c })
}

/// Adds a measure holding a whole rest after the cursor's measure, and moves to it.
pub fn insert_measure_after(ms: &mut Vec<StaffMeasure>, c: StaffCursor) -> Option<StaffCursor> {
    let at = (c.measure + 1).min(ms.len());
    ms.insert(at, StaffMeasure::new(vec![StaffEvent::rest(NoteValue::Whole)]));
    Some(StaffCursor { measure: at, event: 0 })
}

/// Takes the cursor's measure out, unless it is the only one.
pub fn remove_measure(ms: &mut Vec<StaffMeasure>, c: StaffCursor) -> Option<StaffCursor> {
    if ms.len() < 2 || c.measure >= ms.len() {
        return None;
    }
    ms.remove(c.measure);
    Some(StaffCursor { measure: c.measure.min(ms.len() - 1), event: 0 })
}

/// Called after an edit of a staff, with what it was.
pub type StaffEditHandler = Rc<dyn Fn(&StaffEdit)>;

/// What an edit of a staff reports.
#[derive(Clone, Debug, PartialEq)]
pub struct StaffEdit {
    /// What was done ("Move note up"), as the undo history names it.
    pub label: String,
    /// Where the cursor is after it.
    pub cursor: StaffCursor,
}

/// What an editable staff tells the app.
#[derive(Clone, Default)]
pub struct StaffEditing {
    /// Called after every edit, with what it was. The measures signal already holds the result.
    pub on_edit: Option<StaffEditHandler>,
}

/// The state an editable staff edits through.
#[derive(Clone)]
pub(crate) struct StaffEditor {
    pub(crate) ui: Ui,
    pub(crate) measures: Signal<Vec<StaffMeasure>>,
    pub(crate) cursor: Signal<StaffCursor>,
    pub(crate) options: NoteStaffOptions,
    pub(crate) on_edit: Option<StaffEditHandler>,
}

impl StaffEditor {
    /// Runs `f` on a copy of the measures; if it returns a cursor the change is made, recorded
    /// for undo and reported.
    fn apply(
        &self,
        label: &str,
        f: impl FnOnce(&mut Vec<StaffMeasure>, StaffCursor) -> Option<StaffCursor>,
    ) -> bool {
        let before = self.measures.get_untracked().unwrap_or_default();
        let c0 = self.cursor.get_untracked().unwrap_or_default();
        let mut after = before.clone();
        let Some(c1) = f(&mut after, c0) else { return false };
        let (measures, cursor) = (self.measures, self.cursor);
        measures.set(after.clone());
        cursor.set(c1);
        self.ui.record_change(
            label,
            None,
            move || {
                measures.set(before.clone());
                cursor.set(c0);
            },
            move || {
                measures.set(after.clone());
                cursor.set(c1);
            },
        );
        if let Some(cb) = &self.on_edit {
            cb(&StaffEdit { label: label.to_owned(), cursor: c1 });
        }
        true
    }

    /// Puts the pitch at staff `position` on the cursor's event.
    pub(crate) fn pitch_at(&self, position: i32, add: bool) -> bool {
        let (clef, key) = (self.options.clef, self.options.key);
        self.apply("Set note", |ms, c| set_pitch_at(ms, c, position, clef, key, add).then_some(c))
    }

    /// Handles an editing key; true if it was one.
    pub(crate) fn key(&self, e: &remedy_ui_input::KeyEvent) -> bool {
        let m = e.modifiers;
        if m.meta || e.composing {
            return false;
        }
        let (clef, key) = (self.options.clef, self.options.key);
        match &e.key {
            Key::ArrowUp | Key::ArrowDown => {
                let up = e.key == Key::ArrowUp;
                let d = if up { 1 } else { -1 };
                if m.alt {
                    return false;
                }
                if m.ctrl {
                    self.apply(
                        if up { "Move note up an octave" } else { "Move note down an octave" },
                        |ms, c| move_octaves(ms, c, d).then_some(c),
                    )
                } else if m.shift {
                    self.apply(if up { "Raise note a semitone" } else { "Lower note a semitone" }, |ms, c| {
                        move_semitones(ms, c, d, key).then_some(c)
                    })
                } else {
                    self.apply(if up { "Move note up" } else { "Move note down" }, |ms, c| {
                        move_steps(ms, c, d, key).then_some(c)
                    })
                }
            }
            Key::Delete | Key::Backspace => {
                if m.ctrl {
                    return self.apply("Delete measure", remove_measure);
                }
                if m.shift {
                    return self.apply("Delete event", |ms, c| remove_event(ms, c));
                }
                // A note becomes a rest; a rest goes away.
                let is_rest = self
                    .measures
                    .get_untracked()
                    .ok()
                    .and_then(|ms| {
                        let c = self.cursor.get_untracked().ok()?;
                        ms.get(c.measure)?.events.get(c.event).map(StaffEvent::is_rest)
                    })
                    .unwrap_or(false);
                if is_rest {
                    self.apply("Delete rest", |ms, c| remove_event(ms, c))
                } else {
                    self.apply("Make rest", |ms, c| make_rest(ms, c).then_some(c))
                }
            }
            Key::Named(n) if n == "Insert" => self.apply("Insert rest", |ms, c| insert_rest_after(ms, c)),
            Key::Enter if m.ctrl => self.apply("Insert measure", insert_measure_after),
            Key::Character(ch) if !m.ctrl && !m.alt => {
                let letter = match ch.as_str() {
                    "a" => Some(Step::A),
                    "b" => Some(Step::B),
                    "c" => Some(Step::C),
                    "d" => Some(Step::D),
                    "e" => Some(Step::E),
                    "f" => Some(Step::F),
                    "g" => Some(Step::G),
                    _ => None,
                };
                if let Some(step) = letter {
                    let add = m.shift;
                    return self.apply(if add { "Add note to chord" } else { "Set note" }, |ms, c| {
                        set_letter(ms, c, step, clef, key, add).then_some(c)
                    });
                }
                let value = match ch.as_str() {
                    "1" => Some(NoteValue::Whole),
                    "2" => Some(NoteValue::Half),
                    "3" => Some(NoteValue::Quarter),
                    "4" => Some(NoteValue::Eighth),
                    "5" => Some(NoteValue::Sixteenth),
                    "6" => Some(NoteValue::ThirtySecond),
                    _ => None,
                };
                if let Some(v) = value {
                    return self.apply("Set length", |ms, c| set_value(ms, c, v).then_some(c));
                }
                match ch.as_str() {
                    "+" | "#" => self.apply("Sharpen", |ms, c| alter(ms, c, 1).then_some(c)),
                    "-" => self.apply("Flatten", |ms, c| alter(ms, c, -1).then_some(c)),
                    "=" => self.apply("Make natural", |ms, c| alter(ms, c, 0).then_some(c)),
                    "." => self.apply("Cycle dots", |ms, c| toggle_dot(ms, c).then_some(c)),
                    "[" => self.apply("Shorten", |ms, c| change_value(ms, c, false).then_some(c)),
                    "]" => self.apply("Lengthen", |ms, c| change_value(ms, c, true).then_some(c)),
                    "r" => self.apply("Make rest", |ms, c| make_rest(ms, c).then_some(c)),
                    "t" => self.apply("Tie", |ms, c| toggle_tie(ms, c).then_some(c)),
                    _ => false,
                }
            }
            _ => false,
        }
    }
}
