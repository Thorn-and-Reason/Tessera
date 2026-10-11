//! Calibrated captured consistency screens; no numerical reconstruction.
use super::*;
// Captured consistency only: calibrated energy allowance, not an accuracy certificate.
const ENERGY_ALLOWANCE: f64 = 1. / 67_108_864.;
fn log_relationship(actual: f64, expected_log: f64, energies: usize) -> AppResult<()> {
    check(actual > 0. && expected_log.is_finite())?;
    let allowance = -(energies as f64) * (-ENERGY_ALLOWANCE).ln_1p()
        + 16. * f64::EPSILON * (1. + actual.ln().abs() + expected_log.abs());
    check((actual.ln() - expected_log).abs() <= allowance)
}
fn decomposition(total: &AnovaEnergy, parts: &[&AnovaEnergy]) -> AppResult<()> {
    let Some(total) = total.value.value else {
        return Ok(());
    };
    let values = parts
        .iter()
        .map(|e| e.value.value)
        .collect::<Option<Vec<_>>>();
    let Some(values) = values else {
        return Ok(());
    };
    let scale = values.iter().copied().fold(total, f64::max);
    if scale == 0. {
        return check(total == 0.);
    }
    let expected = values.iter().map(|v| v / scale).sum::<f64>();
    let actual = total / scale;
    let magnitude = expected.abs() + actual.abs();
    check(
        (actual - expected).abs()
            <= magnitude * (ENERGY_ALLOWANCE + ((parts.len() + 2) as f64) * f64::EPSILON),
    )
}
pub(super) fn scalar_relationships(
    r: &ComputedAnova,
    m: &AnovaModelResult,
    repeated: bool,
    c: &JobControl,
) -> AppResult<()> {
    if !repeated {
        decomposition(&m.total_ss, &[&m.error_ss, &m.model_ss])?;
        if m.balanced || r.grid.factors.len() == 1 {
            let effects = m.effects.iter().map(|e| &e.ss).collect::<Vec<_>>();
            decomposition(&m.model_ss, &effects)?;
        }
    } else {
        let mut parts = Vec::with_capacity(1 + 2 * m.effects.len());
        parts.push(m.subject_ss.as_ref().ok_or_else(invalid_analysis)?);
        for e in &m.effects {
            parts.extend([&e.ss, &e.error_ss]);
        }
        decomposition(&m.total_ss, &parts)?;
        let errors = m.effects.iter().map(|e| &e.error_ss).collect::<Vec<_>>();
        let effects = m.effects.iter().map(|e| &e.ss).collect::<Vec<_>>();
        decomposition(&m.error_ss, &errors)?;
        decomposition(&m.model_ss, &effects)?;
    }
    for (i, e) in m.effects.iter().enumerate() {
        c.checkpoint(i, None)?;
        if !repeated {
            check(e.error_ss == m.error_ss)?;
        }
        for (ss, ms, df) in [
            (&e.ss, &e.ms, e.dimension as f64),
            (&e.error_ss, &e.error_ms, e.test.df2.value.unwrap_or(0.)),
        ] {
            if let (Some(ss), Some(ms)) = (ss.value.value, ms.value) {
                if ss == 0. {
                    check(ms == 0.)?;
                } else {
                    check(df > 0.)?;
                    log_relationship(ms, ss.ln() - df.ln(), 1)?;
                }
            }
        }
        if let (Some(ss), Some(error), Some(f), Some(df2)) = (
            e.ss.value.value,
            e.error_ss.value.value,
            e.test.f.value,
            e.test.df2.value,
        ) {
            if ss == 0. {
                check(f == 0.)?;
            } else {
                check(error > 0. && df2 > 0.)?;
                log_relationship(
                    f,
                    ss.ln() - error.ln() + df2.ln() - (e.dimension as f64).ln(),
                    2,
                )?;
            }
        }
        if let (Some(ss), Some(total), Some(eta)) = (
            e.ss.value.value,
            m.total_ss.value.value,
            e.eta_squared.value,
        ) {
            if ss == 0. {
                check(total > 0. && eta == 0.)?;
            } else {
                check(total > 0.)?;
                log_relationship(eta, ss.ln() - total.ln(), 2)?;
            }
        }
        if let (Some(ss), Some(error), Some(eta)) = (
            e.ss.value.value,
            e.error_ss.value.value,
            e.partial_eta_squared.value,
        ) {
            let scale = ss.max(error);
            check(scale > 0.)?;
            let expected = (ss / scale) / (ss / scale + error / scale);
            if ss == 0. {
                check(eta == 0.)?;
            } else {
                log_relationship(eta, expected.ln(), 3)?;
            }
        }
    }
    let _ = r;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Estimate;
    fn positive(v: f64) -> AnovaEnergy {
        AnovaEnergy {
            state: AnovaQuantityState::Positive,
            value: Estimate {
                value: Some(v),
                reason: None,
            },
        }
    }
    #[test]
    fn calibrated_inside_outside_and_extreme_units() {
        let a = positive(1.);
        let b = positive(2.);
        assert!(decomposition(&positive(3. + ENERGY_ALLOWANCE), &[&a, &b]).is_ok());
        assert!(decomposition(&positive(3. + 100. * ENERGY_ALLOWANCE), &[&a, &b]).is_err());
        let a = positive(f64::MAX / 2.);
        let b = positive(f64::MAX / 4.);
        assert!(decomposition(&positive(f64::MAX * 0.75), &[&a, &b]).is_ok());
        assert!(log_relationship(1., 0., 2).is_ok());
        assert!(log_relationship(1. + ENERGY_ALLOWANCE, 0., 2).is_ok());
        assert!(log_relationship(1. + 100. * ENERGY_ALLOWANCE, 0., 2).is_err());
        assert!(log_relationship(f64::from_bits(1), f64::from_bits(1).ln(), 2).is_ok());
    }
}
