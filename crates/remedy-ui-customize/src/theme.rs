//! The person's theme: a W3C design-tokens file in the config folder holding only the tokens
//! they changed (plus anything else the file carries, kept as it was), applied over the app's
//! theme; shared theme files imported as untrusted data; contrast warnings.

use remedy_ui_view::words::en;
use std::path::{Path, PathBuf};
use std::time::Duration;

use remedy_ui_core::Color;
use remedy_ui_style::{Density, Theme};
use remedy_ui_view::Ui;
use serde_json::{Map, Value, json};

use crate::{Area, Customizations, CustomizeError, Notice, files};

/// The theme file's name in the config folder.
pub(crate) const THEME_FILE: &str = "theme.json";

/// One design token, by its place in the tokens file (`color.accent`, `size.body`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Token(Vec<String>);

/// What kind of value a token holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TokenKind {
    /// The theme's name.
    Name,
    /// A color.
    Color,
    /// A size in logical pixels.
    Dimension,
    /// A duration.
    Duration,
    /// Compact, comfortable or spacious.
    Density,
    /// A list of font families, first choice first.
    FontFamily,
    /// An image file the theme brings (its background).
    Image,
    /// A font file the theme brings.
    Font,
    /// Something RemedyUI does not read (another tool's token).
    Other,
}

/// A token's value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TokenValue {
    /// The theme's name.
    Name(String),
    /// A color.
    Color(Color),
    /// A size in logical pixels.
    Px(f32),
    /// A duration.
    Duration(Duration),
    /// Compact, comfortable or spacious.
    Density(Density),
    /// Font families, first choice first.
    Families(Vec<String>),
    /// A file: a path (relative to the config folder) or a `data:` URI.
    File(String),
}

/// The color roles every theme has, by token key, with what people call them (in English;
/// [`remedy_ui_view::words::word_or`] gives a locale's).
pub const COLOR_ROLES: &[(&str, &str)] = &[
    ("bg", en("Background")),
    ("panel", en("Panels")),
    ("card", en("Cards")),
    ("text", en("Text")),
    ("muted", en("Muted text")),
    ("accent", en("Accent")),
    ("on-accent", en("Text on accent")),
    ("danger", en("Danger")),
    ("warn", en("Warning")),
    ("ok", en("Success")),
    ("line", en("Lines")),
];

impl Token {
    /// A token by its path in the file: `["color", "accent"]`.
    pub fn new<S: Into<String>>(path: impl IntoIterator<Item = S>) -> Token {
        Token(path.into_iter().map(Into::into).collect())
    }

    /// The theme's name.
    pub fn name() -> Token {
        Token::new(["$name"])
    }

    /// A color role (`bg`, `panel`, `card`, `text`, `muted`, `accent`, `on-accent`, `danger`,
    /// `warn`, `ok`, `line`).
    pub fn color(role: &str) -> Token {
        Token::new(["color", role])
    }

    /// An app-defined color role.
    pub fn custom_color(name: &str) -> Token {
        Token::new(["color", "custom", name])
    }

    /// A text size: `small`, `body` or `title`.
    pub fn size(which: &str) -> Token {
        Token::new(["size", which])
    }

    /// The spacing unit every gap and padding is a multiple of.
    pub fn spacing_unit() -> Token {
        Token::new(["spacing", "unit"])
    }

    /// The height of buttons, fields and other controls.
    pub fn control_height() -> Token {
        Token::new(["spacing", "control-height"])
    }

    /// A corner radius: `small`, `medium` or `large`.
    pub fn radius(which: &str) -> Token {
        Token::new(["radius", which])
    }

    /// The width of borders and lines.
    pub fn stroke() -> Token {
        Token::new(["stroke"])
    }

    /// A motion duration: `quick`, `standard` or `emphasized`.
    pub fn motion(which: &str) -> Token {
        Token::new(["motion", which])
    }

    /// How tightly controls are packed.
    pub fn density() -> Token {
        Token::new(["density"])
    }

    /// The text font families.
    pub fn font_family() -> Token {
        Token::new(["font", "family"])
    }

    /// The monospace font families.
    pub fn mono_family() -> Token {
        Token::new(["font", "mono"])
    }

    /// The image behind each window's content.
    pub fn background_image() -> Token {
        Token::new(["asset", "background"])
    }

    /// The font file the theme brings for `family`.
    pub fn theme_font(family: &str) -> Token {
        Token::new(["asset", "font", family])
    }

    /// Its path in the file.
    pub fn path(&self) -> &[String] {
        &self.0
    }

    /// Its path written with dots (`color.on-accent`).
    pub fn dotted(&self) -> String {
        self.0.join(".")
    }

    /// What kind of value it holds.
    pub fn kind(&self) -> TokenKind {
        let p: Vec<&str> = self.0.iter().map(String::as_str).collect();
        match p.as_slice() {
            ["$name"] => TokenKind::Name,
            ["color", _] | ["color", "custom", _] => TokenKind::Color,
            ["font", "family" | "mono"] => TokenKind::FontFamily,
            ["size", "small" | "body" | "title"]
            | ["spacing", "unit" | "control-height"]
            | ["radius", "small" | "medium" | "large"]
            | ["stroke"] => TokenKind::Dimension,
            ["motion", "quick" | "standard" | "emphasized"] => TokenKind::Duration,
            ["density"] => TokenKind::Density,
            ["asset", "background"] => TokenKind::Image,
            ["asset", "font", _] => TokenKind::Font,
            _ => TokenKind::Other,
        }
    }

    /// What people call it ("Accent", "Body text size"), in English.
    pub fn label(&self) -> String {
        self.label_in("en")
    }

    /// [`Token::label`] in `locale` (BCP 47), with RemedyUI's built-in words; English where it
    /// has none.
    pub fn label_in(&self, locale: &str) -> String {
        use remedy_ui_view::words::{en, fill, word_or};
        let p: Vec<&str> = self.0.iter().map(String::as_str).collect();
        let s = match p.as_slice() {
            ["$name"] => en("Theme name"),
            ["color", "custom", name] => return fill(word_or(locale, "Color “{name}”"), &[("name", name)]),
            ["color", role] => match COLOR_ROLES.iter().find(|(k, _)| k == role) {
                Some((_, l)) => l,
                None => return fill(word_or(locale, "Color “{name}”"), &[("name", role)]),
            },
            ["font", "family"] => en("Font"),
            ["font", "mono"] => en("Monospace font"),
            ["size", "small"] => en("Small text size"),
            ["size", "body"] => en("Body text size"),
            ["size", "title"] => en("Title size"),
            ["spacing", "unit"] => en("Spacing unit"),
            ["spacing", "control-height"] => en("Control height"),
            ["radius", "small"] => en("Small corner radius"),
            ["radius", "medium"] => en("Medium corner radius"),
            ["radius", "large"] => en("Large corner radius"),
            ["stroke"] => en("Line width"),
            ["motion", "quick"] => en("Quick motion"),
            ["motion", "standard"] => en("Standard motion"),
            ["motion", "emphasized"] => en("Emphasized motion"),
            ["density"] => en("Density"),
            ["asset", "background"] => en("Background image"),
            ["asset", "font", name] => return fill(word_or(locale, "Font file “{name}”"), &[("name", name)]),
            _ => return self.dotted(),
        };
        word_or(locale, s).to_owned()
    }

    /// Its value in `theme` (`None` for a token the theme does not have).
    pub fn value_in(&self, theme: &Theme) -> Option<TokenValue> {
        let p: Vec<&str> = self.0.iter().map(String::as_str).collect();
        let c = &theme.colors;
        Some(match p.as_slice() {
            ["$name"] => TokenValue::Name(theme.name.clone()),
            ["color", "custom", name] => TokenValue::Color(*c.custom.get(*name)?),
            ["color", role] => TokenValue::Color(match *role {
                "bg" => c.bg,
                "panel" => c.panel,
                "card" => c.card,
                "text" => c.text,
                "muted" => c.muted,
                "accent" => c.accent,
                "on-accent" => c.on_accent,
                "danger" => c.danger,
                "warn" => c.warn,
                "ok" => c.ok,
                "line" => theme.derive().line,
                _ => return None,
            }),
            ["font", "family"] => TokenValue::Families(theme.font_families.clone()),
            ["font", "mono"] => TokenValue::Families(theme.mono_families.clone()),
            ["size", "small"] => TokenValue::Px(theme.type_scale.small),
            ["size", "body"] => TokenValue::Px(theme.type_scale.body),
            ["size", "title"] => TokenValue::Px(theme.type_scale.title),
            ["spacing", "unit"] => TokenValue::Px(theme.spacing.unit),
            ["spacing", "control-height"] => TokenValue::Px(theme.spacing.control_height),
            ["radius", "small"] => TokenValue::Px(theme.radii.small),
            ["radius", "medium"] => TokenValue::Px(theme.radii.medium),
            ["radius", "large"] => TokenValue::Px(theme.radii.large),
            ["stroke"] => TokenValue::Px(theme.stroke),
            ["motion", "quick"] => TokenValue::Duration(theme.motion.quick),
            ["motion", "standard"] => TokenValue::Duration(theme.motion.standard),
            ["motion", "emphasized"] => TokenValue::Duration(theme.motion.emphasized),
            ["density"] => TokenValue::Density(theme.density),
            ["asset", "background"] => TokenValue::File(theme.assets.background.clone()?),
            ["asset", "font", name] => TokenValue::File(theme.assets.fonts.get(*name)?.clone()),
            _ => return None,
        })
    }

    /// Every token a theme editor offers for `theme`: the theme's name, its color roles and
    /// app-defined colors, fonts, text sizes, spacing, radii, line width, motion and density.
    pub fn editable(theme: &Theme) -> Vec<Token> {
        let mut v = vec![Token::name()];
        v.extend(COLOR_ROLES.iter().map(|(k, _)| Token::color(k)));
        v.extend(theme.colors.custom.keys().map(|k| Token::custom_color(k)));
        v.extend([Token::font_family(), Token::mono_family()]);
        v.extend(["small", "body", "title"].map(Token::size));
        v.extend([Token::spacing_unit(), Token::control_height()]);
        v.extend(["small", "medium", "large"].map(Token::radius));
        v.push(Token::stroke());
        v.extend(["quick", "standard", "emphasized"].map(Token::motion));
        v.push(Token::density());
        v
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.dotted())
    }
}

impl TokenValue {
    /// The kind of token it fits.
    pub fn kind(&self) -> TokenKind {
        match self {
            TokenValue::Name(_) => TokenKind::Name,
            TokenValue::Color(_) => TokenKind::Color,
            TokenValue::Px(_) => TokenKind::Dimension,
            TokenValue::Duration(_) => TokenKind::Duration,
            TokenValue::Density(_) => TokenKind::Density,
            TokenValue::Families(_) => TokenKind::FontFamily,
            TokenValue::File(_) => TokenKind::Image,
        }
    }

    /// As a token in the file, written exactly as [`remedy_ui_themes::to_tokens`] writes it.
    fn to_json(&self) -> Value {
        match self {
            TokenValue::Name(n) => Value::String(n.clone()),
            TokenValue::Color(c) => json!({ "$type": "color", "$value": c.to_hex() }),
            TokenValue::Px(v) => json!({ "$type": "dimension", "$value": format!("{v}px") }),
            TokenValue::Duration(d) => {
                json!({ "$type": "duration", "$value": format!("{}ms", d.as_millis()) })
            }
            TokenValue::Density(d) => json!({ "$type": "string", "$value": density_name(*d) }),
            TokenValue::Families(f) => json!({ "$type": "fontFamily", "$value": f }),
            TokenValue::File(r) => json!({ "$type": "image", "$value": r }),
        }
    }
}

/// A density's name in the file.
pub fn density_name(d: Density) -> &'static str {
    d.name()
}

// --- Token trees -------------------------------------------------------------------------

/// The leaves of a tokens document: tokens (objects with a `$value`) and plain values, by
/// path. Groups are walked into.
fn leaves(v: &Value, prefix: &mut Vec<String>, out: &mut Vec<(Vec<String>, Value)>) {
    match v {
        Value::Object(m) if !m.contains_key("$value") => {
            for (k, child) in m {
                prefix.push(k.clone());
                leaves(child, prefix, out);
                prefix.pop();
            }
        }
        other => {
            if !prefix.is_empty() {
                out.push((prefix.clone(), other.clone()));
            }
        }
    }
}

fn leaves_of(doc: &Map<String, Value>) -> Vec<(Vec<String>, Value)> {
    let mut out = Vec::new();
    leaves(&Value::Object(doc.clone()), &mut Vec::new(), &mut out);
    out
}

/// The leaf at `path`, if there is one.
fn leaf<'a>(doc: &'a Map<String, Value>, path: &[String]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    let mut v = doc.get(first)?;
    for k in rest {
        v = v.as_object().filter(|m| !m.contains_key("$value"))?.get(k)?;
    }
    Some(v)
}

/// Sets the leaf at `path`, making groups as needed (a leaf in the way becomes a group).
fn set_leaf(doc: &mut Map<String, Value>, path: &[String], value: Value) {
    let Some((last, groups)) = path.split_last() else { return };
    let mut m = doc;
    for k in groups {
        let slot = m.entry(k.clone()).or_insert_with(|| Value::Object(Map::new()));
        if !slot.as_object().is_some_and(|o| !o.contains_key("$value")) {
            *slot = Value::Object(Map::new());
        }
        let Value::Object(next) = slot else { return };
        m = next;
    }
    m.insert(last.clone(), value);
}

/// Removes the leaf at `path` and any group it leaves empty.
fn remove_leaf(doc: &mut Map<String, Value>, path: &[String]) {
    fn go(m: &mut Map<String, Value>, path: &[String]) {
        let Some((first, rest)) = path.split_first() else { return };
        if rest.is_empty() {
            m.remove(first);
            return;
        }
        let empty = match m.get_mut(first) {
            Some(Value::Object(child)) if !child.contains_key("$value") => {
                go(child, rest);
                child.is_empty()
            }
            _ => false,
        };
        if empty {
            m.remove(first);
        }
    }
    go(doc, path);
}

/// The tokens file for `theme` over `base`: only the tokens that differ from `base`, and
/// every leaf of `extras` that RemedyUI does not read (kept as it was).
fn diff_doc(base: &Theme, theme: &Theme, extras: &Map<String, Value>) -> Map<String, Value> {
    let parse = |t: &Theme| match serde_json::from_str::<Value>(&remedy_ui_themes::to_tokens(t)) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let (b, t) = (parse(base), parse(theme));
    let mut out = Map::new();
    let known = leaves_of(&t);
    for (path, v) in &known {
        if leaf(&b, path) != Some(v) {
            set_leaf(&mut out, path, v.clone());
        }
    }
    for (path, v) in leaves_of(extras) {
        if !known.iter().any(|(p, _)| *p == path) {
            set_leaf(&mut out, &path, v);
        }
    }
    out
}

/// A token as the file holds it.
fn token_json(token: &Token, value: &TokenValue) -> Value {
    match (token.kind(), value) {
        (TokenKind::Font, TokenValue::File(r)) => json!({ "$type": "font", "$value": r }),
        _ => value.to_json(),
    }
}

/// Sets `token` to `value` in a tokens document.
pub(crate) fn put_token(doc: &mut Map<String, Value>, token: &Token, value: &TokenValue) {
    set_leaf(doc, token.path(), token_json(token, value));
}

/// Removes the token at `path` from a tokens document.
pub(crate) fn remove_token(doc: &mut Map<String, Value>, path: &[String]) {
    remove_leaf(doc, path);
}

fn doc_text(doc: &Map<String, Value>) -> String {
    serde_json::to_string(&Value::Object(doc.clone())).unwrap_or_else(|_| "{}".into())
}

// --- Untrusted themes --------------------------------------------------------------------

/// What a shared theme may carry beyond plain tokens. Everything is refused unless turned on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportPolicy {
    /// Keep shader tokens (code that runs on the GPU).
    pub allow_shaders: bool,
    /// Keep external links (`https:`, `file:` and other URLs; inline `data:` is not one).
    /// Links are kept in the theme but never fetched.
    pub allow_external_urls: bool,
    /// Use the files the theme brings: its background image and fonts (copied into the
    /// config folder, each checked first).
    pub allow_assets: bool,
    /// Import the rest of a theme without what it refuses, instead of refusing the theme.
    pub drop_refused: bool,
}

impl ImportPolicy {
    /// Sets `allow_shaders`: keep shader tokens (code that runs on the GPU).
    pub fn allow_shaders(mut self, value: bool) -> Self {
        self.allow_shaders = value;
        self
    }

    /// Sets `allow_external_urls`: keep external links (`https:`, `file:` and other URLs; inline `data:` is not one).
    pub fn allow_external_urls(mut self, value: bool) -> Self {
        self.allow_external_urls = value;
        self
    }

    /// Sets `allow_assets`: use the files the theme brings: its background image and fonts (copied into the config folder, each checked first).
    pub fn allow_assets(mut self, value: bool) -> Self {
        self.allow_assets = value;
        self
    }

    /// Sets `drop_refused`: import the rest of a theme without what it refuses, instead of refusing the theme.
    pub fn drop_refused(mut self, value: bool) -> Self {
        self.drop_refused = value;
        self
    }
}

/// Why part of a shared theme was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefusalKind {
    /// A shader: code, not data.
    Shader,
    /// A link to something outside the file.
    ExternalUrl,
    /// A file the theme brings (an image or a font).
    Asset,
}

/// One part of a shared theme that its policy refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refusal {
    /// Where, as a dotted path (`background.$extensions.image`).
    pub token: String,
    /// What it is.
    pub kind: RefusalKind,
    /// What it held (the URL, or the shader's type).
    pub detail: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            RefusalKind::Shader => write!(f, "{}: a shader ({})", self.token, self.detail),
            RefusalKind::ExternalUrl => write!(f, "{}: an external link ({})", self.token, self.detail),
            RefusalKind::Asset => write!(f, "{}: a file the theme brings ({})", self.token, self.detail),
        }
    }
}

/// What importing a shared theme did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportReport {
    /// The theme's name.
    pub name: String,
    /// What was left out (with [`ImportPolicy::drop_refused`]).
    pub dropped: Vec<Refusal>,
}

const SHADER_TYPES: &[&str] = &["shader", "wgsl", "glsl", "hlsl", "msl", "spirv", "spir-v"];

/// Whether `s` points outside the file: a URL with a scheme (other than inline `data:`), a
/// protocol-relative `//host`, or either inside CSS's `url(…)`.
fn is_external_url(s: &str) -> bool {
    let mut t = s.trim();
    if let Some(inner) = t.strip_prefix("url(").or_else(|| t.strip_prefix("URL(")) {
        t = inner.trim_end_matches(')').trim().trim_matches(|c| c == '"' || c == '\'');
    }
    if t.starts_with("//") {
        return true;
    }
    let Some((scheme, _)) = t.split_once(':') else { return false };
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    // One letter is a Windows drive ("C:"), not a scheme.
    valid && scheme.len() > 1 && !scheme.eq_ignore_ascii_case("data")
}

fn scan(v: &Value, path: &mut Vec<String>, out: &mut Vec<Refusal>) {
    let here = |path: &[String]| if path.is_empty() { "(top)".to_owned() } else { path.join(".") };
    match v {
        Value::Object(m) => {
            if let Some(t) = m.get("$type").and_then(Value::as_str)
                && SHADER_TYPES.contains(&t.to_ascii_lowercase().as_str())
            {
                out.push(Refusal { token: here(path), kind: RefusalKind::Shader, detail: t.to_owned() });
                return;
            }
            for (k, child) in m {
                path.push(k.clone());
                if matches!(k.to_ascii_lowercase().as_str(), "shader" | "shaders") {
                    out.push(Refusal { token: here(path), kind: RefusalKind::Shader, detail: k.clone() });
                } else {
                    scan(child, path, out);
                }
                path.pop();
            }
        }
        Value::Array(a) => {
            for (i, child) in a.iter().enumerate() {
                path.push(i.to_string());
                scan(child, path, out);
                path.pop();
            }
        }
        Value::String(s) => {
            if path.first().is_some_and(|p| p == "asset") && path.last().is_none_or(|l| l != "$type") {
                out.push(Refusal { token: here(path), kind: RefusalKind::Asset, detail: s.clone() });
            }
            if is_external_url(s) {
                out.push(Refusal { token: here(path), kind: RefusalKind::ExternalUrl, detail: s.clone() });
            }
        }
        _ => {}
    }
}

/// Removes whatever sits at a refusal's dotted path, and any group left empty.
fn drop_at(v: &mut Value, path: &[&str]) {
    let Some((first, rest)) = path.split_first() else { return };
    match v {
        Value::Object(m) => {
            if rest.is_empty() {
                m.remove(*first);
                return;
            }
            if let Some(child) = m.get_mut(*first) {
                drop_at(child, rest);
                if child.as_object().is_some_and(Map::is_empty) || child.as_array().is_some_and(Vec::is_empty)
                {
                    m.remove(*first);
                }
            }
        }
        Value::Array(a) => {
            let Ok(i) = first.parse::<usize>() else { return };
            if rest.is_empty() {
                if i < a.len() {
                    a.remove(i);
                }
            } else if let Some(child) = a.get_mut(i) {
                drop_at(child, rest);
            }
        }
        _ => {}
    }
}

/// Everything in a shared theme file that is more than tokens: shaders and external links.
/// The file is parsed as data and nothing in it is followed or run.
pub fn inspect_theme(json: &str) -> Result<Vec<Refusal>, CustomizeError> {
    let doc: Value = serde_json::from_str(json).map_err(|e| CustomizeError::Invalid(e.to_string()))?;
    if !doc.is_object() {
        return Err(CustomizeError::Invalid("a theme file is a JSON object".into()));
    }
    let mut out = Vec::new();
    scan(&doc, &mut Vec::new(), &mut out);
    Ok(out)
}

// --- Contrast ----------------------------------------------------------------------------

/// A pair of theme colors that people read one on the other, below the WCAG ratio it needs.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ContrastWarning {
    /// What the pair is ("Text on cards").
    pub pair: &'static str,
    /// The color on top.
    pub foreground: Token,
    /// The color under it.
    pub background: Token,
    /// The contrast ratio now.
    pub ratio: f32,
    /// The ratio WCAG asks for: 4.5 for text, 3 for controls and large marks.
    pub needed: f32,
}

/// The color pairs a theme is read in, and the ratio each needs.
const PAIRS: &[(&str, &str, &str, f32)] = &[
    (en("Text on the background"), "text", "bg", 4.5),
    (en("Text on panels"), "text", "panel", 4.5),
    (en("Text on cards"), "text", "card", 4.5),
    (en("Muted text on the background"), "muted", "bg", 4.5),
    (en("Muted text on panels"), "muted", "panel", 4.5),
    (en("Muted text on cards"), "muted", "card", 4.5),
    (en("Text on the accent"), "on-accent", "accent", 4.5),
    (en("The accent on the background"), "accent", "bg", 3.0),
];

/// The color pairs in `theme` below the contrast WCAG asks for (empty when all pass). Text
/// colors are taken as the widgets draw them: the engine already moves text, muted text and
/// text on the accent toward contrast with the background and the accent
/// ([`Theme::derive`]), so what is left to warn about is what that cannot fix, such as text
/// on panels and cards far from the background.
pub fn contrast_warnings(theme: &Theme) -> Vec<ContrastWarning> {
    let d = theme.derive();
    let color = |role: &str| {
        let c = match role {
            "text" => d.text,
            "muted" => d.muted,
            "on-accent" => d.on_accent,
            _ => match Token::color(role).value_in(theme) {
                Some(TokenValue::Color(c)) => c,
                _ => theme.colors.bg,
            },
        };
        c.with_alpha(1.0)
    };
    PAIRS
        .iter()
        .filter_map(|(pair, fg, bg, needed)| {
            let ratio = color(fg).contrast(color(bg));
            (ratio < *needed).then(|| ContrastWarning {
                pair,
                foreground: Token::color(fg),
                background: Token::color(bg),
                ratio,
                needed: *needed,
            })
        })
        .collect()
}

// --- The area ----------------------------------------------------------------------------

/// The theme as customized: the app's theme, the file's tokens, and the result.
pub(crate) struct ThemeArea {
    pub(crate) default: Theme,
    /// The file's document when it loaded (empty when there is none or it failed).
    pub(crate) doc: Map<String, Value>,
    pub(crate) current: remedy_ui_reactive::Signal<Theme>,
    /// The theme's fonts, checked, and its background image.
    pub(crate) fonts: Vec<crate::assets::LoadedFont>,
    pub(crate) background: Option<(String, remedy_ui_media_image::Source)>,
    /// What each window was given of them.
    pub(crate) windows: Vec<crate::assets::WindowAssets>,
}

impl Customizations {
    /// The theme file.
    pub fn theme_file(&self) -> PathBuf {
        self.0.dir.join(THEME_FILE)
    }

    /// Turns on theme customization over `default`, the theme the app would use: the
    /// person's theme file is read now (a bad one is skipped with a notice) and the result
    /// applied to every attached window.
    pub fn enable_theme(&self, default: Theme) {
        let current = self.in_owner(|| remedy_ui_reactive::Signal::new(default.clone()));
        *self.0.theme.borrow_mut() = Some(ThemeArea {
            default,
            doc: Map::new(),
            current,
            fonts: Vec::new(),
            background: None,
            windows: Vec::new(),
        });
        self.reload(&[Area::Theme]);
    }

    /// Changes the theme the person's tokens apply over (the platform switched to dark, or
    /// to high contrast). Not an undoable change: it is not the person's.
    pub fn set_default_theme(&self, default: Theme) {
        if let Some(a) = self.0.theme.borrow_mut().as_mut() {
            a.default = default;
        }
        self.reload(&[Area::Theme]);
    }

    /// The theme with the person's changes (tracked), or `None` while themes are not on.
    pub fn theme(&self) -> Option<Theme> {
        let current = self.0.theme.borrow().as_ref().map(|a| a.current)?;
        current.try_get().ok()
    }

    /// The app's theme, under the person's changes.
    pub fn default_theme(&self) -> Option<Theme> {
        self.0.theme.borrow().as_ref().map(|a| a.default.clone())
    }

    /// Whether the person changed `token` (tracked).
    pub fn is_customized(&self, token: &Token) -> bool {
        self.0.revision.get();
        self.0.theme.borrow().as_ref().is_some_and(|a| leaf(&a.doc, token.path()).is_some())
    }

    /// The tokens the person changed, as the file holds them (tracked).
    pub fn customized_tokens(&self) -> Vec<Token> {
        self.0.revision.get();
        self.0
            .theme
            .borrow()
            .as_ref()
            .map(|a| leaves_of(&a.doc).into_iter().map(|(p, _)| Token(p)).collect())
            .unwrap_or_default()
    }

    /// Sets one token, live, as one undoable change in `ui`'s history (a run of changes to the
    /// same token, such as a color dragged across its picker, is one entry). A value equal to
    /// the app's leaves the token uncustomized.
    pub fn set_token(&self, ui: &Ui, token: &Token, value: TokenValue) -> Result<(), CustomizeError> {
        let fits = token.kind() == value.kind()
            || (token.kind() == TokenKind::Font && matches!(value, TokenValue::File(_)));
        if !fits || token.kind() == TokenKind::Other {
            return Err(CustomizeError::Invalid(format!(
                "{} does not hold that kind of value",
                token.dotted()
            )));
        }
        let (default, mut doc) = self.theme_parts()?;
        let leaf_value = token_json(token, &value);
        let base_leaf = match serde_json::from_str::<Value>(&remedy_ui_themes::to_tokens(&default)) {
            Ok(Value::Object(m)) => leaf(&m, token.path()).cloned(),
            _ => None,
        };
        if base_leaf.as_ref() == Some(&leaf_value) {
            remove_leaf(&mut doc, token.path());
        } else {
            set_leaf(&mut doc, token.path(), leaf_value);
        }
        remedy_ui_themes::from_tokens(&doc_text(&doc), &default).map_err(CustomizeError::Token)?;
        let label = remedy_ui_view::words::fill(
            &ui.word("Change {name}"),
            &[("name", &token.label_in(&ui.locale()))],
        );
        self.change(ui, label, Some(format!("theme:{}", token.dotted())), &[Area::Theme], |c| {
            c.write_theme(&doc)
        })
    }

    /// Takes one token back to the app's value (undoable).
    pub fn reset_token(&self, ui: &Ui, token: &Token) -> Result<(), CustomizeError> {
        let (_, mut doc) = self.theme_parts()?;
        if leaf(&doc, token.path()).is_none() {
            return Ok(());
        }
        remove_leaf(&mut doc, token.path());
        let label =
            remedy_ui_view::words::fill(&ui.word("Reset {name}"), &[("name", &token.label_in(&ui.locale()))]);
        self.change(ui, label, None, &[Area::Theme], |c| c.write_theme(&doc))
    }

    /// Takes the whole theme back to the app's (undoable).
    pub fn reset_theme(&self, ui: &Ui) -> Result<(), CustomizeError> {
        self.theme_parts()?;
        let all = self.files_of(Area::Theme);
        self.change(ui, ui.word("Reset theme"), None, &[Area::Theme], |_| {
            for p in &all {
                files::remove(p).map_err(|e| CustomizeError::io(p, &e))?;
            }
            Ok(())
        })
    }

    /// Imports a shared theme file's text (from another person, a download) as untrusted
    /// data: shaders and external links are refused unless `policy` allows them (or left out,
    /// with [`ImportPolicy::drop_refused`]), and every token is checked before anything
    /// changes. The imported theme replaces the person's theme, as one undoable change.
    pub fn import_theme(
        &self,
        ui: &Ui,
        json: &str,
        policy: ImportPolicy,
    ) -> Result<ImportReport, CustomizeError> {
        self.import_theme_from(ui, json, None, policy)
    }

    fn import_theme_from(
        &self,
        ui: &Ui,
        json: &str,
        base: Option<&Path>,
        policy: ImportPolicy,
    ) -> Result<ImportReport, CustomizeError> {
        let (default, _) = self.theme_parts()?;
        let mut doc: Value =
            serde_json::from_str(json).map_err(|e| CustomizeError::Invalid(e.to_string()))?;
        if !doc.is_object() {
            return Err(CustomizeError::Invalid("a theme file is a JSON object".into()));
        }
        let mut found = Vec::new();
        scan(&doc, &mut Vec::new(), &mut found);
        let refused: Vec<Refusal> = found
            .into_iter()
            .filter(|r| match r.kind {
                RefusalKind::Shader => !policy.allow_shaders,
                RefusalKind::ExternalUrl => !policy.allow_external_urls,
                RefusalKind::Asset => !policy.allow_assets,
            })
            .collect();
        if !refused.is_empty() {
            if !policy.drop_refused {
                return Err(CustomizeError::Refused(refused));
            }
            // Deepest first, so array positions stay right while removing.
            let mut paths: Vec<Vec<&str>> = refused
                .iter()
                .map(|r| {
                    let mut p: Vec<&str> = r.token.split('.').collect();
                    // A refused value takes its whole token with it.
                    if p.last() == Some(&"$value") {
                        p.pop();
                    }
                    p
                })
                .collect();
            paths.dedup();
            paths.sort_by_key(|p| std::cmp::Reverse(p.len()));
            for p in paths {
                drop_at(&mut doc, &p);
            }
        }
        let Value::Object(map) = doc else {
            return Err(CustomizeError::Invalid("a theme file is a JSON object".into()));
        };
        let theme =
            remedy_ui_themes::from_tokens(&doc_text(&map), &default).map_err(CustomizeError::Token)?;
        let mut file = diff_doc(&default, &theme, &map);
        let name = theme.name.clone();
        // Files are checked (and copied in) inside the change, so undo takes the copies too.
        self.change(
            ui,
            remedy_ui_view::words::fill(&ui.word("Import theme {name}"), &[("name", &name)]),
            None,
            &[Area::Theme],
            |c| {
                c.bring_assets(&mut file, base, None)?;
                c.write_theme(&file)
            },
        )?;
        Ok(ImportReport { name, dropped: if policy.drop_refused { refused } else { Vec::new() } })
    }

    /// [`Customizations::import_theme`] from a file.
    pub fn import_theme_file(
        &self,
        ui: &Ui,
        path: &Path,
        policy: ImportPolicy,
    ) -> Result<ImportReport, CustomizeError> {
        let text = std::fs::read_to_string(path).map_err(|e| CustomizeError::io(path, &e))?;
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        self.import_theme_from(ui, &text, Some(&base), policy)
    }

    /// The whole theme as a design-tokens file to share: every token (not only the changed
    /// ones), so it applies the same over any app's theme, plus what else the person's file
    /// carries.
    pub fn export_theme_text(&self) -> Result<String, CustomizeError> {
        let (default, doc) = self.theme_parts()?;
        // As the file says it: the theme's fonts by the names its tokens use.
        let theme =
            remedy_ui_themes::from_tokens(&doc_text(&doc), &default).map_err(CustomizeError::Token)?;
        let mut full = match serde_json::from_str::<Value>(&remedy_ui_themes::to_tokens(&theme)) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        };
        let known = leaves_of(&full);
        for (path, v) in leaves_of(&doc) {
            if !known.iter().any(|(p, _)| *p == path) {
                set_leaf(&mut full, &path, v);
            }
        }
        Ok(String::from_utf8_lossy(&files::json_text(&Value::Object(full))).into_owned())
    }

    /// Writes [`Customizations::export_theme_text`] to `path` (atomically), with the files
    /// the theme brings copied beside it into a `<name>-assets` folder, so the theme imports
    /// whole elsewhere.
    pub fn export_theme(&self, path: &Path) -> Result<(), CustomizeError> {
        let text = self.export_theme_text()?;
        let mut doc = match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        };
        self.send_assets(&mut doc, path)?;
        files::write_atomic(path, &files::json_text(&Value::Object(doc)))
            .map_err(|e| CustomizeError::io(path, &e))
    }

    pub(crate) fn theme_parts(&self) -> Result<(Theme, Map<String, Value>), CustomizeError> {
        self.0
            .theme
            .borrow()
            .as_ref()
            .map(|a| (a.default.clone(), a.doc.clone()))
            .ok_or(CustomizeError::NotEnabled(Area::Theme))
    }

    pub(crate) fn write_theme(&self, doc: &Map<String, Value>) -> Result<(), CustomizeError> {
        self.prune_assets(doc)?;
        let path = self.theme_file();
        let bytes = (!doc.is_empty()).then(|| files::json_text(&Value::Object(doc.clone())));
        files::put(&path, bytes.as_deref()).map_err(|e| CustomizeError::io(&path, &e))
    }

    /// Reads the theme file and applies the result to every attached window.
    pub(crate) fn reload_theme(&self) {
        let Some((default, current)) = self.0.theme.borrow().as_ref().map(|a| (a.default.clone(), a.current))
        else {
            return;
        };
        let path = self.theme_file();
        let mut notices = Vec::new();
        let (doc, theme) = match files::read(&path) {
            Ok(None) => (Map::new(), default.clone()),
            Ok(Some(bytes)) => {
                let parsed = String::from_utf8(bytes)
                    .map_err(|_| "the file is not UTF-8 text".to_owned())
                    .and_then(|text| {
                        let doc = match serde_json::from_str::<Value>(&text) {
                            Ok(Value::Object(m)) => m,
                            Ok(_) => return Err("a theme file is a JSON object".to_owned()),
                            Err(e) => return Err(format!("not a design tokens file: {e}")),
                        };
                        let theme =
                            remedy_ui_themes::from_tokens(&text, &default).map_err(|e| e.to_string())?;
                        Ok((doc, theme))
                    });
                match parsed {
                    Ok(v) => v,
                    Err(message) => {
                        notices.push(Notice { area: Area::Theme, path: path.clone(), message });
                        (Map::new(), default.clone())
                    }
                }
            }
            Err(e) => {
                notices.push(Notice { area: Area::Theme, path: path.clone(), message: e.to_string() });
                (Map::new(), default.clone())
            }
        };
        let loaded = self.load_assets(&theme);
        let theme = loaded.theme;
        notices.extend(loaded.notices);
        if let Some(a) = self.0.theme.borrow_mut().as_mut() {
            a.doc = doc;
            a.fonts = loaded.fonts;
            a.background = loaded.background;
        }
        self.set_notices(Area::Theme, None, notices);
        current.set_if_changed(theme.clone());
        self.apply_theme_to_windows(&theme);
    }

    /// Styles every attached window with `theme`, when it does not already have it.
    pub(crate) fn apply_theme_to_windows(&self, theme: &Theme) {
        let windows: Vec<Ui> =
            self.0.windows.borrow().iter().filter_map(remedy_ui_view::WeakUi::upgrade).collect();
        for ui in windows {
            // Fonts first, so the families the theme names are there when it applies.
            self.apply_assets(&ui);
            if ui.theme() != *theme {
                ui.set_theme(theme.clone(), remedy_ui_themes::sheet(theme));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_told_from_paths_and_data() {
        for yes in [
            "https://x.example/bg.png",
            "url('http://x/y.png')",
            "//cdn.example/a",
            "file:///etc/passwd",
            "ftp://x",
        ] {
            assert!(is_external_url(yes), "{yes}");
        }
        for no in ["#102030", "14px", "Remedy Sans", "data:image/png;base64,AAAA", "C:/Windows/x.png", "a: b"]
        {
            assert!(!is_external_url(no), "{no}");
        }
    }

    #[test]
    fn leaves_are_set_and_removed_with_their_groups() {
        let mut doc = Map::new();
        let path = Token::custom_color("speaking").path().to_vec();
        set_leaf(&mut doc, &path, json!({"$value": "#ffffff"}));
        assert!(leaf(&doc, &path).is_some());
        remove_leaf(&mut doc, &path);
        assert!(doc.is_empty(), "empty groups go too");
    }
}
