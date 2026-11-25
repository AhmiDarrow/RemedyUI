//! RemedyUI boards: spaces their owners build, such as a FreeSpeak Home (a house that holds
//! its voice and chat rooms, its news, links and reactions), not a fixed list.
//!
//! - [`Board`]: the document, areas on a column grid holding widgets, saved as versioned
//!   JSON that peers receive and render; every change checks the board stays whole.
//! - [`Registry`] and [`WidgetKind`]: what can be placed. Built-ins (text, links,
//!   reactions), the app's own kinds, and [`ScriptKind`]s that people write, in any language
//!   the board accepts: a widget [`Package`] names its language, and the board's
//!   [`Context::engines`] are the script engines it accepts (RemedyLua with
//!   `remedy-ui-script-lua`, WebAssembly with `remedy-ui-script-wasm`). Scripts run
//!   sandboxed with operation, time, stack and memory limits, and are built as ordinary
//!   elements.
//! - [`board`] and [`BoardState`]: one renderer for visitors and for the person arranging
//!   it. Editing selects, drags and drops areas and widgets, does every move from the
//!   keyboard too, undoes and redoes, and edits settings in a property grid (text, numbers,
//!   switches, choices with labels apart from their values, moments, lists and pictures).
//!   [`BoardOptions`] hold an area to one widget, fix the row height, move the panel out
//!   ([`board_panel`], for several boards on a page) and say when typing has settled
//!   ([`BoardState::settled`]: one change, not one a keystroke).
//! - [`BoardLabels`]: every word a board says, in the window's language (the engine ships
//!   eleven, see [`STRINGS`]) unless the app gives its own.
//! - [`reactions()`]: emoji reactions, a component for any widget or screen.

/// The board's own words (its editor, empty states, messages, setting editors, built-in
/// widgets and reactions) in every language the engine ships, from `i18n/*.ftl`: what
/// [`BoardLabels::localized`] reads. Apps reword them, or add a language, with
/// [`remedy_ui_engine::view::strings::add_messages`] under the name `widgets-board`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("widgets-board");

mod board;
mod kinds;
mod labels;
mod model;
pub mod reactions;
mod settings;

pub use board::{AreaLabelFn, BoardOptions, BoardState, Panel, Selection, board, board_panel};
pub use kinds::{
    AppElement, ChoosePictureFn, Context, ElementFn, HandlerFn, PictureFn, Registry, ScriptHooks, ScriptKind,
    SharedWidget, WidgetKind, build_node, build_node_with, mount_script, parse_links, value,
};
pub use labels::{BoardLabels, ReactionLabels};
pub use model::{Area, Board, BoardError, FORMAT, Placed};
pub use reactions::{Reaction, reactions, reactions_in};
pub use remedy_ui_engine::script::{
    Engines, Language, Limits, Package, ScriptEngine, ScriptError, SettingKind, SettingSpec,
};
