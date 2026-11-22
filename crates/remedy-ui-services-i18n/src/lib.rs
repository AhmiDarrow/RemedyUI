//! RemedyUI localization.
//!
//! - [`Messages`]: the app's strings in Fluent (`.ftl`) files, per locale, chosen for each
//!   window's locale with fallback; plurals and selectors; numbers written the locale's way;
//!   message folders read at run time so users can override any string; live reload while
//!   the app runs ([`Messages::watch`]). Missing messages show as `⟨id⟩` and are listed in
//!   [`Messages::problems`].
//! - Locale data: [`NumberSymbols`] (digits, separators, grouping, compact counts such as
//!   `1.2K`) and [`CalendarSymbols`] (month and weekday names, first weekday, 12/24-hour
//!   clock, day periods, long and medium dates, dates with times), read from ICU4X compiled
//!   data; what an app does not use is not linked in. [`language_name`] names a language in
//!   the window's language.
//! - Values: exact [`Decimal`] numbers and [`Date`], [`Time`] and [`Weekday`], with the
//!   arithmetic forms and messages need; [`Date::today`] reads the user's local calendar.

/// The service's own words (why typed text is not a number, the languages' names) in every language the engine
/// ships, from `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_view::strings::add_messages`] under the name `services-i18n`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("services-i18n");

mod calendar;
mod catalog;
mod decimal;
mod ftl;
mod locale;
mod messages;

pub use calendar::{Date, Time, Weekday, days_in_month, is_leap, month_grid};
pub use catalog::{Catalog, OverrideProblem, canonical as canonical_locale, fill};
pub use decimal::{Decimal, DecimalError, MAX_SCALE};
pub use ftl::{FtlError, MessageEntry, entry_source, read_entries, remove_entry, set_entry};
pub use locale::{CalendarSymbols, CompactOptions, NumberSymbols, language_name, language_name_in};
pub use messages::{Arg, Messages, Problem};
