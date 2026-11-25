//! RemedyUI's form widgets, built on the public view API and the core widgets.
//!
//! - [`number_input`] with exact [`Decimal`] values, units, precision and the locale's way
//!   of writing numbers ([`NumberSymbols`])
//! - [`color_picker`] with a WCAG contrast readout ([`contrast_grade`])
//! - [`key_capture`] for recording shortcuts
//! - [`calendar`], [`date_picker`] and [`time_picker`] over [`Date`] and [`Time`], with the
//!   locale's month and weekday names, first weekday and 12/24-hour clock
//!   ([`CalendarSymbols`])
//! - [`Form`]: validation as values change, dirty tracking against the last save, and a
//!   submit state ([`form_field`], [`form_view`], [`submit_button`], [`form_status`])
//!
//! Values and locale data (numbers, dates, times, names) come from `remedy-ui-services-i18n`,
//! which reads ICU4X compiled data; what an app does not use is not linked in.
//! See `docs/design/15-widgets-and-media.md`.

/// The form widgets' own words (status lines, error messages, screen reader names, keyboard
/// hints) in every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a
/// language, with [`remedy_ui_engine::view::strings::add_messages`] under the name `widgets-forms`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("widgets-forms");

/// Message `id` of this pack in the window's locale, kept current as the locale changes
/// when read inside a property closure or binding.
pub(crate) fn tr(ui: &remedy_ui_engine::view::Ui, id: &'static str) -> String {
    STRINGS.get(ui, id)
}

mod color;
mod date;
mod form;
mod keys;
mod number;

pub use color::{Hsv, color_picker, contrast_grade, contrast_grade_label};
pub use date::{calendar, date_picker, time_picker};
// The values and locale data the widgets work with, from the localization service.
pub use form::{Form, FormField, Submission, SubmitState, form_field, form_status, form_view, submit_button};
pub use keys::key_capture;
pub use number::{NumberOptions, number_input, number_input_within};
pub use remedy_ui_services_i18n::{
    CalendarSymbols, Date, Decimal, DecimalError, MAX_SCALE, NumberSymbols, Time, Weekday, days_in_month,
    is_leap, month_grid,
};
