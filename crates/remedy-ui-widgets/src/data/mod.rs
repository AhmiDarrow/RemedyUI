//! RemedyUI data widgets: tables, icon grids, chat lists, trees and property grids that stay fast at any size.
//!
//! Rows are virtual (only the rows near the viewport exist), large sorts run off the UI
//! thread, and every widget is fully usable from the keyboard and screen readers.

/// The data widgets' own words (counts, sort states, keyboard hints, status lines) in every
/// language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_engine::view::strings::add_messages`] under the name `widgets-data`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings =
    remedy_ui_engine::engine_strings!("widgets-data");

/// Message `id` of this pack in the window's locale, kept current as the locale changes
/// when read inside a property closure or binding.
pub(crate) fn tr(ui: &remedy_ui_engine::view::Ui, id: &'static str) -> String {
    STRINGS.get(ui, id)
}

/// A closure for message `id`, for a label that follows the window's locale.
pub(crate) fn tr_fn(ui: &remedy_ui_engine::view::Ui, id: &'static str) -> impl Fn() -> String + 'static {
    let ui = ui.clone();
    move || tr(&ui, id)
}

/// Message `id` with placeholder values in the window's locale now, without following it
/// (for something built once, or rebuilt as it changes).
pub(crate) fn format_now(
    ui: &remedy_ui_engine::view::Ui,
    id: &str,
    args: &[(&str, remedy_ui_engine::view::strings::StrArg)],
) -> String {
    STRINGS.format_in(&remedy_ui_core::reactive::untrack(|| ui.locale()), id, args)
}

mod chat;
mod grid;
mod model;
mod property;
mod rows;
mod table;
mod tree;

pub use chat::{Change, ChatState, change, chat_list};
pub use grid::{GridGeometry, GridOptions, GridState, Tiles, grid_view, grid_view_with};
pub use model::{SORT_OFF_THREAD, Selection, SortDirection, SortValue, natural_cmp, sorted_order};
pub use property::{Property, PropertyGroup, property_grid};
pub use table::{Column, TableState, table};
pub use tree::{MoveBox, TreeItem, TreeState, tree_view};
