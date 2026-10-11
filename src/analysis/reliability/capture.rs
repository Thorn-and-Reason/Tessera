//! A single controlled source/cohort/computation/capture lifetime.
use super::*;
use crate::{
    analysis::{
        model::{checked_product, checked_sum},
        support,
    },
    data::{Budget, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::mem::size_of;
/// `extra_bytes` owns the original request exactly once, alongside caller-retained owners.
pub fn reliability_with_budget(
    d: &Dataset,
    r: &ReliabilityRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ReliabilityRun> {
    control.check_cancelled()?;
    let quiet = control.without_progress();
    let c = &quiet;
    r.admit_controlled(c)?;
    let request_owner = checked_sum([
        size_of::<ReliabilityRequest>(),
        r.heap_bytes_controlled(Some(c))?,
    ])?;
    let external = extra_bytes
        .checked_sub(request_owner)
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    let base = Budget::dataset_bytes_iter(
        std::iter::once(d).chain(retained.iter().copied()),
        extra_bytes,
    )?;
    cohort_admission(d, r, base)?;
    let cohort = capture_cohort(d, r, c)?;
    let numerical = accounting::numerical_admission(d, r, &cohort, c)?;
    let source = Budget::dataset_bytes(&[d], 0)?;
    Budget::check_datasets(
        &[],
        checked_sum([
            base - extra_bytes - source,
            external,
            source,
            numerical.bytes,
        ])?,
    )?;
    let computed = compute(d, r, &cohort, c)?;
    let counts = (
        cohort.diagnostics.clone(),
        cohort.complete_rows,
        cohort.frequency,
        cohort.incomplete_case_frequency,
    );
    drop(cohort);
    let run = capture_computed(d, r, computed, counts, base, c)?;
    c.check_cancelled()?;
    Ok(run)
}
fn cohort_admission(d: &Dataset, r: &ReliabilityRequest, base: usize) -> AppResult<()> {
    Budget::check_datasets(
        &[],
        checked_sum([
            base,
            r.validation_scratch_bytes()?,
            size_of::<ReliabilityCohort>(),
            checked_product(
                d.row_count(),
                size_of::<crate::analysis::selection::Eligible>(),
            )?,
            6640,
        ])?,
    )
}
fn computed_bytes(value: &ReliabilityComputed) -> AppResult<usize> {
    let mut n = checked_sum([
        size_of::<ReliabilityComputed>(),
        checked_product(value.items.capacity(), size_of::<ReliabilityComputedItem>())?,
    ])?;
    for item in &value.items {
        for e in [
            &item.mean,
            &item.sample_sd,
            &item.corrected_item_total,
            &item.raw_alpha_if_removed,
            &item.standardized_alpha_if_removed,
        ] {
            n = checked_sum([n, e.reason.as_ref().map_or(0, String::capacity)])?;
        }
    }
    for e in [&value.raw_alpha, &value.standardized_alpha] {
        n = checked_sum([n, e.reason.as_ref().map_or(0, String::capacity)])?;
    }
    Ok(n)
}
type Counts = (crate::analysis::RunDiagnostics, usize, u64, u64);
fn capture_computed(
    d: &Dataset,
    r: &ReliabilityRequest,
    value: ReliabilityComputed,
    counts: Counts,
    base: usize,
    c: &JobControl,
) -> AppResult<ReliabilityRun> {
    let ids = admit_capture(d, r, &value, base, c)?;
    let columns = support::capture_columns(d, &ids, c)?;
    let request = capture_request(r, c)?;
    let items = convert_items(value.items, &r.items, c)?;
    let run = ReliabilityRun {
        revision: d.revision(),
        request,
        columns,
        source_rows: d.row_count(),
        diagnostics: counts.0,
        complete_rows: counts.1,
        frequency: counts.2,
        incomplete_case_frequency: counts.3,
        items,
        raw_alpha: value.raw_alpha,
        standardized_alpha: value.standardized_alpha,
        total_energy: value.total_energy,
        standardized_total_energy: value.standardized_total_energy,
        method: ReliabilityMethod::ExactCenteredIntervalV1,
    };
    drop(ids);
    Budget::check_datasets(
        &[],
        checked_sum([
            base,
            run.owned_bytes_controlled(Some(c))?,
            run.scratch_controlled(Some(c))?,
        ])?,
    )?;
    run.validate_controlled(c)?;
    Ok(run)
}
fn admit_capture(
    d: &Dataset,
    r: &ReliabilityRequest,
    value: &ReliabilityComputed,
    base: usize,
    c: &JobControl,
) -> AppResult<std::collections::BTreeSet<crate::data::ColumnId>> {
    let output = computed_bytes(value)?;
    Budget::check_datasets(
        &[],
        checked_sum([base, output, r.validation_scratch_bytes()?])?,
    )?;
    let ids = validation::request(r, |id| Ok(d.column(id)?.kind()), c)?;
    let snapshots = support::snapshot_bytes_controlled(d, &ids, c)?;
    let scratch = capture_scratch(d, r, &ids, c)?;
    // Reasons move from the computed owner; include both result vectors before conversion.
    let copies = checked_sum([
        size_of::<ReliabilityRun>(),
        r.heap_bytes_controlled(Some(c))?,
        snapshots,
        checked_product(r.items.len(), size_of::<ReliabilityItem>())?,
    ])?;
    Budget::check_datasets(&[], checked_sum([base, output, copies, scratch])?)?;
    Ok(ids)
}

fn capture_scratch(
    d: &Dataset,
    r: &ReliabilityRequest,
    ids: &std::collections::BTreeSet<crate::data::ColumnId>,
    c: &JobControl,
) -> AppResult<usize> {
    let mut labels = 0;
    for (i, id) in ids.iter().enumerate() {
        if i.is_multiple_of(64) {
            c.checkpoint(i, None)?;
        }
        labels = labels.max(checked_product(
            d.column(*id)?.metadata().value_labels.len(),
            2 * size_of::<usize>(),
        )?);
    }
    // Original role set overlaps passive role set/map until capture returns.
    checked_sum([
        checked_product(2, r.validation_scratch_bytes()?)?,
        checked_product(ids.len(), 1024)?,
        labels,
    ])
}
fn convert_items(
    items: Vec<ReliabilityComputedItem>,
    ids: &[crate::data::ColumnId],
    c: &JobControl,
) -> AppResult<Vec<ReliabilityItem>> {
    c.check_cancelled()?;
    let mut result = Vec::with_capacity(ids.len());
    for (i, item) in items.into_iter().enumerate() {
        c.checkpoint(i, None)?;
        result.push(ReliabilityItem {
            column: ids[i],
            mean: item.mean,
            sample_sd: item.sample_sd,
            corrected_item_total: item.corrected_item_total,
            raw_alpha_if_removed: item.raw_alpha_if_removed,
            standardized_alpha_if_removed: item.standardized_alpha_if_removed,
            energy: item.energy,
            remaining_energy: item.remaining_energy,
            remaining_standardized_energy: item.remaining_standardized_energy,
        });
    }
    Ok(result)
}
