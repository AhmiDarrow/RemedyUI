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

use remedy_ui_engine::view::strings::StrArg;

/// The pack's own words (what screen readers hear about notes, chords, strings and frets,
/// and the undo history's names for edits) in every language the engine ships, from
/// `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_engine::view::strings::add_messages`] under the name `widgets-music`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("widgets-music");

/// Message `id` with placeholder values in `locale` (not tracked: widgets pass
/// `ui.locale()`, which is). What this pack says is read by screen readers or names an undo
/// step, and its placeholders are numbers, note letters and its own phrases, so the
/// direction-isolation marks around them, which only matter on screen, are left out.
pub(crate) fn say(locale: &str, id: &str, args: &[(&str, StrArg)]) -> String {
    STRINGS.format_in(locale, id, args).replace(['\u{2068}', '\u{2069}'], "")
}

/// Items said as a list in `locale` ("C 4, E 4").
pub(crate) fn list(locale: &str, items: &[String]) -> String {
    items.join(&say(locale, "list-separator", &[]))
}
