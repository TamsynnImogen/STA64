//! 32.32 signed fixed-point number for deterministic simulation maths.

use std::ops::{Add, Div, Mul, Neg, Sub};

/// A 32.32 fixed-point number: every quantity in simulation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(i64);

const FRAC: u32 = 32;

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);
    pub const ONE: Fixed = Fixed(1 << FRAC);

    pub const fn from_int(v: i32) -> Self {
        Fixed((v as i64) << FRAC)
    }
    pub const fn from_bits(bits: i64) -> Self {
        Fixed(bits)
    }
    pub const fn to_bits(self) -> i64 {
        self.0
    }
    /// Lossy conversion for loading data and for rendering only — never feed
    /// the result back into simulation state.
    pub fn from_f64(v: f64) -> Self {
        Fixed((v * (1u64 << FRAC) as f64).round() as i64)
    }
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / (1u64 << FRAC) as f32
    }

    pub fn abs(self) -> Self {
        Fixed(self.0.wrapping_abs())
    }

    /// Integer square root on the raw representation; exact and portable: the largest whole
    /// number whose square is not above the raw value shifted up by the fraction bits. The
    /// standard library's integer square root gives exactly that number with 64-bit steps,
    /// where the earlier form ([`reference::sqrt`], kept for the tests) reached it by a
    /// Newton iteration of 128-bit divisions (#1017). The result is the same bit for bit.
    pub fn sqrt(self) -> Self {
        if self.0 <= 0 {
            return Fixed::ZERO;
        }
        Fixed((((self.0 as u128) << FRAC).isqrt()) as i64)
    }
}

/// The forms of the division and the square root before #1017, as written then. They define
/// the results: the faster forms above must agree with them on every input, which the tests
/// below check over edge values and a fixed set of random ones.
#[cfg(test)]
mod reference {
    use super::{FRAC, Fixed};

    pub(super) fn div(a: Fixed, o: Fixed) -> Fixed {
        if o.0 == 0 {
            return Fixed::ZERO;
        }
        Fixed((((a.0 as i128) << FRAC) / o.0 as i128) as i64)
    }

    pub(super) fn sqrt(a: Fixed) -> Fixed {
        if a.0 <= 0 {
            return Fixed::ZERO;
        }
        let n = (a.0 as u128) << FRAC;
        let mut x = 1u128 << ((128 - n.leading_zeros()).div_ceil(2));
        loop {
            let y = (x + n / x) >> 1;
            if y >= x {
                return Fixed(x as i64);
            }
            x = y;
        }
    }
}

impl Add for Fixed {
    type Output = Fixed;
    fn add(self, o: Fixed) -> Fixed {
        Fixed(self.0.wrapping_add(o.0))
    }
}
impl Sub for Fixed {
    type Output = Fixed;
    fn sub(self, o: Fixed) -> Fixed {
        Fixed(self.0.wrapping_sub(o.0))
    }
}
impl Neg for Fixed {
    type Output = Fixed;
    fn neg(self) -> Fixed {
        Fixed(self.0.wrapping_neg())
    }
}
impl Mul for Fixed {
    type Output = Fixed;
    fn mul(self, o: Fixed) -> Fixed {
        Fixed(((self.0 as i128 * o.0 as i128) >> FRAC) as i64)
    }
}
impl Div for Fixed {
    type Output = Fixed;
    /// The quotient truncated toward zero, zero for a division by zero, wrapping where the
    /// result does not fit (as the wide form's cast does).
    ///
    /// The quotient is `(a << 32) / b` on the raw values. Numerator and denominator may both
    /// be divided by the same power of two without changing it, so where `b` has trailing zero
    /// bits (a whole number has 32 of them) the shift of `a` shrinks by that many, and where
    /// what is left of the shift still fits 64 bits the division is one 64-bit division
    /// instead of a 128-bit one (#1017). Otherwise the wide form is used. The result is the
    /// same bit for bit either way ([`reference::div`] in the tests).
    fn div(self, o: Fixed) -> Fixed {
        if o.0 == 0 {
            return Fixed::ZERO;
        }
        let spare = o.0.trailing_zeros().min(FRAC);
        let shift = FRAC - spare;
        let shifted = self.0 << shift;
        if shifted >> shift == self.0 {
            // `wrapping_div`: the one quotient that does not fit (the lowest value by minus
            // one) wraps to the lowest value, which is what the wide form's cast gives.
            return Fixed(shifted.wrapping_div(o.0 >> spare));
        }
        Fixed((((self.0 as i128) << FRAC) / o.0 as i128) as i64)
    }
}

/// Binary formats (`postcard`: snapshots, replays, the wire) carry the exact
/// bits. Human-readable ones (JSON5 content, maps and fixtures) carry a plain
/// number, read with [`Fixed::from_f64`] like any loaded value.
#[cfg(feature = "serde")]
impl serde::Serialize for Fixed {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_f64(self.0 as f64 / (1u64 << FRAC) as f64)
        } else {
            s.serialize_i64(self.0)
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Fixed {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            f64::deserialize(d).map(Fixed::from_f64)
        } else {
            i64::deserialize(d).map(Fixed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let a = Fixed::from_int(3);
        let b = Fixed::from_int(4);
        assert_eq!((a * a + b * b).sqrt(), Fixed::from_int(5));
        assert_eq!(b / Fixed::from_int(2), Fixed::from_int(2));
        assert_eq!(Fixed::from_f64(0.5) * Fixed::from_int(6), a);
        assert_eq!((-a).to_f32(), -3.0);
    }

    /// Raw values at every edge: zero, the smallest steps, one, whole numbers, the lowest and
    /// highest values, and both sides of every power of two (where a shifted dividend stops
    /// fitting 64 bits, and where a divisor gains or loses a trailing zero bit).
    fn edges() -> Vec<i64> {
        let mut v = vec![0, 1, -1, 2, -2, 3, -3, i64::MAX, i64::MIN, i64::MAX - 1, i64::MIN + 1];
        for bit in 0..63 {
            let p = 1i64 << bit;
            for d in [-1, 0, 1] {
                v.push(p + d);
                v.push(-(p + d));
            }
            // a whole number of units, and one with a single low bit set below it
            v.push(p | (1 << (bit / 2)));
            v.push(-(p | (1 << (bit / 2))));
        }
        for whole in [1, 2, 3, 7, 10, 30, 100, 1000, 65_535, 65_536, i32::MAX, i32::MIN] {
            v.push(Fixed::from_int(whole).0);
            v.push(Fixed::from_int(whole).0.wrapping_neg());
            v.push(Fixed::from_int(whole).0.wrapping_add(1));
            v.push(Fixed::from_int(whole).0.wrapping_sub(1));
        }
        v.sort();
        v.dedup();
        v
    }

    /// A fixed sequence of raw values of every size (the same on every run and platform).
    fn random(count: usize) -> Vec<i64> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        (0..count)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                // spread over the magnitudes: from a few bits to the whole width
                (state as i64) >> (i % 61)
            })
            .collect()
    }

    #[test]
    fn division_equals_the_wide_form_bit_for_bit() {
        let edges = edges();
        for &a in &edges {
            for &b in &edges {
                assert_eq!(Fixed(a) / Fixed(b), reference::div(Fixed(a), Fixed(b)), "{a:#x} / {b:#x}");
            }
        }
        // division by zero is zero, as before, whatever the dividend
        for &a in &edges {
            assert_eq!(Fixed(a) / Fixed::ZERO, Fixed::ZERO);
        }
        // truncation toward zero for either sign of quotient
        let (seven, two) = (Fixed(7), Fixed::from_int(2));
        assert_eq!((seven / two).0, 3);
        assert_eq!((-seven / two).0, -3);
        assert_eq!((seven / -two).0, -3);
        assert_eq!((-seven / -two).0, 3);
        // the one quotient that does not fit wraps, as the wide form's cast does
        assert_eq!(Fixed(i64::MIN) / Fixed::from_int(-1), reference::div(Fixed(i64::MIN), Fixed::from_int(-1)));
        assert_eq!(Fixed(i64::MIN) / Fixed::from_int(-1), Fixed(i64::MIN));
        // a fixed set of random pairs, the divisor also as a whole number and as a power of two
        let values = random(40_000);
        for pair in values.chunks_exact(2) {
            let (a, b) = (pair[0], pair[1]);
            assert_eq!(Fixed(a) / Fixed(b), reference::div(Fixed(a), Fixed(b)), "{a:#x} / {b:#x}");
            let whole = Fixed::from_int((b >> 32) as i32);
            assert_eq!(Fixed(a) / whole, reference::div(Fixed(a), whole), "{a:#x} / whole {:#x}", whole.0);
            let power = Fixed(1i64 << (b.unsigned_abs() % 63));
            assert_eq!(Fixed(a) / power, reference::div(Fixed(a), power), "{a:#x} / {:#x}", power.0);
            assert_eq!(Fixed(a) / -power, reference::div(Fixed(a), -power), "{a:#x} / -{:#x}", power.0);
        }
    }

    #[test]
    fn square_root_equals_the_iterated_form_bit_for_bit() {
        for &a in &edges() {
            assert_eq!(Fixed(a).sqrt(), reference::sqrt(Fixed(a)), "sqrt {a:#x}");
        }
        for a in random(60_000) {
            assert_eq!(Fixed(a).sqrt(), reference::sqrt(Fixed(a)), "sqrt {a:#x}");
            // and of squares, where the root is exact or one step off it
            let square = Fixed(a >> 20) * Fixed(a >> 20);
            assert_eq!(square.sqrt(), reference::sqrt(square), "sqrt {:#x}", square.0);
        }
        // zero and below are zero, as before
        assert_eq!(Fixed::ZERO.sqrt(), Fixed::ZERO);
        assert_eq!(Fixed(-1).sqrt(), Fixed::ZERO);
        assert_eq!(Fixed(i64::MIN).sqrt(), Fixed::ZERO);
        // the largest value: the root is the largest whole number whose square fits
        let top = Fixed(i64::MAX).sqrt().0 as u128;
        let n = (i64::MAX as u128) << FRAC;
        assert!(top * top <= n && (top + 1) * (top + 1) > n);
    }
}
