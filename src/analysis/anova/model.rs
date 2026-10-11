//! Frozen ANOVA facts; numerical workspaces never cross this boundary.
use crate::{
    analysis::{ConfidenceLevel, Estimate, RowFilter, RunColumnInfo, RunDiagnostics},
    data::{Cell, ColumnId, RevisionId},
};
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaWithinFactor {
    pub name: String,
    pub levels: Vec<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnovaDesign {
    OneWay {
        outcome: ColumnId,
        factor: ColumnId,
    },
    Factorial {
        outcome: ColumnId,
        factors: Vec<ColumnId>,
    },
    Repeated {
        subject: ColumnId,
        factors: Vec<AnovaWithinFactor>,
        measurements: Vec<ColumnId>,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnovaScope {
    Marginal,
    Fixed(Vec<Cell>),
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaPairRequest {
    pub factor: usize,
    pub first: Cell,
    pub second: Cell,
    pub scope: AnovaScope,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaFamilyRequest {
    pub factor: usize,
    pub scope: AnovaScope,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaRequest {
    pub design: AnovaDesign,
    pub confidence: ConfidenceLevel,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
    pub planned_pairs: Vec<AnovaPairRequest>,
    pub all_pairs: Vec<AnovaFamilyRequest>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaFactorLevels {
    pub column: Option<ColumnId>,
    pub name: String,
    pub values: Vec<Cell>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaCell {
    pub physical_rows: usize,
    pub frequency: u64,
    pub mean: Estimate,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaGrid {
    pub factors: Vec<AnovaFactorLevels>,
    pub cells: Vec<AnovaCell>,
    pub complete: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnovaFailureKind {
    EmptyCohort,
    CollapsedFactor,
    EmptyCells,
    InsufficientErrorDf,
    ZeroError,
    Numerical,
    PublicationRange,
    AbsentRequestedLevel,
    InvalidCovariance,
    Nonconvergence,
    IncompleteFamily,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaFailure {
    pub kind: AnovaFailureKind,
    pub reason: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnovaQuantityState {
    Positive,
    ExactZero,
    Unavailable(AnovaFailureKind),
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaEnergy {
    pub state: AnovaQuantityState,
    pub value: Estimate,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaTest {
    pub df1: Estimate,
    pub df2: Estimate,
    pub f: Estimate,
    pub p: Estimate,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaCorrection {
    pub epsilon: Estimate,
    pub covariance: AnovaQuantityState,
    pub test: AnovaTest,
    pub failure: Option<AnovaFailure>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaEffect {
    /// Nonempty factor subset in ascending ordinal order; subsets use increasing bit masks.
    pub factors: Vec<usize>,
    pub dimension: usize,
    pub ss: AnovaEnergy,
    pub ms: Estimate,
    pub error_ss: AnovaEnergy,
    pub error_ms: Estimate,
    pub test: AnovaTest,
    pub gg: Option<AnovaCorrection>,
    pub eta_squared: Estimate,
    pub partial_eta_squared: Estimate,
    pub failure: Option<AnovaFailure>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaModelResult {
    pub effects: Vec<AnovaEffect>,
    pub error_ss: AnovaEnergy,
    pub model_ss: AnovaEnergy,
    pub total_ss: AnovaEnergy,
    pub subject_ss: Option<AnovaEnergy>,
    pub residual_df: u64,
    pub total_df: u64,
    pub balanced: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnovaOutcome {
    Fitted(AnovaModelResult),
    Unavailable(AnovaFailure),
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnovaComparisonState {
    Available,
    AbsentRequestedLevel(AnovaFailure),
    Unavailable(AnovaFailure),
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaComparison {
    pub ordinal: usize,
    pub request: AnovaPairRequest,
    pub state: AnovaComparisonState,
    pub estimate: Estimate,
    pub standard_error: Estimate,
    pub t: Estimate,
    pub df: Estimate,
    pub raw_p: Estimate,
    pub adjusted_p: Estimate,
    pub interval: [Estimate; 2],
    pub variance: AnovaQuantityState,
}
#[derive(Debug, Clone, PartialEq)]
pub enum AnovaFamilyKind {
    Planned,
    AllPairs(AnovaFamilyRequest),
}
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaContrastFamily {
    pub adjustment_failure: Option<AnovaFailure>,
    pub kind: AnovaFamilyKind,
    pub m: usize,
    pub members: Vec<AnovaComparison>,
}
/// Version1 fixes full-factorial TypeIII equal-cell hypotheses, ordered grid,
/// independent frequency replication, effect-specific within errors, GG and Holm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnovaMethod {
    pub numerical_version: u32,
    pub basis_version: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedAnova {
    pub revision: RevisionId,
    pub request: AnovaRequest,
    pub columns: Vec<RunColumnInfo>,
    pub source_rows: usize,
    pub diagnostics: RunDiagnostics,
    pub fitted_rows: usize,
    pub frequency: u64,
    pub incomplete_case_frequency: u64,
    pub observed_measurements: usize,
    pub grid: AnovaGrid,
    pub method: AnovaMethod,
    pub outcome: AnovaOutcome,
    pub families: Vec<AnovaContrastFamily>,
}
