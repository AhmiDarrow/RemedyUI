//! RemedyUI's navigation widgets, built on the public view API and the core widgets.
//!
//! - [`Commands`]: actions with labels, icons and rebindable shortcuts, shared by the
//!   keyboard ([`Commands::attach`]), [`toolbar`]s, menus and the [`CommandPalette`]
//! - [`toolbar`] and [`toolbar_editor`]: toolbars whose buttons the user picks and orders
//! - [`tabs`], [`nav_item`] ([`nav_item_dyn`]), [`nav_rail`], [`bottom_bar`], [`breadcrumbs`]
//! - [`NavStack`]: pages pushed and popped with a back button, Alt+Left and the mouse's back
//!   button, keeping lower pages alive
//! - [`drawer`] and [`sheet`]: panels from a window edge, with a scrim and focus kept inside
//! - [`title_bar`]: custom chrome for frameless windows (drag, double-click to maximize, and
//!   the window buttons on the platform's side)
//!
//! See `docs/design/15-widgets-and-media.md`.

/// The navigation widgets' own words (button names, tooltips, keyboard hints, the command
/// palette) in every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add
/// a language, with [`remedy_ui_engine::view::strings::add_messages`] under the name `widgets-nav`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("widgets-nav");

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

mod bindings;
mod commands;
mod nav;
mod panels;
mod stack;
mod toolbar;

pub use bindings::{BindingsError, bindings_from_json, bindings_to_json};
pub use commands::{CommandInfo, Commands};
// `nav_item_dyn` is kept, deprecated, until the next minor release: `nav_item` takes a closure.
#[allow(deprecated)]
pub use nav::nav_item_dyn;
pub use nav::{NavEntry, Tab, bottom_bar, breadcrumbs, nav_item, nav_rail, tabs};
pub use panels::{Edge, TitleBarActions, drawer, sheet, title_bar};
pub use stack::NavStack;
pub use toolbar::{CommandPalette, fuzzy_score, toolbar, toolbar_editor};
