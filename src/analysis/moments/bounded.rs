//! Opaque checked access to the existing finite accumulator.
use super::{ExactSum, Scaled};
use crate::error::{AppError, AppResult};
const MAX_COUNT: u64 = 1 << 53;
const MAX_MASS: u128 = 1 << 66;
#[derive(Default, Clone)]
pub(crate) struct BoundedSum {
    sum: ExactSum,
    mass: u128,
}
fn invalid(message: &str) -> AppError {
    AppError::invalid("Exact sum", message)
}
impl BoundedSum {
    pub(crate) fn push(&mut self, value: f64, weight: u64) -> AppResult<()> {
        if !value.is_finite() {
            return Err(invalid("Finite input required."));
        }
        let mass = self
            .mass
            .checked_add(u128::from(weight))
            .filter(|m| *m <= MAX_MASS)
            .ok_or_else(|| invalid("Coefficient mass exceeds 2^66."))?;
        self.sum.add(value, weight);
        self.mass = mass;
        Ok(())
    }
    pub(crate) fn add_sum(&mut self, other: &Self, coefficient: i16) -> AppResult<()> {
        let magnitude = u64::from(coefficient.unsigned_abs());
        if magnitude > 64 {
            return Err(invalid("Integral coefficient exceeds 64."));
        }
        let mass = other
            .mass
            .checked_mul(u128::from(magnitude))
            .and_then(|m| self.mass.checked_add(m))
            .filter(|m| *m <= MAX_MASS)
            .ok_or_else(|| invalid("Coefficient mass exceeds 2^66."))?;
        let mut positive = [0u64; 35];
        let mut negative = [0u64; 35];
        positive[..34].copy_from_slice(&self.sum.positive);
        negative[..34].copy_from_slice(&self.sum.negative);
        let (plus, minus) = if coefficient < 0 {
            (&other.sum.negative, &other.sum.positive)
        } else {
            (&other.sum.positive, &other.sum.negative)
        };
        cross_add(&mut positive, plus, magnitude)?;
        cross_add(&mut negative, minus, magnitude)?;
        if positive[34] != 0 || negative[34] != 0 {
            return Err(invalid("Merged sum limb bound exceeded."));
        }
        self.sum.positive.copy_from_slice(&positive[..34]);
        self.sum.negative.copy_from_slice(&negative[..34]);
        self.mass = mass;
        Ok(())
    }
    pub(crate) fn scaled(&self, divisor: u64) -> AppResult<Scaled> {
        if divisor == 0 {
            return Err(invalid("A positive divisor is required."));
        }
        Ok(self.sum.scaled(divisor))
    }
    pub(crate) fn residual(&self, value: f64, count: u64) -> AppResult<Scaled> {
        if count == 0 || count > MAX_COUNT {
            return Err(invalid("Count must be positive and at most 2^53."));
        }
        let mut copy = self.clone();
        copy.push(-value, count)?;
        copy.scaled(count)
    }
    pub(crate) fn mean_difference(&self, n: u64, other: &Self, m: u64) -> AppResult<Scaled> {
        if n == 0 || m == 0 || n > MAX_COUNT || m > MAX_COUNT {
            return Err(invalid("Counts must be positive and at most 2^53."));
        }
        let mut positive = [0u64; 35];
        let mut negative = [0u64; 35];
        cross_add(&mut positive, &self.sum.positive, m)?;
        cross_add(&mut positive, &other.sum.negative, n)?;
        cross_add(&mut negative, &self.sum.negative, m)?;
        cross_add(&mut negative, &other.sum.positive, n)?;
        let sign = positive.iter().rev().cmp(negative.iter().rev());
        let (large, small) = if sign.is_lt() {
            (&negative, &positive)
        } else {
            (&positive, &negative)
        };
        let mut magnitude = [0u64; 35];
        let mut borrow = false;
        for i in 0..35 {
            let (a, b) = large[i].overflowing_sub(small[i]);
            let (v, c) = a.overflowing_sub(borrow as u64);
            magnitude[i] = v;
            borrow = b || c;
        }
        if borrow {
            return Err(invalid("Cross subtraction invariant failed."));
        }
        let shift = usize::from(magnitude[34] != 0);
        let mut rounded = ExactSum::default();
        rounded
            .positive
            .copy_from_slice(&magnitude[shift..shift + 34]);
        // Sticky jam only discarded low bits; these lie far below the retained significand.
        if shift != 0 && magnitude[0] != 0 {
            rounded.positive[0] |= 1;
        }
        let numerator = rounded.scaled(1);
        let numerator = Scaled {
            fraction: if sign.is_lt() {
                -numerator.fraction
            } else {
                numerator.fraction
            },
            exponent: numerator.exponent + (64 * shift) as i32,
        };
        Ok(numerator.ratio(multiply(Scaled::new(n as f64), Scaled::new(m as f64))))
    }
}
fn cross_add(out: &mut [u64; 35], input: &[u64; 34], count: u64) -> AppResult<()> {
    let mut carry = 0u128;
    for i in 0..34 {
        let product = (input[i] as u128)
            .checked_mul(count as u128)
            .and_then(|v| v.checked_add(out[i] as u128))
            .and_then(|v| v.checked_add(carry))
            .ok_or_else(|| invalid("Cross carry overflow."))?;
        out[i] = product as u64;
        carry = product >> 64;
    }
    let high = (out[34] as u128)
        .checked_add(carry)
        .ok_or_else(|| invalid("Cross high overflow."))?;
    out[34] = u64::try_from(high).map_err(|_| invalid("Cross-count limb bound exceeded."))?;
    Ok(())
}

fn multiply(a: Scaled, b: Scaled) -> Scaled {
    let part = Scaled::new(a.fraction * b.fraction);
    Scaled {
        fraction: part.fraction,
        exponent: a.exponent + b.exponent + part.exponent,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_cross_count_avoids_rounded_anchor_cancellation() {
        let n = 3 * (1u64 << 50);
        let m = n - 1;
        let mut first = BoundedSum::default();
        let mut second = BoundedSum::default();
        first.push(2., 1).unwrap();
        first.push(1., n - 1).unwrap();
        second.push(2., 1).unwrap();
        second.push(1., m - 1).unwrap();
        let expected = -1. / (n as f64 * m as f64);
        let actual = first.mean_difference(n, &second, m).unwrap().value();
        assert!(
            (actual / expected - 1.).abs() < 1e-14,
            "{actual:e} != {expected:e}"
        );
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn mass_and_input_boundaries_preserve_prior_sum() {
        let mut s = BoundedSum::default();
        for _ in 0..4 {
            s.push(f64::MAX, u64::MAX).unwrap();
        }
        s.push(f64::MAX, 4).unwrap();
        let prior = s.scaled(1).unwrap();
        assert!(s.push(1., 1).is_err());
        assert!(s.push(f64::INFINITY, 0).is_err());
        assert!(s.push(f64::NAN, 1).is_err());
        assert_eq!(s.scaled(1).unwrap().fraction, prior.fraction);
        assert_eq!(s.scaled(1).unwrap().exponent, prior.exponent);
        assert!(s.scaled(0).is_err());
        assert!(s.residual(0., 0).is_err());
        assert!(s.residual(0., (1 << 53) + 1).is_err());
        assert!(s.mean_difference(0, &s, 1).is_err());
        assert!(s.mean_difference(1, &s, (1 << 53) + 1).is_err());
    }
    #[test]
    fn cross_count_high_limbs_signed_zero_and_subnormal() {
        let mut a = BoundedSum::default();
        let mut b = BoundedSum::default();
        a.push(f64::MAX, 1 << 53).unwrap();
        b.push(-f64::MAX, (1 << 53) - 1).unwrap();
        let d = a.mean_difference(1 << 53, &b, (1 << 53) - 1).unwrap();
        assert!(d.fraction > 0.);
        assert!(d.value().is_infinite());
        let zero = a.mean_difference(1 << 53, &a, 1 << 53).unwrap();
        assert_eq!(zero.fraction, 0.);
        let mut tiny = BoundedSum::default();
        tiny.push(f64::from_bits(1), 1).unwrap();
        let d = tiny
            .mean_difference(1 << 53, &BoundedSum::default(), 1)
            .unwrap();
        assert!(d.fraction > 0.);
        assert!(d.estimate().value.is_none());
        let mut zero = BoundedSum::default();
        zero.push(f64::MAX, 1 << 53).unwrap();
        zero.push(-f64::MAX, 1 << 53).unwrap();
        assert_eq!(zero.scaled(1).unwrap().fraction, 0.);
    }
}
#[cfg(test)]
mod merge_tests {
    use super::*;
    #[test]
    fn merged_signed_cells_match_row_stream_and_refuse_mass_before_commit() {
        for coefficient in [-64i16, -1, 0, 1, 64] {
            let mut cell = BoundedSum::default();
            let mut expected = BoundedSum::default();
            for (value, weight) in [(f64::MAX, 7), (-f64::MAX, 6), (f64::from_bits(1), 3)] {
                cell.push(value, weight).unwrap();
                expected
                    .push(
                        if coefficient < 0 { -value } else { value },
                        weight * u64::from(coefficient.unsigned_abs()),
                    )
                    .unwrap();
            }
            let mut merged = BoundedSum::default();
            merged.add_sum(&cell, coefficient).unwrap();
            let (a, b) = (merged.scaled(1).unwrap(), expected.scaled(1).unwrap());
            assert_eq!(a.fraction, b.fraction);
            assert_eq!(a.exponent, b.exponent);
            merged.add_sum(&cell, -coefficient).unwrap();
            assert_eq!(merged.scaled(1).unwrap().fraction, 0.);
        }
        let mut full = BoundedSum::default();
        for _ in 0..4 {
            full.push(1., u64::MAX).unwrap();
        }
        full.push(1., 4).unwrap();
        let mut destination = BoundedSum::default();
        destination.push(2., 1).unwrap();
        assert!(destination.add_sum(&full, 1).is_err());
        assert_eq!(destination.scaled(1).unwrap().value(), 2.);
        assert!(destination.add_sum(&full, 65).is_err());
    }
}
