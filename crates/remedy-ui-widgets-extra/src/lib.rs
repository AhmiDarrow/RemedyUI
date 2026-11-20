//! RemedyUI's extra widgets, for setup, pairing and small delights.
//!
//! - [`qr_code`]: a QR code for text that can change, from [`QrCode`], a generator for
//!   ISO/IEC 18004: numeric, alphanumeric, byte and Kanji modes mixed in the segments that take
//!   the fewest bits ([`segment_text`]), UTF-8 ECI headers where text needs them, versions 1 to
//!   40, error correction levels L, M, Q and H, and the mask chosen by penalty score
//! - [`rating`]: stars, with half steps if wanted, as a slider to assistive technology
//! - [`carousel`]: pages that slide, with swipe, arrows, dots, the keyboard and optional
//!   autoplay that respects reduced motion
//! - [`stepper_flow`]: a setup wizard with a progress indicator and per-step validation that
//!   gates Next
//! - [`color_swatch_grid`]: a grid of named swatches, one chosen, with arrow-key navigation
//!
//! Every widget is keyboard operable, described to assistive technology, takes its colors and
//! spacing from the theme and mirrors in right-to-left windows.
//! See `docs/design/15-widgets-and-media.md`.

/// The pack's own words (button labels, screen reader names and values, keyboard hints) in
/// every language the engine ships, from `i18n/*.ftl`. Apps reword them, or add a language,
/// with [`remedy_ui_view::strings::add_messages`] under the name `widgets-extra`.
pub static STRINGS: remedy_ui_view::strings::Strings = remedy_ui_view::engine_strings!("widgets-extra");

/// Message `id` of this pack in the window's locale, kept current as the locale changes
/// when read inside a property closure.
pub(crate) fn tr(ui: &remedy_ui_view::Ui, id: &'static str) -> String {
    STRINGS.get(ui, id)
}

mod carousel;
mod qr;
mod qr_code;
mod qr_segments;
mod rating;
mod stepper_flow;
mod swatches;

pub use carousel::{CarouselOptions, carousel};
pub use qr::{
    Ecc, EncodeOptions, Mode, QrCode, QrError, alignment_positions, block_layout, data_codewords,
    format_bits, rs_generator, rs_remainder, version_bits, version_size,
};
pub use qr_code::{QrOptions, qr_code};
pub use qr_segments::{ECI_UTF8, Segment, segment_text};
pub use rating::{RatingOptions, rating, star_path};
pub use stepper_flow::{Step, StepperLabels, stepper_flow};
pub use swatches::{Swatch, SwatchOptions, color_swatch_grid};
