use crate::{
    analysis::{RowFilter, RunDiagnostics},
    data::ColumnId,
};
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityReverse {
    pub column: ColumnId,
    pub lower: f64,
    pub upper: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityRequest {
    pub items: Vec<ColumnId>,
    pub filter: RowFilter,
    pub frequency_weight: Option<ColumnId>,
    pub reversals: Vec<ReliabilityReverse>,
}
#[derive(Debug)]
pub struct ReliabilityCohort {
    pub(crate) eligible: Vec<super::super::selection::Eligible>,
    pub diagnostics: RunDiagnostics,
    pub complete_rows: usize,
    pub frequency: u64,
    pub incomplete_case_frequency: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReliabilityEnergyState {
    Unavailable,
    ExactZero,
    Positive,
    Unresolved,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReliabilityMethod {
    ExactCenteredIntervalV1,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReliabilityComputedItem {
    pub mean: crate::analysis::Estimate,
    pub sample_sd: crate::analysis::Estimate,
    pub corrected_item_total: crate::analysis::Estimate,
    pub raw_alpha_if_removed: crate::analysis::Estimate,
    pub standardized_alpha_if_removed: crate::analysis::Estimate,
    pub energy: ReliabilityEnergyState,
    pub remaining_energy: ReliabilityEnergyState,
    pub remaining_standardized_energy: ReliabilityEnergyState,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReliabilityComputed {
    pub items: Vec<ReliabilityComputedItem>,
    pub raw_alpha: crate::analysis::Estimate,
    pub standardized_alpha: crate::analysis::Estimate,
    pub total_energy: ReliabilityEnergyState,
    pub standardized_total_energy: ReliabilityEnergyState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityItem {
    pub column: ColumnId,
    pub mean: crate::analysis::Estimate,
    pub sample_sd: crate::analysis::Estimate,
    pub corrected_item_total: crate::analysis::Estimate,
    pub raw_alpha_if_removed: crate::analysis::Estimate,
    pub standardized_alpha_if_removed: crate::analysis::Estimate,
    pub energy: ReliabilityEnergyState,
    pub remaining_energy: ReliabilityEnergyState,
    pub remaining_standardized_energy: ReliabilityEnergyState,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityRun {
    pub revision: crate::data::RevisionId,
    pub request: ReliabilityRequest,
    pub columns: Vec<crate::analysis::RunColumnInfo>,
    pub source_rows: usize,
    pub diagnostics: RunDiagnostics,
    pub complete_rows: usize,
    pub frequency: u64,
    pub incomplete_case_frequency: u64,
    pub items: Vec<ReliabilityItem>,
    pub raw_alpha: crate::analysis::Estimate,
    pub standardized_alpha: crate::analysis::Estimate,
    pub total_energy: ReliabilityEnergyState,
    pub standardized_total_energy: ReliabilityEnergyState,
    pub method: ReliabilityMethod,
}
