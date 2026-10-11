use super::*;
use crate::{
    analysis::{validation::invalid_analysis, RowPredicate},
    data::{ColumnId, ColumnKind},
    error::AppResult,
    operation::JobControl,
};
use std::collections::BTreeSet;
impl ReliabilityRequest {
    pub fn admit(&self) -> AppResult<()> {
        self.admit_core(None)
    }
    pub(crate) fn admit_controlled(&self, c: &JobControl) -> AppResult<()> {
        self.admit_core(Some(c))
    }
    fn admit_core(&self, c: Option<&JobControl>) -> AppResult<()> {
        if let Some(c) = c {
            c.check_cancelled()?;
        }
        self.charge(c)?;
        if !(2..=64).contains(&self.items.len()) {
            return Err(invalid_analysis());
        }
        for (i, id) in self.items.iter().enumerate() {
            if id.0 == 0 || self.items[..i].contains(id) {
                return Err(invalid_analysis());
            }
        }
        for (i, r) in self.reversals.iter().enumerate() {
            if !self.items.contains(&r.column)
                || self.reversals[..i].iter().any(|p| p.column == r.column)
                || !r.lower.is_finite()
                || !r.upper.is_finite()
                || r.lower >= r.upper
            {
                return Err(invalid_analysis());
            }
        }
        for (i, p) in self.filter.predicates.iter().enumerate() {
            if i.is_multiple_of(64) {
                if let Some(c) = c {
                    c.checkpoint(i, None)?;
                }
            }
            if p.column().0 == 0
                || matches!(p,RowPredicate::Number {value,..} if !value.is_finite())
            {
                return Err(invalid_analysis());
            }
        }
        if self.frequency_weight.is_some_and(|id| id.0 == 0) {
            return Err(invalid_analysis());
        }
        Ok(())
    }
}
/// Role indexes are precharged by the caller, independently of the source owner.
pub(crate) fn request(
    r: &ReliabilityRequest,
    mut kind: impl FnMut(ColumnId) -> AppResult<ColumnKind>,
    c: &JobControl,
) -> AppResult<BTreeSet<ColumnId>> {
    c.check_cancelled()?;
    r.admit_controlled(c)?;
    let mut ids = BTreeSet::new();
    for id in &r.items {
        c.checkpoint(0, None)?;
        if kind(*id)? != ColumnKind::Number {
            return Err(invalid_analysis());
        }
        ids.insert(*id);
    }
    for (i, p) in r.filter.predicates.iter().enumerate() {
        if i.is_multiple_of(64) {
            c.checkpoint(i, None)?;
        }
        let k = kind(p.column())?;
        if matches!(p,RowPredicate::Number {..} if k!=ColumnKind::Number)
            || matches!(p,RowPredicate::Text {..} if k!=ColumnKind::Text)
        {
            return Err(invalid_analysis());
        }
        ids.insert(p.column());
    }
    if let Some(id) = r.frequency_weight {
        if kind(id)? != ColumnKind::Number {
            return Err(invalid_analysis());
        }
        ids.insert(id);
    }
    c.check_cancelled()?;
    Ok(ids)
}
