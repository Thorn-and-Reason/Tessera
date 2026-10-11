//! Fixed comparison families using private exact numerical state.
use super::accounting::outcome_bytes;
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
pub(crate) enum Input<'a> {
    Independent(Option<&'a super::independent::IndependentState>),
    Repeated,
}
#[cfg(test)]
mod tests;
fn copy_pair(p: &AnovaPairRequest, c: &JobControl) -> AppResult<AnovaPairRequest> {
    Ok(AnovaPairRequest {
        factor: p.factor,
        first: crate::analysis::support::clone_cell(&p.first, c)?,
        second: crate::analysis::support::clone_cell(&p.second, c)?,
        scope: super::clone_scope(&p.scope, c)?,
    })
}
pub(crate) fn compute(
    d: &Dataset,
    r: &AnovaRequest,
    cohort: &AnovaCohort,
    outcome: &AnovaOutcome,
    input: Input<'_>,
    c: &JobControl,
    extra_bytes: usize,
) -> AppResult<Vec<AnovaContrastFamily>> {
    let quiet = c.without_progress();
    let c = &quiet;
    c.check_cancelled()?;
    r.admit()?;
    let state_bytes = match &input {
        Input::Independent(Some(s)) => s.owned_bytes()?,
        _ => 0,
    };
    r.validate_current(
        d,
        &[],
        checked_sum([
            extra_bytes,
            r.heap_bytes()?,
            std::mem::size_of::<AnovaRequest>(),
            cohort.owned_bytes()?,
            state_bytes,
            outcome_bytes(outcome)?,
        ])?,
        c,
    )?;
    let repeated = matches!(r.design, AnovaDesign::Repeated { .. });
    if repeated != matches!(input, Input::Repeated) {
        return Err(invalid_analysis());
    }
    if matches!(outcome, AnovaOutcome::Fitted(_)) && matches!(input, Input::Independent(None)) {
        return Err(invalid_analysis());
    }
    if let Input::Independent(Some(state)) = &input {
        if state.cells.len() != cohort.cells.len()
            || state
                .cells
                .iter()
                .zip(&cohort.cells)
                .any(|(a, b)| a.count != b.frequency)
        {
            return Err(invalid_analysis());
        }
    }
    workspace_bounds(r)?;
    let families_bytes = super::accounting::family_bytes(r, &cohort.factors, c)?;
    Budget::check_datasets(
        &[d],
        checked_sum([
            extra_bytes,
            r.heap_bytes()?,
            std::mem::size_of::<AnovaRequest>(),
            cohort.owned_bytes()?,
            state_bytes,
            outcome_bytes(outcome)?,
            families_bytes,
            2 * 1024 * 1024,
        ])?,
    )?;
    let mut families =
        Vec::with_capacity(r.all_pairs.len() + usize::from(!r.planned_pairs.is_empty()));
    if !r.planned_pairs.is_empty() {
        let mut members = Vec::with_capacity(r.planned_pairs.len());
        for (i, p) in r.planned_pairs.iter().enumerate() {
            c.checkpoint(i, None)?;
            members.push(member(
                d,
                r,
                cohort,
                outcome,
                &input,
                copy_pair(p, c)?,
                i,
                c,
            )?);
        }
        families.push(family(AnovaFamilyKind::Planned, members, c)?);
    }
    for f in &r.all_pairs {
        c.checkpoint(0, None)?;
        let values = &cohort
            .factors
            .get(f.factor)
            .ok_or_else(invalid_analysis)?
            .values;
        let m = values.len() * (values.len().saturating_sub(1)) / 2;
        let mut members = Vec::with_capacity(m);
        for i in 0..values.len() {
            for j in i + 1..values.len() {
                c.checkpoint(members.len(), None)?;
                let p = AnovaPairRequest {
                    factor: f.factor,
                    first: crate::analysis::support::clone_cell(&values[i], c)?,
                    second: crate::analysis::support::clone_cell(&values[j], c)?,
                    scope: super::clone_scope(&f.scope, c)?,
                };
                members.push(member(d, r, cohort, outcome, &input, p, members.len(), c)?);
            }
        }
        families.push(family(
            AnovaFamilyKind::AllPairs(AnovaFamilyRequest {
                factor: f.factor,
                scope: super::clone_scope(&f.scope, c)?,
            }),
            members,
            c,
        )?);
    }
    Ok(families)
}
fn find(values: &[Cell], wanted: &Cell, c: &JobControl) -> AppResult<Option<usize>> {
    for (i, value) in values.iter().enumerate() {
        c.checkpoint(i, None)?;
        let same = match (value, wanted) {
            (Cell::Text(a), Cell::Text(b)) => {
                crate::analysis::exploration::ordering::compare_text(a, b, c)?.is_eq()
            }
            _ => value == wanted,
        };
        if same {
            return Ok(Some(i));
        }
    }
    Ok(None)
}
fn selected(
    p: &AnovaPairRequest,
    cohort: &AnovaCohort,
    c: &JobControl,
) -> AppResult<Result<(Vec<i16>, u64), AnovaFailure>> {
    let absent = || AnovaFailure {
        kind: AnovaFailureKind::AbsentRequestedLevel,
        reason: "A requested pair or fixed-slice level is absent from the realised grid.".into(),
    };
    let factors = &cohort.factors;
    let Some(first) = find(&factors[p.factor].values, &p.first, c)? else {
        return Ok(Err(absent()));
    };
    let Some(second) = find(&factors[p.factor].values, &p.second, c)? else {
        return Ok(Err(absent()));
    };
    let mut fixed = [None; 6];
    if let AnovaScope::Fixed(values) = &p.scope {
        let mut j = 0;
        for (i, f) in factors.iter().enumerate() {
            if i != p.factor {
                let Some(v) = find(&f.values, &values[j], c)? else {
                    return Ok(Err(absent()));
                };
                fixed[i] = Some(v);
                j += 1;
            }
        }
    }
    let mut coefficients = vec![0i16; cohort.cells.len()];
    let mut divisor = 0u64;
    for (cell, coefficient) in coefficients.iter_mut().enumerate() {
        let mut coordinate = cell;
        let mut target = 0;
        let mut matches = true;
        for i in (0..factors.len()).rev() {
            let v = coordinate % factors[i].values.len();
            coordinate /= factors[i].values.len();
            if i == p.factor {
                target = v;
            } else if fixed[i].is_some_and(|f| f != v) {
                matches = false;
            }
        }
        if matches && target == first {
            *coefficient = 1;
            divisor += 1;
        } else if matches && target == second {
            *coefficient = -1;
        }
    }
    Ok(Ok((coefficients, divisor)))
}
fn unavailable(p: AnovaPairRequest, ordinal: usize, f: AnovaFailure) -> AnovaComparison {
    let missing = || Estimate::unavailable(&f.reason);
    AnovaComparison {
        ordinal,
        request: p,
        state: if f.kind == AnovaFailureKind::AbsentRequestedLevel {
            AnovaComparisonState::AbsentRequestedLevel(f.clone())
        } else {
            AnovaComparisonState::Unavailable(f.clone())
        },
        estimate: missing(),
        standard_error: missing(),
        t: missing(),
        df: missing(),
        raw_p: missing(),
        adjusted_p: missing(),
        interval: [missing(), missing()],
        variance: AnovaQuantityState::Unavailable(f.kind),
    }
}
struct Pair {
    estimate: Scaled,
    absolute_error: Scaled,
    variance: Energy,
    df: u64,
}
fn member(
    d: &Dataset,
    r: &AnovaRequest,
    cohort: &AnovaCohort,
    outcome: &AnovaOutcome,
    input: &Input<'_>,
    request: AnovaPairRequest,
    ordinal: usize,
    c: &JobControl,
) -> AppResult<AnovaComparison> {
    let (coefficients, divisor) = match selected(&request, cohort, c)? {
        Ok(v) => v,
        Err(f) => return Ok(unavailable(request, ordinal, f)),
    };
    if let AnovaOutcome::Unavailable(f) = outcome {
        return Ok(unavailable(request, ordinal, f.clone()));
    }
    let value = match input {
        Input::Independent(Some(s)) => {
            independent_pair(s, &coefficients, divisor, cohort.frequency, c)?
        }
        Input::Independent(None) => Err(failure("The exact independent state is unavailable.")),
        Input::Repeated => repeated_pair(d, r, cohort, &coefficients, divisor, c)?,
    };
    let pair = match value {
        Ok(v) => v,
        Err(f) => return Ok(unavailable(request, ordinal, f)),
    };
    infer(pair, request, ordinal, r.confidence, c)
}
fn independent_pair(
    s: &super::independent::IndependentState,
    coefficients: &[i16],
    divisor: u64,
    n: u64,
    c: &JobControl,
) -> AppResult<Result<Pair, AnovaFailure>> {
    let positive = coefficients
        .iter()
        .enumerate()
        .filter(|(_, v)| **v == 1)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let negative = coefficients
        .iter()
        .enumerate()
        .filter(|(_, v)| **v == -1)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if positive.len() != negative.len() || positive.len() != divisor as usize || divisor == 0 {
        return Err(invalid_analysis());
    }
    let count = s.cells[positive[0]].count;
    let common = count > 0
        && positive
            .iter()
            .chain(&negative)
            .all(|&i| s.cells[i].count == count);
    let mut merged = BoundedSum::default();
    let mut differences = Vec::with_capacity(positive.len());
    let mut weight = 0.;
    for (&i, &j) in positive.iter().zip(&negative) {
        c.checkpoint(i, None)?;
        let (a, b) = (&s.cells[i], &s.cells[j]);
        if a.count == 0 || b.count == 0 {
            return Ok(Err(failure(
                "The comparison needs a complete realised grid.",
            )));
        }
        if common {
            merged.add_sum(&a.sum, 1)?;
            merged.add_sum(&b.sum, -1)?;
        } else {
            differences.push(a.sum.mean_difference(a.count, &b.sum, b.count)?);
        }
        weight += 1. / a.count as f64 + 1. / b.count as f64;
    }
    let (estimate, absolute_error) = if common {
        let denominator = count
            .checked_mul(divisor)
            .filter(|d| *d <= n)
            .ok_or_else(invalid_analysis)?;
        let estimate = merged.scaled(denominator)?;
        let absolute = Scaled {
            fraction: estimate.fraction.abs(),
            exponent: estimate.exponent,
        }
        .times(32. * U * (divisor + 16) as f64);
        (estimate, absolute)
    } else {
        let exponent = differences
            .iter()
            .filter(|s| s.fraction != 0.)
            .map(|s| s.exponent)
            .max()
            .unwrap_or(0);
        let values = match differences
            .iter()
            .map(|s| at(*s, exponent))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(v) => v,
            Err(f) => return Ok(Err(f)),
        };
        let estimate = compensated(values.iter().copied()) / divisor as f64;
        let absolute = compensated(values.iter().map(|v| v.abs())) / divisor as f64
            * 32.
            * U
            * (divisor + 16) as f64;
        if estimate == 0. && differences.iter().any(|s| s.fraction != 0.) {
            return Ok(Err(failure(
                "The independent pair cancellation is unresolved.",
            )));
        }
        if estimate != 0. && (absolute / estimate.abs() > MAX_RELATIVE || absolute == 0.) {
            return Ok(Err(failure(
                "The independent contrast allowance is unresolved.",
            )));
        }
        (
            Scaled {
                fraction: estimate,
                exponent,
            },
            Scaled {
                fraction: absolute,
                exponent,
            },
        )
    };
    let df = n.saturating_sub(s.cells.len() as u64);
    let variance = Energy {
        value: if df == 0 {
            Scaled::new(0.)
        } else {
            s.error
                .value
                .times(weight / (divisor * divisor) as f64 / df as f64)
        },
        relative_error: s.error.relative_error + 32. * U * (divisor + 16) as f64,
    };
    Ok(Ok(Pair {
        estimate,
        absolute_error,
        variance,
        df,
    }))
}
fn repeated_pair(
    d: &Dataset,
    r: &AnovaRequest,
    cohort: &AnovaCohort,
    coefficients: &[i16],
    divisor: u64,
    c: &JobControl,
) -> AppResult<Result<Pair, AnovaFailure>> {
    let AnovaDesign::Repeated { measurements, .. } = &r.design else {
        return Err(invalid_analysis());
    };
    let n = cohort.rows.len() as u64;
    let mut aggregate = BoundedSum::default();
    for row in &cohort.rows {
        c.checkpoint(row.row, None)?;
        aggregate.add_sum(&pair_score(d, measurements, row.row, coefficients, c)?, 1)?;
    }
    if n == 0 || divisor == 0 {
        return Ok(Err(failure("No complete subjects are available.")));
    }
    let mean = aggregate.scaled(n * divisor)?;
    let mut error = EnergySum::default();
    for row in &cohort.rows {
        c.checkpoint(row.row, None)?;
        let sum = pair_score(d, measurements, row.row, coefficients, c)?;
        let residual = sum
            .mean_difference(1, &aggregate, n)?
            .times(1. / divisor as f64);
        if let Err(f) = error.add(residual, 1) {
            return Ok(Err(f));
        }
    }
    let error = match error.finish() {
        Ok(v) => v,
        Err(f) => return Ok(Err(f)),
    };
    let rho = 64. * U * (cohort.rows.len() + measurements.len() + 17) as f64;
    let variance = Energy {
        value: if n < 2 {
            Scaled::new(0.)
        } else {
            error.value.times(1. / (n * (n - 1)) as f64)
        },
        relative_error: rho,
    };
    Ok(Ok(Pair {
        estimate: mean,
        absolute_error: Scaled {
            fraction: mean.fraction.abs(),
            exponent: mean.exponent,
        }
        .times(if mean.fraction == 0. { 0. } else { rho }),
        variance,
        df: n - 1,
    }))
}
fn sqrt(v: Scaled) -> Scaled {
    let odd = v.exponent.rem_euclid(2);
    Scaled {
        fraction: (v.fraction * 2f64.powi(odd)).sqrt(),
        exponent: (v.exponent - odd) / 2,
    }
}
fn infer(
    pair: Pair,
    request: AnovaPairRequest,
    ordinal: usize,
    confidence: crate::analysis::ConfidenceLevel,
    c: &JobControl,
) -> AppResult<AnovaComparison> {
    let missing = || {
        Estimate::unavailable(
            "Positive comparison variance and error degrees of freedom are required.",
        )
    };
    let estimate = publish(pair.estimate, pair.absolute_error);
    let mut result = AnovaComparison {
        ordinal,
        request,
        state: AnovaComparisonState::Available,
        estimate: estimate.clone(),
        standard_error: missing(),
        t: missing(),
        df: Estimate::available(pair.df as f64),
        raw_p: missing(),
        adjusted_p: missing(),
        interval: [missing(), missing()],
        variance: pair.variance.captured().state,
    };
    if pair.df == 0 {
        result.variance = AnovaQuantityState::Unavailable(AnovaFailureKind::InsufficientErrorDf);
        return Ok(result);
    }
    if pair.variance.value.fraction == 0. {
        result.standard_error = Estimate::available(0.);
        result.interval = [estimate.clone(), estimate];
        return Ok(result);
    }
    if pair.variance.relative_error > MAX_RELATIVE {
        return Ok(result);
    }
    let se = sqrt(pair.variance.value);
    result.standard_error = publish(se, se.times(0.5 * pair.variance.relative_error + 32. * U));
    let relative_mean = if pair.estimate.fraction == 0. {
        0.
    } else {
        pair.absolute_error.ratio(pair.estimate).value().abs()
    };
    let uncertainty = relative_mean + 0.5 * pair.variance.relative_error + 32. * U;
    if !uncertainty.is_finite() || uncertainty > MAX_RELATIVE {
        return Ok(result);
    }
    let t = pair.estimate.ratio(se);
    result.t = t.estimate();
    if result.t.value.is_none() {
        return Ok(result);
    }
    if t.fraction == 0. {
        result.raw_p = Estimate::available(1.);
    } else {
        let log = t.log_abs();
        let delta = -(-uncertainty).ln_1p() + 32. * U * (1. + log.abs());
        let a = crate::analysis::inference::student_t_two_sided_p_from_log(log, pair.df as f64, c)?;
        let b = crate::analysis::inference::student_t_two_sided_p_from_log(
            log - delta,
            pair.df as f64,
            c,
        )?;
        let e = crate::analysis::inference::student_t_two_sided_p_from_log(
            log + delta,
            pair.df as f64,
            c,
        )?;
        if let (Some(p), Some(low), Some(high)) = (a.value, b.value, e.value) {
            if [p, low, high]
                .iter()
                .all(|p| p.is_finite() && *p >= f64::MIN_POSITIVE)
                && (low - high).abs() / p <= 1e-10
            {
                result.raw_p = Estimate::available(p)
            }
        }
    }
    if let Some(critical) =
        crate::analysis::inference::student_t_critical(pair.df as f64, confidence, c)?.value
    {
        let Some(critical_error) = critical_allowance(critical, pair.df as f64, confidence, c)?
        else {
            return Ok(result);
        };
        let width = se.times(critical);
        let absolute = pair
            .absolute_error
            .plus(width.times(0.5 * pair.variance.relative_error + critical_error + 32. * U));
        for (i, sign) in [-1., 1.].into_iter().enumerate() {
            let bound = pair.estimate.plus(width.times(sign));
            let ratio = absolute.ratio(bound).value().abs();
            if bound.fraction != 0. && ratio.is_finite() && ratio <= MAX_RELATIVE {
                result.interval[i] = publish(bound, absolute);
            }
        }
    }
    if result.estimate.value.is_none() || result.interval.iter().any(|e| e.value.is_none()) {
        result.interval = [missing(), missing()];
    }
    Ok(result)
}
fn holm(raw: &[Option<f64>]) -> Option<Vec<f64>> {
    let mut ordered = raw
        .iter()
        .enumerate()
        .map(|(i, p)| {
            p.filter(|p| p.is_finite() && *p >= 0. && *p <= 1.)
                .map(|p| (i, p))
        })
        .collect::<Option<Vec<_>>>()?;
    ordered.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut adjusted = vec![0.; raw.len()];
    let mut previous = 0f64;
    for (i, (ordinal, p)) in ordered.into_iter().enumerate() {
        previous = previous.max(((raw.len() - i) as f64 * p).min(1.));
        adjusted[ordinal] = previous;
    }
    Some(adjusted)
}
fn family(
    kind: AnovaFamilyKind,
    mut members: Vec<AnovaComparison>,
    c: &JobControl,
) -> AppResult<AnovaContrastFamily> {
    c.checkpoint(0, None)?;
    let adjusted = holm(&members.iter().map(|m| m.raw_p.value).collect::<Vec<_>>());
    let adjustment_failure = if let Some(values) = adjusted {
        for (i, (m, p)) in members.iter_mut().zip(values).enumerate() {
            c.checkpoint(i, None)?;
            m.adjusted_p = Estimate::available(p);
        }
        None
    } else {
        Some(AnovaFailure {
            kind: AnovaFailureKind::IncompleteFamily,
            reason: "Holm adjustment requires every raw probability in this fixed family.".into(),
        })
    };
    Ok(AnovaContrastFamily {
        m: members.len(),
        kind,
        members,
        adjustment_failure,
    })
}
/// Calibrated conditional-tail crossing plus the unchanged quantile stopping width.
fn critical_allowance(
    q: f64,
    df: f64,
    confidence: crate::analysis::ConfidenceLevel,
    c: &JobControl,
) -> AppResult<Option<f64>> {
    use crate::analysis::inference::{arithmetic::TailFloat, student_t_log_tail_precise};
    let alpha: f64 = match confidence {
        crate::analysis::ConfidenceLevel::P90 => 0.1,
        crate::analysis::ConfidenceLevel::P95 => 0.05,
        crate::analysis::ConfidenceLevel::P99 => 0.01,
    };
    let target = alpha.ln();
    let mut relative = (1e-12 / q + 32. * U).max(32. * U);
    while relative <= MAX_RELATIVE {
        c.checkpoint(0, None)?;
        let low = q * (1. - relative);
        let high = q * (1. + relative);
        let a = student_t_log_tail_precise(TailFloat(low.ln(), 0.), Some(low), df, c)?;
        let b = student_t_log_tail_precise(TailFloat(high.ln(), 0.), Some(high), df, c)?;
        if let (Ok(a), Ok(b)) = (a, b) {
            if [
                a.log_hi,
                a.log_lo,
                a.uncertainty,
                b.log_hi,
                b.log_lo,
                b.uncertainty,
            ]
            .iter()
            .all(|x| x.is_finite())
                && a.log_hi + a.log_lo - a.uncertainty >= target
                && b.log_hi + b.log_lo + b.uncertainty <= target
            {
                return Ok(Some(relative + 32. * U));
            }
        }
        relative *= 2.;
    }
    Ok(None)
}

fn publish(value: Scaled, absolute: Scaled) -> Estimate {
    let candidate = value.estimate();
    let Some(v) = candidate.value else {
        return candidate;
    };
    if value.fraction == 0. {
        return candidate;
    }
    let rounded = match at(Scaled::new(v), value.exponent) {
        Ok(v) => v,
        Err(f) => return Estimate::unavailable(&f.reason),
    };
    let error = match at(absolute, value.exponent) {
        Ok(v) => v.abs(),
        Err(f) => return Estimate::unavailable(&f.reason),
    };
    if ((rounded - value.fraction).abs() + error) / value.fraction.abs() > MAX_RELATIVE {
        Estimate::unavailable(
            "Contrast publication rounding or propagated absolute error is unresolved.",
        )
    } else {
        candidate
    }
}
fn workspace_bounds(r: &AnovaRequest) -> AppResult<()> {
    let planned = super::MAX_REQUEST_BYTES / std::mem::size_of::<AnovaPairRequest>();
    let all = 64 * 63 / 2;
    let largest = planned.max(all);
    // Raw values, sorted tuples, adjusted values and bounded selected-pair arithmetic.
    let bytes = checked_sum([
        largest * (std::mem::size_of::<Option<f64>>() + std::mem::size_of::<(usize, f64)>() + 8),
        64 * std::mem::size_of::<Scaled>(),
        2 * std::mem::size_of::<BoundedSum>(),
        8192,
        4096,
    ])?;
    if bytes > 768 * 1024 || r.planned_pairs.len() > planned {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn pair_score(
    d: &Dataset,
    columns: &[crate::data::ColumnId],
    row: usize,
    coefficients: &[i16],
    c: &JobControl,
) -> AppResult<BoundedSum> {
    let mut values = [0.; 64];
    for (j, (&id, &coefficient)) in columns.iter().zip(coefficients).enumerate() {
        c.checkpoint(j, None)?;
        if coefficient != 0 {
            let Cell::Number(y) = d.column(id)?.cell(row)? else {
                return Err(invalid_analysis());
            };
            values[j] = y;
        }
    }
    super::repeated::score(&values[..columns.len()], coefficients, c)
}

#[cfg(test)]
mod fixround1_tests;
