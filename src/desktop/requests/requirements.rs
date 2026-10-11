//! Cheap setup guidance; checked request builders remain authoritative.
use super::{AnalysisDraft, DesktopState};

impl DesktopState {
    /// Cheap form guidance; core validation still checks every submitted request.
    pub(crate) fn draft_requirement(&self) -> Option<&'static str> {
        self.draft_requirement_for(&self.draft)
    }
    pub(super) fn draft_requirement_for(&self, draft: &AnalysisDraft) -> Option<&'static str> {
        use crate::{
            analysis::{HistogramBins, RowPredicate},
            data::{Cell, ColumnId, ColumnKind},
        };
        let data = self.project().dataset();
        let role = |id: Option<ColumnId>, numeric: bool| -> Option<&'static str> {
            let Some(id) = id else {
                return Some("Select every required variable.");
            };
            match data.column(id) {
                Err(_) => Some("A selected variable is unavailable. Select another variable."),
                Ok(column) if numeric && column.kind() != ColumnKind::Number => {
                    Some("Select a numeric variable. A selected variable now contains text.")
                }
                _ => None,
            }
        };
        let pair = |a, b| {
            role(a, true)
                .or_else(|| role(b, true))
                .or_else(|| (a == b).then_some("Select two different numeric variables."))
        };
        let list = |ids: &[ColumnId], minimum: usize, numeric: bool| {
            if ids.len() < minimum {
                return Some(if minimum == 2 {
                    "Select at least two numeric variables."
                } else if numeric {
                    "Select at least one numeric variable."
                } else {
                    "Select at least one variable."
                });
            }
            for id in ids {
                if let Some(message) = role(Some(*id), numeric) {
                    return Some(message);
                }
            }
            None
        };
        let issue = match draft {
            AnalysisDraft::Describe(r) => list(&r.columns, 1, true).or_else(|| {
                if r.statistics.is_empty() {
                    Some("Select at least one statistic.")
                } else if r
                    .statistics
                    .iter()
                    .enumerate()
                    .any(|(i, value)| r.statistics[..i].contains(value))
                {
                    Some("Select each statistic only once.")
                } else {
                    None
                }
            }),
            AnalysisDraft::Pearson(r) => list(&r.columns, 2, true),
            AnalysisDraft::Frequencies(r) => list(&r.columns, 1, false),
            AnalysisDraft::Histogram { variable, bins, .. } => {
                role(*variable, true).or_else(|| {
                    matches!(bins, HistogramBins::Count(n) if !(1..=128).contains(n))
                        .then_some("Choose between 1 and 128 bins.")
                })
            }
            AnalysisDraft::Scatter { x, y, .. } => pair(*x, *y),
            AnalysisDraft::Boxplot { outcome, group, .. } => role(*outcome, true).or_else(|| {
                group.and_then(|id| role(Some(id), false)).or_else(|| {
                    (group.is_some() && outcome == group)
                        .then_some("Choose a grouping variable different from the outcome.")
                })
            }),
            AnalysisDraft::OneSampleT {
                variable,
                reference_text,
                ..
            } => role(*variable, true).or_else(|| {
                reference_text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .is_none()
                    .then_some("Enter a finite reference mean, for example 0 or 2.5.")
            }),
            AnalysisDraft::PairedT { first, second, .. } => pair(*first, *second),
            AnalysisDraft::Regression(r) => role(r.outcome, true).or_else(|| {
                if r.predictors.is_empty() {
                    return Some("Select at least one predictor.");
                }
                for predictor in &r.predictors {
                    use super::super::draft::regression::RegressionDraftRole;
                    let numeric = matches!(predictor.role, RegressionDraftRole::Numeric);
                    if let Some(message) = role(Some(predictor.column), numeric) {
                        return Some(message);
                    }
                    if matches!(predictor.role, RegressionDraftRole::Categorical { reference: None }) {
                        return Some("Select a reference level for every categorical predictor.");
                    }
                }
                None
            }),
            AnalysisDraft::Anova(r) => {
                use super::super::draft::anova::AnovaDesignDraft;
                match &r.design {
                    AnovaDesignDraft::OneWay { outcome, factor } =>
                        role(*outcome, true).or_else(|| role(*factor, false)),
                    AnovaDesignDraft::Factorial { outcome, factors } =>
                        role(*outcome, true).or_else(|| {
                            if factors.len() < 2 {
                                Some("Select at least two factors.")
                            } else {
                                list(factors, 2, false)
                            }
                        }),
                    AnovaDesignDraft::Repeated { subject, factors, measurements } =>
                        role(*subject, false).or_else(|| {
                            if r.frequency_weight.is_some() {
                                return Some("Repeated measures require unweighted subjects. Choose None for the frequency weight.");
                            }
                            if factors.is_empty() || measurements.is_empty() {
                                return Some("Declare the within-subject factors and assign every measurement.");
                            }
                            measurements.iter().find_map(|id| role(*id, true))
                        }),
                }
            }
            AnalysisDraft::Reliability(r) => list(&r.items, 2, true).or_else(|| {
                r.reversals.iter().find_map(|reversal| {
                    let lower = reversal.lower.trim().parse::<f64>().ok();
                    let upper = reversal.upper.trim().parse::<f64>().ok();
                    (!matches!((lower, upper), (Some(a), Some(b)) if a.is_finite() && b.is_finite() && a < b))
                        .then_some("Enter finite reversal bounds with the lower bound below the upper bound.")
                })
            }),
            AnalysisDraft::IndependentT {
                outcome,
                group,
                groups,
                ..
            } => role(*outcome, true)
                .or_else(|| role(*group, false))
                .or_else(|| {
                    if outcome == group {
                        return Some("Choose a grouping variable different from the outcome.");
                    }
                    if groups.iter().any(Option::is_none) {
                        return Some("Select both groups to compare.");
                    }
                    if groups[0] == groups[1] {
                        return Some("Select two different groups.");
                    }
                    let kind = data.column(group.unwrap()).ok()?.kind();
                    groups
                        .iter()
                        .flatten()
                        .any(|value| {
                            !matches!((value, kind),
                        (Cell::Number(n), ColumnKind::Number) if n.is_finite())
                                && !matches!((value, kind), (Cell::Text(_), ColumnKind::Text))
                        })
                        .then_some("Select groups that match the grouping variable's current type.")
                }),
        };
        if issue.is_some() {
            return issue;
        }
        let (filter, weight) = draft.common();
        if weight.is_some_and(|id| role(Some(id), true).is_some()) {
            return Some(
                "Select an available numeric variable for the frequency weight, or choose None.",
            );
        }
        for predicate in &filter.predicates {
            let Ok(column) = data.column(predicate.column()) else {
                return Some("A filter variable is unavailable. Remove or replace that condition.");
            };
            match predicate {
                RowPredicate::Number { value, .. } if column.kind() != ColumnKind::Number || !value.is_finite() =>
                    return Some("Use a finite numeric filter value for a numeric variable, or remove the condition."),
                RowPredicate::Text { .. } if column.kind() != ColumnKind::Text =>
                    return Some("Use a text filter for a text variable, or remove the condition."),
                _ => {},
            }
        }
        None
    }
}
