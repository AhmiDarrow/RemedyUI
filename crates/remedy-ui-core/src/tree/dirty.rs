//! Dirty flags: which phases an element needs this frame.

use std::fmt;
use std::ops::{BitOr, BitOrAssign};

/// A set of phases an element must redo. See `docs/design/02-state-and-elements.md`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Dirty(u8);

impl Dirty {
    /// Nothing to redo.
    pub const NONE: Dirty = Dirty(0);
    /// Re-resolve style (token, state or style property changed).
    pub const STYLE: Dirty = Dirty(1);
    /// Lay out again (size-affecting property, content or children changed).
    pub const LAYOUT: Dirty = Dirty(1 << 1);
    /// Re-record the paint chunk (visual-only property changed).
    pub const PAINT: Dirty = Dirty(1 << 2);
    /// Transform, opacity, clip or scroll offset changed on a layered element; only the
    /// compositor has work.
    pub const COMPOSITE: Dirty = Dirty(1 << 3);
    /// Rebuild the accessibility node.
    pub const SEMANTICS: Dirty = Dirty(1 << 4);
    /// A descendant needs layout (set on ancestors up to the nearest relayout boundary).
    pub const CHILD_LAYOUT: Dirty = Dirty(1 << 5);
    /// A descendant needs painting (set on ancestors up to the nearest repaint boundary).
    pub const CHILD_PAINT: Dirty = Dirty(1 << 6);

    /// True when every flag in `other` is set.
    pub fn contains(self, other: Dirty) -> bool {
        self.0 & other.0 == other.0
    }

    /// True when any flag in `other` is set.
    pub fn intersects(self, other: Dirty) -> bool {
        self.0 & other.0 != 0
    }

    /// True when no flag is set.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// These flags without those in `other`.
    pub fn without(self, other: Dirty) -> Dirty {
        Dirty(self.0 & !other.0)
    }
}

impl BitOr for Dirty {
    type Output = Dirty;
    fn bitor(self, o: Dirty) -> Dirty {
        Dirty(self.0 | o.0)
    }
}

impl BitOrAssign for Dirty {
    fn bitor_assign(&mut self, o: Dirty) {
        self.0 |= o.0;
    }
}

impl fmt::Debug for Dirty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            (Dirty::STYLE, "STYLE"),
            (Dirty::LAYOUT, "LAYOUT"),
            (Dirty::PAINT, "PAINT"),
            (Dirty::COMPOSITE, "COMPOSITE"),
            (Dirty::SEMANTICS, "SEMANTICS"),
            (Dirty::CHILD_LAYOUT, "CHILD_LAYOUT"),
            (Dirty::CHILD_PAINT, "CHILD_PAINT"),
        ];
        let set: Vec<&str> = names.iter().filter(|(d, _)| self.contains(*d)).map(|(_, n)| *n).collect();
        if set.is_empty() { f.write_str("Dirty(NONE)") } else { write!(f, "Dirty({})", set.join(" | ")) }
    }
}
