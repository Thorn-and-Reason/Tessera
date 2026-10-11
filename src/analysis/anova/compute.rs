//! One captured bridge over the accepted private numerical engines.
use super::*;
use crate::{
    analysis::{model::checked_sum, support, CompletedAnalysis, Estimate},
    data::{Budget, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::mem::size_of;

/// `extra_bytes` includes the borrowed request's inline and heap owner exactly once.
/// Central request/job envelopes remain in extra after that family owner is removed.
pub fn anova_with_budget(
    dataset: &Dataset,
    request: &AnovaRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedAnova> {
    #[cfg(test)]
    ENTRIES.with(|n| n.set(n.get() + 1));
    let quiet = control.without_progress();
    let c = &quiet;
    c.checkpoint(0, None)?;
    request.admit()?;
    let request_owner = checked_sum([size_of::<AnovaRequest>(), request.heap_bytes()?])?;
    let external = extra_bytes
        .checked_sub(request_owner)
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    let baseline = Budget::dataset_bytes_iter(
        std::iter::once(dataset).chain(retained.iter().copied()),
        extra_bytes,
    )?;
    let dataset_bytes = Budget::dataset_bytes(&[dataset], 0)?;
    // Deduplicated retained data excludes the current dataset charged by each private engine.
    let retained_bytes = baseline - extra_bytes - dataset_bytes;
    Budget::check_datasets(
        &[],
        checked_sum([baseline, request.validation_scratch_bytes()?])?,
    )?;
    let ids = validation::request(request, |id| Ok(dataset.column(id)?.kind()), c)?;
    let snapshots = support::snapshot_bytes(dataset, &ids)?;
    let envelope = size_of::<CompletedAnalysis>() - size_of::<ComputedAnova>();
    let captures = checked_sum([
        size_of::<ComputedAnova>(),
        envelope,
        request.heap_bytes()?,
        snapshots,
    ])?;
    Budget::check_datasets(
        &[],
        checked_sum([baseline, captures, request.validation_scratch_bytes()?])?,
    )?;
    let columns = support::capture_columns(dataset, &ids, c)?;
    let captured_request = capture_request(request, c)?;
    drop(ids);
    let cohort = cohort::select(
        dataset,
        request,
        c,
        checked_sum([retained_bytes, extra_bytes, captures])?,
    )?;
    let engine_extra = checked_sum([retained_bytes, external, captures])?;
    let (outcome, means, numerical) = match &request.design {
        AnovaDesign::Repeated { .. } => {
            let result = repeated::compute(dataset, request, &cohort, c, engine_extra)?;
            (result.outcome, result.cell_means, None)
        }
        _ => {
            let result = independent::compute(dataset, request, &cohort, c, engine_extra)?;
            (result.outcome, result.cell_means, result.numerical)
        }
    };
    let means_bytes = checked_sum(
        std::iter::once(accounting::array::<Estimate>(means.capacity())?).chain(
            means
                .iter()
                .map(|v| v.reason.as_ref().map_or(0, String::capacity)),
        ),
    )?;
    let input = if matches!(request.design, AnovaDesign::Repeated { .. }) {
        contrasts::Input::Repeated
    } else {
        contrasts::Input::Independent(numerical.as_ref())
    };
    let families = contrasts::compute(
        dataset,
        request,
        &cohort,
        &outcome,
        input,
        c,
        checked_sum([engine_extra, means_bytes])?,
    )?;
    drop(numerical);
    if means.len() != cohort.cells.len() {
        return Err(crate::analysis::validation::invalid_analysis());
    }
    // New cell storage overlaps the old count/mean vectors until their consuming zip completes.
    let family_bytes = accounting::family_bytes(request, &cohort.factors, c)?;
    Budget::check_datasets(
        &[],
        checked_sum([
            baseline,
            captures,
            cohort.owned_bytes()?,
            means_bytes,
            family_bytes,
            accounting::outcome_bytes(&outcome)?,
            accounting::array::<AnovaCell>(means.len())?,
        ])?,
    )?;
    let fitted_rows = cohort.rows.len();
    let complete = !cohort.cells.is_empty() && cohort.cells.iter().all(|v| v.frequency > 0);
    let cells = cohort
        .cells
        .into_iter()
        .zip(means)
        .map(|(count, mean)| AnovaCell {
            physical_rows: count.physical_rows,
            frequency: count.frequency,
            mean,
        })
        .collect();
    let result = ComputedAnova {
        revision: dataset.revision(),
        request: captured_request,
        columns,
        source_rows: dataset.row_count(),
        diagnostics: cohort.diagnostics,
        fitted_rows,
        frequency: cohort.frequency,
        incomplete_case_frequency: cohort.incomplete_case_frequency,
        observed_measurements: cohort.observed_measurements,
        grid: AnovaGrid {
            factors: cohort.factors,
            cells,
            complete,
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome,
        families,
    };
    drop(cohort.rows);
    drop(cohort.failure);
    Budget::check_datasets(
        &[],
        checked_sum([
            baseline,
            envelope,
            result.owned_bytes()?,
            result.validation_scratch_bytes()?,
        ])?,
    )?;
    result.validate(c)?;
    c.check_cancelled()?;
    Ok(result)
}
#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! { pub(crate) static ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
