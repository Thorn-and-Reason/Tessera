//! Exact treatment coding on complete positive-frequency observations.
use super::*;
use crate::{
    analysis::{
        exploration::ordering::{compare_cells, compare_text, sort_indices},
        model::checked_sum,
        selection::Eligible,
        support::clone_cell,
        validation::invalid_analysis,
    },
    data::{AllocationCounter, Cell, Column, ColumnKind, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::cmp::Ordering;

pub(super) fn oversized_model() -> AppError {
    AppError::invalid(
        "regression model",
        "The model exceeds 64 expanded terms, including the intercept.",
    )
}
/// `live_bytes` includes deduplicated datasets, retained work, live request, eligibility
/// capacity and all other simultaneous caller-owned storage, exactly once.
/// Each new index, level, term and string owner is charged before allocation.
pub(super) fn discover(
    dataset: &Dataset,
    request: &RegressionRequest,
    eligible: &[Eligible],
    control: &JobControl,
    live_bytes: usize,
) -> AppResult<(
    Vec<RegressionLevels>,
    Vec<RegressionTerm>,
    Option<RegressionFailure>,
)> {
    let quiet = control.without_progress();
    let control = &quiet;
    control.check_cancelled()?;
    let mut counter = AllocationCounter { bytes: live_bytes };
    counter.add(request.validation_scratch_bytes()?)?;
    validation::request(request, |id| Ok(dataset.column(id)?.kind()))?;
    counter.bytes = live_bytes;
    if request
        .predictors
        .iter()
        .filter(|p| matches!(p.role, PredictorRole::Numeric))
        .count()
        >= MAX_TERMS
    {
        return Err(oversized_model());
    }
    let mut indices = complete_indices(eligible, control, &mut counter)?;
    let mut scratch = vec![0; indices.len()];
    let result = code_predictors(
        dataset,
        request,
        &mut indices,
        &mut scratch,
        control,
        &mut counter,
    );
    control.check_cancelled()?;
    result
}
fn complete_indices(
    eligible: &[Eligible],
    control: &JobControl,
    counter: &mut AllocationCounter,
) -> AppResult<Vec<usize>> {
    let mut count = 0;
    for (index, row) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(index, None)?;
        }
        count += usize::from(row.complete);
    }
    counter.array::<usize>(count)?;
    counter.array::<usize>(count)?;
    let mut indices = Vec::with_capacity(count);
    for (index, row) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(index, None)?;
        }
        if row.complete {
            indices.push(row.row);
        }
    }
    Ok(indices)
}
fn code_predictors(
    dataset: &Dataset,
    request: &RegressionRequest,
    indices: &mut [usize],
    scratch: &mut [usize],
    control: &JobControl,
    counter: &mut AllocationCounter,
) -> AppResult<(
    Vec<RegressionLevels>,
    Vec<RegressionTerm>,
    Option<RegressionFailure>,
)> {
    let categories = request
        .predictors
        .iter()
        .filter(|p| matches!(p.role, PredictorRole::Categorical { .. }))
        .count()
        .min(MAX_TERMS);
    counter.array::<RegressionLevels>(categories)?;
    counter.array::<RegressionTerm>(MAX_TERMS)?;
    let mut levels = Vec::with_capacity(categories);
    let mut terms = Vec::with_capacity(MAX_TERMS);
    terms.push(RegressionTerm::Intercept);
    if indices.is_empty() {
        return Ok((
            levels,
            terms,
            Some(failure(
                RegressionFailureKind::EmptyCohort,
                "No complete positive-frequency observations remain.",
                counter,
            )?),
        ));
    }
    let mut expanded = checked_sum([
        1,
        request
            .predictors
            .iter()
            .filter(|p| matches!(p.role, PredictorRole::Numeric))
            .count(),
    ])?;
    for predictor in &request.predictors {
        control.check_cancelled()?;
        match &predictor.role {
            PredictorRole::Numeric => terms.push(RegressionTerm::Numeric {
                column: predictor.column,
            }),
            PredictorRole::Categorical { reference } => {
                let values = collect_levels(
                    dataset.column(predictor.column)?,
                    indices,
                    scratch,
                    MAX_TERMS
                        .checked_sub(expanded)
                        .and_then(|n| n.checked_add(1))
                        .ok_or_else(oversized_model)?,
                    control,
                    counter,
                )?;
                expanded = checked_sum([expanded, values.len() - 1])?;
                let early =
                    reference_failure(predictor.column, &values, reference, counter, control)?;
                if early.is_none() {
                    append_contrasts(
                        predictor.column,
                        &values,
                        reference,
                        control,
                        counter,
                        &mut terms,
                    )?;
                }
                levels.push(RegressionLevels {
                    column: predictor.column,
                    values,
                });
                if let Some(failure) = early {
                    return Ok((levels, terms, Some(failure)));
                }
            }
        }
    }
    control.check_cancelled()?;
    Ok((levels, terms, None))
}
fn collect_levels(
    column: &Column,
    indices: &mut [usize],
    scratch: &mut [usize],
    limit: usize,
    control: &JobControl,
    counter: &mut AllocationCounter,
) -> AppResult<Vec<Cell>> {
    sort_indices(indices, scratch, control, |a, b| {
        compare_rows(column, a, b, control)
    })?;
    let count = distinct_count(column, indices, limit, control)?;
    counter.array::<Cell>(count)?;
    let mut values = Vec::with_capacity(count);
    let mut previous = None;
    for (index, row) in indices.iter().copied().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(index, None)?;
        }
        if let Some(p) = previous {
            if compare_rows(column, p, row, control)? == Ordering::Equal {
                continue;
            }
        }
        values.push(copy_value(column, row, control, counter)?);
        previous = Some(row);
    }
    Ok(values)
}
fn reference_failure(
    column: crate::data::ColumnId,
    values: &[Cell],
    reference: &Cell,
    counter: &mut AllocationCounter,
    control: &JobControl,
) -> AppResult<Option<RegressionFailure>> {
    let mut present = false;
    for value in values {
        if compare_cells(value, reference, control)? == Ordering::Equal {
            present = true;
            break;
        }
    }
    let early = if !present {
        Some((
            RegressionFailureKind::AbsentReference { column },
            "The reference category is absent from the complete fitted cohort.",
        ))
    } else if values.len() == 1 {
        Some((
            RegressionFailureKind::NoContrast { column },
            "The categorical predictor has no contrast in the complete fitted cohort.",
        ))
    } else {
        None
    };
    early
        .map(|(kind, reason)| failure(kind, reason, counter))
        .transpose()
}
fn append_contrasts(
    column: crate::data::ColumnId,
    values: &[Cell],
    reference: &Cell,
    control: &JobControl,
    counter: &mut AllocationCounter,
    terms: &mut Vec<RegressionTerm>,
) -> AppResult<()> {
    for value in values {
        if compare_cells(value, reference, control)? == Ordering::Equal {
            continue;
        }
        counter.add(copy_bytes(value))?;
        counter.add(copy_bytes(reference))?;
        terms.push(RegressionTerm::Contrast {
            column,
            level: canonical_clone(value, control)?,
            reference: canonical_clone(reference, control)?,
        });
    }
    Ok(())
}
fn failure(
    kind: RegressionFailureKind,
    reason: &str,
    counter: &mut AllocationCounter,
) -> AppResult<RegressionFailure> {
    counter.add(reason.len())?;
    Ok(RegressionFailure {
        kind,
        reason: reason.into(),
        expected_terms: None,
        rank: None,
        implicated_terms: vec![],
    })
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
fn distinct_count(
    column: &Column,
    rows: &[usize],
    limit: usize,
    control: &JobControl,
) -> AppResult<usize> {
    let mut count = 0;
    let mut previous = None;
    for (index, row) in rows.iter().copied().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(index, None)?;
        }
        if let Some(p) = previous {
            if compare_rows(column, p, row, control)? == Ordering::Equal {
                continue;
            }
        }
        count += 1;
        if count > limit {
            return Err(oversized_model());
        }
        previous = Some(row);
    }
    Ok(count)
}
fn copy_bytes(value: &Cell) -> usize {
    if let Cell::Text(s) = value {
        s.len()
    } else {
        0
    }
}
fn canonical_clone(value: &Cell, control: &JobControl) -> AppResult<Cell> {
    if matches!(value, Cell::Number(v) if *v == 0.) {
        Ok(Cell::Number(0.))
    } else {
        clone_cell(value, control)
    }
}
fn copy_value(
    column: &Column,
    row: usize,
    control: &JobControl,
    counter: &mut AllocationCounter,
) -> AppResult<Cell> {
    match column.kind() {
        ColumnKind::Text => {
            let text = column.text_value(row)?.ok_or_else(invalid_analysis)?;
            counter.add(text.len())?;
            Ok(Cell::Text(crate::analysis::support::clone_text(
                text, control,
            )?))
        }
        ColumnKind::Number => canonical_clone(&column.cell(row)?, control),
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn reference_identity_comparison_cancels_inside_a_long_text_prefix() {
        let control = JobControl::default();
        let cancel = control.clone();
        let values = [Cell::Text("ŵ".repeat(10_000) + "a")];
        let reference = Cell::Text("ŵ".repeat(10_000) + "b");
        crate::operation::CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |p| {
                if p.done >= 8192 {
                    cancel.cancel();
                }
            }));
        });
        let result = reference_failure(
            crate::data::ColumnId(3),
            &values,
            &reference,
            &mut AllocationCounter::default(),
            &control,
        );
        crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
    }
}
