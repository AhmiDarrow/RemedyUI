//! RemedyUI's core widgets, the pack every app uses.
//!
//! Each widget is a function that builds elements with the public view API, takes its look
//! from the theme's style sheet (by widget name and variant), declares its semantics, and
//! works from the keyboard as well as the pointer. Values live in signals the app owns, so a
//! widget shows the app's state rather than keeping its own. See
//! `docs/design/15-widgets-and-media.md`.
//!
//! - Containers: [`column()`], [`row`], [`wrap`], [`stack`], [`spacer`], [`divider`],
//!   [`card`], [`section`], [`scroll`], [`split`]
//! - Lists: [`for_each`], [`list`], [`virtual_list`]
//! - Text: [`text`] (with a variant: `.variant("muted")`), [`heading`], [`link`]
//! - Actions: [`button`], [`button_dyn`], [`icon_button`], [`toggle_button`], [`menu_button`]
//! - Choices: [`check`], [`switch`], [`radio_group`], [`segmented`], [`select`], [`chip`]
//! - Values: [`slider`], [`stepper`]
//! - Input: [`text_input`], [`text_area`], [`search`], [`secret_input`] with
//!   [`reveal_toggle`], [`field`]
//! - Feedback: [`progress`], [`spinner`], [`lamp`], [`badge`], [`toast`], [`empty_state`],
//!   [`skeleton`]
//! - Overlays: [`tooltip`], [`menu`], [`context_menu`], [`popover`], [`modal`], [`dialog`],
//!   [`trusted_prompt`], [`lock_screen`]
//! - Media: [`icon`], [`image`], [`avatar`], [`canvas`]
//! - [`Rejection`]: why the app turned down a person's edit, shown beside it

/// The smallest size, in logical pixels, of anything a pointer can press (WCAG 2.2, 2.5.8
/// target size): interactive widgets are at least this tall, and icon buttons this wide.
pub const MIN_TARGET: f32 = 24.0;

/// Keeps `el` at least [`MIN_TARGET`] tall.
pub fn min_target(el: El) -> El {
    el.layout(|l| {
        l.min_height = remedy_ui_layout::Length::Px(MIN_TARGET);
    })
}

use remedy_ui_view::El;

mod actions;
mod choices;
mod containers;
mod feedback;
mod icons;
mod inputs;
mod lists;
mod overlays;
mod rejection;
mod text;
mod values;

// `button_dyn` is kept, deprecated, until the next minor release: `button` takes a closure.
#[allow(deprecated)]
pub use actions::button_dyn;
pub use actions::{button, icon_button, menu_button, toggle_button};
pub use choices::{check, chip, radio_group, segmented, select, switch};
pub use containers::{card, column, divider, row, scroll, section, spacer, split, stack, wrap};
pub use feedback::{badge, empty_state, lamp, progress, skeleton, spinner, toast};
pub use icons::{Icon, avatar, canvas, icon, image};
pub use inputs::{field, reveal_toggle, search, secret_input, text_area, text_input};
pub use lists::{ListItems, ListTransitions, for_each, for_each_with, list, virtual_list};
pub use overlays::{
    DialogButton, MenuItem, Modal, context_menu, context_menu_at, dialog, fill_window, fit_near, lock_screen,
    menu, modal, place_at, place_near, place_near_within, popover, tooltip, trusted_prompt,
};
pub use rejection::Rejection;
// Kept for apps written against it (FreeSpeak) until the next minor release.
#[allow(deprecated)]
pub use text::label;
pub use text::{heading, link, text};
pub use values::{slider, stepper};
