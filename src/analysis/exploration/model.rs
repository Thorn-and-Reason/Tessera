use crate::{
    analysis::{Estimate, RowFilter, RunColumnInfo, RunDiagnostics},
    data::{Cell, ColumnId, RevisionId},
};

#[derive(Debug, Clone, PartialEq)]
pub struct FrequenciesRequest {
    pub columns: Vec<ColumnId>,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramBins {
    Automatic,
    Count(u16),
}
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramRequest {
    pub variable: ColumnId,
    pub bins: HistogramBins,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterRequest {
    pub x: ColumnId,
    pub y: ColumnId,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BoxplotRequest {
    pub outcome: ColumnId,
    pub group: Option<ColumnId>,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyCategory {
    pub value: Cell,
    /// Index into the captured role's value labels; no per-frame label search.
    pub value_label_index: Option<usize>,
    pub rows: usize,
    pub frequency: u64,
    pub percent: Estimate,
    pub valid_percent: Estimate,
    pub cumulative_frequency: Option<u64>,
    pub cumulative_percent: Option<Estimate>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyTable {
    pub column: ColumnId,
    pub valid_rows: usize,
    pub missing_rows: usize,
    pub valid_n: u64,
    pub missing_n: u64,
    pub missing_percent: Estimate,
    pub categories: Vec<FrequencyCategory>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramBin {
    pub left: f64,
    pub right: f64,
    pub rows: usize,
    pub frequency: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterPoint {
    pub row: usize,
    pub x: f64,
    pub y: f64,
    pub frequency: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BoxplotGroup {
    pub value: Option<Cell>,
    pub value_label_index: Option<usize>,
    pub rows: usize,
    pub n: u64,
    pub minimum: Estimate,
    pub q1: Estimate,
    pub median: Estimate,
    pub q3: Estimate,
    pub maximum: Estimate,
    pub whiskers: [Estimate; 2],
    pub outlier_rows: usize,
    pub outlier_n: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BoxplotPoint {
    pub group_index: usize,
    pub row: usize,
    pub value: f64,
    pub frequency: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedFrequencies {
    pub revision: RevisionId,
    pub request: FrequenciesRequest,
    pub columns: Vec<RunColumnInfo>,
    pub source_rows: usize,
    pub diagnostics: RunDiagnostics,
    pub tables: Vec<FrequencyTable>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedHistogram {
    pub revision: RevisionId,
    pub request: HistogramRequest,
    pub columns: Vec<RunColumnInfo>,
    pub source_rows: usize,
    pub diagnostics: RunDiagnostics,
    pub valid_rows: usize,
    pub n: u64,
    pub missing_rows: usize,
    pub missing_n: u64,
    pub range: [Estimate; 2],
    pub bins: Vec<HistogramBin>,
    pub bin_adjustment: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedScatter {
    pub revision: RevisionId,
    pub request: ScatterRequest,
    pub columns: Vec<RunColumnInfo>,
    pub source_rows: usize,
    pub diagnostics: RunDiagnostics,
    pub valid_rows: usize,
    pub n: u64,
    pub missing_rows: usize,
    pub missing_n: u64,
    pub ranges: [[Estimate; 2]; 2],
    pub points: Vec<ScatterPoint>,
    pub reduced: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedBoxplot {
    pub revision: RevisionId,
    pub request: BoxplotRequest,
    pub columns: Vec<RunColumnInfo>,
    pub source_rows: usize,
    pub diagnostics: RunDiagnostics,
    pub missing_group_rows: usize,
    pub missing_group_n: u64,
    pub missing_outcome_rows: usize,
    pub missing_outcome_n: u64,
    pub groups: Vec<BoxplotGroup>,
    pub points: Vec<BoxplotPoint>,
    pub reduced: bool,
    pub empty_reason: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum RequestRef<'a> {
    Frequencies(&'a FrequenciesRequest),
    Histogram(&'a HistogramRequest),
    Scatter(&'a ScatterRequest),
    Boxplot(&'a BoxplotRequest),
}
impl<'a> RequestRef<'a> {
    pub(super) fn filter(self) -> &'a RowFilter {
        match self {
            Self::Frequencies(r) => &r.filter,
            Self::Histogram(r) => &r.filter,
            Self::Scatter(r) => &r.filter,
            Self::Boxplot(r) => &r.filter,
        }
    }
    pub(super) fn weight(self) -> Option<ColumnId> {
        match self {
            Self::Frequencies(r) => r.frequency_weight,
            Self::Histogram(r) => r.frequency_weight,
            Self::Scatter(r) => r.frequency_weight,
            Self::Boxplot(r) => r.frequency_weight,
        }
    }
    pub(super) fn role_count(self) -> usize {
        match self {
            Self::Frequencies(r) => r.columns.len(),
            Self::Histogram(_) => 1,
            Self::Scatter(_) => 2,
            Self::Boxplot(r) => 1 + usize::from(r.group.is_some()),
        }
    }
}
