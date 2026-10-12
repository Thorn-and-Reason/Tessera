//! Complete frequency-replicated type-7 summaries and globally bounded outlier drawing.
use super::{
    exploration::{
        labels::LabelIndex,
        numeric::{display_index, interpolate, quartile_rank},
        ordering::{compare_cells, compare_text, sort_indices},
    },
    model::{checked_product, checked_sum},
    moments::{inside_box_fences, Moments},
    selection::{self, Eligible},
    support,
    validation::invalid_analysis,
    BoxplotGroup, BoxplotPoint, BoxplotRequest, ComputedBoxplot, Estimate, MAX_PLOT_POINTS,
};
use crate::{
    data::{Budget, Cell, Column, ColumnKind, Dataset},
    error::AppResult,
    operation::JobControl,
};
use std::{cmp::Ordering, mem::size_of};
const EMPTY: &str = "No eligible observations.";
const NO_WHISKERS: &str = "No observed values lie within the quartile fences.";

/// `extra_bytes` charges the live request and other caller-retained work once.
pub fn boxplot_with_budget(
    dataset: &Dataset,
    request: &BoxplotRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedBoxplot> {
    control.check_cancelled()?;
    let baseline = Budget::dataset_bytes_iter(
        std::iter::once(dataset).chain(retained.iter().copied()),
        checked_sum([extra_bytes, request.validation_scratch_bytes()?])?,
    )?;
    let ids = request.validate_current(dataset, retained, extra_bytes)?;
    let outcome = dataset.column(request.outcome)?;
    let group = request.group.map(|id| dataset.column(id)).transpose()?;
    let scratch = checked_sum([
        checked_product(dataset.row_count(), size_of::<Eligible>())?,
        checked_product(dataset.row_count(), 2 * size_of::<usize>())?,
        LabelIndex::scratch_bytes(group.map_or(0, |c| c.metadata().value_labels.len()))?,
        size_of::<Moments>(),
    ])?;
    let mut owned = checked_sum([
        size_of::<ComputedBoxplot>(),
        request.heap_bytes()?,
        support::snapshot_bytes(dataset, &ids)?,
        9 * EMPTY.len(),
    ])?;
    check_work(baseline, scratch, owned)?;
    let rows = dataset.row_count();
    let total = rows.checked_mul(5).ok_or_else(invalid_analysis)?;
    let (eligible, diagnostics) = selection::select_rows(
        dataset,
        &request.filter,
        request.frequency_weight,
        &[],
        control,
        total,
    )?;
    let (mut indices, missing_rows, missing_n) =
        present_indices(outcome, group, &eligible, control, total)?;
    let mut merge = vec![0; indices.len()];
    sort_indices(&mut indices, &mut merge, control, |a, b| {
        let a = eligible[a].row;
        let b = eligible[b].row;
        Ok(compare_group(group, a, b, control)?
            .then(
                number(outcome, a)?
                    .partial_cmp(&number(outcome, b)?)
                    .ok_or_else(invalid_analysis)?,
            )
            .then(a.cmp(&b)))
    })?;
    let (count, growth) = group_growth(group, &eligible, &indices, control, total)?;
    owned = checked_sum([owned, growth])?;
    check_work(baseline, scratch, owned)?;
    control.checkpoint(3 * rows, Some(total))?;
    let columns = support::capture_columns(dataset, &ids, control)?;
    let labels = request
        .group
        .map(|id| {
            let info = columns
                .iter()
                .find(|info| info.id == id)
                .ok_or_else(invalid_analysis)?;
            LabelIndex::new(info, control)
        })
        .transpose()?;
    let mut groups = Vec::with_capacity(count);
    let quiet = control.without_progress();
    let mut start = 0;
    while start < indices.len() {
        control.checkpoint(3 * rows + start, Some(total))?;
        let row = eligible[indices[start]].row;
        let mut end = start + 1;
        while end < indices.len()
            && compare_group(group, row, eligible[indices[end]].row, control)? == Ordering::Equal
        {
            if end.is_multiple_of(4096) {
                control.check_cancelled()?;
            }
            end += 1;
        }
        let mut summary = summarize(outcome, &eligible, &indices[start..end], &quiet)?;
        if let Some(column) = group {
            let value = group_value(column, row, control)?;
            summary.value_label_index = labels
                .as_ref()
                .ok_or_else(invalid_analysis)?
                .find(&value, control)?;
            summary.value = Some(value);
        }
        groups.push(summary);
        start = end;
    }
    if group.is_none() && groups.is_empty() {
        groups.push(empty_group());
    }
    let outliers = groups
        .iter()
        .enumerate()
        .try_fold(0usize, |n, (index, g)| {
            if index.is_multiple_of(4096) {
                control.check_cancelled()?;
            }
            n.checked_add(g.outlier_rows).ok_or_else(invalid_analysis)
        })?;
    let shown = outliers.min(MAX_PLOT_POINTS);
    owned = checked_sum([owned, checked_product(shown, size_of::<BoxplotPoint>())?])?;
    check_work(baseline, scratch, owned)?;
    let points = outlier_points(
        outcome,
        &eligible,
        &indices,
        &groups,
        control,
        (total, outliers),
    )?;
    let request = BoxplotRequest {
        outcome: request.outcome,
        group: request.group,
        filter: support::clone_filter(&request.filter, control)?,
        frequency_weight: request.frequency_weight,
    };
    control.checkpoint(total, Some(total))?;
    Ok(ComputedBoxplot {
        revision: dataset.revision(),
        request,
        columns,
        source_rows: rows,
        diagnostics,
        missing_group_rows: missing_rows[0],
        missing_group_n: missing_n[0],
        missing_outcome_rows: missing_rows[1],
        missing_outcome_n: missing_n[1],
        empty_reason: indices.is_empty().then(|| EMPTY.into()),
        groups,
        points,
        reduced: outliers > MAX_PLOT_POINTS,
    })
}
fn check_work(baseline: usize, scratch: usize, owned: usize) -> AppResult<()> {
    Budget::check_datasets(&[], checked_sum([baseline, scratch, owned])?)
}
fn number(column: &Column, row: usize) -> AppResult<f64> {
    match column.cell(row)? {
        Cell::Number(value) => Ok(value),
        _ => Err(invalid_analysis()),
    }
}
fn compare_group(
    group: Option<&Column>,
    a: usize,
    b: usize,
    control: &JobControl,
) -> AppResult<Ordering> {
    control.check_cancelled()?;
    match group {
        None => Ok(Ordering::Equal),
        Some(c) if c.kind() == ColumnKind::Text => compare_text(
            c.text_value(a)?.ok_or_else(invalid_analysis)?,
            c.text_value(b)?.ok_or_else(invalid_analysis)?,
            control,
        ),
        Some(c) => compare_cells(&c.cell(a)?, &c.cell(b)?, control),
    }
}
fn group_value(column: &Column, row: usize, control: &JobControl) -> AppResult<Cell> {
    if column.kind() == ColumnKind::Text {
        Ok(Cell::Text(support::clone_text(
            column.text_value(row)?.ok_or_else(invalid_analysis)?,
            control,
        )?))
    } else {
        let value = number(column, row)?;
        Ok(Cell::Number(if value == 0.0 { 0.0 } else { value }))
    }
}
fn present_indices(
    outcome: &Column,
    group: Option<&Column>,
    eligible: &[Eligible],
    control: &JobControl,
    total: usize,
) -> AppResult<(Vec<usize>, [usize; 2], [u64; 2])> {
    let mut indices = Vec::with_capacity(outcome.len());
    let (mut missing_rows, mut missing_n) = ([0; 2], [0; 2]);
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(outcome.len() + index, Some(total))?;
        }
        let role = if group
            .map(|c| c.is_missing(entry.row))
            .transpose()?
            .unwrap_or(false)
        {
            Some(0)
        } else if outcome.is_missing(entry.row)? {
            Some(1)
        } else {
            None
        };
        if let Some(role) = role {
            missing_rows[role] += 1;
            missing_n[role] += entry.weight;
        } else {
            indices.push(index);
        }
    }
    Ok((indices, missing_rows, missing_n))
}
fn group_growth(
    group: Option<&Column>,
    eligible: &[Eligible],
    indices: &[usize],
    control: &JobControl,
    total: usize,
) -> AppResult<(usize, usize)> {
    let (mut count, mut bytes) = (0, 0);
    for (position, &index) in indices.iter().enumerate() {
        if position.is_multiple_of(4096) {
            control.checkpoint(total / 5 * 2 + position, Some(total))?;
        }
        let row = eligible[index].row;
        if position == 0
            || compare_group(group, eligible[indices[position - 1]].row, row, control)?
                != Ordering::Equal
        {
            count += 1;
            let text = match group {
                Some(c) if c.kind() == ColumnKind::Text => {
                    c.text_value(row)?.ok_or_else(invalid_analysis)?.len()
                }
                _ => 0,
            };
            bytes = checked_sum([
                bytes,
                size_of::<BoxplotGroup>(),
                text,
                2 * NO_WHISKERS.len(),
            ])?;
        }
    }
    if group.is_none() && count == 0 {
        return Ok((1, size_of::<BoxplotGroup>()));
    }
    Ok((count, bytes))
}
fn quartile(
    column: &Column,
    eligible: &[Eligible],
    indices: &[usize],
    n: u64,
    quarter: u8,
    control: &JobControl,
) -> AppResult<f64> {
    let (rank, numerator, denominator) = quartile_rank(n, quarter)?;
    let upper = rank + u64::from(numerator != 0);
    let (mut cumulative, mut left) = (0, None);
    for (position, &index) in indices.iter().enumerate() {
        if position.is_multiple_of(4096) {
            control.checkpoint(position, None)?;
        }
        cumulative += eligible[index].weight;
        if left.is_none() && cumulative > rank {
            left = Some(number(column, eligible[index].row)?);
        }
        if cumulative > upper {
            return interpolate(
                left.ok_or_else(invalid_analysis)?,
                number(column, eligible[index].row)?,
                numerator,
                denominator,
            );
        }
    }
    Err(invalid_analysis())
}
fn summarize(
    column: &Column,
    eligible: &[Eligible],
    indices: &[usize],
    control: &JobControl,
) -> AppResult<BoxplotGroup> {
    let mut moments = Moments::default();
    for (position, &index) in indices.iter().enumerate() {
        if position.is_multiple_of(4096) {
            control.checkpoint(position, None)?;
        }
        let entry = eligible[index];
        moments.add(number(column, entry.row)?, entry.weight);
    }
    let n = moments.n;
    let q1 = quartile(column, eligible, indices, n, 1, control)?;
    let median = quartile(column, eligible, indices, n, 2, control)?;
    let q3 = quartile(column, eligible, indices, n, 3, control)?;
    let (mut low, mut high) = (None, None);
    let (mut outlier_rows, mut outlier_n) = (0, 0);
    for (position, &index) in indices.iter().enumerate() {
        if position.is_multiple_of(4096) {
            control.checkpoint(position, None)?;
        }
        let entry = eligible[index];
        let value = number(column, entry.row)?;
        if inside_box_fences(value, q1, q3) {
            if low.is_none() {
                low = Some(value);
            }
            high = Some(value);
        } else {
            outlier_rows += 1;
            outlier_n += entry.weight;
        }
    }
    Ok(BoxplotGroup {
        value: None,
        value_label_index: None,
        rows: indices.len(),
        n,
        mean: Estimate::available(moments.mean().ok_or_else(invalid_analysis)?),
        minimum: Estimate::available(number(column, eligible[indices[0]].row)?),
        q1: Estimate::available(q1),
        median: Estimate::available(median),
        q3: Estimate::available(q3),
        maximum: Estimate::available(number(
            column,
            eligible[*indices.last().ok_or_else(invalid_analysis)?].row,
        )?),
        whiskers: [low, high].map(|value| match value {
            Some(value) => Estimate::available(value),
            None => Estimate::unavailable(NO_WHISKERS),
        }),
        outlier_rows,
        outlier_n,
    })
}
fn empty_group() -> BoxplotGroup {
    BoxplotGroup {
        value: None,
        value_label_index: None,
        rows: 0,
        n: 0,
        mean: Estimate::unavailable(EMPTY),
        minimum: Estimate::unavailable(EMPTY),
        q1: Estimate::unavailable(EMPTY),
        median: Estimate::unavailable(EMPTY),
        q3: Estimate::unavailable(EMPTY),
        maximum: Estimate::unavailable(EMPTY),
        whiskers: [Estimate::unavailable(EMPTY), Estimate::unavailable(EMPTY)],
        outlier_rows: 0,
        outlier_n: 0,
    }
}
fn outlier_points(
    column: &Column,
    eligible: &[Eligible],
    indices: &[usize],
    groups: &[BoxplotGroup],
    control: &JobControl,
    progress: (usize, usize),
) -> AppResult<Vec<BoxplotPoint>> {
    let (total, outliers) = progress;
    let shown = outliers.min(MAX_PLOT_POINTS);
    let mut points = Vec::with_capacity(shown);
    let (mut ordinal, mut group_index, mut group_end) =
        (0, 0, groups.first().map_or(0, |g| g.rows));
    for (position, &index) in indices.iter().enumerate() {
        if position.is_multiple_of(4096) {
            control.checkpoint(4 * column.len() + position, Some(total))?;
        }
        if position == group_end {
            group_index += 1;
            group_end += groups[group_index].rows;
        }
        let g = &groups[group_index];
        let entry = eligible[index];
        let value = number(column, entry.row)?;
        if !inside_box_fences(
            value,
            g.q1.value.ok_or_else(invalid_analysis)?,
            g.q3.value.ok_or_else(invalid_analysis)?,
        ) {
            if points.len() < shown && ordinal == display_index(outliers, shown, points.len())? {
                points.push(BoxplotPoint {
                    group_index,
                    row: entry.row,
                    value,
                    frequency: entry.weight,
                });
            }
            ordinal += 1;
        }
    }
    Ok(points)
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::{data::ColumnId, error::AppError, operation::CHECKPOINT_HOOK};
    use std::cell::Cell as Flag;

    #[test]
    fn cancellation_after_cohort_during_merge_quantiles_and_drawing_preserves_source() {
        for phase in 0..5 {
            let dataset = Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "Value",
                (0..10_000).rev().map(|v| Some(v as f64)).collect(),
            )
            .unwrap()])
            .unwrap();
            let request = BoxplotRequest {
                outcome: ColumnId(1),
                group: None,
                filter: super::super::RowFilter::default(),
                frequency_weight: None,
            };
            let control = JobControl::default();
            let cancel = control.clone();
            let summarizing = Flag::new(false);
            let summary_pass = Flag::new(0);
            CHECKPOINT_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |p| {
                    if p.done == 30_000 && p.total == Some(50_000) {
                        summarizing.set(true);
                        summary_pass.set(0);
                    }
                    if summarizing.get() && p.total.is_none() && p.done == 0 {
                        summary_pass.set(summary_pass.get() + 1);
                    }
                    let stop = match phase {
                        0 => p.done == 14_096 && p.total == Some(50_000),
                        1 => p.done == 4096 && p.total == Some(10_000),
                        2 => {
                            summarizing.get()
                                && summary_pass.get() == 3
                                && p.total.is_none()
                                && p.done == 4096
                        }
                        3 => {
                            summarizing.get()
                                && summary_pass.get() == 5
                                && p.total.is_none()
                                && p.done == 4096
                        }
                        _ => p.done == 44_096 && p.total == Some(50_000),
                    };
                    if stop {
                        cancel.cancel();
                    }
                }));
            });
            let result = boxplot_with_budget(&dataset, &request, &control, &[], 0);
            CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(
                matches!(result, Err(AppError::Cancelled)),
                "phase {phase}: {result:?}"
            );
            assert_eq!(dataset.cell(ColumnId(1), 0).unwrap(), Cell::Number(9999.));
            assert_eq!(dataset.cell(ColumnId(1), 9999).unwrap(), Cell::Number(0.));
        }
    }

    #[test]
    fn cancellation_inside_borrowed_text_sort_and_captured_labels_preserves_source() {
        for metadata in [false, true] {
            let column = Column::text(
                ColumnId(2),
                "Group",
                vec![
                    Some("a".repeat(100_000) + "b"),
                    Some("a".repeat(100_000) + "c"),
                ],
            )
            .unwrap();
            let column = if metadata {
                column
                    .with_metadata(crate::data::ColumnMetadata {
                        value_labels: vec![crate::data::ValueLabel {
                            value: Cell::Text("a".repeat(100_000) + "b"),
                            label: "ŵ".repeat(100_000),
                        }],
                        ..column.metadata().clone()
                    })
                    .unwrap()
            } else {
                column
            };
            let dataset = Dataset::new(vec![
                Column::numeric(ColumnId(1), "Value", vec![Some(1.); 2]).unwrap(),
                column,
            ])
            .unwrap();
            let request = BoxplotRequest {
                outcome: ColumnId(1),
                group: Some(ColumnId(2)),
                filter: super::super::RowFilter::default(),
                frequency_weight: None,
            };
            let control = JobControl::default();
            let cancel = control.clone();
            // For metadata mode allow sorting to finish, then cancel in the snapshot clone.
            let capturing = Flag::new(false);
            CHECKPOINT_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |p| {
                    if p.done == 6 && p.total == Some(10) {
                        capturing.set(true);
                    }
                    if p.total.is_none() && p.done >= 8192 && (!metadata || capturing.get()) {
                        cancel.cancel();
                    }
                }));
            });
            let result = boxplot_with_budget(&dataset, &request, &control, &[], 0);
            CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(matches!(result, Err(AppError::Cancelled)));
            assert_eq!(
                dataset
                    .column(ColumnId(2))
                    .unwrap()
                    .text_value(0)
                    .unwrap()
                    .unwrap()
                    .len(),
                100_001
            );
        }
    }
}
