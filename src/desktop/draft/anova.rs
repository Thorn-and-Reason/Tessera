//! Editable roles preserve captured typed identities and declared grid order.
use crate::{
    analysis::*,
    data::{AllocationCounter, Budget, Cell, ColumnId, Dataset},
    error::{AppError, AppResult},
};

#[derive(Debug, Clone, PartialEq)]
pub enum AnovaDesignDraft {
    OneWay {
        outcome: Option<ColumnId>,
        factor: Option<ColumnId>,
    },
    Factorial {
        outcome: Option<ColumnId>,
        factors: Vec<ColumnId>,
    },
    Repeated {
        subject: Option<ColumnId>,
        factors: Vec<AnovaWithinFactor>,
        measurements: Vec<Option<ColumnId>>,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaDraft {
    pub design: AnovaDesignDraft,
    pub confidence: ConfidenceLevel,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
    pub planned_pairs: Vec<AnovaPairRequest>,
    pub all_pairs: Vec<AnovaFamilyRequest>,
    pub comparison_factor: usize,
    pub endpoints: [Option<Cell>; 2],
    pub fixed: Option<Vec<Option<Cell>>>,
}
impl Default for AnovaDraft {
    fn default() -> Self {
        Self {
            design: AnovaDesignDraft::OneWay {
                outcome: None,
                factor: None,
            },
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
            planned_pairs: vec![],
            all_pairs: vec![],
            comparison_factor: 0,
            endpoints: [None, None],
            fixed: None,
        }
    }
}
impl AnovaDraft {
    pub fn from_request(r: AnovaRequest) -> Self {
        Self {
            design: match r.design {
                AnovaDesign::OneWay { outcome, factor } => AnovaDesignDraft::OneWay {
                    outcome: Some(outcome),
                    factor: Some(factor),
                },
                AnovaDesign::Factorial { outcome, factors } => AnovaDesignDraft::Factorial {
                    outcome: Some(outcome),
                    factors,
                },
                AnovaDesign::Repeated {
                    subject,
                    factors,
                    measurements,
                } => AnovaDesignDraft::Repeated {
                    subject: Some(subject),
                    factors,
                    measurements: measurements.into_iter().map(Some).collect(),
                },
            },
            confidence: r.confidence,
            filter: r.filter,
            frequency_weight: r.frequency_weight,
            planned_pairs: r.planned_pairs,
            all_pairs: r.all_pairs,
            comparison_factor: 0,
            endpoints: [None, None],
            fixed: None,
        }
    }
    pub fn conversion_bytes(r: &AnovaRequest) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        if let AnovaDesign::Repeated { measurements, .. } = &r.design {
            n.array::<Option<ColumnId>>(measurements.len())?;
        }
        Ok(n.bytes)
    }
    pub fn matches_saved(&self, r: &AnovaRequest) -> bool {
        self.confidence == r.confidence
            && self.filter == r.filter
            && self.frequency_weight == r.frequency_weight
            && self.planned_pairs == r.planned_pairs
            && self.all_pairs == r.all_pairs
            && match (&self.design, &r.design) {
                (
                    AnovaDesignDraft::OneWay { outcome, factor },
                    AnovaDesign::OneWay {
                        outcome: o,
                        factor: f,
                    },
                ) => *outcome == Some(*o) && *factor == Some(*f),
                (
                    AnovaDesignDraft::Factorial { outcome, factors },
                    AnovaDesign::Factorial {
                        outcome: o,
                        factors: f,
                    },
                ) => *outcome == Some(*o) && factors == f,
                (
                    AnovaDesignDraft::Repeated {
                        subject,
                        factors,
                        measurements,
                    },
                    AnovaDesign::Repeated {
                        subject: s,
                        factors: f,
                        measurements: m,
                    },
                ) => {
                    *subject == Some(*s)
                        && factors == f
                        && measurements.len() == m.len()
                        && measurements.iter().zip(m).all(|(a, b)| *a == Some(*b))
                }
                _ => false,
            }
    }
    pub fn factor_count(&self) -> usize {
        match &self.design {
            AnovaDesignDraft::OneWay { factor, .. } => usize::from(factor.is_some()),
            AnovaDesignDraft::Factorial { factors, .. } => factors.len(),
            AnovaDesignDraft::Repeated { factors, .. } => factors.len(),
        }
    }
    pub fn independent_factor(&self, index: usize) -> Option<ColumnId> {
        match &self.design {
            AnovaDesignDraft::OneWay { factor, .. } if index == 0 => *factor,
            AnovaDesignDraft::Factorial { factors, .. } => factors.get(index).copied(),
            _ => None,
        }
    }
    pub fn clear_comparisons(&mut self) {
        self.planned_pairs.clear();
        self.all_pairs.clear();
        self.comparison_factor = 0;
        self.endpoints = [None, None];
        self.fixed = None;
    }
    pub fn group_roles(
        &self,
        active: Option<ColumnId>,
    ) -> Option<super::regression::GroupRoles<'_>> {
        let column = active?;
        let ordinal =
            (0..self.factor_count()).find(|i| self.independent_factor(*i) == Some(column))?;
        let selected = if ordinal == self.comparison_factor {
            [self.endpoints[0].as_ref(), self.endpoints[1].as_ref()]
        } else {
            let index = ordinal - usize::from(ordinal > self.comparison_factor);
            [
                self.fixed
                    .as_ref()
                    .and_then(|v| v.get(index))
                    .and_then(Option::as_ref),
                None,
            ]
        };
        Some(super::regression::GroupRoles {
            column: Some(column),
            selected,
        })
    }
    pub fn build_request(&self, dataset: &Dataset) -> AppResult<AnovaRequest> {
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
        peak.add(crate::operation::JobControl::DEFAULT_HEAP_BYTES)?;
        Budget::check_datasets(&[dataset], peak.bytes)?;
        // Missing roles and repeated weights refuse before copying any request vectors.
        match &self.design {
            AnovaDesignDraft::OneWay { outcome, factor } => {
                super::required(*outcome)?;
                super::required(*factor)?;
            }
            AnovaDesignDraft::Factorial { outcome, factors } => {
                super::required(*outcome)?;
                if factors.len() < 2 {
                    return Err(AppError::invalid("factors", "Select at least two factors."));
                }
            }
            AnovaDesignDraft::Repeated {
                subject,
                measurements,
                ..
            } => {
                super::required(*subject)?;
                for m in measurements {
                    super::required(*m)?;
                }
                if self.frequency_weight.is_some() {
                    return Err(AppError::invalid(
                        "weight",
                        "Repeated measures require unweighted subjects.",
                    ));
                }
            }
        }
        self.check_roles(dataset)?;
        let r = AnovaRequest {
            design: match &self.design {
                AnovaDesignDraft::OneWay { outcome, factor } => AnovaDesign::OneWay {
                    outcome: super::required(*outcome)?,
                    factor: super::required(*factor)?,
                },
                AnovaDesignDraft::Factorial { outcome, factors } => AnovaDesign::Factorial {
                    outcome: super::required(*outcome)?,
                    factors: factors.clone(),
                },
                AnovaDesignDraft::Repeated {
                    subject,
                    factors,
                    measurements,
                } => AnovaDesign::Repeated {
                    subject: super::required(*subject)?,
                    factors: factors.clone(),
                    measurements: measurements
                        .iter()
                        .map(|m| super::required(*m))
                        .collect::<AppResult<_>>()?,
                },
            },
            confidence: self.confidence,
            filter: self.filter.clone(),
            frequency_weight: self.frequency_weight,
            planned_pairs: self.planned_pairs.clone(),
            all_pairs: self.all_pairs.clone(),
        };
        // shortcut: synchronous structural validation is capped at the accepted 512 KiB request bound; worker validation uses its supplied control.
        let mut live = AllocationCounter::default();
        live.add(heap)?;
        live.add(r.heap_bytes()?)?;
        live.add(crate::operation::JobControl::DEFAULT_HEAP_BYTES)?;
        r.validate_current(
            dataset,
            &[],
            live.bytes,
            &crate::operation::JobControl::default(),
        )?;
        Ok(r)
    }
    fn check_roles(&self, data: &Dataset) -> AppResult<()> {
        let role = |id: ColumnId, numeric: bool| -> AppResult<()> {
            if id.0 == 0 || numeric && data.column(id)?.kind() != crate::data::ColumnKind::Number {
                return Err(AppError::invalid(
                    "role",
                    "Select distinct numeric measurements and a categorical factor or subject ID.",
                ));
            }
            data.column(id)?;
            Ok(())
        };
        let invalid = || {
            AppError::invalid("ANOVA setup", "Use distinct roles and a complete declared grid of at most 64 measurements. Names and levels must be nonblank and unique.")
        };
        match &self.design {
            AnovaDesignDraft::OneWay { outcome, factor } => {
                let o = super::required(*outcome)?;
                let f = super::required(*factor)?;
                role(o, true)?;
                role(f, false)?;
                if o == f {
                    return Err(invalid());
                }
            }
            AnovaDesignDraft::Factorial { outcome, factors } => {
                let o = super::required(*outcome)?;
                role(o, true)?;
                if !(2..=6).contains(&factors.len()) {
                    return Err(invalid());
                }
                for (i, f) in factors.iter().enumerate() {
                    role(*f, false)?;
                    if *f == o || factors[..i].contains(f) {
                        return Err(invalid());
                    }
                }
            }
            AnovaDesignDraft::Repeated {
                subject,
                factors,
                measurements,
            } => {
                let s = super::required(*subject)?;
                role(s, false)?;
                if factors.is_empty() || factors.len() > 6 {
                    return Err(invalid());
                }
                let mut grid = 1usize;
                for (i, f) in factors.iter().enumerate() {
                    if f.name.chars().all(char::is_whitespace)
                        || factors[..i].iter().any(|p| p.name == f.name)
                        || f.levels.len() < 2
                    {
                        return Err(invalid());
                    }
                    grid = grid
                        .checked_mul(f.levels.len())
                        .filter(|n| *n <= 64)
                        .ok_or_else(invalid)?;
                    for (j, l) in f.levels.iter().enumerate() {
                        if l.chars().all(char::is_whitespace) || f.levels[..j].contains(l) {
                            return Err(invalid());
                        }
                    }
                }
                if measurements.len() != grid {
                    return Err(invalid());
                }
                for (i, m) in measurements.iter().enumerate() {
                    let m = super::required(*m)?;
                    role(m, true)?;
                    if m == s || measurements[..i].contains(&Some(m)) {
                        return Err(invalid());
                    }
                }
            }
        }
        Ok(())
    }
    pub fn heap_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        match &self.design {
            AnovaDesignDraft::OneWay { .. } => {}
            AnovaDesignDraft::Factorial { factors, .. } => {
                n.array::<ColumnId>(factors.capacity())?
            }
            AnovaDesignDraft::Repeated {
                factors,
                measurements,
                ..
            } => {
                n.array::<AnovaWithinFactor>(factors.capacity())?;
                n.array::<Option<ColumnId>>(measurements.capacity())?;
                for f in factors {
                    n.add(f.name.capacity())?;
                    n.array::<String>(f.levels.capacity())?;
                    for l in &f.levels {
                        n.add(l.capacity())?;
                    }
                }
            }
        }
        n.array::<RowPredicate>(self.filter.predicates.capacity())?;
        for p in &self.filter.predicates {
            if let RowPredicate::Text { value, .. } = p {
                n.add(value.capacity())?;
            }
        }
        n.array::<AnovaPairRequest>(self.planned_pairs.capacity())?;
        for p in &self.planned_pairs {
            cell_bytes(&mut n, &p.first)?;
            cell_bytes(&mut n, &p.second)?;
            scope_bytes(&mut n, &p.scope)?;
        }
        n.array::<AnovaFamilyRequest>(self.all_pairs.capacity())?;
        for p in &self.all_pairs {
            scope_bytes(&mut n, &p.scope)?;
        }
        for c in self.endpoints.iter().flatten() {
            cell_bytes(&mut n, c)?;
        }
        if let Some(v) = &self.fixed {
            n.array::<Option<Cell>>(v.capacity())?;
            for c in v.iter().flatten() {
                cell_bytes(&mut n, c)?;
            }
        }
        Ok(n.bytes)
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        let roles = match &self.design {
            AnovaDesignDraft::OneWay { .. } => 2,
            AnovaDesignDraft::Factorial { factors, .. } => factors.len() + 1,
            AnovaDesignDraft::Repeated { measurements, .. } => measurements.len() + 1,
        };
        for count in [
            roles,
            self.filter.predicates.len(),
            usize::from(self.frequency_weight.is_some()),
            self.planned_pairs.len(),
            self.all_pairs.len(),
        ] {
            n.array::<[u8; 1024]>(count)?;
        }
        Ok(n.bytes)
    }
}
fn cell_bytes(n: &mut AllocationCounter, c: &Cell) -> AppResult<()> {
    if let Cell::Text(s) = c {
        n.add(s.capacity())?;
    }
    Ok(())
}
fn scope_bytes(n: &mut AllocationCounter, s: &AnovaScope) -> AppResult<()> {
    if let AnovaScope::Fixed(v) = s {
        n.array::<Cell>(v.capacity())?;
        for c in v {
            cell_bytes(n, c)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
