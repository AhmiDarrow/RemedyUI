//! Code for RemedyUI apps: syntax colors from the theme ([`highlight`], [`Kind`]), a code
//! view with line numbers and folding ([`code_view`]), a code editor with several cursors
//! ([`code_editor`]), and diffs ([`diff_lines`],
//! [`diff_view`]). Grammars are Sublime Text's, read by syntect with its pure-Rust regex
//! engine; apps add their own with [`add_grammar`].
//!
//! Layer 5: built only from public engine APIs and `widgets-core`.

mod diff;
mod editor;
mod highlight;
mod view;

pub use diff::{DiffLine, diff_lines};
pub use editor::code_editor;
pub use highlight::{GrammarError, Highlighter, Kind, LineRuns, add_grammar, highlight, knows, languages};
pub use view::{CodeOptions, code_view, diff_view, kind_color};
