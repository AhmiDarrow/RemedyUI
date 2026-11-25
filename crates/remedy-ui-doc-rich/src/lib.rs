//! Rich documents for RemedyUI apps: Markdown shown as themed, selectable, accessible text
//! ([`markdown`]), a rich text editor with formatting, links and mentions ([`rich_editor`])
//! saving to Markdown ([`MarkdownFormat`]), and a find bar that searches any text on screen
//! ([`find_bar`]).
//!
//! Layer 5: built only from public engine APIs and `widgets-core`.

mod editor;
mod find;
mod format;
mod markdown;

pub use editor::{MentionCandidate, MentionSource, RichEditorOptions, rich_editor};
pub use find::{FindState, find_bar};
pub use format::{MENTION_SCHEME, MarkdownFormat, RichFormat};
pub use markdown::{OpenLink, heading_id, headings, markdown, plain_text, scroll_to_heading};

use remedy_ui_engine::view::Ui;
use remedy_ui_engine::view::strings::StrArg;

/// The pack's own words (toolbar names, the find bar, screen reader names, keyboard hints)
/// in every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a
/// language, with [`remedy_ui_engine::view::strings::add_messages`] under the name `doc-rich`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("doc-rich");

/// Message `id` of this pack in the window's locale, kept current as the locale changes
/// when read inside a property closure.
pub(crate) fn tr(ui: &Ui, id: &str) -> String {
    STRINGS.get(ui, id)
}

/// Message `id` with placeholder values in the window's locale (tracked). The values here
/// are counts and names read by screen readers or shown alone in a status, so the
/// direction-isolation marks around them are left out.
pub(crate) fn trf(ui: &Ui, id: &str, args: &[(&str, StrArg)]) -> String {
    STRINGS.format(ui, id, args).replace(['\u{2068}', '\u{2069}'], "")
}
