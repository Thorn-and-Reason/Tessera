//! Captured regression results and the complete-cohort direct family API.
mod accounting;
mod coding;
mod diagnostics;
mod inference;
pub(crate) use inference::f_upper_tail;
mod solver;
const MAX_TERMS: usize = 64;
mod model;
pub(crate) mod validation;
pub use model::*;
#[cfg(test)]
mod tests;

use crate::{
    analysis::{
        model::checked_sum,
        selection::{select_rows, Eligible},
        support, Estimate,
    },
    data::{AllocationCounter, Budget, ColumnId, Dataset},
    error::AppResult,
    operation::JobControl,
};
use std::mem::size_of;

/// `extra_bytes` includes the live request once and all other caller-owned work.
/// Retained datasets are deduplicated with the current dataset before allocation.
pub fn regression_with_budget(
    dataset: &Dataset,
    request: &RegressionRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedRegression> {
    let quiet = control.without_progress();
    quiet.checkpoint(0, None)?;
    let result = compute(dataset, request, &quiet, retained, extra_bytes);
    quiet.checkpoint(0, None)?;
    result
}
fn compute(
    dataset: &Dataset,
    request: &RegressionRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedRegression> {
    let baseline = Budget::dataset_bytes_iter(
        std::iter::once(dataset).chain(retained.iter().copied()),
        extra_bytes,
    )?;
    let mut live = AllocationCounter { bytes: baseline };
    live.add(request.validation_scratch_bytes()?)?;
    let ids = validation::request(request, |id| Ok(dataset.column(id)?.kind()))?;
    live.add(support::snapshot_bytes(dataset, &ids)?)?;
    live.add(size_of::<ComputedRegression>())?;
    live.add(request.heap_bytes()?)?;
    live.array::<Eligible>(dataset.row_count())?;
    live.array::<ColumnId>(checked_sum([1, request.predictors.len()])?)?;
    let columns = support::capture_columns(dataset, &ids, control)?;
    let captured_request = capture_request(request, control)?;
    let mut complete = Vec::with_capacity(request.predictors.len() + 1);
    complete.push(request.outcome);
    for predictor in &request.predictors {
        control.checkpoint(0, None)?;
        complete.push(predictor.column);
    }
    let (eligible, run_diagnostics) = select_rows(
        dataset,
        &request.filter,
        request.frequency_weight,
        &complete,
        control,
        dataset.row_count(),
    )?;
    let (mut fitted_rows, mut frequency, mut incomplete_case_frequency) = (0usize, 0u64, 0u64);
    for (i, r) in eligible.iter().enumerate() {
        if i.is_multiple_of(64) {
            control.checkpoint(i, None)?;
        }
        if r.complete {
            fitted_rows += 1;
            frequency += r.weight;
        } else {
            incomplete_case_frequency += r.weight;
        }
    }
    let (levels, terms, failure) =
        coding::discover(dataset, request, &eligible, control, live.bytes)?;
    live.add(accounting::coding_bytes(&levels, &terms)?)?;
    let outcome = match failure {
        Some(failure) => RegressionOutcome::Unavailable(failure),
        None => {
            // Kernel reservation is additional to every owner above, never the baseline.
            let mut peak = AllocationCounter { bytes: live.bytes };
            peak.add(size_of::<RegressionFit>())?;
            peak.add(accounting::kernel_work_bytes(fitted_rows, terms.len())?)?;
            peak.add(accounting::diagnostics_work_bytes(
                fitted_rows,
                frequency,
                terms.len(),
            )?)?;
            match solver::solve(dataset, request, &eligible, &levels, &terms, control)? {
                solver::KernelOutcome::Unavailable(failure) => {
                    RegressionOutcome::Unavailable(failure)
                }
                solver::KernelOutcome::Fitted(fit) => {
                    let (coefficients, statistics) =
                        inference::infer(&fit, request.confidence, control)?;
                    let residual_diagnostics = diagnostics::compute(
                        dataset, request, &eligible, &levels, &terms, &fit, control,
                    )?;
                    let mut term_scales = Vec::with_capacity(terms.len());
                    for j in 0..terms.len() {
                        control.checkpoint(j, None)?;
                        term_scales.push(solver::scaling::estimate(fit.term_unit(j)));
                    }
                    RegressionOutcome::Fitted(Box::new(RegressionFit {
                        rank: fit.rank(),
                        model_df: (fit.rank() - 1) as u64,
                        residual_df: fit.residual_df(),
                        total_df: frequency - 1,
                        condition: Estimate::available(fit.condition()),
                        condition_warned: fit.condition_warned(),
                        outcome_scale: solver::scaling::estimate(fit.outcome_unit()),
                        term_scales,
                        coefficients,
                        statistics,
                        residual_diagnostics,
                    }))
                }
            }
        }
    };
    drop(eligible);
    drop(complete);
    drop(ids);
    let result = ComputedRegression {
        revision: dataset.revision(),
        request: captured_request,
        columns,
        source_rows: dataset.row_count(),
        diagnostics: run_diagnostics,
        fitted_rows,
        frequency,
        incomplete_case_frequency,
        levels,
        terms,
        method: RegressionMethod {
            numerical_version: 1,
            rank_threshold: 1e-12,
            condition_warning: 1e8,
        },
        outcome,
    };
    // Validation allocates borrowed indexes; all captured owners and external data stay live.
    let mut final_peak = AllocationCounter { bytes: baseline };
    final_peak.add(result.owned_bytes()?)?;
    final_peak.add(result.validation_scratch_bytes()?)?;
    control.checkpoint(0, None)?;
    validation::completed(&result)?;
    control.checkpoint(0, None)?;
    Ok(result)
}
fn capture_request(
    request: &RegressionRequest,
    control: &JobControl,
) -> AppResult<RegressionRequest> {
    let mut predictors = Vec::with_capacity(request.predictors.len());
    for predictor in &request.predictors {
        control.checkpoint(0, None)?;
        let role = match &predictor.role {
            PredictorRole::Numeric => PredictorRole::Numeric,
            PredictorRole::Categorical { reference } => PredictorRole::Categorical {
                reference: support::clone_cell(reference, control)?,
            },
        };
        predictors.push(RegressionPredictor {
            column: predictor.column,
            role,
        });
    }
    Ok(RegressionRequest {
        outcome: request.outcome,
        predictors,
        confidence: request.confidence,
        filter: support::clone_filter(&request.filter, control)?,
        frequency_weight: request.frequency_weight,
    })
}
