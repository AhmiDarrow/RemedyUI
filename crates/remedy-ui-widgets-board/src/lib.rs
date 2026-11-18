//! RemedyUI boards: spaces their owners build, such as a FreeSpeak Home (a house that holds
//! its voice and chat rooms, its news, links and reactions), not a fixed list.
//!
//! - [`Board`]: the document, areas on a column grid holding widgets, saved as versioned
//!   JSON that peers receive and render; every change checks the board stays whole.
//! - [`Registry`] and [`WidgetKind`]: what can be placed. Built-ins (text, links,
//!   reactions), the app's own kinds, and [`ScriptKind`]s that people write in Luau, run
//!   sandboxed with time and memory limits, and built as ordinary elements.
//! - [`board`] and [`BoardState`]: one renderer for visitors and for the person arranging
//!   it. Editing selects, drags and drops areas and widgets, does every move from the
//!   keyboard too, undoes and redoes, and edits settings in a property grid.
//! - [`reactions()`]: emoji reactions, a component for any widget or screen.

mod board;
mod kinds;
mod model;
pub mod reactions;

pub use board::{BoardState, Selection, board};
pub use kinds::{Context, Registry, ScriptKind, WidgetKind, build_node, parse_links, value};
pub use model::{Area, Board, BoardError, FORMAT, Placed};
pub use reactions::{Reaction, reactions};
pub use remedy_ui_script::{Limits, SettingKind, SettingSpec};
