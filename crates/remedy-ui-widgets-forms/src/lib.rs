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
//! Locale data comes from ICU4X compiled data; what an app does not use is not linked in.
//! See `docs/design/15-widgets-and-media.md`.

mod color;
mod date;
mod decimal;
mod form;
mod keys;
mod locale;
mod number;

pub use color::{Hsv, color_picker, contrast_grade};
pub use date::{Date, Time, Weekday, calendar, date_picker, days_in_month, is_leap, month_grid, time_picker};
pub use decimal::{Decimal, DecimalError, MAX_SCALE};
pub use form::{Form, FormField, Submission, SubmitState, form_field, form_status, form_view, submit_button};
pub use keys::key_capture;
pub use locale::{CalendarSymbols, NumberSymbols};
pub use number::{NumberOptions, number_input};
