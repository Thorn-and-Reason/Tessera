//! Full complete-pair ranges with one deterministic, row-aligned display selection.
use super::{
    exploration::numeric::display_index,
    model::{checked_product, checked_sum},
    selection::{self, Eligible},
    support,
    validation::invalid_analysis,
    ComputedScatter, Estimate, ScatterPoint, ScatterRequest, MAX_PLOT_POINTS,
};
use crate::{
    data::{Budget, Cell, Column, Dataset},
    error::AppResult,
    operation::JobControl,
};
use std::mem::size_of;
const EMPTY: &str = "No eligible complete pairs.";

/// `extra_bytes` charges the live request and other caller-retained work once.
pub fn scatter_with_budget(
    dataset: &Dataset,
    request: &ScatterRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedScatter> {
    control.check_cancelled()?;
    let baseline = Budget::dataset_bytes_iter(
        std::iter::once(dataset).chain(retained.iter().copied()),
        checked_sum([extra_bytes, request.validation_scratch_bytes()?])?,
    )?;
    let ids = request.validate_current(dataset, retained, extra_bytes)?;
    Budget::check_datasets(
        &[],
        checked_sum([
            baseline,
            size_of::<ComputedScatter>(),
            request.heap_bytes()?,
            support::snapshot_bytes(dataset, &ids)?,
            checked_product(dataset.row_count(), size_of::<Eligible>())?,
            checked_product(
                dataset.row_count().min(MAX_PLOT_POINTS),
                size_of::<ScatterPoint>(),
            )?,
            4 * EMPTY.len(),
        ])?,
    )?;
    let rows = dataset.row_count();
    let total = rows.checked_mul(3).ok_or_else(invalid_analysis)?;
    let (eligible, diagnostics) = selection::select_rows(
        dataset,
        &request.filter,
        request.frequency_weight,
        &[],
        control,
        total,
    )?;
    let x = dataset.column(request.x)?;
    let y = dataset.column(request.y)?;
    let (mut valid_rows, mut n, mut missing_rows, mut missing_n) = (0, 0, 0, 0);
    let mut bounds = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(rows + index, Some(total))?;
        }
        let Some(pair) = pair(x, y, entry.row)? else {
            missing_rows += 1;
            missing_n += entry.weight;
            continue;
        };
        valid_rows += 1;
        n += entry.weight;
        for (range, value) in bounds.iter_mut().zip(pair) {
            if value < range[0] {
                range[0] = value;
            }
            if value > range[1] {
                range[1] = value;
            }
        }
    }
    let shown = valid_rows.min(MAX_PLOT_POINTS);
    let mut points = Vec::with_capacity(shown);
    let mut ordinal = 0;
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(2 * rows + index, Some(total))?;
        }
        let Some([px, py]) = pair(x, y, entry.row)? else {
            continue;
        };
        if points.len() < shown && ordinal == display_index(valid_rows, shown, points.len())? {
            points.push(ScatterPoint {
                row: entry.row,
                x: px,
                y: py,
                frequency: entry.weight,
            });
        }
        ordinal += 1;
    }
    let ranges = bounds.map(|[lo, hi]| {
        if valid_rows == 0 {
            [Estimate::unavailable(EMPTY), Estimate::unavailable(EMPTY)]
        } else {
            [Estimate::available(lo), Estimate::available(hi)]
        }
    });
    let columns = support::capture_columns(dataset, &ids, control)?;
    let request = ScatterRequest {
        x: request.x,
        y: request.y,
        filter: support::clone_filter(&request.filter, control)?,
        frequency_weight: request.frequency_weight,
    };
    control.checkpoint(total, Some(total))?;
    Ok(ComputedScatter {
        revision: dataset.revision(),
        request,
        columns,
        source_rows: rows,
        diagnostics,
        valid_rows,
        n,
        missing_rows,
        missing_n,
        ranges,
        points,
        reduced: valid_rows > MAX_PLOT_POINTS,
    })
}
fn pair(x: &Column, y: &Column, row: usize) -> AppResult<Option<[f64; 2]>> {
    if x.is_missing(row)? || y.is_missing(row)? {
        return Ok(None);
    }
    match (x.cell(row)?, y.cell(row)?) {
        (Cell::Number(x), Cell::Number(y)) => Ok(Some([x, y])),
        _ => Err(invalid_analysis()),
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::{data::ColumnId, error::AppError, operation::CHECKPOINT_HOOK};
    #[test]
    fn cancellation_during_summary_and_paired_selection_preserves_source() {
        for done in [9096, 14096] {
            let dataset = Dataset::new(vec![
                Column::numeric(
                    ColumnId(1),
                    "X",
                    (0..5000).map(|v| Some(v as f64)).collect(),
                )
                .unwrap(),
                Column::numeric(ColumnId(2), "Y", vec![Some(3.); 5000]).unwrap(),
            ])
            .unwrap();
            let request = ScatterRequest {
                x: ColumnId(1),
                y: ColumnId(2),
                filter: super::super::RowFilter::default(),
                frequency_weight: None,
            };
            let control = JobControl::default();
            let cancel = control.clone();
            CHECKPOINT_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |progress| {
                    if progress.done == done {
                        cancel.cancel();
                    }
                }))
            });
            let result = scatter_with_budget(&dataset, &request, &control, &[], 0);
            CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(matches!(result, Err(AppError::Cancelled)));
            assert_eq!(
                dataset.cell(ColumnId(1), 4999).unwrap(),
                Cell::Number(4999.)
            );
        }
    }
}
