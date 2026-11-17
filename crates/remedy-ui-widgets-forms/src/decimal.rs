//! Exact decimal numbers for inputs where binary floating point would be wrong (money,
//! recipes, lye calculations): `0.1 + 0.2` is `0.3`.

use std::cmp::Ordering;
use std::fmt;

/// The most fraction digits a [`Decimal`] keeps.
pub const MAX_SCALE: u8 = 28;

/// An exact decimal: `mantissa / 10^scale`, kept normalized (no trailing fraction zeros),
/// so equal values are equal and hash alike. About 38 significant digits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Decimal {
    mantissa: i128,
    scale: u8,
}

/// Why text is not a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalError {
    /// Nothing to read.
    Empty,
    /// A character that is not part of a number here.
    Invalid,
    /// Too many digits for 38 significant digits or 28 fraction digits.
    Overflow,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DecimalError::Empty => "Enter a number",
            DecimalError::Invalid => "Not a number",
            DecimalError::Overflow => "Too many digits",
        })
    }
}

impl std::error::Error for DecimalError {}

const fn pow10(n: u8) -> i128 {
    let mut v = 1i128;
    let mut i = 0;
    while i < n {
        v *= 10;
        i += 1;
    }
    v
}

impl Decimal {
    /// Zero.
    pub const ZERO: Decimal = Decimal { mantissa: 0, scale: 0 };
    /// One.
    pub const ONE: Decimal = Decimal { mantissa: 1, scale: 0 };

    /// `mantissa / 10^scale`, or `None` when `scale` exceeds [`MAX_SCALE`].
    pub fn new(mantissa: i128, scale: u8) -> Option<Decimal> {
        (scale <= MAX_SCALE).then(|| Decimal { mantissa, scale }.normalized())
    }

    /// A whole number.
    pub fn from_int(v: i64) -> Decimal {
        Decimal { mantissa: i128::from(v), scale: 0 }
    }

    fn normalized(mut self) -> Decimal {
        while self.scale > 0 && self.mantissa % 10 == 0 {
            self.mantissa /= 10;
            self.scale -= 1;
        }
        if self.mantissa == 0 {
            self.scale = 0;
        }
        self
    }

    /// The digits without the decimal point, and how many of them are fraction digits.
    pub fn parts(self) -> (i128, u8) {
        (self.mantissa, self.scale)
    }

    /// Fraction digits needed to write it exactly.
    pub fn scale(self) -> u8 {
        self.scale
    }

    /// True below zero.
    pub fn is_negative(self) -> bool {
        self.mantissa < 0
    }

    /// The closest `f64` (for drawing, never for arithmetic that must be exact).
    pub fn to_f64(self) -> f64 {
        // Through text: exact to the nearest double, with no accumulated error.
        self.to_string().parse().unwrap_or(0.0)
    }

    /// The decimal closest to `v` with at most `places` fraction digits (`None` for NaN or
    /// infinities, or values out of range).
    pub fn from_f64(v: f64, places: u8) -> Option<Decimal> {
        if !v.is_finite() {
            return None;
        }
        let s = format!("{:.*}", usize::from(places.min(MAX_SCALE)), v);
        Decimal::parse(&s).ok()
    }

    /// Reads plain text: an optional `-` or `+`, digits, and an optional `.` with digits.
    pub fn parse(s: &str) -> Result<Decimal, DecimalError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(DecimalError::Empty);
        }
        let (neg, rest) = match s.as_bytes().first() {
            Some(b'-') => (true, s.get(1..).unwrap_or("")),
            Some(b'+') => (false, s.get(1..).unwrap_or("")),
            _ => (false, s),
        };
        let mut mantissa: i128 = 0;
        let mut scale: u8 = 0;
        let mut seen_point = false;
        let mut digits = 0;
        for c in rest.chars() {
            match c {
                '0'..='9' => {
                    let d = i128::from(c as u8 - b'0');
                    mantissa = mantissa
                        .checked_mul(10)
                        .and_then(|m| m.checked_add(d))
                        .ok_or(DecimalError::Overflow)?;
                    digits += 1;
                    if seen_point {
                        scale =
                            scale.checked_add(1).filter(|s| *s <= MAX_SCALE).ok_or(DecimalError::Overflow)?;
                    }
                }
                '.' if !seen_point => seen_point = true,
                _ => return Err(DecimalError::Invalid),
            }
        }
        if digits == 0 {
            return Err(DecimalError::Invalid);
        }
        Ok(Decimal { mantissa: if neg { -mantissa } else { mantissa }, scale }.normalized())
    }

    fn aligned(self, other: Decimal) -> Option<(i128, i128, u8)> {
        let scale = self.scale.max(other.scale);
        let a = self.mantissa.checked_mul(pow10(scale - self.scale))?;
        let b = other.mantissa.checked_mul(pow10(scale - other.scale))?;
        Some((a, b, scale))
    }

    /// `self + other`, or `None` on overflow.
    pub fn checked_add(self, other: Decimal) -> Option<Decimal> {
        let (a, b, scale) = self.aligned(other)?;
        Some(Decimal { mantissa: a.checked_add(b)?, scale }.normalized())
    }

    /// `self - other`, or `None` on overflow.
    pub fn checked_sub(self, other: Decimal) -> Option<Decimal> {
        let (a, b, scale) = self.aligned(other)?;
        Some(Decimal { mantissa: a.checked_sub(b)?, scale }.normalized())
    }

    /// `self × other`, or `None` on overflow (fraction digits beyond 28 are rounded).
    pub fn checked_mul(self, other: Decimal) -> Option<Decimal> {
        let m = self.mantissa.checked_mul(other.mantissa)?;
        let scale = self.scale + other.scale;
        if scale <= MAX_SCALE {
            return Some(Decimal { mantissa: m, scale }.normalized());
        }
        Decimal { mantissa: m, scale: MAX_SCALE }.checked_div_pow10(scale - MAX_SCALE)
    }

    fn checked_div_pow10(self, n: u8) -> Option<Decimal> {
        let d = pow10(n);
        let q = self.mantissa / d;
        let r = self.mantissa % d;
        let q = if r.abs() * 2 >= d { q.checked_add(self.mantissa.signum())? } else { q };
        Some(Decimal { mantissa: q, scale: self.scale }.normalized())
    }

    /// Rounded to `places` fraction digits, halves away from zero (2.5 → 3, −2.5 → −3).
    pub fn round(self, places: u8) -> Decimal {
        if self.scale <= places {
            return self;
        }
        let drop = self.scale - places;
        let d = pow10(drop);
        let q = self.mantissa / d;
        let r = self.mantissa % d;
        let q = if r.abs() * 2 >= d { q.saturating_add(self.mantissa.signum()) } else { q };
        Decimal { mantissa: q, scale: places }.normalized()
    }

    /// The integer part and the fraction digits (absolute values), with `places` fraction
    /// digits after rounding: `(-12.345).digits(2)` is `(true, "12", "35")`.
    pub fn digits(self, places: u8) -> (bool, String, String) {
        let r = self.round(places);
        let neg = r.mantissa < 0;
        let abs = r.mantissa.unsigned_abs();
        let d = pow10(r.scale).unsigned_abs();
        let int = (abs / d).to_string();
        let mut frac = if r.scale == 0 {
            String::new()
        } else {
            format!("{:0width$}", abs % d, width = usize::from(r.scale))
        };
        while frac.len() < usize::from(places) {
            frac.push('0');
        }
        (neg, int, frac)
    }

    /// The smaller of two values.
    pub fn min(self, other: Decimal) -> Decimal {
        if self <= other { self } else { other }
    }

    /// The larger of two values.
    pub fn max(self, other: Decimal) -> Decimal {
        if self >= other { self } else { other }
    }

    /// Kept within `lo..=hi` (either bound optional).
    pub fn clamp_to(self, lo: Option<Decimal>, hi: Option<Decimal>) -> Decimal {
        let v = lo.map_or(self, |l| self.max(l));
        hi.map_or(v, |h| v.min(h))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        // Whole parts first, then fractions scaled to 28 digits: never overflows.
        let split = |d: &Decimal| {
            let p = pow10(d.scale);
            (d.mantissa / p, (d.mantissa % p) * pow10(MAX_SCALE - d.scale))
        };
        split(self).cmp(&split(other))
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (neg, int, frac) = self.digits(self.scale);
        if neg {
            f.write_str("-")?;
        }
        f.write_str(&int)?;
        if !frac.is_empty() {
            write!(f, ".{frac}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Decimal {
    type Err = DecimalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::parse(s)
    }
}
