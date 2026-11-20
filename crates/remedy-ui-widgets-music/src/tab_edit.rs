//! Editing tablature: the changes the keyboard and pointer make, as plain functions on the
//! measures so apps can use them too. Each returns whether anything changed.

use remedy_ui_view::words::en;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use remedy_ui_input::Key;
use remedy_ui_reactive::Signal;
use remedy_ui_view::Ui;

use crate::glyphs::NoteValue;
use crate::tab::{TabCursor, TabEvent, TabMeasure, TabNote, Technique};

fn event_mut(ms: &mut [TabMeasure], c: TabCursor) -> Option<&mut TabEvent> {
    ms.get_mut(c.measure)?.events.get_mut(c.event)
}

/// Puts `fret` on the cursor's string in its event; a rest becomes a note. Any technique the
/// string's note had stays.
pub fn set_fret(ms: &mut [TabMeasure], c: TabCursor, fret: u32) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    match e.notes.iter_mut().find(|n| n.string == c.string) {
        Some(n) if n.fret == fret => false,
        Some(n) => {
            n.fret = fret;
            true
        }
        None => {
            e.notes.push(TabNote::new(c.string, fret));
            e.notes.sort_by_key(|n| n.string);
            true
        }
    }
}

/// Changes the fret of the note on the cursor's string by `delta`, not below the open string.
pub fn nudge_fret(ms: &mut [TabMeasure], c: TabCursor, delta: i32) -> bool {
    let Some(n) = event_mut(ms, c).and_then(|e| e.notes.iter_mut().find(|n| n.string == c.string)) else {
        return false;
    };
    let next = (i64::from(n.fret) + i64::from(delta)).clamp(0, i64::from(u32::MAX)) as u32;
    let changed = next != n.fret;
    n.fret = next;
    changed
}

/// Takes the note on the cursor's string out of its event; an event left with no notes is a rest.
pub fn remove_note(ms: &mut [TabMeasure], c: TabCursor) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    let before = e.notes.len();
    e.notes.retain(|n| n.string != c.string);
    e.notes.len() != before
}

/// Turns the cursor's event into a rest of the same length.
pub fn make_rest(ms: &mut [TabMeasure], c: TabCursor) -> bool {
    match event_mut(ms, c) {
        Some(e) if !e.notes.is_empty() => {
            e.notes.clear();
            true
        }
        _ => false,
    }
}

/// Takes the cursor's event out of its measure. Returns where the cursor goes: the event
/// that took its place, or the one before.
pub fn remove_event(ms: &mut [TabMeasure], c: TabCursor) -> Option<TabCursor> {
    let m = ms.get_mut(c.measure)?;
    if c.event >= m.events.len() {
        return None;
    }
    m.events.remove(c.event);
    let event = c.event.min(m.events.len().saturating_sub(1));
    Some(TabCursor { event, ..c })
}

/// Adds a rest after the cursor's event, as long as that event, and moves to it.
pub fn insert_rest_after(ms: &mut [TabMeasure], c: TabCursor) -> Option<TabCursor> {
    let m = ms.get_mut(c.measure)?;
    let (value, dots) = m.events.get(c.event).map_or((NoteValue::Quarter, 0), |e| (e.value, e.dots));
    let at = (c.event + 1).min(m.events.len());
    m.events.insert(at, TabEvent::rest(value).dotted(dots));
    Some(TabCursor { event: at, ..c })
}

/// Adds a measure holding a whole rest after the cursor's measure, and moves to it.
pub fn insert_measure_after(ms: &mut Vec<TabMeasure>, c: TabCursor) -> Option<TabCursor> {
    let at = (c.measure + 1).min(ms.len());
    ms.insert(at, TabMeasure::new(vec![TabEvent::rest(NoteValue::Whole)]));
    Some(TabCursor { measure: at, event: 0, ..c })
}

/// Takes the cursor's measure out, unless it is the only one.
pub fn remove_measure(ms: &mut Vec<TabMeasure>, c: TabCursor) -> Option<TabCursor> {
    if ms.len() < 2 || c.measure >= ms.len() {
        return None;
    }
    ms.remove(c.measure);
    Some(TabCursor { measure: c.measure.min(ms.len() - 1), event: 0, ..c })
}

/// Marks the note on the cursor's string with `technique`, or clears the mark if it already
/// has that one (a dead note has no fret of its own, but keeps the number for when it is cleared).
pub fn set_technique(ms: &mut [TabMeasure], c: TabCursor, technique: Technique) -> bool {
    let Some(n) = event_mut(ms, c).and_then(|e| e.notes.iter_mut().find(|n| n.string == c.string)) else {
        return false;
    };
    n.technique = if n.technique == technique { Technique::None } else { technique };
    true
}

/// Makes the cursor's event a note value longer or shorter.
pub fn change_value(ms: &mut [TabMeasure], c: TabCursor, longer: bool) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    let next = if longer { e.value.longer() } else { e.value.shorter() };
    match next {
        Some(v) => {
            e.value = v;
            true
        }
        None => false,
    }
}

/// Sets the cursor's event to a note value.
pub fn set_value(ms: &mut [TabMeasure], c: TabCursor, value: NoteValue) -> bool {
    match event_mut(ms, c) {
        Some(e) if e.value != value => {
            e.value = value;
            true
        }
        _ => false,
    }
}

/// Cycles the cursor's event through no dot, one dot and two dots.
pub fn toggle_dot(ms: &mut [TabMeasure], c: TabCursor) -> bool {
    let Some(e) = event_mut(ms, c) else { return false };
    e.dots = (e.dots + 1) % 3;
    true
}

/// Called after an edit of a tab, with what it was.
pub type TabEditHandler = Rc<dyn Fn(&TabEdit)>;

/// What an edit of a tab reports.
#[derive(Clone, Debug, PartialEq)]
pub struct TabEdit {
    /// What was done ("Set fret"), as the undo history names it.
    pub label: String,
    /// Where the cursor is after it.
    pub cursor: TabCursor,
}

/// What an editable tab tells the app.
#[derive(Clone, Default)]
pub struct TabEditing {
    /// Called after every edit, with what it was. The measures signal already holds the result.
    pub on_edit: Option<TabEditHandler>,
}

/// How long a typed fret number waits for its second digit.
const TYPING: Duration = Duration::from_millis(1000);

/// The state an editable tab edits through.
#[derive(Clone)]
pub(crate) struct TabEditor {
    pub(crate) ui: Ui,
    pub(crate) measures: Signal<Vec<TabMeasure>>,
    pub(crate) cursor: Signal<TabCursor>,
    pub(crate) on_edit: Option<TabEditHandler>,
    /// The fret being typed: where, and the number so far.
    typing: Rc<Cell<Option<(TabCursor, u32)>>>,
    generation: Rc<Cell<u64>>,
}

impl TabEditor {
    pub(crate) fn new(
        ui: Ui,
        measures: Signal<Vec<TabMeasure>>,
        cursor: Signal<TabCursor>,
        on_edit: Option<TabEditHandler>,
    ) -> TabEditor {
        TabEditor {
            ui,
            measures,
            cursor,
            on_edit,
            typing: Rc::new(Cell::new(None)),
            generation: Rc::new(Cell::new(0)),
        }
    }

    /// Runs `f` on a copy of the measures; if it returns a cursor the change is made, recorded
    /// for undo and reported.
    fn apply(
        &self,
        label: &'static str,
        f: impl FnOnce(&mut Vec<TabMeasure>, TabCursor) -> Option<TabCursor>,
    ) -> bool {
        let before = self.measures.get_untracked().unwrap_or_default();
        let c0 = self.cursor.get_untracked().unwrap_or_default();
        let mut after = before.clone();
        let Some(c1) = f(&mut after, c0) else { return false };
        let (measures, cursor) = (self.measures, self.cursor);
        measures.set(after.clone());
        cursor.set(c1);
        self.ui.record_change(
            self.ui.word(label),
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
            cb(&TabEdit { label: label.to_owned(), cursor: c1 });
        }
        true
    }

    /// Types one digit of a fret number: a second digit soon after the first makes a two-digit fret.
    fn digit(&self, d: u32) -> bool {
        let here = self.cursor.get_untracked().unwrap_or_default();
        let fret = match self.typing.get() {
            Some((at, so_far)) if at == here => {
                so_far.checked_mul(10).and_then(|v| v.checked_add(d)).unwrap_or(d)
            }
            _ => d,
        };
        self.apply(en("Set fret"), |ms, c| set_fret(ms, c, fret).then_some(c));
        self.typing.set(Some((here, fret)));
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let (typing, current) = (self.typing.clone(), self.generation.clone());
        self.ui.after(TYPING, move || {
            if current.get() == generation {
                typing.set(None);
            }
        });
        true
    }

    /// Puts an open string note where a double-click landed, or takes the note there away.
    pub(crate) fn toggle_note(&self) -> bool {
        let there = self.measures.get_untracked().ok().and_then(|ms| {
            let c = self.cursor.get_untracked().ok()?;
            Some(ms.get(c.measure)?.events.get(c.event)?.notes.iter().any(|n| n.string == c.string))
        });
        if there == Some(true) {
            self.apply(en("Delete note"), |ms, c| remove_note(ms, c).then_some(c))
        } else {
            self.apply(en("Add note"), |ms, c| set_fret(ms, c, 0).then_some(c))
        }
    }

    /// Handles an editing key; true if it was one.
    pub(crate) fn key(&self, e: &remedy_ui_input::KeyEvent) -> bool {
        let m = e.modifiers;
        if m.meta || e.composing {
            return false;
        }
        match &e.key {
            Key::ArrowUp | Key::ArrowDown if m.alt => {
                let d = if e.key == Key::ArrowUp { 1 } else { -1 };
                self.apply(if d > 0 { en("Raise fret") } else { en("Lower fret") }, |ms, c| {
                    nudge_fret(ms, c, d).then_some(c)
                })
            }
            Key::Delete | Key::Backspace => {
                if m.ctrl {
                    return self.apply(en("Delete measure"), remove_measure);
                }
                if m.shift {
                    return self.apply(en("Delete event"), |ms, c| remove_event(ms, c));
                }
                // The note on the string goes; a rest, which has none, goes away itself.
                let has_note = self
                    .measures
                    .get_untracked()
                    .ok()
                    .and_then(|ms| {
                        let c = self.cursor.get_untracked().ok()?;
                        let ev = ms.get(c.measure)?.events.get(c.event)?;
                        Some(ev.notes.iter().any(|n| n.string == c.string))
                    })
                    .unwrap_or(false);
                if has_note {
                    self.apply(en("Delete note"), |ms, c| remove_note(ms, c).then_some(c))
                } else {
                    self.apply(en("Make rest"), |ms, c| make_rest(ms, c).then_some(c))
                }
            }
            Key::Named(n) if n == "Insert" => self.apply(en("Insert rest"), |ms, c| insert_rest_after(ms, c)),
            Key::Enter if m.ctrl => self.apply(en("Insert measure"), insert_measure_after),
            Key::Character(ch) if !m.ctrl && !m.alt => {
                if let Some(d) =
                    ch.chars().next().and_then(|c| c.to_digit(10)).filter(|_| ch.chars().count() == 1)
                {
                    return self.digit(d);
                }
                let technique = match ch.as_str() {
                    "h" => Some(Technique::HammerOn),
                    "p" => Some(Technique::PullOff),
                    "/" => Some(Technique::SlideUp),
                    "\\" => Some(Technique::SlideDown),
                    "b" => Some(Technique::Bend(2)),
                    "v" => Some(Technique::Vibrato),
                    "x" => Some(Technique::Dead),
                    "o" => Some(Technique::Harmonic),
                    "m" => Some(Technique::PalmMute),
                    "t" => Some(Technique::Tie),
                    "n" => Some(Technique::None),
                    _ => None,
                };
                if let Some(t) = technique {
                    return self.apply(en("Set technique"), |ms, c| set_technique(ms, c, t).then_some(c));
                }
                match ch.as_str() {
                    "." => self.apply(en("Cycle dots"), |ms, c| toggle_dot(ms, c).then_some(c)),
                    "[" => self.apply(en("Shorten"), |ms, c| change_value(ms, c, false).then_some(c)),
                    "]" => self.apply(en("Lengthen"), |ms, c| change_value(ms, c, true).then_some(c)),
                    "r" => self.apply(en("Make rest"), |ms, c| make_rest(ms, c).then_some(c)),
                    _ => false,
                }
            }
            _ => false,
        }
    }
}
