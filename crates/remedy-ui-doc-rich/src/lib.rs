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
pub use markdown::{OpenLink, markdown, markdown_dyn, plain_text};
