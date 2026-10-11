//! Private exact adapters for immutable reliability captures.
use super::wire::*;
use crate::{analysis::*, data::*};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::fmt;
scalar_enum!(Energy, ReliabilityEnergyState, Unavailable => "unavailable", ExactZero => "exact_zero", Positive => "positive", Unresolved => "unresolved");
scalar_enum!(Method, ReliabilityMethod, ExactCenteredIntervalV1 => "exact_centered_interval_v1");

#[derive(Serialize, Deserialize)]
#[serde(remote = "ReliabilityReverse", deny_unknown_fields)]
pub(super) struct ReliabilityReverseWire {
    #[serde(with = "column_id")]
    column: ColumnId,
    lower: f64,
    upper: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ReliabilityRequest", deny_unknown_fields)]
pub(super) struct ReliabilityRequestWire {
    #[serde(with = "column_ids")]
    items: Vec<ColumnId>,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
    #[serde(with = "reversals")]
    reversals: Vec<ReliabilityReverse>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ReliabilityItem", deny_unknown_fields)]
pub(super) struct ReliabilityItemWire {
    #[serde(with = "column_id")]
    column: ColumnId,
    #[serde(with = "EstimateWire")]
    mean: Estimate,
    #[serde(with = "EstimateWire")]
    sample_sd: Estimate,
    #[serde(with = "EstimateWire")]
    corrected_item_total: Estimate,
    #[serde(with = "EstimateWire")]
    raw_alpha_if_removed: Estimate,
    #[serde(with = "EstimateWire")]
    standardized_alpha_if_removed: Estimate,
    #[serde(with = "Energy")]
    energy: ReliabilityEnergyState,
    #[serde(with = "Energy")]
    remaining_energy: ReliabilityEnergyState,
    #[serde(with = "Energy")]
    remaining_standardized_energy: ReliabilityEnergyState,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ReliabilityRun", deny_unknown_fields)]
pub(super) struct ReliabilityRunWire {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "ReliabilityRequestWire")]
    request: ReliabilityRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    source_rows: usize,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    complete_rows: usize,
    frequency: u64,
    incomplete_case_frequency: u64,
    #[serde(with = "items")]
    items: Vec<ReliabilityItem>,
    #[serde(with = "EstimateWire")]
    raw_alpha: Estimate,
    #[serde(with = "EstimateWire")]
    standardized_alpha: Estimate,
    #[serde(with = "Energy")]
    total_energy: ReliabilityEnergyState,
    #[serde(with = "Energy")]
    standardized_total_energy: ReliabilityEnergyState,
    #[serde(with = "Method")]
    method: ReliabilityMethod,
}

remote_vector!(column_ids, ColumnId, "column_id");
remote_vector!(reversals, ReliabilityReverse, "ReliabilityReverseWire");
remote_vector!(items, ReliabilityItem, "ReliabilityItemWire");
