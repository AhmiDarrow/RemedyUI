//! How a scroll container shows its scroll bars: the style properties the view resolves and
//! the compositor draws with (as CSS `scrollbar-width` and `scrollbar-color`, plus when they
//! show).

use crate::Color;

/// How thick a scroll container's scroll bars are, as CSS `scrollbar-width`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum ScrollbarWidth {
    /// The platform's own thickness ([`ScrollbarWidth::AUTO_PX`]).
    #[default]
    Auto,
    /// The platform's thin bars ([`ScrollbarWidth::THIN_PX`]).
    Thin,
    /// No scroll bars; the container still scrolls.
    None,
    /// This many logical pixels.
    Px(f32),
}

impl ScrollbarWidth {
    /// The thickness of [`ScrollbarWidth::Auto`] bars, in logical pixels: what browsers give a
    /// bar of `scrollbar-width: auto` on Windows and Linux.
    pub const AUTO_PX: f32 = 12.0;
    /// The thickness of [`ScrollbarWidth::Thin`] bars, in logical pixels (Firefox's thin bar).
    pub const THIN_PX: f32 = 8.0;

    /// The thickness in logical pixels; 0 for none.
    ///
    /// ```
    /// use remedy_ui_core::ScrollbarWidth;
    /// assert_eq!(ScrollbarWidth::Thin.px(), 8.0);
    /// assert_eq!(ScrollbarWidth::None.px(), 0.0);
    /// assert_eq!(ScrollbarWidth::Px(-3.0).px(), 0.0);
    /// ```
    pub fn px(self) -> f32 {
        match self {
            ScrollbarWidth::Auto => Self::AUTO_PX,
            ScrollbarWidth::Thin => Self::THIN_PX,
            ScrollbarWidth::None => 0.0,
            ScrollbarWidth::Px(v) if v.is_finite() => v.max(0.0),
            ScrollbarWidth::Px(_) => 0.0,
        }
    }
}

/// When a scroll container shows its scroll bars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScrollbarVisibility {
    /// Overlay bars, as browsers draw them on Windows 11, macOS, Android and in GTK: drawn
    /// over the content while it scrolls and for a moment after, then faded out.
    #[default]
    Auto,
    /// Always drawn while the content overflows.
    Always,
    /// Never drawn; the container still scrolls.
    Never,
}

/// The colors of a scroll bar's thumb and track, as CSS `scrollbar-color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarColor {
    /// The thumb, the part that shows where the view is.
    pub thumb: Color,
    /// The track it runs in; transparent for none.
    pub track: Color,
}

impl ScrollbarColor {
    /// A `thumb` over a `track`.
    pub fn new(thumb: Color, track: Color) -> Self {
        ScrollbarColor { thumb, track }
    }
}
