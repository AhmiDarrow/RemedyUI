//! What can go wrong when checking spelling.

use std::fmt;

/// Why a spelling request did not produce an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpellError {
    /// A language tag is not a BCP 47 tag.
    BadLanguage(String),
    /// No back end can check any of the languages asked for. `reasons` says, for each language,
    /// what was missing or broken.
    Unavailable {
        /// The languages asked for.
        languages: Vec<String>,
        /// One line per language and back end that failed, such as a dictionary that did not
        /// parse.
        reasons: Vec<String>,
    },
    /// A back end failed while checking or suggesting.
    Backend(crate::BackendError),
    /// The checking thread is not running.
    Stopped(String),
}

impl fmt::Display for SpellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpellError::BadLanguage(t) => write!(f, "`{t}` is not a language tag"),
            SpellError::Unavailable { languages, reasons } => {
                write!(f, "no spell checker for {}", languages.join(", "))?;
                if !reasons.is_empty() {
                    write!(f, " ({})", reasons.join("; "))?;
                }
                Ok(())
            }
            SpellError::Backend(m) => write!(f, "the spell checker failed: {m}"),
            SpellError::Stopped(m) => write!(f, "the spell checking thread is not running: {m}"),
        }
    }
}

impl std::error::Error for SpellError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SpellError::Backend(e) => Some(e),
            _ => None,
        }
    }
}
