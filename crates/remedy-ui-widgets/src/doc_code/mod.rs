//! Code for RemedyUI apps: syntax colors from the theme ([`highlight`], [`Kind`]), a code
//! view with line numbers and folding ([`code_view`]), a code editor with several cursors
//! ([`code_editor`]), and diffs ([`diff_lines`],
//! [`diff_view`]). Grammars are Sublime Text's, read by syntect with its pure-Rust regex
//! engine; apps add their own with [`add_grammar`].
//!
//! Layer 5: built only from public engine APIs and the base widgets ([`crate::base`]).

/// The code views' own words (fold buttons, how diff lines are read) in every language the
/// engine ships, from `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_engine::view::strings::add_messages`] under the name `doc-code`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("doc-code");

mod diff;
mod editor;
mod highlight;
mod view;

pub use diff::{DiffLine, diff_lines};
pub use editor::code_editor;
pub use highlight::{GrammarError, Highlighter, Kind, LineRuns, add_grammar, highlight, knows, languages};
pub use view::{CodeOptions, code_view, diff_view, kind_color};
