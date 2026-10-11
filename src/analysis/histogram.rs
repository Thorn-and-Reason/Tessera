//! Complete-cohort histogram counts on stored, finite Float64 boundaries.
use super::{
    exploration::numeric::{interpolate, sturges_bins},
    model::{checked_product, checked_sum},
    selection::{self, Eligible},
    support,
    validation::invalid_analysis,
    ComputedHistogram, Estimate, HistogramBin, HistogramBins, HistogramRequest,
};
use crate::{
    data::{Budget, Cell, Dataset},
    error::AppResult,
    operation::JobControl,
};
use std::mem::size_of;
const EMPTY: &str = "No eligible observations.";
const MERGED: &str =
    "Coincident Float64 boundaries were merged; the actual bin count is smaller than requested.";

/// `extra_bytes` charges the live request and other caller-retained work once.
pub fn histogram_with_budget(
    dataset: &Dataset,
    request: &HistogramRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedHistogram> {
    control.check_cancelled()?;
    let baseline = Budget::dataset_bytes_iter(
        std::iter::once(dataset).chain(retained.iter().copied()),
        checked_sum([extra_bytes, request.validation_scratch_bytes()?])?,
    )?;
    let ids = request.validate_current(dataset, retained, extra_bytes)?;
    let max_bins = match request.bins {
        HistogramBins::Count(n) => usize::from(n),
        HistogramBins::Automatic => sturges_bins(selection::MAX_FREQUENCY)?,
    };
    // The request copy, frozen definitions, selector capacity and all bin storage coexist.
    Budget::check_datasets(
        &[],
        checked_sum([
            baseline,
            size_of::<ComputedHistogram>(),
            request.heap_bytes()?,
            support::snapshot_bytes(dataset, &ids)?,
            checked_product(dataset.row_count(), size_of::<Eligible>())?,
            checked_product(max_bins, size_of::<HistogramBin>())?,
            2 * EMPTY.len(),
            MERGED.len(),
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
    let column = dataset.column(request.variable)?;
    let (mut valid_rows, mut n, mut missing_rows, mut missing_n) = (0, 0, 0, 0);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(rows + index, Some(total))?;
        }
        if column.is_missing(entry.row)? {
            missing_rows += 1;
            missing_n += entry.weight;
            continue;
        }
        let Cell::Number(value) = column.cell(entry.row)? else {
            return Err(invalid_analysis());
        };
        valid_rows += 1;
        n += entry.weight;
        // Strict comparisons keep the first observed signed zero for a constant sample.
        if value < lo {
            lo = value;
        }
        if value > hi {
            hi = value;
        }
    }
    let target = match request.bins {
        HistogramBins::Count(n) => usize::from(n),
        HistogramBins::Automatic => sturges_bins(n)?,
    };
    let mut bins = Vec::with_capacity(if valid_rows == 0 { 0 } else { target });
    if valid_rows > 0 {
        if lo == hi {
            bins.push(HistogramBin {
                left: lo,
                right: hi,
                rows: valid_rows,
                frequency: n,
            });
        } else {
            let mut left = lo;
            for edge in 1..=target {
                control.checkpoint(2 * rows, Some(total))?;
                let right = interpolate(lo, hi, edge as u64, target as u64)?;
                if right > left {
                    bins.push(HistogramBin {
                        left,
                        right,
                        rows: 0,
                        frequency: 0,
                    });
                    left = right;
                }
            }
            // Direct comparisons against the retained edges avoid index-rounding ambiguity.
            for (index, entry) in eligible.iter().enumerate() {
                if index.is_multiple_of(4096) {
                    control.checkpoint(2 * rows + index, Some(total))?;
                }
                if column.is_missing(entry.row)? {
                    continue;
                }
                let Cell::Number(value) = column.cell(entry.row)? else {
                    return Err(invalid_analysis());
                };
                let index = bins
                    .partition_point(|bin| bin.right <= value)
                    .min(bins.len() - 1);
                bins[index].rows += 1;
                bins[index].frequency += entry.weight;
            }
        }
    }
    let bin_adjustment = if valid_rows > 0 && lo < hi && bins.len() < target {
        Some(support::clone_text(MERGED, control)?)
    } else {
        None
    };
    let range = if valid_rows == 0 {
        [Estimate::unavailable(EMPTY), Estimate::unavailable(EMPTY)]
    } else {
        [Estimate::available(lo), Estimate::available(hi)]
    };
    let columns = support::capture_columns(dataset, &ids, control)?;
    let request = HistogramRequest {
        variable: request.variable,
        bins: request.bins,
        filter: support::clone_filter(&request.filter, control)?,
        frequency_weight: request.frequency_weight,
    };
    control.checkpoint(total, Some(total))?;
    Ok(ComputedHistogram {
        revision: dataset.revision(),
        request,
        columns,
        source_rows: rows,
        diagnostics,
        valid_rows,
        n,
        missing_rows,
        missing_n,
        range,
        bins,
        bin_adjustment,
    })
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::{
        data::{Column, ColumnId},
        error::AppError,
        operation::CHECKPOINT_HOOK,
    };
    #[test]
    fn cancellation_during_edges_and_counting_preserves_source() {
        for cancel_edge in [true, false] {
            let dataset = Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "Value",
                (0..5000).map(|v| Some(v as f64)).collect(),
            )
            .unwrap()])
            .unwrap();
            let request = HistogramRequest {
                variable: ColumnId(1),
                bins: HistogramBins::Count(128),
                filter: super::super::RowFilter::default(),
                frequency_weight: None,
            };
            let control = JobControl::default();
            let cancel = control.clone();
            let visits = std::cell::Cell::new(0);
            CHECKPOINT_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |progress| {
                    if progress.done == 10_000 {
                        visits.set(visits.get() + 1);
                        if cancel_edge && visits.get() == 64 {
                            cancel.cancel();
                        }
                    }
                    if !cancel_edge && progress.done == 14_096 {
                        cancel.cancel();
                    }
                }))
            });
            let result = histogram_with_budget(&dataset, &request, &control, &[], 0);
            CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(matches!(result, Err(AppError::Cancelled)));
            assert_eq!(
                dataset.cell(ColumnId(1), 4999).unwrap(),
                Cell::Number(4999.)
            );
        }
    }
}
