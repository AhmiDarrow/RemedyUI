//! Resolving an element's style: widget default, theme entry, element properties.

use std::collections::HashMap;

use remedy_ui_core::{Color, Insets};

use crate::states::States;

/// Visual properties a style sets. Every field is optional so a patch changes only what it
/// names.
#[derive(Clone, Debug, Default, PartialEq)]
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
}

impl StylePatch {
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
        }
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
    /// radius, padding, opacity) start fresh from `base`.
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
