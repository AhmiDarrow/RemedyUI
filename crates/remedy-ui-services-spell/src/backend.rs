//! The seam between the checker and whatever knows how words are spelled.

/// What a back end says when asked to get ready for a language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prepared {
    /// It can check the language. `dictionary` names what it will use (`en_US`, `en-US`), which
    /// may be a close relative of the language asked for (`en_US` for `en-AU`).
    Ready {
        /// The dictionary or language the back end uses.
        dictionary: String,
    },
    /// It has nothing for this language.
    Missing,
    /// It has something for this language that does not work; the text says why.
    Broken(String),
}

/// A spell checking engine. The checker owns its back ends on a thread of its own, so a back
/// end need not be `Send`: it is made on that thread by the factory the app gives
/// ([`crate::SpellCheckerBuilder::backend`]).
///
/// Words come already cut out of the text, and personal-dictionary words and scripts the
/// language does not use are dealt with before a back end is asked.
pub trait SpellBackend {
    /// A short name for reports, such as `hunspell` or `windows`.
    fn name(&self) -> &str;

    /// Gets ready to check `language` (a normalized BCP 47 tag), loading what it needs. Called
    /// once per language; the answer is remembered.
    fn prepare(&mut self, language: &str) -> Prepared;

    /// The languages this back end can check, for a settings page. Back ends that cannot list
    /// them return nothing.
    fn languages(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Whether `word` is spelled correctly in `language`, which `prepare` answered `Ready` for.
    fn check_word(&mut self, language: &str, word: &str) -> Result<bool, String>;

    /// Corrections for `word` in `language`, best first. Empty when there are none.
    fn suggest(&mut self, language: &str, word: &str) -> Result<Vec<String>, String>;
}

/// Builds a back end on the checking thread. `Err` says why it cannot work here (for example
/// the platform has no spell checker), and is shown in [`crate::SpellChecker::backends`].
pub type BackendFactory = Box<dyn FnOnce() -> Result<Box<dyn SpellBackend>, String> + Send>;

/// Whether a language can be checked, and by whom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    /// A back end can check it.
    Ready {
        /// The back end's name.
        backend: String,
        /// What it uses for the language.
        dictionary: String,
    },
    /// No back end has a dictionary for it. `reasons` has one line for each back end that has
    /// a dictionary that is broken.
    Missing {
        /// Broken dictionaries, if any.
        reasons: Vec<String>,
    },
}

/// One back end and whether it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendReport {
    /// The back end's name.
    pub name: String,
    /// True when it is running.
    pub running: bool,
    /// Why it is not running, when it is not.
    pub reason: Option<String>,
}
