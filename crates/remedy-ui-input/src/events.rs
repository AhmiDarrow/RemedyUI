//! The event model every bridge translates into.

use std::time::Duration;

use remedy_ui_core::{Point, Vec2};

/// What produced a pointer event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PointerKind {
    /// A mouse or touchpad cursor.
    #[default]
    Mouse,
    /// A finger.
    Touch,
    /// A stylus.
    Pen,
}

/// Pointer buttons, as flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Buttons(pub u8);

impl Buttons {
    /// Primary (left, or a touch contact).
    pub const PRIMARY: Buttons = Buttons(1);
    /// Secondary (right).
    pub const SECONDARY: Buttons = Buttons(1 << 1);
    /// Middle.
    pub const MIDDLE: Buttons = Buttons(1 << 2);
    /// Back (mouse button 4).
    pub const BACK: Buttons = Buttons(1 << 3);
    /// Forward (mouse button 5).
    pub const FORWARD: Buttons = Buttons(1 << 4);

    /// True when every button in `b` is down.
    pub fn contains(self, b: Buttons) -> bool {
        self.0 & b.0 == b.0 && b.0 != 0
    }
}

/// The phase of a pointer event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    /// Contact made or button pressed.
    Down,
    /// Moved.
    Move,
    /// Contact lifted or button released.
    Up,
    /// The system took the pointer away (a gesture became a system gesture).
    Cancel,
}

/// One pointer event.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerEvent {
    /// Which pointer (one per finger; the mouse is 0).
    pub id: u32,
    /// What it is.
    pub kind: PointerKind,
    /// The phase.
    pub phase: PointerPhase,
    /// Position in window logical pixels.
    pub position: Point,
    /// Buttons down after this event.
    pub buttons: Buttons,
    /// Pressure, 0 to 1 (0.5 when the device does not report it).
    pub pressure: f32,
    /// When it happened.
    pub time: Duration,
    /// Earlier positions merged into this event since the last frame, oldest first, with
    /// their times. Ink and velocity use them; handlers see one event per frame.
    pub coalesced: Vec<(Point, Duration)>,
    /// Modifier keys held when it happened (Ctrl-click, Shift-click).
    pub modifiers: Modifiers,
}

/// A key's meaning in the user's keyboard layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A character key, by what it types (lower case).
    Character(String),
    /// Enter or Return.
    Enter,
    /// Tab.
    Tab,
    /// Escape.
    Escape,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Space.
    Space,
    /// Arrow left.
    ArrowLeft,
    /// Arrow right.
    ArrowRight,
    /// Arrow up.
    ArrowUp,
    /// Arrow down.
    ArrowDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// A function key, F1 to F24.
    F(u8),
    /// A key the model does not name, by the platform's name for it.
    Named(String),
}

impl Key {
    /// A character key.
    pub fn char(c: char) -> Key {
        Key::Character(c.to_lowercase().collect())
    }
}

/// Modifier keys held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub ctrl: bool,
    /// Alt or Option.
    pub alt: bool,
    /// The Windows, Command or Super key.
    pub meta: bool,
}

/// Which platform's conventions apply to shortcuts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    /// Windows, Linux, Android, RemedyOS: Ctrl is the primary modifier.
    #[default]
    Standard,
    /// macOS and iOS: Command is the primary modifier.
    Apple,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Modifiers = Modifiers { shift: false, ctrl: false, alt: false, meta: false };

    /// The primary modifier for `platform` alone: Ctrl, or Command on Apple platforms.
    pub fn primary(platform: Platform) -> Modifiers {
        match platform {
            Platform::Standard => Modifiers { ctrl: true, ..Modifiers::NONE },
            Platform::Apple => Modifiers { meta: true, ..Modifiers::NONE },
        }
    }

    /// These modifiers plus Shift.
    pub fn with_shift(self) -> Modifiers {
        Modifiers { shift: true, ..self }
    }
}

/// A key event.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    /// The key's meaning in the current layout.
    pub key: Key,
    /// The physical key, by its position name (`"KeyW"`), for position-based shortcuts.
    pub physical: String,
    /// Modifiers held.
    pub modifiers: Modifiers,
    /// True for a press, false for a release.
    pub pressed: bool,
    /// True when the key is auto-repeating.
    pub repeat: bool,
    /// True while an input method is composing; shortcuts do not fire then.
    pub composing: bool,
    /// The text the key typed with the current layout and modifiers (case, dead keys,
    /// AltGr), when it typed any. Text fields insert this, not the key.
    pub text: Option<String>,
}

/// A scroll from a wheel or touchpad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelEvent {
    /// Position in window logical pixels.
    pub position: Point,
    /// The amount, in logical pixels (lines are converted by the bridge).
    pub delta: Vec2,
    /// True for a touchpad's precise deltas (no smoothing needed).
    pub precise: bool,
    /// When it happened.
    pub time: Duration,
}
