//! RemedyUI spell checking, for text fields and anything else that holds words.
//!
//! - [`SpellChecker`]: check words and texts, ask for corrections, in one language or several
//!   at once. Work runs on a thread of its own, so nothing waits on the UI thread.
//! - Engines, chosen at run time for each language: the platform's own checker (Windows 8 and
//!   later, macOS) or Hunspell dictionaries (`.aff`/`.dic` files found in the usual system
//!   folders or given by the app), through one trait, [`SpellBackend`]. A language no engine
//!   can check is reported as such; nothing pretends.
//! - [`find_words`]: the words worth checking in a text, by ICU word segmentation in the
//!   text's language, leaving out links, addresses, mentions, numbers, code and acronyms.
//! - [`PersonalDictionary`]: the user's own words, saved and loaded by the app as a list.
//! - [`SpellCheckField`]: misspellings underlined in a [`remedy_ui_engine::view::TextField`] as people
//!   type, with a context menu of corrections, "Add to dictionary" and "Ignore"
//!   ([`SpellHook::context_menu`]).
//!
//! See `docs/design/05-text.md` and `docs/design/15-widgets-and-media.md`.
//!
//! ```
//! use remedy_ui_widgets::spell::{SpellChecker, find_words};
//!
//! // The words worth checking:
//! let text = "Ask @sam about https://example.com and the NASA plan";
//! let words: Vec<&str> = find_words(text, "en").into_iter().filter_map(|r| text.get(r)).collect();
//! assert_eq!(words, ["Ask", "about", "and", "the", "plan"]);
//!
//! // A checker with a dictionary the app holds itself:
//! let checker = SpellChecker::builder()
//!     .platform(false)
//!     .system_dictionaries(false)
//!     .dictionary("en-US", "SET UTF-8\n".into(), "3\nthe\nplan\nabout\n".into())
//!     .build();
//! assert_eq!(checker.check_word_in("plan", &["en-US"])?, true);
//! assert_eq!(checker.check_word_in("plna", &["en-US"])?, false);
//! # Ok::<(), remedy_ui_widgets::spell::SpellError>(())
//! ```

/// The spelling menu's own words in every language the engine ships, from `i18n/*.ftl`. Apps
/// reword them, or add a language, with [`remedy_ui_engine::view::strings::add_messages`] under the
/// name `services-spell`, or give a field its own [`MenuLabels`].
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("services-spell");

mod backend;
mod checker;
mod engine;
mod error;
mod field;
mod hunspell;
mod languages;
#[allow(unsafe_code)]
mod os;
mod personal;
mod words;

pub use backend::{Availability, BackendError, BackendFactory, BackendReport, Prepared, SpellBackend};
pub use checker::{PendingCheck, SpellChecker, SpellCheckerBuilder};
pub use error::SpellError;
pub use field::{MenuLabels, SpellCheckField, SpellHook, SpellOptions};
pub use hunspell::{HunspellBackend, system_directories};
pub use languages::normalize as normalize_language;
pub use personal::PersonalDictionary;
pub use words::find_words;
