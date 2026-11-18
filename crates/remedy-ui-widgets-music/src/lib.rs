//! RemedyUI's music widgets, for instrument apps.
//!
//! - [`fretboard`]: a neck with any tuning and any number of strings and frets, wires spaced
//!   by the real twelfth-root-of-two rule ([`fret_position`]), inlay dots, marked notes, a
//!   sounding-pitch ring for a tuner, and a position people pick by click or keyboard
//! - [`chord_diagram`]: the box of strings and frets with fingers, barres, open and muted
//!   strings and a base-fret label ([`ChordShape`])
//! - [`tab_staff`]: tablature for any number of strings, with measures, rhythm and techniques
//!   (hammer-on, pull-off, slides, bends, vibrato, harmonics, palm mute, dead notes, ties);
//!   [`tab_staff_editor`] edits it by keyboard and pointer
//! - [`note_staff`]: standard notation with treble, bass, alto and tenor clefs, key and time
//!   signatures, notes placed by [`Pitch`] with ledger lines and the accidentals each needs,
//!   note values with beams that follow the notes' contour ([`beam_line`]), dots, ties and
//!   rests; [`note_staff_editor`] edits it by keyboard and pointer. Every edit is recorded in
//!   the window's undo history and reported to the app
//!
//! They are drawn on the canvas from theme colors, repaint only when a `Signal` they read
//! changes (a tuner can drive a marked pitch many times a second), take the keyboard as one tab
//! stop with a cursor moved by the arrow keys, and describe the cursor's string, fret, note and
//! duration to screen readers. Instruments and notation are read the way they are built, so
//! none of these mirror in right-to-left windows. See `docs/design/15-widgets-and-media.md`.

mod chord;
mod fretboard;
mod glyphs;
mod layout;
mod note_staff;
mod pitch;
pub mod staff_edit;
mod tab;
pub mod tab_edit;

pub use chord::*;
pub use fretboard::*;
pub use glyphs::{Clef, NoteValue};
pub use layout::*;
pub use note_staff::*;
pub use pitch::*;
pub use staff_edit::{StaffEdit, StaffEditHandler, StaffEditing};
pub use tab::*;
pub use tab_edit::{TabEdit, TabEditHandler, TabEditing};
