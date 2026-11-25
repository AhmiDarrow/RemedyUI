//! Gradient backgrounds: CSS's `linear-gradient`, `radial-gradient` (circles and ellipses,
//! with the extent keywords) and `conic-gradient`, as values and parsed from CSS text, with
//! colors from theme roles. An element's `background_layers` draw over its `background` color,
//! the first layer on top, as CSS's `background-image` does.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::{Color, Point, Size, Vec2};

use crate::style::{ColorRef, ColorRole, Theme};

/// A color stop: a color (a value or a theme role), an alpha it is scaled by (CSS
/// `color-mix(in srgb, var(--gold) 16%, transparent)` is the gold role at alpha 0.16), and
/// where it sits along the gradient (0 to 1), or `None` to space it as CSS does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// The color.
    pub color: ColorRef,
    /// Multiplies the color's alpha, 0 to 1.
    pub alpha: f32,
    /// Where it sits, 0 to 1; `None` spaces it evenly between its neighbours.
    pub at: Option<f32>,
}

impl GradientStop {
    /// A stop of `color`, placed as CSS places a stop with no position.
    pub fn new(color: impl Into<ColorRef>) -> Self {
        GradientStop { color: color.into(), alpha: 1.0, at: None }
    }

    /// The stop at `at` (0 to 1) along the gradient.
    pub fn at(mut self, at: f32) -> Self {
        self.at = Some(at);
        self
    }

    /// The stop's color with its alpha scaled by `alpha`.
    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }
}

/// A coordinate or length: logical pixels, or a fraction of the box's width or height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pos {
    /// Logical pixels.
    Px(f32),
    /// A fraction of the box (CSS percent / 100).
    Fraction(f32),
}

impl Pos {
    /// The center: 50%.
    pub const CENTER: Pos = Pos::Fraction(0.5);

    /// In logical pixels along a side `len` long.
    pub fn px(self, len: f32) -> f32 {
        match self {
            Pos::Px(v) => v,
            Pos::Fraction(f) => f * len,
        }
    }
}

/// CSS's radial extent keywords: how far offset 1 reaches from the center.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    /// To the nearest side (of each axis, for an ellipse).
    ClosestSide,
    /// Through the nearest corner.
    ClosestCorner,
    /// To the farthest side (of each axis, for an ellipse).
    FarthestSide,
    /// Through the farthest corner (CSS's default).
    FarthestCorner,
}

/// The size of a radial gradient's offset-1 shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialSize {
    /// Reaching an [`Extent`].
    Extent(Extent),
    /// These radii: horizontal and vertical (a circle uses the first).
    Radii(Pos, Pos),
}

/// A gradient: what one background layer draws.
///
/// ```
/// use remedy_ui_core::Size;
/// use remedy_ui_core::style::*;
///
/// // Portal's page glow, as CSS writes it.
/// let g = Gradient::parse(
///     "radial-gradient(ellipse 72rem 28rem at -6% -12%, \
///      color-mix(in srgb, var(--gold) 16%, transparent), transparent 62%)",
/// )
/// .unwrap();
/// let GradientGeometry::Radial { center, radii } = g.geometry(Size::new(1000.0, 800.0)) else { panic!() };
/// let round = |v: f32| v.round();
/// assert_eq!([center.x, center.y, radii.x, radii.y].map(round), [-60.0, -96.0, 1152.0, 448.0]);
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Gradient {
    /// Along a line through the box's center at `angle` degrees (CSS: 0 up, 90 to the right).
    Linear {
        /// The direction, in degrees clockwise from up.
        angle: f32,
        /// The stops.
        stops: Vec<GradientStop>,
    },
    /// Out from a center, in a circle or an axis-aligned ellipse.
    Radial {
        /// A circle rather than an ellipse.
        circle: bool,
        /// How far offset 1 reaches.
        size: RadialSize,
        /// The center (CSS `at x y`).
        at: [Pos; 2],
        /// The stops.
        stops: Vec<GradientStop>,
    },
    /// Around a center, clockwise from `from` degrees (0 up).
    Conic {
        /// Where offset 0 is, in degrees clockwise from up.
        from: f32,
        /// The center.
        at: [Pos; 2],
        /// The stops.
        stops: Vec<GradientStop>,
    },
}

/// Where a gradient lies in a box, in the box's own logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum GradientGeometry {
    /// From `start` (offset 0) to `end` (offset 1).
    Linear {
        /// Offset 0.
        start: Point,
        /// Offset 1.
        end: Point,
    },
    /// Offset 1 on the ellipse with these radii around `center` (equal radii: a circle).
    Radial {
        /// The center.
        center: Point,
        /// The horizontal and vertical radii.
        radii: Vec2,
    },
    /// Around `center` from `start_angle` radians (from the positive x axis, clockwise).
    Conic {
        /// The center.
        center: Point,
        /// Offset 0's angle.
        start_angle: f32,
    },
}

/// Why CSS gradient text did not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseGradientError(pub String);

impl std::fmt::Display for ParseGradientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gradient: {}", self.0)
    }
}

impl std::error::Error for ParseGradientError {}

impl Gradient {
    /// A linear gradient at `angle` degrees (0 up, 90 to the right).
    pub fn linear(angle: f32, stops: impl IntoIterator<Item = GradientStop>) -> Gradient {
        Gradient::Linear { angle, stops: stops.into_iter().collect() }
    }

    /// An elliptical radial gradient centered `at`, reaching the farthest corner (change it
    /// with [`size`](Self::size)).
    pub fn ellipse(at: [Pos; 2], stops: impl IntoIterator<Item = GradientStop>) -> Gradient {
        Gradient::Radial {
            circle: false,
            size: RadialSize::Extent(Extent::FarthestCorner),
            at,
            stops: stops.into_iter().collect(),
        }
    }

    /// A circular radial gradient centered `at`, reaching the farthest corner.
    pub fn circle(at: [Pos; 2], stops: impl IntoIterator<Item = GradientStop>) -> Gradient {
        Gradient::Radial {
            circle: true,
            size: RadialSize::Extent(Extent::FarthestCorner),
            at,
            stops: stops.into_iter().collect(),
        }
    }

    /// A conic gradient centered `at`, starting `from` degrees (0 up) and going clockwise.
    pub fn conic(from: f32, at: [Pos; 2], stops: impl IntoIterator<Item = GradientStop>) -> Gradient {
        Gradient::Conic { from, at, stops: stops.into_iter().collect() }
    }

    /// The same radial gradient with offset 1 at `size` (other kinds are unchanged).
    pub fn size(mut self, new: RadialSize) -> Gradient {
        if let Gradient::Radial { size, .. } = &mut self {
            *size = new;
        }
        self
    }

    /// Its stops as written.
    pub fn stop_list(&self) -> &[GradientStop] {
        match self {
            Gradient::Linear { stops, .. }
            | Gradient::Radial { stops, .. }
            | Gradient::Conic { stops, .. } => stops,
        }
    }

    fn stop_list_mut(&mut self) -> &mut [GradientStop] {
        match self {
            Gradient::Linear { stops, .. }
            | Gradient::Radial { stops, .. }
            | Gradient::Conic { stops, .. } => stops,
        }
    }

    /// The same gradient with its theme roles resolved under `theme`.
    pub fn resolve(&self, theme: &Theme) -> Gradient {
        let mut g = self.clone();
        for s in g.stop_list_mut() {
            s.color = ColorRef::Value(s.color.resolve(theme));
        }
        g
    }

    /// The stops ready to draw: colors under `theme` with their alpha applied, and positions
    /// fixed up as CSS does (a missing first is 0, a missing last 1, missing ones between
    /// spread evenly, and none before the one ahead of it).
    pub fn stops(&self, theme: &Theme) -> Vec<(f32, Color)> {
        let list = self.stop_list();
        let mut at: Vec<Option<f32>> = list.iter().map(|s| s.at).collect();
        if let Some(first) = at.first_mut() {
            first.get_or_insert(0.0);
        }
        if let Some(last) = at.last_mut() {
            last.get_or_insert(1.0);
        }
        let mut max = f32::MIN;
        for a in at.iter_mut().flatten() {
            max = max.max(*a);
            *a = max;
        }
        // Missing ones between known neighbours spread evenly (the last is always known).
        let (mut prev, mut gap, mut filled) = (0.0f32, Vec::new(), Vec::with_capacity(at.len()));
        for a in at {
            match a {
                None => {
                    gap.push(filled.len());
                    filled.push(prev);
                }
                Some(v) => {
                    let steps = gap.len() as f32 + 1.0;
                    for (k, idx) in gap.drain(..).enumerate() {
                        if let Some(slot) = filled.get_mut(idx) {
                            *slot = prev + (v - prev) * (k as f32 + 1.0) / steps;
                        }
                    }
                    filled.push(v);
                    prev = v;
                }
            }
        }
        list.iter()
            .zip(filled)
            .map(|(s, a)| {
                let c = s.color.resolve(theme);
                // shortcut: positions outside 0..1 are clamped; CSS extends the line instead. Upgrade if a theme needs it.
                (a.clamp(0.0, 1.0), c.with_alpha(c.a * s.alpha.clamp(0.0, 1.0)))
            })
            .collect()
    }

    /// Where it lies in a box `size` large, as CSS places it.
    pub fn geometry(&self, size: Size) -> GradientGeometry {
        let (w, h) = (size.width, size.height);
        let center = |at: &[Pos; 2]| Point::new(at[0].px(w), at[1].px(h));
        match self {
            Gradient::Linear { angle, .. } => {
                let (s, c) = angle.to_radians().sin_cos();
                // Half the gradient line: long enough that its ends' perpendiculars touch the corners.
                let half = ((w * s).abs() + (h * c).abs()) / 2.0;
                let (dx, dy) = (s * half, -c * half);
                GradientGeometry::Linear {
                    start: Point::new(w / 2.0 - dx, h / 2.0 - dy),
                    end: Point::new(w / 2.0 + dx, h / 2.0 + dy),
                }
            }
            Gradient::Radial { circle, size, at, .. } => {
                let c = center(at);
                let (near_x, far_x) = ((c.x).abs().min((w - c.x).abs()), (c.x).abs().max((w - c.x).abs()));
                let (near_y, far_y) = ((c.y).abs().min((h - c.y).abs()), (c.y).abs().max((h - c.y).abs()));
                let radii = match (*size, *circle) {
                    (RadialSize::Radii(rx, _), true) => {
                        let r = rx.px(w);
                        Vec2::new(r, r)
                    }
                    (RadialSize::Radii(rx, ry), false) => Vec2::new(rx.px(w), ry.px(h)),
                    (RadialSize::Extent(e), true) => {
                        let r = match e {
                            Extent::ClosestSide => near_x.min(near_y),
                            Extent::FarthestSide => far_x.max(far_y),
                            Extent::ClosestCorner => near_x.hypot(near_y),
                            Extent::FarthestCorner => far_x.hypot(far_y),
                        };
                        Vec2::new(r, r)
                    }
                    (RadialSize::Extent(e), false) => match e {
                        Extent::ClosestSide => Vec2::new(near_x, near_y),
                        Extent::FarthestSide => Vec2::new(far_x, far_y),
                        // The side ellipse scaled to pass through the corner: by √2.
                        Extent::ClosestCorner => Vec2::new(near_x, near_y) * std::f32::consts::SQRT_2,
                        Extent::FarthestCorner => Vec2::new(far_x, far_y) * std::f32::consts::SQRT_2,
                    },
                };
                GradientGeometry::Radial { center: c, radii }
            }
            Gradient::Conic { from, at, .. } => {
                GradientGeometry::Conic { center: center(at), start_angle: (from - 90.0).to_radians() }
            }
        }
    }

    /// Parses one CSS gradient: `linear-gradient(...)`, `radial-gradient(...)` or
    /// `conic-gradient(...)`. Colors are `#hex`, `rgb()`/`rgba()`, `transparent`, `black`,
    /// `white`, a theme role as `var(--accent)` (any role's name; other names are the theme's
    /// custom colors), and `color-mix(in srgb, <color> N%, transparent)`. Lengths are `px`,
    /// `rem`/`em` (16 px) and `%`; angles `deg`, `turn`, `rad` and `grad`, or `to right` and
    /// the other sides.
    pub fn parse(css: &str) -> Result<Gradient, ParseGradientError> {
        let css = css.trim();
        let open = css.find('(').ok_or_else(|| err("no arguments"))?;
        let inner = css[open + 1..].strip_suffix(')').ok_or_else(|| err("no closing parenthesis"))?;
        let name = css[..open].trim();
        let args = split_top(inner, ',');
        let (prelude, stop_args) = match args.first().map(|a| parse_stops(a)) {
            Some(Ok(_)) | None => (None, &args[..]),
            Some(Err(_)) => (args.first().copied(), args.get(1..).unwrap_or_default()),
        };
        let mut stops = Vec::new();
        for a in stop_args {
            stops.extend(parse_stops(a)?);
        }
        if stops.len() < 2 {
            return Err(err("a gradient needs at least two stops"));
        }
        let words: Vec<&str> = prelude.map(|p| split_top(p, ' ')).unwrap_or_default();
        match name {
            "linear-gradient" => {
                let angle = match words.as_slice() {
                    [] => 180.0,
                    ["to", side] => match *side {
                        "top" => 0.0,
                        "right" => 90.0,
                        "bottom" => 180.0,
                        "left" => 270.0,
                        // shortcut: corner directions (`to top right`) depend on the box; not supported until a theme needs them.
                        other => return Err(err(&format!("unknown direction `{other}`"))),
                    },
                    [a] => angle(a)?,
                    _ => return Err(err(&format!("unknown direction `{}`", words.join(" ")))),
                };
                Ok(Gradient::Linear { angle, stops })
            }
            "radial-gradient" => {
                let (shape, at) = split_at_keyword(&words);
                let at = position(at)?;
                let (mut circle, mut extent, mut lengths) = (None, None, Vec::new());
                for w in shape {
                    match *w {
                        "circle" => circle = Some(true),
                        "ellipse" => circle = Some(false),
                        "closest-side" => extent = Some(Extent::ClosestSide),
                        "closest-corner" => extent = Some(Extent::ClosestCorner),
                        "farthest-side" => extent = Some(Extent::FarthestSide),
                        "farthest-corner" => extent = Some(Extent::FarthestCorner),
                        other => lengths.push(length(other)?),
                    }
                }
                let circle = circle.unwrap_or(lengths.len() == 1);
                let size = match (lengths.as_slice(), extent) {
                    ([], e) => RadialSize::Extent(e.unwrap_or(Extent::FarthestCorner)),
                    ([r], None) if circle => RadialSize::Radii(*r, *r),
                    ([rx, ry], None) if !circle => RadialSize::Radii(*rx, *ry),
                    _ => return Err(err("a circle takes one radius, an ellipse two")),
                };
                Ok(Gradient::Radial { circle, size, at, stops })
            }
            "conic-gradient" => {
                let (head, at) = split_at_keyword(&words);
                let from = match head {
                    [] => 0.0,
                    ["from", a] => angle(a)?,
                    _ => return Err(err(&format!("unknown conic start `{}`", head.join(" ")))),
                };
                Ok(Gradient::Conic { from, at: position(at)?, stops })
            }
            other => Err(err(&format!("unknown gradient `{other}`"))),
        }
    }

    /// Parses CSS `background-image` layers: gradients separated by commas, the first on top.
    ///
    /// ```
    /// use remedy_ui_core::style::Gradient;
    /// let layers = Gradient::parse_layers(
    ///     "linear-gradient(90deg, var(--panel) 0%, transparent 55%), linear-gradient(to top, #000, transparent)",
    /// )
    /// .unwrap();
    /// assert_eq!(layers.len(), 2);
    /// ```
    pub fn parse_layers(css: &str) -> Result<Vec<Gradient>, ParseGradientError> {
        split_top(css, ',').into_iter().map(Gradient::parse).collect()
    }
}

fn err(msg: &str) -> ParseGradientError {
    ParseGradientError(msg.to_owned())
}

/// Splits at `sep` outside parentheses, dropping empty pieces.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0);
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if depth == 0 && (c == sep || (sep == ' ' && c.is_whitespace())) => {
                out.push(&s[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

fn split_at_keyword<'a, 'b>(words: &'b [&'a str]) -> (&'b [&'a str], &'b [&'a str]) {
    match words.iter().position(|w| *w == "at").and_then(|i| words.split_at_checked(i)) {
        Some((head, rest)) => (head, rest.split_first().map_or(&[][..], |(_, r)| r)),
        None => (words, &[]),
    }
}

fn number(s: &str, unit: &str) -> Option<f32> {
    s.strip_suffix(unit)?.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn angle(s: &str) -> Result<f32, ParseGradientError> {
    number(s, "deg")
        .or_else(|| number(s, "turn").map(|t| t * 360.0))
        .or_else(|| number(s, "grad").map(|g| g * 0.9))
        .or_else(|| number(s, "rad").map(f32::to_degrees))
        .or_else(|| (s == "0").then_some(0.0))
        .ok_or_else(|| err(&format!("bad angle `{s}`")))
}

fn length(s: &str) -> Result<Pos, ParseGradientError> {
    number(s, "%")
        .map(|p| Pos::Fraction(p / 100.0))
        .or_else(|| number(s, "px").map(Pos::Px))
        .or_else(|| number(s, "rem").or_else(|| number(s, "em")).map(|r| Pos::Px(r * 16.0)))
        .or_else(|| (s == "0").then_some(Pos::Px(0.0)))
        .ok_or_else(|| err(&format!("bad length `{s}`")))
}

/// CSS `<position>`: one or two keywords or lengths; nothing is the center.
fn position(words: &[&str]) -> Result<[Pos; 2], ParseGradientError> {
    let keyword = |w: &str| match w {
        "left" | "top" => Some(Pos::Fraction(0.0)),
        "center" => Some(Pos::CENTER),
        "right" | "bottom" => Some(Pos::Fraction(1.0)),
        _ => None,
    };
    let one = |w: &str| keyword(w).map_or_else(|| length(w), Ok);
    match words {
        [] => Ok([Pos::CENTER, Pos::CENTER]),
        [w] if matches!(*w, "top" | "bottom") => Ok([Pos::CENTER, one(w)?]),
        [w] => Ok([one(w)?, Pos::CENTER]),
        // `top left` names y first.
        [a, b] if matches!(*a, "top" | "bottom") || matches!(*b, "left" | "right") => Ok([one(b)?, one(a)?]),
        [a, b] => Ok([one(a)?, one(b)?]),
        _ => Err(err(&format!("unsupported position `{}`", words.join(" ")))),
    }
}

/// One stop argument: a color and up to two positions (two make two stops).
fn parse_stops(arg: &str) -> Result<Vec<GradientStop>, ParseGradientError> {
    let words = split_top(arg, ' ');
    let (first, rest) = words.split_first().ok_or_else(|| err("empty stop"))?;
    let (color, alpha) = parse_color(first)?;
    let stop = GradientStop { color, alpha, at: None };
    let fraction =
        |w: &str| number(w, "%").map(|p| p / 100.0).ok_or_else(|| err(&format!("bad stop position `{w}`")));
    match rest {
        [] => Ok(vec![stop]),
        [a] => Ok(vec![stop.at(fraction(a)?)]),
        [a, b] => Ok(vec![stop.at(fraction(a)?), stop.at(fraction(b)?)]),
        _ => Err(err(&format!("bad stop `{arg}`"))),
    }
}

/// A CSS color: the color and an alpha to scale it by.
fn parse_color(s: &str) -> Result<(ColorRef, f32), ParseGradientError> {
    let bad = || err(&format!("bad color `{s}`"));
    match s {
        "transparent" => return Ok((ColorRef::Value(Color::TRANSPARENT), 1.0)),
        "black" => return Ok((ColorRef::Value(Color::BLACK), 1.0)),
        "white" => return Ok((ColorRef::Value(Color::WHITE), 1.0)),
        _ => {}
    }
    if s.starts_with('#') {
        return Color::from_hex(s).map(|c| (ColorRef::Value(c), 1.0)).map_err(|_| bad());
    }
    let (name, inner) = s.split_once('(').ok_or_else(bad)?;
    let inner = inner.strip_suffix(')').ok_or_else(bad)?.trim();
    match name {
        "var" => {
            let var = inner.strip_prefix("--").ok_or_else(bad)?;
            Ok((ColorRef::Role(role_named(var)), 1.0))
        }
        "rgb" | "rgba" => {
            let parts: Vec<&str> = inner
                .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
                .filter(|p| !p.is_empty())
                .collect();
            let channel = |p: &str| p.parse::<f32>().ok().filter(|v| v.is_finite()).ok_or_else(bad);
            let alpha = |p: &str| number(p, "%").map(|v| v / 100.0).map_or_else(|| channel(p), Ok);
            match parts.as_slice() {
                [r, g, b] => Ok((
                    ColorRef::Value(Color::new(
                        channel(r)? / 255.0,
                        channel(g)? / 255.0,
                        channel(b)? / 255.0,
                        1.0,
                    )),
                    1.0,
                )),
                [r, g, b, a] => Ok((
                    ColorRef::Value(Color::new(
                        channel(r)? / 255.0,
                        channel(g)? / 255.0,
                        channel(b)? / 255.0,
                        alpha(a)?.clamp(0.0, 1.0),
                    )),
                    1.0,
                )),
                _ => Err(bad()),
            }
        }
        "color-mix" => {
            // shortcut: only mixes with `transparent` (a color at an alpha), the form themes use; mixing two colors needs a ColorRef of two roles.
            let parts = split_top(inner, ',');
            let ["in srgb", a, b] = parts.as_slice() else { return Err(bad()) };
            let side = |p: &str| -> Result<(ColorRef, f32, Option<f32>), ParseGradientError> {
                let w = split_top(p, ' ');
                let (c, a) = parse_color(w.first().ok_or_else(bad)?)?;
                let pct = match w.get(1) {
                    Some(v) => Some(number(v, "%").ok_or_else(bad)? / 100.0),
                    None => None,
                };
                Ok((c, a, pct))
            };
            let (a, b) = (side(a)?, side(b)?);
            let clear = |c: &ColorRef| *c == ColorRef::Value(Color::TRANSPARENT);
            let (color, other) = match (clear(&a.0), clear(&b.0)) {
                (false, true) => (a, b),
                (true, false) => (b, a),
                _ => return Err(err(&format!("only color-mix with transparent is supported: `{s}`"))),
            };
            let share = color.2.or(other.2.map(|p| 1.0 - p)).unwrap_or(0.5);
            Ok((color.0, color.1 * share.clamp(0.0, 1.0)))
        }
        _ => Err(bad()),
    }
}

/// The theme role a CSS variable names; names that are no role are the theme's custom colors.
fn role_named(name: &str) -> ColorRole {
    match name {
        "bg" => ColorRole::Bg,
        "panel" => ColorRole::Panel,
        "card" => ColorRole::Card,
        "text" => ColorRole::Text,
        "muted" => ColorRole::Muted,
        "accent" => ColorRole::Accent,
        "on-accent" => ColorRole::OnAccent,
        "danger" => ColorRole::Danger,
        "warn" => ColorRole::Warn,
        "ok" => ColorRole::Ok,
        "line" => ColorRole::Line,
        "focus" => ColorRole::Focus,
        "selected" => ColorRole::Selected,
        other => ColorRole::Custom(intern(other)),
    }
}

/// Custom roles hold `&'static str`; each distinct name is kept once for the process.
fn intern(name: &str) -> &'static str {
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut guard = NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let set = guard.get_or_insert_with(HashSet::new);
    if let Some(n) = set.get(name) {
        return n;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    set.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        let mut t = Theme::dark_forest();
        t.colors.custom.insert("gold".into(), Color::rgb8(226, 177, 90));
        t
    }

    #[test]
    fn portals_page_glow_parses_and_resolves() {
        let g = Gradient::parse(
            "radial-gradient(ellipse 72rem 28rem at -6% -12%, color-mix(in srgb, var(--gold) 16%, transparent), transparent 62%)",
        )
        .unwrap_or_else(|e| unreachable!("{e:?}"));
        let Gradient::Radial { circle, size, at, stops } = &g else { unreachable!("{g:?}") };
        assert!(!circle);
        assert_eq!(*size, RadialSize::Radii(Pos::Px(1152.0), Pos::Px(448.0)));
        assert_eq!(*at, [Pos::Fraction(-0.06), Pos::Fraction(-0.12)]);
        assert_eq!(stops.first().map(|s| s.color), Some(ColorRef::Role(ColorRole::Custom("gold"))));
        assert!(stops.first().is_some_and(|s| (s.alpha - 0.16).abs() < 1e-6));
        let t = theme();
        let s = g.stops(&t);
        let [(a0, c0), (a1, c1)] = s[..] else { unreachable!("{s:?}") };
        assert_eq!(a0, 0.0);
        assert!((c0.a - 0.16).abs() < 1e-6 && (c0.r - 226.0 / 255.0).abs() < 1e-6);
        assert!((a1 - 0.62).abs() < 1e-6 && c1.a == 0.0);
        // Resolving swaps the role for its value.
        let r = g.resolve(&t);
        assert_eq!(r.stop_list().first().map(|s| s.color), Some(ColorRef::Value(Color::rgb8(226, 177, 90))));
    }

    #[test]
    fn every_portal_gradient_parses() {
        for css in [
            "radial-gradient(ellipse 46rem 24rem at 108% -14%, color-mix(in srgb, var(--gold) 9%, transparent), transparent 58%)",
            "radial-gradient(circle at 50% 42%, color-mix(in srgb, var(--gold) 16%, transparent), transparent 62%)",
            "linear-gradient(to right, var(--text) 30%, var(--track) 30%)",
            "linear-gradient(rgba(0, 0, 0, 0.55), rgba(0, 0, 0, 0.78))",
            "linear-gradient(to top, rgba(0, 0, 0, 0.85), rgba(0, 0, 0, 0.4) 60%, transparent)",
            "linear-gradient(to bottom, rgba(0, 0, 0, 0.7), transparent)",
            "radial-gradient(ellipse at center, color-mix(in srgb, var(--gold) 12%, transparent), transparent 62%)",
            "linear-gradient(90deg, var(--raised) 0%, color-mix(in srgb, var(--raised) 82%, transparent) 42%, color-mix(in srgb, var(--raised) 35%, transparent) 100%)",
            "linear-gradient(0deg, var(--raised) 0%, transparent 55%)",
            "linear-gradient(to bottom, color-mix(in srgb, var(--raised) 10%, transparent) 20%, var(--raised) 98%)",
            "conic-gradient(from 90deg at 25% 75%, #f00, #00f)",
        ] {
            assert!(Gradient::parse(css).is_ok(), "{css}: {:?}", Gradient::parse(css));
        }
        for bad in [
            "linear-gradient(#000)",
            "linear-gradient(to top right, #000, #fff)",
            "radial-gradient(circle 10px 20px, #000, #fff)",
            "linear-gradient(#000, nonsense)",
            "spiral-gradient(#000, #fff)",
            "linear-gradient(#000, #fff",
        ] {
            assert!(Gradient::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn stops_are_placed_as_css_places_them() {
        let t = theme();
        let at = |g: Gradient| g.stops(&t).into_iter().map(|(a, _)| (a * 100.0).round()).collect::<Vec<_>>();
        let g = Gradient::parse("linear-gradient(#000, #111, #222, #333 90%, #444 20%)")
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        // Spread evenly up to 90%; a stop behind the one before it moves up to it.
        assert_eq!(at(g), vec![0.0, 30.0, 60.0, 90.0, 90.0]);
        let hard = Gradient::parse("linear-gradient(to right, #000 25% 50%, #fff 50%)")
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(at(hard), vec![25.0, 50.0, 50.0]);
    }

    #[test]
    fn geometry_follows_css() {
        let size = Size::new(200.0, 100.0);
        let lin =
            |css: &str| match Gradient::parse(css).unwrap_or_else(|e| unreachable!("{e:?}")).geometry(size) {
                GradientGeometry::Linear { start, end } => {
                    (start.x.round(), start.y.round(), end.x.round(), end.y.round())
                }
                g => unreachable!("{g:?}"),
            };
        assert_eq!(lin("linear-gradient(to right, #000, #fff)"), (0.0, 50.0, 200.0, 50.0));
        assert_eq!(lin("linear-gradient(#000, #fff)"), (100.0, 0.0, 100.0, 100.0));
        assert_eq!(lin("linear-gradient(0deg, #000, #fff)"), (100.0, 100.0, 100.0, 0.0));
        // 45deg: half-length (200 + 100) / 2 · sin 45°.
        let (x0, y0, x1, y1) = lin("linear-gradient(45deg, #000, #fff)");
        assert_eq!((x0, y0, x1, y1), (25.0, 125.0, 175.0, -25.0));
        let rad =
            |css: &str| match Gradient::parse(css).unwrap_or_else(|e| unreachable!("{e:?}")).geometry(size) {
                GradientGeometry::Radial { center, radii } => {
                    let r = |v: f32| (v * 1000.0).round() / 1000.0;
                    (r(center.x), r(center.y), r(radii.x), r(radii.y))
                }
                g => unreachable!("{g:?}"),
            };
        assert_eq!(rad("radial-gradient(closest-side at 50px 50%, #000, #fff)"), (50.0, 50.0, 50.0, 50.0));
        assert_eq!(
            rad("radial-gradient(circle closest-side at 50px 50%, #000, #fff)"),
            (50.0, 50.0, 50.0, 50.0)
        );
        assert_eq!(rad("radial-gradient(farthest-side at 50px 30%, #000, #fff)"), (50.0, 30.0, 150.0, 70.0));
        let (_, _, rx, ry) = rad("radial-gradient(ellipse at center, #000, #fff)");
        assert!(
            (rx - 100.0 * std::f32::consts::SQRT_2).abs() < 1e-2
                && (ry - 50.0 * std::f32::consts::SQRT_2).abs() < 1e-2
        );
        let (_, _, r, r2) = rad("radial-gradient(circle at left top, #000, #fff)");
        assert!((r - 200.0f32.hypot(100.0)).abs() < 1e-2 && r == r2);
        assert_eq!(rad("radial-gradient(circle 40px, #000, #fff)"), (100.0, 50.0, 40.0, 40.0));
        match Gradient::parse("conic-gradient(from 90deg at 25% 75%, #f00, #00f)")
            .unwrap_or_else(|e| unreachable!("{e:?}"))
            .geometry(size)
        {
            GradientGeometry::Conic { center, start_angle } => {
                assert_eq!((center.x, center.y, start_angle), (50.0, 75.0, 0.0));
            }
            g => unreachable!("{g:?}"),
        }
    }

    #[test]
    fn layers_resolve_through_sheets_and_elements() {
        use crate::style::{States, StyleKey, StylePatch, StyleSheet};
        let t = theme();
        let glow = Gradient::ellipse(
            [Pos::CENTER, Pos::Fraction(0.0)],
            [GradientStop::new(ColorRole::Accent).alpha(0.2), GradientStop::new(Color::TRANSPARENT).at(0.6)],
        );
        let mut sheet = StyleSheet::new();
        sheet.rule("page", None, States::NONE, StylePatch::new().background_layers([glow.clone()]));
        let key = StyleKey { widget: "page", variant: "", states: States::NONE };
        let d = StylePatch::defaults(&t);
        assert_eq!(sheet.resolve(&key, &d, &StylePatch::new()).background_layers, vec![glow.clone()]);
        // An element's own layers replace the sheet's; an empty list takes them away.
        let scrim =
            Gradient::linear(180.0, [GradientStop::new(Color::BLACK), GradientStop::new(Color::TRANSPARENT)]);
        let own = StylePatch::new().background_layers([scrim.clone()]);
        assert_eq!(sheet.resolve(&key, &d, &own).background_layers, vec![scrim]);
        assert!(
            sheet.resolve(&key, &d, &StylePatch::new().background_layers([])).background_layers.is_empty()
        );
        // Nothing set: none, and children do not inherit them.
        let other = StyleKey { widget: "box", variant: "", states: States::NONE };
        assert!(sheet.resolve(&other, &d, &StylePatch::new()).background_layers.is_empty());
        // A token style with roles is not plain, and resolves them.
        let s = crate::style::Style::new().background_layers([glow]);
        assert!(!s.is_plain());
        let p = s.resolve(&t);
        let layers = p.background_layers.unwrap_or_default();
        let first = layers.first().and_then(|g| g.stop_list().first()).map(|s| s.color);
        assert_eq!(first, Some(ColorRef::Value(t.colors.accent)));
    }

    #[test]
    fn colors_parse() {
        assert_eq!(
            parse_color("rgba(0, 0, 0, 0.5)").unwrap_or_else(|e| unreachable!("{e:?}")).0,
            ColorRef::Value(Color::new(0.0, 0.0, 0.0, 0.5))
        );
        assert_eq!(
            parse_color("rgb(255 0 0 / 50%)").unwrap_or_else(|e| unreachable!("{e:?}")).0,
            ColorRef::Value(Color::new(1.0, 0.0, 0.0, 0.5))
        );
        assert_eq!(
            parse_color("var(--accent)").unwrap_or_else(|e| unreachable!("{e:?}")),
            (ColorRef::Role(ColorRole::Accent), 1.0)
        );
        let (c, a) = parse_color("color-mix(in srgb, transparent 70%, var(--bg))")
            .unwrap_or_else(|e| unreachable!("{e:?}"));
        assert_eq!(c, ColorRef::Role(ColorRole::Bg));
        assert!((a - 0.3).abs() < 1e-6);
        assert!(parse_color("color-mix(in srgb, #fff 50%, #000)").is_err());
        // The same custom name is the same &'static str.
        assert!(std::ptr::eq(intern("glow"), intern("glow")));
    }
}
