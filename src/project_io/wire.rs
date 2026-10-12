//! Current dataset and descriptive adapters, independent of runtime chunks and jobs.
use crate::{analysis::*, data::*, operation::RunId};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use std::{collections::BTreeMap, fmt};

macro_rules! scalar_enum {
    ($wire:ident, $core:ident, $($variant:ident => $name:literal),+ $(,)?) => {
        pub(crate) struct $wire;
        impl $wire {
            pub fn serialize<S: Serializer>(value: &$core, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(match value { $($core::$variant => $name,)+ })
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<$core, D::Error> {
                struct Scalar;
                impl Visitor<'_> for Scalar {
                    type Value = $core;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("a documented enum string") }
                    fn visit_str<E: de::Error>(self, value: &str) -> Result<$core, E> {
                        match value { $($name => Ok($core::$variant),)+ _ => Err(E::custom("Unknown enum string.")) }
                    }
                }
                d.deserialize_str(Scalar)
            }
        }
    };
}
pub(super) use scalar_enum;
scalar_enum!(Kind, ColumnKind, Number => "number", Text => "text");
scalar_enum!(Measure, Measurement, Unknown => "unknown", Nominal => "nominal", Ordinal => "ordinal", Scale => "scale");
#[derive(Serialize)]
#[serde(
    remote = "MissingRules",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum Rules {
    None,
    Number {
        values: Vec<f64>,
        ranges: Vec<(f64, f64)>,
    },
    Text(Vec<String>),
}
impl Rules {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<MissingRules, D::Error> {
        struct RulesVisitor;
        impl<'de> Visitor<'de> for RulesVisitor {
            type Value = MissingRules;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("none or a number/text rule object")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<MissingRules, E> {
                if value == "none" {
                    Ok(MissingRules::None)
                } else {
                    Err(E::custom("Unknown missing rules."))
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<MissingRules, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Numeric {
                    values: Vec<f64>,
                    ranges: Vec<(f64, f64)>,
                }
                let rule = match map.next_key::<String>()?.as_deref() {
                    Some("number") => {
                        let Numeric { values, ranges } = map.next_value()?;
                        MissingRules::Number { values, ranges }
                    }
                    Some("text") => MissingRules::Text(map.next_value()?),
                    _ => {
                        return Err(de::Error::custom(
                            "Missing rules require a number/text object.",
                        ))
                    }
                };
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("Missing rules require exactly one tag."));
                }
                Ok(rule)
            }
        }
        d.deserialize_any(RulesVisitor)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ValueLabel", deny_unknown_fields)]
pub(super) struct Label {
    #[serde(with = "cell")]
    value: Cell,
    label: String,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ColumnMetadata", deny_unknown_fields)]
pub(super) struct Metadata {
    name: String,
    description: String,
    #[serde(with = "Measure")]
    measurement: Measurement,
    #[serde(with = "Rules")]
    missing_rules: MissingRules,
    #[serde(with = "labels")]
    value_labels: Vec<ValueLabel>,
    display_decimals: u8,
}
scalar_enum!(Stat, Statistic, Mean => "mean", SampleStdDev => "sample_std_dev", Variance => "variance", Minimum => "minimum", Maximum => "maximum", Sum => "sum", Range => "range", StdErrorMean => "std_error_mean");
scalar_enum!(Policy, MissingPolicy, PerVariable => "per_variable", CompleteCase => "complete_case");
scalar_enum!(Comparison, NumericComparison, Equal => "equal", NotEqual => "not_equal", Less => "less", LessOrEqual => "less_or_equal", Greater => "greater", GreaterOrEqual => "greater_or_equal");
#[derive(Serialize, Deserialize)]
#[serde(
    remote = "RowPredicate",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum Predicate {
    Number {
        #[serde(with = "column_id")]
        column: ColumnId,
        #[serde(with = "Comparison")]
        comparison: NumericComparison,
        value: f64,
    },
    Text {
        #[serde(with = "column_id")]
        column: ColumnId,
        value: String,
        equal: bool,
    },
    Missing {
        #[serde(with = "column_id")]
        column: ColumnId,
        missing: bool,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "RowFilter", deny_unknown_fields)]
pub(super) struct Filter {
    #[serde(with = "predicates")]
    predicates: Vec<RowPredicate>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "DescribeRequest", deny_unknown_fields)]
pub(super) struct Request {
    #[serde(with = "column_ids")]
    columns: Vec<ColumnId>,
    #[serde(with = "statistics")]
    statistics: Vec<Statistic>,
    #[serde(with = "Policy")]
    missing_policy: MissingPolicy,
    #[serde(with = "Filter")]
    filter: RowFilter,
    #[serde(with = "optional_column_id")]
    frequency_weight: Option<ColumnId>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "Estimate", deny_unknown_fields)]
pub(super) struct EstimateWire {
    #[serde(deserialize_with = "required_nullable")]
    value: Option<f64>,
    #[serde(deserialize_with = "required_nullable")]
    reason: Option<String>,
}
pub(super) fn required_nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "DescriptiveRow", deny_unknown_fields)]
pub(super) struct Row {
    #[serde(with = "column_id")]
    column: ColumnId,
    valid_n: u64,
    missing_n: u64,
    complete_case_excluded_n: u64,
    #[serde(with = "estimates")]
    estimates: BTreeMap<Statistic, Estimate>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "RunDiagnostics", deny_unknown_fields)]
pub(super) struct Diagnostics {
    filtered_rows: usize,
    missing_weight_rows: usize,
    zero_weight_rows: usize,
    complete_case_rows: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "RunColumnInfo", deny_unknown_fields)]
pub(super) struct Info {
    #[serde(with = "column_id")]
    id: ColumnId,
    name: String,
    #[serde(with = "Kind")]
    kind: ColumnKind,
    description: String,
    #[serde(with = "Rules")]
    missing_rules: MissingRules,
    #[serde(with = "labels")]
    value_labels: Vec<ValueLabel>,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "PlotPoint", deny_unknown_fields)]
pub(super) struct Point {
    row: usize,
    value: f64,
    frequency: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "PlotData", deny_unknown_fields)]
pub(super) struct Plot {
    #[serde(with = "column_id")]
    column: ColumnId,
    #[serde(with = "points")]
    points: Vec<PlotPoint>,
    source_rows: usize,
    reduced: bool,
    label: String,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "ComputedAnalysis", deny_unknown_fields)]
pub(super) struct Analysis {
    #[serde(with = "revision_id")]
    revision: RevisionId,
    #[serde(with = "Request")]
    request: DescribeRequest,
    #[serde(with = "infos")]
    columns: Vec<RunColumnInfo>,
    #[serde(with = "rows")]
    rows: Vec<DescriptiveRow>,
    #[serde(with = "Diagnostics")]
    diagnostics: RunDiagnostics,
    #[serde(with = "plots")]
    plots: Vec<PlotData>,
}
macro_rules! id_module {
    ($name:ident, $ty:ident) => {
        pub(crate) mod $name {
            use super::*;
            pub fn serialize<S: Serializer>(id: &$ty, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(&id.0)
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$ty, D::Error> {
                struct IdVisitor;
                impl Visitor<'_> for IdVisitor {
                    type Value = $ty;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str("a positive canonical decimal u64 string")
                    }
                    fn visit_str<E: de::Error>(self, text: &str) -> Result<$ty, E> {
                        if text.is_empty()
                            || text.starts_with('0')
                            || !text.bytes().all(|byte| byte.is_ascii_digit())
                        {
                            return Err(E::custom("Invalid decimal identity."));
                        }
                        text.parse::<u64>()
                            .map($ty)
                            .map_err(|_| E::custom("Identity exceeds u64."))
                    }
                }
                deserializer.deserialize_str(IdVisitor)
            }
        }
    };
}
id_module!(column_id, ColumnId);
id_module!(revision_id, RevisionId);
id_module!(run_id, RunId);
// Remote wrappers keep serde out of the shared model and borrow every saved value.
macro_rules! remote_vector {
    ($name:ident, $ty:ty, $remote:literal) => {
        remote_vector!(pub(super) $name, $ty, $remote);
    };
    ($visibility:vis $name:ident, $ty:ty, $remote:literal) => {
        $visibility mod $name {
            use super::*;
            #[derive(Serialize)]
            struct Ref<'a>(#[serde(with = $remote)] &'a $ty);
            #[derive(Deserialize)]
            struct Owned(#[serde(with = $remote)] $ty);
            pub fn serialize<S: Serializer>(
                values: &[$ty],
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                serializer.collect_seq(values.iter().map(Ref))
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Vec<$ty>, D::Error> {
                // Reuse the wrapper vector allocation rather than collecting a second vector.
                struct Values;
                impl<'de> Visitor<'de> for Values {
                    type Value = Vec<$ty>;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str("an array")
                    }
                    fn visit_seq<A: de::SeqAccess<'de>>(
                        self,
                        mut seq: A,
                    ) -> Result<Self::Value, A::Error> {
                        let mut values = Vec::new();
                        while let Some(Owned(value)) = seq.next_element()? {
                            values.push(value);
                        }
                        Ok(values)
                    }
                }
                deserializer.deserialize_seq(Values)
            }
        }
    };
}
pub(super) use remote_vector;
remote_vector!(labels, ValueLabel, "Label");
remote_vector!(pub(crate) column_ids, ColumnId, "column_id");
remote_vector!(statistics, Statistic, "Stat");
remote_vector!(predicates, RowPredicate, "Predicate");
remote_vector!(infos, RunColumnInfo, "Info");
remote_vector!(rows, DescriptiveRow, "Row");
remote_vector!(points, PlotPoint, "Point");
remote_vector!(plots, PlotData, "Plot");

pub(super) mod optional_column_id {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Id(#[serde(with = "column_id")] ColumnId);
    pub fn serialize<S: Serializer>(
        id: &Option<ColumnId>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        id.map(Id).serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<ColumnId>, D::Error> {
        Option::<Id>::deserialize(deserializer).map(|id| id.map(|id| id.0))
    }
}
pub(crate) mod cell {
    use super::*;
    pub fn serialize<S: Serializer>(cell: &Cell, serializer: S) -> Result<S::Ok, S::Error> {
        match cell {
            Cell::Missing => serializer.serialize_none(),
            Cell::Number(value) => serializer.serialize_f64(*value),
            Cell::Text(value) => serializer.serialize_str(value),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Cell, D::Error> {
        struct CellVisitor;
        impl Visitor<'_> for CellVisitor {
            type Value = Cell;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a finite Float64, text or null")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Cell, E> {
                Ok(Cell::Missing)
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Cell, E> {
                Ok(Cell::Text(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Cell, E> {
                if value.is_finite() {
                    Ok(Cell::Number(value))
                } else {
                    Err(E::custom("Numeric cell must be finite."))
                }
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Cell, E> {
                self.visit_f64(value as f64)
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Cell, E> {
                self.visit_f64(value as f64)
            }
        }
        deserializer.deserialize_any(CellVisitor)
    }
}
pub(super) mod estimates {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Key(#[serde(with = "Stat")] Statistic);
    #[derive(Serialize)]
    struct Ref<'a>(#[serde(with = "EstimateWire")] &'a Estimate);
    #[derive(Deserialize)]
    struct Owned(#[serde(with = "EstimateWire")] Estimate);
    pub fn serialize<S: Serializer>(
        values: &BTreeMap<Statistic, Estimate>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(values.iter().map(|(key, value)| (Key(*key), Ref(value))))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<Statistic, Estimate>, D::Error> {
        struct Estimates;
        impl<'de> Visitor<'de> for Estimates {
            type Value = BTreeMap<Statistic, Estimate>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unique statistic estimates")
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(Key(key)) = map.next_key()? {
                    let Owned(value) = map.next_value()?;
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("Duplicate statistic estimate."));
                    }
                }
                Ok(values)
            }
        }
        deserializer.deserialize_map(Estimates)
    }
}
