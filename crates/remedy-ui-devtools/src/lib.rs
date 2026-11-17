//! RemedyUI devtools, version 1.
//!
//! - **Inspector** ([`Inspector`], [`inspector_panel`]): pick an element in a window to see
//!   its widget, the source line that built it, its box, resolved style, states, semantics
//!   and bindings; browse the element tree; run the accessibility check ([`audit`]: unnamed
//!   controls, low-contrast text, small targets); edit theme colors live.
//! - **Previews** ([`Preview`]): render a piece of UI in every theme, locale (right-to-left
//!   ones mirrored), window size and text scale asked for, show them side by side
//!   ([`gallery`]), and check them against golden images ([`check_golden`]).
//!
//! The `remedy-ui` facade opens these as windows with its `devtools` feature (F12 opens
//! the inspector). See `docs/design/12-developer-experience.md`.

mod inspect;
mod preview;

/// The theme and style sheet devtools windows draw with (the first Remedy palette and the
/// widget sheet), whatever the inspected app uses.
pub fn tool_style() -> (remedy_ui_style::Theme, remedy_ui_style::StyleSheet) {
    let theme = remedy_ui_themes::PALETTES
        .first()
        .map_or_else(remedy_ui_style::Theme::dark_forest, remedy_ui_themes::theme);
    let sheet = remedy_ui_themes::sheet(&theme);
    (theme, sheet)
}

pub use inspect::{
    COLOR_ROLES, Inspector, Problem, ProblemKind, SheetFn, audit, describe, inspector_panel, walk,
};
pub use preview::{
    GOLDEN_TOLERANCE, GoldenError, Preview, PreviewBuilder, Snapshot, Variation, background, check_golden,
    gallery, is_rtl,
};
