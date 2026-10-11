//! Complete independent tables over the shared filter/weight cohort.
mod search;
use super::{
    exploration::{
        labels::LabelIndex,
        ordering::{compare_text, sort_indices},
    },
    model::{checked_product, checked_sum},
    selection::{self, Eligible},
    support,
    validation::invalid_analysis,
    ComputedFrequencies, Estimate, FrequenciesRequest, FrequencyCategory, FrequencyTable,
    RunColumnInfo,
};
use crate::{
    data::{Budget, Cell, Column, ColumnKind, Dataset},
    error::AppResult,
    operation::JobControl,
};
pub use search::{frequency_rows_with_budget, FrequencySearchRequest, FrequencySearchResult};
use std::{cmp::Ordering, mem::size_of};
const NO_DENOMINATOR: &str = "No eligible frequency.";

/// `extra_bytes` charges the live request and other caller-retained work once.
pub fn frequencies_with_budget(
    dataset: &Dataset,
    request: &FrequenciesRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedFrequencies> {
    control.check_cancelled()?;
    let validation = request.validation_scratch_bytes()?;
    let baseline = Budget::dataset_bytes_iter(
        std::iter::once(dataset).chain(retained.iter().copied()),
        checked_sum([extra_bytes, validation])?,
    )?;
    let ids = request.validate_current(dataset, retained, extra_bytes)?;
    let mut label_work = 0;
    for id in &request.columns {
        label_work = label_work.max(LabelIndex::scratch_bytes(
            dataset.column(*id)?.metadata().value_labels.len(),
        )?);
    }
    // Both merge arrays and cohort remain live while each table is built.
    let scratch = checked_sum([
        checked_product(dataset.row_count(), size_of::<Eligible>())?,
        checked_product(dataset.row_count(), 2 * size_of::<usize>())?,
        label_work,
    ])?;
    let mut owned = checked_sum([
        size_of::<ComputedFrequencies>(),
        request.heap_bytes()?,
        support::snapshot_bytes(dataset, &ids)?,
        checked_product(
            request.columns.len(),
            size_of::<FrequencyTable>() + NO_DENOMINATOR.len(),
        )?,
    ])?;
    check_work(baseline, scratch, owned)?;
    let rows = dataset.row_count();
    let passes = request
        .columns
        .len()
        .checked_add(1)
        .ok_or_else(invalid_analysis)?;
    let total = rows.checked_mul(passes).ok_or_else(invalid_analysis)?;
    let (eligible, diagnostics) = selection::select_rows(
        dataset,
        &request.filter,
        request.frequency_weight,
        &[],
        control,
        total,
    )?;
    let columns = support::capture_columns(dataset, &ids, control)?;
    let mut tables = Vec::with_capacity(request.columns.len());
    let quiet = control.without_progress();
    for (index, id) in request.columns.iter().enumerate() {
        control.checkpoint(rows * (index + 1), Some(total))?;
        let info = columns
            .iter()
            .find(|info| info.id == *id)
            .ok_or_else(invalid_analysis)?;
        let table = table(dataset.column(*id)?, info, &eligible, &quiet, |growth| {
            owned = checked_sum([owned, growth])?;
            check_work(baseline, scratch, owned)
        })?;
        tables.push(table);
        control.checkpoint(rows * (index + 2), Some(total))?;
    }
    let request = clone_request(request, control)?;
    control.checkpoint(total, Some(total))?;
    Ok(ComputedFrequencies {
        revision: dataset.revision(),
        request,
        columns,
        source_rows: rows,
        diagnostics,
        tables,
    })
}
fn check_work(baseline: usize, scratch: usize, owned: usize) -> AppResult<()> {
    Budget::check_datasets(&[], checked_sum([baseline, scratch, owned])?)
}
fn compare_rows(column: &Column, a: usize, b: usize, control: &JobControl) -> AppResult<Ordering> {
    match column.kind() {
        ColumnKind::Text => compare_text(
            column.text_value(a)?.ok_or_else(invalid_analysis)?,
            column.text_value(b)?.ok_or_else(invalid_analysis)?,
            control,
        ),
        ColumnKind::Number => match (column.cell(a)?, column.cell(b)?) {
            (Cell::Number(a), Cell::Number(b)) => a.partial_cmp(&b).ok_or_else(invalid_analysis),
            _ => Err(invalid_analysis()),
        },
    }
}
fn table(
    column: &Column,
    info: &RunColumnInfo,
    eligible: &[Eligible],
    control: &JobControl,
    mut charge: impl FnMut(usize) -> AppResult<()>,
) -> AppResult<FrequencyTable> {
    let (mut missing_rows, mut missing_n, mut valid_n) = (0, 0, 0);
    let mut indices = Vec::with_capacity(column.len());
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(index, Some(eligible.len()))?;
        }
        if column.is_missing(entry.row)? {
            missing_rows += 1;
            missing_n += entry.weight;
        } else {
            indices.push(index);
            valid_n += entry.weight;
        }
    }
    let mut scratch = vec![0; indices.len()];
    sort_indices(&mut indices, &mut scratch, control, |a, b| {
        compare_rows(column, eligible[a].row, eligible[b].row, control)
    })?;
    let mut category_count = 0;
    let mut growth = 0;
    for (position, &index) in indices.iter().enumerate() {
        if position.is_multiple_of(4096) {
            control.checkpoint(position, Some(indices.len()))?;
        }
        if position == 0
            || compare_rows(
                column,
                eligible[indices[position - 1]].row,
                eligible[index].row,
                control,
            )? != Ordering::Equal
        {
            category_count += 1;
            growth = checked_sum([
                growth,
                size_of::<FrequencyCategory>(),
                match column.kind() {
                    ColumnKind::Text => column
                        .text_value(eligible[index].row)?
                        .ok_or_else(invalid_analysis)?
                        .len(),
                    ColumnKind::Number => 0,
                },
            ])?;
        }
    }
    // Refuse before reserving output or copying any category text.
    charge(growth)?;
    let labels = LabelIndex::new(info, control)?;
    let mut categories = Vec::with_capacity(category_count);
    let mut position = 0;
    let mut cumulative = 0;
    let all = valid_n + missing_n;
    while position < indices.len() {
        control.checkpoint(position, Some(indices.len()))?;
        let first = position;
        let row = eligible[indices[first]].row;
        let mut frequency = 0;
        while position < indices.len()
            && (position == first
                || compare_rows(column, row, eligible[indices[position]].row, control)?
                    == Ordering::Equal)
        {
            if position.is_multiple_of(4096) {
                control.check_cancelled()?;
            }
            frequency += eligible[indices[position]].weight;
            position += 1;
        }
        let value = match column.kind() {
            ColumnKind::Text => Cell::Text(support::clone_text(
                column.text_value(row)?.ok_or_else(invalid_analysis)?,
                control,
            )?),
            ColumnKind::Number => match column.cell(row)? {
                Cell::Number(v) => Cell::Number(if v == 0.0 { 0.0 } else { v }),
                _ => return Err(invalid_analysis()),
            },
        };
        cumulative += frequency;
        let numeric = column.kind() == ColumnKind::Number;
        categories.push(FrequencyCategory {
            value_label_index: labels.find(&value, control)?,
            value,
            rows: position - first,
            frequency,
            percent: percent(frequency, all),
            valid_percent: percent(frequency, valid_n),
            cumulative_frequency: numeric.then_some(cumulative),
            cumulative_percent: numeric.then(|| percent(cumulative, valid_n)),
        });
    }
    Ok(FrequencyTable {
        column: column.id(),
        valid_rows: indices.len(),
        missing_rows,
        valid_n,
        missing_n,
        missing_percent: percent(missing_n, all),
        categories,
    })
}
fn percent(n: u64, denominator: u64) -> Estimate {
    if denominator == 0 {
        Estimate::unavailable(NO_DENOMINATOR)
    } else {
        Estimate::available(100.0 * n as f64 / denominator as f64)
    }
}
fn clone_request(
    request: &FrequenciesRequest,
    control: &JobControl,
) -> AppResult<FrequenciesRequest> {
    let mut columns = Vec::with_capacity(request.columns.len());
    for chunk in request.columns.chunks(4096) {
        control.check_cancelled()?;
        columns.extend_from_slice(chunk);
    }
    Ok(FrequenciesRequest {
        columns,
        filter: support::clone_filter(&request.filter, control)?,
        frequency_weight: request.frequency_weight,
    })
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::{
        data::{ColumnId, ColumnMetadata, ValueLabel},
        error::AppError,
        operation::CHECKPOINT_HOOK,
    };
    #[test]
    fn huge_captured_label_cancellation_preserves_source() {
        let column = Column::numeric(ColumnId(1), "N", vec![Some(1.)]).unwrap();
        let column = column
            .with_metadata(ColumnMetadata {
                value_labels: vec![ValueLabel {
                    value: Cell::Number(1.),
                    label: "ŵ".repeat(100_000),
                }],
                ..column.metadata().clone()
            })
            .unwrap();
        let dataset = Dataset::new(vec![column]).unwrap();
        let request = FrequenciesRequest {
            columns: vec![ColumnId(1)],
            filter: super::super::RowFilter::default(),
            frequency_weight: None,
        };
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done >= 8192 {
                    cancel.cancel();
                }
            }))
        });
        let result = frequencies_with_budget(
            &dataset,
            &request,
            &control,
            &[],
            request.heap_bytes().unwrap(),
        );
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(
            dataset.column(ColumnId(1)).unwrap().metadata().value_labels[0]
                .label
                .len(),
            200_000
        );
    }
    #[test]
    fn long_borrowed_category_sort_cancels_before_any_output() {
        let column = Column::text(
            ColumnId(1),
            "T",
            vec![
                Some("a".repeat(100_000) + "c"),
                Some("a".repeat(100_000) + "b"),
            ],
        )
        .unwrap();
        let dataset = Dataset::new(vec![column]).unwrap();
        let request = FrequenciesRequest {
            columns: vec![ColumnId(1)],
            filter: super::super::RowFilter::default(),
            frequency_weight: None,
        };
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done >= 8192 {
                    cancel.cancel();
                }
            }))
        });
        let result = frequencies_with_budget(
            &dataset,
            &request,
            &control,
            &[],
            request.heap_bytes().unwrap(),
        );
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
    }
}
