//! Complete-profile repeated ANOVA.
use super::{arithmetic::*, cohort::AnovaCohort, *};
use crate::{
    analysis::{
        model::checked_sum,
        moments::{BoundedSum, Scaled},
        validation::invalid_analysis,
        Estimate,
    },
    data::{Budget, Cell, Dataset},
    error::AppResult,
    operation::JobControl,
};
#[cfg(test)]
mod tests;
pub(crate) struct RepeatedResult {
    pub(crate) outcome: AnovaOutcome,
    pub(crate) cell_means: Vec<Estimate>,
}
fn norm(v: Scaled, divisor: f64) -> Result<Scaled, AnovaFailure> {
    Ok(Scaled {
        fraction: normalized_product(v.fraction, 1., divisor)?,
        exponent: v.exponent,
    })
}
pub(super) fn profile(
    d: &Dataset,
    columns: &[crate::data::ColumnId],
    row: usize,
    c: &JobControl,
) -> AppResult<[f64; 64]> {
    if columns.len() > 64 {
        return Err(invalid_analysis());
    }
    let mut values = [0.; 64];
    for (j, &id) in columns.iter().enumerate() {
        c.checkpoint(j, None)?;
        let Cell::Number(y) = d.column(id)?.cell(row)? else {
            return Err(invalid_analysis());
        };
        values[j] = y;
    }
    Ok(values)
}
pub(super) fn score(values: &[f64], coefficients: &[i16], c: &JobControl) -> AppResult<BoundedSum> {
    if values.len() != coefficients.len() {
        return Err(invalid_analysis());
    }
    let mut sum = BoundedSum::default();
    for (j, (&y, &coefficient)) in values.iter().zip(coefficients).enumerate() {
        c.checkpoint(j, None)?;
        if coefficient != 0 {
            sum.push(
                if coefficient < 0 { -y } else { y },
                u64::from(coefficient.unsigned_abs()),
            )?;
        }
    }
    Ok(sum)
}
pub(crate) fn compute(
    d: &Dataset,
    r: &AnovaRequest,
    cohort: &AnovaCohort,
    c: &JobControl,
    extra_bytes: usize,
) -> AppResult<RepeatedResult> {
    let quiet = c.without_progress();
    let c = &quiet;
    c.check_cancelled()?;
    r.admit()?;
    r.validate_current(
        d,
        &[],
        checked_sum([
            extra_bytes,
            r.heap_bytes()?,
            std::mem::size_of::<AnovaRequest>(),
            cohort.owned_bytes()?,
        ])?,
        c,
    )?;
    let AnovaDesign::Repeated {
        measurements,
        factors,
        ..
    } = &r.design
    else {
        return Err(invalid_analysis());
    };
    let (n, k) = (cohort.rows.len(), measurements.len());
    if cohort.frequency != n as u64
        || cohort.cells.len() != k
        || cohort.factors.len() != factors.len()
        || cohort.rows.iter().any(|row| row.weight != 1)
    {
        return Err(invalid_analysis());
    }
    let effects = (1usize << factors.len()) - 1;
    let output = checked_sum([
        effects * std::mem::size_of::<AnovaEffect>(),
        k * std::mem::size_of::<Estimate>(),
        effects * 4096,
        8192,
    ])?;
    let workspace = 128 * std::mem::size_of::<BoundedSum>()
        + 64 * std::mem::size_of::<Scaled>()
        + 3 * 64 * 63 * 8
        + 64 * 63 * 2
        + 8192;
    if workspace > 768 * 1024 {
        return Err(invalid_analysis());
    }
    Budget::check_datasets(
        &[d],
        checked_sum([
            extra_bytes,
            r.heap_bytes()?,
            std::mem::size_of::<AnovaRequest>(),
            cohort.owned_bytes()?,
            output,
            2 * 1024 * 1024,
        ])?,
    )?;
    let mut sums = vec![BoundedSum::default(); k];
    let mut grand = BoundedSum::default();
    for row in &cohort.rows {
        c.checkpoint(row.row, None)?;
        for (j, &id) in measurements.iter().enumerate() {
            let Cell::Number(y) = d.column(id)?.cell(row.row)? else {
                return Err(invalid_analysis());
            };
            sums[j].push(y, 1)?;
            grand.push(y, 1)?;
        }
    }
    let means = sums
        .iter()
        .map(|s| {
            if n == 0 {
                Ok(Estimate::unavailable("No complete subjects."))
            } else {
                s.scaled(n as u64).map(|s| s.estimate())
            }
        })
        .collect::<AppResult<Vec<_>>>()?;
    if let Some(f) = &cohort.failure {
        return Ok(RepeatedResult {
            outcome: AnovaOutcome::Unavailable(f.clone()),
            cell_means: means,
        });
    }
    let mut total = EnergySum::default();
    let mut subjects = EnergySum::default();
    for row in &cohort.rows {
        c.checkpoint(row.row, None)?;
        let mut row_sum = BoundedSum::default();
        for &id in measurements {
            let Cell::Number(y) = d.column(id)?.cell(row.row)? else {
                return Err(invalid_analysis());
            };
            row_sum.push(y, 1)?;
            if let Err(f) = total.add(grand.residual(y, (n * k) as u64)?, 1) {
                return Ok(RepeatedResult {
                    outcome: AnovaOutcome::Unavailable(f),
                    cell_means: means,
                });
            }
        }
        if let Err(f) = subjects.add(
            row_sum.mean_difference(k as u64, &grand, (n * k) as u64)?,
            k as u64,
        ) {
            return Ok(RepeatedResult {
                outcome: AnovaOutcome::Unavailable(f),
                cell_means: means,
            });
        }
    }
    let (total, subjects) = match (total.finish(), subjects.finish()) {
        (Ok(t), Ok(s)) => (t, s),
        (Err(f), _) | (_, Err(f)) => {
            return Ok(RepeatedResult {
                outcome: AnovaOutcome::Unavailable(f),
                cell_means: means,
            })
        }
    };
    let levels = factors.iter().map(|f| f.levels.len()).collect::<Vec<_>>();
    let mut result = Vec::with_capacity(effects);
    let mut model = Energy::default();
    let mut error = Energy::default();
    let (mut model_valid, mut error_valid) = (true, true);
    for mask in 1..=effects {
        c.checkpoint(mask, None)?;
        let b = basis(&levels, mask, c)?;
        let value = effect_data(d, measurements, cohort, &sums, &b, c)?;
        match value {
            Ok((ss, err, gram)) => {
                match combine(model, ss) {
                    Ok(v) => model = v,
                    Err(_) => model_valid = false,
                }
                match combine(error, err) {
                    Ok(v) => error = v,
                    Err(_) => error_valid = false,
                }
                result.push(effect(mask, b.rank, ss, err, &gram, n, k, c)?);
            }
            Err(f) => result.push(unavailable_effect(mask, b.rank, n, f)),
        }
    }
    if result.iter().all(|e| e.ss.value.value.is_some()) && model_valid && error_valid {
        let sum = match combine(model, error).and_then(|v| combine(v, subjects)) {
            Ok(v) => v.value,
            Err(f) => {
                return Ok(RepeatedResult {
                    outcome: AnovaOutcome::Unavailable(f),
                    cell_means: means,
                })
            }
        };
        let relative = sum.ratio(total.value).value();
        if total.value.fraction != 0.
            && (!relative.is_finite() || (relative - 1.).abs() > MAX_RELATIVE)
        {
            return Ok(RepeatedResult {
                outcome: AnovaOutcome::Unavailable(failure(
                    "Repeated direct energies do not decompose.",
                )),
                cell_means: means,
            });
        }
    }
    let complete = result
        .iter()
        .all(|e| !matches!(e.ss.state, AnovaQuantityState::Unavailable(_)));
    let missing = || AnovaEnergy {
        state: AnovaQuantityState::Unavailable(AnovaFailureKind::Numerical),
        value: Estimate::unavailable("An effect energy is unresolved."),
    };
    Ok(RepeatedResult {
        outcome: AnovaOutcome::Fitted(AnovaModelResult {
            effects: result,
            error_ss: if complete && error_valid {
                error.captured()
            } else {
                missing()
            },
            model_ss: if complete && model_valid {
                model.captured()
            } else {
                missing()
            },
            total_ss: total.captured(),
            subject_ss: Some(subjects.captured()),
            residual_df: n.saturating_sub(1) as u64,
            total_df: (n * k - 1) as u64,
            balanced: true,
        }),
        cell_means: means,
    })
}
fn effect_data(
    d: &Dataset,
    cols: &[crate::data::ColumnId],
    cohort: &AnovaCohort,
    sums: &[BoundedSum],
    b: &Basis,
    c: &JobControl,
) -> AppResult<Result<(Energy, Energy, Vec<f64>), AnovaFailure>> {
    let n = cohort.rows.len() as u64;
    let mut aggregate = vec![BoundedSum::default(); b.rank];
    let mut ss = EnergySum::default();
    for i in 0..b.rank {
        for (j, s) in sums.iter().enumerate() {
            aggregate[i].add_sum(s, b.rows[i * b.cells + j])?;
        }
        let m = match norm(aggregate[i].scaled(n)?, b.norms[i]) {
            Ok(v) => v,
            Err(f) => return Ok(Err(f)),
        };
        if let Err(f) = ss.add(m, n) {
            return Ok(Err(f));
        }
    }
    let mut error = EnergySum::default();
    let mut common = None;
    for row in &cohort.rows {
        c.checkpoint(row.row, None)?;
        let values = profile(d, cols, row.row, c)?;
        for i in 0..b.rank {
            let s = score(
                &values[..cols.len()],
                &b.rows[i * b.cells..(i + 1) * b.cells],
                c,
            )?;
            let v = match norm(s.mean_difference(1, &aggregate[i], n)?, b.norms[i]) {
                Ok(v) => v,
                Err(f) => return Ok(Err(f)),
            };
            if v.fraction != 0. {
                common = Some(common.map_or(v.exponent, |e: i32| e.max(v.exponent)));
            }
            if let Err(f) = error.add(v, 1) {
                return Ok(Err(f));
            }
        }
    }
    let mut cross =
        vec![crate::analysis::inference::arithmetic::TailFloat(0., 0.); b.rank * b.rank];
    if let Some(exponent) = common {
        for row in &cohort.rows {
            c.checkpoint(row.row, None)?;
            let mut residual = [0.; 63];
            let values = profile(d, cols, row.row, c)?;
            for i in 0..b.rank {
                let s = score(
                    &values[..cols.len()],
                    &b.rows[i * b.cells..(i + 1) * b.cells],
                    c,
                )?;
                let v = match norm(s.mean_difference(1, &aggregate[i], n)?, b.norms[i])
                    .and_then(|v| at(v, exponent))
                {
                    Ok(v) => v,
                    Err(f) => return Ok(Err(f)),
                };
                residual[i] = v;
            }
            for i in 0..b.rank {
                for j in 0..b.rank {
                    let p = match normalized_product(residual[i], residual[j], 1.) {
                        Ok(_) => {
                            crate::analysis::inference::arithmetic::TailFloat(residual[i], 0.).mul(
                                crate::analysis::inference::arithmetic::TailFloat(residual[j], 0.),
                            )
                        }
                        Err(f) => return Ok(Err(f)),
                    };
                    let index = i * b.rank + j;
                    cross[index] = cross[index].add(p);
                }
            }
        }
    }
    let gram = cross.iter().map(|v| v.0 + v.1).collect();
    let (mut ss, mut error) = match (ss.finish(), error.finish()) {
        (Ok(s), Ok(e)) => (s, e),
        (Err(f), _) | (_, Err(f)) => return Ok(Err(f)),
    };
    let rho = 64. * U * (cohort.rows.len() + b.cells * b.rank + b.rank * b.rank + 16) as f64;
    if rho > MAX_RELATIVE {
        return Ok(Err(failure(
            "Repeated energy allowance exceeds its admitted bound.",
        )));
    }
    if ss.value.fraction != 0. {
        ss.relative_error = rho;
    }
    if error.value.fraction != 0. {
        error.relative_error = rho;
    }
    Ok(Ok((ss, error, gram)))
}
fn effect(
    mask: usize,
    rank: usize,
    ss: Energy,
    error: Energy,
    gram: &[f64],
    n: usize,
    k: usize,
    c: &JobControl,
) -> AppResult<AnovaEffect> {
    let df = (n.saturating_sub(1) * rank) as u64;
    let test = integer_f_test(ss, error, rank as u64, df, c)?;
    let gg = correction(ss, error, gram, n, k, rank, &test, c)?;
    let failure = if n < 2 {
        Some(AnovaFailure {
            kind: AnovaFailureKind::InsufficientErrorDf,
            reason: "At least two complete subjects are required.".into(),
        })
    } else if error.value.fraction == 0. {
        Some(AnovaFailure {
            kind: AnovaFailureKind::ZeroError,
            reason: "Subject-by-effect error is exactly zero.".into(),
        })
    } else {
        None
    };
    Ok(AnovaEffect {
        factors: (0..6).filter(|i| mask & (1 << i) != 0).collect(),
        dimension: rank,
        ss: ss.captured(),
        ms: ss.mean_square(rank as u64),
        error_ss: error.captured(),
        error_ms: error.mean_square(df),
        test,
        gg: Some(gg),
        eta_squared: Estimate::unavailable(
            "Ordinary eta squared is not reported for repeated effects.",
        ),
        partial_eta_squared: match combine(ss, error) {
            Ok(v) => energy_ratio(ss, v),
            Err(f) => Estimate::unavailable(&f.reason),
        },
        failure,
    })
}
fn correction(
    ss: Energy,
    error: Energy,
    gram: &[f64],
    n: usize,
    k: usize,
    rank: usize,
    test: &AnovaTest,
    c: &JobControl,
) -> AppResult<AnovaCorrection> {
    if rank == 1 {
        return Ok(AnovaCorrection {
            epsilon: Estimate::available(1.),
            covariance: if error.value.fraction == 0. {
                AnovaQuantityState::ExactZero
            } else {
                AnovaQuantityState::Positive
            },
            test: test.clone(),
            failure: None,
        });
    }
    let covariance = super::covariance::evaluate(gram, rank, n, k);
    let v = match covariance {
        Ok(v) => v,
        Err(mut f) => {
            f.kind = AnovaFailureKind::InvalidCovariance;
            let missing = || Estimate::unavailable(&f.reason);
            return Ok(AnovaCorrection {
                epsilon: missing(),
                covariance: if error.value.fraction == 0. {
                    AnovaQuantityState::ExactZero
                } else {
                    AnovaQuantityState::Unavailable(f.kind)
                },
                test: AnovaTest {
                    df1: missing(),
                    df2: missing(),
                    f: test.f.clone(),
                    p: missing(),
                },
                failure: Some(f),
            });
        }
    };
    let p = if rank == 1 {
        test.p.clone()
    } else if ss.value.fraction == 0. {
        Estimate::available(1.)
    } else {
        let log = ss.value.ratio(error.value).times((n - 1) as f64).log_abs();
        let delta = -(-ss.relative_error).ln_1p() - (-error.relative_error).ln_1p()
            + 32. * f64::EPSILON * (1. + log.abs());
        let center = tails::evaluate(log, v.df1, v.df2, c)?;
        let mut corners = Vec::with_capacity(4);
        for l in [log - delta, log + delta] {
            for e in [
                (v.epsilon * (1. - v.allowance)).max(v.lower),
                (v.epsilon * (1. + v.allowance)).min(v.upper),
            ] {
                let df1 = if n == 2 { 1. } else { e * rank as f64 };
                corners.push(tails::evaluate(l, df1, (n - 1) as f64 * df1, c)?);
            }
        }
        let upstream = match center.estimate.value {
            Some(p) if p > 0. => corners.iter().try_fold(0f64, |spread, e| {
                e.estimate
                    .value
                    .filter(|v| v.is_finite())
                    .map(|v| spread.max((v - p).abs() / p))
            }),
            _ => None,
        };
        tails::combined_probability(&center, &corners, upstream)
    };
    Ok(AnovaCorrection {
        epsilon: Estimate::available(v.epsilon),
        covariance: AnovaQuantityState::Positive,
        test: AnovaTest {
            df1: Estimate::available(v.df1),
            df2: Estimate::available(v.df2),
            f: test.f.clone(),
            p,
        },
        failure: None,
    })
}
fn unavailable_effect(mask: usize, rank: usize, n: usize, f: AnovaFailure) -> AnovaEffect {
    let missing = || Estimate::unavailable(&f.reason);
    let energy = || AnovaEnergy {
        state: AnovaQuantityState::Unavailable(f.kind),
        value: missing(),
    };
    let test = AnovaTest {
        df1: Estimate::available(rank as f64),
        df2: Estimate::available((n.saturating_sub(1) * rank) as f64),
        f: missing(),
        p: missing(),
    };
    AnovaEffect {
        factors: (0..6).filter(|i| mask & (1 << i) != 0).collect(),
        dimension: rank,
        ss: energy(),
        ms: missing(),
        error_ss: energy(),
        error_ms: missing(),
        gg: Some(AnovaCorrection {
            epsilon: if rank == 1 {
                Estimate::available(1.)
            } else {
                missing()
            },
            covariance: AnovaQuantityState::Unavailable(f.kind),
            test: if rank == 1 {
                test.clone()
            } else {
                AnovaTest {
                    df1: missing(),
                    df2: missing(),
                    f: missing(),
                    p: missing(),
                }
            },
            failure: Some(f.clone()),
        }),
        test,
        eta_squared: missing(),
        partial_eta_squared: missing(),
        failure: Some(f),
    }
}

fn combine(a: Energy, b: Energy) -> Result<Energy, AnovaFailure> {
    if a.value.fraction == 0. {
        return Ok(b);
    }
    if b.value.fraction == 0. {
        return Ok(a);
    }
    let exponent = a.value.exponent.max(b.value.exponent);
    at(a.value, exponent)?;
    at(b.value, exponent)?;
    Ok(Energy {
        value: a.value.plus(b.value),
        relative_error: a.relative_error.max(b.relative_error) + 8. * U,
    })
}
