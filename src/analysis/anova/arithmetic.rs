//! Bounded carried energies and deterministic integer effect spaces.
use super::*;
use crate::analysis::Estimate;
use crate::{
    analysis::moments::Scaled,
    error::{AppError, AppResult},
    operation::JobControl,
};
pub(crate) const MAX_RELATIVE: f64 = 1. / 67_108_864.;
pub(crate) const U: f64 = f64::EPSILON / 2.;
pub(crate) fn failure(reason: &str) -> AnovaFailure {
    AnovaFailure {
        kind: AnovaFailureKind::Numerical,
        reason: reason.into(),
    }
}
pub(crate) fn multiply(a: Scaled, b: Scaled) -> Scaled {
    let p = Scaled::new(a.fraction * b.fraction);
    Scaled {
        fraction: p.fraction,
        exponent: a.exponent + b.exponent + p.exponent,
    }
}
pub(crate) fn at(value: Scaled, exponent: i32) -> Result<f64, AnovaFailure> {
    if value.fraction == 0. {
        return Ok(0.);
    }
    let result = value.fraction * 2f64.powi(value.exponent - exponent);
    if !result.is_finite() || (result == 0. && value.fraction != 0.) {
        Err(failure(
            "A nonzero component cannot be retained at the common scale.",
        ))
    } else {
        Ok(result)
    }
}
/// Refuse a known nonzero term lost during product formation or normalisation.
pub(crate) fn normalized_product(
    value: f64,
    factor: f64,
    divisor: f64,
) -> Result<f64, AnovaFailure> {
    if !value.is_finite() || !factor.is_finite() || !divisor.is_finite() || divisor <= 0. {
        return Err(failure("Invalid contrast normalisation."));
    }
    let product = value * factor;
    let result = product / divisor;
    if !product.is_finite()
        || !result.is_finite()
        || (value != 0. && factor != 0. && product == 0.)
        || (product != 0. && result == 0.)
    {
        Err(failure(
            "A nonzero contrast component or allowance cannot be retained during normalisation.",
        ))
    } else {
        Ok(result)
    }
}
#[derive(Clone, Copy, Default)]
pub(crate) struct Energy {
    pub(crate) value: Scaled,
    pub(crate) relative_error: f64,
}
impl Energy {
    pub(crate) fn captured(self) -> AnovaEnergy {
        AnovaEnergy {
            state: if self.value.fraction == 0. {
                AnovaQuantityState::ExactZero
            } else {
                AnovaQuantityState::Positive
            },
            value: publish_energy(self.value),
        }
    }
    pub(crate) fn mean_square(self, df: u64) -> Estimate {
        if df == 0 {
            Estimate::unavailable("There are no error degrees of freedom.")
        } else {
            publish_energy(self.value.times(1. / df as f64))
        }
    }
}
// Original-unit subnormals cannot support the captured relative consistency screen.
// Keep carried energies and their inference independent of scalar publication.
fn publish_energy(value: Scaled) -> Estimate {
    let candidate = value.estimate();
    if candidate
        .value
        .is_some_and(|v| v > 0. && v < f64::MIN_POSITIVE)
    {
        Estimate::unavailable("Original-unit energy publication rounding is unresolved.")
    } else {
        candidate
    }
}
#[derive(Default)]
pub(crate) struct EnergySum {
    value: Scaled,
    terms: usize,
}
impl EnergySum {
    pub(crate) fn add(&mut self, deviation: Scaled, weight: u64) -> Result<(), AnovaFailure> {
        if deviation.fraction == 0. || weight == 0 {
            return Ok(());
        }
        let term = multiply(deviation, deviation).times(weight as f64);
        if !term.fraction.is_finite() || term.fraction <= 0. {
            return Err(failure("Squared energy is invalid."));
        }
        let exponent = if self.value.fraction == 0. {
            term.exponent
        } else {
            self.value.exponent.max(term.exponent)
        };
        if self.value.fraction != 0. {
            at(self.value, exponent)?;
        }
        at(term, exponent)?;
        self.value = self.value.plus(term);
        self.terms += 1;
        Ok(())
    }
    pub(crate) fn finish(self) -> Result<Energy, AnovaFailure> {
        let relative_error = if self.value.fraction == 0. {
            0.
        } else {
            16. * U * (self.terms + 16) as f64
        };
        if relative_error > MAX_RELATIVE {
            return Err(failure(
                "Energy accumulation allowance exceeds its admitted bound.",
            ));
        }
        Ok(Energy {
            value: self.value,
            relative_error,
        })
    }
}
pub(crate) fn energy_ratio(numerator: Energy, denominator: Energy) -> Estimate {
    if denominator.value.fraction == 0. {
        return Estimate::unavailable("The energy denominator is exactly zero.");
    }
    let ratio = numerator.value.ratio(denominator.value).estimate();
    if ratio.value.is_some_and(|v| v < 0. || v > 1.) {
        Estimate::unavailable("The bounded energy ratio is outside its publication range.")
    } else {
        ratio
    }
}
#[derive(Debug)]
pub(crate) struct Basis {
    pub(crate) rows: Vec<i16>,
    pub(crate) norms: Vec<f64>,
    pub(crate) rank: usize,
    pub(crate) cells: usize,
}
/// The last factor varies fastest, matching cohort cell identities.
pub(crate) fn basis(levels: &[usize], mask: usize, control: &JobControl) -> AppResult<Basis> {
    let mut cells = 1usize;
    let mut rank = 1usize;
    for (i, &level) in levels.iter().enumerate() {
        if level < 2 {
            return Err(AppError::invalid(
                "ANOVA basis",
                "Each fitted factor needs at least two levels.",
            ));
        }
        cells = cells
            .checked_mul(level)
            .ok_or(AppError::SizeLimit { limit: MAX_GRID })?;
        if mask & (1 << i) != 0 {
            rank = rank
                .checked_mul(level - 1)
                .ok_or(AppError::SizeLimit { limit: MAX_GRID })?;
        }
    }
    if cells > MAX_GRID || mask == 0 || mask >= 1 << levels.len() {
        return Err(AppError::invalid("ANOVA basis", "Invalid effect grid."));
    }
    let mut rows = vec![0i16; cells * rank];
    let mut norms = Vec::with_capacity(rank);
    for r in 0..rank {
        control.checkpoint(r, None)?;
        let mut selections = [0usize; 6];
        let mut index = r;
        for j in (0..levels.len()).rev() {
            if mask & (1 << j) != 0 {
                selections[j] = index % (levels[j] - 1);
                index /= levels[j] - 1;
            }
        }
        let mut sum = 0i32;
        let mut square = 0u64;
        let mut mass = 0u64;
        for cell in 0..cells {
            control.checkpoint(cell, None)?;
            let mut coordinate = cell;
            let mut coefficient = 1i16;
            for j in (0..levels.len()).rev() {
                let level = coordinate % levels[j];
                coordinate /= levels[j];
                if mask & (1 << j) != 0 {
                    let h = selections[j] + 1;
                    let part = if level < h {
                        1
                    } else if level == h {
                        -(h as i16)
                    } else {
                        0
                    };
                    coefficient = coefficient
                        .checked_mul(part)
                        .ok_or_else(|| AppError::invalid("ANOVA basis", "Coefficient overflow."))?;
                }
            }
            rows[r * cells + cell] = coefficient;
            sum += i32::from(coefficient);
            square += i64::from(coefficient).pow(2) as u64;
            mass += u64::from(coefficient.unsigned_abs());
        }
        if sum != 0
            || square == 0
            || mass > 4096
            || rows[r * cells..(r + 1) * cells]
                .iter()
                .any(|c| c.unsigned_abs() > 64)
        {
            return Err(AppError::invalid(
                "ANOVA basis",
                "Integer contrast bounds failed.",
            ));
        }
        norms.push((square as f64).sqrt());
    }
    Ok(Basis {
        rows,
        norms,
        rank,
        cells,
    })
}
pub(crate) fn compensated(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut correction) = (0., 0.);
    for value in values {
        let y = value - correction;
        let next = sum + y;
        correction = (next - sum) - y;
        sum = next;
    }
    sum
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integer_spaces_are_zero_sum_orthogonal() {
        for levels in [vec![64], vec![2, 3], vec![4, 4, 4], vec![2, 2, 2, 2, 2, 2]] {
            for mask in 1..1 << levels.len() {
                let b = basis(&levels, mask, &JobControl::default()).unwrap();
                for i in 0..b.rank {
                    for j in 0..b.rank {
                        let product: i64 = (0..b.cells)
                            .map(|c| {
                                i64::from(b.rows[i * b.cells + c])
                                    * i64::from(b.rows[j * b.cells + c])
                            })
                            .sum();
                        if i != j {
                            assert_eq!(product, 0);
                        } else {
                            assert!(product > 0);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn positive_energy_survives_publication_limits_and_omission_refuses() {
        for value in [f64::from_bits(1), 2f64.powi(1000)] {
            let mut s = EnergySum::default();
            s.add(Scaled::new(value), 1).unwrap();
            let e = s.finish().unwrap();
            assert!(e.value.fraction > 0.);
            assert_eq!(e.captured().state, AnovaQuantityState::Positive);
            assert!(e.captured().value.value.is_none());
        }
        let mut s = EnergySum::default();
        s.add(Scaled::new(2f64.powi(1000)), 1).unwrap();
        assert!(s.add(Scaled::new(f64::from_bits(1)), 1).is_err());
    }
    #[test]
    fn normalisation_guards_score_products_and_absolute_allowances() {
        let tiny = f64::from_bits(1);
        assert!(normalized_product(tiny, 1., 6f64.sqrt()).is_err());
        assert!(normalized_product(tiny, 0.5, 1.).is_err());
        assert!(normalized_product(tiny, 16. * U * 4., 1.).is_err());
        assert_eq!(normalized_product(0., 1., 6f64.sqrt()).unwrap(), 0.);
        assert_eq!(normalized_product(tiny, 0., 1.).unwrap(), 0.);
        assert_eq!(normalized_product(6., 2., 3.).unwrap(), 4.);
        assert!(normalized_product(1., 1., 0.).is_err());
    }
}

pub(crate) fn integer_f_test(
    ss: Energy,
    error: Energy,
    df1: u64,
    df2: u64,
    c: &JobControl,
) -> AppResult<AnovaTest> {
    let missing = || {
        Estimate::unavailable("A positive error energy and error degrees of freedom are required.")
    };
    let (f, p) = if df2 == 0 || error.value.fraction == 0. {
        (missing(), missing())
    } else if ss.value.fraction == 0. {
        (Estimate::available(0.), Estimate::available(1.))
    } else {
        let f = ss.value.ratio(error.value).times(df2 as f64 / df1 as f64);
        let log = f.log_abs();
        let uncertainty = -(1. - ss.relative_error).ln() - (1. - error.relative_error).ln()
            + 32. * f64::EPSILON * (1. + log.abs());
        let center = crate::analysis::regression::f_upper_tail(log, df1, df2, c)?;
        let low = crate::analysis::regression::f_upper_tail(log - uncertainty, df1, df2, c)?;
        let high = crate::analysis::regression::f_upper_tail(log + uncertainty, df1, df2, c)?;
        // Calibrated upstream screen matches the existing 1e-10 conditional log-tail allowance.
        let p = match (center.value, low.value, high.value) {
            (Some(center), Some(a), Some(b))
                if center >= f64::MIN_POSITIVE
                    && center.is_finite()
                    && a >= f64::MIN_POSITIVE
                    && a.is_finite()
                    && b >= f64::MIN_POSITIVE
                    && b.is_finite()
                    && ((a - b).abs() / center) <= 1e-10 =>
            {
                Estimate::available(center)
            }
            _ => Estimate::unavailable(
                "Integer F-tail publication is unresolved under the upstream energy allowance.",
            ),
        };
        (f.estimate(), p)
    };
    Ok(AnovaTest {
        df1: Estimate::available(df1 as f64),
        df2: Estimate::available(df2 as f64),
        f,
        p,
    })
}

#[cfg(test)]
mod publication_tests;
