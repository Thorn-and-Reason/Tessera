use super::*;
use crate::{
    analysis::{
        model::{checked_product, checked_sum},
        selection::{select_rows, Eligible},
        validation::invalid_analysis,
        MAX_FREQUENCY,
    },
    data::{Budget, Dataset},
    error::AppResult,
    operation::JobControl,
};
use std::mem::size_of;
pub fn capture_cohort(
    d: &Dataset,
    r: &ReliabilityRequest,
    control: &JobControl,
) -> AppResult<ReliabilityCohort> {
    let quiet = control.without_progress();
    let c = &quiet;
    c.check_cancelled()?;
    r.admit_controlled(c)?;
    let work = super::accounting::source_work(d, r, c)?;
    let scratch = checked_sum([
        r.validation_scratch_bytes()?,
        size_of::<ReliabilityRequest>(),
        r.heap_bytes_controlled(Some(c))?,
        size_of::<ReliabilityCohort>(),
        checked_product(d.row_count(), size_of::<Eligible>())?,
        6640,
    ])?;
    Budget::check_datasets(&[d], scratch)?;
    super::validation::request(r, |id| Ok(d.column(id)?.kind()), c)?;
    let (eligible, diagnostics) =
        select_rows(d, &r.filter, r.frequency_weight, &r.items, control, work)?;
    let mut complete_rows = 0;
    let mut frequency = 0_u64;
    let mut incomplete_case_frequency = 0_u64;
    for (index, e) in eligible.iter().enumerate() {
        if index.is_multiple_of(64) {
            c.checkpoint(index, None)?;
        }
        if e.complete {
            complete_rows += 1;
            frequency = frequency
                .checked_add(e.weight)
                .filter(|n| *n <= MAX_FREQUENCY)
                .ok_or_else(invalid_analysis)?;
        } else {
            incomplete_case_frequency = incomplete_case_frequency
                .checked_add(e.weight)
                .filter(|n| *n <= MAX_FREQUENCY)
                .ok_or_else(invalid_analysis)?;
        }
    }
    2_u128
        .checked_mul((r.items.len() + 1) as u128)
        .and_then(|v| v.checked_mul(u128::from(frequency)))
        .filter(|v| *v <= 1_u128 << 66)
        .ok_or_else(invalid_analysis)?;
    for (index, e) in eligible.iter().enumerate() {
        if index.is_multiple_of(64) {
            c.checkpoint(index, None)?;
        }
        for reverse in &r.reversals {
            c.check_cancelled()?;
            let col = d.column(reverse.column)?;
            if !col.is_missing_controlled(e.row, c)? {
                let i = r
                    .items
                    .iter()
                    .position(|id| *id == reverse.column)
                    .ok_or_else(invalid_analysis)?;
                score_item(d, r, e.row, i)?;
            }
        }
    }
    let cohort = ReliabilityCohort {
        eligible,
        diagnostics,
        complete_rows,
        frequency,
        incomplete_case_frequency,
    };
    Budget::check_datasets(
        &[d],
        checked_sum([
            r.validation_scratch_bytes()?,
            size_of::<ReliabilityRequest>(),
            r.heap_bytes_controlled(Some(c))?,
            cohort.owned_bytes()?,
            6640,
        ])?,
    )?;
    c.check_cancelled()?;
    Ok(cohort)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::RowPredicate,
        data::{Column, ColumnId},
        error::AppError,
        operation::CHECKPOINT_HOOK,
    };
    fn source() -> Dataset {
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "a", vec![Some(1.), None, Some(2.)]).unwrap(),
            Column::numeric(ColumnId(2), "b", vec![Some(4.); 3]).unwrap(),
        ])
        .unwrap()
    }
    fn recipe() -> ReliabilityRequest {
        ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2)],
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![],
        }
    }
    #[test]
    fn cohort_retains_original_eligible_order_flags_and_capacity() {
        let d = source();
        let c = capture_cohort(&d, &recipe(), &JobControl::default()).unwrap();
        assert_eq!(
            c.eligible
                .iter()
                .map(|e| (e.row, e.weight, e.complete))
                .collect::<Vec<_>>(),
            vec![(0, 1, true), (1, 1, false), (2, 1, true)]
        );
        assert_eq!(
            (c.complete_rows, c.frequency, c.incomplete_case_frequency),
            (2, 2, 1)
        );
        assert!(c.eligible.capacity() >= 3);
        assert!(c.owned_bytes().unwrap() >= 3 * size_of::<Eligible>());
    }
    #[test]
    fn wide_request_cancels_in_capacity_scan_before_role_or_row_allocation() {
        let d = source();
        let r = ReliabilityRequest {
            filter: crate::analysis::RowFilter {
                predicates: vec![
                    RowPredicate::Missing {
                        column: ColumnId(1),
                        missing: false
                    };
                    1024
                ],
            },
            ..recipe()
        };
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |p| {
                if p.done >= 64 {
                    cancel.cancel()
                }
            }))
        });
        let (result, largest) =
            crate::data::measure_test_allocations(|| capture_cohort(&d, &r, &control));
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(largest, 0);
    }
    #[test]
    fn role_workspace_budget_is_checked_before_rows_allocate() {
        let d = source();
        let r = ReliabilityRequest {
            filter: crate::analysis::RowFilter {
                predicates: vec![
                    RowPredicate::Missing {
                        column: ColumnId(1),
                        missing: false
                    };
                    270000
                ],
            },
            ..recipe()
        };
        let control = JobControl::default();
        let (result, largest) =
            crate::data::measure_test_allocations(|| capture_cohort(&d, &r, &control));
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
    }
}
