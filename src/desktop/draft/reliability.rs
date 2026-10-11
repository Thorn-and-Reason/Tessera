//! Exact saved bounds use round-trip decimal text while edits remain explicit.
use crate::{
    analysis::*,
    data::{AllocationCounter, Budget, ColumnId, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::mem::size_of;
#[derive(Debug, Clone, PartialEq)]
pub struct ReverseDraft {
    pub column: ColumnId,
    pub lower: String,
    pub upper: String,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReliabilityDraft {
    pub items: Vec<ColumnId>,
    pub reversals: Vec<ReverseDraft>,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
fn bound(text: &str) -> AppResult<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| AppError::invalid("Reversal bounds", "Enter finite lower and upper bounds."))
}
impl ReverseDraft {
    fn request(&self) -> AppResult<ReliabilityReverse> {
        Ok(ReliabilityReverse {
            column: self.column,
            lower: bound(&self.lower)?,
            upper: bound(&self.upper)?,
        })
    }
}
impl ReliabilityDraft {
    pub fn from_request(r: ReliabilityRequest) -> Self {
        Self {
            items: r.items,
            filter: r.filter,
            frequency_weight: r.frequency_weight,
            reversals: r
                .reversals
                .into_iter()
                .map(|r| ReverseDraft {
                    column: r.column,
                    lower: r.lower.to_string(),
                    upper: r.upper.to_string(),
                })
                .collect(),
        }
    }
    pub fn conversion_bytes(r: &ReliabilityRequest) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.array::<ReverseDraft>(r.reversals.len())?;
        n.array::<[u8; 1024]>(r.reversals.len())?;
        Ok(n.bytes)
    }
    pub fn heap_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.array::<ColumnId>(self.items.capacity())?;
        n.array::<ReverseDraft>(self.reversals.capacity())?;
        n.array::<RowPredicate>(self.filter.predicates.capacity())?;
        for r in &self.reversals {
            n.add(r.lower.capacity())?;
            n.add(r.upper.capacity())?;
        }
        for p in &self.filter.predicates {
            if let RowPredicate::Text { value, .. } = p {
                n.add(value.capacity())?;
            }
        }
        Ok(n.bytes)
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.array::<[u8; 1024]>(
            self.items
                .len()
                .checked_add(self.filter.predicates.len())
                .and_then(|n| n.checked_add(usize::from(self.frequency_weight.is_some())))
                .ok_or(AppError::SizeLimit {
                    limit: crate::data::MAX_OWNED_BYTES,
                })?,
        )?;
        Ok(n.bytes)
    }
    pub fn matches_saved(&self, r: &ReliabilityRequest) -> bool {
        self.items == r.items
            && self.filter == r.filter
            && self.frequency_weight == r.frequency_weight
            && self.reversals.len() == r.reversals.len()
            && self
                .reversals
                .iter()
                .zip(&r.reversals)
                .all(|(a, b)| a.request().is_ok_and(|v| v == *b))
    }
    pub fn build_request(&self, data: &Dataset) -> AppResult<ReliabilityRequest> {
        let heap = self.heap_bytes()?;
        if heap > super::super::state::MAX_DRAFT_BYTES {
            return Err(AppError::SizeLimit {
                limit: super::super::state::MAX_DRAFT_BYTES,
            });
        }
        let mut peak = AllocationCounter::default();
        peak.add(heap)?;
        peak.add(heap)?;
        peak.add(self.validation_scratch_bytes()?)?;
        peak.add(JobControl::DEFAULT_HEAP_BYTES)?;
        peak.add(size_of::<ReliabilityRequest>())?;
        Budget::check_datasets(&[data], peak.bytes)?;
        if !(2..=64).contains(&self.items.len())
            || self
                .items
                .iter()
                .enumerate()
                .any(|(i, id)| self.items[..i].contains(id))
        {
            return Err(AppError::invalid(
                "Items",
                "Select 2 to 64 distinct numeric items.",
            ));
        }
        for (i, r) in self.reversals.iter().enumerate() {
            let scored = r.request()?;
            if !self.items.contains(&r.column)
                || self.reversals[..i].iter().any(|p| p.column == r.column)
                || scored.lower >= scored.upper
            {
                return Err(AppError::invalid(
                    "Reversal bounds",
                    "Reverse selected items once, with lower bound below upper bound.",
                ));
            }
        }
        let r = ReliabilityRequest {
            items: self.items.clone(),
            filter: self.filter.clone(),
            frequency_weight: self.frequency_weight,
            reversals: self
                .reversals
                .iter()
                .map(ReverseDraft::request)
                .collect::<AppResult<_>>()?,
        };
        r.validate_current(data, &[], heap + r.heap_bytes()?, &JobControl::default())?;
        Ok(r)
    }
}
#[cfg(test)]
mod tests;
