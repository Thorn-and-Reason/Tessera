//! Typed recipes and completed results; historical recipes retain their own column schema.
use super::*;
use crate::data::MAX_OWNED_BYTES;
use std::mem::size_of;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfidenceLevel {
    P90,
    #[default]
    P95,
    P99,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CorrelationMissingPolicy {
    #[default]
    PairwiseComplete,
    CompleteCases,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndependentMethod {
    #[default]
    Welch,
    Pooled,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PearsonRequest {
    pub columns: Vec<ColumnId>,
    pub missing_policy: CorrelationMissingPolicy,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OneSampleTRequest {
    pub variable: ColumnId,
    pub reference_mean: f64,
    pub confidence: ConfidenceLevel,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PairedTRequest {
    pub first: ColumnId,
    pub second: ColumnId,
    pub confidence: ConfidenceLevel,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct IndependentTRequest {
    pub outcome: ColumnId,
    pub group: ColumnId,
    pub groups: [Cell; 2],
    pub method: IndependentMethod,
    pub confidence: ConfidenceLevel,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisRequest {
    Describe(DescribeRequest),
    Pearson(PearsonRequest),
    OneSampleT(OneSampleTRequest),
    PairedT(PairedTRequest),
    IndependentT(IndependentTRequest),
    Frequencies(FrequenciesRequest),
    Histogram(HistogramRequest),
    Scatter(ScatterRequest),
    Boxplot(BoxplotRequest),
    Regression(RegressionRequest),
    Anova(AnovaRequest),
    Reliability(ReliabilityRequest),
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnalysisRequestRef<'a> {
    Describe(&'a DescribeRequest),
    Pearson(&'a PearsonRequest),
    OneSampleT(&'a OneSampleTRequest),
    PairedT(&'a PairedTRequest),
    IndependentT(&'a IndependentTRequest),
    Frequencies(&'a FrequenciesRequest),
    Histogram(&'a HistogramRequest),
    Scatter(&'a ScatterRequest),
    Boxplot(&'a BoxplotRequest),
    Regression(&'a RegressionRequest),
    Anova(&'a AnovaRequest),
    Reliability(&'a ReliabilityRequest),
}
#[derive(Debug, Clone, PartialEq)]
pub struct CorrelationRow {
    pub first: ColumnId,
    pub second: ColumnId,
    pub n: u64,
    pub r: Estimate,
    pub p: Estimate,
    pub missing_rows: usize,
    pub complete_case_excluded_rows: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SampleSummary {
    pub n: u64,
    pub mean: Estimate,
    pub sample_sd: Estimate,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TStatistics {
    pub difference: Estimate,
    pub standard_error: Estimate,
    pub t: Estimate,
    pub df: Estimate,
    pub p: Estimate,
    pub interval: [Estimate; 2],
    pub effect_size: Estimate,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupExclusions {
    pub missing_group_rows: usize,
    pub other_group_rows: usize,
    pub missing_outcome_rows: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedPearson {
    pub revision: RevisionId,
    pub request: PearsonRequest,
    pub columns: Vec<RunColumnInfo>,
    pub diagnostics: RunDiagnostics,
    pub rows: Vec<CorrelationRow>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedOneSampleT {
    pub revision: RevisionId,
    pub request: OneSampleTRequest,
    pub columns: Vec<RunColumnInfo>,
    pub diagnostics: RunDiagnostics,
    pub sample: SampleSummary,
    pub missing_rows: usize,
    pub statistics: TStatistics,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedPairedT {
    pub revision: RevisionId,
    pub request: PairedTRequest,
    pub columns: Vec<RunColumnInfo>,
    pub diagnostics: RunDiagnostics,
    pub first: SampleSummary,
    pub second: SampleSummary,
    pub difference_sd: Estimate,
    pub statistics: TStatistics,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedIndependentT {
    pub revision: RevisionId,
    pub request: IndependentTRequest,
    pub columns: Vec<RunColumnInfo>,
    pub diagnostics: RunDiagnostics,
    pub groups: [SampleSummary; 2],
    pub exclusions: GroupExclusions,
    pub statistics: TStatistics,
}
#[derive(Debug, Clone, PartialEq)]
pub enum CompletedAnalysis {
    Describe(ComputedAnalysis),
    Pearson(ComputedPearson),
    OneSampleT(ComputedOneSampleT),
    PairedT(ComputedPairedT),
    IndependentT(ComputedIndependentT),
    Frequencies(ComputedFrequencies),
    Histogram(ComputedHistogram),
    Scatter(ComputedScatter),
    Boxplot(ComputedBoxplot),
    Regression(ComputedRegression),
    Anova(ComputedAnova),
    Reliability(ReliabilityRun),
}
impl From<DescribeRequest> for AnalysisRequest {
    fn from(value: DescribeRequest) -> Self {
        Self::Describe(value)
    }
}
impl From<ComputedAnalysis> for CompletedAnalysis {
    fn from(value: ComputedAnalysis) -> Self {
        Self::Describe(value)
    }
}

pub(crate) fn checked_sum(values: impl IntoIterator<Item = usize>) -> AppResult<usize> {
    values.into_iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value)
            .filter(|n| *n <= MAX_OWNED_BYTES)
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })
    })
}
pub(crate) fn checked_product(count: usize, width: usize) -> AppResult<usize> {
    count
        .checked_mul(width)
        .filter(|n| *n <= MAX_OWNED_BYTES)
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })
}
impl AnalysisRequest {
    pub fn borrowed(&self) -> AnalysisRequestRef<'_> {
        match self {
            Self::Describe(v) => AnalysisRequestRef::Describe(v),
            Self::Pearson(v) => AnalysisRequestRef::Pearson(v),
            Self::OneSampleT(v) => AnalysisRequestRef::OneSampleT(v),
            Self::PairedT(v) => AnalysisRequestRef::PairedT(v),
            Self::IndependentT(v) => AnalysisRequestRef::IndependentT(v),
            Self::Frequencies(v) => AnalysisRequestRef::Frequencies(v),
            Self::Histogram(v) => AnalysisRequestRef::Histogram(v),
            Self::Scatter(v) => AnalysisRequestRef::Scatter(v),
            Self::Boxplot(v) => AnalysisRequestRef::Boxplot(v),
            Self::Regression(v) => AnalysisRequestRef::Regression(v),
            Self::Anova(v) => AnalysisRequestRef::Anova(v),
            Self::Reliability(v) => AnalysisRequestRef::Reliability(v),
        }
    }
    pub(crate) fn heap_bytes_controlled(&self, control: &JobControl) -> AppResult<usize> {
        control.check_cancelled()?;
        match self {
            Self::Reliability(v) => v.heap_bytes_controlled(Some(control)),
            _ => self.heap_bytes(),
        }
    }
    pub fn heap_bytes(&self) -> AppResult<usize> {
        self.borrowed().heap_bytes()
    }
    pub fn validate_current(
        &self,
        dataset: &Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<()> {
        self.validate_current_impl(dataset, retained, extra_bytes, None)
    }
    pub fn validate_current_controlled(
        &self,
        dataset: &Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
        control: &JobControl,
    ) -> AppResult<()> {
        self.validate_current_impl(dataset, retained, extra_bytes, Some(control))
    }
    fn validate_current_impl(
        &self,
        dataset: &Dataset,
        retained: &[&Dataset],
        extra_bytes: usize,
        control: Option<&JobControl>,
    ) -> AppResult<()> {
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        if let Self::Reliability(v) = self {
            match control {
                Some(c) => v.admit_controlled(c)?,
                None => v.admit()?,
            };
        }
        if let Self::Anova(v) = self {
            v.admit()?;
        }
        let fallback_bytes = if control.is_none() {
            JobControl::DEFAULT_HEAP_BYTES
        } else {
            0
        };
        let extra = checked_sum([
            extra_bytes,
            fallback_bytes,
            self.borrowed().validation_scratch_bytes()?,
        ])?;
        let bytes = Budget::dataset_bytes_iter(
            std::iter::once(dataset).chain(retained.iter().copied()),
            extra,
        )?;
        Budget::check_datasets(&[], bytes)?;
        let fallback;
        let c = match control {
            Some(c) => c,
            None => {
                fallback = JobControl::default();
                &fallback
            }
        };
        if let Self::Anova(v) = self {
            return v.validate_current(
                dataset,
                retained,
                checked_sum([extra_bytes, fallback_bytes])?,
                c,
            );
        }
        validation::validate_request_controlled(
            self.borrowed(),
            |id| Ok(dataset.column(id)?.kind()),
            c,
        )
        .map(|_| ())
    }
}
impl PartialEq<AnalysisRequest> for AnalysisRequestRef<'_> {
    fn eq(&self, other: &AnalysisRequest) -> bool {
        *self == other.borrowed()
    }
}
impl PartialEq<AnalysisRequestRef<'_>> for AnalysisRequest {
    fn eq(&self, other: &AnalysisRequestRef<'_>) -> bool {
        self.borrowed() == *other
    }
}
impl<'a> AnalysisRequestRef<'a> {
    pub fn clone_owned(self) -> AnalysisRequest {
        match self {
            Self::Describe(v) => AnalysisRequest::Describe(v.clone()),
            Self::Pearson(v) => AnalysisRequest::Pearson(v.clone()),
            Self::OneSampleT(v) => AnalysisRequest::OneSampleT(v.clone()),
            Self::PairedT(v) => AnalysisRequest::PairedT(v.clone()),
            Self::IndependentT(v) => AnalysisRequest::IndependentT(v.clone()),
            Self::Frequencies(v) => AnalysisRequest::Frequencies(v.clone()),
            Self::Histogram(v) => AnalysisRequest::Histogram(v.clone()),
            Self::Scatter(v) => AnalysisRequest::Scatter(v.clone()),
            Self::Boxplot(v) => AnalysisRequest::Boxplot(v.clone()),
            Self::Regression(v) => AnalysisRequest::Regression(v.clone()),
            Self::Anova(v) => AnalysisRequest::Anova(v.clone()),
            Self::Reliability(v) => AnalysisRequest::Reliability(v.clone()),
        }
    }
    pub(crate) fn filter(self) -> &'a RowFilter {
        match self {
            Self::Describe(v) => &v.filter,
            Self::Pearson(v) => &v.filter,
            Self::OneSampleT(v) => &v.filter,
            Self::PairedT(v) => &v.filter,
            Self::IndependentT(v) => &v.filter,
            Self::Frequencies(v) => &v.filter,
            Self::Histogram(v) => &v.filter,
            Self::Scatter(v) => &v.filter,
            Self::Boxplot(v) => &v.filter,
            Self::Regression(v) => &v.filter,
            Self::Anova(v) => &v.filter,
            Self::Reliability(v) => &v.filter,
        }
    }
    pub(crate) fn frequency_weight(self) -> Option<ColumnId> {
        match self {
            Self::Describe(v) => v.frequency_weight,
            Self::Pearson(v) => v.frequency_weight,
            Self::OneSampleT(v) => v.frequency_weight,
            Self::PairedT(v) => v.frequency_weight,
            Self::IndependentT(v) => v.frequency_weight,
            Self::Frequencies(v) => v.frequency_weight,
            Self::Histogram(v) => v.frequency_weight,
            Self::Scatter(v) => v.frequency_weight,
            Self::Boxplot(v) => v.frequency_weight,
            Self::Regression(v) => v.frequency_weight,
            Self::Anova(v) => v.frequency_weight,
            Self::Reliability(v) => v.frequency_weight,
        }
    }
    pub(crate) fn role_count(self) -> usize {
        match self {
            Self::Describe(v) => v.columns.len(),
            Self::Pearson(v) => v.columns.len(),
            Self::OneSampleT(_) | Self::Histogram(_) => 1,
            Self::Frequencies(v) => v.columns.len(),
            Self::Boxplot(v) => 1 + usize::from(v.group.is_some()),
            Self::Regression(v) => 1 + v.predictors.len(),
            Self::Reliability(v) => v.items.len(),
            Self::Anova(v) => match &v.design {
                AnovaDesign::OneWay { .. } => 2,
                AnovaDesign::Factorial { factors, .. } => 1 + factors.len(),
                AnovaDesign::Repeated { measurements, .. } => 1 + measurements.len(),
            },
            _ => 2,
        }
    }
    pub fn validation_scratch_bytes(self) -> AppResult<usize> {
        if let Self::Anova(v) = self {
            return v.validation_scratch_bytes();
        }
        if let Self::Reliability(v) = self {
            return v.validation_scratch_bytes();
        }
        if let Self::Regression(v) = self {
            return v.validation_scratch_bytes();
        }
        let count = self
            .role_count()
            .checked_add(self.filter().predicates.len())
            .and_then(|n| n.checked_add(usize::from(self.frequency_weight().is_some())))
            .and_then(|n| {
                n.checked_add(match self {
                    Self::Describe(v) => v.statistics.len(),
                    _ => 0,
                })
            })
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?;
        // Reference/role/filter indexes coexist; sparse tree entries have a conservative bound.
        checked_product(count, 1024)
    }
    pub fn heap_bytes(self) -> AppResult<usize> {
        match self {
            Self::Frequencies(v) => return v.heap_bytes(),
            Self::Histogram(v) => return v.heap_bytes(),
            Self::Scatter(v) => return v.heap_bytes(),
            Self::Boxplot(v) => return v.heap_bytes(),
            Self::Regression(v) => return v.heap_bytes(),
            Self::Anova(v) => return v.heap_bytes(),
            Self::Reliability(v) => return v.heap_bytes(),
            _ => {}
        }
        let filter = self.filter();
        let mut bytes = checked_product(filter.predicates.capacity(), size_of::<RowPredicate>())?;
        for predicate in &filter.predicates {
            if let RowPredicate::Text { value, .. } = predicate {
                bytes = checked_sum([bytes, value.capacity()])?;
            }
        }
        match self {
            Self::Describe(v) => checked_sum([
                bytes,
                checked_product(v.columns.capacity(), size_of::<ColumnId>())?,
                checked_product(v.statistics.capacity(), size_of::<Statistic>())?,
            ]),
            Self::Pearson(v) => checked_sum([
                bytes,
                checked_product(v.columns.capacity(), size_of::<ColumnId>())?,
            ]),
            Self::IndependentT(v) => {
                for value in &v.groups {
                    if let Cell::Text(value) = value {
                        bytes = checked_sum([bytes, value.capacity()])?;
                    }
                }
                Ok(bytes)
            }
            _ => Ok(bytes),
        }
    }
}
impl CompletedAnalysis {
    pub(crate) fn owned_bytes_controlled(&self, control: &JobControl) -> AppResult<usize> {
        control.check_cancelled()?;
        match self {
            Self::Reliability(v) => checked_sum([
                size_of::<Self>() - size_of::<ReliabilityRun>(),
                v.owned_bytes_controlled(Some(control))?,
            ]),
            _ => self.owned_bytes(),
        }
    }
    pub(crate) fn validation_scratch_bytes_controlled(
        &self,
        control: &JobControl,
    ) -> AppResult<usize> {
        control.check_cancelled()?;
        match self {
            Self::Reliability(v) => v.scratch_controlled(Some(control)),
            _ => self.validation_scratch_bytes(),
        }
    }
    pub fn is_exploration(&self) -> bool {
        matches!(
            self,
            Self::Frequencies(_) | Self::Histogram(_) | Self::Scatter(_) | Self::Boxplot(_)
        )
    }
    pub fn revision(&self) -> RevisionId {
        match self {
            Self::Describe(v) => v.revision,
            Self::Pearson(v) => v.revision,
            Self::OneSampleT(v) => v.revision,
            Self::PairedT(v) => v.revision,
            Self::IndependentT(v) => v.revision,
            Self::Frequencies(v) => v.revision,
            Self::Histogram(v) => v.revision,
            Self::Scatter(v) => v.revision,
            Self::Boxplot(v) => v.revision,
            Self::Regression(v) => v.revision,
            Self::Anova(v) => v.revision,
            Self::Reliability(v) => v.revision,
        }
    }
    pub fn columns(&self) -> &[RunColumnInfo] {
        match self {
            Self::Describe(v) => &v.columns,
            Self::Pearson(v) => &v.columns,
            Self::OneSampleT(v) => &v.columns,
            Self::PairedT(v) => &v.columns,
            Self::IndependentT(v) => &v.columns,
            Self::Frequencies(v) => &v.columns,
            Self::Histogram(v) => &v.columns,
            Self::Scatter(v) => &v.columns,
            Self::Boxplot(v) => &v.columns,
            Self::Regression(v) => &v.columns,
            Self::Anova(v) => &v.columns,
            Self::Reliability(v) => &v.columns,
        }
    }
    pub fn diagnostics(&self) -> &RunDiagnostics {
        match self {
            Self::Describe(v) => &v.diagnostics,
            Self::Pearson(v) => &v.diagnostics,
            Self::OneSampleT(v) => &v.diagnostics,
            Self::PairedT(v) => &v.diagnostics,
            Self::IndependentT(v) => &v.diagnostics,
            Self::Frequencies(v) => &v.diagnostics,
            Self::Histogram(v) => &v.diagnostics,
            Self::Scatter(v) => &v.diagnostics,
            Self::Boxplot(v) => &v.diagnostics,
            Self::Regression(v) => &v.diagnostics,
            Self::Anova(v) => &v.diagnostics,
            Self::Reliability(v) => &v.diagnostics,
        }
    }
    pub fn request(&self) -> AnalysisRequestRef<'_> {
        match self {
            Self::Describe(v) => AnalysisRequestRef::Describe(&v.request),
            Self::Pearson(v) => AnalysisRequestRef::Pearson(&v.request),
            Self::OneSampleT(v) => AnalysisRequestRef::OneSampleT(&v.request),
            Self::PairedT(v) => AnalysisRequestRef::PairedT(&v.request),
            Self::IndependentT(v) => AnalysisRequestRef::IndependentT(&v.request),
            Self::Frequencies(v) => AnalysisRequestRef::Frequencies(&v.request),
            Self::Histogram(v) => AnalysisRequestRef::Histogram(&v.request),
            Self::Scatter(v) => AnalysisRequestRef::Scatter(&v.request),
            Self::Boxplot(v) => AnalysisRequestRef::Boxplot(&v.request),
            Self::Regression(v) => AnalysisRequestRef::Regression(&v.request),
            Self::Anova(v) => AnalysisRequestRef::Anova(&v.request),
            Self::Reliability(v) => AnalysisRequestRef::Reliability(&v.request),
        }
    }
    pub fn validation_scratch_bytes(&self) -> AppResult<usize> {
        match self {
            Self::Frequencies(v) => return v.validation_scratch_bytes(),
            Self::Histogram(v) => return v.validation_scratch_bytes(),
            Self::Scatter(v) => return v.validation_scratch_bytes(),
            Self::Boxplot(v) => return v.validation_scratch_bytes(),
            Self::Regression(v) => return v.validation_scratch_bytes(),
            Self::Anova(v) => return v.validation_scratch_bytes(),
            Self::Reliability(v) => return v.validation_scratch_bytes(),
            _ => {}
        }
        let mut bytes = self.request().validation_scratch_bytes()?;
        for info in self.columns() {
            bytes = checked_sum([
                bytes,
                checked_sum([1024, checked_product(info.value_labels.len(), 1024)?])?,
            ])?;
        }
        if let Self::Describe(v) = self {
            for plot in &v.plots {
                bytes = checked_sum([bytes, checked_product(plot.points.len(), 256)?])?;
            }
        }
        Ok(bytes)
    }
    pub fn owned_bytes(&self) -> AppResult<usize> {
        match self {
            Self::Frequencies(v) => {
                return checked_sum([
                    size_of::<Self>() - size_of::<ComputedFrequencies>(),
                    v.owned_bytes()?,
                ])
            }
            Self::Histogram(v) => {
                return checked_sum([
                    size_of::<Self>() - size_of::<ComputedHistogram>(),
                    v.owned_bytes()?,
                ])
            }
            Self::Scatter(v) => {
                return checked_sum([
                    size_of::<Self>() - size_of::<ComputedScatter>(),
                    v.owned_bytes()?,
                ])
            }
            Self::Reliability(v) => {
                return checked_sum([
                    size_of::<Self>() - size_of::<ReliabilityRun>(),
                    v.owned_bytes()?,
                ]);
            }
            Self::Anova(v) => {
                return checked_sum([
                    size_of::<Self>() - size_of::<ComputedAnova>(),
                    v.owned_bytes()?,
                ]);
            }
            Self::Regression(v) => {
                return checked_sum([
                    size_of::<Self>() - size_of::<ComputedRegression>(),
                    v.owned_bytes()?,
                ])
            }
            Self::Boxplot(v) => {
                return checked_sum([
                    size_of::<Self>() - size_of::<ComputedBoxplot>(),
                    v.owned_bytes()?,
                ])
            }
            _ => {}
        }
        if let Self::Describe(v) = self {
            return checked_sum([
                size_of::<Self>() - size_of::<ComputedAnalysis>(),
                v.owned_bytes()?,
            ]);
        }
        let capacity = match self {
            Self::Pearson(v) => v.columns.capacity(),
            Self::OneSampleT(v) => v.columns.capacity(),
            Self::PairedT(v) => v.columns.capacity(),
            Self::IndependentT(v) => v.columns.capacity(),
            Self::Describe(_)
            | Self::Frequencies(_)
            | Self::Histogram(_)
            | Self::Scatter(_)
            | Self::Boxplot(_)
            | Self::Regression(_)
            | Self::Reliability(_)
            | Self::Anova(_) => unreachable!("handled above"),
        };
        let mut bytes = checked_sum([
            size_of::<Self>(),
            self.request().heap_bytes()?,
            checked_product(capacity, size_of::<RunColumnInfo>())?,
        ])?;
        for info in self.columns() {
            bytes = checked_sum([bytes, info.heap_bytes()?])?;
        }
        let mut estimates = [None; 13];
        // The fixed t output has at most 13 estimates; Pearson reasons are counted in-place.
        match self {
            Self::Pearson(v) => {
                bytes = checked_sum([
                    bytes,
                    checked_product(v.rows.capacity(), size_of::<CorrelationRow>())?,
                ])?;
                for row in &v.rows {
                    bytes = checked_sum([bytes, reason_bytes(&row.r), reason_bytes(&row.p)])?;
                }
            }
            Self::OneSampleT(v) => {
                estimates[..2].copy_from_slice(&[Some(&v.sample.mean), Some(&v.sample.sample_sd)]);
                for (slot, value) in estimates[2..].iter_mut().zip(v.statistics.estimates()) {
                    *slot = Some(value);
                }
            }
            Self::PairedT(v) => {
                estimates[..5].copy_from_slice(&[
                    Some(&v.first.mean),
                    Some(&v.first.sample_sd),
                    Some(&v.second.mean),
                    Some(&v.second.sample_sd),
                    Some(&v.difference_sd),
                ]);
                for (slot, value) in estimates[5..].iter_mut().zip(v.statistics.estimates()) {
                    *slot = Some(value);
                }
            }
            Self::IndependentT(v) => {
                estimates[..4].copy_from_slice(&[
                    Some(&v.groups[0].mean),
                    Some(&v.groups[0].sample_sd),
                    Some(&v.groups[1].mean),
                    Some(&v.groups[1].sample_sd),
                ]);
                for (slot, value) in estimates[4..].iter_mut().zip(v.statistics.estimates()) {
                    *slot = Some(value);
                }
            }
            Self::Describe(_)
            | Self::Frequencies(_)
            | Self::Histogram(_)
            | Self::Scatter(_)
            | Self::Boxplot(_)
            | Self::Regression(_)
            | Self::Reliability(_)
            | Self::Anova(_) => unreachable!("handled above"),
        }
        for estimate in estimates.into_iter().flatten() {
            bytes = checked_sum([bytes, reason_bytes(estimate)])?;
        }
        Ok(bytes)
    }
    pub fn validate(&self) -> AppResult<()> {
        self.validate_impl(None)
    }
    pub fn validate_controlled(&self, control: &JobControl) -> AppResult<()> {
        self.validate_impl(Some(control))
    }
    fn validate_impl(&self, control: Option<&JobControl>) -> AppResult<()> {
        if let Some(c) = control {
            c.check_cancelled()?;
        }
        if let Self::Reliability(v) = self {
            match control {
                Some(c) => v.request.admit_controlled(c)?,
                None => v.request.admit()?,
            };
        }
        if let Self::Anova(v) = self {
            v.request.admit()?;
        }
        Budget::check_datasets(
            &[],
            checked_sum([
                match control {
                    Some(c) => self.owned_bytes_controlled(c)?,
                    None => self.owned_bytes()?,
                },
                match control {
                    Some(c) => self.validation_scratch_bytes_controlled(c)?,
                    None => self.validation_scratch_bytes()?,
                },
                if control.is_none() {
                    JobControl::DEFAULT_HEAP_BYTES
                } else {
                    0
                },
            ])?,
        )?;
        match control {
            Some(c) => validation::validate_completed_controlled(self, c),
            None => validation::validate_completed_controlled(self, &JobControl::default()),
        }
    }
}
fn reason_bytes(estimate: &Estimate) -> usize {
    estimate.reason.as_ref().map_or(0, String::capacity)
}
impl TStatistics {
    pub(crate) fn estimates(&self) -> [&Estimate; 8] {
        [
            &self.difference,
            &self.standard_error,
            &self.t,
            &self.df,
            &self.p,
            &self.interval[0],
            &self.interval[1],
            &self.effect_size,
        ]
    }
}
