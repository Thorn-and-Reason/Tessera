//! Ordered, pair-specific Pearson estimates over a shared eligible-row cohort.
mod inference;
use super::{
    inference::INFERENCE_WORK_BYTES,
    model::{checked_product, checked_sum},
    moments::{BoundedSum, Moments},
    selection::{select_rows, Eligible},
    support, AnalysisRequestRef, ComputedPearson, CorrelationMissingPolicy, CorrelationRow,
    Estimate, PearsonRequest,
};
use crate::{
    data::{Budget, Cell, Column, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::mem::size_of;

/// `extra_bytes` includes the caller's live request and other non-dataset work.
pub fn pearson_with_budget(
    dataset: &Dataset,
    request: &PearsonRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedPearson> {
    control.check_cancelled()?;
    let view = AnalysisRequestRef::Pearson(request);
    let pairs = checked_product(
        request.columns.len(),
        request.columns.len().saturating_sub(1),
    )? / 2;
    let validation = view.validation_scratch_bytes()?;
    let base = checked_sum([extra_bytes, validation, INFERENCE_WORK_BYTES])?;
    let datasets = std::iter::once(dataset).chain(retained.iter().copied());
    // Charge live inputs and validator indexes before reference validation grows them.
    Budget::dataset_bytes_iter(datasets.clone(), base)?;
    let ids = support::validate_current(dataset, view)?;
    let result_bytes = checked_sum([
        size_of::<ComputedPearson>(),
        view.heap_bytes()?,
        support::snapshot_bytes(dataset, &ids)?,
        checked_product(pairs, checked_sum([size_of::<CorrelationRow>(), 256])?)?,
    ])?;
    let work = checked_sum([
        checked_product(dataset.row_count(), size_of::<Eligible>())?,
        // Three raw/normalized/cross moment pairs coexist while computing a pair.
        checked_product(8, size_of::<Moments>())?,
        size_of::<inference::Inference>(),
        size_of::<BoundedSum>(),
    ])?;
    // Keep validator storage live while reserving snapshots, cloned request, results and selection.
    Budget::dataset_bytes_iter(datasets, checked_sum([base, result_bytes, work])?)?;
    let total = progress_total(dataset.row_count(), pairs)?;
    let complete_columns = if request.missing_policy == CorrelationMissingPolicy::CompleteCases {
        request.columns.as_slice()
    } else {
        &[]
    };
    let (eligible, diagnostics) = select_rows(
        dataset,
        &request.filter,
        request.frequency_weight,
        complete_columns,
        control,
        total,
    )?;
    let mut rows = Vec::with_capacity(pairs);
    for (i, first) in request.columns.iter().enumerate() {
        for second in &request.columns[i + 1..] {
            let progress = dataset.row_count() * (1 + 4 * rows.len());
            rows.push(pair(
                dataset.column(*first)?,
                dataset.column(*second)?,
                &eligible,
                control,
                progress,
                total,
            )?);
        }
    }
    let columns = support::capture_columns(dataset, &ids, control)?;
    control.checkpoint(total, Some(total))?;
    Ok(ComputedPearson {
        revision: dataset.revision(),
        request: request.clone(),
        columns,
        diagnostics,
        rows,
    })
}
// Progress counts visits, not owned bytes. Every later offset is bounded by this total.
fn progress_total(rows: usize, pairs: usize) -> AppResult<usize> {
    pairs
        .checked_mul(4)
        .and_then(|passes| passes.checked_add(1))
        .and_then(|passes| rows.checked_mul(passes))
        .ok_or_else(|| AppError::invalid("Pearson", "Progress count exceeds its numeric range."))
}
fn values(first: &Column, second: &Column, row: usize) -> AppResult<Option<(f64, f64)>> {
    if first.is_missing(row)? || second.is_missing(row)? {
        return Ok(None);
    }
    match (first.cell(row)?, second.cell(row)?) {
        (Cell::Number(x), Cell::Number(y)) => Ok(Some((x, y))),
        _ => Err(AppError::invalid("Pearson", "Select numeric variables.")),
    }
}
fn pair(
    first: &Column,
    second: &Column,
    eligible: &[Eligible],
    control: &JobControl,
    progress: usize,
    total: usize,
) -> AppResult<CorrelationRow> {
    let (mut x, mut y) = (Moments::default(), Moments::default());
    let mut bounds = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
    let mut anchor = None;
    let mut endpoint = None;
    let (mut missing_rows, mut complete_case_excluded_rows) = (0, 0);
    for (i, entry) in eligible.iter().enumerate() {
        if i.is_multiple_of(4096) {
            control.checkpoint(progress + i, Some(total))?;
        }
        let Some((a, b)) = values(first, second, entry.row)? else {
            missing_rows += 1;
            continue;
        };
        if !entry.complete {
            complete_case_excluded_rows += 1;
            continue;
        }
        x.add(a, entry.weight);
        y.add(b, entry.weight);
        if a < bounds[0][0] {
            anchor = Some([a, b]);
        }
        if a > bounds[0][1] {
            endpoint = Some([a, b]);
        }
        for (range, value) in bounds.iter_mut().zip([a, b]) {
            range[0] = range[0].min(value);
            range[1] = range[1].max(value);
        }
    }
    let mut inference = inference::Inference::new(
        if x.n == 0 { [[0.; 2]; 2] } else { bounds },
        anchor.unwrap_or([0.; 2]),
        endpoint.unwrap_or([0.; 2]),
    );
    let (mut mx, mut my) = (Moments::default(), Moments::default());
    for (i, entry) in eligible.iter().enumerate() {
        if i.is_multiple_of(4096) {
            control.checkpoint(progress + first.len() + i, Some(total))?;
        }
        if !entry.complete {
            continue;
        }
        if let Some((a, b)) = values(first, second, entry.row)? {
            mx.add(x.normalized(a), entry.weight);
            my.add(y.normalized(b), entry.weight);
            inference.add_means(a, b, entry.weight);
        }
    }
    inference.center(x.n);
    let (cx, cy) = (mx.mean().unwrap_or(0.), my.mean().unwrap_or(0.));
    let (mut xx, mut yy, mut xy) = (Moments::default(), Moments::default(), Moments::default());
    for (i, entry) in eligible.iter().enumerate() {
        if i.is_multiple_of(4096) {
            control.checkpoint(progress + 2 * first.len() + i, Some(total))?;
        }
        if !entry.complete {
            continue;
        }
        if let Some((a, b)) = values(first, second, entry.row)? {
            let dx = x.normalized(a) - cx;
            let dy = y.normalized(b) - cy;
            xx.add(dx * dx, entry.weight);
            yy.add(dy * dy, entry.weight);
            xy.add(dx * dy, entry.weight);
            inference.add_cross(a, b, entry.weight);
        }
    }
    let mut r = if x.n < 2 {
        Estimate::unavailable("At least two observations are required.")
    } else if xx.mean() == Some(0.) || yy.mean() == Some(0.) {
        Estimate::unavailable("Both variables must vary.")
    } else {
        Estimate::available(bounded_r(
            // Range normalization bounds each variance by 4 and, when nonzero,
            // by at least 1/(4N). Their product is representable at N<=2^53.
            // One square root preserves equal covariance/variance identities.
            xy.mean().unwrap() / (xx.mean().unwrap() * yy.mean().unwrap()).sqrt(),
        )?)
    };
    for (i, entry) in eligible.iter().enumerate() {
        if i.is_multiple_of(4096) {
            control.checkpoint(progress + 3 * first.len() + i, Some(total))?;
        }
        if entry.complete {
            if let Some((a, b)) = values(first, second, entry.row)? {
                inference.add_residual(a, b, entry.weight)?;
            }
        }
    }
    let mut p = match r.value {
        None => Estimate::unavailable("Correlation is unavailable."),
        Some(_) if x.n < 3 => {
            Estimate::unavailable("At least three observations are required for inference.")
        }
        Some(_) => inference.probability(x.n, control)?,
    };
    if inference.is_perfect() && p.value == Some(0.) {
        r = Estimate::available(r.value.unwrap().signum());
    } else if p.value == Some(0.) {
        p = Estimate::unavailable(inference::UNRESOLVED);
    }
    control.checkpoint(progress + 4 * first.len(), Some(total))?;
    Ok(CorrelationRow {
        first: first.id(),
        second: second.id(),
        n: x.n,
        r,
        p,
        missing_rows,
        complete_case_excluded_rows,
    })
}
fn bounded_r(r: f64) -> AppResult<f64> {
    // Exact weighted sums leave only rounded centering, products, a root and division.
    // 32 epsilons conservatively cover their dimensionless accumulated rounding.
    if !r.is_finite() || r.abs() > 1. + 32. * f64::EPSILON {
        Err(AppError::invalid(
            "Pearson",
            "Correlation exceeds its numerical range.",
        ))
    } else {
        Ok(r.clamp(-1., 1.))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_count_checks_usize_overflow_without_a_byte_limit() {
        assert_eq!(progress_total(15_625, 8128).unwrap(), 508_015_625);
        assert_eq!(progress_total(0, 8128).unwrap(), 0);
        assert_eq!(progress_total(7, 0).unwrap(), 7);
        assert!(progress_total(1, usize::MAX).is_err());
        assert!(progress_total(1, usize::MAX / 4 + 1).is_err());
        assert!(progress_total(usize::MAX, 1).is_err());
    }
    #[test]
    fn rounding_excursion_is_bounded() {
        assert_eq!(bounded_r(1. + 4. * f64::EPSILON).unwrap(), 1.);
        assert!(bounded_r(1. + 64. * f64::EPSILON).is_err());
        assert!(bounded_r(f64::NAN).is_err());
    }
    #[test]
    fn cancellation_during_pair_visits() {
        let d = Dataset::new(vec![
            Column::numeric(crate::data::ColumnId(1), "X", vec![Some(1.); 8193]).unwrap(),
            Column::numeric(crate::data::ColumnId(2), "Y", vec![Some(2.); 8193]).unwrap(),
        ])
        .unwrap();
        let r = PearsonRequest {
            columns: vec![crate::data::ColumnId(1), crate::data::ColumnId(2)],
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: super::super::RowFilter::default(),
            frequency_weight: None,
        };
        let c = JobControl::default();
        let hook_control = c.clone();
        crate::operation::CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |p| {
                if p.done > 8193 {
                    hook_control.cancel();
                }
            }))
        });
        let result = pearson_with_budget(
            &d,
            &r,
            &c,
            &[],
            AnalysisRequestRef::Pearson(&r).heap_bytes().unwrap(),
        );
        crate::operation::CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
    }
    #[test]
    fn cancellation_during_direct_residual_visits() {
        let d = Dataset::new(vec![
            Column::numeric(
                crate::data::ColumnId(1),
                "X",
                (0..4097).map(|i| Some((i % 3) as f64)).collect(),
            )
            .unwrap(),
            Column::numeric(
                crate::data::ColumnId(2),
                "Y",
                (0..4097)
                    .map(|i| Some((i % 3) as f64 + (i % 2) as f64 * 1e-7))
                    .collect(),
            )
            .unwrap(),
        ])
        .unwrap();
        let r = PearsonRequest {
            columns: vec![crate::data::ColumnId(1), crate::data::ColumnId(2)],
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: super::super::RowFilter::default(),
            frequency_weight: None,
        };
        let c = JobControl::default();
        let hook_control = c.clone();
        crate::operation::CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |p| {
                if p.done >= 4 * 4097 {
                    hook_control.cancel();
                }
            }))
        });
        let result = pearson_with_budget(&d, &r, &c, &[], 0);
        crate::operation::CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
    }
    #[test]
    fn wide_progress_reaches_selection_within_allocation_budget() {
        let d = Dataset::new(
            (1..=128)
                .map(|i| {
                    Column::numeric(
                        crate::data::ColumnId(i),
                        &format!("V{i}"),
                        vec![Some(1.); 15_625],
                    )
                    .unwrap()
                })
                .collect(),
        )
        .unwrap();
        let r = PearsonRequest {
            columns: (1..=128).map(crate::data::ColumnId).collect(),
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: super::super::RowFilter::default(),
            frequency_weight: None,
        };
        let total = 508_015_625;
        assert!(total > crate::data::MAX_OWNED_BYTES);
        let c = JobControl::default();
        let hook_control = c.clone();
        crate::operation::CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |p| {
                assert!(p.done <= total);
                assert_eq!(p.total, Some(total));
                hook_control.cancel();
            }));
        });
        let result = pearson_with_budget(
            &d,
            &r,
            &c,
            &[],
            AnalysisRequestRef::Pearson(&r).heap_bytes().unwrap(),
        );
        crate::operation::CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)), "{result:?}");
    }
    #[test]
    fn pair_capacity_refusal_precedes_selection_and_result_growth() {
        use std::{cell::Cell as Flag, rc::Rc};
        let columns = (1..=1800)
            .map(|i| Column::numeric(crate::data::ColumnId(i), &format!("V{i}"), vec![]).unwrap())
            .collect();
        let d = Dataset::new(columns).unwrap();
        let r = PearsonRequest {
            columns: (1..=1800).map(crate::data::ColumnId).collect(),
            missing_policy: CorrelationMissingPolicy::PairwiseComplete,
            filter: super::super::RowFilter::default(),
            frequency_weight: None,
        };
        let visited = Rc::new(Flag::new(false));
        let hook_visited = Rc::clone(&visited);
        crate::operation::CHECKPOINT_HOOK
            .with(|h| *h.borrow_mut() = Some(Box::new(move |_| hook_visited.set(true))));
        let result = pearson_with_budget(
            &d,
            &r,
            &JobControl::default(),
            &[],
            AnalysisRequestRef::Pearson(&r).heap_bytes().unwrap(),
        );
        crate::operation::CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert!(!visited.get());
    }
}
