//! Editing tablature and notation: the pure edit functions, the keyboard and pointer on real
//! windows, undo and redo through the window's history, and slanted beams.
//!
//! The test data has a known shape, so it is indexed directly.
#![allow(clippy::indexing_slicing)]

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::*;
use remedy_ui_core::a11y::Action;
use remedy_ui_core::Point;
use remedy_ui_core::input::{Key, Modifiers};
use remedy_ui_core::reactive::Signal;
use remedy_ui_view::{El, Request, Ui};
use remedy_ui_widgets_music::staff_edit as se;
use remedy_ui_widgets_music::tab_edit as te;
use remedy_ui_widgets_music::*;

fn p(s: &str) -> Pitch {
    Pitch::parse(s).unwrap_or_else(|| unreachable!("{s}"))
}

fn mount(ui: &Ui, el: &El) {
    let _ = ui.root().child(el.clone());
    let _ = frame(ui);
}

const SHIFT: Modifiers = Modifiers { shift: true, ctrl: false, alt: false, meta: false };
const CTRL: Modifiers = Modifiers { shift: false, ctrl: true, alt: false, meta: false };
const ALT: Modifiers = Modifiers { shift: false, ctrl: false, alt: true, meta: false };

// --- the edit functions on tablature ---

fn tab_measures() -> Vec<TabMeasure> {
    use NoteValue::*;
    vec![
        TabMeasure::new(vec![
            TabEvent::new(vec![TabNote::new(2, 0), TabNote::new(5, 3)], Quarter),
            TabEvent::rest(Eighth),
        ]),
        TabMeasure::new(vec![TabEvent::new(vec![TabNote::new(1, 7)], Half)]),
    ]
}

fn tc(measure: usize, event: usize, string: usize) -> TabCursor {
    TabCursor { measure, event, string }
}

#[test]
fn tab_notes_are_set_nudged_and_removed() {
    let mut ms = tab_measures();
    // A fret on a string with a note changes it; on one without adds it, in string order.
    assert!(te::set_fret(&mut ms, tc(0, 0, 5), 12));
    assert!(!te::set_fret(&mut ms, tc(0, 0, 5), 12), "the same fret is no change");
    assert!(te::set_fret(&mut ms, tc(0, 0, 3), 4));
    let strings: Vec<usize> = ms[0].events[0].notes.iter().map(|n| n.string).collect();
    assert_eq!(strings, [2, 3, 5]);
    // A rest becomes a note.
    assert!(te::set_fret(&mut ms, tc(0, 1, 4), 2));
    assert!(!ms[0].events[1].is_rest());
    // A technique stays when the fret changes.
    assert!(te::set_technique(&mut ms, tc(0, 0, 5), Technique::Vibrato));
    assert!(te::set_fret(&mut ms, tc(0, 0, 5), 14));
    assert_eq!(ms[0].events[0].notes.last().map(|n| (n.fret, n.technique)), Some((14, Technique::Vibrato)));
    // Nudging stops at the open string.
    assert!(te::nudge_fret(&mut ms, tc(0, 0, 3), 1));
    assert!(te::nudge_fret(&mut ms, tc(0, 0, 3), -100));
    assert_eq!(ms[0].events[0].notes.iter().find(|n| n.string == 3).map(|n| n.fret), Some(0));
    assert!(!te::nudge_fret(&mut ms, tc(0, 0, 3), -1));
    assert!(!te::nudge_fret(&mut ms, tc(0, 0, 4), 1), "no note there");
    // Removing the last note leaves a rest of the same length.
    assert!(te::remove_note(&mut ms, tc(1, 0, 1)));
    assert!(ms[1].events[0].is_rest() && ms[1].events[0].value == NoteValue::Half);
    assert!(!te::remove_note(&mut ms, tc(1, 0, 1)));
    assert!(te::make_rest(&mut ms, tc(0, 0, 0)));
    assert!(ms[0].events[0].is_rest());
    assert!(!te::make_rest(&mut ms, tc(0, 0, 0)));
    // Nothing happens off the end.
    assert!(!te::set_fret(&mut ms, tc(9, 0, 0), 1) && !te::remove_note(&mut ms, tc(0, 9, 0)));
}

#[test]
fn tab_techniques_toggle_and_lengths_change() {
    let mut ms = tab_measures();
    let c = tc(0, 0, 5);
    assert!(te::set_technique(&mut ms, c, Technique::HammerOn));
    assert_eq!(ms[0].events[0].notes.last().map(|n| n.technique), Some(Technique::HammerOn));
    assert!(te::set_technique(&mut ms, c, Technique::HammerOn));
    assert_eq!(ms[0].events[0].notes.last().map(|n| n.technique), Some(Technique::None), "again clears it");
    assert!(te::set_technique(&mut ms, c, Technique::Bend(2)));
    assert!(te::set_technique(&mut ms, c, Technique::Dead));
    assert_eq!(ms[0].events[0].notes.last().map(|n| n.technique), Some(Technique::Dead), "one at a time");
    assert!(!te::set_technique(&mut ms, tc(0, 0, 4), Technique::Dead), "no note there");
    // Lengths.
    assert!(te::change_value(&mut ms, c, false));
    assert_eq!(ms[0].events[0].value, NoteValue::Eighth);
    assert!(te::change_value(&mut ms, c, true) && te::change_value(&mut ms, c, true));
    assert_eq!(ms[0].events[0].value, NoteValue::Half);
    for _ in 0..4 {
        te::change_value(&mut ms, c, true);
    }
    assert_eq!(ms[0].events[0].value, NoteValue::Whole);
    assert!(!te::change_value(&mut ms, c, true), "no longer than whole");
    for _ in 0..8 {
        te::change_value(&mut ms, c, false);
    }
    assert_eq!(ms[0].events[0].value, NoteValue::ThirtySecond);
    assert!(!te::change_value(&mut ms, c, false));
    assert!(te::set_value(&mut ms, c, NoteValue::Quarter) && !te::set_value(&mut ms, c, NoteValue::Quarter));
    assert!(te::toggle_dot(&mut ms, c) && ms[0].events[0].dots == 1);
    assert!(te::toggle_dot(&mut ms, c) && ms[0].events[0].dots == 2);
    assert!(te::toggle_dot(&mut ms, c) && ms[0].events[0].dots == 0);
}

#[test]
fn tab_events_and_measures_are_inserted_and_removed() {
    let mut ms = tab_measures();
    let at = te::insert_rest_after(&mut ms, tc(0, 0, 5));
    assert_eq!(at, Some(tc(0, 1, 5)));
    assert_eq!(ms[0].events.len(), 3);
    assert!(
        ms[0].events[1].is_rest() && ms[0].events[1].value == NoteValue::Quarter,
        "as long as the event before"
    );
    assert_eq!(te::remove_event(&mut ms, tc(0, 1, 5)), Some(tc(0, 1, 5)));
    assert_eq!(
        te::remove_event(&mut ms, tc(0, 1, 5)),
        Some(tc(0, 0, 5)),
        "the cursor falls back when the last goes"
    );
    assert_eq!(te::remove_event(&mut ms, tc(0, 5, 5)), None);
    let at = te::insert_measure_after(&mut ms, tc(0, 0, 5));
    assert_eq!(at, Some(tc(1, 0, 5)));
    assert_eq!(ms.len(), 3);
    assert!(ms[1].events[0].is_rest() && ms[1].events[0].value == NoteValue::Whole);
    assert_eq!(te::remove_measure(&mut ms, tc(1, 0, 5)), Some(tc(1, 0, 5)));
    assert_eq!(ms.len(), 2);
    let mut one = vec![TabMeasure::default()];
    assert_eq!(te::remove_measure(&mut one, tc(0, 0, 0)), None, "the last measure stays");
}

// --- the edit functions on notation ---

fn sc(measure: usize, event: usize) -> StaffCursor {
    StaffCursor { measure, event }
}

fn staff_measures() -> Vec<StaffMeasure> {
    use NoteValue::*;
    vec![
        StaffMeasure::new(vec![
            StaffEvent::note(p("E4"), Quarter),
            StaffEvent::rest(Quarter),
            StaffEvent::chord(vec![p("C4"), p("G4")], Half),
        ]),
        StaffMeasure::new(vec![StaffEvent::note(p("C5"), Whole)]),
    ]
}

#[test]
fn pitches_come_from_staff_positions_and_move_by_step_semitone_and_octave() {
    // The bottom line and top line of each clef.
    assert_eq!(se::pitch_at_position(0, Clef::Treble, 0), p("E4"));
    assert_eq!(se::pitch_at_position(8, Clef::Treble, 0), p("F5"));
    assert_eq!(se::pitch_at_position(-2, Clef::Treble, 0), p("C4"));
    assert_eq!(se::pitch_at_position(0, Clef::Bass, 0), p("G2"));
    assert_eq!(se::pitch_at_position(4, Clef::Alto, 0), p("C4"));
    // The key spells it: F on the top line in G major is F sharp.
    assert_eq!(se::pitch_at_position(8, Clef::Treble, 1), p("F#5"));
    assert_eq!(se::pitch_at_position(8, Clef::Treble, -2), p("F5"));
    assert_eq!(se::pitch_at_position(4, Clef::Treble, -1), p("Bb4"));
    // A step moves by letter and takes the key's spelling; a semitone by half steps.
    assert_eq!(se::step_pitch(p("E4"), 1, 1), p("F#4"));
    assert_eq!(se::step_pitch(p("E4"), 1, 0), p("F4"));
    assert_eq!(se::step_pitch(p("B4"), 1, 0), p("C5"));
    assert_eq!(se::step_pitch(p("C4"), -1, 0), p("B3"));
    assert_eq!(se::step_pitch(p("C#4"), 2, 0), p("E4"), "an accidental is not carried along");
    assert_eq!(se::shift_pitch(p("C4"), 1, 0), p("C#4"));
    assert_eq!(se::shift_pitch(p("C4"), -1, 0), p("B3"));
    assert_eq!(se::shift_pitch(p("E4"), -1, 0), p("Eb4"));
    assert_eq!(se::shift_pitch(p("D4"), 1, -2), p("Eb4"), "flat keys spell with flats");
    assert_eq!(se::shift_pitch(p("B3"), 1, 0), p("C4"));
}

#[test]
fn notes_are_set_by_letter_sharpened_tied_and_removed() {
    let mut ms = staff_measures();
    // Up a step, a semitone and an octave change every note of a chord.
    assert!(se::move_steps(&mut ms, sc(0, 0), 1, 0));
    assert_eq!(ms[0].events[0].pitches, [p("F4")]);
    assert!(se::move_steps(&mut ms, sc(0, 2), -1, 0));
    assert_eq!(ms[0].events[2].pitches, [p("B3"), p("F4")]);
    assert!(se::move_semitones(&mut ms, sc(0, 0), 1, 0));
    assert_eq!(ms[0].events[0].pitches, [p("F#4")]);
    assert!(se::move_octaves(&mut ms, sc(0, 0), -1));
    assert_eq!(ms[0].events[0].pitches, [p("F#3")]);
    assert!(!se::move_steps(&mut ms, sc(0, 1), 1, 0), "a rest has no pitch to move");
    // A letter picks the nearest octave: from C5, B is B4, not B5.
    assert!(se::set_letter(&mut ms, sc(1, 0), Step::B, Clef::Treble, 0, false));
    assert_eq!(ms[1].events[0].pitches, [p("B4")]);
    assert!(se::set_letter(&mut ms, sc(1, 0), Step::D, Clef::Treble, 0, false));
    assert_eq!(ms[1].events[0].pitches, [p("D5")]);
    // A rest becomes a note near the middle line; the key spells it.
    assert!(se::set_letter(&mut ms, sc(0, 1), Step::F, Clef::Treble, 1, false));
    assert_eq!(ms[0].events[1].pitches, [p("F#4")]);
    assert!(se::make_rest(&mut ms, sc(0, 1)));
    assert!(se::set_letter(&mut ms, sc(0, 1), Step::A, Clef::Bass, 0, false));
    assert_eq!(ms[0].events[1].pitches, [p("A2")], "the bass staff's middle line is D3, and A2 is nearest");
    // Adding makes a chord (low to high) without doubling.
    assert!(se::set_letter(&mut ms, sc(1, 0), Step::F, Clef::Treble, 0, true));
    assert_eq!(ms[1].events[0].pitches, [p("D5"), p("F5")]);
    assert!(!se::set_letter(&mut ms, sc(1, 0), Step::F, Clef::Treble, 0, true));
    // Accidentals.
    assert!(se::alter(&mut ms, sc(1, 0), 1));
    assert_eq!(ms[1].events[0].pitches, [p("D#5"), p("F#5")]);
    assert!(se::alter(&mut ms, sc(1, 0), -1) && se::alter(&mut ms, sc(1, 0), -1));
    assert_eq!(ms[1].events[0].pitches, [p("Db5"), p("Fb5")]);
    assert!(se::alter(&mut ms, sc(1, 0), 0));
    assert_eq!(ms[1].events[0].pitches, [p("D5"), p("F5")]);
    for _ in 0..5 {
        se::alter(&mut ms, sc(1, 0), 1);
    }
    assert_eq!(ms[1].events[0].pitches.first().map(|q| q.alter), Some(2), "at most a double sharp");
    // The pointer's way in.
    assert!(se::set_pitch_at(&mut ms, sc(0, 1), 2, Clef::Treble, 0, false));
    assert_eq!(ms[0].events[1].pitches, [p("G4")]);
    // Ties need a note.
    assert!(se::toggle_tie(&mut ms, sc(0, 1)) && ms[0].events[1].tie);
    assert!(se::toggle_tie(&mut ms, sc(0, 1)) && !ms[0].events[1].tie);
    assert!(se::toggle_tie(&mut ms, sc(0, 1)));
    assert!(se::make_rest(&mut ms, sc(0, 1)));
    assert!(ms[0].events[1].is_rest() && !ms[0].events[1].tie, "a rest holds no tie");
    assert!(!se::toggle_tie(&mut ms, sc(0, 1)));
}

#[test]
fn staff_lengths_events_and_measures_change() {
    let mut ms = staff_measures();
    let c = sc(0, 0);
    assert!(
        se::set_value(&mut ms, c, NoteValue::Sixteenth) && !se::set_value(&mut ms, c, NoteValue::Sixteenth)
    );
    assert!(se::change_value(&mut ms, c, true) && ms[0].events[0].value == NoteValue::Eighth);
    assert!(se::change_value(&mut ms, c, false) && ms[0].events[0].value == NoteValue::Sixteenth);
    assert!(se::toggle_dot(&mut ms, c) && se::toggle_dot(&mut ms, c) && se::toggle_dot(&mut ms, c));
    assert_eq!(ms[0].events[0].dots, 0);
    assert_eq!(se::insert_rest_after(&mut ms, c), Some(sc(0, 1)));
    assert_eq!(ms[0].events.len(), 4);
    assert_eq!(se::remove_event(&mut ms, sc(0, 3)), Some(sc(0, 2)));
    assert_eq!(se::insert_measure_after(&mut ms, sc(0, 0)), Some(sc(1, 0)));
    assert_eq!(ms.len(), 3);
    assert_eq!(se::remove_measure(&mut ms, sc(1, 0)), Some(sc(1, 0)));
    assert_eq!(se::remove_measure(&mut vec![StaffMeasure::default()], sc(0, 0)), None);
}

// --- slanted beams ---

#[test]
fn beams_follow_the_contour_within_the_usual_limits() {
    let sp = 10.0;
    let middle = 100.0;
    // Stems up: y grows downward, so higher notes are smaller y. Rising notes slant the beam up.
    let rising = [(0.0, 140.0), (20.0, 130.0), (40.0, 120.0), (60.0, 110.0)];
    let (y0, slope) = beam_line(&rising, true, sp, middle);
    assert!(slope < 0.0, "rises to the right: {slope}");
    let rise = -slope * 60.0;
    assert!(rise <= sp + 1e-3, "no more than one staff space: {rise}");
    // Half the rise from the first to the last note.
    assert!((rise - 15.0f32.min(sp)).abs() < 1e-3);
    // Every stem keeps at least its usual length, and the beam is as close as that allows.
    for (x, y) in rising {
        let beam = y0 + slope * x;
        assert!(y - beam >= 3.5 * sp - 1e-3, "stem at {x}: {}", y - beam);
    }
    let tight = rising.iter().any(|(x, y)| ((y - (y0 + slope * x)) - 3.5 * sp).abs() < 1e-3);
    assert!(tight, "the beam is no farther out than it must be");
    // Falling notes slant the other way; a small interval slants by half of it.
    let (_, down) = beam_line(&[(0.0, 120.0), (30.0, 124.0)], true, sp, middle);
    assert!(down > 0.0 && (down * 30.0 - 2.0).abs() < 1e-3, "{down}");
    // Level ends are level; so is a peak or a valley in the middle; so is one note.
    assert_eq!(beam_line(&[(0.0, 130.0), (30.0, 130.0)], true, sp, middle).1, 0.0);
    let valley_for_up = [(0.0, 130.0), (20.0, 150.0), (40.0, 135.0)];
    assert_ne!(
        beam_line(&valley_for_up, true, sp, middle).1,
        0.0,
        "a low middle note does not stop the slant"
    );
    let peak_for_up = [(0.0, 130.0), (20.0, 110.0), (40.0, 135.0)];
    assert_eq!(beam_line(&peak_for_up, true, sp, middle).1, 0.0, "a high middle note makes it level");
    let peak_for_down = [(0.0, 130.0), (20.0, 150.0), (40.0, 135.0)];
    assert_eq!(beam_line(&peak_for_down, false, sp, middle).1, 0.0);
    assert_eq!(beam_line(&[(5.0, 130.0)], true, sp, middle).1, 0.0);
    assert_eq!(beam_line(&[], true, sp, middle).1, 0.0);
    // Stems down mirror: the beam is below the lowest heads.
    let (y0, slope) = beam_line(&[(0.0, 60.0), (20.0, 70.0)], false, sp, middle);
    assert!(slope > 0.0);
    for (x, y) in [(0.0f32, 60.0f32), (20.0, 70.0)] {
        assert!(y0 + slope * x - y >= 3.5 * sp - 1e-3);
    }
    // Stems always reach the middle line.
    let (y0, slope) = beam_line(&[(0.0, 95.0), (20.0, 95.0)], true, sp, middle);
    assert!(y0 + slope * 0.0 <= middle + 1e-3 && y0 + slope * 20.0 <= middle + 1e-3);
    // A wide group at the limit never slants more than a space, however far apart the ends.
    let (_, slope) = beam_line(&[(0.0, 300.0), (100.0, 100.0)], true, sp, middle);
    assert!((-slope * 100.0 - sp).abs() < 1e-3);
}

/// Where the beam's ink is in the left and the right half of a staff of four eighth notes: the
/// average row of the thick, wide ink above the staff's middle line (beams, not stems or lines).
fn beam_heights(notes: &[&str]) -> Option<(f32, f32)> {
    use NoteValue::Eighth;
    let ui = ui();
    let events: Vec<StaffEvent> = notes.iter().map(|n| StaffEvent::note(p(n), Eighth)).collect();
    let ms = Signal::new(vec![StaffMeasure::new(events)]);
    let el = note_staff(&ui, "Run", ms, Signal::new(None), {
        let mut v_ = NoteStaffOptions::default();
        v_.space = Some(14.0);
        v_.time = None;
        v_
    });
    mount(&ui, &el);
    let mut shot = Shot::new(&ui);
    for _ in 0..4 {
        shot.take(&ui);
    }
    let b = el.bounds().unwrap_or_default();
    let bg = rgb(ui.theme().colors.bg);
    let ink = |x: u32, y: u32| !near(shot.at(x, y), bg, 12);
    let mut found: Vec<(u32, u32)> = Vec::new();
    // Above the middle line (70 down from the top of this staff), right of the clef.
    for y in (b.min.y as u32 + 16)..(b.min.y as u32 + 66) {
        for x in (b.min.x as u32 + 70)..(b.max.x as u32 - 4) {
            let wide = (x - 3..=x + 3).all(|xx| ink(xx, y));
            let thick = (0..5).all(|d| ink(x, y + d));
            if wide && thick {
                found.push((x, y));
            }
        }
    }
    let (lo, hi) = (found.iter().map(|f| f.0).min()?, found.iter().map(|f| f.0).max()?);
    let mid = (lo + hi) / 2;
    let avg = |side: &dyn Fn(u32) -> bool| {
        let ys: Vec<f32> = found.iter().filter(|f| side(f.0)).map(|f| f.1 as f32).collect();
        (!ys.is_empty()).then(|| ys.iter().sum::<f32>() / ys.len() as f32)
    };
    Some((avg(&|x| x < mid)?, avg(&|x| x >= mid)?))
}

#[test]
fn a_run_of_eighths_is_drawn_with_a_beam_that_follows_the_contour() {
    let (left, right) =
        beam_heights(&["C4", "D4", "E4", "F4"]).unwrap_or_else(|| unreachable!("a beam is drawn"));
    assert!(right < left - 2.0, "a rising run slants up to the right: {left} to {right}");
    let (left, right) =
        beam_heights(&["F4", "E4", "D4", "C4"]).unwrap_or_else(|| unreachable!("a beam is drawn"));
    assert!(right > left + 2.0, "a falling run slants down: {left} to {right}");
    let (left, right) =
        beam_heights(&["D4", "D4", "D4", "D4"]).unwrap_or_else(|| unreachable!("a beam is drawn"));
    assert!((right - left).abs() < 1.5, "repeated notes are level: {left} {right}");
}

// --- the editors on real windows ---

struct TabRig {
    ui: Ui,
    el: El,
    measures: Signal<Vec<TabMeasure>>,
    edits: Rc<RefCell<Vec<TabEdit>>>,
}

fn tab_rig(editable: bool) -> TabRig {
    let ui = ui();
    let measures = Signal::new(tab_measures());
    let edits: Rc<RefCell<Vec<TabEdit>>> = Rc::default();
    let log = edits.clone();
    let editing = TabEditing { on_edit: Some(Rc::new(move |e: &TabEdit| log.borrow_mut().push(e.clone()))) };
    let el = if editable {
        tab_staff_editor(&ui, "Riff", measures, Signal::new(None), TabOptions::default(), editing)
    } else {
        tab_staff(&ui, "Riff", measures, Signal::new(None), TabOptions::default())
    };
    mount(&ui, &el);
    el.focus();
    let _ = frame(&ui);
    TabRig { ui, el, measures, edits }
}

fn tab_at(r: &TabRig, m: usize, e: usize, s: usize) -> Option<(u32, Technique)> {
    let ms = r.measures.get_untracked().ok()?;
    ms.get(m)?.events.get(e)?.notes.iter().find(|n| n.string == s).map(|n| (n.fret, n.technique))
}

fn fire_timers(ui: &Ui) {
    for r in ui.take_requests() {
        if let Request::Timer { id, .. } = r {
            ui.fire_timer(id);
        }
    }
}

#[test]
fn typing_digits_enters_frets_and_two_digits_make_one() {
    let r = tab_rig(true);
    // The cursor starts on the top string of the first event, which has no note there.
    assert_eq!(tab_at(&r, 0, 0, 5), Some((3, Technique::None)));
    key(&r.ui, Key::char('7'));
    assert_eq!(tab_at(&r, 0, 0, 5), Some((7, Technique::None)));
    key(&r.ui, Key::char('2'));
    assert_eq!(
        tab_at(&r, 0, 0, 5),
        Some((72, Technique::None)),
        "a second digit soon after follows the first"
    );
    fire_timers(&r.ui);
    key(&r.ui, Key::char('5'));
    assert_eq!(tab_at(&r, 0, 0, 5), Some((5, Technique::None)), "after a pause a digit starts a new number");
    // Moving on starts again too.
    key(&r.ui, Key::ArrowDown);
    key(&r.ui, Key::char('1'));
    key(&r.ui, Key::char('0'));
    assert_eq!(tab_at(&r, 0, 0, 4), Some((10, Technique::None)));
    assert_eq!(value_text(&r.el), "measure 1, beat 1, string 2: fret 10");
    // Every edit was reported with what it was and where the cursor is.
    let labels: Vec<String> = r.edits.borrow().iter().map(|e| e.label.clone()).collect();
    assert_eq!(labels, ["Set fret", "Set fret", "Set fret", "Set fret", "Set fret"]);
    assert_eq!(r.edits.borrow().last().map(|e| e.cursor), Some(tc(0, 0, 4)));
}

#[test]
fn tab_edits_are_undone_and_redone_through_the_window_history() {
    let r = tab_rig(true);
    let before = r.measures.get_untracked().unwrap_or_default();
    key(&r.ui, Key::char('9'));
    fire_timers(&r.ui);
    key(&r.ui, Key::char('h'));
    assert_eq!(tab_at(&r, 0, 0, 5), Some((9, Technique::HammerOn)));
    let names: Vec<String> = r.ui.history().into_iter().map(|h| h.label).collect();
    assert_eq!(names, ["Set fret", "Set technique"]);
    assert!(r.ui.undo_change());
    assert_eq!(tab_at(&r, 0, 0, 5), Some((9, Technique::None)));
    assert!(r.ui.undo_change());
    assert_eq!(r.measures.get_untracked().ok(), Some(before.clone()));
    assert_eq!(value_text(&r.el), "measure 1, beat 1, string 1: fret 3", "the cursor comes back too");
    assert!(!r.ui.undo_change(), "nothing more to take back");
    assert!(r.ui.redo_change() && r.ui.redo_change());
    assert_eq!(tab_at(&r, 0, 0, 5), Some((9, Technique::HammerOn)));
    // Ctrl+Z on the keyboard does the same when no field has focus.
    key_mod(&r.ui, Key::char('z'), CTRL);
    assert_eq!(tab_at(&r, 0, 0, 5), Some((9, Technique::None)));
    // A new edit drops what was undone.
    key(&r.ui, Key::char('v'));
    assert!(!r.ui.redo_change());
}

#[test]
fn tab_keys_delete_mark_nudge_insert_and_resize() {
    let r = tab_rig(true);
    // Techniques toggle.
    key(&r.ui, Key::char('v'));
    assert_eq!(tab_at(&r, 0, 0, 5), Some((3, Technique::Vibrato)));
    key(&r.ui, Key::char('v'));
    assert_eq!(tab_at(&r, 0, 0, 5), Some((3, Technique::None)));
    for (k, want) in [
        ('h', Technique::HammerOn),
        ('p', Technique::PullOff),
        ('/', Technique::SlideUp),
        ('\\', Technique::SlideDown),
        ('b', Technique::Bend(2)),
        ('x', Technique::Dead),
        ('o', Technique::Harmonic),
        ('m', Technique::PalmMute),
        ('t', Technique::Tie),
    ] {
        key(&r.ui, Key::char(k));
        assert_eq!(tab_at(&r, 0, 0, 5).map(|t| t.1), Some(want), "key {k}");
    }
    key(&r.ui, Key::char('n'));
    assert_eq!(tab_at(&r, 0, 0, 5).map(|t| t.1), Some(Technique::None));
    // Alt+Up and Alt+Down change the fret.
    key_mod(&r.ui, Key::ArrowUp, ALT);
    key_mod(&r.ui, Key::ArrowUp, ALT);
    assert_eq!(tab_at(&r, 0, 0, 5).map(|t| t.0), Some(5));
    key_mod(&r.ui, Key::ArrowDown, ALT);
    assert_eq!(tab_at(&r, 0, 0, 5).map(|t| t.0), Some(4));
    // Lengths and dots.
    key(&r.ui, Key::char(']'));
    assert_eq!(r.measures.get_untracked().ok().map(|m| m[0].events[0].value), Some(NoteValue::Half));
    key(&r.ui, Key::char('['));
    key(&r.ui, Key::char('['));
    key(&r.ui, Key::char('.'));
    let e = r.measures.get_untracked().ok().map(|m| (m[0].events[0].value, m[0].events[0].dots));
    assert_eq!(e, Some((NoteValue::Eighth, 1)));
    // Delete takes the note on the cursor's string; the other string's note stays.
    key(&r.ui, Key::Delete);
    assert_eq!(tab_at(&r, 0, 0, 5), None);
    assert_eq!(tab_at(&r, 0, 0, 2), Some((0, Technique::None)));
    // With nothing on the string, Delete makes the whole event a rest.
    key(&r.ui, Key::Delete);
    assert!(r.measures.get_untracked().ok().is_some_and(|m| m[0].events[0].is_rest()));
    // Insert adds a rest after, Shift+Delete takes an event out, Ctrl+Enter adds a measure.
    key(&r.ui, Key::Named("Insert".into()));
    assert_eq!(r.measures.get_untracked().ok().map(|m| m[0].events.len()), Some(3));
    assert!(
        value_text(&r.el).starts_with("measure 1, beat 1.5")
            || value_text(&r.el).starts_with("measure 1, beat 1.")
    );
    key_mod(&r.ui, Key::Delete, SHIFT);
    assert_eq!(r.measures.get_untracked().ok().map(|m| m[0].events.len()), Some(2));
    key_mod(&r.ui, Key::Enter, CTRL);
    assert_eq!(r.measures.get_untracked().ok().map(|m| m.len()), Some(3));
    assert!(value_text(&r.el).starts_with("measure 2"));
    key_mod(&r.ui, Key::Delete, CTRL);
    assert_eq!(r.measures.get_untracked().ok().map(|m| m.len()), Some(2));
    // Keys that are not edits still move the cursor, and Ctrl+Z is not eaten.
    assert!(key(&r.ui, Key::ArrowRight));
    assert!(!key_mod(&r.ui, Key::char('q'), Modifiers::NONE), "other letters are not edits");
    assert!(audit(&r.ui).is_empty(), "{:?}", audit(&r.ui));
}

#[test]
fn a_tab_that_is_not_editable_ignores_edit_keys() {
    let r = tab_rig(false);
    let before = r.measures.get_untracked().ok();
    for k in ['5', 'h', 'r', '['] {
        assert!(!key(&r.ui, Key::char(k)), "{k}");
    }
    assert!(!key(&r.ui, Key::Delete));
    assert_eq!(r.measures.get_untracked().ok(), before);
    assert!(r.ui.history().is_empty());
    assert!(r.edits.borrow().is_empty());
}

#[test]
fn double_clicking_a_tab_adds_or_removes_a_note() {
    let r = tab_rig(true);
    let b = r.el.bounds().unwrap_or_default();
    let gap = 14.0;
    let top_line = b.min.y + gap * 0.5 + gap * 2.4;
    // The second string from the top of the first event is empty: find the column by clicking the event with a note.
    let x = ((b.min.x as i32 + 90)..(b.max.x as i32 - 20))
        .step_by(4)
        .map(|x| x as f32)
        .find(|x| {
            click(&r.ui, Point::new(*x, top_line + gap));
            r.el.info().is_some() && value_text(&r.el).starts_with("measure 1, beat 1, string 2")
        })
        .unwrap_or(b.min.x + 100.0);
    let at = Point::new(x, top_line + gap);
    let away = Point::new(b.max.x - 5.0, b.max.y - 3.0);
    let double = |r: &TabRig| {
        click(&r.ui, away);
        click(&r.ui, at);
        click(&r.ui, at);
    };
    assert_eq!(tab_at(&r, 0, 0, 4), None);
    double(&r);
    assert_eq!(tab_at(&r, 0, 0, 4), Some((0, Technique::None)), "a double-click adds a note");
    // A third press is not another double-click.
    click(&r.ui, at);
    assert_eq!(tab_at(&r, 0, 0, 4), Some((0, Technique::None)));
    double(&r);
    assert_eq!(tab_at(&r, 0, 0, 4), None, "and another removes it");
    assert!(r.ui.history().iter().any(|h| h.label == "Add note"));
}

struct StaffRig {
    ui: Ui,
    el: El,
    measures: Signal<Vec<StaffMeasure>>,
    edits: Rc<RefCell<Vec<StaffEdit>>>,
}

fn staff_rig(editable: bool, ms: Vec<StaffMeasure>, options: NoteStaffOptions) -> StaffRig {
    let ui = ui();
    let measures = Signal::new(ms);
    let edits: Rc<RefCell<Vec<StaffEdit>>> = Rc::default();
    let log = edits.clone();
    let editing =
        StaffEditing { on_edit: Some(Rc::new(move |e: &StaffEdit| log.borrow_mut().push(e.clone()))) };
    let el = if editable {
        note_staff_editor(&ui, "Melody", measures, Signal::new(None), options, editing)
    } else {
        note_staff(&ui, "Melody", measures, Signal::new(None), options)
    };
    mount(&ui, &el);
    el.focus();
    let _ = frame(&ui);
    StaffRig { ui, el, measures, edits }
}

fn pitches_at(r: &StaffRig, m: usize, e: usize) -> Vec<Pitch> {
    r.measures
        .get_untracked()
        .ok()
        .and_then(|ms| ms.get(m)?.events.get(e).map(|ev| ev.pitches.clone()))
        .unwrap_or_default()
}

#[test]
fn staff_keys_move_notes_by_step_semitone_and_octave() {
    let r = staff_rig(true, staff_measures(), NoteStaffOptions::default());
    assert_eq!(value_text(&r.el), "measure 1, beat 1: E 4 quarter note");
    key(&r.ui, Key::ArrowUp);
    assert_eq!(pitches_at(&r, 0, 0), [p("F4")]);
    assert_eq!(value_text(&r.el), "measure 1, beat 1: F 4 quarter note");
    key_mod(&r.ui, Key::ArrowUp, SHIFT);
    assert_eq!(pitches_at(&r, 0, 0), [p("F#4")]);
    key_mod(&r.ui, Key::ArrowDown, SHIFT);
    key_mod(&r.ui, Key::ArrowDown, SHIFT);
    assert_eq!(pitches_at(&r, 0, 0), [p("E4")]);
    key_mod(&r.ui, Key::ArrowUp, CTRL);
    assert_eq!(pitches_at(&r, 0, 0), [p("E5")]);
    key(&r.ui, Key::ArrowDown);
    key(&r.ui, Key::ArrowDown);
    assert_eq!(pitches_at(&r, 0, 0), [p("C5")]);
    // Letters set the note, + - = alter it.
    key(&r.ui, Key::char('g'));
    assert_eq!(pitches_at(&r, 0, 0), [p("G4")], "the nearest G to C5");
    key(&r.ui, Key::char('+'));
    assert_eq!(pitches_at(&r, 0, 0), [p("G#4")]);
    key(&r.ui, Key::char('-'));
    key(&r.ui, Key::char('-'));
    assert_eq!(pitches_at(&r, 0, 0), [p("Gb4")]);
    key(&r.ui, Key::char('='));
    assert_eq!(pitches_at(&r, 0, 0), [p("G4")]);
    // Shift+letter joins a chord.
    key_mod(&r.ui, Key::char('b'), SHIFT);
    assert_eq!(pitches_at(&r, 0, 0), [p("G4"), p("B4")]);
    assert_eq!(value_text(&r.el), "measure 1, beat 1: quarter chord of G 4, B 4");
    // The screen reader's increment and decrement are the arrow keys.
    assert!(actions_of(&r.el).contains(&Action::Increment) && actions_of(&r.el).contains(&Action::Decrement));
    let labels: Vec<String> = r.edits.borrow().iter().map(|e| e.label.clone()).collect();
    assert_eq!(labels.first().map(String::as_str), Some("Move note up"));
    assert_eq!(labels.last().map(String::as_str), Some("Add note to chord"));
}

#[test]
fn staff_keys_set_lengths_rests_ties_and_structure() {
    let r = staff_rig(true, staff_measures(), {
        let mut v_ = NoteStaffOptions::default();
        v_.key = 1;
        v_
    });
    for (k, v) in [
        ('1', NoteValue::Whole),
        ('2', NoteValue::Half),
        ('3', NoteValue::Quarter),
        ('4', NoteValue::Eighth),
        ('5', NoteValue::Sixteenth),
        ('6', NoteValue::ThirtySecond),
    ] {
        key(&r.ui, Key::char(k));
        assert_eq!(r.measures.get_untracked().ok().map(|m| m[0].events[0].value), Some(v), "{k}");
    }
    key(&r.ui, Key::char(']'));
    key(&r.ui, Key::char('.'));
    let e = r.measures.get_untracked().ok().map(|m| (m[0].events[0].value, m[0].events[0].dots));
    assert_eq!(e, Some((NoteValue::Sixteenth, 1)));
    key(&r.ui, Key::char('['));
    // In G major a letter F is F sharp.
    key(&r.ui, Key::char('f'));
    assert_eq!(pitches_at(&r, 0, 0), [p("F#4")]);
    // Ties and rests.
    key(&r.ui, Key::char('t'));
    assert!(r.measures.get_untracked().ok().is_some_and(|m| m[0].events[0].tie));
    key(&r.ui, Key::char('r'));
    assert!(pitches_at(&r, 0, 0).is_empty());
    assert!(r.measures.get_untracked().ok().is_some_and(|m| !m[0].events[0].tie));
    assert!(!key(&r.ui, Key::char('t')), "a rest cannot be tied: the key is not an edit and goes unused");
    // On a rest a letter makes a note again; Delete makes a note a rest and a rest gone.
    key(&r.ui, Key::char('d'));
    assert_eq!(pitches_at(&r, 0, 0), [p("D5")], "near the middle line, B4, the D above");
    key(&r.ui, Key::Delete);
    assert!(pitches_at(&r, 0, 0).is_empty());
    key(&r.ui, Key::Delete);
    assert_eq!(r.measures.get_untracked().ok().map(|m| m[0].events.len()), Some(2));
    key(&r.ui, Key::Named("Insert".into()));
    assert_eq!(r.measures.get_untracked().ok().map(|m| m[0].events.len()), Some(3));
    key_mod(&r.ui, Key::Delete, SHIFT);
    key_mod(&r.ui, Key::Enter, CTRL);
    assert_eq!(r.measures.get_untracked().ok().map(|m| m.len()), Some(3));
    key_mod(&r.ui, Key::Delete, CTRL);
    assert_eq!(r.measures.get_untracked().ok().map(|m| m.len()), Some(2));
    assert!(audit(&r.ui).is_empty(), "{:?}", audit(&r.ui));
}

#[test]
fn staff_edits_are_undone_and_redone_and_reported() {
    let r = staff_rig(true, staff_measures(), NoteStaffOptions::default());
    let before = r.measures.get_untracked().unwrap_or_default();
    key(&r.ui, Key::ArrowUp);
    key(&r.ui, Key::char('4'));
    key(&r.ui, Key::char('r'));
    assert_eq!(r.edits.borrow().len(), 3);
    let history: Vec<String> = r.ui.history().into_iter().map(|h| h.label).collect();
    assert_eq!(history, ["Move note up", "Set length", "Make rest"]);
    assert_eq!(r.ui.next_undo().map(|h| h.undo_label()), Some("Undo Make rest".to_owned()));
    assert!(r.ui.undo_change() && r.ui.undo_change());
    assert_eq!(pitches_at(&r, 0, 0), [p("F4")]);
    assert!(r.ui.undo_change());
    assert_eq!(r.measures.get_untracked().ok(), Some(before));
    assert!(r.ui.redo_change() && r.ui.redo_change() && r.ui.redo_change());
    assert!(pitches_at(&r, 0, 0).is_empty());
    // A no-op is neither recorded nor reported: the rest cannot move.
    let n = r.ui.history().len();
    key(&r.ui, Key::ArrowUp);
    assert_eq!(r.ui.history().len(), n);
    assert_eq!(r.edits.borrow().len(), 3);
    // The structure edits undo as one step each.
    key_mod(&r.ui, Key::Enter, CTRL);
    assert_eq!(r.measures.get_untracked().ok().map(|m| m.len()), Some(3));
    assert!(r.ui.undo_change());
    assert_eq!(r.measures.get_untracked().ok().map(|m| m.len()), Some(2));
}

#[test]
fn a_staff_that_is_not_editable_ignores_edit_keys() {
    let r = staff_rig(false, staff_measures(), NoteStaffOptions::default());
    let before = r.measures.get_untracked().ok();
    for k in ['c', '3', 'r', '+'] {
        assert!(!key(&r.ui, Key::char(k)), "{k}");
    }
    assert!(!key(&r.ui, Key::ArrowUp) && !key(&r.ui, Key::Delete));
    assert_eq!(r.measures.get_untracked().ok(), before);
    assert!(r.ui.history().is_empty() && r.edits.borrow().is_empty());
    assert!(!actions_of(&r.el).contains(&Action::Increment));
}

#[test]
fn double_clicking_the_staff_puts_the_note_at_that_height() {
    // One rest, so the staff has no notes or stems above or below it: the bottom line is a
    // known distance from the top of the element.
    let rest = vec![StaffMeasure::new(vec![StaffEvent::rest(NoteValue::Whole)])];
    let r = staff_rig(true, rest, {
        let mut v_ = NoteStaffOptions::default();
        v_.space = Some(10.0);
        v_.time = None;
        v_
    });
    let b = r.el.bounds().unwrap_or_default();
    let bottom = b.min.y + 10.0 + 10.0 + 40.0;
    let x = b.min.x + 120.0;
    let at_position = |pos: i32| Point::new(x, bottom - pos as f32 * 5.0);
    click(&r.ui, at_position(2));
    click(&r.ui, at_position(2));
    assert_eq!(pitches_at(&r, 0, 0), [p("G4")], "the second line");
    click(&r.ui, at_position(7));
    click(&r.ui, at_position(7));
    assert_eq!(pitches_at(&r, 0, 0), [p("E5")], "the space above the fourth line");
    click(&r.ui, at_position(-2));
    click(&r.ui, at_position(-2));
    assert_eq!(pitches_at(&r, 0, 0), [p("C4")], "below the staff");
    assert!(r.ui.history().iter().all(|h| h.label == "Set note"));
    // A single click only picks.
    let n = r.edits.borrow().len();
    click(&r.ui, at_position(5));
    assert_eq!(r.edits.borrow().len(), n);
}

#[test]
fn editing_waits_for_the_timer_and_survives_an_empty_piece() {
    let r = staff_rig(true, Vec::new(), NoteStaffOptions::default());
    for k in [Key::ArrowUp, Key::Delete, Key::char('c'), Key::char('3')] {
        key(&r.ui, k);
    }
    assert!(r.measures.get_untracked().ok().is_some_and(|m| m.is_empty()));
    let t = tab_rig(true);
    t.measures.set(Vec::new());
    for k in [Key::char('5'), Key::Delete, Key::char('h')] {
        key(&t.ui, k);
    }
    assert!(t.measures.get_untracked().ok().is_some_and(|m| m.is_empty()));
    let _ = Duration::ZERO;
}

#[test]
fn the_staff_speaks_and_names_edits_in_the_windows_language() {
    let r = staff_rig(true, staff_measures(), NoteStaffOptions::default());
    assert_eq!(value_text(&r.el), "measure 1, beat 1: E 4 quarter note");
    let description = || {
        r.el.info()
            .and_then(|i| i.semantics)
            .and_then(|s| s.description_text().map(str::to_owned))
            .unwrap_or_default()
    };
    assert!(description().starts_with("treble clef, no sharps or flats, "), "{}", description());
    r.ui.set_locale("de");
    let _ = frame(&r.ui);
    assert_eq!(value_text(&r.el), "Takt 1, Schlag 1: E 4, Viertelnote");
    assert!(description().starts_with("Violinschlüssel, keine Vorzeichen, "), "{}", description());
    key(&r.ui, Key::ArrowUp);
    assert_eq!(r.edits.borrow().last().map(|e| e.label.clone()).as_deref(), Some("Note nach oben"));
    assert_eq!(r.ui.history().last().map(|h| h.label.clone()).as_deref(), Some("Note nach oben"));
}
