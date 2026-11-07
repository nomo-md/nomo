//! Layout dimension: exact rational (`num / den`).
//!
//! TeX layout is ratios of integers (font units, mu = 1/18 em, style scales).
//! Hardware `f32` / `f64` never appear as calculation terminals.

use core::cmp::Ordering;
use core::fmt;
use core::ops::{Add, Div, Mul, Neg, Sub};

use crate::error::Error;

/// Kept in the public API. Layout values are exact rationals, not rounded bits.
pub const DIM_PREC: usize = 256;

fn gcd_u(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// The dimension units the reader accepts and the printer writes, with how
/// many of each make one em (`pt` is a tenth of an em here, `mu` is 1/18).
/// `bp` reads as `pt` and no unit as `em`, see [`Dim::in_em`].
const UNITS: [(&str, i128); 3] = [("em", 1), ("mu", 18), ("pt", 10)];

/// TeX-style dimension: width, height, depth, italic correction, mu.
///
/// One unit is one em at the current math style unless a method says otherwise.
///
/// # Examples
///
/// ```
/// use latex_rust::Dim;
///
/// let half = Dim::ratio(1, 2);
/// assert!(half.eq_dim(&(&Dim::one() / &Dim::from_i64(2))));
/// assert!(!Dim::zero().eq_dim(&Dim::one()));
/// ```
#[derive(Clone, Debug)]
pub struct Dim {
    num: i128,
    den: i128,
    nan: bool,
}

impl Dim {
    fn raw(num: i128, den: i128) -> Self {
        if den == 0 {
            return Self::nan();
        }
        if num == 0 {
            return Self {
                num: 0,
                den: 1,
                nan: false,
            };
        }
        let g = gcd_u(num.unsigned_abs(), den.unsigned_abs());
        let g = i128::try_from(g).unwrap_or(1);
        let mut n = num / g;
        let mut d = den / g;
        if d < 0 {
            n = -n;
            d = -d;
        }
        Self {
            num: n,
            den: d,
            nan: false,
        }
    }

    fn nan() -> Self {
        Self {
            num: 0,
            den: 1,
            nan: true,
        }
    }

    fn binop(
        a: &Self,
        b: &Self,
        f: impl Fn(i128, i128, i128, i128) -> Option<(i128, i128)>,
    ) -> Self {
        if a.nan || b.nan {
            return Self::nan();
        }
        match f(a.num, a.den, b.num, b.den) {
            Some((n, d)) => Self::raw(n, d),
            None => Self::nan(),
        }
    }

    /// Zero em.
    #[must_use]
    pub fn zero() -> Self {
        Self::raw(0, 1)
    }

    /// One em.
    #[must_use]
    pub fn one() -> Self {
        Self::raw(1, 1)
    }

    /// Integer em count.
    #[must_use]
    pub fn from_i64(v: i64) -> Self {
        Self::raw(i128::from(v), 1)
    }

    /// Exact rational `num / den` em. `den == 0` yields NaN.
    #[must_use]
    pub fn ratio(num: i64, den: i64) -> Self {
        Self::raw(i128::from(num), i128::from(den))
    }

    /// Parse a decimal string (optional sign, fraction, `e` exponent).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        let t = s.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("nan") {
            return Self::nan();
        }
        let mut rest = t;
        let neg = if let Some(r) = rest.strip_prefix('-') {
            rest = r;
            true
        } else if let Some(r) = rest.strip_prefix('+') {
            rest = r;
            false
        } else {
            false
        };
        let (mant, exp_s) = match rest.find(['e', 'E']) {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };
        let (ip, fp) = match mant.find('.') {
            Some(i) => (&mant[..i], &mant[i + 1..]),
            None => (mant, ""),
        };
        if ip.is_empty() && fp.is_empty() {
            return Self::nan();
        }
        if !ip.chars().all(|c| c.is_ascii_digit()) || !fp.chars().all(|c| c.is_ascii_digit()) {
            return Self::nan();
        }
        let mut num: i128 = 0;
        for c in ip.bytes().chain(fp.bytes()) {
            let d = i128::from(c - b'0');
            num = match num.checked_mul(10).and_then(|n| n.checked_add(d)) {
                Some(n) => n,
                None => return Self::nan(),
            };
        }
        let mut den = 1i128;
        for _ in 0..fp.len() {
            den = match den.checked_mul(10) {
                Some(d) => d,
                None => return Self::nan(),
            };
        }
        if let Some(es) = exp_s {
            let e: i32 = match es.parse() {
                Ok(v) => v,
                Err(_) => return Self::nan(),
            };
            if e > 0 {
                for _ in 0..e {
                    num = match num.checked_mul(10) {
                        Some(n) => n,
                        None => return Self::nan(),
                    };
                }
            } else {
                for _ in 0..(-e) {
                    den = match den.checked_mul(10) {
                        Some(d) => d,
                        None => return Self::nan(),
                    };
                }
            }
        }
        if neg {
            num = -num;
        }
        Self::raw(num, den)
    }

    /// Convert integer font units to em: `units / units_per_em`.
    #[must_use]
    pub fn from_font_units(units: i64, units_per_em: u16) -> Self {
        Self::raw(i128::from(units), i128::from(units_per_em))
    }

    /// One math unit (mu). TeX: `18 mu = 1 em`.
    #[must_use]
    pub fn mu() -> Self {
        Self::ratio(1, 18)
    }

    /// Convert this em value to mu (`* 18`).
    #[must_use]
    pub fn to_mu(&self) -> Self {
        self.clone() * Self::from_i64(18)
    }

    /// Convert a mu value to em (`/ 18`).
    #[must_use]
    pub fn from_mu(mu: &Self) -> Self {
        mu.clone() / Self::from_i64(18)
    }

    /// Absolute value.
    #[must_use]
    pub fn abs(&self) -> Self {
        if self.nan {
            return Self::nan();
        }
        if self.num == i128::MIN {
            return Self::nan();
        }
        Self::raw(self.num.abs(), self.den)
    }

    /// Maximum of two dimensions.
    #[must_use]
    pub fn max(&self, other: &Self) -> Self {
        match self.cmp(other) {
            Some(Ordering::Less) => other.clone(),
            _ => self.clone(),
        }
    }

    /// Minimum of two dimensions.
    #[must_use]
    pub fn min(&self, other: &Self) -> Self {
        match self.cmp(other) {
            Some(Ordering::Greater) => other.clone(),
            _ => self.clone(),
        }
    }

    /// `max(self, 0)`.
    #[must_use]
    pub fn clamp_nonneg(&self) -> Self {
        self.max(&Self::zero())
    }

    /// True when the value is NaN.
    #[must_use]
    pub fn is_nan(&self) -> bool {
        self.nan
    }

    /// True when the value compares equal to zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        !self.nan && self.num == 0
    }

    /// The exact rational value in em, as `(numerator, denominator)`, or
    /// `None` if this dimension is NaN.
    ///
    /// The denominator is positive and the fraction is in lowest terms. This is
    /// the only accessor that does not round: [`Self::to_dec_string`] formats a
    /// decimal expansion and [`Self::to_ieee32_bits`] rounds to binary32. An
    /// external renderer that wants to convert at its own pixel boundary,
    /// rather than take the crate's rounding, should start here.
    ///
    /// # Examples
    ///
    /// ```
    /// use latex_rust::Dim;
    ///
    /// assert_eq!(Dim::ratio(2, 6).as_ratio(), Some((1, 3)));
    /// assert_eq!(Dim::ratio(1, -2).as_ratio(), Some((-1, 2)));
    /// assert_eq!(Dim::zero().as_ratio(), Some((0, 1)));
    /// assert_eq!(Dim::ratio(1, 0).as_ratio(), None);
    /// ```
    #[must_use]
    pub fn as_ratio(&self) -> Option<(i128, i128)> {
        (!self.nan).then_some((self.num, self.den))
    }

    /// Decimal string (exact terminating expansion, or scientific).
    #[must_use]
    pub fn to_dec_string(&self) -> String {
        if self.nan {
            return "NaN".into();
        }
        format_rational(self.num, self.den)
    }

    /// The number `self`, read in `unit`, as em: the one definition of the
    /// dimension units, used by the reader and by the printer's check of what it
    /// writes. `""` is `em`, `bp` is `pt`; `None` for any other unit.
    pub(crate) fn in_em(self, unit: &str) -> Option<Self> {
        let unit = match unit {
            "" => "em",
            "bp" => "pt",
            other => other,
        };
        let (_, per_em) = UNITS.iter().find(|(name, _)| *name == unit)?;
        Some(self / Self::from_i64(i64::try_from(*per_em).ok()?))
    }

    /// The number and unit the printer writes in `\hspace{..}` and `\rule{..}{..}`
    /// (`0.25em`, `1.5mu`), so that the text reads back as this exact `Dim`.
    ///
    /// Every value a parse can make is exact in the unit it was read in: a
    /// finite decimal in em, in mu (1/18 em) when em would never end, or in pt
    /// when em has more digits than the reader holds. Each candidate is checked
    /// against the reader before it is used (`X.0` of a 38-digit whole number
    /// overflows the reader, `X` does not). A value that no text reaches (1/3,
    /// say) falls back to the 24-digit text of [`Self::to_dec_string`], which
    /// is lossy.
    #[must_use]
    pub(crate) fn to_tex_string(&self) -> String {
        if self.nan {
            return "NaNem".to_string();
        }
        for (unit, per_em) in UNITS {
            let Some((text, done)) = scaled_decimal(self.num, self.den, per_em) else {
                continue;
            };
            if !done {
                continue;
            }
            let bare = text.strip_suffix(".0").unwrap_or(&text);
            for text in [text.as_str(), bare] {
                if self.reads_back(text, unit) {
                    return format!("{text}{unit}");
                }
            }
        }
        // No parse makes this value; the text is the lossy decimal.
        format!("{}em", self.to_dec_string())
    }

    /// True when the reader turns `text`, in `unit`, into exactly `self`.
    fn reads_back(&self, text: &str, unit: &str) -> bool {
        Self::parse(text)
            .in_em(unit)
            .is_some_and(|read| read.eq_dim(self))
    }

    /// Compact decimal for SVG attributes.
    #[must_use]
    pub fn to_svg_string(&self) -> String {
        self.to_dec_string()
    }

    /// Layout dimension from an IEEE-754 binary32 bit pattern (outline boundary).
    #[must_use]
    pub fn from_ieee32_bits(bits: u32) -> Self {
        let sign = (bits >> 31) != 0;
        let exp = (bits >> 23) & 0xff;
        let frac = bits & 0x7f_ffff;
        if exp == 255 {
            return Self::nan();
        }
        let (num, den, sh) = if exp == 0 {
            if frac == 0 {
                return Self::zero();
            }
            (i128::from(frac), 1i128, 149i32)
        } else {
            (i128::from(frac) + (1i128 << 23), 1i128, 150i32 - exp as i32)
        };
        let mut n = num;
        let mut d = den;
        if sh >= 0 {
            for _ in 0..sh {
                d = match d.checked_mul(2) {
                    Some(v) => v,
                    None => return Self::nan(),
                };
            }
        } else {
            for _ in 0..(-sh) {
                n = match n.checked_mul(2) {
                    Some(v) => v,
                    None => return Self::nan(),
                };
            }
        }
        if sign {
            n = -n;
        }
        Self::raw(n, d)
    }

    /// Round to IEEE-754 binary32 bits for PNG / raster emission only.
    #[must_use]
    pub fn to_ieee32_bits(&self) -> u32 {
        if self.nan {
            return 0x7fc0_0000;
        }
        if self.num == 0 {
            return 0;
        }
        let sign = if self.num < 0 { 1u32 << 31 } else { 0 };
        let n = self.num.unsigned_abs();
        let d = self.den.unsigned_abs();
        // Find e such that 2^e <= n/d < 2^{e+1}.
        let mut e: i32 = 0;
        // Compare n/d vs 1: n ? d
        if n < d {
            let mut t = n;
            while t < d {
                t = match t.checked_mul(2) {
                    Some(v) => v,
                    None => break,
                };
                e -= 1;
                if e < -149 {
                    return sign;
                }
            }
        } else {
            let mut t = d;
            while t <= n / 2 {
                t = match t.checked_mul(2) {
                    Some(v) => v,
                    None => break,
                };
                e += 1;
                if e > 127 {
                    return sign | 0x7f80_0000;
                }
            }
        }
        // mantissa: floor((n/d) / 2^(e-23)) = floor(n * 2^(23-e) / d)
        let mut shift = 23i32 - e;
        let mut num = n;
        let den = d;
        while shift > 0 {
            num = match num.checked_mul(2) {
                Some(v) => v,
                None => return sign | 0x7f80_0000,
            };
            shift -= 1;
        }
        while shift < 0 {
            num /= 2;
            shift += 1;
        }
        let mut mant = num / den;
        let rem = num % den;
        // round to nearest even
        if rem * 2 > den || (rem * 2 == den && mant & 1 == 1) {
            mant += 1;
        }
        if mant >= (1u128 << 24) {
            mant >>= 1;
            e += 1;
        }
        if e > 127 {
            return sign | 0x7f80_0000;
        }
        if e < -126 {
            // subnormal
            let sub_shift = -126 - e;
            if sub_shift >= 24 {
                return sign;
            }
            mant >>= sub_shift as u32;
            let frac = (mant as u32) & 0x7f_ffff;
            return sign | frac;
        }
        let biased = (e + 127) as u32;
        let frac = (mant as u32) & 0x7f_ffff;
        sign | (biased << 23) | frac
    }

    /// Largest `u32` that is not greater than `self`. Negative and NaN fail.
    pub fn floor_to_u32(&self) -> Result<u32, Error> {
        if self.is_nan() {
            return Err(Error::InvalidOption {
                what: "dimension is NaN".into(),
            });
        }
        if matches!(self.cmp(&Self::zero()), Some(Ordering::Less)) {
            return Err(Error::InvalidOption {
                what: "negative dimension".into(),
            });
        }
        const MAX: u32 = 1 << 20;
        if matches!(
            self.cmp(&Self::from_i64(i64::from(MAX))),
            Some(Ordering::Greater) | Some(Ordering::Equal)
        ) {
            return Err(Error::InvalidOption {
                what: "dimension exceeds raster limit".into(),
            });
        }
        let q = (self.num / self.den) as u32;
        Ok(q)
    }

    /// Smallest `u32` that is not less than `self`.
    pub fn ceil_to_u32(&self) -> Result<u32, Error> {
        let floor = self.floor_to_u32()?;
        if self.eq_dim(&Self::from_i64(i64::from(floor))) {
            Ok(floor)
        } else {
            floor.checked_add(1).ok_or_else(|| Error::InvalidOption {
                what: "dimension overflow".into(),
            })
        }
    }

    /// Compare two dimensions. `None` if either is NaN.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn cmp(&self, other: &Self) -> Option<Ordering> {
        if self.nan || other.nan {
            return None;
        }
        // Exact for every value: no cross products, so nothing overflows.
        let signs = self.num.signum().cmp(&other.num.signum());
        if signs != Ordering::Equal || self.num == 0 {
            return Some(signs);
        }
        let (n1, d1) = (self.num.unsigned_abs(), self.den.unsigned_abs());
        let (n2, d2) = (other.num.unsigned_abs(), other.den.unsigned_abs());
        Some(if self.num < 0 {
            cmp_ratio(n2, d2, n1, d1)
        } else {
            cmp_ratio(n1, d1, n2, d2)
        })
    }

    /// True when `self` and `other` compare equal.
    #[must_use]
    pub fn eq_dim(&self, other: &Self) -> bool {
        matches!(self.cmp(other), Some(Ordering::Equal))
    }
}

/// Compares `n1/d1` with `n2/d2` (denominators positive) without multiplying:
/// the integer parts first, then the fractional parts by their reciprocals
/// (the Euclidean algorithm, so it ends in at most about 190 steps).
fn cmp_ratio(n1: u128, d1: u128, n2: u128, d2: u128) -> Ordering {
    let (q1, q2) = (n1 / d1, n2 / d2);
    if q1 != q2 {
        return q1.cmp(&q2);
    }
    match (n1 % d1, n2 % d2) {
        (0, 0) => Ordering::Equal,
        (0, _) => Ordering::Less,
        (_, 0) => Ordering::Greater,
        (r1, r2) => cmp_ratio(d2, r2, d1, r1),
    }
}

fn expand_decimal(num: i128, den: i128, cap: usize) -> (String, bool) {
    if num == 0 {
        return ("0.0".into(), true);
    }
    let neg = num < 0;
    let mut n = num.unsigned_abs();
    let d = den.unsigned_abs();
    let ip = n / d;
    n %= d;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if n == 0 {
        s.push_str(&ip.to_string());
        s.push_str(".0");
        return (s, true);
    }
    s.push_str(&ip.to_string());
    s.push('.');
    // Long division, cap at `cap` digits.
    for _ in 0..cap {
        if n == 0 {
            break;
        }
        let mut digit = 0u8;
        let mut next = 0u128;
        // Preserve long division without forming n * 10, which can overflow.
        for _ in 0..10 {
            next += n;
            if next >= d {
                next -= d;
                digit += 1;
            }
        }
        s.push(char::from(b'0' + digit));
        n = next;
    }
    // Trim trailing zeros but keep one fractional digit.
    while s.ends_with('0') && !s.ends_with(".0") {
        s.pop();
    }
    (s, n == 0)
}

fn format_rational(num: i128, den: i128) -> String {
    expand_decimal(num, den, 24).0
}

/// The decimal expansion of `per_em * num / den` (the value in a unit that is
/// `per_em` to the em), and whether it is complete; `None` on overflow. The
/// fraction is reduced by `gcd(per_em, den)` first, so only a numerator that
/// really needs more than 127 bits overflows.
fn scaled_decimal(num: i128, den: i128, per_em: i128) -> Option<(String, bool)> {
    let g = gcd_u(per_em.unsigned_abs(), den.unsigned_abs());
    let g = i128::try_from(g).ok()?;
    let n = num.checked_mul(per_em / g)?;
    let d = den / g;
    Some(expand_decimal(n, d, 130))
}

impl PartialEq for Dim {
    fn eq(&self, other: &Self) -> bool {
        self.eq_dim(other)
    }
}

impl Eq for Dim {}

impl PartialOrd for Dim {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.cmp(other)
    }
}

impl fmt::Display for Dim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_dec_string())
    }
}

impl Neg for Dim {
    type Output = Self;

    fn neg(self) -> Self {
        if self.nan {
            return Self::nan();
        }
        Self::raw(-self.num, self.den)
    }
}

fn add_r(a: i128, b: i128, c: i128, d: i128) -> Option<(i128, i128)> {
    let ad = a.checked_mul(d)?;
    let cb = c.checked_mul(b)?;
    let n = ad.checked_add(cb)?;
    let den = b.checked_mul(d)?;
    Some((n, den))
}

fn sub_r(a: i128, b: i128, c: i128, d: i128) -> Option<(i128, i128)> {
    let ad = a.checked_mul(d)?;
    let cb = c.checked_mul(b)?;
    let n = ad.checked_sub(cb)?;
    let den = b.checked_mul(d)?;
    Some((n, den))
}

fn mul_r(a: i128, b: i128, c: i128, d: i128) -> Option<(i128, i128)> {
    let n = a.checked_mul(c)?;
    let den = b.checked_mul(d)?;
    Some((n, den))
}

fn div_r(a: i128, b: i128, c: i128, d: i128) -> Option<(i128, i128)> {
    if c == 0 {
        return None;
    }
    let n = a.checked_mul(d)?;
    let den = b.checked_mul(c)?;
    Some((n, den))
}

macro_rules! impl_dim_op {
    ($Trait:ident, $method:ident, $helper:ident) => {
        impl $Trait for Dim {
            type Output = Dim;
            fn $method(self, rhs: Dim) -> Dim {
                Dim::binop(&self, &rhs, $helper)
            }
        }
        impl $Trait<&Dim> for Dim {
            type Output = Dim;
            fn $method(self, rhs: &Dim) -> Dim {
                Dim::binop(&self, rhs, $helper)
            }
        }
        impl $Trait<Dim> for &Dim {
            type Output = Dim;
            fn $method(self, rhs: Dim) -> Dim {
                Dim::binop(self, &rhs, $helper)
            }
        }
        impl $Trait for &Dim {
            type Output = Dim;
            fn $method(self, rhs: &Dim) -> Dim {
                Dim::binop(self, rhs, $helper)
            }
        }
    };
}

impl_dim_op!(Add, add, add_r);
impl_dim_op!(Sub, sub, sub_r);
impl_dim_op!(Mul, mul, mul_r);
impl_dim_op!(Div, div, div_r);

#[cfg(test)]
mod tests {
    use super::Dim;

    #[test]
    fn half_and_font_units() {
        assert!(Dim::ratio(1, 2).eq_dim(&(&Dim::one() / &Dim::from_i64(2))));
        let w = Dim::from_font_units(479, 1000);
        assert_eq!(w.to_dec_string(), "0.479");
        assert_eq!((Dim::from_i64(12) * w).to_dec_string(), "5.748");
    }

    #[test]
    fn ieee32_one() {
        let one = Dim::from_ieee32_bits(0x3f80_0000);
        assert!(one.eq_dim(&Dim::one()));
        assert_eq!(Dim::one().to_ieee32_bits(), 0x3f80_0000);
    }

    #[test]
    fn decimal_format_avoids_large_remainder_overflow() {
        let near_one = Dim::ratio(i64::MAX - 1, i64::MAX);
        let squared = &near_one * &near_one;

        assert_eq!(squared.to_dec_string(), "0.999999999999999999783159");
    }
}
