use super::*;
use crate::{
    analysis::{
        model::{checked_product, checked_sum},
        selection::Eligible,
        RowPredicate,
    },
    data::{Budget, Column, Dataset, MissingRules, MAX_CELLS},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::mem::size_of;
impl ReliabilityRequest {
    pub fn heap_bytes(&self) -> AppResult<usize> {
        self.heap_bytes_controlled(None)
    }
    pub(crate) fn heap_bytes_controlled(&self, c: Option<&JobControl>) -> AppResult<usize> {
        let mut bytes = checked_sum([
            checked_product(self.items.capacity(), size_of::<crate::data::ColumnId>())?,
            checked_product(self.reversals.capacity(), size_of::<ReliabilityReverse>())?,
            checked_product(self.filter.predicates.capacity(), size_of::<RowPredicate>())?,
        ])?;
        for (i, p) in self.filter.predicates.iter().enumerate() {
            if i.is_multiple_of(64) {
                if let Some(c) = c {
                    c.checkpoint(i, None)?;
                }
            }
            if let RowPredicate::Text { value, .. } = p {
                bytes = checked_sum([bytes, value.capacity()])?;
            }
        }
        Ok(bytes)
    }
    pub(super) fn charge(&self, c: Option<&JobControl>) -> AppResult<()> {
        Budget::check_datasets(
            &[],
            checked_sum([size_of::<Self>(), self.heap_bytes_controlled(c)?])?,
        )
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        checked_product(
            checked_sum([
                self.items.len(),
                self.filter.predicates.len(),
                usize::from(self.frequency_weight.is_some()),
            ])?,
            1024,
        )
    }
    pub fn validate_current(
        &self,
        d: &Dataset,
        retained: &[&Dataset],
        extra: usize,
        c: &JobControl,
    ) -> AppResult<()> {
        c.check_cancelled()?;
        self.admit_controlled(c)?;
        Budget::dataset_bytes_iter(
            std::iter::once(d).chain(retained.iter().copied()),
            checked_sum([
                extra,
                size_of::<Self>(),
                self.heap_bytes_controlled(Some(c))?,
                self.validation_scratch_bytes()?,
            ])?,
        )?;
        super::validation::request(self, |id| Ok(d.column(id)?.kind()), c).map(|_| ())
    }
}
impl ReliabilityCohort {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        checked_sum([
            size_of::<Self>(),
            checked_product(self.eligible.capacity(), size_of::<Eligible>())?,
        ])
    }
}
// Source-work accounting uses usize overflow checks, not the byte budget as a CPU limit.
fn work_sum(values: impl IntoIterator<Item = usize>) -> AppResult<usize> {
    values.into_iter().try_fold(0usize, |n, v| {
        n.checked_add(v)
            .ok_or_else(crate::analysis::validation::invalid_analysis)
    })
}
fn work_product(a: usize, b: usize) -> AppResult<usize> {
    a.checked_mul(b)
        .ok_or_else(crate::analysis::validation::invalid_analysis)
}
fn missing_work(col: &Column, c: &JobControl) -> AppResult<usize> {
    match &col.metadata().missing_rules {
        MissingRules::None => Ok(1),
        MissingRules::Number { values, ranges } => {
            work_sum([1, values.len(), work_product(2, ranges.len())?])
        }
        MissingRules::Text(values) => {
            let mut n = 1;
            for (i, s) in values.iter().enumerate() {
                if i.is_multiple_of(64) {
                    c.checkpoint(i, None)?;
                }
                n = work_sum([n, 1, s.len()])?;
            }
            Ok(n)
        }
    }
}
/// Checked visits/bytes bound source loops without imposing a new runtime cap.
pub(super) fn source_work(d: &Dataset, r: &ReliabilityRequest, c: &JobControl) -> AppResult<usize> {
    c.check_cancelled()?;
    let cells = work_product(d.row_count(), r.items.len())?;
    if cells > MAX_CELLS {
        return Err(AppError::SizeLimit { limit: MAX_CELLS });
    }
    let mut per = 1;
    for (i, p) in r.filter.predicates.iter().enumerate() {
        if i.is_multiple_of(64) {
            c.checkpoint(i, None)?;
        }
        per = work_sum([
            per,
            missing_work(d.column(p.column())?, c)?,
            1,
            if let RowPredicate::Text { value, .. } = p {
                value.len()
            } else {
                0
            },
        ])?;
    }
    if let Some(id) = r.frequency_weight {
        per = work_sum([per, missing_work(d.column(id)?, c)?, 2])?;
    }
    for id in &r.items {
        per = work_sum([per, missing_work(d.column(*id)?, c)?])?;
    }
    for reverse in &r.reversals {
        per = work_sum([per, missing_work(d.column(reverse.column)?, c)?, 2])?;
    }
    work_product(d.row_count(), per)
}

pub(super) struct NumericalAdmission {
    pub work: usize,
    pub bytes: usize,
}
/// Actual kernel/result/conversion owners, borrowed recipe/cohort and source are simultaneous.
pub(super) fn numerical_admission(
    d: &Dataset,
    r: &ReliabilityRequest,
    cohort: &ReliabilityCohort,
    c: &JobControl,
) -> AppResult<NumericalAdmission> {
    use super::arithmetic::Interval;
    use crate::analysis::{
        moments::{BoundedSum, Moments, Scaled},
        Estimate,
    };
    let k = r.items.len();
    let rows = cohort.eligible.len();
    let complete = cohort.complete_rows;
    if work_product(d.row_count(), k)? > MAX_CELLS {
        return Err(AppError::SizeLimit { limit: MAX_CELLS });
    }
    let kernel = checked_sum([
        checked_product(k, 9 * size_of::<BoundedSum>())?,
        checked_product(k, 4 * size_of::<Interval>())?,
        size_of::<Interval>(),
        checked_product(k, 2 * size_of::<Option<i32>>())?,
        checked_product(k, 5 * size_of::<Estimate>())?,
        checked_product(k, 3 * size_of::<ReliabilityEnergyState>())?,
        checked_product(k, size_of::<bool>())?,
        checked_product(k, 2 * size_of::<Scaled>())?,
    ])?;
    let early = checked_product(
        k,
        size_of::<Moments>() + size_of::<BoundedSum>() + size_of::<Estimate>(),
    )?;
    let bytes = checked_sum([
        kernel
            .max(early)
            .max(checked_sum([checked_product(5420, k)?, 32])?),
        6640,
        checked_product(k, size_of::<f64>())?,
        checked_product(k, size_of::<(&Column, Option<&ReliabilityReverse>)>())?,
        checked_product(k, size_of::<ReliabilityComputedItem>())?,
        size_of::<ReliabilityComputed>(),
        checked_product(5 * k + 2, 128)?, // All fixed publication reasons are below 128 UTF-8 bytes.
        size_of::<ReliabilityRequest>(),
        r.heap_bytes_controlled(Some(c))?,
        cohort.owned_bytes()?,
    ])?;
    Budget::check_datasets(&[d], bytes)?;
    // Numerical kernel bound, source scans/scoring, two state scans and linear finalization.
    let work = work_sum([
        work_product(work_product(10, complete)?, k)?,
        work_product(5, k)?,
        work_product(2, work_product(k, k)?)?,
        work_product(6, rows)?, // consistency + four passes + first complete-row search
        work_product(
            work_sum([work_product(4, complete)?, 1])?,
            work_product(k, 4)?,
        )?, // score + three reversed exact pushes
        work_product(k, work_sum([d.column_count(), r.reversals.len()])?)?,
        work_product(32, k)?,
    ])?;
    Ok(NumericalAdmission { work, bytes })
}

fn checkpoint(c: Option<&JobControl>, index: usize) -> AppResult<()> {
    if let Some(c) = c {
        c.checkpoint(index, None)?;
    }
    Ok(())
}

impl ReliabilityRun {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        self.owned_bytes_controlled(None)
    }
    pub(crate) fn owned_bytes_controlled(&self, c: Option<&JobControl>) -> AppResult<usize> {
        checkpoint(c, 0)?;
        let mut n = checked_sum([
            size_of::<Self>(),
            self.request.heap_bytes_controlled(c)?,
            checked_product(
                self.columns.capacity(),
                size_of::<crate::analysis::RunColumnInfo>(),
            )?,
            checked_product(self.items.capacity(), size_of::<ReliabilityItem>())?,
        ])?;
        for (i, info) in self.columns.iter().enumerate() {
            if i.is_multiple_of(64) {
                checkpoint(c, i)?;
            }
            n = checked_sum([
                n,
                crate::analysis::support::metadata_heap_bytes(
                    &info.name,
                    &info.description,
                    &info.missing_rules,
                    &info.value_labels,
                    c,
                )?,
            ])?;
        }
        for item in &self.items {
            checkpoint(c, 0)?;
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
        for e in [&self.raw_alpha, &self.standardized_alpha] {
            n = checked_sum([n, e.reason.as_ref().map_or(0, String::capacity)])?;
        }
        checkpoint(c, 0)?;
        Ok(n)
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        self.scratch_controlled(None)
    }
    pub(crate) fn scratch_controlled(&self, c: Option<&JobControl>) -> AppResult<usize> {
        let mut labels = 0;
        for (i, info) in self.columns.iter().enumerate() {
            if i.is_multiple_of(64) {
                checkpoint(c, i)?;
            }
            labels = labels.max(crate::analysis::validation::snapshot_scratch_bytes(info)?);
        }
        checked_sum([
            self.request.validation_scratch_bytes()?,
            checked_product(self.columns.len(), 1024)?,
            labels,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{ColumnId, ColumnMetadata, MAX_OWNED_BYTES};
    #[test]
    fn source_work_above_byte_budget_is_admitted_and_usize_overflow_is_rejected() {
        let col = Column::numeric(ColumnId(1), "large rules", vec![Some(500000.); 1000]).unwrap();
        let col = col
            .with_metadata(ColumnMetadata {
                missing_rules: MissingRules::Number {
                    values: vec![0.; 400000],
                    ranges: vec![],
                },
                ..col.metadata().clone()
            })
            .unwrap();
        let d = Dataset::new(vec![
            col,
            Column::numeric(ColumnId(2), "other", vec![Some(1.); 1000]).unwrap(),
        ])
        .unwrap();
        let r = ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2)],
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![],
        };
        assert!(source_work(&d, &r, &JobControl::default()).unwrap() > MAX_OWNED_BYTES);
        assert!(work_product(usize::MAX, 2).is_err());
        assert!(work_sum([usize::MAX, 1]).is_err());
    }
    #[test]
    fn controlled_cancel_and_extra_budget_reject_before_role_indexes_allocate() {
        let d = Dataset::new(vec![
            Column::numeric(ColumnId(1), "a", vec![]).unwrap(),
            Column::numeric(ColumnId(2), "b", vec![]).unwrap(),
        ])
        .unwrap();
        let r = ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2)],
            filter: Default::default(),
            frequency_weight: None,
            reversals: vec![],
        };
        let c = JobControl::default();
        c.cancel();
        let (result, largest) =
            crate::data::measure_test_allocations(|| capture_cohort(&d, &r, &c));
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(largest, 0);
        let (result, largest) = crate::data::measure_test_allocations(|| {
            r.validate_current(&d, &[], MAX_OWNED_BYTES, &JobControl::default())
        });
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, JobControl::DEFAULT_HEAP_BYTES);
    }
}
