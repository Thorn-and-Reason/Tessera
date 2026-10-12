//! V3 captured frequency and chart payloads; shared strict visitors enforce map-only input.
use super::wire::*;
use crate::{analysis::*, data::*};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
#[derive(Serialize, Deserialize)]
#[serde(remote = "HistogramBins", rename_all = "snake_case")]
pub(super) enum Bins {
    Automatic,
    Count(u16),
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "FrequenciesRequest", deny_unknown_fields)]
pub(super) struct FrequenciesRequestWire {
    #[serde(default, with = "optional_column_id")]
    group: Option<ColumnId>,
    #[serde(with = "column_ids")]
    columns: Vec<ColumnId>,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "HistogramRequest", deny_unknown_fields)]
pub(super) struct HistogramRequestWire {
    #[serde(with = "column_id")]
    variable: ColumnId,
    #[serde(with = "Bins")]
    bins: HistogramBins,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ScatterRequest", deny_unknown_fields)]
pub(super) struct ScatterRequestWire {
    #[serde(with = "column_id")]
    x: ColumnId,
    #[serde(with = "column_id")]
    y: ColumnId,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "BoxplotRequest", deny_unknown_fields)]
pub(super) struct BoxplotRequestWire {
    #[serde(with = "column_id")]
    outcome: ColumnId,
    #[serde(with = "optional_column_id")]
    group: Option<ColumnId>,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "FrequencyCategory", deny_unknown_fields)]
pub(super) struct FrequencyCategoryWire {
    #[serde(with = "cell")]
    value: Cell,
    #[serde(deserialize_with = "required_nullable")]
    value_label_index: Option<usize>,
    rows: usize,
    frequency: u64,
    #[serde(with = "EstimateWire")]
    percent: Estimate,
    #[serde(with = "EstimateWire")]
    valid_percent: Estimate,
    #[serde(deserialize_with = "required_nullable")]
    cumulative_frequency: Option<u64>,
    #[serde(with = "optional_estimate")]
    cumulative_percent: Option<Estimate>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "FrequencyTable", deny_unknown_fields)]
pub(super) struct FrequencyTableWire {
    #[serde(with = "column_id")]
    column: ColumnId,
    valid_rows: usize,
    missing_rows: usize,
    valid_n: u64,
    missing_n: u64,
    #[serde(with = "EstimateWire")]
    missing_percent: Estimate,
    #[serde(with = "frequencycategorys")]
    categories: Vec<FrequencyCategory>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "HistogramBin", deny_unknown_fields)]
pub(super) struct HistogramBinWire {
    left: f64,
    right: f64,
    rows: usize,
    frequency: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ScatterPoint", deny_unknown_fields)]
pub(super) struct ScatterPointWire {
    row: usize,
    x: f64,
    y: f64,
    frequency: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "BoxplotGroup", deny_unknown_fields)]
pub(super) struct BoxplotGroupWire {
    #[serde(with = "optional_cell")]
    value: Option<Cell>,
    #[serde(deserialize_with = "required_nullable")]
    value_label_index: Option<usize>,
    rows: usize,
    n: u64,
    #[serde(default = "legacy_boxplot_mean", with = "EstimateWire")]
    mean: Estimate,
    #[serde(with = "EstimateWire")]
    minimum: Estimate,
    #[serde(with = "EstimateWire")]
    q1: Estimate,
    #[serde(with = "EstimateWire")]
    median: Estimate,
    #[serde(with = "EstimateWire")]
    q3: Estimate,
    #[serde(with = "EstimateWire")]
    maximum: Estimate,
    #[serde(with = "super::wire_v2::interval")]
    whiskers: [Estimate; 2],
    outlier_rows: usize,
    outlier_n: u64,
}
fn legacy_boxplot_mean() -> Estimate {
    Estimate {
        value: None,
        reason: Some("Mean was not captured. Rerun the boxplot analysis.".into()),
    }
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "BoxplotPoint", deny_unknown_fields)]
pub(super) struct BoxplotPointWire {
    group_index: usize,
    row: usize,
    value: f64,
    frequency: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedFrequencies", deny_unknown_fields)]
pub(super) struct ComputedFrequenciesWire {
    #[serde(default, with = "frequencygroups")]
    groups: Vec<FrequencyGroup>,
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "FrequenciesRequestWire")]
    request: FrequenciesRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    source_rows: usize,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    #[serde(with = "frequencytables")]
    tables: Vec<FrequencyTable>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedHistogram", deny_unknown_fields)]
pub(super) struct ComputedHistogramWire {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "HistogramRequestWire")]
    request: HistogramRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    source_rows: usize,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    valid_rows: usize,
    n: u64,
    missing_rows: usize,
    missing_n: u64,
    #[serde(with = "super::wire_v2::interval")]
    range: [Estimate; 2],
    #[serde(with = "histogrambins")]
    bins: Vec<HistogramBin>,
    #[serde(deserialize_with = "required_nullable")]
    bin_adjustment: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedScatter", deny_unknown_fields)]
pub(super) struct ComputedScatterWire {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "ScatterRequestWire")]
    request: ScatterRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    source_rows: usize,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    valid_rows: usize,
    n: u64,
    missing_rows: usize,
    missing_n: u64,
    #[serde(with = "ranges")]
    ranges: [[Estimate; 2]; 2],
    #[serde(with = "scatterpoints")]
    points: Vec<ScatterPoint>,
    reduced: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedBoxplot", deny_unknown_fields)]
pub(super) struct ComputedBoxplotWire {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "BoxplotRequestWire")]
    request: BoxplotRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    source_rows: usize,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    missing_group_rows: usize,
    missing_group_n: u64,
    missing_outcome_rows: usize,
    missing_outcome_n: u64,
    #[serde(with = "boxplotgroups")]
    groups: Vec<BoxplotGroup>,
    #[serde(with = "boxplotpoints")]
    points: Vec<BoxplotPoint>,
    reduced: bool,
    #[serde(deserialize_with = "required_nullable")]
    empty_reason: Option<String>,
}
remote_vector!(
    frequencycategorys,
    FrequencyCategory,
    "FrequencyCategoryWire"
);
remote_vector!(frequencytables, FrequencyTable, "FrequencyTableWire");
remote_vector!(histogrambins, HistogramBin, "HistogramBinWire");
remote_vector!(scatterpoints, ScatterPoint, "ScatterPointWire");
remote_vector!(boxplotgroups, BoxplotGroup, "BoxplotGroupWire");
remote_vector!(boxplotpoints, BoxplotPoint, "BoxplotPointWire");
mod optional_cell {
    use super::*;
    #[derive(Serialize)]
    struct Ref<'a>(#[serde(with = "cell")] &'a Cell);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "cell")] Cell);
    pub fn serialize<S: Serializer>(v: &Option<Cell>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().map(Ref).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Cell>, D::Error> {
        Option::<Owned>::deserialize(d).map(|v| v.map(|v| v.0))
    }
}
mod optional_estimate {
    use super::*;
    #[derive(Serialize)]
    struct Ref<'a>(#[serde(with = "EstimateWire")] &'a Estimate);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "EstimateWire")] Estimate);
    pub fn serialize<S: Serializer>(v: &Option<Estimate>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().map(Ref).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Estimate>, D::Error> {
        Option::<Owned>::deserialize(d).map(|v| v.map(|v| v.0))
    }
}
mod ranges {
    use super::*;
    #[derive(Serialize)]
    struct Ref<'a>(#[serde(with = "super::super::wire_v2::interval")] &'a [Estimate; 2]);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "super::super::wire_v2::interval")] [Estimate; 2]);
    pub fn serialize<S: Serializer>(v: &[[Estimate; 2]; 2], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(v.iter().map(Ref))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[[Estimate; 2]; 2], D::Error> {
        <[Owned; 2]>::deserialize(d).map(|[Owned(a), Owned(b)]| [a, b])
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "FrequencyGroup", deny_unknown_fields)]
pub(super) struct FrequencyGroupWire {
    #[serde(with = "cell")]
    value: Cell,
    #[serde(deserialize_with = "required_nullable")]
    value_label_index: Option<usize>,
    rows: usize,
    frequency: u64,
    #[serde(with = "frequencytables")]
    tables: Vec<FrequencyTable>,
}
remote_vector!(frequencygroups, FrequencyGroup, "FrequencyGroupWire");
