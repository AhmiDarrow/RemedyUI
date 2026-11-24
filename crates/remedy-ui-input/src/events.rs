//! The event model every bridge translates into.

use std::time::Duration;

use remedy_ui_core::{Point, Vec2};

/// What produced a pointer event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
// Exhaustive on purpose: the pointer lifecycle; hover, enter and leave are derived from it by the view.
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
///
/// Bridges and tests make one with [`PointerEvent::new`] and the `with_*` methods; fields are
/// added over time (tilt and twist for pens, for example), so it is never a struct literal
/// outside this crate:
///
/// ```
/// use remedy_ui_core::Point;
/// use remedy_ui_input::{Buttons, PointerEvent, PointerKind, PointerPhase};
///
/// let down = PointerEvent::new(PointerPhase::Down, Point::new(10.0, 20.0));
/// assert_eq!(down.buttons, Buttons::PRIMARY);
/// let pen = PointerEvent::new(PointerPhase::Move, Point::new(11.0, 21.0)).with_kind(PointerKind::Pen).with_pressure(0.8);
/// assert_eq!((pen.kind, pen.pressure), (PointerKind::Pen, 0.8));
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
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

impl PointerEvent {
    /// The mouse (pointer 0) at `position` in `phase`: the primary button is down for
    /// [`PointerPhase::Down`] and up otherwise, pressure is 0.5, at time zero, with no
    /// modifiers.
    pub fn new(phase: PointerPhase, position: Point) -> Self {
        PointerEvent {
            id: 0,
            kind: PointerKind::Mouse,
            phase,
            position,
            buttons: if phase == PointerPhase::Down { Buttons::PRIMARY } else { Buttons::default() },
            pressure: 0.5,
            time: Duration::ZERO,
            coalesced: Vec::new(),
            modifiers: Modifiers::NONE,
        }
    }

    /// With pointer id `id` (one per finger; the mouse is 0).
    pub fn with_id(mut self, id: u32) -> Self {
        self.id = id;
        self
    }

    /// From a `kind` of device.
    pub fn with_kind(mut self, kind: PointerKind) -> Self {
        self.kind = kind;
        self
    }

    /// With `buttons` down after the event.
    pub fn with_buttons(mut self, buttons: Buttons) -> Self {
        self.buttons = buttons;
        self
    }

    /// With `pressure`, 0 to 1.
    pub fn with_pressure(mut self, pressure: f32) -> Self {
        self.pressure = pressure;
        self
    }

    /// Happening at `time`.
    pub fn with_time(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }

    /// With earlier positions merged into it, oldest first.
    pub fn with_coalesced(mut self, coalesced: Vec<(Point, Duration)>) -> Self {
        self.coalesced = coalesced;
        self
    }

    /// With `modifiers` held.
    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }
}

/// A key's meaning in the user's keyboard layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
    /// The system's back action: Android's back button and gesture, a keyboard's browser-back
    /// key. What nothing takes dismisses like Escape, then leaves the window.
    Back,
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
// Exhaustive on purpose: these are the four modifiers every platform has, and apps write
// `Modifiers { ctrl: true, ..Modifiers::NONE }`. A platform-only key (Fn, Hyper) would be a
// new type, not a field here.
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
#[non_exhaustive]
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
    /// Shift alone.
    pub const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };
    /// Control alone.
    pub const CTRL: Modifiers = Modifiers { ctrl: true, ..Modifiers::NONE };
    /// Alt (Option) alone.
    pub const ALT: Modifiers = Modifiers { alt: true, ..Modifiers::NONE };
    /// The Windows, Command or Super key alone.
    pub const META: Modifiers = Modifiers { meta: true, ..Modifiers::NONE };

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
#[non_exhaustive]
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

impl KeyEvent {
    /// `key` pressed with no modifiers. A [`Key::Character`] types its character, as the
    /// keyboard would with no modifier held; [`Key::Space`] types a space.
    ///
    /// ```
    /// use remedy_ui_input::{Key, KeyEvent, Modifiers};
    ///
    /// let a = KeyEvent::press(Key::char('a'));
    /// assert_eq!(a.text.as_deref(), Some("a"));
    /// let save = KeyEvent::press(Key::char('s')).with_modifiers(Modifiers::CTRL);
    /// assert_eq!(save.text, None, "a shortcut types nothing");
    /// ```
    pub fn press(key: Key) -> Self {
        let text = match &key {
            Key::Character(c) => Some(c.clone()),
            Key::Space => Some(" ".to_owned()),
            _ => None,
        };
        KeyEvent {
            key,
            physical: String::new(),
            modifiers: Modifiers::NONE,
            pressed: true,
            repeat: false,
            composing: false,
            text,
        }
    }

    /// `key` released.
    pub fn release(key: Key) -> Self {
        KeyEvent { pressed: false, text: None, ..KeyEvent::press(key) }
    }

    /// The key a shortcut is matched on. It is [`key`](Self::key), except on a layout whose
    /// key types a letter outside ASCII (Russian, Greek, Hebrew, Arabic): there it is the
    /// letter or digit in that key's place on a US keyboard, taken from
    /// [`physical`](Self::physical), so Ctrl+C, Ctrl+V and Ctrl+F are the same keys on every
    /// layout, as desktops have them. Layouts that type ASCII letters elsewhere (AZERTY,
    /// Dvorak) keep their own.
    ///
    /// ```
    /// use remedy_ui_input::{Key, KeyEvent, Modifiers};
    ///
    /// // Ctrl with the key that types "с" on a Russian layout: the C key.
    /// let copy = KeyEvent::press(Key::char('с')).with_physical("KeyC").with_modifiers(Modifiers::CTRL);
    /// assert_eq!(copy.shortcut_key(), Key::char('c'));
    /// assert!(copy.is_shortcut_key("c"));
    /// // AZERTY's A is where a US keyboard has Q: it stays A.
    /// let all = KeyEvent::press(Key::char('a')).with_physical("KeyQ").with_modifiers(Modifiers::CTRL);
    /// assert!(all.is_shortcut_key("a") && !all.is_shortcut_key("q"));
    /// ```
    pub fn shortcut_key(&self) -> Key {
        let Key::Character(c) = &self.key else { return self.key.clone() };
        if c.is_ascii() {
            return self.key.clone();
        }
        let place = self.physical.strip_prefix("Key").or_else(|| self.physical.strip_prefix("Digit"));
        match place {
            Some(p) if p.len() == 1 && p.chars().all(|c| c.is_ascii_alphanumeric()) => {
                Key::Character(p.to_ascii_lowercase())
            }
            _ => self.key.clone(),
        }
    }

    /// True when a shortcut on `letter` (a lower-case ASCII letter or a digit) is this key:
    /// see [`shortcut_key`](Self::shortcut_key).
    pub fn is_shortcut_key(&self, letter: &str) -> bool {
        matches!(self.shortcut_key(), Key::Character(k) if k == letter)
    }

    /// As `key` instead (a bridge mapping a platform key, the view turning Back into Escape).
    pub fn with_key(mut self, key: Key) -> Self {
        self.key = key;
        self
    }

    /// With `modifiers` held. Ctrl, Alt or Meta make the key a shortcut: it types no text
    /// (set some with [`KeyEvent::with_text`] for AltGr layouts).
    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        if modifiers.ctrl || modifiers.alt || modifiers.meta {
            self.text = None;
        }
        self.modifiers = modifiers;
        self
    }

    /// From the physical key at `position` (`"KeyW"`).
    pub fn with_physical(mut self, position: impl Into<String>) -> Self {
        self.physical = position.into();
        self
    }

    /// Typing `text` (or nothing).
    pub fn with_text(mut self, text: Option<String>) -> Self {
        self.text = text;
        self
    }

    /// As an auto-repeat (`true`) or not.
    pub fn with_repeat(mut self, repeat: bool) -> Self {
        self.repeat = repeat;
        self
    }

    /// While an input method composes (`true`) or not.
    pub fn with_composing(mut self, composing: bool) -> Self {
        self.composing = composing;
        self
    }

    /// As a press (`true`) or a release.
    pub fn with_pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }
}

/// A scroll from a wheel or touchpad.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WheelEvent {
    /// Position in window logical pixels.
    pub position: Point,
    /// The amount, in logical pixels (lines are converted by the bridge).
    pub delta: Vec2,
    /// True for a touchpad's precise deltas (no smoothing needed).
    pub precise: bool,
    /// When it happened.
    pub time: Duration,
    /// Modifier keys held (Ctrl with the wheel zooms; Shift scrolls sideways).
    pub modifiers: Modifiers,
}

impl WheelEvent {
    /// A wheel scroll of `delta` logical pixels at `position`: not precise, at time zero,
    /// with no modifiers.
    pub fn new(position: Point, delta: Vec2) -> Self {
        WheelEvent { position, delta, precise: false, time: Duration::ZERO, modifiers: Modifiers::NONE }
    }

    /// From a touchpad's precise deltas (`true`) or not.
    pub fn with_precise(mut self, precise: bool) -> Self {
        self.precise = precise;
        self
    }

    /// Happening at `time`.
    pub fn with_time(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }

    /// With `modifiers` held.
    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }
}

/// Where a pinch is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Exhaustive on purpose: the gesture lifecycle.
pub enum PinchPhase {
    /// Two fingers came down (or a touchpad pinch began).
    Start,
    /// They moved.
    Change,
    /// They lifted.
    End,
    /// The system took the gesture away.
    Cancel,
}

/// A pinch to zoom from a touchpad or touch screen, already recognized by the platform.
/// (Two-finger pinches on touch screens arrive as pointer events, and widgets that zoom
/// recognize them there too.)
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PinchEvent {
    /// The point between the fingers, in window logical pixels.
    pub position: Point,
    /// The change in scale since the previous event: 1.1 is 10% larger.
    pub scale: f32,
    /// Where the gesture is.
    pub phase: PinchPhase,
    /// When it happened.
    pub time: Duration,
}

impl PinchEvent {
    /// A pinch at `position` in `phase`, scaling by `scale`, at time zero.
    pub fn new(phase: PinchPhase, position: Point, scale: f32) -> Self {
        PinchEvent { position, scale, phase, time: Duration::ZERO }
    }

    /// Happening at `time`.
    pub fn with_time(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }
}
