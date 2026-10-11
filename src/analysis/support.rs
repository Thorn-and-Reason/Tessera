//! Precharged current-schema reference checks and frozen metadata snapshots.
use super::{
    model::{checked_product, checked_sum},
    validation, AnalysisRequestRef, RowFilter, RowPredicate, RunColumnInfo,
};
use crate::{
    data::{Cell, ColumnId, Dataset, MissingRules, ValueLabel},
    error::AppResult,
    operation::JobControl,
};
use std::{collections::BTreeSet, mem::size_of};

/// Caller must precharge `request.validation_scratch_bytes()` before allocating indexes.
pub(crate) fn validate_current(
    dataset: &Dataset,
    request: AnalysisRequestRef<'_>,
) -> AppResult<BTreeSet<ColumnId>> {
    validation::validate_request(request, |id| Ok(dataset.column(id)?.kind()))
}
/// Precharged strings copy at UTF-8 boundaries with a neutral cancellation check per 4 KiB.
pub(crate) fn clone_text(source: &str, control: &JobControl) -> AppResult<String> {
    control.check_cancelled()?;
    let mut result = String::with_capacity(source.len());
    let mut start = 0;
    while start < source.len() {
        copy_checkpoint(control, start)?;
        let mut end = start.saturating_add(4096).min(source.len());
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        result.push_str(&source[start..end]);
        start = end;
    }
    control.check_cancelled()?;
    Ok(result)
}
// Copy work must not reset the enclosing analysis's public progress.
fn copy_checkpoint(control: &JobControl, _bytes: usize) -> AppResult<()> {
    #[cfg(test)]
    crate::operation::CHECKPOINT_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook(crate::operation::Progress {
                done: _bytes,
                total: None,
            });
        }
    });
    control.check_cancelled()
}
pub(super) fn clone_cell(value: &Cell, control: &JobControl) -> AppResult<Cell> {
    Ok(match value {
        Cell::Text(v) => Cell::Text(clone_text(v, control)?),
        other => other.clone(),
    })
}
/// Caller precharges `snapshot_bytes` before this cancellable metadata copy.
pub(crate) fn capture_columns(
    dataset: &Dataset,
    ids: &BTreeSet<crate::data::ColumnId>,
    control: &JobControl,
) -> AppResult<Vec<RunColumnInfo>> {
    control.check_cancelled()?;
    let mut columns = Vec::with_capacity(ids.len());
    for id in ids {
        control.check_cancelled()?;
        let column = dataset.column(*id)?;
        let m = column.metadata();
        let missing_rules = match &m.missing_rules {
            MissingRules::None => MissingRules::None,
            MissingRules::Text(values) => {
                let mut copied = Vec::with_capacity(values.len());
                for value in values {
                    copied.push(clone_text(value, control)?);
                }
                MissingRules::Text(copied)
            }
            MissingRules::Number { values, ranges } => {
                let mut copied_values = Vec::with_capacity(values.len());
                let mut copied_ranges = Vec::with_capacity(ranges.len());
                for chunk in values.chunks(4096) {
                    control.check_cancelled()?;
                    copied_values.extend_from_slice(chunk);
                }
                for chunk in ranges.chunks(4096) {
                    control.check_cancelled()?;
                    copied_ranges.extend_from_slice(chunk);
                }
                MissingRules::Number {
                    values: copied_values,
                    ranges: copied_ranges,
                }
            }
        };
        let mut value_labels = Vec::with_capacity(m.value_labels.len());
        for (index, label) in m.value_labels.iter().enumerate() {
            if index.is_multiple_of(4096) {
                control.check_cancelled()?;
            }
            value_labels.push(ValueLabel {
                value: clone_cell(&label.value, control)?,
                label: clone_text(&label.label, control)?,
            });
        }
        columns.push(RunColumnInfo {
            id: *id,
            name: clone_text(&m.name, control)?,
            kind: column.kind(),
            description: clone_text(&m.description, control)?,
            missing_rules,
            value_labels,
        });
    }
    Ok(columns)
}
pub(crate) fn snapshot_bytes(dataset: &Dataset, ids: &BTreeSet<ColumnId>) -> AppResult<usize> {
    let mut bytes = checked_product(ids.len(), size_of::<RunColumnInfo>())?;
    for id in ids {
        bytes = checked_sum([bytes, dataset.column(*id)?.metadata().heap_bytes()?])?;
    }
    Ok(bytes)
}

/// Caller precharges filter vector/string storage before this cancellable clone.
pub(crate) fn clone_filter(filter: &RowFilter, control: &JobControl) -> AppResult<RowFilter> {
    control.check_cancelled()?;
    let mut predicates = Vec::with_capacity(filter.predicates.len());
    for predicate in &filter.predicates {
        control.check_cancelled()?;
        predicates.push(match predicate {
            RowPredicate::Text {
                column,
                value,
                equal,
            } => RowPredicate::Text {
                column: *column,
                value: clone_text(value, control)?,
                equal: *equal,
            },
            other => other.clone(),
        });
    }
    Ok(RowFilter { predicates })
}

/// Borrowed capacity accounting, with no allocation or neutral-control owner.
pub(crate) fn metadata_heap_bytes(
    name: &String,
    description: &String,
    rules: &MissingRules,
    labels: &Vec<ValueLabel>,
    c: Option<&JobControl>,
) -> AppResult<usize> {
    let mut n = checked_sum([
        name.capacity(),
        description.capacity(),
        checked_product(labels.capacity(), size_of::<ValueLabel>())?,
    ])?;
    match rules {
        MissingRules::None => {}
        MissingRules::Number { values, ranges } => {
            n = checked_sum([
                n,
                checked_product(values.capacity(), size_of::<f64>())?,
                checked_product(ranges.capacity(), size_of::<(f64, f64)>())?,
            ])?
        }
        MissingRules::Text(values) => {
            n = checked_sum([n, checked_product(values.capacity(), size_of::<String>())?])?;
            for (i, s) in values.iter().enumerate() {
                if i.is_multiple_of(64) {
                    if let Some(c) = c {
                        c.checkpoint(i, None)?;
                    }
                }
                n = checked_sum([n, s.capacity()])?;
            }
        }
    }
    metadata_labels_bytes(labels, n, c)
}
fn metadata_labels_bytes(
    labels: &[ValueLabel],
    mut n: usize,
    c: Option<&JobControl>,
) -> AppResult<usize> {
    for (i, label) in labels.iter().enumerate() {
        if i.is_multiple_of(64) {
            if let Some(c) = c {
                c.checkpoint(i, None)?;
            }
        }
        n = checked_sum([
            n,
            label.label.capacity(),
            if let Cell::Text(v) = &label.value {
                v.capacity()
            } else {
                0
            },
        ])?;
    }
    Ok(n)
}
pub(crate) fn snapshot_bytes_controlled(
    dataset: &Dataset,
    ids: &BTreeSet<ColumnId>,
    c: &JobControl,
) -> AppResult<usize> {
    c.check_cancelled()?;
    let mut n = checked_product(ids.len(), size_of::<RunColumnInfo>())?;
    for id in ids {
        c.checkpoint(0, None)?;
        let m = dataset.column(*id)?.metadata();
        n = checked_sum([
            n,
            metadata_heap_bytes(
                &m.name,
                &m.description,
                &m.missing_rules,
                &m.value_labels,
                Some(c),
            )?,
        ])?;
    }
    c.check_cancelled()?;
    Ok(n)
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::{data::Column, error::AppError, operation::CHECKPOINT_HOOK};
    #[test]
    fn multibyte_copy_keeps_bytes_and_cancels_inside_strings_and_metadata() {
        let source = "ŵ🦀İ".repeat(50_000);
        assert_eq!(clone_text(&source, &JobControl::default()).unwrap(), source);
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done >= 8192 {
                    cancel.cancel();
                }
            }))
        });
        let result = clone_text(&source, &control);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        let column = Column::numeric(ColumnId(1), "N", vec![Some(1.)]).unwrap();
        let column = column
            .with_metadata(crate::data::ColumnMetadata {
                value_labels: vec![ValueLabel {
                    value: Cell::Number(1.),
                    label: source.clone(),
                }],
                ..column.metadata().clone()
            })
            .unwrap();
        let dataset = Dataset::new(vec![column]).unwrap();
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done >= 8192 {
                    cancel.cancel();
                }
            }))
        });
        let result = capture_columns(&dataset, &BTreeSet::from([ColumnId(1)]), &control);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(
            dataset.column(ColumnId(1)).unwrap().metadata().value_labels[0].label,
            source
        );
    }
    #[test]
    fn filter_copy_preserves_exact_predicates_and_can_cancel() {
        let filter = RowFilter {
            predicates: vec![
                RowPredicate::Text {
                    column: ColumnId(1),
                    value: "ŵ ".repeat(20_000),
                    equal: false,
                },
                RowPredicate::Missing {
                    column: ColumnId(2),
                    missing: true,
                },
            ],
        };
        assert_eq!(
            clone_filter(&filter, &JobControl::default()).unwrap(),
            filter
        );
        let control = JobControl::default();
        control.cancel();
        assert!(matches!(
            clone_filter(&filter, &control),
            Err(AppError::Cancelled)
        ));
    }
}
