//! Private captured ANOVA adapters; central version dispatch belongs to Task 7.
//! Typed decoding must use bounded::Guard and controlled captured validation.
use super::wire::*;
use crate::{analysis::*, data::*};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::fmt;

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaWithinFactor", deny_unknown_fields)]
pub(super) struct AnovaWithinFactorWire {
    name: String,
    levels: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaPairRequest", deny_unknown_fields)]
pub(super) struct AnovaPairRequestWire {
    factor: usize,
    #[serde(with = "cell")]
    first: Cell,
    #[serde(with = "cell")]
    second: Cell,
    #[serde(with = "AnovaScopeWire")]
    scope: AnovaScope,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaFamilyRequest", deny_unknown_fields)]
pub(super) struct AnovaFamilyRequestWire {
    factor: usize,
    #[serde(with = "AnovaScopeWire")]
    scope: AnovaScope,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaRequest", deny_unknown_fields)]
pub(super) struct AnovaRequestWire {
    #[serde(with = "AnovaDesignWire")]
    design: AnovaDesign,
    #[serde(with = "super::wire_v2::Confidence")]
    confidence: ConfidenceLevel,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
    #[serde(with = "pairs")]
    planned_pairs: Vec<AnovaPairRequest>,
    #[serde(with = "family_requests")]
    all_pairs: Vec<AnovaFamilyRequest>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaFactorLevels", deny_unknown_fields)]
pub(super) struct AnovaFactorLevelsWire {
    #[serde(with = "optional_column_id")]
    column: Option<ColumnId>,
    name: String,
    #[serde(with = "cells")]
    values: Vec<Cell>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaCell", deny_unknown_fields)]
pub(super) struct AnovaCellWire {
    physical_rows: usize,
    frequency: u64,
    #[serde(with = "EstimateWire")]
    mean: Estimate,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaGrid", deny_unknown_fields)]
pub(super) struct AnovaGridWire {
    #[serde(with = "factor_levels")]
    factors: Vec<AnovaFactorLevels>,
    #[serde(with = "grid_cells")]
    cells: Vec<AnovaCell>,
    complete: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaFailure", deny_unknown_fields)]
pub(super) struct AnovaFailureWire {
    #[serde(with = "AnovaFailureKindWire")]
    kind: AnovaFailureKind,
    reason: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaEnergy", deny_unknown_fields)]
pub(super) struct AnovaEnergyWire {
    #[serde(with = "AnovaQuantityStateWire")]
    state: AnovaQuantityState,
    #[serde(with = "EstimateWire")]
    value: Estimate,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaTest", deny_unknown_fields)]
pub(super) struct AnovaTestWire {
    #[serde(with = "EstimateWire")]
    df1: Estimate,
    #[serde(with = "EstimateWire")]
    df2: Estimate,
    #[serde(with = "EstimateWire")]
    f: Estimate,
    #[serde(with = "EstimateWire")]
    p: Estimate,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaCorrection", deny_unknown_fields)]
pub(super) struct AnovaCorrectionWire {
    #[serde(with = "EstimateWire")]
    epsilon: Estimate,
    #[serde(with = "AnovaQuantityStateWire")]
    covariance: AnovaQuantityState,
    #[serde(with = "AnovaTestWire")]
    test: AnovaTest,
    #[serde(with = "optional_failure")]
    failure: Option<AnovaFailure>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaEffect", deny_unknown_fields)]
pub(super) struct AnovaEffectWire {
    factors: Vec<usize>,
    dimension: usize,
    #[serde(with = "AnovaEnergyWire")]
    ss: AnovaEnergy,
    #[serde(with = "EstimateWire")]
    ms: Estimate,
    #[serde(with = "AnovaEnergyWire")]
    error_ss: AnovaEnergy,
    #[serde(with = "EstimateWire")]
    error_ms: Estimate,
    #[serde(with = "AnovaTestWire")]
    test: AnovaTest,
    #[serde(with = "optional_correction")]
    gg: Option<AnovaCorrection>,
    #[serde(with = "EstimateWire")]
    eta_squared: Estimate,
    #[serde(with = "EstimateWire")]
    partial_eta_squared: Estimate,
    #[serde(with = "optional_failure")]
    failure: Option<AnovaFailure>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaModelResult", deny_unknown_fields)]
pub(super) struct AnovaModelResultWire {
    #[serde(with = "effects")]
    effects: Vec<AnovaEffect>,
    #[serde(with = "AnovaEnergyWire")]
    error_ss: AnovaEnergy,
    #[serde(with = "AnovaEnergyWire")]
    model_ss: AnovaEnergy,
    #[serde(with = "AnovaEnergyWire")]
    total_ss: AnovaEnergy,
    #[serde(with = "optional_energy")]
    subject_ss: Option<AnovaEnergy>,
    residual_df: u64,
    total_df: u64,
    balanced: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaComparison", deny_unknown_fields)]
pub(super) struct AnovaComparisonWire {
    ordinal: usize,
    #[serde(with = "AnovaPairRequestWire")]
    request: AnovaPairRequest,
    #[serde(with = "AnovaComparisonStateWire")]
    state: AnovaComparisonState,
    #[serde(with = "EstimateWire")]
    estimate: Estimate,
    #[serde(with = "EstimateWire")]
    standard_error: Estimate,
    #[serde(with = "EstimateWire")]
    t: Estimate,
    #[serde(with = "EstimateWire")]
    df: Estimate,
    #[serde(with = "EstimateWire")]
    raw_p: Estimate,
    #[serde(with = "EstimateWire")]
    adjusted_p: Estimate,
    #[serde(with = "super::wire_v2::interval")]
    interval: [Estimate; 2],
    #[serde(with = "AnovaQuantityStateWire")]
    variance: AnovaQuantityState,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaContrastFamily", deny_unknown_fields)]
pub(super) struct AnovaContrastFamilyWire {
    #[serde(with = "optional_failure")]
    adjustment_failure: Option<AnovaFailure>,
    #[serde(with = "AnovaFamilyKindWire")]
    kind: AnovaFamilyKind,
    m: usize,
    #[serde(with = "comparisons")]
    members: Vec<AnovaComparison>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaMethod", deny_unknown_fields)]
pub(super) struct AnovaMethodWire {
    numerical_version: u32,
    basis_version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedAnova", deny_unknown_fields)]
pub(super) struct ComputedAnovaWire {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "AnovaRequestWire")]
    request: AnovaRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    source_rows: usize,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    fitted_rows: usize,
    frequency: u64,
    incomplete_case_frequency: u64,
    observed_measurements: usize,
    #[serde(with = "AnovaGridWire")]
    grid: AnovaGrid,
    #[serde(with = "AnovaMethodWire")]
    method: AnovaMethod,
    #[serde(with = "AnovaOutcomeWire")]
    outcome: AnovaOutcome,
    #[serde(with = "families")]
    families: Vec<AnovaContrastFamily>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "AnovaDesign", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum AnovaDesignWire {
    OneWay {
        #[serde(with = "column_id")]
        outcome: ColumnId,
        #[serde(with = "column_id")]
        factor: ColumnId,
    },
    Factorial {
        #[serde(with = "column_id")]
        outcome: ColumnId,
        #[serde(with = "column_ids")]
        factors: Vec<ColumnId>,
    },
    Repeated {
        #[serde(with = "column_id")]
        subject: ColumnId,
        #[serde(with = "within_factors")]
        factors: Vec<AnovaWithinFactor>,
        #[serde(with = "column_ids")]
        measurements: Vec<ColumnId>,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(
    remote = "AnovaOutcome",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum AnovaOutcomeWire {
    Fitted(#[serde(with = "AnovaModelResultWire")] AnovaModelResult),
    Unavailable(#[serde(with = "AnovaFailureWire")] AnovaFailure),
}
#[derive(Serialize)]
#[serde(remote = "AnovaScope", rename_all = "snake_case")]
pub(super) enum AnovaScopeWire {
    Marginal,
    Fixed(#[serde(with = "cells")] Vec<Cell>),
}
#[derive(Serialize)]
#[serde(remote = "AnovaQuantityState", rename_all = "snake_case")]
pub(super) enum AnovaQuantityStateWire {
    Positive,
    ExactZero,
    Unavailable(#[serde(with = "AnovaFailureKindWire")] AnovaFailureKind),
}
#[derive(Serialize)]
#[serde(remote = "AnovaComparisonState", rename_all = "snake_case")]
pub(super) enum AnovaComparisonStateWire {
    Available,
    AbsentRequestedLevel(#[serde(with = "AnovaFailureWire")] AnovaFailure),
    Unavailable(#[serde(with = "AnovaFailureWire")] AnovaFailure),
}
#[derive(Serialize)]
#[serde(remote = "AnovaFamilyKind", rename_all = "snake_case")]
pub(super) enum AnovaFamilyKindWire {
    Planned,
    AllPairs(#[serde(with = "AnovaFamilyRequestWire")] AnovaFamilyRequest),
}
scalar_enum!(AnovaFailureKindWire,AnovaFailureKind,
EmptyCohort=>"empty_cohort",CollapsedFactor=>"collapsed_factor",EmptyCells=>"empty_cells",InsufficientErrorDf=>"insufficient_error_df",ZeroError=>"zero_error",Numerical=>"numerical",PublicationRange=>"publication_range",AbsentRequestedLevel=>"absent_requested_level",InvalidCovariance=>"invalid_covariance",Nonconvergence=>"nonconvergence",IncompleteFamily=>"incomplete_family");
remote_vector!(pairs, AnovaPairRequest, "AnovaPairRequestWire");
remote_vector!(
    family_requests,
    AnovaFamilyRequest,
    "AnovaFamilyRequestWire"
);
remote_vector!(factor_levels, AnovaFactorLevels, "AnovaFactorLevelsWire");
remote_vector!(grid_cells, AnovaCell, "AnovaCellWire");
remote_vector!(effects, AnovaEffect, "AnovaEffectWire");
remote_vector!(comparisons, AnovaComparison, "AnovaComparisonWire");
remote_vector!(families, AnovaContrastFamily, "AnovaContrastFamilyWire");
remote_vector!(within_factors, AnovaWithinFactor, "AnovaWithinFactorWire");
remote_vector!(cells, Cell, "cell");
remote_vector!(column_ids, ColumnId, "column_id");

mod optional_failure {
    use super::*;
    #[derive(Serialize)]
    struct Borrowed<'a>(#[serde(with = "AnovaFailureWire")] &'a AnovaFailure);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "AnovaFailureWire")] AnovaFailure);
    pub fn serialize<S: Serializer>(v: &Option<AnovaFailure>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().map(Borrowed).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<AnovaFailure>, D::Error> {
        Option::<Owned>::deserialize(d).map(|v| v.map(|v| v.0))
    }
}

mod optional_correction {
    use super::*;
    #[derive(Serialize)]
    struct Borrowed<'a>(#[serde(with = "AnovaCorrectionWire")] &'a AnovaCorrection);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "AnovaCorrectionWire")] AnovaCorrection);
    pub fn serialize<S: Serializer>(v: &Option<AnovaCorrection>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().map(Borrowed).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<AnovaCorrection>, D::Error> {
        Option::<Owned>::deserialize(d).map(|v| v.map(|v| v.0))
    }
}

mod optional_energy {
    use super::*;
    #[derive(Serialize)]
    struct Borrowed<'a>(#[serde(with = "AnovaEnergyWire")] &'a AnovaEnergy);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "AnovaEnergyWire")] AnovaEnergy);
    pub fn serialize<S: Serializer>(v: &Option<AnovaEnergy>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().map(Borrowed).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<AnovaEnergy>, D::Error> {
        Option::<Owned>::deserialize(d).map(|v| v.map(|v| v.0))
    }
}

mod tags;
#[cfg(test)]
mod tests;

/// Guard has already charged input, root and decoded owners. Add semantic scratch
/// before validation; Task 7 supplies the same live state at project boundaries.
pub(super) fn validate_decoded(
    result: &ComputedAnova,
    state: &std::cell::RefCell<super::bounded::Accounting<'_>>,
) -> crate::error::AppResult<()> {
    let control = state.borrow().control;
    control.check_cancelled()?;
    if result.request.heap_bytes()? > anova::MAX_REQUEST_BYTES {
        return Err(crate::error::AppError::SizeLimit {
            limit: anova::MAX_REQUEST_BYTES,
        });
    }
    state.borrow_mut().add(result.validation_scratch_bytes()?)?;
    anova::validation::completed(result, control)?;
    control.check_cancelled()
}
