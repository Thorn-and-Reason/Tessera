//! Checked capacities are charged before indexes, snapshots or captured copies.
use super::*;
use crate::{
    analysis::{
        model::{checked_product, checked_sum},
        RowPredicate,
    },
    data::{Budget, Cell, ColumnId, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::mem::size_of;
pub(super) fn array<T>(capacity: usize) -> AppResult<usize> {
    checked_product(capacity, size_of::<T>())
}
pub(super) fn cell_bytes(c: &Cell) -> usize {
    if let Cell::Text(s) = c {
        s.capacity()
    } else {
        0
    }
}
fn scope(s: &AnovaScope) -> AppResult<usize> {
    match s {
        AnovaScope::Marginal => Ok(0),
        AnovaScope::Fixed(v) => checked_sum(
            std::iter::once(array::<Cell>(v.capacity())?).chain(v.iter().map(cell_bytes)),
        ),
    }
}
fn pair(p: &AnovaPairRequest) -> AppResult<usize> {
    checked_sum([
        cell_bytes(&p.first),
        cell_bytes(&p.second),
        scope(&p.scope)?,
    ])
}
fn estimate(e: &crate::analysis::Estimate) -> usize {
    e.reason.as_ref().map_or(0, String::capacity)
}
impl AnovaRequest {
    pub fn heap_bytes(&self) -> AppResult<usize> {
        let design = match &self.design {
            AnovaDesign::OneWay { .. } => 0,
            AnovaDesign::Factorial { factors, .. } => array::<ColumnId>(factors.capacity())?,
            AnovaDesign::Repeated {
                factors,
                measurements,
                ..
            } => {
                let mut n = checked_sum([
                    array::<AnovaWithinFactor>(factors.capacity())?,
                    array::<ColumnId>(measurements.capacity())?,
                ])?;
                for f in factors {
                    n = checked_sum([n, f.name.capacity(), array::<String>(f.levels.capacity())?])?;
                    for s in &f.levels {
                        n = checked_sum([n, s.capacity()])?;
                    }
                }
                n
            }
        };
        let mut n = checked_sum([
            design,
            array::<RowPredicate>(self.filter.predicates.capacity())?,
            array::<AnovaPairRequest>(self.planned_pairs.capacity())?,
            array::<AnovaFamilyRequest>(self.all_pairs.capacity())?,
        ])?;
        for p in &self.filter.predicates {
            if let RowPredicate::Text { value, .. } = p {
                n = checked_sum([n, value.capacity()])?;
            }
        }
        for p in &self.planned_pairs {
            n = checked_sum([n, pair(p)?])?;
        }
        for p in &self.all_pairs {
            n = checked_sum([n, scope(&p.scope)?])?;
        }
        Ok(n)
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        let columns = match &self.design {
            AnovaDesign::OneWay { .. } => 2,
            AnovaDesign::Factorial { factors, .. } => checked_sum([1, factors.len()])?,
            AnovaDesign::Repeated { measurements, .. } => checked_sum([1, measurements.len()])?,
        };
        checked_product(
            checked_sum([
                columns,
                self.filter.predicates.len(),
                usize::from(self.frequency_weight.is_some()),
                self.planned_pairs.len(),
                self.all_pairs.len(),
            ])?,
            1024,
        )
    }
    pub fn validate_current(
        &self,
        dataset: &Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
        control: &JobControl,
    ) -> AppResult<()> {
        control.check_cancelled()?;
        self.admit()?;
        Budget::dataset_bytes_iter(
            std::iter::once(dataset).chain(retained.iter().copied()),
            checked_sum([extra_bytes, self.validation_scratch_bytes()?])?,
        )?;
        validation::request(self, |id| Ok(dataset.column(id)?.kind()), control).map(|_| ())
    }
    pub(crate) fn admit(&self) -> AppResult<()> {
        if self.heap_bytes()? > MAX_REQUEST_BYTES {
            Err(AppError::SizeLimit {
                limit: MAX_REQUEST_BYTES,
            })
        } else {
            Ok(())
        }
    }
}
impl ComputedAnova {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut n = checked_sum([
            size_of::<Self>(),
            self.request.heap_bytes()?,
            array::<crate::analysis::RunColumnInfo>(self.columns.capacity())?,
            array::<AnovaFactorLevels>(self.grid.factors.capacity())?,
            array::<AnovaCell>(self.grid.cells.capacity())?,
            array::<AnovaContrastFamily>(self.families.capacity())?,
        ])?;
        for c in &self.columns {
            n = checked_sum([n, c.heap_bytes()?])?;
        }
        for f in &self.grid.factors {
            n = checked_sum([n, f.name.capacity(), array::<Cell>(f.values.capacity())?])?;
            for v in &f.values {
                n = checked_sum([n, cell_bytes(v)])?;
            }
        }
        for c in &self.grid.cells {
            n = checked_sum([n, estimate(&c.mean)])?;
        }
        n = checked_sum([n, outcome_heap_bytes(&self.outcome)?])?;
        for f in &self.families {
            n = checked_sum([
                n,
                f.adjustment_failure
                    .as_ref()
                    .map_or(0, |v| v.reason.capacity()),
            ])?;
            n = checked_sum([n, array::<AnovaComparison>(f.members.capacity())?])?;
            if let AnovaFamilyKind::AllPairs(r) = &f.kind {
                n = checked_sum([n, scope(&r.scope)?])?;
            }
            for c in &f.members {
                n = checked_sum([n, pair(&c.request)?])?;
                if let AnovaComparisonState::AbsentRequestedLevel(f)
                | AnovaComparisonState::Unavailable(f) = &c.state
                {
                    n = checked_sum([n, f.reason.capacity()])?;
                }
                for e in [
                    &c.estimate,
                    &c.standard_error,
                    &c.t,
                    &c.df,
                    &c.raw_p,
                    &c.adjusted_p,
                    &c.interval[0],
                    &c.interval[1],
                ] {
                    n = checked_sum([n, estimate(e)])?;
                }
            }
        }
        Ok(n)
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        let mut n = checked_sum([
            self.request.validation_scratch_bytes()?,
            array::<usize>(checked_sum([
                self.columns.len(),
                self.grid.cells.len(),
                self.families.len(),
            ])?)?,
        ])?;
        for c in &self.columns {
            n = checked_sum([n, c.heap_bytes()?, 1024])?;
        }
        for f in &self.families {
            n = checked_sum([
                n,
                f.adjustment_failure
                    .as_ref()
                    .map_or(0, |v| v.reason.capacity()),
            ])?;
            n = checked_sum([n, checked_product(f.members.len(), 1024)?])?;
        }
        Ok(n)
    }
    pub fn validate(&self, control: &JobControl) -> AppResult<()> {
        control.check_cancelled()?;
        Budget::check_datasets(
            &[],
            checked_sum([self.owned_bytes()?, self.validation_scratch_bytes()?])?,
        )?;
        validation::completed(self, control)
    }
}
fn energy(e: &AnovaEnergy) -> usize {
    estimate(&e.value)
}
fn test(t: &AnovaTest) -> AppResult<usize> {
    checked_sum([
        estimate(&t.df1),
        estimate(&t.df2),
        estimate(&t.f),
        estimate(&t.p),
    ])
}
/// Borrowed membership pass before constructing any expanded comparison family.
pub(crate) fn family_bytes(
    r: &AnovaRequest,
    factors: &[AnovaFactorLevels],
    control: &JobControl,
) -> AppResult<usize> {
    let families = r.all_pairs.len() + usize::from(!r.planned_pairs.is_empty());
    let mut n = checked_sum([
        array::<AnovaContrastFamily>(families)?,
        checked_product(families, 160)?,
    ])?;
    let mut member = |p: &AnovaPairRequest| -> AppResult<()> {
        n = checked_sum([
            n,
            size_of::<AnovaComparison>(),
            pair(p)?,
            8 * 160 + 160,
            2 * size_of::<usize>(),
        ])?;
        Ok(())
    };
    for p in &r.planned_pairs {
        control.checkpoint(0, None)?;
        member(p)?;
    }
    for request in &r.all_pairs {
        control.checkpoint(0, None)?;
        let values = &factors[request.factor].values;
        let pairs = checked_product(values.len(), values.len().saturating_sub(1))? / 2;
        let scope_bytes = scope(&request.scope)?;
        n = checked_sum([
            n,
            scope_bytes,
            checked_product(
                pairs,
                size_of::<AnovaComparison>() + 8 * 160 + 160 + 2 * size_of::<usize>(),
            )?,
            checked_product(pairs, scope_bytes)?,
        ])?;
        for (i, v) in values.iter().enumerate() {
            control.checkpoint(i, None)?;
            n = checked_sum([
                n,
                checked_product(values.len().saturating_sub(1), cell_bytes(v))?,
            ])?;
        }
    }
    Ok(n)
}

/// Captured outcome heap excludes its inline storage already owned by the result.
pub(super) fn outcome_heap_bytes(outcome: &AnovaOutcome) -> AppResult<usize> {
    let mut bytes = 0;
    match outcome {
        AnovaOutcome::Unavailable(f) => bytes = checked_sum([bytes, f.reason.capacity()])?,
        AnovaOutcome::Fitted(m) => {
            bytes = checked_sum([
                bytes,
                array::<AnovaEffect>(m.effects.capacity())?,
                energy(&m.error_ss),
                energy(&m.model_ss),
                energy(&m.total_ss),
                m.subject_ss.as_ref().map_or(0, energy),
            ])?;
            for e in &m.effects {
                bytes = checked_sum([
                    bytes,
                    array::<usize>(e.factors.capacity())?,
                    energy(&e.ss),
                    energy(&e.error_ss),
                    estimate(&e.ms),
                    estimate(&e.error_ms),
                    test(&e.test)?,
                    estimate(&e.eta_squared),
                    estimate(&e.partial_eta_squared),
                    e.failure.as_ref().map_or(0, |f| f.reason.capacity()),
                ])?;
                if let Some(g) = &e.gg {
                    bytes = checked_sum([
                        bytes,
                        estimate(&g.epsilon),
                        test(&g.test)?,
                        g.failure.as_ref().map_or(0, |f| f.reason.capacity()),
                    ])?;
                }
            }
        }
    }
    Ok(bytes)
}
pub(super) fn outcome_bytes(outcome: &AnovaOutcome) -> AppResult<usize> {
    checked_sum([size_of::<AnovaOutcome>(), outcome_heap_bytes(outcome)?])
}
