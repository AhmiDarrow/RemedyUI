//! RemedyUI styling: themes, derived tokens, states and style resolution.
//!
//! A [`Theme`] is a typed set of design tokens. [`DerivedTokens`] are computed from it once per
//! theme change (hover, pressed and selected fills; text colors balanced to WCAG contrast; the
//! six text sizes), not per widget. A style comes from exactly three places, in order: the
//! widget's default, the theme's entry for that widget, variant and state, and properties set
//! on the element. There is no selector cascade. Resolved styles are cached by (widget,
//! variant, state, theme version). See `docs/design/07-styling-and-customization.md`.
//!
//! ```
//! use remedy_ui_core::Color;
//! use remedy_ui_style::*;
//!
//! let theme = Theme::dark_forest();
//! let d = theme.derive();
//! // Body text meets WCAG AAA (7:1) on the background, muted text at least AA (4.5:1).
//! assert!(d.text.contrast(theme.colors.bg) >= 7.0);
//! assert!(d.muted.contrast(theme.colors.bg) >= 4.5);
//! ```

mod contrast;
mod gradient;
mod resolve;
mod states;
mod theme;
mod tokens;

pub use contrast::{CONTRAST_AA, CONTRAST_AAA, ensure_contrast};
pub use gradient::{Extent, Gradient, GradientGeometry, GradientStop, ParseGradientError, Pos, RadialSize};
pub use remedy_ui_core::TextTransform;
pub use resolve::{LetterSpacing, ResolvedStyle, Shadow, StickyInsets, StyleKey, StylePatch, StyleSheet};
pub use states::States;
pub use theme::{
    ColorRoles, Density, DerivedTokens, MotionTokens, Radii, Spacing, Theme, ThemeAssets, TypeScale,
};
pub use tokens::{ColorRef, ColorRole, Dp, Pad, RADIUS_FULL, Radius, Space, Style, TextRole};
