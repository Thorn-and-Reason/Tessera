use super::{Estimate, Statistic};

// Exact signed sums in units of the smallest subnormal. At total frequency <=2^53,
// 34 limbs cover every finite input and its integer frequency product.
const LIMBS: usize = 34;
#[derive(Clone)]
struct ExactSum {
    positive: [u64; LIMBS],
    negative: [u64; LIMBS],
}
impl Default for ExactSum {
    fn default() -> Self {
        Self {
            positive: [0; LIMBS],
            negative: [0; LIMBS],
        }
    }
}
impl ExactSum {
    fn add(&mut self, value: f64, weight: u64) {
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 2047) as usize;
        let mantissa = (bits & ((1_u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1_u64 << 52 };
        let shift = exponent.saturating_sub(1);
        let limbs = if value.is_sign_negative() {
            &mut self.negative
        } else {
            &mut self.positive
        };
        let product = u128::from(mantissa) * u128::from(weight);
        let offset = shift % 64;
        for (part, word) in [product as u64, (product >> 64) as u64]
            .into_iter()
            .enumerate()
        {
            add_word(limbs, shift / 64 + part, word << offset);
            if offset != 0 {
                add_word(limbs, shift / 64 + part + 1, word >> (64 - offset));
            }
        }
    }
    fn divided(&self, divisor: u64) -> f64 {
        let negative = self
            .negative
            .iter()
            .rev()
            .cmp(self.positive.iter().rev())
            .is_gt();
        let (large, small) = if negative {
            (&self.negative, &self.positive)
        } else {
            (&self.positive, &self.negative)
        };
        let mut magnitude = [0; LIMBS];
        let mut borrow = false;
        for index in 0..LIMBS {
            let (a, b1) = large[index].overflowing_sub(small[index]);
            let (value, b2) = a.overflowing_sub(u64::from(borrow));
            magnitude[index] = value;
            borrow = b1 || b2;
        }
        let mut remainder = 0_u128;
        for limb in magnitude.iter_mut().rev() {
            let dividend = (remainder << 64) | u128::from(*limb);
            *limb = (dividend / u128::from(divisor)) as u64;
            remainder = dividend % u128::from(divisor);
        }
        let value = rounded_float(&magnitude, remainder, divisor);
        if negative {
            -value
        } else {
            value
        }
    }
}
/// Convex finite endpoints use the existing exact integer sum and one final rounding.
/// The caller validates the fraction; even u64 weights fit the 34-limb accumulator.
pub(super) fn convex_interpolation(left: f64, right: f64, numerator: u64, denominator: u64) -> f64 {
    let mut sum = ExactSum::default();
    sum.add(left, denominator - numerator);
    sum.add(right, numerator);
    sum.divided(denominator)
}

/// Exact inclusive 1.5-IQR comparisons for finite quartiles and observations.
/// Integer coefficients avoid rounding a half-ULP or subnormal fence onto a point.
pub(crate) fn inside_box_fences(value: f64, q1: f64, q3: f64) -> bool {
    if !value.is_finite() || !q1.is_finite() || !q3.is_finite() || q1 > q3 {
        return false;
    }
    let side = |plus: f64, minus: f64| {
        let mut sum = ExactSum::default();
        sum.add(value, 2);
        sum.add(plus, 3);
        sum.add(-minus, 5);
        sum.positive.iter().rev().cmp(sum.negative.iter().rev())
    };
    side(q3, q1).is_ge() && side(q1, q3).is_le()
}
fn add_word(limbs: &mut [u64; LIMBS], mut index: usize, mut word: u64) {
    while word != 0 {
        let (value, carry) = limbs[index].overflowing_add(word);
        limbs[index] = value;
        word = u64::from(carry);
        index += 1;
    }
}
fn bit(limbs: &[u64; LIMBS], index: usize) -> bool {
    limbs[index / 64] & (1_u64 << (index % 64)) != 0
}
// The retained significand spans at most two limbs.
fn significand_window(limbs: &[u64; LIMBS], shift: usize) -> u64 {
    let index = shift / 64;
    let offset = shift % 64;
    let low = limbs[index] >> offset;
    let high = if offset == 0 {
        0
    } else {
        limbs.get(index + 1).copied().unwrap_or(0) << (64 - offset)
    };
    (low | high) & ((1_u64 << 53) - 1)
}

fn bits_below(limbs: &[u64; LIMBS], boundary: usize) -> bool {
    let whole = boundary / 64;
    let partial = boundary % 64;
    limbs[..whole].iter().any(|limb| *limb != 0)
        || (partial != 0 && limbs[whole] & ((1_u64 << partial) - 1) != 0)
}

fn rounded_float(limbs: &[u64; LIMBS], remainder: u128, divisor: u64) -> f64 {
    let highest = limbs
        .iter()
        .rposition(|limb| *limb != 0)
        .map(|index| index * 64 + 63 - limbs[index].leading_zeros() as usize);
    let Some(mut highest) = highest else {
        return if remainder * 2 > u128::from(divisor) {
            f64::from_bits(1)
        } else {
            0.0
        };
    };
    if highest < 52 {
        let bits = round_fraction(limbs[0], remainder, divisor);
        return f64::from_bits(bits);
    }
    let shift = highest - 52;
    let mut significand = significand_window(limbs, shift);
    if shift > 0 && bit(limbs, shift - 1) {
        let sticky = remainder != 0 || bits_below(limbs, shift - 1);
        if sticky || !significand.is_multiple_of(2) {
            significand += 1;
        }
    } else if shift == 0 {
        significand = round_fraction(significand, remainder, divisor);
    }
    if significand == 1_u64 << 53 {
        significand >>= 1;
        highest += 1;
    }
    let exponent = highest - 51;
    if exponent >= 2047 {
        return f64::INFINITY;
    }
    f64::from_bits((exponent as u64) << 52 | (significand & ((1_u64 << 52) - 1)))
}

fn round_fraction(significand: u64, remainder: u128, divisor: u64) -> u64 {
    let twice = remainder * 2;
    significand
        + u64::from(
            twice > u128::from(divisor)
                || (twice == u128::from(divisor) && !significand.is_multiple_of(2)),
        )
}

#[derive(Default)]
pub(crate) struct Moments {
    pub n: u64,
    sum: ExactSum,
    minimum: Option<f64>,
    maximum: Option<f64>,
}
impl Moments {
    pub fn add(&mut self, value: f64, weight: u64) {
        self.n += weight;
        self.sum.add(value, weight);
        self.minimum = Some(self.minimum.map_or(value, |old| old.min(value)));
        self.maximum = Some(self.maximum.map_or(value, |old| old.max(value)));
    }
    pub fn mean(&self) -> Option<f64> {
        (self.n != 0).then(|| self.sum.divided(self.n))
    }
    fn spread_scale(&self) -> f64 {
        let range = self.maximum.unwrap_or(0.0) - self.minimum.unwrap_or(0.0);
        if range.is_finite() {
            range
        } else {
            self.maximum.unwrap().abs().max(self.minimum.unwrap().abs())
        }
    }
    pub fn normalized(&self, value: f64) -> f64 {
        let range = self.maximum.unwrap() - self.minimum.unwrap();
        if range == 0.0 {
            0.0
        } else if range.is_finite() {
            (value - self.minimum.unwrap()) / range
        } else {
            value / self.spread_scale()
        }
    }
    pub fn estimate(&self, statistic: Statistic, norm: &ScaledNorm) -> Estimate {
        if self.n == 0 {
            return Estimate::unavailable("No valid observations.");
        }
        if self.n < 2
            && matches!(
                statistic,
                Statistic::SampleStdDev | Statistic::Variance | Statistic::StdErrorMean
            )
        {
            return Estimate::unavailable("At least two observations are required.");
        }
        match statistic {
            Statistic::Mean => Estimate::representable(self.sum.divided(self.n)),
            Statistic::Sum => Estimate::representable(self.sum.divided(1)),
            Statistic::Minimum => Estimate::available(self.minimum.unwrap()),
            Statistic::Maximum => Estimate::available(self.maximum.unwrap()),
            Statistic::Range => {
                Estimate::representable(self.maximum.unwrap() - self.minimum.unwrap())
            }
            Statistic::SampleStdDev => spread_estimate(
                self.spread_scale() * (norm.scale * (norm.squares / (self.n - 1) as f64).sqrt()),
                norm.scale != 0.0,
            ),
            Statistic::StdErrorMean => spread_estimate(
                self.spread_scale()
                    * (norm.scale * (norm.squares / (self.n - 1) as f64 / self.n as f64).sqrt()),
                norm.scale != 0.0,
            ),
            Statistic::Variance => {
                let sd = self.spread_scale()
                    * (norm.scale * (norm.squares / (self.n - 1) as f64).sqrt());
                if norm.scale != 0.0 && sd * sd == 0.0 {
                    Estimate::unavailable("Positive variance is below Float64 range.")
                } else {
                    Estimate::representable(sd * sd)
                }
            }
        }
    }
}
#[derive(Default)]
pub(crate) struct ScaledNorm {
    scale: f64,
    squares: f64,
    compensation: f64,
}
impl ScaledNorm {
    pub fn add_deviation(&mut self, value: f64, mean: f64, weight: u64) {
        let difference = value - mean;
        let (magnitude, frequency) = (difference.abs(), weight as f64);
        if magnitude == 0.0 {
            return;
        }
        let contribution = if self.scale < magnitude {
            let ratio = self.scale / magnitude;
            self.squares *= ratio * ratio;
            self.compensation *= ratio * ratio;
            self.scale = magnitude;
            frequency
        } else {
            frequency * (magnitude / self.scale).powi(2)
        };
        let corrected = contribution - self.compensation;
        let sum = self.squares + corrected;
        self.compensation = (sum - self.squares) - corrected;
        self.squares = sum;
    }
}

fn spread_estimate(value: f64, positive: bool) -> Estimate {
    if value == 0.0 && positive {
        Estimate::unavailable("Positive spread is below Float64 range.")
    } else {
        Estimate::representable(value)
    }
}

// Exponent-scaled values keep ratios independent of Float64 display limits.
#[derive(Clone, Copy, Default)]
pub(crate) struct Scaled {
    pub fraction: f64,
    pub exponent: i32,
}
impl Scaled {
    pub fn new(value: f64) -> Self {
        if value == 0.0 {
            return Self::default();
        }
        let exponent = value.abs().log2().floor() as i32;
        let exponent = exponent.clamp(-1074, 1023);
        let fraction = if exponent < -1022 {
            value * 2f64.powi(1022) / 2f64.powi(exponent + 1022)
        } else {
            value / 2f64.powi(exponent)
        };
        Self { fraction, exponent }
    }
    pub fn times(self, factor: f64) -> Self {
        let part = Self::new(self.fraction * factor);
        Self {
            fraction: part.fraction,
            exponent: self.exponent + part.exponent,
        }
    }
    pub fn plus(self, other: Self) -> Self {
        if self.fraction == 0.0 {
            return other;
        }
        if other.fraction == 0.0 {
            return self;
        }
        let exponent = self.exponent.max(other.exponent);
        let part = Self::new(
            self.fraction * 2f64.powi(self.exponent - exponent)
                + other.fraction * 2f64.powi(other.exponent - exponent),
        );
        Self {
            fraction: part.fraction,
            exponent: exponent + part.exponent,
        }
    }
    pub fn ratio(self, other: Self) -> Self {
        let part = Self::new(self.fraction / other.fraction);
        Self {
            fraction: part.fraction,
            exponent: self.exponent - other.exponent + part.exponent,
        }
    }
    pub fn value(self) -> f64 {
        if self.fraction == 0.0 {
            return 0.0;
        }
        if self.exponent < -1022 {
            (self.fraction * 2f64.powi(self.exponent + 1022)) * 2f64.powi(-1022)
        } else if self.exponent > 1023 {
            (self.fraction * 2f64.powi(self.exponent - 1023)) * 2f64.powi(1023)
        } else {
            self.fraction * 2f64.powi(self.exponent)
        }
    }
    pub fn estimate(self) -> Estimate {
        let value = self.value();
        if value == 0.0 && self.fraction != 0.0 {
            Estimate::unavailable("The estimate is below Float64 range.")
        } else {
            Estimate::representable(value)
        }
    }
    pub fn log_abs(self) -> f64 {
        self.fraction.abs().ln() + f64::from(self.exponent) * std::f64::consts::LN_2
    }
}
impl ExactSum {
    fn scaled(&self, divisor: u64) -> Scaled {
        let negative = self
            .negative
            .iter()
            .rev()
            .cmp(self.positive.iter().rev())
            .is_gt();
        let (large, small) = if negative {
            (&self.negative, &self.positive)
        } else {
            (&self.positive, &self.negative)
        };
        let mut magnitude = [0; LIMBS];
        let mut borrow = false;
        for index in 0..LIMBS {
            let (a, b) = large[index].overflowing_sub(small[index]);
            let (v, c) = a.overflowing_sub(u64::from(borrow));
            magnitude[index] = v;
            borrow = b || c;
        }
        let Some(index) = magnitude.iter().rposition(|x| *x != 0) else {
            return Scaled::default();
        };
        let highest = index * 64 + 63 - magnitude[index].leading_zeros() as usize;
        let shift = highest.saturating_sub(52);
        let mut significand = significand_window(&magnitude, shift);
        if shift > 0
            && bit(&magnitude, shift - 1)
            && (bits_below(&magnitude, shift - 1) || !significand.is_multiple_of(2))
        {
            significand += 1;
        }
        let part =
            Scaled::new(significand as f64 / divisor as f64 * if negative { -1. } else { 1. });
        Scaled {
            fraction: part.fraction,
            exponent: part.exponent + shift as i32 - 1074,
        }
    }
}
#[derive(Default)]
pub(crate) struct ContrastMoments {
    pub n: u64,
    sum: ExactSum,
}
impl ContrastMoments {
    pub fn add(&mut self, first: f64, second: f64, weight: u64) {
        self.n += weight;
        self.sum.add(first, weight);
        self.sum.add(-second, weight);
    }
    pub fn mean(&self) -> Scaled {
        self.sum.scaled(self.n.max(1))
    }
    pub fn difference(&self, reference: f64) -> Scaled {
        let mut sum = self.sum.clone();
        sum.add(-reference, self.n);
        sum.scaled(self.n.max(1))
    }
    pub fn contrast(&self, other: &Self) -> Scaled {
        let anchor = self.mean().value();
        self.difference(anchor)
            .plus(other.difference(anchor).times(-1.))
    }
    pub fn deviation(&self, first: f64, second: f64) -> Scaled {
        let mut sum = self.sum.clone();
        sum.add(-first, self.n);
        sum.add(second, self.n);
        sum.scaled(self.n.max(1))
    }
}
#[derive(Default)]
pub(crate) struct ContrastNorm {
    scale: Scaled,
    norm: ScaledNorm,
}
impl ContrastNorm {
    pub fn add(&mut self, value: Scaled, weight: u64) {
        if value.fraction == 0.0 {
            return;
        }
        if self.scale.fraction == 0.0 {
            self.scale = value;
        }
        if value.exponent > self.scale.exponent {
            let ratio = self.scale.ratio(value).value().abs();
            self.norm.scale *= ratio;
            self.scale = value;
        }
        self.norm
            .add_deviation(value.ratio(self.scale).value(), 0., weight);
    }
    pub fn sd(&self, n: u64) -> Scaled {
        self.scale
            .times(self.norm.scale.abs() * (self.norm.squares / (n - 1) as f64).sqrt())
            .times(self.scale.fraction.signum())
    }
}

mod bounded;
pub(crate) use bounded::BoundedSum;

#[cfg(test)]
mod rounding_tests {
    use super::*;
    fn slow_rounded_float(limbs: &[u64; LIMBS], remainder: u128, divisor: u64) -> f64 {
        let highest = limbs
            .iter()
            .rposition(|limb| *limb != 0)
            .map(|index| index * 64 + 63 - limbs[index].leading_zeros() as usize);
        let Some(mut highest) = highest else {
            return if remainder * 2 > u128::from(divisor) {
                f64::from_bits(1)
            } else {
                0.0
            };
        };
        if highest < 52 {
            let bits = round_fraction(limbs[0], remainder, divisor);
            return f64::from_bits(bits);
        }
        let shift = highest - 52;
        let mut significand = 0_u64;
        for offset in 0..53 {
            if bit(limbs, shift + offset) {
                significand |= 1_u64 << offset;
            }
        }
        if shift > 0 && bit(limbs, shift - 1) {
            let sticky = remainder != 0 || (0..shift - 1).any(|index| bit(limbs, index));
            if sticky || !significand.is_multiple_of(2) {
                significand += 1;
            }
        } else if shift == 0 {
            significand = round_fraction(significand, remainder, divisor);
        }
        if significand == 1_u64 << 53 {
            significand >>= 1;
            highest += 1;
        }
        let exponent = highest - 51;
        if exponent >= 2047 {
            return f64::INFINITY;
        }
        f64::from_bits((exponent as u64) << 52 | (significand & ((1_u64 << 52) - 1)))
    }

    impl ExactSum {
        fn slow_scaled(&self, divisor: u64) -> Scaled {
            let negative = self
                .negative
                .iter()
                .rev()
                .cmp(self.positive.iter().rev())
                .is_gt();
            let (large, small) = if negative {
                (&self.negative, &self.positive)
            } else {
                (&self.positive, &self.negative)
            };
            let mut magnitude = [0; LIMBS];
            let mut borrow = false;
            for index in 0..LIMBS {
                let (a, b) = large[index].overflowing_sub(small[index]);
                let (v, c) = a.overflowing_sub(u64::from(borrow));
                magnitude[index] = v;
                borrow = b || c;
            }
            let Some(index) = magnitude.iter().rposition(|x| *x != 0) else {
                return Scaled::default();
            };
            let highest = index * 64 + 63 - magnitude[index].leading_zeros() as usize;
            let shift = highest.saturating_sub(52);
            let mut significand = 0u64;
            for offset in 0..=highest - shift {
                if bit(&magnitude, shift + offset) {
                    significand |= 1 << offset;
                }
            }
            if shift > 0
                && bit(&magnitude, shift - 1)
                && ((0..shift - 1).any(|i| bit(&magnitude, i)) || !significand.is_multiple_of(2))
            {
                significand += 1;
            }
            let part =
                Scaled::new(significand as f64 / divisor as f64 * if negative { -1. } else { 1. });
            Scaled {
                fraction: part.fraction,
                exponent: part.exponent + shift as i32 - 1074,
            }
        }
    }
    #[test]
    fn limb_windows_match_bit_reference_at_every_boundary() {
        let zero = [0; LIMBS];
        for divisor in [1, 2, 3, u64::MAX] {
            for remainder in [0, divisor / 2, divisor - 1] {
                assert_eq!(
                    rounded_float(&zero, u128::from(remainder), divisor).to_bits(),
                    slow_rounded_float(&zero, u128::from(remainder), divisor).to_bits()
                );
            }
            let cancelled = ExactSum {
                positive: [u64::MAX; LIMBS],
                negative: [u64::MAX; LIMBS],
            };
            assert_eq!(
                cancelled.scaled(divisor).fraction.to_bits(),
                0.0_f64.to_bits()
            );
            assert_eq!(cancelled.scaled(divisor).exponent, 0);
        }
        let mut seed = 0x7e57_5eed_1234_5678_u64;
        for highest in 0..LIMBS * 64 {
            for pattern in 0..6 {
                let mut magnitude = [0; LIMBS];
                for index in 0..=highest {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let set = match pattern {
                        0 => false,
                        1 => true,
                        2 => index % 64 == 0,
                        3 => index % 64 == 63,
                        4 => index == highest.saturating_sub(53),
                        _ => seed & 1 != 0,
                    };
                    if set || index == highest {
                        magnitude[index / 64] |= 1 << (index % 64);
                    }
                }
                let shift = highest.saturating_sub(52);
                let expected = (0..=highest - shift).fold(0, |value, offset| {
                    value | (u64::from(bit(&magnitude, shift + offset)) << offset)
                });
                assert_eq!(significand_window(&magnitude, shift), expected);
                for boundary in [0, shift.saturating_sub(1), shift, highest, highest + 1] {
                    assert_eq!(
                        bits_below(&magnitude, boundary),
                        (0..boundary).any(|index| bit(&magnitude, index))
                    );
                }
                for divisor in [1, 2, 3, (1_u64 << 53) - 1, 1_u64 << 53, u64::MAX] {
                    for remainder in [0, divisor / 2, divisor - 1] {
                        assert_eq!(
                            rounded_float(&magnitude, u128::from(remainder), divisor).to_bits(),
                            slow_rounded_float(&magnitude, u128::from(remainder), divisor)
                                .to_bits()
                        );
                    }
                    for negative in [false, true] {
                        let sum = if negative {
                            ExactSum {
                                positive: [0; LIMBS],
                                negative: magnitude,
                            }
                        } else {
                            ExactSum {
                                positive: magnitude,
                                negative: [0; LIMBS],
                            }
                        };
                        let actual = sum.scaled(divisor);
                        let expected = sum.slow_scaled(divisor);
                        assert_eq!(actual.fraction.to_bits(), expected.fraction.to_bits());
                        assert_eq!(actual.exponent, expected.exponent);
                    }
                }
            }
        }
    }
}
