//! Canonical unit strings and single-tag objects keep serde's tuple/unit alternatives out.
use super::*;

impl PredictorRoleWire {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PredictorRole, D::Error> {
        struct Role;
        impl<'de> Visitor<'de> for Role {
            type Value = PredictorRole;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("numeric or a categorical reference object")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "numeric" => Ok(PredictorRole::Numeric),
                    _ => Err(E::custom("Unknown predictor role.")),
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Reference {
                    #[serde(with = "cell")]
                    reference: Cell,
                }
                if map.next_key::<String>()?.as_deref() != Some("categorical") {
                    return Err(de::Error::custom("Unknown predictor role."));
                }
                let Reference { reference } = map.next_value()?;
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(
                        "A predictor role requires exactly one tag.",
                    ));
                }
                Ok(PredictorRole::Categorical { reference })
            }
        }
        d.deserialize_any(Role)
    }
}

impl RegressionTermWire {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<RegressionTerm, D::Error> {
        struct Term;
        impl<'de> Visitor<'de> for Term {
            type Value = RegressionTerm;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("intercept or a numeric/contrast term object")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "intercept" => Ok(RegressionTerm::Intercept),
                    _ => Err(E::custom("Unknown regression term.")),
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Numeric {
                    #[serde(with = "column_id")]
                    column: ColumnId,
                }
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Contrast {
                    #[serde(with = "column_id")]
                    column: ColumnId,
                    #[serde(with = "cell")]
                    level: Cell,
                    #[serde(with = "cell")]
                    reference: Cell,
                }
                let term = match map.next_key::<String>()?.as_deref() {
                    Some("numeric") => {
                        let Numeric { column } = map.next_value()?;
                        RegressionTerm::Numeric { column }
                    }
                    Some("contrast") => {
                        let Contrast {
                            column,
                            level,
                            reference,
                        } = map.next_value()?;
                        RegressionTerm::Contrast {
                            column,
                            level,
                            reference,
                        }
                    }
                    _ => return Err(de::Error::custom("Unknown regression term.")),
                };
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(
                        "A regression term requires exactly one tag.",
                    ));
                }
                Ok(term)
            }
        }
        d.deserialize_any(Term)
    }
}

impl RegressionFailureKindWire {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<RegressionFailureKind, D::Error> {
        struct Kind;
        impl<'de> Visitor<'de> for Kind {
            type Value = RegressionFailureKind;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a documented failure string or column failure object")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "empty_cohort" => Ok(RegressionFailureKind::EmptyCohort),
                    "unidentified" => Ok(RegressionFailureKind::Unidentified),
                    "nonconvergence" => Ok(RegressionFailureKind::Nonconvergence),
                    "numerical" => Ok(RegressionFailureKind::Numerical),
                    _ => Err(E::custom("Unknown regression failure.")),
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Column {
                    #[serde(with = "column_id")]
                    column: ColumnId,
                }
                let kind = match map.next_key::<String>()?.as_deref() {
                    Some("absent_reference") => {
                        let Column { column } = map.next_value()?;
                        RegressionFailureKind::AbsentReference { column }
                    }
                    Some("no_contrast") => {
                        let Column { column } = map.next_value()?;
                        RegressionFailureKind::NoContrast { column }
                    }
                    _ => return Err(de::Error::custom("Unknown regression failure.")),
                };
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(
                        "A regression failure requires exactly one tag.",
                    ));
                }
                Ok(kind)
            }
        }
        d.deserialize_any(Kind)
    }
}
