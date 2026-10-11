//! Count flattened cells before any typed column/value vector is allocated.
use crate::data::MAX_CELLS;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::fmt;

#[derive(Clone, Copy)]
enum Field {
    Dataset,
    Format,
    Version,
    Rows,
    Columns,
    Values,
    Runs,
    Other,
}
impl<'de> serde::Deserialize<'de> for Field {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Key;
        impl Visitor<'_> for Key {
            type Value = Field;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a field name")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Field, E> {
                Ok(match s {
                    "dataset" => Field::Dataset,
                    "format" => Field::Format,
                    "version" => Field::Version,
                    "rows" => Field::Rows,
                    "columns" => Field::Columns,
                    "values" => Field::Values,
                    "runs" => Field::Runs,
                    _ => Field::Other,
                })
            }
        }
        d.deserialize_identifier(Key)
    }
}
pub(super) struct Dimensions;
impl<'de> DeserializeSeed<'de> for Dimensions {
    type Value = u64;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<u64, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Dimensions {
    type Value = u64;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a project object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<u64, A::Error> {
        let (mut version, mut format, mut dataset) = (None, false, false);
        while let Some(field) = map.next_key::<Field>()? {
            match field {
                Field::Version => {
                    if version.is_some() {
                        return Err(de::Error::custom("Duplicate version."));
                    }
                    let v = map.next_value::<u64>()?;
                    if v != 3 {
                        return Err(de::Error::custom("Unsupported version."));
                    }
                    version = Some(v);
                }
                Field::Format => {
                    if format {
                        return Err(de::Error::custom("Duplicate format."));
                    }
                    map.next_value::<super::decode::FormatToken>()?;
                    format = true;
                }
                Field::Runs => {
                    map.next_value::<IgnoredAny>()?;
                }
                Field::Dataset => {
                    if dataset {
                        return Err(de::Error::custom("Duplicate dataset."));
                    }
                    map.next_value_seed(Data)?;
                    dataset = true;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if !format || !dataset {
            return Err(de::Error::custom("Missing project header or dataset."));
        }
        let version = version.ok_or_else(|| de::Error::missing_field("version"))?;
        Ok(version)
    }
}
struct Data;
impl<'de> DeserializeSeed<'de> for Data {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Data {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a dataset object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let (mut rows, mut dimensions) = (None, None);
        while let Some(field) = map.next_key::<Field>()? {
            match field {
                Field::Rows => {
                    let n = map.next_value::<usize>()?;
                    if n > MAX_CELLS {
                        return Err(limit());
                    }
                    rows = Some(n);
                }
                Field::Columns => dimensions = Some(map.next_value_seed(Columns)?),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let (columns, length) = dimensions.ok_or_else(|| de::Error::missing_field("columns"))?;
        let rows = rows.ok_or_else(|| de::Error::missing_field("rows"))?;
        if rows.checked_mul(columns).is_none_or(|n| n > MAX_CELLS) || length.unwrap_or(0) != rows {
            return Err(limit());
        }
        Ok(())
    }
}
struct Columns;
impl<'de> DeserializeSeed<'de> for Columns {
    type Value = (usize, Option<usize>);
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Columns {
    type Value = (usize, Option<usize>);
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("flattened columns")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let (mut count, mut total, mut length) = (0_usize, 0_usize, None);
        while let Some(n) = seq.next_element_seed(Column {
            remaining: MAX_CELLS - total,
        })? {
            count += 1;
            if count > MAX_CELLS || length.is_some_and(|length| length != n) {
                return Err(limit());
            }
            total += n;
            length = Some(n);
        }
        Ok((count, length))
    }
}
struct Column {
    remaining: usize,
}
impl<'de> DeserializeSeed<'de> for Column {
    type Value = usize;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<usize, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for Column {
    type Value = usize;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a column object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<usize, A::Error> {
        let mut length = None;
        while let Some(field) = map.next_key::<Field>()? {
            if matches!(field, Field::Values) {
                length = Some(map.next_value_seed(Values {
                    remaining: self.remaining,
                })?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        length.ok_or_else(|| de::Error::missing_field("values"))
    }
}
struct Values {
    remaining: usize,
}
impl<'de> DeserializeSeed<'de> for Values {
    type Value = usize;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<usize, D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for Values {
    type Value = usize;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cell values")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<usize, A::Error> {
        let mut count = 0;
        while seq
            .next_element_seed(Cell {
                allowed: count < self.remaining,
            })?
            .is_some()
        {
            count += 1;
        }
        Ok(count)
    }
}
struct Cell {
    allowed: bool,
}
impl<'de> DeserializeSeed<'de> for Cell {
    type Value = IgnoredAny;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<IgnoredAny, D::Error> {
        if !self.allowed {
            return Err(limit());
        }
        serde::Deserialize::deserialize(d)
    }
}
fn limit<E: de::Error>() -> E {
    E::custom("Invalid dimensions or two-million-cell limit.")
}
