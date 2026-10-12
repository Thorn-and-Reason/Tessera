//! Incomplete roles stay absent until a recipe is checked for computation.
use crate::{
    analysis::*,
    data::{AllocationCounter, Cell, ColumnId, Dataset},
    error::{AppError, AppResult},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Procedure {
    Describe,
    Pearson,
    OneSampleT,
    PairedT,
    IndependentT,
    Frequencies,
    Histogram,
    Scatter,
    Boxplot,
    Regression,
    Anova,
    Reliability,
}
impl Procedure {
    pub const ALL: [Self; 12] = [
        Self::Describe,
        Self::Frequencies,
        Self::Histogram,
        Self::Boxplot,
        Self::Scatter,
        Self::Pearson,
        Self::Regression,
        Self::OneSampleT,
        Self::PairedT,
        Self::IndependentT,
        Self::Anova,
        Self::Reliability,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Regression => "Linear regression",
            Self::Anova => "ANOVA",
            Self::Reliability => "Scale reliability",
            Self::Describe => "Descriptives",
            Self::Frequencies => "Frequencies",
            Self::Histogram => "Histogram",
            Self::Scatter => "Scatterplot",
            Self::Boxplot => "Boxplot",
            Self::Pearson => "Pearson correlation",
            Self::OneSampleT => "One-sample t-test",
            Self::PairedT => "Paired t-test",
            Self::IndependentT => "Independent t-test",
        }
    }
    pub fn family(self) -> &'static str {
        match self {
            Self::Describe | Self::Frequencies | Self::Histogram | Self::Boxplot => "Summaries",
            Self::Pearson | Self::Scatter | Self::Regression => "Relationships",
            _ => "Comparisons",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisDraft {
    Anova(anova::AnovaDraft),
    Reliability(reliability::ReliabilityDraft),
    Regression(regression::RegressionDraft),
    Frequencies(FrequenciesRequest),
    Histogram {
        variable: Option<ColumnId>,
        bins: HistogramBins,
        filter: RowFilter,
        frequency_weight: Option<ColumnId>,
    },
    Scatter {
        x: Option<ColumnId>,
        y: Option<ColumnId>,
        filter: RowFilter,
        frequency_weight: Option<ColumnId>,
    },
    Boxplot {
        outcome: Option<ColumnId>,
        group: Option<ColumnId>,
        filter: RowFilter,
        frequency_weight: Option<ColumnId>,
    },
    Describe(DescribeRequest),
    Pearson(PearsonRequest),
    OneSampleT {
        variable: Option<ColumnId>,
        reference_text: String,
        confidence: ConfidenceLevel,
        filter: RowFilter,
        frequency_weight: Option<ColumnId>,
    },
    PairedT {
        first: Option<ColumnId>,
        second: Option<ColumnId>,
        confidence: ConfidenceLevel,
        filter: RowFilter,
        frequency_weight: Option<ColumnId>,
    },
    IndependentT {
        outcome: Option<ColumnId>,
        group: Option<ColumnId>,
        groups: [Option<Cell>; 2],
        method: IndependentMethod,
        confidence: ConfidenceLevel,
        filter: RowFilter,
        frequency_weight: Option<ColumnId>,
    },
}
impl From<DescribeRequest> for AnalysisDraft {
    fn from(r: DescribeRequest) -> Self {
        Self::Describe(r)
    }
}
fn required(role: Option<ColumnId>) -> AppResult<ColumnId> {
    role.ok_or_else(|| AppError::invalid("roles", "Select every required variable."))
}
impl AnalysisDraft {
    pub fn procedure(&self) -> Procedure {
        match self {
            Self::Anova(_) => Procedure::Anova,
            Self::Reliability(_) => Procedure::Reliability,
            Self::Regression(_) => Procedure::Regression,
            Self::Describe(_) => Procedure::Describe,
            Self::Pearson(_) => Procedure::Pearson,
            Self::Frequencies(_) => Procedure::Frequencies,
            Self::Histogram { .. } => Procedure::Histogram,
            Self::Scatter { .. } => Procedure::Scatter,
            Self::Boxplot { .. } => Procedure::Boxplot,
            Self::OneSampleT { .. } => Procedure::OneSampleT,
            Self::PairedT { .. } => Procedure::PairedT,
            Self::IndependentT { .. } => Procedure::IndependentT,
        }
    }
    pub fn common(&self) -> (&RowFilter, Option<ColumnId>) {
        match self {
            Self::Anova(r) => (&r.filter, r.frequency_weight),
            Self::Reliability(r) => (&r.filter, r.frequency_weight),
            Self::Regression(r) => (&r.filter, r.frequency_weight),
            Self::Describe(r) => (&r.filter, r.frequency_weight),
            Self::Pearson(r) => (&r.filter, r.frequency_weight),
            Self::Frequencies(r) => (&r.filter, r.frequency_weight),
            Self::Histogram {
                filter,
                frequency_weight,
                ..
            }
            | Self::Scatter {
                filter,
                frequency_weight,
                ..
            }
            | Self::Boxplot {
                filter,
                frequency_weight,
                ..
            }
            | Self::OneSampleT {
                filter,
                frequency_weight,
                ..
            }
            | Self::PairedT {
                filter,
                frequency_weight,
                ..
            }
            | Self::IndependentT {
                filter,
                frequency_weight,
                ..
            } => (filter, *frequency_weight),
        }
    }
    pub(crate) fn common_mut(&mut self) -> (&mut RowFilter, &mut Option<ColumnId>) {
        match self {
            Self::Anova(r) => (&mut r.filter, &mut r.frequency_weight),
            Self::Reliability(r) => (&mut r.filter, &mut r.frequency_weight),
            Self::Regression(r) => (&mut r.filter, &mut r.frequency_weight),
            Self::Describe(r) => (&mut r.filter, &mut r.frequency_weight),
            Self::Pearson(r) => (&mut r.filter, &mut r.frequency_weight),
            Self::Frequencies(r) => (&mut r.filter, &mut r.frequency_weight),
            Self::Histogram {
                filter,
                frequency_weight,
                ..
            }
            | Self::Scatter {
                filter,
                frequency_weight,
                ..
            }
            | Self::Boxplot {
                filter,
                frequency_weight,
                ..
            }
            | Self::OneSampleT {
                filter,
                frequency_weight,
                ..
            }
            | Self::PairedT {
                filter,
                frequency_weight,
                ..
            }
            | Self::IndependentT {
                filter,
                frequency_weight,
                ..
            } => (filter, frequency_weight),
        }
    }
    /// UI callers preflight the draft copy before changing procedure.
    pub fn for_procedure(&self, procedure: Procedure) -> Self {
        if procedure == Procedure::Anova {
            if let Self::Anova(r) = self {
                return Self::Anova(r.clone());
            }
        }
        let (filter, frequency_weight) = self.common();
        let filter = filter.clone();
        match procedure {
            Procedure::Reliability => match self {
                Self::Reliability(r) => Self::Reliability(r.clone()),
                _ => Self::Reliability(reliability::ReliabilityDraft {
                    items: vec![],
                    filter,
                    frequency_weight,
                    reversals: vec![],
                }),
            },
            Procedure::Anova => match self {
                Self::Anova(r) => Self::Anova(r.clone()),
                _ => Self::Anova(anova::AnovaDraft {
                    filter,
                    frequency_weight,
                    ..Default::default()
                }),
            },
            Procedure::Regression => Self::Regression(regression::RegressionDraft {
                filter,
                frequency_weight,
                ..Default::default()
            }),
            Procedure::Frequencies => Self::Frequencies(FrequenciesRequest {
                group: None,
                columns: vec![],
                filter,
                frequency_weight,
            }),
            Procedure::Histogram => Self::Histogram {
                variable: None,
                bins: HistogramBins::Automatic,
                filter,
                frequency_weight,
            },
            Procedure::Scatter => Self::Scatter {
                x: None,
                y: None,
                filter,
                frequency_weight,
            },
            Procedure::Boxplot => Self::Boxplot {
                outcome: None,
                group: None,
                filter,
                frequency_weight,
            },
            Procedure::Describe => Self::Describe(DescribeRequest {
                columns: vec![],
                statistics: vec![
                    Statistic::Mean,
                    Statistic::SampleStdDev,
                    Statistic::Minimum,
                    Statistic::Maximum,
                ],
                missing_policy: MissingPolicy::PerVariable,
                filter,
                frequency_weight,
            }),
            Procedure::Pearson => Self::Pearson(PearsonRequest {
                columns: vec![],
                missing_policy: CorrelationMissingPolicy::PairwiseComplete,
                filter,
                frequency_weight,
            }),
            Procedure::OneSampleT => Self::OneSampleT {
                variable: None,
                reference_text: "0".into(),
                confidence: ConfidenceLevel::P95,
                filter,
                frequency_weight,
            },
            Procedure::PairedT => Self::PairedT {
                first: None,
                second: None,
                confidence: ConfidenceLevel::P95,
                filter,
                frequency_weight,
            },
            Procedure::IndependentT => Self::IndependentT {
                outcome: None,
                group: None,
                groups: [None, None],
                method: IndependentMethod::Welch,
                confidence: ConfidenceLevel::P95,
                filter,
                frequency_weight,
            },
        }
    }
    pub(crate) fn group_roles(
        &self,
        active: Option<ColumnId>,
    ) -> Option<regression::GroupRoles<'_>> {
        match self {
            Self::IndependentT { group, groups, .. } => Some(regression::GroupRoles {
                column: *group,
                selected: [groups[0].as_ref(), groups[1].as_ref()],
            }),
            Self::Regression(r) => r.group_roles(active),
            Self::Anova(r) => r.group_roles(active),
            _ => None,
        }
    }
    /// The caller checks the complete retained baseline before this checked allocation.
    pub fn from_request(request: AnalysisRequest) -> Self {
        match request {
            AnalysisRequest::Reliability(r) => {
                Self::Reliability(reliability::ReliabilityDraft::from_request(r))
            }
            AnalysisRequest::Anova(r) => Self::Anova(anova::AnovaDraft::from_request(r)),
            AnalysisRequest::Regression(r) => {
                Self::Regression(regression::RegressionDraft::from_request(r))
            }
            AnalysisRequest::Frequencies(r) => Self::Frequencies(r),
            AnalysisRequest::Histogram(r) => Self::Histogram {
                variable: Some(r.variable),
                bins: r.bins,
                filter: r.filter,
                frequency_weight: r.frequency_weight,
            },
            AnalysisRequest::Scatter(r) => Self::Scatter {
                x: Some(r.x),
                y: Some(r.y),
                filter: r.filter,
                frequency_weight: r.frequency_weight,
            },
            AnalysisRequest::Boxplot(r) => Self::Boxplot {
                outcome: Some(r.outcome),
                group: r.group,
                filter: r.filter,
                frequency_weight: r.frequency_weight,
            },
            AnalysisRequest::Describe(r) => Self::Describe(r),
            AnalysisRequest::Pearson(r) => Self::Pearson(r),
            AnalysisRequest::OneSampleT(r) => Self::OneSampleT {
                variable: Some(r.variable),
                reference_text: r.reference_mean.to_string(),
                confidence: r.confidence,
                filter: r.filter,
                frequency_weight: r.frequency_weight,
            },
            AnalysisRequest::PairedT(r) => Self::PairedT {
                first: Some(r.first),
                second: Some(r.second),
                confidence: r.confidence,
                filter: r.filter,
                frequency_weight: r.frequency_weight,
            },
            AnalysisRequest::IndependentT(r) => Self::IndependentT {
                outcome: Some(r.outcome),
                group: Some(r.group),
                groups: r.groups.map(Some),
                method: r.method,
                confidence: r.confidence,
                filter: r.filter,
                frequency_weight: r.frequency_weight,
            },
        }
    }
    pub fn heap_bytes(&self) -> AppResult<usize> {
        let (filter, text, groups) = match self {
            Self::Describe(r) => return AnalysisRequestRef::Describe(r).heap_bytes(),
            Self::Pearson(r) => return AnalysisRequestRef::Pearson(r).heap_bytes(),
            Self::Anova(r) => return r.heap_bytes(),
            Self::Reliability(r) => return r.heap_bytes(),
            Self::Regression(r) => return r.heap_bytes(),
            Self::Frequencies(r) => return r.heap_bytes(),
            Self::Histogram { filter, .. }
            | Self::Scatter { filter, .. }
            | Self::Boxplot { filter, .. } => (filter, 0, None),
            Self::OneSampleT {
                filter,
                reference_text,
                ..
            } => (filter, reference_text.capacity(), None),
            Self::PairedT { filter, .. } => (filter, 0, None),
            Self::IndependentT { filter, groups, .. } => (filter, 0, Some(groups)),
        };
        let mut n = AllocationCounter::default();
        n.array::<RowPredicate>(filter.predicates.capacity())?;
        n.add(text)?;
        for pred in &filter.predicates {
            if let RowPredicate::Text { value, .. } = pred {
                n.add(value.capacity())?;
            }
        }
        if let Some(groups) = groups {
            for c in groups.iter().flatten() {
                if let Cell::Text(v) = c {
                    n.add(v.capacity())?;
                }
            }
        }
        Ok(n.bytes)
    }
    pub fn matches_saved(&self, saved: AnalysisRequestRef<'_>) -> bool {
        match (self, saved) {
            (Self::Reliability(a), AnalysisRequestRef::Reliability(b)) => a.matches_saved(b),
            (Self::Anova(a), AnalysisRequestRef::Anova(b)) => a.matches_saved(b),
            (Self::Regression(a), AnalysisRequestRef::Regression(b)) => a.matches_saved(b),
            (Self::Frequencies(a), AnalysisRequestRef::Frequencies(b)) => a == b,
            (
                Self::Histogram {
                    variable,
                    bins,
                    filter,
                    frequency_weight,
                },
                AnalysisRequestRef::Histogram(b),
            ) => {
                *variable == Some(b.variable)
                    && *bins == b.bins
                    && *filter == b.filter
                    && *frequency_weight == b.frequency_weight
            }
            (
                Self::Scatter {
                    x,
                    y,
                    filter,
                    frequency_weight,
                },
                AnalysisRequestRef::Scatter(b),
            ) => {
                *x == Some(b.x)
                    && *y == Some(b.y)
                    && *filter == b.filter
                    && *frequency_weight == b.frequency_weight
            }
            (
                Self::Boxplot {
                    outcome,
                    group,
                    filter,
                    frequency_weight,
                },
                AnalysisRequestRef::Boxplot(b),
            ) => {
                *outcome == Some(b.outcome)
                    && *group == b.group
                    && *filter == b.filter
                    && *frequency_weight == b.frequency_weight
            }
            (Self::Describe(a), AnalysisRequestRef::Describe(b)) => a == b,
            (Self::Pearson(a), AnalysisRequestRef::Pearson(b)) => a == b,
            (
                Self::OneSampleT {
                    variable,
                    reference_text,
                    confidence,
                    filter,
                    frequency_weight,
                },
                AnalysisRequestRef::OneSampleT(b),
            ) => {
                *variable == Some(b.variable)
                    && reference_value(reference_text) == Some(b.reference_mean)
                    && *confidence == b.confidence
                    && *filter == b.filter
                    && *frequency_weight == b.frequency_weight
            }
            (
                Self::PairedT {
                    first,
                    second,
                    confidence,
                    filter,
                    frequency_weight,
                },
                AnalysisRequestRef::PairedT(b),
            ) => {
                *first == Some(b.first)
                    && *second == Some(b.second)
                    && *confidence == b.confidence
                    && *filter == b.filter
                    && *frequency_weight == b.frequency_weight
            }
            (
                Self::IndependentT {
                    outcome,
                    group,
                    groups,
                    method,
                    confidence,
                    filter,
                    frequency_weight,
                },
                AnalysisRequestRef::IndependentT(b),
            ) => {
                *outcome == Some(b.outcome)
                    && *group == Some(b.group)
                    && groups
                        .iter()
                        .zip(&b.groups)
                        .all(|(a, b)| a.as_ref() == Some(b))
                    && *method == b.method
                    && *confidence == b.confidence
                    && *filter == b.filter
                    && *frequency_weight == b.frequency_weight
            }
            _ => false,
        }
    }
    pub fn build_request(&self, dataset: &Dataset) -> AppResult<AnalysisRequest> {
        let bytes = self.heap_bytes()?;
        crate::data::Budget::check_datasets(
            &[dataset],
            bytes.saturating_mul(2).saturating_add(4096),
        )?;
        let r = match self {
            Self::Reliability(r) => AnalysisRequest::Reliability(r.build_request(dataset)?),
            Self::Anova(r) => AnalysisRequest::Anova(r.build_request(dataset)?),
            Self::Regression(r) => AnalysisRequest::Regression(r.build_request(dataset)?),
            Self::Frequencies(r) => AnalysisRequest::Frequencies(r.clone()),
            Self::Histogram {
                variable,
                bins,
                filter,
                frequency_weight,
            } => AnalysisRequest::Histogram(HistogramRequest {
                variable: required(*variable)?,
                bins: *bins,
                filter: filter.clone(),
                frequency_weight: *frequency_weight,
            }),
            Self::Scatter {
                x,
                y,
                filter,
                frequency_weight,
            } => AnalysisRequest::Scatter(ScatterRequest {
                x: required(*x)?,
                y: required(*y)?,
                filter: filter.clone(),
                frequency_weight: *frequency_weight,
            }),
            Self::Boxplot {
                outcome,
                group,
                filter,
                frequency_weight,
            } => AnalysisRequest::Boxplot(BoxplotRequest {
                outcome: required(*outcome)?,
                group: *group,
                filter: filter.clone(),
                frequency_weight: *frequency_weight,
            }),
            Self::Describe(r) => AnalysisRequest::Describe(r.clone()),
            Self::Pearson(r) => AnalysisRequest::Pearson(r.clone()),
            Self::OneSampleT {
                variable,
                reference_text,
                confidence,
                filter,
                frequency_weight,
            } => AnalysisRequest::OneSampleT(OneSampleTRequest {
                variable: required(*variable)?,
                reference_mean: reference_value(reference_text).ok_or_else(|| {
                    AppError::invalid("reference mean", "Enter a finite reference mean.")
                })?,
                confidence: *confidence,
                filter: filter.clone(),
                frequency_weight: *frequency_weight,
            }),
            Self::PairedT {
                first,
                second,
                confidence,
                filter,
                frequency_weight,
            } => AnalysisRequest::PairedT(PairedTRequest {
                first: required(*first)?,
                second: required(*second)?,
                confidence: *confidence,
                filter: filter.clone(),
                frequency_weight: *frequency_weight,
            }),
            Self::IndependentT {
                outcome,
                group,
                groups,
                method,
                confidence,
                filter,
                frequency_weight,
            } => {
                let outcome = required(*outcome)?;
                let group = required(*group)?;
                let a = groups[0]
                    .as_ref()
                    .ok_or_else(|| AppError::invalid("groups", "Select both groups."))?;
                let b = groups[1]
                    .as_ref()
                    .ok_or_else(|| AppError::invalid("groups", "Select both groups."))?;
                AnalysisRequest::IndependentT(IndependentTRequest {
                    outcome,
                    group,
                    groups: [a.clone(), b.clone()],
                    method: *method,
                    confidence: *confidence,
                    filter: filter.clone(),
                    frequency_weight: *frequency_weight,
                })
            }
        };
        r.validate_current(dataset, &[], bytes + r.heap_bytes()?)?;
        Ok(r)
    }
    /// Upper bound before recipe cloning; accounts for indexes used to check the roles.
    pub(crate) fn validation_scratch_bytes(&self) -> AppResult<usize> {
        let (roles, filter) = match self {
            Self::Describe(r) => return AnalysisRequestRef::Describe(r).validation_scratch_bytes(),
            Self::Pearson(r) => return AnalysisRequestRef::Pearson(r).validation_scratch_bytes(),
            Self::Anova(r) => return r.validation_scratch_bytes(),
            Self::Reliability(r) => return r.validation_scratch_bytes(),
            Self::Regression(r) => return r.validation_scratch_bytes(),
            Self::Frequencies(r) => return r.validation_scratch_bytes(),
            Self::Histogram { filter, .. } => (2, filter),
            Self::Scatter { filter, .. } | Self::Boxplot { filter, .. } => (3, filter),
            Self::OneSampleT { filter, .. } => (2, filter),
            Self::PairedT { filter, .. } | Self::IndependentT { filter, .. } => (3, filter),
        };
        let mut n = AllocationCounter::default();
        n.array::<[u8; 1024]>(roles + filter.predicates.len())?;
        Ok(n.bytes)
    }
}

fn reference_value(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

pub mod regression;

pub mod anova;

pub mod reliability;
