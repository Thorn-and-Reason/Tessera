//! Canonical unit strings and one-tag objects; no map-form unit alternatives.
use super::*;
impl AnovaScopeWire {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<AnovaScope, D::Error> {
        struct Scope;
        impl<'de> Visitor<'de> for Scope {
            type Value = AnovaScope;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("marginal or a fixed scope object")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "marginal" => Ok(AnovaScope::Marginal),
                    _ => Err(E::custom("Unknown ANOVA scope.")),
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                struct Values(#[serde(with = "cells")] Vec<Cell>);
                if map.next_key::<String>()?.as_deref() != Some("fixed") {
                    return Err(de::Error::custom("Unknown ANOVA scope."));
                }
                let Values(values) = map.next_value()?;
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("ANOVA scope requires one tag."));
                }
                Ok(AnovaScope::Fixed(values))
            }
        }
        d.deserialize_any(Scope)
    }
}
impl AnovaQuantityStateWire {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<AnovaQuantityState, D::Error> {
        struct State;
        impl<'de> Visitor<'de> for State {
            type Value = AnovaQuantityState;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a quantity state string or unavailable object")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "positive" => Ok(AnovaQuantityState::Positive),
                    "exact_zero" => Ok(AnovaQuantityState::ExactZero),
                    _ => Err(E::custom("Unknown ANOVA quantity state.")),
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                struct Kind(#[serde(with = "AnovaFailureKindWire")] AnovaFailureKind);
                if map.next_key::<String>()?.as_deref() != Some("unavailable") {
                    return Err(de::Error::custom("Unknown ANOVA quantity state."));
                }
                let Kind(kind) = map.next_value()?;
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("ANOVA quantity state requires one tag."));
                }
                Ok(AnovaQuantityState::Unavailable(kind))
            }
        }
        d.deserialize_any(State)
    }
}
impl AnovaComparisonStateWire {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<AnovaComparisonState, D::Error> {
        struct State;
        impl<'de> Visitor<'de> for State {
            type Value = AnovaComparisonState;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("available or a comparison failure object")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v == "available" {
                    Ok(AnovaComparisonState::Available)
                } else {
                    Err(E::custom("Unknown ANOVA comparison state."))
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                struct Failure(#[serde(with = "AnovaFailureWire")] AnovaFailure);
                let value = match map.next_key::<String>()?.as_deref() {
                    Some("absent_requested_level") => {
                        let Failure(f) = map.next_value()?;
                        AnovaComparisonState::AbsentRequestedLevel(f)
                    }
                    Some("unavailable") => {
                        let Failure(f) = map.next_value()?;
                        AnovaComparisonState::Unavailable(f)
                    }
                    _ => return Err(de::Error::custom("Unknown ANOVA comparison state.")),
                };
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(
                        "ANOVA comparison state requires one tag.",
                    ));
                }
                Ok(value)
            }
        }
        d.deserialize_any(State)
    }
}
impl AnovaFamilyKindWire {
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<AnovaFamilyKind, D::Error> {
        struct Family;
        impl<'de> Visitor<'de> for Family {
            type Value = AnovaFamilyKind;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("planned or an all-pairs family object")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v == "planned" {
                    Ok(AnovaFamilyKind::Planned)
                } else {
                    Err(E::custom("Unknown ANOVA family kind."))
                }
            }
            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                struct Request(#[serde(with = "AnovaFamilyRequestWire")] AnovaFamilyRequest);
                if map.next_key::<String>()?.as_deref() != Some("all_pairs") {
                    return Err(de::Error::custom("Unknown ANOVA family kind."));
                }
                let Request(r) = map.next_value()?;
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("ANOVA family kind requires one tag."));
                }
                Ok(AnovaFamilyKind::AllPairs(r))
            }
        }
        d.deserialize_any(Family)
    }
}
