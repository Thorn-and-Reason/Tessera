//! Current tagged history and inference payload adapters.
use super::wire::*;
use crate::{analysis::*, data::*, operation::RunId, project::AnalysisRun};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::{fmt, sync::Arc};
scalar_enum!(Confidence, ConfidenceLevel, P90 => "90%", P95 => "95%", P99 => "99%");
scalar_enum!(CorrelationPolicy, CorrelationMissingPolicy, PairwiseComplete => "pairwise_complete", CompleteCases => "complete_cases");
scalar_enum!(Method, IndependentMethod, Welch => "welch", Pooled => "pooled");
#[derive(Serialize, Deserialize)]
#[serde(remote = "PearsonRequest", deny_unknown_fields)]
pub(super) struct PearsonRecipe {
    #[serde(with = "column_ids")]
    columns: Vec<ColumnId>,
    #[serde(with = "CorrelationPolicy")]
    missing_policy: CorrelationMissingPolicy,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "OneSampleTRequest", deny_unknown_fields)]
pub(super) struct OneRecipe {
    #[serde(with = "column_id")]
    variable: ColumnId,
    reference_mean: f64,
    #[serde(with = "Confidence")]
    confidence: ConfidenceLevel,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "PairedTRequest", deny_unknown_fields)]
pub(super) struct PairedRecipe {
    #[serde(with = "column_id")]
    first: ColumnId,
    #[serde(with = "column_id")]
    second: ColumnId,
    #[serde(with = "Confidence")]
    confidence: ConfidenceLevel,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "IndependentTRequest", deny_unknown_fields)]
pub(super) struct IndependentRecipe {
    #[serde(with = "column_id")]
    outcome: ColumnId,
    #[serde(with = "column_id")]
    group: ColumnId,
    #[serde(with = "group_cells")]
    groups: [Cell; 2],
    #[serde(with = "Method")]
    method: IndependentMethod,
    #[serde(with = "Confidence")]
    confidence: ConfidenceLevel,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "CorrelationRow", deny_unknown_fields)]
pub(super) struct Correlation {
    #[serde(with = "column_id")]
    first: ColumnId,
    #[serde(with = "column_id")]
    second: ColumnId,
    n: u64,
    #[serde(with = "EstimateWire")]
    r: Estimate,
    #[serde(with = "EstimateWire")]
    p: Estimate,
    missing_rows: usize,
    complete_case_excluded_rows: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "SampleSummary", deny_unknown_fields)]
pub(super) struct Sample {
    n: u64,
    #[serde(with = "EstimateWire")]
    mean: Estimate,
    #[serde(with = "EstimateWire")]
    sample_sd: Estimate,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "TStatistics", deny_unknown_fields)]
pub(super) struct Test {
    #[serde(with = "EstimateWire")]
    difference: Estimate,
    #[serde(with = "EstimateWire")]
    standard_error: Estimate,
    #[serde(with = "EstimateWire")]
    t: Estimate,
    #[serde(with = "EstimateWire")]
    df: Estimate,
    #[serde(with = "EstimateWire")]
    p: Estimate,
    #[serde(with = "interval")]
    interval: [Estimate; 2],
    #[serde(with = "EstimateWire")]
    effect_size: Estimate,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "GroupExclusions", deny_unknown_fields)]
pub(super) struct Exclusions {
    missing_group_rows: usize,
    other_group_rows: usize,
    missing_outcome_rows: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedPearson", deny_unknown_fields)]
pub(super) struct Pearson {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "PearsonRecipe")]
    request: PearsonRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    #[serde(with = "correlations")]
    rows: Vec<CorrelationRow>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedOneSampleT", deny_unknown_fields)]
pub(super) struct One {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "OneRecipe")]
    request: OneSampleTRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    #[serde(with = "Sample")]
    sample: SampleSummary,
    missing_rows: usize,
    #[serde(with = "Test")]
    statistics: TStatistics,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedPairedT", deny_unknown_fields)]
pub(super) struct Paired {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "PairedRecipe")]
    request: PairedTRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    #[serde(with = "Sample")]
    first: SampleSummary,
    #[serde(with = "Sample")]
    second: SampleSummary,
    #[serde(with = "EstimateWire")]
    difference_sd: Estimate,
    #[serde(with = "Test")]
    statistics: TStatistics,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedIndependentT", deny_unknown_fields)]
pub(super) struct Independent {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "IndependentRecipe")]
    request: IndependentTRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    #[serde(with = "samples")]
    groups: [SampleSummary; 2],
    #[serde(with = "Exclusions")]
    exclusions: GroupExclusions,
    #[serde(with = "Test")]
    statistics: TStatistics,
}
mod group_cells {
    use super::*;
    #[derive(Serialize)]
    struct Ref<'a>(#[serde(with = "cell")] &'a Cell);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "cell")] Cell);
    pub fn serialize<S: Serializer>(values: &[Cell; 2], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(values.iter().map(Ref))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[Cell; 2], D::Error> {
        <[Owned; 2]>::deserialize(d).map(|[Owned(a), Owned(b)]| [a, b])
    }
}
pub(super) mod interval {
    use super::*;
    #[derive(Serialize)]
    struct Ref<'a>(#[serde(with = "EstimateWire")] &'a Estimate);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "EstimateWire")] Estimate);
    pub fn serialize<S: Serializer>(values: &[Estimate; 2], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(values.iter().map(Ref))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[Estimate; 2], D::Error> {
        <[Owned; 2]>::deserialize(d).map(|[Owned(a), Owned(b)]| [a, b])
    }
}
mod samples {
    use super::*;
    #[derive(Serialize)]
    struct Ref<'a>(#[serde(with = "Sample")] &'a SampleSummary);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "Sample")] SampleSummary);
    pub fn serialize<S: Serializer>(values: &[SampleSummary; 2], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(values.iter().map(Ref))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[SampleSummary; 2], D::Error> {
        <[Owned; 2]>::deserialize(d).map(|[Owned(a), Owned(b)]| [a, b])
    }
}
remote_vector!(correlations, CorrelationRow, "Correlation");
#[derive(Serialize, Deserialize)]
#[serde(remote = "CompletedAnalysis", rename_all = "snake_case")]
pub(super) enum Completed {
    Reliability(#[serde(with = "super::wire_v6::ReliabilityRunWire")] reliability::ReliabilityRun),
    Anova(#[serde(with = "super::wire_v5::ComputedAnovaWire")] ComputedAnova),
    Regression(#[serde(with = "super::wire_v4::ComputedRegressionWire")] ComputedRegression),
    Describe(#[serde(with = "Analysis")] ComputedAnalysis),
    Frequencies(#[serde(with = "super::wire_v3::ComputedFrequenciesWire")] ComputedFrequencies),
    Histogram(#[serde(with = "super::wire_v3::ComputedHistogramWire")] ComputedHistogram),
    Scatter(#[serde(with = "super::wire_v3::ComputedScatterWire")] ComputedScatter),
    Boxplot(#[serde(with = "super::wire_v3::ComputedBoxplotWire")] ComputedBoxplot),
    Pearson(#[serde(with = "Pearson")] ComputedPearson),
    OneSampleT(#[serde(with = "One")] ComputedOneSampleT),
    PairedT(#[serde(with = "Paired")] ComputedPairedT),
    IndependentT(#[serde(with = "Independent")] ComputedIndependentT),
}
pub(super) mod runs {
    use super::*;
    #[derive(Serialize)]
    struct Ref<'a> {
        #[serde(with = "run_id")]
        id: RunId,
        #[serde(with = "Completed")]
        analysis: &'a CompletedAnalysis,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Owned {
        #[serde(with = "run_id")]
        id: RunId,
        #[serde(with = "Completed")]
        analysis: CompletedAnalysis,
    }
    pub fn serialize<S: Serializer>(runs: &[Arc<AnalysisRun>], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(runs.iter().map(|r| Ref {
            id: r.id,
            analysis: &r.analysis,
        }))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Arc<AnalysisRun>>, D::Error> {
        struct Runs;
        impl<'de> Visitor<'de> for Runs {
            type Value = Vec<Arc<AnalysisRun>>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("saved analysis runs")
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut runs = Vec::new();
                while let Some(Owned { id, analysis }) = seq.next_element()? {
                    runs.push(Arc::new(AnalysisRun { id, analysis }));
                }
                Ok(runs)
            }
        }
        d.deserialize_seq(Runs)
    }
}
