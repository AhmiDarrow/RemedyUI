//! The music math: pitches, fret spacing, staff positions, accidentals, beams, line breaking
//! and how things are said aloud.

use remedy_ui_widgets_music::*;

fn p(s: &str) -> Pitch {
    Pitch::parse(s).unwrap_or_else(|| unreachable!("{s}"))
}

#[test]
fn pitches_have_midi_numbers_names_and_spoken_forms() {
    assert_eq!(p("C4").midi(), 60);
    assert_eq!(p("A4").midi(), 69);
    assert_eq!(p("E2").midi(), 40, "the low E of a guitar");
    assert_eq!(p("C-1").midi(), 0);
    assert_eq!(p("F#3").midi(), 54);
    assert_eq!(p("Bb2").midi(), 46);
    assert_eq!(p("Ebb5").midi(), p("D5").midi());
    assert_eq!(p("G##1").midi(), p("A1").midi());
    assert_eq!(p("B#3").midi(), p("C4").midi(), "enharmonic spellings share a number");
    assert_eq!(p("F\u{266f}4"), p("F#4"));
    assert_eq!(Pitch::parse("H4"), None);
    assert_eq!(Pitch::parse("C"), None);
    assert_eq!(Pitch::parse("C#x4"), None);
    assert_eq!(Pitch::parse(""), None);
    assert_eq!(p("F#3").name(), "F#3");
    assert_eq!(p("Bb2").letter_name(), "Bb");
    assert_eq!(p("C#4").spoken(), "C sharp 4");
    assert_eq!(p("Bb2").spoken(), "B flat 2");
    assert_eq!(p("Ebb5").spoken(), "E double flat 5");
    assert_eq!(p("C4").spoken(), "C 4");
    assert_eq!(p("D#2").to_string(), "D#2");
}

#[test]
fn midi_numbers_spell_with_sharps_or_flats_and_round_trip() {
    for midi in 0..128 {
        for flats in [false, true] {
            assert_eq!(Pitch::from_midi(midi, flats).midi(), midi);
        }
    }
    assert_eq!(Pitch::from_midi(61, false).name(), "C#4");
    assert_eq!(Pitch::from_midi(61, true).name(), "Db4");
    assert_eq!(Pitch::from_midi(59, true).name(), "B3");
    assert_eq!(Pitch::from_midi(-1, false).name(), "B-2");
}

#[test]
fn fret_wires_follow_the_twelfth_root_of_two() {
    assert_eq!(fret_position(0), 0.0);
    assert!((fret_position(12) - 0.5).abs() < 1e-12, "the octave is half the string");
    assert!((fret_position(24) - 0.75).abs() < 1e-12);
    assert!((fret_position(7) - (1.0 - 2f64.powf(-7.0 / 12.0))).abs() < 1e-12);
    // Each fret is shorter than the one before by the same ratio.
    let ratio = 2f64.powf(-1.0 / 12.0);
    for n in 1..60 {
        let this = fret_position(n + 1) - fret_position(n);
        let before = fret_position(n) - fret_position(n - 1);
        assert!((this / before - ratio).abs() < 1e-9, "fret {n}");
    }
}

#[test]
fn the_fretboard_layout_spaces_wires_by_that_rule_and_hits_positions() {
    let options = {
        let mut v_ = FretboardOptions::default();
        v_.frets = 24;
        v_
    };
    let g = FretboardLayout::new(&options, 960.0, 4.0);
    // The wires' spacing shrinks by the same ratio all the way up.
    let w = |n: u32| g.wire_x(n + 1) - g.wire_x(n);
    for n in 0..23 {
        assert!((w(n + 1) / w(n) - 2f64.powf(-1.0 / 12.0) as f32).abs() < 1e-3, "fret {n}");
    }
    // The 12th wire is where half the playable length is gone.
    let span = g.wire_x(24) - g.wire_x(0);
    assert!(((g.wire_x(12) - g.wire_x(0)) / span - (0.5 / 0.75) as f32).abs() < 1e-4);
    // Every position is found at its own center, the open strings at the nut.
    for string in 0..6 {
        for fret in 0..=24 {
            let at = remedy_ui_core::Point::new(g.cell_x(fret), g.string_y(string));
            assert_eq!(g.hit(at), Some(FretPos { string, fret }));
        }
    }
    assert_eq!(g.fret_range(), (0, 24));
    // Off the neck is nothing.
    assert_eq!(g.hit(remedy_ui_core::Point::new(g.cell_x(5), -50.0)), None);
    assert_eq!(g.hit(remedy_ui_core::Point::new(g.cell_x(5), g.height + 50.0)), None);
    assert_eq!(g.hit(remedy_ui_core::Point::new(g.width + 50.0, g.string_y(2))), None);
    // Strings: the lowest is at the bottom.
    assert!(g.string_y(0) > g.string_y(5));
}

#[test]
fn a_window_of_frets_and_left_handed_mirroring() {
    let options = {
        let mut v_ = FretboardOptions::default();
        v_.first_fret = 5;
        v_.frets = 7;
        v_
    };
    let g = FretboardLayout::new(&options, 700.0, 4.0);
    assert_eq!(g.fret_range(), (6, 12), "frets after the fifth wire");
    assert!(g.wire_x(5) < g.wire_x(6) && g.wire_x(6) < g.wire_x(12));
    let at = remedy_ui_core::Point::new(g.cell_x(6), g.string_y(0));
    assert_eq!(g.hit(at), Some(FretPos { string: 0, fret: 6 }));
    let lh = FretboardLayout::new(
        &{
            let mut v_ = options.clone();
            v_.left_handed = true;
            v_
        },
        700.0,
        4.0,
    );
    for n in 5..=12 {
        assert!((lh.wire_x(n) - (700.0 - g.wire_x(n))).abs() < 1e-3, "wire {n} mirrored");
    }
    assert_eq!(
        lh.hit(remedy_ui_core::Point::new(lh.cell_x(9), lh.string_y(3))),
        Some(FretPos { string: 3, fret: 9 })
    );
}

#[test]
fn any_tuning_and_string_count_lays_out_and_describes() {
    let seven = {
        let mut v_ = FretboardOptions::default();
        v_.tuning = vec![35, 40, 45, 50, 55, 59, 64];
        v_
    };
    let g = FretboardLayout::new(&seven, 800.0, 4.0);
    assert_eq!(g.strings, 7);
    assert!(g.height > FretboardLayout::new(&FretboardOptions::default(), 800.0, 4.0).height);
    assert_eq!(describe_position(&seven, FretPos { string: 0, fret: 0 }), "string 7 (B 1), open, B 1");
    assert_eq!(describe_position(&seven, FretPos { string: 6, fret: 5 }), "string 1 (E 4), fret 5, A 4");
    let single = {
        let mut v_ = FretboardOptions::default();
        v_.tuning = vec![64];
        v_
    };
    assert_eq!(FretboardLayout::new(&single, 400.0, 4.0).strings, 1);
    let flats = {
        let mut v_ = FretboardOptions::default();
        v_.flats = true;
        v_
    };
    assert_eq!(describe_position(&flats, FretPos { string: 5, fret: 6 }), "string 1 (E 4), fret 6, B flat 4");
    // Beyond the strings there is no note to say.
    assert_eq!(describe_position(&flats, FretPos { string: 9, fret: 1 }), "string 0, fret 1");
}

#[test]
fn inlays_follow_the_standard_pattern_for_any_fret() {
    let std = Inlays::Standard;
    let dots: Vec<u8> = (0..=25).map(|f| std.dots_at(f)).collect();
    assert_eq!(dots.iter().filter(|d| **d == 1).count(), 8, "3 5 7 9 and 15 17 19 21");
    assert_eq!((std.dots_at(12), std.dots_at(24), std.dots_at(0)), (2, 2, 0));
    assert_eq!((std.dots_at(3), std.dots_at(15), std.dots_at(4)), (1, 1, 0));
    assert_eq!(std.dots_at(120), 2);
    let ukulele = Inlays::Custom { single: vec![5, 7, 10], double: vec![12] };
    assert_eq!((ukulele.dots_at(5), ukulele.dots_at(12), ukulele.dots_at(3)), (1, 2, 0));
    assert_eq!(Inlays::None.dots_at(12), 0);
}

#[test]
fn staff_positions_count_lines_and_spaces_from_the_bottom_line() {
    use Clef::*;
    assert_eq!(staff_position(p("E4"), Treble), 0, "the bottom line");
    assert_eq!(staff_position(p("G4"), Treble), 2);
    assert_eq!(staff_position(p("F5"), Treble), 8, "the top line");
    assert_eq!(staff_position(p("C4"), Treble), -2, "middle C hangs on a ledger line");
    assert_eq!(staff_position(p("A5"), Treble), 10);
    assert_eq!(staff_position(p("G2"), Bass), 0);
    assert_eq!(staff_position(p("A3"), Bass), 8);
    assert_eq!(staff_position(p("C4"), Bass), 10, "middle C is a ledger line above");
    assert_eq!(staff_position(p("C4"), Alto), 4, "middle C on the middle line");
    assert_eq!(staff_position(p("C4"), Tenor), 6, "middle C on the fourth line");
    // A sharp or flat does not move a note on the staff.
    assert_eq!(staff_position(p("F#4"), Treble), staff_position(p("F4"), Treble));
}

#[test]
fn key_signatures_alter_the_right_letters() {
    assert_eq!(key_alter(Step::F, 1), 1);
    assert_eq!(key_alter(Step::C, 1), 0);
    assert_eq!(key_alter(Step::C, 2), 1);
    assert_eq!(key_alter(Step::B, 7), 1);
    assert_eq!(key_alter(Step::B, -1), -1);
    assert_eq!(key_alter(Step::E, -1), 0);
    assert_eq!(key_alter(Step::F, -7), -1);
    assert_eq!(key_alter(Step::G, 0), 0);
    // Where the signs sit on each clef.
    assert_eq!(key_positions(3, Clef::Treble), [8, 5, 9]);
    assert_eq!(key_positions(-3, Clef::Treble), [4, 7, 3]);
    assert_eq!(key_positions(2, Clef::Bass), [6, 3]);
    assert_eq!(key_positions(-2, Clef::Bass), [2, 5]);
    assert_eq!(key_positions(1, Clef::Alto), [7]);
    assert!(key_positions(0, Clef::Treble).is_empty());
    assert_eq!(key_positions(7, Clef::Treble).len(), 7);
    for clef in [Clef::Treble, Clef::Bass, Clef::Alto, Clef::Tenor] {
        for k in -7..=7 {
            assert!(key_positions(k, clef).iter().all(|q| (-2..=9).contains(q)), "{clef:?} {k}");
        }
    }
}

#[test]
fn accidentals_are_written_when_a_note_differs_from_the_key_and_the_measure_so_far() {
    use NoteValue::Quarter as Q;
    let m = StaffMeasure::new(vec![
        StaffEvent::note(p("F#4"), Q),
        StaffEvent::note(p("F#4"), Q),
        StaffEvent::note(p("F4"), Q),
        StaffEvent::note(p("F#5"), Q),
        StaffEvent::note(p("F4"), Q),
    ]);
    // In the key of G (one sharp, F#): the first F# needs nothing, F natural needs a natural,
    // and it stays natural for the rest of the measure in that octave.
    let a = accidentals(&m, 1);
    let flat: Vec<Option<i8>> = a.iter().map(|v| v.first().copied().flatten()).collect();
    assert_eq!(flat, [None, None, Some(0), None, None]);
    // In C major every F# is written the first time, then remembered.
    let a = accidentals(&m, 0);
    let flat: Vec<Option<i8>> = a.iter().map(|v| v.first().copied().flatten()).collect();
    assert_eq!(flat, [Some(1), None, Some(0), Some(1), None]);
    // A new measure starts over.
    let again = accidentals(&StaffMeasure::new(vec![StaffEvent::note(p("F#4"), Q)]), 0);
    assert_eq!(again, [[Some(1)]]);
    // Chords: each note by itself.
    let chord = StaffMeasure::new(vec![StaffEvent::chord(vec![p("C4"), p("E4"), p("Bb4")], Q)]);
    assert_eq!(accidentals(&chord, 0), [[None, None, Some(-1)]]);
    assert_eq!(accidentals(&StaffMeasure::new(vec![StaffEvent::rest(Q)]), 3), [Vec::<Option<i8>>::new()]);
}

#[test]
fn ledger_lines_run_from_the_staff_out_to_the_note() {
    assert!(
        ledger_positions(0).is_empty() && ledger_positions(8).is_empty() && ledger_positions(5).is_empty()
    );
    assert_eq!(ledger_positions(-2), [-2], "middle C");
    assert!(ledger_positions(-1).is_empty(), "the space under the staff needs no line");
    assert_eq!(ledger_positions(-3), [-2], "the space under middle C's line");
    assert_eq!(ledger_positions(-4), [-4, -2]);
    assert_eq!(ledger_positions(-5), [-4, -2]);
    assert_eq!(ledger_positions(10), [10]);
    assert_eq!(ledger_positions(9), Vec::<i32>::new(), "a note in the first space above needs none");
    assert_eq!(ledger_positions(11), [10]);
    assert_eq!(ledger_positions(14), [10, 12, 14]);
}

#[test]
fn stems_point_away_from_the_middle_line() {
    assert!(stem_up(&[0]) && stem_up(&[3]));
    assert!(!stem_up(&[5]) && !stem_up(&[8]));
    assert!(stem_up(&[4]), "the middle line takes a stem up");
    assert!(stem_up(&[0, 5]), "a chord goes by its farthest note");
    assert!(!stem_up(&[3, 8]));
    assert!(stem_up(&[]));
}

#[test]
fn eighth_notes_beam_within_a_beat_and_rests_break_beams() {
    use NoteValue::*;
    let n = |v| StaffEvent::note(p("C5"), v);
    let m = [n(Eighth), n(Eighth), n(Eighth), n(Eighth), n(Quarter), n(Eighth), StaffEvent::rest(Eighth)];
    assert_eq!(beam_groups(&m, (4, 4)), [0..2, 2..4], "two beats of two");
    // In 6/8 they group in threes.
    let six: Vec<StaffEvent> = (0..6).map(|_| n(Eighth)).collect();
    assert_eq!(beam_groups(&six, (6, 8)), [0..3, 3..6]);
    assert_eq!(beam_groups(&six, (2, 4)), [0..2, 2..4, 4..6]);
    // A rest in the middle splits a group; a lone note has no beam.
    let rest = [n(Eighth), StaffEvent::rest(Eighth), n(Eighth), n(Eighth)];
    assert_eq!(beam_groups(&rest, (4, 4)), vec![2..4], "the lone note before the rest has none");
    // Sixteenths: four in a beat.
    let sixteenths: Vec<StaffEvent> = (0..8).map(|_| n(Sixteenth)).collect();
    assert_eq!(beam_groups(&sixteenths, (4, 4)), [0..4, 4..8]);
    assert_eq!(beat_quarters((4, 4)), 1.0);
    assert_eq!(beat_quarters((6, 8)), 1.5);
    assert_eq!(beat_quarters((3, 8)), 0.5);
    assert_eq!(beat_quarters((2, 2)), 2.0);
}

#[test]
fn note_values_know_their_lengths_flags_and_heads() {
    use NoteValue::*;
    assert_eq!(Whole.quarters(0), 4.0);
    assert_eq!(Half.quarters(0), 2.0);
    assert_eq!(Quarter.quarters(0), 1.0);
    assert_eq!(Eighth.quarters(0), 0.5);
    assert_eq!(Quarter.quarters(1), 1.5);
    assert_eq!(Quarter.quarters(2), 1.75);
    assert_eq!(Half.quarters(1), 3.0);
    assert_eq!(ThirtySecond.quarters(0), 0.125);
    assert_eq!(
        [Whole, Half, Quarter, Eighth, Sixteenth, ThirtySecond].map(NoteValue::flags),
        [0, 0, 0, 1, 2, 3]
    );
    assert!(!Whole.has_stem() && Half.has_stem());
    assert!(!Half.filled() && Quarter.filled());
    assert_eq!(Eighth.name(), "eighth");
    let dotted = StaffEvent::note(p("C4"), Quarter).dotted(1);
    assert_eq!(dotted.quarters(), 1.5);
    assert!(StaffEvent::rest(Half).is_rest() && !dotted.is_rest());
}

#[test]
fn measures_are_broken_into_lines_that_fit() {
    let widths = [100.0, 100.0, 100.0, 100.0, 100.0];
    // 60 for the clef; the first line also spends 30 on a time signature.
    assert_eq!(break_systems(&widths, 60.0, 30.0, 400.0), [0..3, 3..5]);
    assert_eq!(break_systems(&widths, 60.0, 0.0, 400.0), [0..3, 3..5]);
    assert_eq!(break_systems(&widths, 60.0, 0.0, 700.0), vec![0..5]);
    // A measure wider than the line still gets a line of its own.
    assert_eq!(break_systems(&[500.0, 50.0], 60.0, 0.0, 300.0), [0..1, 1..2]);
    assert_eq!(break_systems(&[], 60.0, 0.0, 300.0), Vec::<std::ops::Range<usize>>::new());
    // Every measure is in exactly one line, in order.
    let many: Vec<f32> = (0..50).map(|i| 40.0 + (i % 7) as f32 * 13.0).collect();
    let lines = break_systems(&many, 30.0, 20.0, 500.0);
    assert_eq!(lines.first().map(|r| r.start), Some(0));
    assert_eq!(lines.last().map(|r| r.end), Some(50));
    assert!(lines.windows(2).all(|w| w.first().map(|a| a.end) == w.get(1).map(|b| b.start)));
    for (i, l) in lines.iter().enumerate() {
        let used: f32 =
            30.0 + if i == 0 { 20.0 } else { 0.0 } + many.get(l.clone()).unwrap_or(&[]).iter().sum::<f32>();
        assert!(used <= 500.0 || l.len() == 1, "line {i} fits");
    }
    // Full lines stretch to the edges; the last one keeps its natural width.
    assert!((stretch(300.0, 60.0, 450.0, false) - 1.625).abs() < 1e-6);
    assert_eq!(stretch(300.0, 60.0, 450.0, true), 1.0);
    assert_eq!(stretch(460.0, 60.0, 450.0, false), 1.0, "never squeezed");
}

#[test]
fn the_cursor_steps_through_events_and_measures() {
    let counts = [3, 0, 2, 1];
    assert_eq!(step_event(&counts, (0, 0), true), Some((0, 1)));
    assert_eq!(step_event(&counts, (0, 2), true), Some((2, 0)), "empty measures are skipped");
    assert_eq!(step_event(&counts, (2, 1), true), Some((3, 0)));
    assert_eq!(step_event(&counts, (3, 0), true), None);
    assert_eq!(step_event(&counts, (2, 0), false), Some((0, 2)));
    assert_eq!(step_event(&counts, (0, 0), false), None);
    assert_eq!(step_measure(&counts, (0, 1), true), Some((2, 0)));
    assert_eq!(step_measure(&counts, (3, 0), false), Some((2, 0)));
    assert_eq!(step_measure(&counts, (3, 0), true), None);
    assert_eq!(ends(&counts), Some(((0, 0), (3, 0))));
    assert_eq!(ends(&[0, 0]), None);
    assert_eq!(ends(&[]), None);
}

#[test]
fn events_are_described_for_screen_readers() {
    use NoteValue::*;
    assert_eq!(describe_event(&StaffEvent::note(p("C#4"), Quarter)), "C sharp 4 quarter note");
    assert_eq!(
        describe_event(&StaffEvent::note(p("Bb3"), Eighth).dotted(1).tied()),
        "B flat 3 dotted eighth note, tied to the next"
    );
    assert_eq!(describe_event(&StaffEvent::rest(Half)), "half rest");
    assert_eq!(describe_event(&StaffEvent::rest(Quarter).dotted(2)), "double dotted quarter rest");
    assert_eq!(describe_event(&StaffEvent::chord(vec![p("C4"), p("E4")], Whole)), "whole chord of C 4, E 4");
    let ms = vec![
        StaffMeasure::new(vec![
            StaffEvent::note(p("C4"), Quarter),
            StaffEvent::note(p("D4"), Eighth),
            StaffEvent::note(p("E4"), Eighth),
        ]),
        StaffMeasure {
            events: vec![StaffEvent::note(p("C4"), Half), StaffEvent::note(p("D4"), Half)],
            time: Some((6, 8)),
        },
    ];
    assert_eq!(beat_of(&ms, Some((4, 4)), StaffCursor { measure: 0, event: 0 }), "1");
    assert_eq!(beat_of(&ms, Some((4, 4)), StaffCursor { measure: 0, event: 2 }), "2.5");
    assert_eq!(
        beat_of(&ms, Some((4, 4)), StaffCursor { measure: 1, event: 1 }),
        "2.33",
        "in 6/8 a beat is a dotted quarter"
    );
}

#[test]
fn tab_notes_and_techniques_are_written_and_said() {
    use NoteValue::*;
    assert_eq!(fret_text(&TabNote::new(0, 5)), "5");
    assert_eq!(fret_text(&TabNote::new(0, 12).with(Technique::Harmonic)), "<12>");
    assert_eq!(fret_text(&TabNote::new(0, 3).with(Technique::Dead)), "x");
    assert_eq!(bend_text(1), "1/2");
    assert_eq!(bend_text(2), "full");
    assert_eq!(bend_text(3), "1 1/2");
    assert_eq!(bend_text(4), "2");
    assert_eq!(bend_text(0), "");
    assert!(Technique::HammerOn.connects() && Technique::SlideDown.connects() && Technique::Tie.connects());
    assert!(!Technique::Vibrato.connects() && !Technique::Bend(2).connects());
    assert_eq!(Technique::Bend(2).spoken(), "bend a whole step");
    assert_eq!(Technique::Bend(1).spoken(), "bend a half step");
    assert_eq!(Technique::PullOff.spoken(), "pull-off");
    let e = TabEvent::new(vec![TabNote::new(2, 5).with(Technique::HammerOn), TabNote::new(0, 0)], Eighth);
    assert_eq!(describe_tab_note(&e, 2, 6), "string 4: fret 5, hammer-on");
    assert_eq!(describe_tab_note(&e, 0, 6), "string 6: open");
    assert_eq!(describe_tab_note(&e, 4, 6), "string 2: no note");
    assert_eq!(describe_tab_note(&TabEvent::rest(Quarter), 1, 6), "quarter rest");
    assert_eq!(
        describe_tab_note(&TabEvent::new(vec![TabNote::new(1, 3).with(Technique::Dead)], Quarter), 1, 6),
        "string 5: dead note"
    );
    let ms = vec![TabMeasure::new(vec![
        TabEvent::new(vec![], Quarter),
        TabEvent::new(vec![], Eighth),
        TabEvent::new(vec![], Eighth),
    ])];
    assert_eq!(beat_in(&ms, Some((4, 4)), 0, 2), "2.5");
}

#[test]
fn chords_are_described_string_by_string() {
    use StringState::*;
    let am = ChordShape::new(
        Some("Am"),
        vec![
            Muted,
            Open,
            Fretted { fret: 2, finger: Some(2) },
            Fretted { fret: 2, finger: Some(3) },
            Fretted { fret: 1, finger: Some(1) },
            Open,
        ],
    );
    assert_eq!(am.base_fret, 1);
    assert_eq!(am.row_count(), 4);
    assert_eq!(
        describe_chord(&am),
        "string 6 muted, string 5 open, string 4 fret 2 finger 2, string 3 fret 2 finger 3, string 2 fret 1 finger 1, string 1 open"
    );
    let barre = ChordShape::new(
        Some("F#m"),
        vec![
            Fretted { fret: 2, finger: Some(1) },
            Fretted { fret: 4, finger: None },
            Fretted { fret: 4, finger: None },
            Fretted { fret: 2, finger: Some(1) },
            Fretted { fret: 2, finger: Some(1) },
            Fretted { fret: 2, finger: Some(1) },
        ],
    )
    .with_barre(Barre { fret: 2, from: 0, to: 5, finger: Some(1) });
    assert_eq!(barre.base_fret, 1, "all within the first four frets");
    assert!(describe_chord(&barre).ends_with("barre at fret 2 across strings 6 to 1 with finger 1"));
    // A chord up the neck starts its diagram at its lowest fret and says so.
    let high = ChordShape::new(
        None,
        vec![
            Muted,
            Fretted { fret: 7, finger: Some(1) },
            Fretted { fret: 9, finger: Some(3) },
            Fretted { fret: 9, finger: Some(4) },
            Fretted { fret: 8, finger: Some(2) },
            Fretted { fret: 7, finger: Some(1) },
        ],
    );
    assert_eq!(high.base_fret, 7);
    assert!(describe_chord(&high).ends_with("diagram starts at fret 7"));
    assert_eq!(high.row_count(), 4);
    assert_eq!(high.clone().with_base_fret(5).row_count(), 5, "the lower top row needs another row");
    assert_eq!(ChordShape { rows: Some(6), ..high }.row_count(), 6);
    // Other string counts work (a ukulele).
    let uke = ChordShape::new(Some("C"), vec![Open, Open, Open, Fretted { fret: 3, finger: Some(3) }]);
    assert!(describe_chord(&uke).starts_with("string 4 open"));
    let (w6, _) = diagram_size(&am, &ChordOptions::default(), 4.0);
    let (w4, _) = diagram_size(&uke, &ChordOptions::default(), 4.0);
    assert!(w4 < w6, "fewer strings, narrower diagram");
    let (w2x, h2x) = diagram_size(
        &am,
        &{
            let mut v_ = ChordOptions::default();
            v_.scale = 2.0;
            v_
        },
        4.0,
    );
    let (w1x, h1x) = diagram_size(&am, &ChordOptions::default(), 4.0);
    assert!((w2x / w1x - 2.0).abs() < 1e-3 && (h2x / h1x - 2.0).abs() < 1e-3);
}

#[test]
fn every_locale_has_every_message_with_its_plurals() {
    let problems = remedy_ui_view::strings::check(&remedy_ui_widgets_music::STRINGS);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn notes_are_said_the_way_musicians_of_each_language_say_them() {
    use NoteValue::*;
    assert_eq!(p("C#4").spoken_in("de"), "Cis 4");
    assert_eq!(p("Bb2").spoken_in("de"), "B 2");
    assert_eq!(p("B3").spoken_in("de-AT"), "H 3");
    assert_eq!(p("Bb2").spoken_in("fr"), "si bémol 2");
    assert_eq!(p("F#3").spoken_in("ja"), "Fシャープ 3");
    assert_eq!(describe_event_in(&StaffEvent::note(p("C#4"), Quarter), "de"), "Cis 4, Viertelnote");
    assert_eq!(describe_event_in(&StaffEvent::rest(Quarter).dotted(1), "fr"), "soupir pointé");
    assert_eq!(describe_event_in(&StaffEvent::rest(Half).dotted(1), "fr"), "demi-pause pointée");
    assert_eq!(
        describe_event_in(&StaffEvent::chord(vec![p("C4"), p("E4")], Whole), "zh-Hans"),
        "C 4、E 4 和弦，全音符"
    );
    assert_eq!(Technique::Bend(3).spoken(), "bend 1.5 steps");
    assert_eq!(Technique::Bend(4).spoken_in("ru"), "подтяжка на 2 тона");
    let e = TabEvent::new(vec![TabNote::new(2, 5).with(Technique::HammerOn)], Quarter);
    assert_eq!(describe_tab_note_in(&e, 2, 6, "es"), "cuerda 4: traste 5, hammer-on");
    let am = ChordShape::new(Some("Am"), vec![StringState::Muted, StringState::Open]);
    assert_eq!(describe_chord_in(&am, "de"), "Saite 2 nicht gespielt, Saite 1 leer");
}

#[test]
fn a_chord_with_string_names_is_taller_and_keeps_its_roots_and_labels() {
    let plain = ChordShape::new(
        Some("C"),
        vec![
            StringState::Muted,
            StringState::Fretted { fret: 3, finger: Some(3) },
            StringState::Fretted { fret: 2, finger: Some(2) },
            StringState::Open,
            StringState::Fretted { fret: 1, finger: Some(1) },
            StringState::Open,
        ],
    );
    let named = plain
        .clone()
        .with_roots(vec![1, 4])
        .with_labels(vec![None, Some("R".into()), Some("3".into()), None, Some("R".into())])
        .with_string_names(["E", "A", "D", "G", "B", "E"].map(String::from).to_vec());
    let o = ChordOptions::default();
    let (w0, h0) = diagram_size(&plain, &o, 4.0);
    let (w1, h1) = diagram_size(&named, &o, 4.0);
    assert!(w1 == w0 && (h1 - h0 - 16.0).abs() < 1e-3, "a row of names: {h0} -> {h1}");
    assert_eq!(named.roots, vec![1, 4]);
}
