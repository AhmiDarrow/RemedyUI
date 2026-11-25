//! Tokens on elements: style and layout lengths given as design tokens resolve through the
//! window's theme, and again whenever it changes.

use remedy_ui_core::layout::{LayoutStyle, Length};
use remedy_ui_core::style::{Dp, StylePatch, Theme};

use crate::element::ElData;

/// A layout length an element can take as a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayoutField {
    Gap,
    Width,
    Height,
}

impl LayoutField {
    pub(crate) fn set(self, l: &mut LayoutStyle, v: f32) {
        match self {
            LayoutField::Gap => l.gap = (v, v),
            LayoutField::Width => l.width = Length::Px(v),
            LayoutField::Height => l.height = Length::Px(v),
        }
    }
}

/// The element's own style under `theme` (padding sides it leaves unset are 0 here; the
/// frame fills them from the widget's style).
pub(crate) fn resolve_local(d: &ElData, theme: &Theme) -> StylePatch {
    let merged = match &d.style_bound {
        Some(b) => d.style_src.merge(b),
        None => d.style_src.clone(),
    };
    merged.resolve(theme)
}

/// True when the element's padding sets only some sides.
pub(crate) fn partial_padding(d: &ElData) -> bool {
    d.style_src.has_partial_padding()
        || d.style_bound.as_ref().is_some_and(remedy_ui_core::style::Style::has_partial_padding)
}

/// The element's own style under `theme`, padding sides it leaves unset taken from `under`.
pub(crate) fn resolve_local_over(d: &ElData, theme: &Theme, under: remedy_ui_core::Insets) -> StylePatch {
    let merged = match &d.style_bound {
        Some(b) => d.style_src.merge(b),
        None => d.style_src.clone(),
    };
    merged.resolve_over(theme, under)
}

/// Writes the element's layout tokens under `theme` into its layout style.
pub(crate) fn apply_layout_tokens(d: &mut ElData, theme: &Theme) {
    let tokens: Vec<(LayoutField, Dp)> = d.layout_tokens.clone();
    for (f, v) in tokens {
        f.set(&mut d.layout, v.resolve(theme));
    }
}
