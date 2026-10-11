//! Capacity-aware storage and precharged borrowed-index validation.
use super::{validation, *};
use crate::{
    analysis::{
        model::{checked_product, checked_sum},
        RowPredicate,
    },
    data::{Budget, Cell, Dataset, MissingRules},
    error::AppResult,
};
use std::mem::size_of;

pub(super) fn cell_bytes(cell: &Cell) -> usize {
    if let Cell::Text(text) = cell {
        text.capacity()
    } else {
        0
    }
}
fn reason(estimate: &crate::analysis::Estimate) -> usize {
    estimate.reason.as_ref().map_or(0, String::capacity)
}
/// Retained coding owners after discovery scratch has been released.
pub(super) fn coding_bytes(
    levels: &Vec<RegressionLevels>,
    terms: &Vec<RegressionTerm>,
) -> AppResult<usize> {
    let mut bytes = checked_sum([
        checked_product(levels.capacity(), size_of::<RegressionLevels>())?,
        checked_product(terms.capacity(), size_of::<RegressionTerm>())?,
    ])?;
    for levels in levels {
        bytes = checked_sum([
            bytes,
            checked_product(levels.values.capacity(), size_of::<Cell>())?,
        ])?;
        for cell in &levels.values {
            bytes = checked_sum([bytes, cell_bytes(cell)])?;
        }
    }
    for term in terms {
        if let RegressionTerm::Contrast {
            level, reference, ..
        } = term
        {
            bytes = checked_sum([bytes, cell_bytes(level), cell_bytes(reference)])?;
        }
    }
    Ok(bytes)
}
impl RegressionRequest {
    /// Caller-owned live request storage belongs in `extra_bytes`, once.
    pub fn heap_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            checked_product(self.predictors.capacity(), size_of::<RegressionPredictor>())?,
            checked_product(self.filter.predicates.capacity(), size_of::<RowPredicate>())?,
        ])?;
        for p in &self.predictors {
            if let PredictorRole::Categorical { reference } = &p.role {
                bytes = checked_sum([bytes, cell_bytes(reference)])?;
            }
        }
        for p in &self.filter.predicates {
            if let RowPredicate::Text { value, .. } = p {
                bytes = checked_sum([bytes, value.capacity()])?;
            }
        }
        Ok(bytes)
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        checked_product(
            checked_sum([
                1,
                self.predictors.len(),
                self.filter.predicates.len(),
                usize::from(self.frequency_weight.is_some()),
            ])?,
            1024,
        )
    }
    pub fn validate_current(
        &self,
        dataset: &Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<()> {
        Budget::dataset_bytes_iter(
            std::iter::once(dataset).chain(retained.iter().copied()),
            checked_sum([extra_bytes, self.validation_scratch_bytes()?])?,
        )?;
        validation::request(self, |id| Ok(dataset.column(id)?.kind())).map(|_| ())
    }
}
impl ComputedRegression {
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            self.request.validation_scratch_bytes()?,
            checked_product(self.columns.len(), 1024)?,
            checked_product(self.levels.len(), 1024)?,
            checked_product(self.terms.len(), 1024)?,
        ])?;
        for c in &self.columns {
            let missing = match &c.missing_rules {
                MissingRules::None => 0,
                MissingRules::Number { values, ranges } => {
                    checked_sum([values.len(), ranges.len()])?
                }
                MissingRules::Text(values) => values.len(),
            };
            bytes = checked_sum([
                bytes,
                checked_product(checked_sum([missing, c.value_labels.len()])?, 1024)?,
            ])?;
        }
        if let RegressionOutcome::Fitted(fit) = &self.outcome {
            bytes = checked_sum([
                bytes,
                checked_product(
                    checked_sum([
                        fit.residual_diagnostics.residual_points.len(),
                        fit.residual_diagnostics.qq_points.len(),
                    ])?,
                    1024,
                )?,
            ])?;
        }
        Ok(bytes)
    }
    pub fn validate(&self) -> AppResult<()> {
        Budget::check_datasets(
            &[],
            checked_sum([self.owned_bytes()?, self.validation_scratch_bytes()?])?,
        )?;
        validation::completed(self)
    }
    pub(crate) fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            size_of::<Self>(),
            self.request.heap_bytes()?,
            checked_product(
                self.columns.capacity(),
                size_of::<crate::analysis::RunColumnInfo>(),
            )?,
            coding_bytes(&self.levels, &self.terms)?,
        ])?;
        for c in &self.columns {
            bytes = checked_sum([bytes, c.heap_bytes()?])?;
        }
        match &self.outcome {
            RegressionOutcome::Unavailable(failure) => checked_sum([
                bytes,
                failure.reason.capacity(),
                checked_product(failure.implicated_terms.capacity(), size_of::<usize>())?,
            ]),
            RegressionOutcome::Fitted(fit) => {
                checked_sum([bytes, size_of::<RegressionFit>(), fit.heap_bytes()?])
            }
        }
    }
}
impl RegressionFit {
    fn heap_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            reason(&self.condition),
            reason(&self.outcome_scale),
            checked_product(
                self.term_scales.capacity(),
                size_of::<crate::analysis::Estimate>(),
            )?,
            checked_product(
                self.coefficients.capacity(),
                size_of::<RegressionCoefficient>(),
            )?,
            self.residual_diagnostics.heap_bytes()?,
        ])?;
        for e in &self.term_scales {
            bytes = checked_sum([bytes, reason(e)])?;
        }
        for c in &self.coefficients {
            for e in [
                &c.estimate,
                &c.standard_error,
                &c.t,
                &c.p,
                &c.interval[0],
                &c.interval[1],
            ] {
                bytes = checked_sum([bytes, reason(e)])?;
            }
        }
        let s = &self.statistics;
        for e in [
            &s.sse,
            &s.sst,
            &s.model_ss,
            &s.residual_standard_error,
            &s.r_squared,
            &s.adjusted_r_squared,
            &s.f,
            &s.f_p,
        ] {
            bytes = checked_sum([bytes, reason(e)])?;
        }
        Ok(bytes)
    }
}
impl RegressionDiagnostics {
    fn heap_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            checked_product(
                self.residual_points.capacity(),
                size_of::<RegressionResidualPoint>(),
            )?,
            checked_product(self.qq_points.capacity(), size_of::<RegressionQqPoint>())?,
        ])?;
        for e in [&self.residual_mean, &self.residual_sample_sd]
            .into_iter()
            .chain(self.residual_range.iter())
            .chain(self.fitted_range.iter())
            .chain(self.theoretical_range.iter())
            .chain(self.leverage.range.iter())
            .chain(self.studentised.range.iter())
            .chain(self.cooks_distance.range.iter())
        {
            bytes = checked_sum([bytes, reason(e)])?;
        }
        for p in &self.residual_points {
            for e in [
                &p.fitted,
                &p.residual,
                &p.leverage,
                &p.studentised,
                &p.cooks_distance,
            ] {
                bytes = checked_sum([bytes, reason(e)])?;
            }
        }
        for p in &self.qq_points {
            bytes = checked_sum([bytes, reason(&p.theoretical), reason(&p.residual)])?;
        }
        Ok(bytes)
    }
}

/// Task 4 phase reservation; caller adds the complete deduplicated live baseline.
/// Includes the single shared Student/F workspace and all inference result owners.
pub(super) fn kernel_work_bytes(rows: usize, terms: usize) -> AppResult<usize> {
    use super::solver::{scaling::Scale, ScaledFit};
    use crate::analysis::{inference::INFERENCE_WORK_BYTES, moments::Scaled};
    if terms > MAX_TERMS {
        return Err(super::coding::oversized_model());
    }
    let square = checked_product(terms, terms)?;
    // Dependency peak: consumed R/U/VT plus six p-vectors; retained R is separate.
    let decomposition = checked_product(
        8,
        checked_sum([checked_product(4, square)?, checked_product(8, terms)?])?,
    )?;
    // Validation peak: retained R, VT, B, correlations; s/rhs/gamma/row/norms,
    // independent check row and compensated stationarity sums (two p-vectors).
    let validation = checked_product(
        8,
        checked_sum([
            checked_product(4, square)?,
            checked_product(8, terms)?,
            checked_product(2, rows)?,
        ])?,
    )?;
    // Original transforms: retained B plus scaled B, coefficients and row norms.
    let transformed = checked_sum([
        checked_product(square, 8 + size_of::<Scaled>())?,
        checked_product(terms, 8 + 2 * size_of::<Scaled>())?,
        checked_product(rows, 8)?,
    ])?;
    let result = checked_sum([
        checked_product(terms, size_of::<RegressionCoefficient>() + 6 * 160)?,
        size_of::<RegressionStatistics>() + 8 * 160,
        // One shared critical Estimate remains live while coefficients are built.
        size_of::<crate::analysis::Estimate>() + 160,
        INFERENCE_WORK_BYTES,
    ])?;
    checked_sum([
        decomposition
            .max(validation)
            .max(checked_sum([transformed, result])?),
        checked_product(terms, size_of::<Scale>())?,
        size_of::<ScaledFit>(),
        1024,
    ])
}

/// Full diagnostic phase, while ScaledFit and coefficient/statistic results remain live.
pub(super) fn diagnostics_work_bytes(
    rows: usize,
    frequency: u64,
    terms: usize,
) -> AppResult<usize> {
    let marks = rows.min(crate::analysis::MAX_PLOT_POINTS);
    let qq = frequency.min(crate::analysis::MAX_PLOT_POINTS as u64) as usize;
    checked_sum([
        checked_product(rows, size_of::<super::diagnostics::Record>())?,
        checked_product(rows, 2 * size_of::<usize>())?,
        checked_product(terms, size_of::<f64>())?,
        checked_product(marks, size_of::<RegressionResidualPoint>() + 5 * 160)?,
        checked_product(qq, size_of::<RegressionQqPoint>() + 2 * 160)?,
        checked_product(terms, size_of::<crate::analysis::Estimate>() + 160)?,
        // Twenty result estimates; three first-reason copies and bounded transient estimates.
        size_of::<RegressionDiagnostics>() + 20 * 160 + 3 * 160 + 2048,
    ])
}
