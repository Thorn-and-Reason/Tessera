//! Fallible stable sorting over caller-precharged index and scratch buffers.
use crate::{
    analysis::validation::invalid_analysis, data::Cell, error::AppResult, operation::JobControl,
};
use std::cmp::Ordering;

const CHECK_INTERVAL: usize = 4096;

pub(crate) fn sort_indices(
    indices: &mut [usize],
    scratch: &mut [usize],
    control: &JobControl,
    compare: impl Fn(usize, usize) -> AppResult<Ordering>,
) -> AppResult<()> {
    let quiet = control.without_progress();
    let control = &quiet;
    control.checkpoint(0, None)?;
    let n = indices.len();
    if scratch.len() < n {
        return Err(invalid_analysis());
    }
    let mut width = 1;
    while width < n {
        let mut start = 0;
        while start < n {
            let middle = start.saturating_add(width).min(n);
            let end = middle.saturating_add(width).min(n);
            let (mut left, mut right) = (start, middle);
            for (index, slot) in scratch.iter_mut().enumerate().take(end).skip(start) {
                if index.is_multiple_of(CHECK_INTERVAL) {
                    control.checkpoint(index, Some(n))?;
                }
                if right == end
                    || (left < middle
                        && compare(indices[left], indices[right])? != Ordering::Greater)
                {
                    *slot = indices[left];
                    left += 1;
                } else {
                    *slot = indices[right];
                    right += 1;
                }
            }
            start = end;
        }
        for (chunk, (target, source)) in indices
            .chunks_mut(CHECK_INTERVAL)
            .zip(scratch.chunks(CHECK_INTERVAL))
            .enumerate()
        {
            control.checkpoint(
                chunk
                    .checked_mul(CHECK_INTERVAL)
                    .ok_or_else(invalid_analysis)?,
                Some(n),
            )?;
            target.copy_from_slice(&source[..target.len()]);
        }
        control.checkpoint(n, Some(n))?;
        if width >= n - width {
            break;
        }
        width *= 2;
    }
    control.check_cancelled()
}

pub(crate) fn compare_cells(
    left: &Cell,
    right: &Cell,
    control: &JobControl,
) -> AppResult<Ordering> {
    control.check_cancelled()?;
    match (left, right) {
        (Cell::Number(a), Cell::Number(b)) if a.is_finite() && b.is_finite() => {
            // Numeric comparison makes the two signed zeros one category.
            a.partial_cmp(b).ok_or_else(invalid_analysis)
        }
        (Cell::Text(a), Cell::Text(b)) => compare_text(a, b, control),
        _ => Err(invalid_analysis()),
    }
}

/// Exact borrowed text comparison, with cancellation inside common long prefixes.
pub(crate) fn compare_text(a: &str, b: &str, control: &JobControl) -> AppResult<Ordering> {
    let quiet = control.without_progress();
    let control = &quiet;
    control.check_cancelled()?;
    for (chunk, (a, b)) in a
        .as_bytes()
        .chunks(CHECK_INTERVAL)
        .zip(b.as_bytes().chunks(CHECK_INTERVAL))
        .enumerate()
    {
        control.checkpoint(
            chunk
                .checked_mul(CHECK_INTERVAL)
                .ok_or_else(invalid_analysis)?,
            None,
        )?;
        let order = a.cmp(b);
        if order != Ordering::Equal {
            return Ok(order);
        }
    }
    Ok(a.len().cmp(&b.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{error::AppError, operation::CHECKPOINT_HOOK};
    use std::cell::Cell as Counter;

    #[test]
    fn shared_sort_text_and_label_checks_preserve_coalesced_progress_and_hooks() {
        use crate::{
            analysis::{exploration::labels::LabelIndex, RunColumnInfo},
            data::{ColumnId, ColumnKind, MissingRules, ValueLabel},
            operation::Progress,
        };
        use std::{
            rc::Rc,
            sync::{Arc, Mutex},
        };
        let latest = Arc::new(Mutex::new(None));
        let control = JobControl::coalesced(Arc::clone(&latest));
        let expected = Progress {
            done: 17,
            total: Some(99),
        };
        control.checkpoint(expected.done, expected.total).unwrap();
        let calls = Rc::new(Counter::new(0));
        let observed = Rc::clone(&calls);
        let slot = Arc::clone(&latest);
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |_| {
                assert_eq!(*slot.lock().unwrap(), Some(expected));
                observed.set(observed.get() + 1);
            }))
        });
        let values = ["b", "a", "a", "c"].map(|suffix| "ŵ".repeat(6000) + suffix);
        let mut indices = [0, 1, 2, 3];
        sort_indices(&mut indices, &mut [0; 4], &control, |a, b| {
            compare_text(&values[a], &values[b], &control)
        })
        .unwrap();
        let info = RunColumnInfo {
            id: ColumnId(1),
            name: "Group".into(),
            kind: ColumnKind::Text,
            description: String::new(),
            missing_rules: MissingRules::None,
            value_labels: vec![
                ValueLabel {
                    value: Cell::Text(values[0].clone()),
                    label: "B".into(),
                },
                ValueLabel {
                    value: Cell::Text(values[1].clone()),
                    label: "A".into(),
                },
            ],
        };
        let labels = LabelIndex::new(&info, &control).unwrap();
        assert_eq!(
            labels
                .find(&Cell::Text(values[1].clone()), &control)
                .unwrap(),
            Some(1)
        );
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(indices, [1, 2, 0, 3]);
        assert!(calls.get() > 10);
        assert_eq!(*latest.lock().unwrap(), Some(expected));
    }

    #[test]
    fn stable_numeric_and_exact_text_ordering() {
        let control = JobControl::default();
        let cells = [
            Cell::Number(3.0),
            Cell::Number(-0.0),
            Cell::Number(0.0),
            Cell::Number(-1.0),
            Cell::Number(3.0),
        ];
        let mut indices = [0, 1, 2, 3, 4];
        sort_indices(&mut indices, &mut [0; 5], &control, |a, b| {
            compare_cells(&cells[a], &cells[b], &control)
        })
        .unwrap();
        assert_eq!(indices, [3, 1, 2, 0, 4]);
        let cells = ["é", "A", "a ", "", " ", "a", "a", "\n"].map(|s| Cell::Text(s.into()));
        let mut indices = [0, 1, 2, 3, 4, 5, 6, 7];
        sort_indices(&mut indices, &mut [0; 8], &control, |a, b| {
            compare_cells(&cells[a], &cells[b], &control)
        })
        .unwrap();
        assert_eq!(indices, [3, 7, 4, 1, 5, 6, 2, 0]);
        // Repeated keys exercise stability across every bottom-up merge stage.
        let keys = (0..513).map(|i| (i * 73) % 17).collect::<Vec<_>>();
        let mut indices = (0..keys.len()).collect::<Vec<_>>();
        let mut expected = indices.clone();
        expected.sort_by_key(|i| keys[*i]);
        let mut scratch = vec![0; keys.len() + 1];
        sort_indices(&mut indices, &mut scratch, &control, |a, b| {
            Ok(keys[a].cmp(&keys[b]))
        })
        .unwrap();
        assert_eq!(indices, expected);
        sort_indices(&mut [], &mut [], &control, |_, _| unreachable!()).unwrap();
        sort_indices(&mut [0], &mut [0], &control, |_, _| unreachable!()).unwrap();
    }

    #[test]
    fn rejects_missing_mixed_nonfinite_and_short_scratch() {
        let control = JobControl::default();
        for (a, b) in [
            (Cell::Missing, Cell::Missing),
            (Cell::Number(1.0), Cell::Text("1".into())),
            (Cell::Number(f64::NAN), Cell::Number(1.0)),
            (Cell::Number(1.0), Cell::Number(f64::INFINITY)),
        ] {
            assert!(compare_cells(&a, &b, &control).is_err());
        }
        assert!(sort_indices(&mut [1, 0], &mut [0], &control, |a, b| Ok(a.cmp(&b))).is_err());
    }

    #[test]
    fn cancellation_inside_long_text_comparison() {
        let control = JobControl::default();
        let a = Cell::Text("a".repeat(400_000) + "b");
        let b = Cell::Text("a".repeat(400_000) + "c");
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |p| {
                if p.done >= 8192 {
                    cancel.cancel();
                }
            }))
        });
        let result = compare_cells(&a, &b, &control);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(
            compare_cells(&a, &b, &JobControl::default()).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn comparator_errors_and_midmerge_cancellation_propagate() {
        let control = JobControl::default();
        let calls = Counter::new(0);
        let mut indices = (0..16_384).rev().collect::<Vec<_>>();
        let mut scratch = vec![0; indices.len()];
        let result = sort_indices(&mut indices, &mut scratch, &control, |a, b| {
            calls.set(calls.get() + 1);
            if calls.get() == 10_000 {
                control.cancel();
            }
            Ok(a.cmp(&b))
        });
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert!(calls.get() >= 10_000 && calls.get() < 15_000);
        let control = JobControl::default();
        let result = sort_indices(&mut [1, 0], &mut [0; 2], &control, |_, _| {
            Err(AppError::Conflict("comparison failed".into()))
        });
        assert!(matches!(result, Err(AppError::Conflict(_))));
        control.cancel();
        assert!(matches!(
            sort_indices(&mut [], &mut [], &control, |a, b| Ok(a.cmp(&b))),
            Err(AppError::Cancelled)
        ));
    }
}
