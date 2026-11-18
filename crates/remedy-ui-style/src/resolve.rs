//! Resolving an element's style: widget default, theme entry, element properties.

use std::collections::HashMap;

use remedy_ui_core::{Color, Insets};

use crate::states::States;

/// Visual properties a style sets. Every field is optional so a patch changes only what it
/// names.
///
/// Build one from [`StylePatch::new`] with a method per property; properties are added over
/// time, so a patch is never written as a struct literal outside this crate:
///
/// ```
/// use remedy_ui_core::Color;
/// use remedy_ui_style::StylePatch;
///
/// let patch = StylePatch::new().background(Color::rgb8(30, 60, 90)).radius(6.0).text_size(15.0);
/// assert_eq!(patch.radius, Some(6.0));
/// assert_eq!(patch.foreground, None);
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct StylePatch {
    /// Background fill.
    pub background: Option<Color>,
    /// Text and icon color.
    pub foreground: Option<Color>,
    /// Border color.
    pub border: Option<Color>,
    /// Border width.
    pub border_width: Option<f32>,
    /// Corner radius.
    pub radius: Option<f32>,
    /// Padding.
    pub padding: Option<Insets>,
    /// Text size.
    pub text_size: Option<f32>,
    /// Text weight.
    pub weight: Option<f32>,
    /// Opacity, 0 to 1.
    pub opacity: Option<f32>,
    /// Text in the theme's monospace families (codes, keys, numbers that must line up).
    pub mono: Option<bool>,
    /// The focus ring's color, drawn while the element has keyboard focus.
    pub outline: Option<Color>,
    /// The focus ring's width; 0 for elements that show focus another way.
    pub outline_width: Option<f32>,
    /// The gap between the element's edge and its focus ring; negative draws it inside.
    pub outline_offset: Option<f32>,
}

impl StylePatch {
    /// A patch that sets nothing.
    pub fn new() -> Self {
        StylePatch::default()
    }

    /// Sets the background fill.
    pub fn background(mut self, value: Color) -> Self {
        self.background = Some(value);
        self
    }

    /// Sets the text and icon color.
    pub fn foreground(mut self, value: Color) -> Self {
        self.foreground = Some(value);
        self
    }

    /// Sets the border color.
    pub fn border(mut self, value: Color) -> Self {
        self.border = Some(value);
        self
    }

    /// Sets the border width.
    pub fn border_width(mut self, value: f32) -> Self {
        self.border_width = Some(value);
        self
    }

    /// Sets the corner radius.
    pub fn radius(mut self, value: f32) -> Self {
        self.radius = Some(value);
        self
    }

    /// Sets the padding.
    pub fn padding(mut self, value: Insets) -> Self {
        self.padding = Some(value);
        self
    }

    /// Sets the text size.
    pub fn text_size(mut self, value: f32) -> Self {
        self.text_size = Some(value);
        self
    }

    /// Sets the text weight.
    pub fn weight(mut self, value: f32) -> Self {
        self.weight = Some(value);
        self
    }

    /// Sets the opacity, 0 to 1.
    pub fn opacity(mut self, value: f32) -> Self {
        self.opacity = Some(value);
        self
    }

    /// Sets whether text uses the theme's monospace families.
    pub fn mono(mut self, value: bool) -> Self {
        self.mono = Some(value);
        self
    }

    /// Sets the focus ring's color.
    pub fn outline(mut self, value: Color) -> Self {
        self.outline = Some(value);
        self
    }

    /// Sets the focus ring's width.
    pub fn outline_width(mut self, value: f32) -> Self {
        self.outline_width = Some(value);
        self
    }

    /// Sets the gap between the element's edge and its focus ring.
    pub fn outline_offset(mut self, value: f32) -> Self {
        self.outline_offset = Some(value);
        self
    }

    /// The engine's starting style under `theme`, before any rule: transparent boxes, body
    /// text in the theme's text color, and the theme's focus ring for keyboard focus (as
    /// `:focus-visible` does on the web) unless a rule shows focus another way.
    pub fn defaults(theme: &crate::Theme) -> StylePatch {
        StylePatch {
            background: Some(Color::TRANSPARENT),
            foreground: Some(theme.derive().text),
            border: Some(Color::TRANSPARENT),
            border_width: Some(0.0),
            radius: Some(0.0),
            padding: Some(Insets::ZERO),
            text_size: Some(theme.type_scale.body),
            weight: Some(400.0),
            opacity: Some(1.0),
            mono: Some(false),
            outline: Some(theme.derive().focus),
            outline_width: Some(2.0),
            outline_offset: Some(2.0),
        }
    }

    /// `other`'s set fields over this patch's.
    pub fn merge(&self, other: &StylePatch) -> StylePatch {
        StylePatch {
            background: other.background.or(self.background),
            foreground: other.foreground.or(self.foreground),
            border: other.border.or(self.border),
            border_width: other.border_width.or(self.border_width),
            radius: other.radius.or(self.radius),
            padding: other.padding.or(self.padding),
            text_size: other.text_size.or(self.text_size),
            weight: other.weight.or(self.weight),
            opacity: other.opacity.or(self.opacity),
            mono: other.mono.or(self.mono),
            outline: other.outline.or(self.outline),
            outline_width: other.outline_width.or(self.outline_width),
            outline_offset: other.outline_offset.or(self.outline_offset),
        }
    }
}

/// A complete style, every property decided.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle {
    /// Background fill.
    pub background: Color,
    /// Text and icon color.
    pub foreground: Color,
    /// Border color.
    pub border: Color,
    /// Border width.
    pub border_width: f32,
    /// Corner radius.
    pub radius: f32,
    /// Padding.
    pub padding: Insets,
    /// Text size.
    pub text_size: f32,
    /// Text weight.
    pub weight: f32,
    /// Opacity.
    pub opacity: f32,
    /// Monospace text.
    pub mono: bool,
    /// The focus ring's color.
    pub outline: Color,
    /// The focus ring's width (0: none).
    pub outline_width: f32,
    /// The focus ring's gap from the edge (negative: inside).
    pub outline_offset: f32,
}

impl ResolvedStyle {
    fn from_patch(p: &StylePatch) -> ResolvedStyle {
        ResolvedStyle {
            background: p.background.unwrap_or(Color::TRANSPARENT),
            foreground: p.foreground.unwrap_or(Color::BLACK),
            border: p.border.unwrap_or(Color::TRANSPARENT),
            border_width: p.border_width.unwrap_or(0.0),
            radius: p.radius.unwrap_or(0.0),
            padding: p.padding.unwrap_or(Insets::ZERO),
            text_size: p.text_size.unwrap_or(14.0),
            weight: p.weight.unwrap_or(400.0),
            opacity: p.opacity.unwrap_or(1.0),
            mono: p.mono.unwrap_or(false),
            outline: p.outline.unwrap_or(Color::TRANSPARENT),
            outline_width: p.outline_width.unwrap_or(0.0),
            outline_offset: p.outline_offset.unwrap_or(0.0),
        }
    }

    /// True when keyboard focus on an element styled `self` draws a ring.
    pub fn shows_focus_ring(&self) -> bool {
        self.outline_width > 0.0 && self.outline.a > 0.0
    }
}

/// What a resolved style is cached by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StyleKey {
    /// The widget kind, such as `"button"`.
    pub widget: &'static str,
    /// The variant, such as `"primary"`.
    pub variant: &'static str,
    /// The element's states.
    pub states: States,
}

#[derive(Clone, Debug)]
struct Rule {
    widget: &'static str,
    variant: Option<&'static str>,
    states: States,
    patch: StylePatch,
}

/// The theme's style rules for widgets, and a cache of resolved styles. Changing the sheet or
/// the theme bumps its version and empties the cache.
#[derive(Clone, Debug, Default)]
pub struct StyleSheet {
    rules: Vec<Rule>,
    version: u64,
    cache: HashMap<StyleKey, StylePatch>,
    hits: u64,
    misses: u64,
}

impl ResolvedStyle {
    /// The defaults for a child of an element styled `self`: text properties (color, size,
    /// weight, monospace) are inherited, as in CSS; box properties (background, border,
    /// radius, padding, opacity, focus ring) start fresh from `base`.
    pub fn inherited_by_children(&self, base: &StylePatch) -> StylePatch {
        StylePatch {
            foreground: Some(self.foreground),
            text_size: Some(self.text_size),
            weight: Some(self.weight),
            mono: Some(self.mono),
            ..base.clone()
        }
    }
}

impl StyleSheet {
    /// An empty sheet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a rule: `patch` applies to `widget` (and only its `variant` if given) when the
    /// element has at least `states`. Later rules win over earlier ones, and rules that need
    /// more states win over rules that need fewer.
    pub fn rule(
        &mut self,
        widget: &'static str,
        variant: Option<&'static str>,
        states: States,
        patch: StylePatch,
    ) {
        self.rules.push(Rule { widget, variant, states, patch });
        self.invalidate();
    }

    /// Empties the cache (call after the theme changes).
    pub fn invalidate(&mut self) {
        self.version += 1;
        self.cache.clear();
    }

    /// The sheet's version; it changes whenever rules or the theme change.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The style for `key`, starting from `defaults` (what the element inherits and the
    /// engine's defaults), then the sheet's rules for the key, then `local` (properties set on
    /// the element). The rules' combined patch is cached per key, so elements with different
    /// inherited values share it.
    pub fn resolve(&mut self, key: &StyleKey, defaults: &StylePatch, local: &StylePatch) -> ResolvedStyle {
        let rules = match self.cache.get(key) {
            Some(p) => {
                self.hits += 1;
                p.clone()
            }
            None => {
                self.misses += 1;
                let mut matching: Vec<&Rule> = self
                    .rules
                    .iter()
                    .filter(|r| r.widget == key.widget)
                    .filter(|r| r.variant.is_none_or(|v| v == key.variant))
                    .filter(|r| key.states.contains(r.states))
                    .collect();
                // Fewer required states first, so more specific rules apply last; stable sort
                // keeps declaration order among equals.
                matching.sort_by_key(|r| r.states.bits().count_ones() + u32::from(r.variant.is_some()));
                let patch = matching.iter().fold(StylePatch::default(), |acc, r| acc.merge(&r.patch));
                self.cache.insert(key.clone(), patch.clone());
                patch
            }
        };
        ResolvedStyle::from_patch(&defaults.merge(&rules).merge(local))
    }

    /// (cache hits, cache misses), for tests and the performance overlay.
    pub fn cache_stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}
