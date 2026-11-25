//! The seam between the checker and whatever knows how words are spelled.

/// What a back end says when asked to get ready for a language.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
/// ([`crate::spell::SpellCheckerBuilder::backend`]).
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
    fn check_word(&mut self, language: &str, word: &str) -> Result<bool, BackendError>;

    /// Corrections for `word` in `language`, best first. Empty when there are none.
    fn suggest(&mut self, language: &str, word: &str) -> Result<Vec<String>, BackendError>;
}

/// Builds a back end on the checking thread. `Err` says why it cannot work here (for example
/// the platform has no spell checker), and is shown in [`crate::spell::SpellChecker::backends`].
pub type BackendFactory = Box<dyn FnOnce() -> Result<Box<dyn SpellBackend>, BackendError> + Send>;

/// Why a back end could not start, check or suggest, in its own words.
///
/// Back ends make one from their engine's message; `?` converts a `String` or `&str`:
///
/// ```
/// use remedy_ui_widgets::spell::BackendError;
///
/// fn open(path: &str) -> Result<(), BackendError> {
///     Err(format!("no dictionary at {path}"))?
/// }
/// let e = open("/dicts/xx.dic").err();
/// assert_eq!(e.map(|e| e.to_string()), Some("no dictionary at /dicts/xx.dic".to_owned()));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BackendError {
    reason: String,
}

impl BackendError {
    /// A failure described by `reason`.
    pub fn new(reason: impl Into<String>) -> Self {
        BackendError { reason: reason.into() }
    }

    /// What failed.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for BackendError {}

impl From<String> for BackendError {
    fn from(reason: String) -> Self {
        BackendError { reason }
    }
}

impl From<&str> for BackendError {
    fn from(reason: &str) -> Self {
        BackendError { reason: reason.to_owned() }
    }
}

/// Whether a language can be checked, and by whom.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct BackendReport {
    /// The back end's name.
    pub name: String,
    /// True when it is running.
    pub running: bool,
    /// Why it is not running, when it is not.
    pub reason: Option<String>,
}
