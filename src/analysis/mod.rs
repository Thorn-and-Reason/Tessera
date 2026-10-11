//! Numeric estimates remain independent of display precision.
pub(crate) mod anova;
pub mod reliability;
pub use anova::*;
pub use reliability::{
    reliability_with_budget, ReliabilityEnergyState, ReliabilityItem, ReliabilityMethod,
    ReliabilityRequest, ReliabilityReverse, ReliabilityRun,
};
pub(crate) mod exploration;
pub use exploration::*;
pub(crate) mod regression;
pub use regression::*;
mod boxplot;
mod frequencies;
mod groups;
mod histogram;
mod scatter;
pub use boxplot::boxplot_with_budget;
pub use frequencies::{
    frequencies_with_budget, frequency_rows_with_budget, FrequencySearchRequest,
    FrequencySearchResult,
};
pub use histogram::histogram_with_budget;
pub use scatter::scatter_with_budget;
mod model;
pub(crate) mod support;
pub(crate) mod validation;
pub use model::*;
mod inference;
mod moments;
mod pearson;
mod quadrature;
mod selection;
mod t_tests;
use crate::{
    data::{Budget, Cell, ColumnId, ColumnKind, Dataset, MissingRules, RevisionId, ValueLabel},
    error::{AppError, AppResult},
    operation::JobControl,
};
pub use groups::{group_values_with_budget, GroupChoice, GroupLookupRequest, GroupLookupResult};
pub use pearson::pearson_with_budget;
pub use selection::{NumericComparison, RowFilter, RowPredicate, MAX_FREQUENCY};
use std::{collections::BTreeMap, mem::size_of};
pub use t_tests::{independent_t_with_budget, one_sample_t_with_budget, paired_t_with_budget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Statistic {
    Mean,
    SampleStdDev,
    Variance,
    Minimum,
    Maximum,
    Sum,
    Range,
    StdErrorMean,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingPolicy {
    PerVariable,
    CompleteCase,
}
#[derive(Debug, Clone, PartialEq)]
pub struct DescribeRequest {
    pub columns: Vec<ColumnId>,
    pub statistics: Vec<Statistic>,
    pub missing_policy: MissingPolicy,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub value: Option<f64>,
    pub reason: Option<String>,
}
impl Estimate {
    fn available(value: f64) -> Self {
        Self {
            value: Some(value),
            reason: None,
        }
    }
    fn unavailable(reason: &str) -> Self {
        Self {
            value: None,
            reason: Some(reason.into()),
        }
    }
    fn representable(value: f64) -> Self {
        if value.is_finite() {
            Self::available(value)
        } else {
            Self::unavailable("The estimate exceeds Float64 range.")
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptiveRow {
    pub column: ColumnId,
    /// Valid integer frequencies; equals observation rows when unweighted.
    pub valid_n: u64,
    /// Frequencies missing in this variable after filtering and weight eligibility.
    pub missing_n: u64,
    /// Present frequencies excluded because another selected variable is missing.
    pub complete_case_excluded_n: u64,
    pub estimates: BTreeMap<Statistic, Estimate>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// These are observation-row counts, not summed frequencies.
pub struct RunDiagnostics {
    pub filtered_rows: usize,
    pub missing_weight_rows: usize,
    pub zero_weight_rows: usize,
    pub complete_case_rows: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RunColumnInfo {
    pub id: ColumnId,
    pub name: String,
    pub kind: ColumnKind,
    pub description: String,
    pub missing_rules: MissingRules,
    pub value_labels: Vec<ValueLabel>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PlotPoint {
    /// Zero-based source observation row.
    pub row: usize,
    pub value: f64,
    pub frequency: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PlotData {
    pub column: ColumnId,
    pub points: Vec<PlotPoint>,
    pub source_rows: usize,
    pub reduced: bool,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedAnalysis {
    pub revision: RevisionId,
    pub request: DescribeRequest,
    pub columns: Vec<RunColumnInfo>,
    pub rows: Vec<DescriptiveRow>,
    pub diagnostics: RunDiagnostics,
    pub plots: Vec<PlotData>,
}
pub const MAX_PLOT_POINTS: usize = 2000;

/// Computes descriptives with no caller-retained allocations.
pub fn describe(
    dataset: &Dataset,
    request: &DescribeRequest,
    control: &JobControl,
) -> AppResult<ComputedAnalysis> {
    describe_with_budget(dataset, request, control, &[], 0)
}
/// Includes caller-retained dataset revisions and other owned work before allocation.
/// `extra_bytes` includes retained runs, title, input and pending work outside these datasets.
pub fn describe_with_budget(
    dataset: &Dataset,
    request: &DescribeRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ComputedAnalysis> {
    control.checkpoint(0, Some(total_work(dataset, request)))?;
    let datasets = std::iter::once(dataset)
        .chain(retained.iter().copied())
        .collect::<Vec<_>>();
    let overhead = datasets.capacity() * size_of::<&Dataset>();
    let extra = extra_bytes
        .checked_add(overhead)
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    let validation_bytes = selection::validation_bytes(request)?;
    let validation_extra = extra
        .checked_add(validation_bytes)
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    Budget::check_datasets(&datasets, validation_extra)?;
    let ids = selection::validate(dataset, request)?;
    let result_bytes = result_budget(dataset, request, &ids)?;
    let work = dataset.row_count() * size_of::<selection::Eligible>() + validation_bytes;
    let aggregate = extra
        .checked_add(result_bytes)
        .and_then(|n| n.checked_add(work))
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
    Budget::check_datasets(&datasets, aggregate)?;
    compute_analysis(dataset, request, control, ids)
}
fn compute_analysis(
    dataset: &Dataset,
    request: &DescribeRequest,
    control: &JobControl,
    ids: std::collections::BTreeSet<ColumnId>,
) -> AppResult<ComputedAnalysis> {
    let total = total_work(dataset, request);
    let (eligible, diagnostics) = selection::select(dataset, request, control, total)?;
    let mut rows = Vec::with_capacity(request.columns.len());
    let mut plots = Vec::with_capacity(request.columns.len());
    for (index, id) in request.columns.iter().enumerate() {
        let progress = (dataset.row_count() * (1 + 3 * index), total);
        let (row, plot) = describe_column(dataset, *id, request, &eligible, control, progress)?;
        rows.push(row);
        plots.push(plot);
    }
    let columns = support::capture_columns(dataset, &ids, control)?;
    control.checkpoint(total, Some(total))?;
    Ok(ComputedAnalysis {
        revision: dataset.revision(),
        request: request.clone(),
        columns,
        rows,
        diagnostics,
        plots,
    })
}
fn result_budget(
    dataset: &Dataset,
    request: &DescribeRequest,
    ids: &std::collections::BTreeSet<ColumnId>,
) -> AppResult<usize> {
    let per_column = model::checked_sum([
        size_of::<DescriptiveRow>(),
        size_of::<PlotData>(),
        model::checked_product(MAX_PLOT_POINTS, size_of::<PlotPoint>())?,
        256,
        model::checked_product(request.statistics.len(), 256)?,
    ])?;
    model::checked_sum([
        size_of::<ComputedAnalysis>(),
        AnalysisRequestRef::Describe(request).heap_bytes()?,
        support::snapshot_bytes(dataset, ids)?,
        model::checked_product(request.columns.len(), per_column)?,
    ])
}
impl ComputedAnalysis {
    pub(crate) fn owned_bytes(&self) -> AppResult<usize> {
        let mut counter = crate::data::AllocationCounter::default();
        counter.add(size_of::<Self>())?;
        counter.add(AnalysisRequestRef::Describe(&self.request).heap_bytes()?)?;
        counter.array::<DescriptiveRow>(self.rows.capacity())?;
        counter.array::<RunColumnInfo>(self.columns.capacity())?;
        counter.array::<PlotData>(self.plots.capacity())?;
        for info in &self.columns {
            counter.add(info.heap_bytes()?)?;
        }
        for row in &self.rows {
            counter.add(model::checked_product(row.estimates.len(), 256)?)?;
            for estimate in row.estimates.values() {
                if let Some(reason) = &estimate.reason {
                    counter.add(reason.capacity())?;
                }
            }
        }
        for plot in &self.plots {
            counter.array::<PlotPoint>(plot.points.capacity())?;
            counter.add(plot.label.capacity())?;
        }
        Ok(counter.bytes)
    }
}
fn describe_column(
    dataset: &Dataset,
    id: ColumnId,
    request: &DescribeRequest,
    eligible: &[selection::Eligible],
    control: &JobControl,
    progress: (usize, usize),
) -> AppResult<(DescriptiveRow, PlotData)> {
    let column = dataset.column(id)?;
    let (moments, missing_n, excluded_n, source_rows) =
        summarize(column, eligible, control, progress)?;
    let mean = normalized_mean(
        column,
        &moments,
        eligible,
        control,
        (progress.0 + column.len(), progress.1),
    )?;
    let (norm, plot) = spread_and_plot(
        column,
        &moments,
        mean,
        eligible,
        source_rows,
        control,
        (progress.0 + 2 * column.len(), progress.1),
    )?;
    let estimates = request
        .statistics
        .iter()
        .map(|stat| (*stat, moments.estimate(*stat, &norm)))
        .collect();
    let row = DescriptiveRow {
        column: id,
        valid_n: moments.n,
        missing_n,
        complete_case_excluded_n: excluded_n,
        estimates,
    };
    let label = if request.frequency_weight.is_some() {
        "Observation rows; point frequency records integer weight."
    } else {
        "Observation rows; each point has frequency 1."
    };
    Ok((
        row,
        PlotData {
            label: label.into(),
            ..plot
        },
    ))
}
fn summarize(
    column: &crate::data::Column,
    eligible: &[selection::Eligible],
    control: &JobControl,
    progress: (usize, usize),
) -> AppResult<(moments::Moments, u64, u64, usize)> {
    let mut moments = moments::Moments::default();
    let (mut missing, mut excluded, mut rows) = (0, 0, 0);
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(progress.0 + index, Some(progress.1))?;
        }
        if column.is_missing(entry.row)? {
            missing += entry.weight;
            continue;
        }
        if !entry.complete {
            excluded += entry.weight;
            continue;
        }
        if let Cell::Number(value) = column.cell(entry.row)? {
            moments.add(value, entry.weight);
            rows += 1;
        }
    }
    control.checkpoint(progress.0 + column.len(), Some(progress.1))?;
    Ok((moments, missing, excluded, rows))
}
fn normalized_mean(
    column: &crate::data::Column,
    moments: &moments::Moments,
    eligible: &[selection::Eligible],
    control: &JobControl,
    progress: (usize, usize),
) -> AppResult<f64> {
    let mut normalized = moments::Moments::default();
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(progress.0 + index, Some(progress.1))?;
        }
        if !entry.complete || column.is_missing(entry.row)? {
            continue;
        }
        if let Cell::Number(value) = column.cell(entry.row)? {
            normalized.add(moments.normalized(value), entry.weight);
        }
    }
    control.checkpoint(progress.0 + column.len(), Some(progress.1))?;
    Ok(normalized.mean().unwrap_or(0.0))
}
fn spread_and_plot(
    column: &crate::data::Column,
    moments: &moments::Moments,
    mean: f64,
    eligible: &[selection::Eligible],
    source_rows: usize,
    control: &JobControl,
    progress: (usize, usize),
) -> AppResult<(moments::ScaledNorm, PlotData)> {
    let mut norm = moments::ScaledNorm::default();
    let mut points = Vec::with_capacity(source_rows.min(MAX_PLOT_POINTS));
    let mut valid_index = 0;
    for (index, entry) in eligible.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(progress.0 + index, Some(progress.1))?;
        }
        if !entry.complete || column.is_missing(entry.row)? {
            continue;
        }
        if let Cell::Number(value) = column.cell(entry.row)? {
            norm.add_deviation(moments.normalized(value), mean, entry.weight);
            // Deterministic evenly spaced observations; equal values retain distinct row identities.
            if source_rows <= MAX_PLOT_POINTS
                || valid_index == 0
                || valid_index * MAX_PLOT_POINTS / source_rows
                    != (valid_index - 1) * MAX_PLOT_POINTS / source_rows
            {
                points.push(PlotPoint {
                    row: entry.row,
                    value,
                    frequency: entry.weight,
                });
            }
            valid_index += 1;
        }
    }
    let plot = PlotData {
        column: column.id(),
        points,
        source_rows,
        reduced: source_rows > MAX_PLOT_POINTS,
        label: String::new(),
    };
    control.checkpoint(progress.0 + column.len(), Some(progress.1))?;
    Ok((norm, plot))
}

impl RunColumnInfo {
    fn heap_bytes(&self) -> AppResult<usize> {
        let mut counter = crate::data::AllocationCounter::default();
        counter.add(self.name.capacity())?;
        counter.add(self.description.capacity())?;
        match &self.missing_rules {
            MissingRules::None => {}
            MissingRules::Number { values, ranges } => {
                counter.array::<f64>(values.capacity())?;
                counter.array::<(f64, f64)>(ranges.capacity())?;
            }
            MissingRules::Text(values) => {
                counter.array::<String>(values.capacity())?;
                for value in values {
                    counter.add(value.capacity())?;
                }
            }
        }
        counter.array::<ValueLabel>(self.value_labels.capacity())?;
        for label in &self.value_labels {
            counter.add(label.label.capacity())?;
            if let Cell::Text(value) = &label.value {
                counter.add(value.capacity())?;
            }
        }
        Ok(counter.bytes)
    }
}

fn total_work(dataset: &Dataset, request: &DescribeRequest) -> usize {
    dataset.row_count() * (1 + 3 * request.columns.len())
}

/// Dispatches a typed recipe with caller-retained input charged once in `extra_bytes`.
pub fn analyze_with_budget(
    dataset: &Dataset,
    request: &AnalysisRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<CompletedAnalysis> {
    match request {
        AnalysisRequest::Reliability(r) => {
            reliability_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Reliability)
        }
        AnalysisRequest::Anova(r) => {
            anova::anova_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Anova)
        }
        AnalysisRequest::Regression(r) => {
            regression_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Regression)
        }
        AnalysisRequest::Frequencies(r) => {
            frequencies_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Frequencies)
        }
        AnalysisRequest::Histogram(r) => {
            histogram_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Histogram)
        }
        AnalysisRequest::Scatter(r) => {
            scatter_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Scatter)
        }
        AnalysisRequest::Boxplot(r) => {
            boxplot_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Boxplot)
        }
        AnalysisRequest::Describe(r) => {
            describe_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Describe)
        }
        AnalysisRequest::Pearson(r) => {
            pearson_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::Pearson)
        }
        AnalysisRequest::OneSampleT(r) => {
            one_sample_t_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::OneSampleT)
        }
        AnalysisRequest::PairedT(r) => {
            paired_t_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::PairedT)
        }
        AnalysisRequest::IndependentT(r) => {
            independent_t_with_budget(dataset, r, control, retained, extra_bytes)
                .map(CompletedAnalysis::IndependentT)
        }
    }
}

pub use anova::anova_with_budget;
