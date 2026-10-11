//! Standalone regression settings; categorical references retain their raw typed identity.
use crate::{
    analysis::*,
    data::{AllocationCounter, Budget, Cell, ColumnId, Dataset},
    error::{AppError, AppResult},
};

#[derive(Debug, Clone, PartialEq)]
pub struct RegressionDraft {
    pub outcome: Option<ColumnId>,
    pub predictors: Vec<RegressionPredictorDraft>,
    pub confidence: ConfidenceLevel,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionPredictorDraft {
    pub column: ColumnId,
    pub role: RegressionDraftRole,
}
#[derive(Debug, Clone, PartialEq)]
pub enum RegressionDraftRole {
    Numeric,
    Categorical { reference: Option<Cell> },
}
#[derive(Debug, PartialEq)]
pub struct GroupRoles<'a> {
    pub column: Option<ColumnId>,
    pub selected: [Option<&'a Cell>; 2],
}
impl Default for RegressionDraft {
    fn default() -> Self {
        Self {
            outcome: None,
            predictors: vec![],
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        }
    }
}
impl RegressionDraft {
    pub fn build_request(&self, dataset: &Dataset) -> AppResult<RegressionRequest> {
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
        Budget::check_datasets(&[dataset], peak.bytes)?;
        let outcome = super::required(self.outcome)?;
        // Check missing references before any predictor or string copy.
        if self
            .predictors
            .iter()
            .any(|p| matches!(p.role, RegressionDraftRole::Categorical { reference: None }))
        {
            return Err(AppError::invalid(
                "reference level",
                "Select a reference level for each categorical predictor.",
            ));
        }
        let request = RegressionRequest {
            outcome,
            predictors: self
                .predictors
                .iter()
                .map(|p| RegressionPredictor {
                    column: p.column,
                    role: match &p.role {
                        RegressionDraftRole::Numeric => PredictorRole::Numeric,
                        RegressionDraftRole::Categorical {
                            reference: Some(reference),
                        } => PredictorRole::Categorical {
                            reference: reference.clone(),
                        },
                        RegressionDraftRole::Categorical { reference: None } => unreachable!(),
                    },
                })
                .collect(),
            confidence: self.confidence,
            filter: self.filter.clone(),
            frequency_weight: self.frequency_weight,
        };
        let mut live = AllocationCounter::default();
        live.add(heap)?;
        live.add(request.heap_bytes()?)?;
        request.validate_current(dataset, &[], live.bytes)?;
        Ok(request)
    }
    /// The caller admits the captured request and draft-vector overlap before conversion.
    pub fn from_request(request: RegressionRequest) -> Self {
        Self {
            outcome: Some(request.outcome),
            predictors: request
                .predictors
                .into_iter()
                .map(|p| RegressionPredictorDraft {
                    column: p.column,
                    role: match p.role {
                        PredictorRole::Numeric => RegressionDraftRole::Numeric,
                        PredictorRole::Categorical { reference } => {
                            RegressionDraftRole::Categorical {
                                reference: Some(reference),
                            }
                        }
                    },
                })
                .collect(),
            confidence: request.confidence,
            filter: request.filter,
            frequency_weight: request.frequency_weight,
        }
    }
    pub fn matches_saved(&self, saved: &RegressionRequest) -> bool {
        self.outcome == Some(saved.outcome)
            && self.confidence == saved.confidence
            && self.filter == saved.filter
            && self.frequency_weight == saved.frequency_weight
            && self.predictors.len() == saved.predictors.len()
            && self.predictors.iter().zip(&saved.predictors).all(|(a, b)| {
                a.column == b.column
                    && match (&a.role, &b.role) {
                        (RegressionDraftRole::Numeric, PredictorRole::Numeric) => true,
                        (
                            RegressionDraftRole::Categorical { reference },
                            PredictorRole::Categorical { reference: saved },
                        ) => reference.as_ref() == Some(saved),
                        _ => false,
                    }
            })
    }
    pub fn heap_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.array::<RegressionPredictorDraft>(self.predictors.capacity())?;
        n.array::<RowPredicate>(self.filter.predicates.capacity())?;
        for p in &self.predictors {
            if let RegressionDraftRole::Categorical {
                reference: Some(Cell::Text(s)),
            } = &p.role
            {
                n.add(s.capacity())?;
            }
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
        for roles in [
            1,
            self.predictors.len(),
            self.filter.predicates.len(),
            usize::from(self.frequency_weight.is_some()),
        ] {
            n.array::<[u8; 1024]>(roles)?;
        }
        Ok(n.bytes)
    }
    pub fn group_roles(&self, active: Option<ColumnId>) -> Option<GroupRoles<'_>> {
        let p = self.predictors.iter().find(|p| Some(p.column) == active)?;
        if let RegressionDraftRole::Categorical { reference } = &p.role {
            Some(GroupRoles {
                column: Some(p.column),
                selected: [reference.as_ref(), None],
            })
        } else {
            None
        }
    }
}
#[cfg(test)]
mod tests;
