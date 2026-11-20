//! Pitches: names, MIDI numbers and how they are said aloud.

use std::fmt;

/// The seven note letters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
// Exhaustive on purpose: the seven note letters.
pub enum Step {
    /// C
    C,
    /// D
    D,
    /// E
    E,
    /// F
    F,
    /// G
    G,
    /// A
    A,
    /// B
    B,
}

impl Step {
    /// All steps from C up.
    pub const ALL: [Step; 7] = [Step::C, Step::D, Step::E, Step::F, Step::G, Step::A, Step::B];

    /// 0 for C up to 6 for B.
    pub fn index(self) -> i32 {
        match self {
            Step::C => 0,
            Step::D => 1,
            Step::E => 2,
            Step::F => 3,
            Step::G => 4,
            Step::A => 5,
            Step::B => 6,
        }
    }

    /// The step with this index (counted from C, wrapping).
    pub fn from_index(i: i32) -> Step {
        match i.rem_euclid(7) {
            0 => Step::C,
            1 => Step::D,
            2 => Step::E,
            3 => Step::F,
            4 => Step::G,
            5 => Step::A,
            _ => Step::B,
        }
    }

    /// Semitones above C.
    pub fn semitone(self) -> i32 {
        match self {
            Step::C => 0,
            Step::D => 2,
            Step::E => 4,
            Step::F => 5,
            Step::G => 7,
            Step::A => 9,
            Step::B => 11,
        }
    }

    /// The letter.
    pub fn letter(self) -> char {
        match self {
            Step::C => 'C',
            Step::D => 'D',
            Step::E => 'E',
            Step::F => 'F',
            Step::G => 'G',
            Step::A => 'A',
            Step::B => 'B',
        }
    }

    fn from_letter(c: char) -> Option<Step> {
        Step::ALL.into_iter().find(|s| s.letter() == c.to_ascii_uppercase())
    }
}

/// A note with its octave. Octave 4 is the one that holds middle C (C4, MIDI 60).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pitch {
    /// The letter.
    pub step: Step,
    /// Semitones sharp (positive) or flat (negative): -2 to 2 are drawn.
    pub alter: i8,
    /// The octave.
    pub octave: i8,
}

impl Pitch {
    /// A pitch.
    pub const fn new(step: Step, alter: i8, octave: i8) -> Pitch {
        Pitch { step, alter, octave }
    }

    /// The MIDI note number (middle C is 60).
    pub fn midi(self) -> i32 {
        (i32::from(self.octave) + 1) * 12 + self.step.semitone() + i32::from(self.alter)
    }

    /// The pitch for a MIDI note number, spelled with sharps or with flats.
    pub fn from_midi(midi: i32, prefer_flats: bool) -> Pitch {
        let octave = midi.div_euclid(12) - 1;
        let pc = midi.rem_euclid(12);
        let (step, alter) = match (pc, prefer_flats) {
            (0, _) => (Step::C, 0),
            (1, false) => (Step::C, 1),
            (1, true) => (Step::D, -1),
            (2, _) => (Step::D, 0),
            (3, false) => (Step::D, 1),
            (3, true) => (Step::E, -1),
            (4, _) => (Step::E, 0),
            (5, _) => (Step::F, 0),
            (6, false) => (Step::F, 1),
            (6, true) => (Step::G, -1),
            (7, _) => (Step::G, 0),
            (8, false) => (Step::G, 1),
            (8, true) => (Step::A, -1),
            (9, _) => (Step::A, 0),
            (10, false) => (Step::A, 1),
            (10, true) => (Step::B, -1),
            _ => (Step::B, 0),
        };
        Pitch { step, alter, octave: octave.clamp(-128, 127) as i8 }
    }

    /// Steps counted up the staff: C0 is 0, D0 is 1, C1 is 7. Staff positions use this.
    pub fn diatonic(self) -> i32 {
        i32::from(self.octave) * 7 + self.step.index()
    }

    /// Parses "C4", "F#3", "Bb2", "Ebb5" or "G##1"; the octave is required.
    pub fn parse(text: &str) -> Option<Pitch> {
        let mut chars = text.trim().chars();
        let step = Step::from_letter(chars.next()?)?;
        let rest: String = chars.collect();
        let digits_at = rest.find(|c: char| c.is_ascii_digit() || c == '-')?;
        let (marks, octave) = rest.split_at(digits_at);
        let mut alter = 0i8;
        for c in marks.chars() {
            match c {
                '#' | '\u{266f}' => alter += 1,
                'b' | '\u{266d}' => alter -= 1,
                _ => return None,
            }
        }
        Some(Pitch { step, alter, octave: octave.parse().ok()? })
    }

    /// The note name in compact text: `C4`, `F#3`, `Bb2`.
    pub fn name(self) -> String {
        format!("{}{}", self.letter_name(), self.octave)
    }

    /// The letter and accidental without the octave: `C`, `F#`, `Bb`.
    pub fn letter_name(self) -> String {
        let mut s = String::new();
        s.push(self.step.letter());
        match self.alter {
            0 => {}
            a if a > 0 => s.push_str(&"#".repeat(usize::from(a.unsigned_abs()))),
            a => s.push_str(&"b".repeat(usize::from(a.unsigned_abs()))),
        }
        s
    }

    /// The note as it is said: "C sharp 4", "B flat 2".
    pub fn spoken(self) -> String {
        self.spoken_in("en")
    }

    /// [`Pitch::spoken`] in `locale` (BCP 47), with RemedyUI's built-in words; English where
    /// it has none. The letters stay letters.
    pub fn spoken_in(self, locale: &str) -> String {
        use remedy_ui_view::words::word_or;
        let template = match self.alter {
            0 => word_or(locale, "{letter} {octave}"),
            1 => word_or(locale, "{letter} sharp {octave}"),
            2 => word_or(locale, "{letter} double sharp {octave}"),
            -1 => word_or(locale, "{letter} flat {octave}"),
            -2 => word_or(locale, "{letter} double flat {octave}"),
            a if a > 0 => word_or(locale, "{letter} sharp {octave}"),
            _ => word_or(locale, "{letter} flat {octave}"),
        };
        remedy_ui_view::words::fill(template, &[
            ("letter", &self.step.letter().to_string()),
            ("octave", &self.octave.to_string()),
        ])
    }
}

impl fmt::Display for Pitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

/// Standard guitar tuning, lowest string first: E2 A2 D3 G3 B3 E4 (MIDI 40 45 50 55 59 64).
pub const GUITAR: [i32; 6] = [40, 45, 50, 55, 59, 64];
/// Standard bass guitar tuning, lowest string first: E1 A1 D2 G2.
pub const BASS: [i32; 4] = [28, 33, 38, 43];
/// Standard ukulele (re-entrant) tuning, first string last: G4 C4 E4 A4.
pub const UKULELE: [i32; 4] = [67, 60, 64, 69];
