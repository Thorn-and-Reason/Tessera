//! Standalone v4 regression payload adapters; version dispatch is integrated separately.
//! Deserialize only through project_io::bounded::Guard, as for earlier wire families.
use super::wire::*;
use crate::{analysis::*, data::*};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::fmt;

#[derive(Serialize)]
#[serde(
    remote = "PredictorRole",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum PredictorRoleWire {
    Numeric,
    Categorical {
        #[serde(with = "cell")]
        reference: Cell,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionPredictor", deny_unknown_fields)]
pub(super) struct RegressionPredictorWire {
    #[serde(with = "column_id")]
    column: ColumnId,
    #[serde(with = "PredictorRoleWire")]
    role: PredictorRole,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionRequest", deny_unknown_fields)]
pub(super) struct RegressionRequestWire {
    #[serde(with = "column_id")]
    outcome: ColumnId,
    #[serde(with = "predictors")]
    predictors: Vec<RegressionPredictor>,
    #[serde(with = "super::wire_v2::Confidence")]
    confidence: ConfidenceLevel,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionLevels", deny_unknown_fields)]
pub(super) struct RegressionLevelsWire {
    #[serde(with = "column_id")]
    column: ColumnId,
    #[serde(with = "cells")]
    values: Vec<Cell>,
}

#[derive(Serialize)]
#[serde(
    remote = "RegressionTerm",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum RegressionTermWire {
    Intercept,
    Numeric {
        #[serde(with = "column_id")]
        column: ColumnId,
    },
    Contrast {
        #[serde(with = "column_id")]
        column: ColumnId,
        #[serde(with = "cell")]
        level: Cell,
        #[serde(with = "cell")]
        reference: Cell,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionMethod", deny_unknown_fields)]
pub(super) struct RegressionMethodWire {
    numerical_version: u32,
    rank_threshold: f64,
    condition_warning: f64,
}

#[derive(Serialize)]
#[serde(
    remote = "RegressionFailureKind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum RegressionFailureKindWire {
    EmptyCohort,
    AbsentReference {
        #[serde(with = "column_id")]
        column: ColumnId,
    },
    NoContrast {
        #[serde(with = "column_id")]
        column: ColumnId,
    },
    Unidentified,
    Nonconvergence,
    Numerical,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionFailure", deny_unknown_fields)]
pub(super) struct RegressionFailureWire {
    #[serde(with = "RegressionFailureKindWire")]
    kind: RegressionFailureKind,
    reason: String,
    #[serde(deserialize_with = "required_nullable")]
    expected_terms: Option<usize>,
    #[serde(deserialize_with = "required_nullable")]
    rank: Option<usize>,
    implicated_terms: Vec<usize>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionCoefficient", deny_unknown_fields)]
pub(super) struct RegressionCoefficientWire {
    #[serde(with = "EstimateWire")]
    estimate: Estimate,
    #[serde(with = "EstimateWire")]
    standard_error: Estimate,
    #[serde(with = "EstimateWire")]
    t: Estimate,
    #[serde(with = "EstimateWire")]
    p: Estimate,
    #[serde(with = "super::wire_v2::interval")]
    interval: [Estimate; 2],
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionStatistics", deny_unknown_fields)]
pub(super) struct RegressionStatisticsWire {
    #[serde(with = "EstimateWire")]
    sse: Estimate,
    #[serde(with = "EstimateWire")]
    sst: Estimate,
    #[serde(with = "EstimateWire")]
    model_ss: Estimate,
    #[serde(with = "EstimateWire")]
    residual_standard_error: Estimate,
    #[serde(with = "EstimateWire")]
    r_squared: Estimate,
    #[serde(with = "EstimateWire")]
    adjusted_r_squared: Estimate,
    #[serde(with = "EstimateWire")]
    f: Estimate,
    #[serde(with = "EstimateWire")]
    f_p: Estimate,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "DiagnosticExtent", deny_unknown_fields)]
pub(super) struct DiagnosticExtentWire {
    #[serde(with = "super::wire_v2::interval")]
    range: [Estimate; 2],
    undefined_rows: usize,
    undefined_frequency: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionResidualPoint", deny_unknown_fields)]
pub(super) struct RegressionResidualPointWire {
    cohort_ordinal: usize,
    row: usize,
    frequency: u64,
    #[serde(with = "EstimateWire")]
    fitted: Estimate,
    #[serde(with = "EstimateWire")]
    residual: Estimate,
    #[serde(with = "EstimateWire")]
    leverage: Estimate,
    #[serde(with = "EstimateWire")]
    studentised: Estimate,
    #[serde(with = "EstimateWire")]
    cooks_distance: Estimate,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionQqPoint", deny_unknown_fields)]
pub(super) struct RegressionQqPointWire {
    rank: u64,
    row: usize,
    frequency: u64,
    #[serde(with = "EstimateWire")]
    theoretical: Estimate,
    #[serde(with = "EstimateWire")]
    residual: Estimate,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionDiagnostics", deny_unknown_fields)]
pub(super) struct RegressionDiagnosticsWire {
    #[serde(with = "EstimateWire")]
    residual_mean: Estimate,
    #[serde(with = "EstimateWire")]
    residual_sample_sd: Estimate,
    #[serde(with = "super::wire_v2::interval")]
    residual_range: [Estimate; 2],
    #[serde(with = "super::wire_v2::interval")]
    fitted_range: [Estimate; 2],
    #[serde(with = "DiagnosticExtentWire")]
    leverage: DiagnosticExtent,
    #[serde(with = "DiagnosticExtentWire")]
    studentised: DiagnosticExtent,
    #[serde(with = "DiagnosticExtentWire")]
    cooks_distance: DiagnosticExtent,
    #[serde(with = "residual_points")]
    residual_points: Vec<RegressionResidualPoint>,
    residual_reduced: bool,
    #[serde(with = "qq_points")]
    qq_points: Vec<RegressionQqPoint>,
    qq_reduced: bool,
    #[serde(with = "super::wire_v2::interval")]
    theoretical_range: [Estimate; 2],
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "RegressionFit", deny_unknown_fields)]
pub(super) struct RegressionFitWire {
    rank: usize,
    model_df: u64,
    residual_df: u64,
    total_df: u64,
    #[serde(with = "EstimateWire")]
    condition: Estimate,
    condition_warned: bool,
    #[serde(with = "EstimateWire")]
    outcome_scale: Estimate,
    #[serde(with = "estimate_values")]
    term_scales: Vec<Estimate>,
    #[serde(with = "coefficients")]
    coefficients: Vec<RegressionCoefficient>,
    #[serde(with = "RegressionStatisticsWire")]
    statistics: RegressionStatistics,
    #[serde(with = "RegressionDiagnosticsWire")]
    residual_diagnostics: RegressionDiagnostics,
}

#[derive(Serialize, Deserialize)]
#[serde(
    remote = "RegressionOutcome",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum RegressionOutcomeWire {
    Fitted(#[serde(with = "boxed_fit")] Box<RegressionFit>),
    Unavailable(#[serde(with = "RegressionFailureWire")] RegressionFailure),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedRegression", deny_unknown_fields)]
pub(super) struct ComputedRegressionWire {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "RegressionRequestWire")]
    request: RegressionRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    source_rows: usize,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    fitted_rows: usize,
    frequency: u64,
    incomplete_case_frequency: u64,
    #[serde(with = "levels")]
    levels: Vec<RegressionLevels>,
    #[serde(with = "terms")]
    terms: Vec<RegressionTerm>,
    #[serde(with = "RegressionMethodWire")]
    method: RegressionMethod,
    #[serde(with = "RegressionOutcomeWire")]
    outcome: RegressionOutcome,
}
remote_vector!(predictors, RegressionPredictor, "RegressionPredictorWire");
remote_vector!(levels, RegressionLevels, "RegressionLevelsWire");
remote_vector!(terms, RegressionTerm, "RegressionTermWire");
remote_vector!(
    coefficients,
    RegressionCoefficient,
    "RegressionCoefficientWire"
);
remote_vector!(
    residual_points,
    RegressionResidualPoint,
    "RegressionResidualPointWire"
);
remote_vector!(qq_points, RegressionQqPoint, "RegressionQqPointWire");
remote_vector!(cells, Cell, "cell");
remote_vector!(estimate_values, Estimate, "EstimateWire");

mod tags;
#[cfg(test)]
mod tests;

mod boxed_fit {
    use super::*;
    pub fn serialize<S: Serializer>(fit: &RegressionFit, s: S) -> Result<S::Ok, S::Error> {
        RegressionFitWire::serialize(fit, s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Box<RegressionFit>, D::Error> {
        struct Fit;
        impl<'de> serde::de::Visitor<'de> for Fit {
            type Value = Box<RegressionFit>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a captured regression fit")
            }
            fn visit_newtype_struct<D: Deserializer<'de>>(
                self,
                d: D,
            ) -> Result<Self::Value, D::Error> {
                RegressionFitWire::deserialize(d).map(Box::new)
            }
        }
        d.deserialize_newtype_struct("RegressionFitBox", Fit)
    }
}
