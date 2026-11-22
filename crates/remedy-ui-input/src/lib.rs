//! RemedyUI input.
//!
//! - [`events`]: the event model every platform bridge translates into: pointers (mouse,
//!   touch, pen; buttons, pressure, coalesced history), keys (layout-aware meaning, physical
//!   position, composing state) and wheels.
//! - [`VelocityTracker`] and [`GestureArena`]: nested gesture recognizers compete for a
//!   pointer, and one wins.
//! - [`ScrollAxis`]: flings that decay exponentially, and overscroll that resists and springs
//!   back, per platform.
//! - [`CommandTable`]: commands with labels, default chords, scopes and enabled state; the
//!   user's rebindings with conflict checks; and shortcut matching that respects input
//!   methods. `Primary` is Ctrl, or Command on Apple platforms.
//! - [`gamepad`]: gamepads as a TV remote (arrows, Enter, Back), with held directions
//!   repeating as keys do ([`GamepadTuning`]).
//!
//! See `docs/design/06-input-and-motion.md`.

mod commands;
pub mod events;
pub mod gamepad;
mod gesture;
mod scroll;

pub use commands::{Chord, ChordError, Clash, Command, CommandTable, Conflict, Scope};
pub use events::{
    Buttons, Key, KeyEvent, Modifiers, PinchEvent, PinchPhase, Platform, PointerEvent, PointerKind,
    PointerPhase, WheelEvent,
};
pub use gamepad::{GamepadKeys, GamepadTuning, PadButtons, PadState};
pub use gesture::{Claim, GestureArena, GestureKind, GestureSettings, Recognizer, VelocityTracker};
pub use scroll::{ScrollAxis, ScrollPhysics};
