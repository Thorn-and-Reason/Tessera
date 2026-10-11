//! Full-cohort per-copy influence and integer-rank residual plots.
use super::{solver::ScaledFit, *};
use crate::{
    analysis::{
        exploration::{numeric::display_index, ordering::sort_indices},
        moments::Scaled,
        selection::Eligible,
        validation::invalid_analysis,
        Estimate, MAX_PLOT_POINTS,
    },
    data::Dataset,
    error::AppResult,
    operation::JobControl,
};
use solver::scaling::{estimate, multiply};
use statrs::distribution::{ContinuousCDF, Normal};

const NO_DF: &str = "There are no residual degrees of freedom.";
const ZERO: &str = "Residual variance is exactly zero.";
const DOMAIN: &str = "Per-observation leverage is outside the studentisation domain.";

pub(super) struct Record {
    row: usize,
    frequency: u64,
}
#[derive(Default)]
struct Sum {
    value: f64,
    correction: f64,
}
impl Sum {
    fn add(&mut self, value: f64) {
        let adjusted = value - self.correction;
        let next = self.value + adjusted;
        self.correction = (next - self.value) - adjusted;
        self.value = next;
    }
}
struct Extent {
    lo: f64,
    hi: f64,
    rows: usize,
    frequency: u64,
    reason: Option<String>,
}
impl Extent {
    fn new() -> Self {
        Self {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY,
            rows: 0,
            frequency: 0,
            reason: None,
        }
    }
    fn add(&mut self, estimate: &Estimate, frequency: u64) -> AppResult<()> {
        if let Some(value) = estimate.value {
            self.lo = self.lo.min(value);
            self.hi = self.hi.max(value);
        } else {
            self.rows = self.rows.checked_add(1).ok_or_else(invalid_analysis)?;
            self.frequency = self
                .frequency
                .checked_add(frequency)
                .ok_or_else(invalid_analysis)?;
            if self.reason.is_none() {
                self.reason = estimate.reason.clone();
            }
        }
        Ok(())
    }
    fn finish(self) -> DiagnosticExtent {
        let range = if self.lo.is_finite() {
            [Estimate::available(self.lo), Estimate::available(self.hi)]
        } else {
            let reason = self
                .reason
                .as_deref()
                .unwrap_or("No diagnostic values are available.");
            [Estimate::unavailable(reason), Estimate::unavailable(reason)]
        };
        DiagnosticExtent {
            range,
            undefined_rows: self.rows,
            undefined_frequency: self.frequency,
        }
    }
}

/// Caller precharges diagnostics_work_bytes in addition to the live solver owners.
pub(super) fn compute(
    dataset: &Dataset,
    _request: &RegressionRequest,
    eligible: &[Eligible],
    _levels: &[RegressionLevels],
    terms: &[RegressionTerm],
    fit: &ScaledFit,
    control: &JobControl,
) -> AppResult<RegressionDiagnostics> {
    let quiet = control.without_progress();
    quiet.checkpoint(0, None)?;
    let result = compute_inner(dataset, eligible, terms, fit, &quiet);
    quiet.checkpoint(0, None)?;
    result
}
fn compute_inner(
    dataset: &Dataset,
    eligible: &[Eligible],
    terms: &[RegressionTerm],
    fit: &ScaledFit,
    control: &JobControl,
) -> AppResult<RegressionDiagnostics> {
    let m = fit.residuals().len();
    let n = fit.frequency();
    let k = m.min(MAX_PLOT_POINTS);
    if m == 0 || n == 0 || fit.rank() != terms.len() {
        return Err(invalid_analysis());
    }
    let mut records = Vec::with_capacity(m);
    let mut points = Vec::with_capacity(k);
    let mut row = vec![0.; terms.len()];
    let mut mean = Sum::default();
    let (mut e_lo, mut e_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut f_lo, mut f_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut h_extent, mut t_extent, mut d_extent) = (Extent::new(), Extent::new(), Extent::new());
    for (index, source) in eligible.iter().enumerate() {
        if index.is_multiple_of(64) {
            control.checkpoint(index, None)?;
        }
        if !source.complete {
            continue;
        }
        let ordinal = records.len();
        let e = *fit.residuals().get(ordinal).ok_or_else(invalid_analysis)?;
        fit.row(dataset, terms, source.row, &mut row, control)?;
        let prediction = fit.predict(&row, control)?;
        let h = fit.leverage(&row, control)?;
        let leverage = Estimate::available(h);
        let (studentised, cooks_distance) = influence(fit, e, h);
        h_extent.add(&leverage, source.weight)?;
        t_extent.add(&studentised, source.weight)?;
        d_extent.add(&cooks_distance, source.weight)?;
        mean.add(e * (source.weight as f64 / n as f64));
        e_lo = e_lo.min(e);
        e_hi = e_hi.max(e);
        f_lo = f_lo.min(prediction);
        f_hi = f_hi.max(prediction);
        if points.len() < k && ordinal == display_index(m, k, points.len())? {
            points.push(RegressionResidualPoint {
                cohort_ordinal: ordinal,
                row: source.row,
                frequency: source.weight,
                fitted: fit.fitted_estimate(prediction),
                residual: fit.residual_estimate(e),
                leverage,
                studentised,
                cooks_distance,
            });
        }
        records.push(Record {
            row: source.row,
            frequency: source.weight,
        });
    }
    if records.len() != m || points.len() != k {
        return Err(invalid_analysis());
    }
    let mut spread = 0f64;
    for (i, record) in records.iter().enumerate() {
        if i.is_multiple_of(64) {
            control.checkpoint(i, None)?;
        }
        spread = spread
            .hypot((fit.residuals()[i] - mean.value) * (record.frequency as f64 / n as f64).sqrt());
    }
    let residual_sample_sd = if n > 1 {
        estimate(multiply(
            fit.outcome_unit(),
            Scaled::new(spread).times((n as f64 / (n - 1) as f64).sqrt()),
        ))
    } else {
        Estimate::unavailable(
            "At least two observations are needed for the sample standard deviation.",
        )
    };
    let qq_points = qq_points(&records, fit, control)?;
    let theoretical_range = [
        qq_points[0].theoretical.clone(),
        qq_points[qq_points.len() - 1].theoretical.clone(),
    ];
    Ok(RegressionDiagnostics {
        residual_mean: fit.residual_estimate(mean.value),
        residual_sample_sd,
        residual_range: [fit.residual_estimate(e_lo), fit.residual_estimate(e_hi)],
        fitted_range: [fit.fitted_estimate(f_lo), fit.fitted_estimate(f_hi)],
        leverage: h_extent.finish(),
        studentised: t_extent.finish(),
        cooks_distance: d_extent.finish(),
        residual_points: points,
        residual_reduced: m > MAX_PLOT_POINTS,
        qq_points,
        qq_reduced: n > MAX_PLOT_POINTS as u64,
        theoretical_range,
    })
}
fn influence(fit: &ScaledFit, residual: f64, h: f64) -> (Estimate, Estimate) {
    let reason = if fit.residual_df() == 0 {
        Some(NO_DF)
    } else if fit.residual_norm() == 0. {
        Some(ZERO)
    } else if h >= 1. {
        Some(DOMAIN)
    } else {
        None
    };
    if let Some(reason) = reason {
        return (Estimate::unavailable(reason), Estimate::unavailable(reason));
    }
    let t = Scaled::new(residual)
        .ratio(Scaled::new(fit.residual_norm()))
        .times((fit.residual_df() as f64 / fit.frequency() as f64).sqrt())
        .ratio(Scaled::new((1. - h).sqrt()));
    let cook = multiply(t, t)
        .times(h)
        .ratio(Scaled::new(fit.rank() as f64))
        .ratio(Scaled::new(1. - h));
    (estimate(t), estimate(cook))
}
fn qq_points(
    records: &[Record],
    fit: &ScaledFit,
    control: &JobControl,
) -> AppResult<Vec<RegressionQqPoint>> {
    let m = records.len();
    let k = fit.frequency().min(MAX_PLOT_POINTS as u64) as usize;
    let mut indices = Vec::with_capacity(m);
    for i in 0..m {
        if i.is_multiple_of(64) {
            control.checkpoint(i, None)?;
        }
        indices.push(i);
    }
    let mut scratch = vec![0; m];
    sort_indices(&mut indices, &mut scratch, control, |a, b| {
        Ok(fit.residuals()[a]
            .partial_cmp(&fit.residuals()[b])
            .ok_or_else(invalid_analysis)?
            .then_with(|| records[a].row.cmp(&records[b].row)))
    })?;
    let mut points = Vec::with_capacity(k);
    let mut cursor = 0usize;
    let mut end = u128::from(records[indices[0]].frequency);
    for i in 0..k {
        control.checkpoint(i, None)?;
        let rank = qq_rank(fit.frequency(), k, i)?;
        while u128::from(rank) >= end {
            cursor += 1;
            if cursor.is_multiple_of(64) {
                control.checkpoint(cursor, None)?;
            }
            let ordinal = *indices.get(cursor).ok_or_else(invalid_analysis)?;
            end = end
                .checked_add(u128::from(records[ordinal].frequency))
                .ok_or_else(invalid_analysis)?;
        }
        let ordinal = indices[cursor];
        points.push(RegressionQqPoint {
            rank,
            row: records[ordinal].row,
            frequency: records[ordinal].frequency,
            theoretical: Estimate::available(qq_theoretical(fit.frequency(), rank)?),
            residual: fit.residual_estimate(fit.residuals()[ordinal]),
        });
    }
    Ok(points)
}
fn qq_rank(n: u64, k: usize, i: usize) -> AppResult<u64> {
    if n == 0 || k == 0 || i >= k || k as u128 > u128::from(n) {
        return Err(invalid_analysis());
    }
    if k == 1 {
        return Ok(0);
    }
    let rank = (i as u128)
        .checked_mul(u128::from(n - 1))
        .ok_or_else(invalid_analysis)?
        / (k - 1) as u128;
    u64::try_from(rank).map_err(|_| invalid_analysis())
}
fn qq_theoretical(n: u64, rank: u64) -> AppResult<f64> {
    if n == 0 || rank >= n {
        return Err(invalid_analysis());
    }
    let lower = rank.min(n - 1 - rank);
    let numerator = u128::from(lower)
        .checked_mul(2)
        .and_then(|v| v.checked_add(1))
        .ok_or_else(invalid_analysis)?;
    let denominator = u128::from(n).checked_mul(2).ok_or_else(invalid_analysis)?;
    let z = Normal::standard().inverse_cdf(numerator as f64 / denominator as f64);
    let z = if rank > n - 1 - rank { -z } else { z };
    if z.is_finite() {
        Ok(z)
    } else {
        Err(invalid_analysis())
    }
}
#[cfg(test)]
mod tests;
