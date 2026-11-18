//! Editions: the switch that changes of default behavior hang off.
//!
//! RemedyUI keeps its behavior stable for an app until the app asks for newer behavior. When
//! a default has to change (better scroll physics, a new focus rule), the new behavior
//! arrives in a new edition, and code in the engine asks the window's edition before choosing:
//!
//! ```
//! use remedy_ui_core::Edition;
//!
//! fn snap_scroll_to_lines(edition: Edition) -> bool {
//!     // A behavior introduced by a later edition would test
//!     // `edition.at_least(Edition::E20xx)`; every behavior today is the first edition's.
//!     edition.at_least(Edition::E2026)
//! }
//!
//! assert!(snap_scroll_to_lines(Edition::default()));
//! assert_eq!(Edition::FIRST, Edition::E2026);
//! assert_eq!("2026".parse::<Edition>(), Ok(Edition::E2026));
//! ```
//!
//! An app that never names an edition gets [`Edition::FIRST`], so it keeps the behavior it was
//! written against for as long as it runs on RemedyUI 1.x. `cargo remedy new` writes the
//! latest edition into new apps. Editions are rare: each one is listed in `docs/STABILITY.md`
//! with every behavior it changes.

/// A set of default behaviors, named by the year it was introduced.
///
/// Editions only ever compare as older or newer: a behavior introduced by an edition applies
/// to that edition and every later one ([`Edition::at_least`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
// New editions are added over time; matching code keeps a wildcard arm for them.
#[non_exhaustive]
pub enum Edition {
    /// The first edition: RemedyUI 1.0's behavior.
    #[default]
    E2026,
}

impl Edition {
    /// The edition an app gets when it does not choose one.
    pub const FIRST: Edition = Edition::E2026;
    /// The newest edition this version of RemedyUI knows; new apps start on it.
    pub const LATEST: Edition = Edition::E2026;
    /// Every edition, oldest first.
    pub const ALL: &'static [Edition] = &[Edition::E2026];

    /// The year that names the edition.
    pub const fn year(self) -> u16 {
        match self {
            Edition::E2026 => 2026,
        }
    }

    /// True when this edition is `other` or newer, so it has every behavior `other` introduced.
    pub fn at_least(self, other: Edition) -> bool {
        self >= other
    }
}

impl std::fmt::Display for Edition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.year())
    }
}

/// Text that does not name an edition this version of RemedyUI knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseEditionError {
    text: String,
}

impl ParseEditionError {
    /// The text that was not an edition.
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Display for ParseEditionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let known: Vec<String> = Edition::ALL.iter().map(Edition::to_string).collect();
        write!(f, "`{}` is not a RemedyUI edition; known editions: {}", self.text, known.join(", "))
    }
}

impl std::error::Error for ParseEditionError {}

impl std::str::FromStr for Edition {
    type Err = ParseEditionError;

    /// Reads an edition's year, such as `2026`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        Edition::ALL
            .iter()
            .copied()
            .find(|e| e.to_string() == t)
            .ok_or_else(|| ParseEditionError { text: s.to_owned() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editions_order_and_parse() {
        assert_eq!(Edition::default(), Edition::FIRST);
        assert!(Edition::LATEST.at_least(Edition::FIRST));
        assert_eq!(Edition::E2026.to_string(), "2026");
        for e in Edition::ALL {
            assert_eq!(e.to_string().parse::<Edition>(), Ok(*e));
        }
        let Err(e) = "2019".parse::<Edition>() else { unreachable!("2019 is not an edition") };
        assert_eq!(e.text(), "2019");
        assert!(e.to_string().contains("2026"), "{e}");
        assert!(Edition::ALL.windows(2).all(|w| matches!(w, [a, b] if a < b)));
    }
}
