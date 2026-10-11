//! Shared two-component compensated arithmetic for inference and regression scaling.
//! Fixed series bounds avoid libm log rounding being magnified by df or t.
#[derive(Clone, Copy)]
pub(crate) struct TailFloat(pub f64, pub f64);
impl TailFloat {
    pub(crate) const LN_2: Self = Self(std::f64::consts::LN_2, 2.3190468138462996e-17);
    pub(crate) const PI: Self = Self(std::f64::consts::PI, 1.2246467991473532e-16);
    pub(crate) const ONE: Self = Self(1.0, 0.0);
    pub(crate) fn add(self, other: Self) -> Self {
        let sum = self.0 + other.0;
        let part = sum - self.0;
        let error = (self.0 - (sum - part)) + (other.0 - part) + self.1 + other.1;
        let high = sum + error;
        Self(high, error - (high - sum))
    }
    pub(crate) fn neg(self) -> Self {
        Self(-self.0, -self.1)
    }
    pub(crate) fn sub(self, other: Self) -> Self {
        self.add(other.neg())
    }
    pub(crate) fn mul(self, other: Self) -> Self {
        let product = self.0 * other.0;
        let error = self.0.mul_add(other.0, -product)
            + self.0 * other.1
            + self.1 * other.0
            + self.1 * other.1;
        let high = product + error;
        Self(high, error - (high - product))
    }
    pub(crate) fn div(self, other: Self) -> Self {
        let first = self.0 / other.0;
        let residual = self.sub(other.mul(Self(first, 0.0)));
        Self(first, 0.0).add(Self(residual.0 / other.0, 0.0))
    }
    pub(crate) fn ln(self) -> Self {
        let exponent = (((self.0.to_bits() >> 52) & 0x7ff) as i32 - 1023).clamp(-1022, 1022);
        let mantissa = self.mul(Self(2_f64.powi(-exponent), 0.0));
        let y = mantissa.sub(Self::ONE).div(mantissa.add(Self::ONE));
        Self::atanh_series(y, 64)
            .mul(Self(2.0, 0.0))
            .add(Self::LN_2.mul(Self(f64::from(exponent), 0.0)))
    }
    pub(crate) fn ln_1p(self) -> Self {
        if self.0 > 0.5 {
            return self.add(Self::ONE).ln();
        }
        let y = self.div(Self(2.0, 0.0).add(self));
        Self::atanh_series(y, 24).mul(Self(2.0, 0.0))
    }
    fn atanh_series(y: Self, terms: i32) -> Self {
        let squared = y.mul(y);
        let mut term = y;
        let mut sum = y;
        for i in 1..terms {
            term = term.mul(squared);
            sum = sum.add(term.div(Self(f64::from(2 * i + 1), 0.0)));
        }
        sum
    }
    pub(crate) fn exp(self) -> Self {
        if self.0 < -746.0 {
            return Self(0.0, 0.0);
        }
        let exponent = (self.0 / std::f64::consts::LN_2).round() as i32;
        let remainder = self.sub(Self::LN_2.mul(Self(f64::from(exponent), 0.0)));
        let mut term = Self::ONE;
        let mut sum = Self::ONE;
        for i in 1..32 {
            term = term.mul(remainder).div(Self(f64::from(i), 0.0));
            sum = sum.add(term);
        }
        // Perform the final subnormal rounding once, after reducing the exponent.
        if exponent < -1022 {
            let scaled = sum.mul(Self(2_f64.powi(exponent + 1022), 0.0));
            return Self((scaled.0 + scaled.1) * f64::MIN_POSITIVE, 0.0);
        }
        sum.mul(Self(2_f64.powi(exponent), 0.0))
    }
}
