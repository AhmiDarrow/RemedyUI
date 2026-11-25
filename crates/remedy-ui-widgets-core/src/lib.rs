//! RemedyUI's core widgets, the pack every app uses.
//!
//! Each widget is a function that builds elements with the public view API, takes its look
//! from the theme's style sheet (by widget name and variant), declares its semantics, and
//! works from the keyboard as well as the pointer. Values live in signals the app owns, so a
//! widget shows the app's state rather than keeping its own. See
//! `docs/design/15-widgets-and-media.md`.
//!
//! - Containers: [`column()`], [`row`], [`overflow_row`], [`wrap`], [`stack`], [`spacer`], [`divider`],
//!   [`card`], [`section`], [`disclosure`], [`scroll`], [`split`], [`dock`]
//! - Lists: [`for_each`], [`list`], [`virtual_list`]
//! - Text: [`text`] (with a variant: `.variant("muted")`), [`heading`], [`link`]
//! - Actions: [`button`], [`button_dyn`], [`icon_button`], [`toggle_button`], [`menu_button`]
//! - Choices: [`check`], [`switch`], [`radio_group`], [`segmented`], [`segmented_unavailable`], [`select`], [`chip`]
//! - Values: [`slider`], [`stepper`]
//! - Input: [`text_input`], [`text_area`], [`search`], [`secret_input`] with
//!   [`reveal_toggle`], [`field`]
//! - Feedback: [`progress`], [`spinner`], [`lamp`], [`badge`], [`toast`], [`toast_at`], [`empty_state`],
//!   [`skeleton`]
//! - Overlays: [`tooltip`], [`popover`], [`modal`], [`modal_fitted`], [`dialog`], [`trusted_prompt`],
//!   [`lock_screen`]
//! - Menus: [`MenuItem`] (commands, check and radio items, submenus, separators, shortcuts,
//!   icons, access keys), [`menu`], [`context_menu`], [`menu_button`], [`menu_bar`]
//! - Media: [`icon`], [`image`], [`avatar`], [`canvas`]
//! - [`Rejection`]: why the app turned down a person's edit, shown beside it

/// The smallest size, in logical pixels, of anything a pointer can press (WCAG 2.2, 2.5.8
/// target size): interactive widgets are at least this tall, and icon buttons this wide.
pub const MIN_TARGET: f32 = 24.0;

/// Keeps `el` at least [`MIN_TARGET`] tall.
pub fn min_target(el: El) -> El {
    el.layout(|l| {
        l.min_height = remedy_ui_core::layout::Length::Px(MIN_TARGET);
    })
}

use remedy_ui_view::El;

/// The pack's own words (labels, screen reader names, keyboard hints) in every language the
/// engine ships, from `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_view::strings::add_messages`] under the name `widgets-core`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("widgets-core");

/// Message `id` of this pack in the window's locale, kept current as the locale changes
/// when read inside a property closure.
pub(crate) fn tr(ui: &remedy_ui_view::Ui, id: &'static str) -> String {
    STRINGS.get(ui, id)
}

mod actions;
mod choices;
mod containers;
mod dock;
mod feedback;
mod icons;
mod inputs;
mod lists;
mod menu_bar;
mod menus;
mod overflow;
mod overlays;
mod rejection;
mod text;
mod values;

// `button_dyn` is kept, deprecated, until the next minor release: `button` takes a closure.
#[allow(deprecated)]
pub use actions::button_dyn;
pub use actions::{button, icon_button, menu_button, toggle_button};
pub use choices::{check, chip, radio_group, segmented, segmented_unavailable, select, switch};
pub use containers::{card, column, disclosure, divider, row, scroll, section, spacer, split, stack, wrap};
pub use dock::{DockSide, DockSpec, DockState, dock};
pub use feedback::{badge, empty_state, lamp, progress, skeleton, spinner, toast, toast_at};
pub use icons::{Icon, avatar, canvas, icon, image, pixel_image};
pub use inputs::{field, reveal_toggle, search, secret_input, text_area, text_edit_menu, text_input};
pub use lists::{ListItems, ListTransitions, for_each, for_each_with, list, virtual_list};
pub use menu_bar::menu_bar;
pub use menus::{MenuItem, close_menus_in, context_menu, context_menu_at, menu, menu_press_through};
pub use overflow::{OverflowRow, overflow_row};
pub use overlays::{
    DialogButton, Modal, PartTooltip, dialog, fill_window, fit_beside, fit_near, fit_near_dir,
    full_text_tooltip, lock_screen, modal, modal_fitted, place_at, place_beside, place_near,
    place_near_within, popover, tooltip, trusted_prompt,
};
pub use rejection::Rejection;
// Kept for apps written against it (FreeSpeak) until the next minor release.
#[allow(deprecated)]
pub use text::label;
pub use text::{heading, link, text};
pub use values::{reveal_on_hover, slider, stepper};
