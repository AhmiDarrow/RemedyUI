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
