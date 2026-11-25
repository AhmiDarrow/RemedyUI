//! RemedyUI live audio visuals: the speaking ring, level meters, sparklines and bar groups.
//!
//! These are for things that move many times a second, like a voice level. Each one
//! follows a `Signal` the app updates from its audio thread or timer, smooths what it shows
//! so it does not flicker, and does no work while nothing changes: a silent ring schedules
//! no timers and a still sparkline paints nothing. Under reduced motion they switch state
//! instead of animating. All of them are described to screen readers, and the meter's
//! threshold is a slider that works from the keyboard.
//!
//! ```
//! use remedy_ui_core::reactive::Signal;
//! use remedy_ui_widgets::viz::{MeterOptions, level_meter};
//!
//! # fn demo(ui: &remedy_ui_engine::view::Ui) {
//! let level = Signal::new(0.0f32);
//! let threshold = Signal::new(0.2f32);
//! let meter = level_meter(
//!     ui,
//!     "Microphone",
//!     level,
//!     MeterOptions::default().threshold(threshold),
//! );
//! level.set(0.6); // from the audio callback's timer
//! # let _ = meter;
//! # }
//! ```

/// The crate's own words (screen reader values: levels, zones, states) in every language the
/// engine ships, from `i18n/*.ftl`. Apps reword them, or add a language, with
/// [`remedy_ui_engine::view::strings::add_messages`] under the name `viz`.
pub static STRINGS: remedy_ui_engine::view::strings::Strings = remedy_ui_engine::engine_strings!("viz");

mod bars;
mod drive;
mod meter;
mod peak;
mod ring;
mod series;
mod sparkline;

pub use bars::{BarsOptions, bars};
pub use drive::{Clock, Gate, Moved, Smoothing, Ticker, smooth};
pub use meter::{MeterOptions, Orientation, Zone, Zones, level_meter};
pub use peak::PeakHold;
pub use ring::{RingOptions, speaking_ring};
pub use series::{History, PlotBox, Range, Samples, extent, fraction, group_max, plot};
pub use sparkline::{SparklineOptions, sparkline};

/// A level kept to 0..1; a value that is not a number counts as silence.
pub fn unit_level(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}
