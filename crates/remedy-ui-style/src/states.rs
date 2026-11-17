//! Element states.

use std::fmt;
use std::ops::{BitOr, BitOrAssign};

/// The states an element can be in, as flags. Styles are chosen per state combination.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct States(u16);

impl States {
    /// No state.
    pub const NONE: States = States(0);
    /// The pointer is over it.
    pub const HOVER: States = States(1);
    /// It is being pressed.
    pub const PRESSED: States = States(1 << 1);
    /// It has keyboard focus.
    pub const FOCUS: States = States(1 << 2);
    /// It has focus and the user is navigating by keyboard (show the focus ring).
    pub const FOCUS_VISIBLE: States = States(1 << 3);
    /// It cannot be used.
    pub const DISABLED: States = States(1 << 4);
    /// It is selected.
    pub const SELECTED: States = States(1 << 5);
    /// It is checked or on.
    pub const CHECKED: States = States(1 << 6);
    /// Its value is not valid.
    pub const INVALID: States = States(1 << 7);
    /// It is being dragged.
    pub const DRAGGING: States = States(1 << 8);

    /// True when every flag in `other` is set.
    pub fn contains(self, other: States) -> bool {
        self.0 & other.0 == other.0
    }

    /// These states with `other` set or cleared.
    pub fn with(self, other: States, on: bool) -> States {
        if on { States(self.0 | other.0) } else { States(self.0 & !other.0) }
    }

    /// The raw bits, for cache keys.
    pub fn bits(self) -> u16 {
        self.0
    }
}

impl BitOr for States {
    type Output = States;
    fn bitor(self, o: States) -> States {
        States(self.0 | o.0)
    }
}

impl BitOrAssign for States {
    fn bitor_assign(&mut self, o: States) {
        self.0 |= o.0;
    }
}

impl fmt::Debug for States {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            (States::HOVER, "HOVER"),
            (States::PRESSED, "PRESSED"),
            (States::FOCUS, "FOCUS"),
            (States::FOCUS_VISIBLE, "FOCUS_VISIBLE"),
            (States::DISABLED, "DISABLED"),
            (States::SELECTED, "SELECTED"),
            (States::CHECKED, "CHECKED"),
            (States::INVALID, "INVALID"),
            (States::DRAGGING, "DRAGGING"),
        ];
        let set: Vec<&str> = names.iter().filter(|(s, _)| self.contains(*s)).map(|(_, n)| *n).collect();
        write!(f, "States({})", if set.is_empty() { "NONE".to_string() } else { set.join(" | ") })
    }
}
