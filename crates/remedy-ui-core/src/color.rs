//! Colors.
//!
//! A [`Color`] is sRGB with straight (not premultiplied) alpha, the way design tools and
//! themes write colors. Rendering converts to premultiplied form; animation mixes in Oklab so
//! a fade between two colors does not pass through a muddy grey.

/// An sRGB color with straight alpha, each channel 0.0 to 1.0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    /// Red, sRGB-encoded.
    pub r: f32,
    /// Green, sRGB-encoded.
    pub g: f32,
    /// Blue, sRGB-encoded.
    pub b: f32,
    /// Opacity, 0.0 transparent to 1.0 opaque.
    pub a: f32,
}

/// Why a color string could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseColorError {
    /// The string does not start with `#`.
    MissingHash,
    /// The string is not 3, 4, 6 or 8 hex digits after `#`.
    BadLength(usize),
    /// A character is not a hex digit.
    NotHex(char),
}

impl std::fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseColorError::MissingHash => write!(f, "a color must start with '#', like #1C1C1A"),
            ParseColorError::BadLength(n) => {
                write!(f, "a color needs 3, 4, 6 or 8 hex digits after '#', found {n}")
            }
            ParseColorError::NotHex(c) => write!(f, "'{c}' is not a hex digit"),
        }
    }
}

impl std::error::Error for ParseColorError {}

impl Color {
    /// Fully transparent.
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    /// Opaque black.
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    /// Opaque white.
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// A color from sRGB channels and alpha, each clamped to 0.0..=1.0 (NaN becomes 0.0).
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r: unit(r), g: unit(g), b: unit(b), a: unit(a) }
    }

    /// An opaque color from 8-bit sRGB channels.
    pub fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Color::rgba8(r, g, b, 255)
    }

    /// A color from 8-bit sRGB channels and alpha.
    pub fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: f32::from(a) / 255.0,
        }
    }

    /// Reads `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`.
    pub fn from_hex(s: &str) -> Result<Color, ParseColorError> {
        let digits = s.trim().strip_prefix('#').ok_or(ParseColorError::MissingHash)?;
        let mut v = [0u8; 8];
        let mut n = 0;
        for c in digits.chars() {
            let d = c.to_digit(16).ok_or(ParseColorError::NotHex(c))?;
            if let Some(slot) = v.get_mut(n) {
                *slot = d as u8;
            }
            n += 1;
        }
        let at = |i: usize| v.get(i).copied().unwrap_or(0);
        let short = |i: usize| at(i) * 17;
        let long = |i: usize| at(i) * 16 + at(i + 1);
        match n {
            3 => Ok(Color::rgb8(short(0), short(1), short(2))),
            4 => Ok(Color::rgba8(short(0), short(1), short(2), short(3))),
            6 => Ok(Color::rgb8(long(0), long(2), long(4))),
            8 => Ok(Color::rgba8(long(0), long(2), long(4), long(6))),
            n => Err(ParseColorError::BadLength(n)),
        }
    }

    /// `#RRGGBB`, or `#RRGGBBAA` when not opaque.
    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.to_rgba8();
        if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
    }

    /// The channels as 8-bit values, rounded.
    pub fn to_rgba8(self) -> [u8; 4] {
        [byte(self.r), byte(self.g), byte(self.b), byte(self.a)]
    }

    /// This color with alpha set to `a`.
    pub fn with_alpha(self, a: f32) -> Color {
        Color { a: unit(a), ..self }
    }

    /// The channels in linear light (straight alpha).
    pub fn to_linear(self) -> [f32; 4] {
        [srgb_to_linear(self.r), srgb_to_linear(self.g), srgb_to_linear(self.b), self.a]
    }

    /// A color from linear-light channels and alpha.
    pub fn from_linear(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color::new(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b), a)
    }

    /// sRGB channels premultiplied by alpha, as the renderer blends them.
    pub fn premultiplied(self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }

    /// Relative luminance (WCAG 2), 0.0 for black to 1.0 for white. Alpha is ignored.
    pub fn luminance(self) -> f32 {
        let [r, g, b, _] = self.to_linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// WCAG 2 contrast ratio between two opaque colors, 1.0 to 21.0.
    pub fn contrast(self, other: Color) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// This color drawn over an opaque `bg`, giving an opaque color.
    pub fn over(self, bg: Color) -> Color {
        let a = self.a;
        Color::new(
            self.r * a + bg.r * (1.0 - a),
            self.g * a + bg.g * (1.0 - a),
            self.b * a + bg.b * (1.0 - a),
            1.0,
        )
    }

    /// The color in Oklab: lightness, a (green to red), b (blue to yellow), and alpha.
    pub fn to_oklab(self) -> [f32; 4] {
        let [l, a, b] = oklab(self);
        [l, a, b, self.a]
    }

    /// A color from Oklab lightness, a, b and alpha.
    pub fn from_oklab(l: f32, a: f32, b: f32, alpha: f32) -> Color {
        let [r, g, bl] = oklab_to_linear(l, a, b);
        Color::from_linear(r, g, bl, alpha)
    }

    /// The color part way from `self` to `other` in Oklab, with alpha mixed linearly:
    /// `t = 0` gives `self`, `t = 1` gives `other`. Used for color transitions.
    pub fn mix_oklab(self, other: Color, t: f32) -> Color {
        let t = unit(t);
        let a = oklab(self);
        let b = oklab(other);
        let l = a[0] + (b[0] - a[0]) * t;
        let aa = a[1] + (b[1] - a[1]) * t;
        let bb = a[2] + (b[2] - a[2]) * t;
        let [r, g, bl] = oklab_to_linear(l, aa, bb);
        Color::from_linear(r, g, bl, self.a + (other.a - self.a) * t)
    }
}

impl std::str::FromStr for Color {
    type Err = ParseColorError;
    fn from_str(s: &str) -> Result<Color, ParseColorError> {
        Color::from_hex(s)
    }
}

fn unit(v: f32) -> f32 {
    if v >= 0.0 { v.min(1.0) } else { 0.0 }
}

fn byte(v: f32) -> u8 {
    (unit(v) * 255.0).round() as u8
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB → Oklab (Björn Ottosson's matrices).
fn oklab(c: Color) -> [f32; 3] {
    let [r, g, b, _] = c.to_linear();
    let l = 0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn oklab_to_linear(l: f32, a: f32, b: f32) -> [f32; 3] {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3,
        -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        for s in ["#1C1C1A", "#5A8C7E", "#FFFFFF", "#00000080"] {
            assert_eq!(Color::from_hex(s).map(Color::to_hex), Ok(s.to_string()));
        }
        assert_eq!(Color::from_hex("#fff"), Ok(Color::WHITE));
        assert_eq!(Color::from_hex("#0008").map(|c| c.to_rgba8()), Ok([0, 0, 0, 136]));
    }

    #[test]
    fn hex_errors_say_why() {
        assert_eq!(Color::from_hex("1C1C1A"), Err(ParseColorError::MissingHash));
        assert_eq!(Color::from_hex("#12345"), Err(ParseColorError::BadLength(5)));
        assert_eq!(Color::from_hex("#12G456"), Err(ParseColorError::NotHex('G')));
        assert_eq!(Color::from_hex("#123456789ABC"), Err(ParseColorError::BadLength(12)));
    }

    #[test]
    fn contrast_matches_wcag() {
        let c = Color::BLACK.contrast(Color::WHITE);
        assert!((c - 21.0).abs() < 0.01, "{c}");
        assert!((Color::WHITE.contrast(Color::WHITE) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn linear_round_trip() {
        let c = Color::rgb8(90, 140, 126);
        let [r, g, b, a] = c.to_linear();
        assert_eq!(Color::from_linear(r, g, b, a).to_rgba8(), c.to_rgba8());
    }

    #[test]
    fn oklab_mix_ends_and_midpoint() {
        let a = Color::rgb8(255, 0, 0);
        let b = Color::rgb8(0, 0, 255);
        assert_eq!(a.mix_oklab(b, 0.0).to_rgba8(), a.to_rgba8());
        assert_eq!(a.mix_oklab(b, 1.0).to_rgba8(), b.to_rgba8());
        let mid = a.mix_oklab(b, 0.5);
        // Oklab keeps the midpoint bright; a plain sRGB average would be (128, 0, 128).
        assert!(mid.luminance() > Color::rgb8(128, 0, 128).luminance());
    }

    #[test]
    fn clamps_out_of_range() {
        assert_eq!(Color::new(2.0, -1.0, f32::NAN, 0.5), Color { r: 1.0, g: 0.0, b: 0.0, a: 0.5 });
    }
}
