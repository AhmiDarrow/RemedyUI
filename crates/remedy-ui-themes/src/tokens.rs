//! Themes as files: the W3C Design Tokens format (JSON), so a user's theme, a designer's
//! export and an app's own skin format all meet in one place. Every token a [`Theme`] has
//! is written; reading starts from a base theme and changes only the tokens the file has, so
//! a file can be a whole theme or a few edits.
//!
//! ```json
//! { "$name": "Ocean",
//!   "color": { "bg": { "$type": "color", "$value": "#0d1b2a" },
//!              "custom": { "speaking": { "$type": "color", "$value": "#7ab08c" } } },
//!   "size": { "body": { "$type": "dimension", "$value": "14px" } } }
//! ```

use std::time::Duration;

use remedy_ui_core::Color;
use remedy_ui_style::{Density, Theme};
use serde_json::{Map, Value, json};

/// Why a theme file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenError {
    /// Not JSON, or not an object at the top.
    Invalid(String),
    /// A token's value is not what its place needs (a color that is not a color...).
    Value {
        /// Where, as a dotted path (`color.bg`).
        token: String,
        /// What was wrong.
        reason: String,
    },
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Invalid(why) => write!(f, "not a design tokens file: {why}"),
            TokenError::Value { token, reason } => write!(f, "token {token}: {reason}"),
        }
    }
}

impl std::error::Error for TokenError {}

fn color_token(c: Color) -> Value {
    json!({ "$type": "color", "$value": c.to_hex() })
}

fn px(v: f32) -> Value {
    json!({ "$type": "dimension", "$value": format!("{v}px") })
}

fn ms(d: Duration) -> Value {
    json!({ "$type": "duration", "$value": format!("{}ms", d.as_millis()) })
}

/// `theme` as a design tokens file.
pub fn to_tokens(theme: &Theme) -> String {
    let c = &theme.colors;
    let mut colors = Map::new();
    for (k, v) in [
        ("bg", c.bg),
        ("panel", c.panel),
        ("card", c.card),
        ("text", c.text),
        ("muted", c.muted),
        ("accent", c.accent),
        ("on-accent", c.on_accent),
        ("danger", c.danger),
        ("warn", c.warn),
        ("ok", c.ok),
    ] {
        colors.insert(k.into(), color_token(v));
    }
    if let Some(line) = c.line {
        colors.insert("line".into(), color_token(line));
    }
    if !c.custom.is_empty() {
        let custom: Map<String, Value> = c.custom.iter().map(|(k, v)| (k.clone(), color_token(*v))).collect();
        colors.insert("custom".into(), Value::Object(custom));
    }
    let doc = json!({
        "$name": theme.name,
        "color": colors,
        "font": {
            "family": { "$type": "fontFamily", "$value": theme.font_families },
            "mono": { "$type": "fontFamily", "$value": theme.mono_families },
        },
        "size": {
            "small": px(theme.type_scale.small),
            "body": px(theme.type_scale.body),
            "title": px(theme.type_scale.title),
        },
        "spacing": {
            "unit": px(theme.spacing.unit),
            "control-height": px(theme.spacing.control_height),
        },
        "radius": {
            "small": px(theme.radii.small),
            "medium": px(theme.radii.medium),
            "large": px(theme.radii.large),
        },
        "stroke": px(theme.stroke),
        "motion": {
            "quick": ms(theme.motion.quick),
            "standard": ms(theme.motion.standard),
            "emphasized": ms(theme.motion.emphasized),
        },
        "density": { "$type": "string", "$value": match theme.density {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
            Density::Spacious => "spacious",
        } },
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// A token's `$value` (or the value itself, for files that skip the wrapper).
fn value(v: &Value) -> &Value {
    v.get("$value").unwrap_or(v)
}

fn read_color(v: &Value, path: &str) -> Result<Color, TokenError> {
    let s = value(v).as_str().ok_or_else(|| bad(path, "a color is a string such as \"#1a2b3c\""))?;
    Color::from_hex(s).map_err(|_| bad(path, &format!("{s:?} is not a color")))
}

fn read_px(v: &Value, path: &str) -> Result<f32, TokenError> {
    let n = match value(v) {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().trim_end_matches("px").trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite() && *n >= 0.0)
    .ok_or_else(|| bad(path, "a size is a number of pixels, such as \"14px\""))?;
    Ok(n as f32)
}

fn read_ms(v: &Value, path: &str) -> Result<Duration, TokenError> {
    let n = match value(v) {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim();
            match s.strip_suffix("ms") {
                Some(m) => m.trim().parse::<f64>().ok(),
                None => s.trim_end_matches('s').trim().parse::<f64>().ok().map(|v| v * 1000.0),
            }
        }
        _ => None,
    }
    .filter(|n| n.is_finite() && *n >= 0.0)
    .ok_or_else(|| bad(path, "a duration is a number of milliseconds, such as \"150ms\""))?;
    Ok(Duration::from_secs_f64(n / 1000.0))
}

fn read_families(v: &Value, path: &str) -> Result<Vec<String>, TokenError> {
    match value(v) {
        Value::String(s) => Ok(s
            .split(',')
            .map(|f| f.trim().trim_matches('"').to_owned())
            .filter(|f| !f.is_empty())
            .collect()),
        Value::Array(a) => a
            .iter()
            .map(|f| f.as_str().map(str::to_owned).ok_or_else(|| bad(path, "font families are names")))
            .collect(),
        _ => Err(bad(path, "font families are a name or a list of names")),
    }
}

fn bad(path: &str, reason: &str) -> TokenError {
    TokenError::Value { token: path.to_owned(), reason: reason.to_owned() }
}

/// `base` with the tokens `json` holds. Tokens the file leaves out keep `base`'s values;
/// tokens RemedyUI does not know are ignored (other tools' tokens can share the file).
pub fn from_tokens(json: &str, base: &Theme) -> Result<Theme, TokenError> {
    let doc: Value = serde_json::from_str(json).map_err(|e| TokenError::Invalid(e.to_string()))?;
    let doc = doc.as_object().ok_or_else(|| TokenError::Invalid("the top is not an object".into()))?;
    let mut t = base.clone();
    if let Some(name) = doc.get("$name").and_then(Value::as_str) {
        t.name = name.to_owned();
    }
    let get = |group: &str, key: &str| doc.get(group).and_then(|g| g.get(key));
    let c = &mut t.colors;
    for (key, slot) in [
        ("bg", &mut c.bg),
        ("panel", &mut c.panel),
        ("card", &mut c.card),
        ("text", &mut c.text),
        ("muted", &mut c.muted),
        ("accent", &mut c.accent),
        ("on-accent", &mut c.on_accent),
        ("danger", &mut c.danger),
        ("warn", &mut c.warn),
        ("ok", &mut c.ok),
    ] {
        if let Some(v) = get("color", key) {
            *slot = read_color(v, &format!("color.{key}"))?;
        }
    }
    if let Some(v) = get("color", "line") {
        c.line = Some(read_color(v, "color.line")?);
    }
    if let Some(custom) = get("color", "custom").and_then(Value::as_object) {
        for (k, v) in custom {
            c.custom.insert(k.clone(), read_color(v, &format!("color.custom.{k}"))?);
        }
    }
    if let Some(v) = get("font", "family") {
        t.font_families = read_families(v, "font.family")?;
    }
    if let Some(v) = get("font", "mono") {
        t.mono_families = read_families(v, "font.mono")?;
    }
    for (key, slot) in [
        ("small", &mut t.type_scale.small),
        ("body", &mut t.type_scale.body),
        ("title", &mut t.type_scale.title),
    ] {
        if let Some(v) = get("size", key) {
            *slot = read_px(v, &format!("size.{key}"))?;
        }
    }
    if let Some(v) = get("spacing", "unit") {
        t.spacing.unit = read_px(v, "spacing.unit")?;
    }
    if let Some(v) = get("spacing", "control-height") {
        t.spacing.control_height = read_px(v, "spacing.control-height")?;
    }
    for (key, slot) in
        [("small", &mut t.radii.small), ("medium", &mut t.radii.medium), ("large", &mut t.radii.large)]
    {
        if let Some(v) = get("radius", key) {
            *slot = read_px(v, &format!("radius.{key}"))?;
        }
    }
    if let Some(v) = doc.get("stroke") {
        t.stroke = read_px(v, "stroke")?;
    }
    for (key, slot) in [
        ("quick", &mut t.motion.quick),
        ("standard", &mut t.motion.standard),
        ("emphasized", &mut t.motion.emphasized),
    ] {
        if let Some(v) = get("motion", key) {
            *slot = read_ms(v, &format!("motion.{key}"))?;
        }
    }
    if let Some(v) = doc.get("density") {
        t.density = match value(v).as_str() {
            Some("compact") => Density::Compact,
            Some("comfortable") => Density::Comfortable,
            Some("spacious") => Density::Spacious,
            _ => return Err(bad("density", "density is compact, comfortable or spacious")),
        };
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_round_trips() {
        for p in crate::PALETTES {
            let t = crate::theme(p);
            let back = from_tokens(&to_tokens(&t), &Theme::dark_forest());
            assert_eq!(back.as_ref(), Ok(&t), "{}", p.id);
        }
    }

    #[test]
    fn a_file_with_a_few_tokens_edits_the_base() {
        let base = Theme::dark_forest();
        let t = from_tokens(
            r##"{ "$name": "Mine", "color": { "accent": { "$value": "#ff8800" }, "custom": { "speaking": "#7ab08c" } },
                 "size": { "body": "15px" }, "motion": { "quick": "0.1s" }, "other-tool": { "x": 1 } }"##,
            &base,
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(t.name, "Mine");
        assert_eq!(t.colors.accent, Color::rgb8(0xff, 0x88, 0));
        assert_eq!(t.colors.custom.get("speaking"), Some(&Color::rgb8(0x7a, 0xb0, 0x8c)));
        assert_eq!(t.type_scale.body, 15.0);
        assert_eq!(t.motion.quick, Duration::from_millis(100));
        assert_eq!(t.colors.bg, base.colors.bg, "untouched tokens stay");
    }

    #[test]
    fn bad_values_say_where() {
        let base = Theme::dark_forest();
        assert_eq!(
            from_tokens(r#"{ "color": { "bg": { "$value": "teal-ish" } } }"#, &base),
            Err(TokenError::Value { token: "color.bg".into(), reason: "\"teal-ish\" is not a color".into() })
        );
        assert!(matches!(from_tokens("[1]", &base), Err(TokenError::Invalid(_))));
        assert!(matches!(
            from_tokens(r#"{ "size": { "body": "-3px" } }"#, &base),
            Err(TokenError::Value { .. })
        ));
    }
}
