//! 32.32 signed fixed-point number for deterministic simulation maths.

use std::ops::{Add, Div, Mul, Neg, Sub};

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

    /// Integer square root on the raw representation; exact and portable.
    pub fn sqrt(self) -> Self {
        if self.0 <= 0 {
            return Fixed::ZERO;
        }
        let n = (self.0 as u128) << FRAC;
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
    fn div(self, o: Fixed) -> Fixed {
        if o.0 == 0 {
            return Fixed::ZERO;
        }
        Fixed((((self.0 as i128) << FRAC) / o.0 as i128) as i64)
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
}
