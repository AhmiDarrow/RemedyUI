//! RemedyUI localization.
//!
//! - [`Messages`]: the app's strings in Fluent (`.ftl`) files, per locale, chosen for each
//!   window's locale with fallback; plurals and selectors; numbers written the locale's way;
//!   message folders read at run time so users can override any string; live reload while
//!   the app runs ([`Messages::watch`]). Missing messages show as `⟨id⟩` and are listed in
//!   [`Messages::problems`].
//! - Locale data: [`NumberSymbols`] (digits, separators, grouping) and [`CalendarSymbols`]
//!   (month and weekday names, first weekday, 12/24-hour clock, day periods), read from
//!   ICU4X compiled data; what an app does not use is not linked in.
//! - Values: exact [`Decimal`] numbers and [`Date`], [`Time`] and [`Weekday`], with the
//!   arithmetic forms and messages need; [`Date::today`] reads the user's local calendar.

mod calendar;
mod catalog;
mod decimal;
mod locale;
mod messages;

pub use calendar::{Date, Time, Weekday, days_in_month, is_leap, month_grid};
pub use catalog::{Catalog, fill};
pub use decimal::{Decimal, DecimalError, MAX_SCALE};
pub use locale::{CalendarSymbols, NumberSymbols};
pub use messages::{Arg, Messages, Problem};
