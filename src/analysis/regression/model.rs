//! Immutable captured regression contract. No solver representation crosses this boundary.
use crate::{
    analysis::{ConfidenceLevel, Estimate, RowFilter, RunColumnInfo, RunDiagnostics},
    data::{Cell, ColumnId, RevisionId},
};

#[derive(Debug, Clone, PartialEq)]
pub enum PredictorRole {
    Numeric,
    Categorical { reference: Cell },
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionPredictor {
    pub column: ColumnId,
    pub role: PredictorRole,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionRequest {
    pub outcome: ColumnId,
    pub predictors: Vec<RegressionPredictor>,
    pub confidence: ConfidenceLevel,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionLevels {
    pub column: ColumnId,
    pub values: Vec<Cell>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum RegressionTerm {
    Intercept,
    Numeric {
        column: ColumnId,
    },
    Contrast {
        column: ColumnId,
        level: Cell,
        reference: Cell,
    },
}
#[derive(Debug, Clone, PartialEq)]
/// Version 1 fixes an intercept, treatment coding, centred/scaled design,
/// per-copy influence and symmetric integer-rank raw-residual Q–Q positions.
pub struct RegressionMethod {
    pub numerical_version: u32,
    pub rank_threshold: f64,
    pub condition_warning: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub enum RegressionFailureKind {
    EmptyCohort,
    AbsentReference { column: ColumnId },
    NoContrast { column: ColumnId },
    Unidentified,
    Nonconvergence,
    Numerical,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionFailure {
    pub kind: RegressionFailureKind,
    pub reason: String,
    pub expected_terms: Option<usize>,
    pub rank: Option<usize>,
    pub implicated_terms: Vec<usize>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionCoefficient {
    pub estimate: Estimate,
    pub standard_error: Estimate,
    pub t: Estimate,
    pub p: Estimate,
    pub interval: [Estimate; 2],
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionStatistics {
    pub sse: Estimate,
    pub sst: Estimate,
    pub model_ss: Estimate,
    pub residual_standard_error: Estimate,
    pub r_squared: Estimate,
    pub adjusted_r_squared: Estimate,
    pub f: Estimate,
    pub f_p: Estimate,
}
#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticExtent {
    pub range: [Estimate; 2],
    pub undefined_rows: usize,
    pub undefined_frequency: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionResidualPoint {
    pub cohort_ordinal: usize,
    pub row: usize,
    pub frequency: u64,
    pub fitted: Estimate,
    pub residual: Estimate,
    pub leverage: Estimate,
    pub studentised: Estimate,
    pub cooks_distance: Estimate,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionQqPoint {
    pub rank: u64,
    pub row: usize,
    pub frequency: u64,
    pub theoretical: Estimate,
    pub residual: Estimate,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionDiagnostics {
    pub residual_mean: Estimate,
    pub residual_sample_sd: Estimate,
    pub residual_range: [Estimate; 2],
    pub fitted_range: [Estimate; 2],
    pub leverage: DiagnosticExtent,
    pub studentised: DiagnosticExtent,
    pub cooks_distance: DiagnosticExtent,
    pub residual_points: Vec<RegressionResidualPoint>,
    pub residual_reduced: bool,
    pub qq_points: Vec<RegressionQqPoint>,
    pub qq_reduced: bool,
    pub theoretical_range: [Estimate; 2],
}
#[derive(Debug, Clone, PartialEq)]
pub struct RegressionFit {
    pub rank: usize,
    pub model_df: u64,
    pub residual_df: u64,
    pub total_df: u64,
    pub condition: Estimate,
    pub condition_warned: bool,
    pub outcome_scale: Estimate,
    pub term_scales: Vec<Estimate>,
    pub coefficients: Vec<RegressionCoefficient>,
    pub statistics: RegressionStatistics,
    pub residual_diagnostics: RegressionDiagnostics,
}
#[derive(Debug, Clone, PartialEq)]
pub enum RegressionOutcome {
    Fitted(Box<RegressionFit>),
    Unavailable(RegressionFailure),
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedRegression {
    pub revision: RevisionId,
    pub request: RegressionRequest,
    pub columns: Vec<RunColumnInfo>,
    pub source_rows: usize,
    pub diagnostics: RunDiagnostics,
    pub fitted_rows: usize,
    pub frequency: u64,
    pub incomplete_case_frequency: u64,
    pub levels: Vec<RegressionLevels>,
    pub terms: Vec<RegressionTerm>,
    pub method: RegressionMethod,
    pub outcome: RegressionOutcome,
}
