//! Wrap typed serde visitors so limits precede each owned string or collection growth.
use crate::{
    data::MAX_OWNED_BYTES,
    error::{AppError, AppResult},
    operation::JobControl,
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::{cell::RefCell, fmt, mem::size_of};

pub(super) struct Accounting<'a> {
    pub bytes: usize,
    pub control: &'a JobControl,
    pub error: Option<AppError>,
    pub steps: usize,
}
impl Accounting<'_> {
    pub fn reserve<E: de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.steps += 1;
        let checkpoint = if self.steps.is_multiple_of(4096) {
            self.control.checkpoint(self.bytes, None)
        } else {
            Ok(())
        };
        let result = checkpoint.and_then(|()| {
            self.bytes = self
                .bytes
                .checked_add(bytes)
                .filter(|n| *n <= MAX_OWNED_BYTES)
                .ok_or(AppError::SizeLimit {
                    limit: MAX_OWNED_BYTES,
                })?;
            Ok(())
        });
        result.map_err(|error| {
            self.error = Some(error);
            E::custom("Project work limit or cancellation.")
        })
    }
    pub fn release(&mut self, bytes: usize) {
        self.bytes -= bytes;
    }
    pub fn add(&mut self, bytes: usize) -> AppResult<()> {
        self.reserve::<serde::de::value::Error>(bytes)
            .map_err(|_| self.error.take().expect("recorded error"))
    }
}
pub(super) struct Guard<'a, 'b, D> {
    pub inner: D,
    pub state: &'a RefCell<Accounting<'b>>,
    pub depth: usize,
}
struct GuardVisitor<'a, 'b, V> {
    inner: V,
    temporary_keys: bool,
    state: &'a RefCell<Accounting<'b>>,
    depth: usize,
}
struct Seed<'a, 'b, S> {
    inner: S,
    state: &'a RefCell<Accounting<'b>>,
    depth: usize,
}
impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Seed<'_, '_, S> {
    type Value = S::Value;
    fn deserialize<D: de::Deserializer<'de>>(self, inner: D) -> Result<Self::Value, D::Error> {
        self.inner.deserialize(Guard {
            inner,
            state: self.state,
            depth: self.depth,
        })
    }
}
macro_rules! methods {
    ($($method:ident $(($($arg:ident: $ty:ty),*))?;)*) => {$ (
        fn $method<V: Visitor<'de>>(self, $($($arg: $ty,)*)? visitor: V) -> Result<V::Value, Self::Error> {
            if self.depth > super::MAX_DEPTH { return Err(de::Error::custom("Project nesting exceeds 64.")); }
            self.inner.$method($($($arg,)*)? GuardVisitor { inner: visitor, state: self.state, depth: self.depth + 1, temporary_keys: false })
        }
    )*};
}
impl<'de, D: de::Deserializer<'de>> de::Deserializer<'de> for Guard<'_, '_, D> {
    type Error = D::Error;
    methods! {
        deserialize_any; deserialize_option; deserialize_ignored_any;

    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if self.depth > super::MAX_DEPTH {
            return Err(de::Error::custom("Project nesting exceeds 64."));
        }
        if name == "RegressionFitBox" {
            self.state
                .borrow_mut()
                .reserve::<D::Error>(size_of::<crate::analysis::RegressionFit>())?;
        }
        self.inner.deserialize_newtype_struct(
            name,
            GuardVisitor {
                inner: visitor,
                state: self.state,
                depth: self.depth + 1,
                temporary_keys: false,
            },
        )
    }
    // JSON is self-describing. Typed serde_json entry points can format a whole
    // wrong-type string before invoking a visitor; dispatch through our visitor first.
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
        byte_buf unit unit_struct seq tuple tuple_struct map identifier
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if self.depth > super::MAX_DEPTH {
            return Err(de::Error::custom("Project nesting exceeds 64."));
        }
        self.inner.deserialize_any(GuardVisitor {
            inner: MapOnly(visitor),
            temporary_keys: true,
            state: self.state,
            depth: self.depth + 1,
        })
    }
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if self.depth > super::MAX_DEPTH {
            return Err(de::Error::custom("Project nesting exceeds 64."));
        }
        self.inner.deserialize_any(EnumVisitor(GuardVisitor {
            inner: visitor,
            temporary_keys: false,
            state: self.state,
            depth: self.depth + 1,
        }))
    }
    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}
// Records are maps even though serde's derived struct visitor also accepts tuples.
struct MapOnly<V>(V);
impl<'de, V: Visitor<'de>> Visitor<'de> for MapOnly<V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a record object")
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        self.0.visit_map(map)
    }
}
// Drop serde's Display arguments without formatting untrusted strings. The
// public codec already reports only location and a fixed invalid-project message.
#[derive(Debug)]
struct ValueError;
impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Invalid project value.")
    }
}
impl std::error::Error for ValueError {}
impl de::Error for ValueError {
    fn custom<T: fmt::Display>(_: T) -> Self {
        Self
    }
}
fn bounded_error<T, E: de::Error>(result: Result<T, ValueError>) -> Result<T, E> {
    result.map_err(|_| E::custom("Invalid project value."))
}
struct EnumVisitor<'a, 'b, V>(GuardVisitor<'a, 'b, V>);
impl<'de, V: Visitor<'de>> Visitor<'de> for EnumVisitor<'_, '_, V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a project enum")
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        bounded_error(
            self.0
                .inner
                .visit_enum(de::value::StrDeserializer::<ValueError>::new(value)),
        )
    }
    fn visit_map<A: MapAccess<'de>>(self, inner: A) -> Result<Self::Value, A::Error> {
        let mut map = Access {
            inner,
            state: self.0.state,
            depth: self.0.depth,
            temporary_keys: true,
            count: 0,
            slots: 0,
            slot_size: 0,
        };
        let result = self.0.inner.visit_enum(EnumMap(&mut map))?;
        if map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(
                "An enum object requires exactly one tag.",
            ));
        }
        Ok(result)
    }
}
// A map-backed enum keeps variant payloads inside Guard and forbids sequence records.
struct EnumMap<'a, A>(&'a mut A);
impl<'de, A: MapAccess<'de>> de::EnumAccess<'de> for EnumMap<'_, A> {
    type Error = A::Error;
    type Variant = Self;
    fn variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<(S::Value, Self), A::Error> {
        let tag = self
            .0
            .next_key_seed(seed)?
            .ok_or_else(|| de::Error::custom("An enum object requires one tag."))?;
        Ok((tag, self))
    }
}
impl<'de, A: MapAccess<'de>> de::VariantAccess<'de> for EnumMap<'_, A> {
    type Error = A::Error;
    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.next_value()
    }
    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, A::Error> {
        self.0.next_value_seed(seed)
    }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, A::Error> {
        self.0.next_value_seed(VariantSeed {
            visitor,
            shape: VariantShape::Tuple(len),
        })
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.next_value_seed(VariantSeed {
            visitor,
            shape: VariantShape::Record(fields),
        })
    }
}
enum VariantShape {
    Tuple(usize),
    Record(&'static [&'static str]),
}
struct VariantSeed<V> {
    visitor: V,
    shape: VariantShape,
}
impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for VariantSeed<V> {
    type Value = V::Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<V::Value, D::Error> {
        match self.shape {
            VariantShape::Tuple(n) => d.deserialize_tuple(n, self.visitor),
            VariantShape::Record(fields) => d.deserialize_struct("variant", fields, self.visitor),
        }
    }
}
macro_rules! simple_visits {
    ($($name:ident($ty:ty);)*) => {$ (
        fn $name<E: de::Error>(self, value: $ty) -> Result<Self::Value, E> { bounded_error(self.inner.$name::<ValueError>(value)) }
    )*};
}
impl<'de, V: Visitor<'de>> Visitor<'de> for GuardVisitor<'_, '_, V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.expecting(f)
    }
    simple_visits! { visit_bool(bool); visit_i8(i8); visit_i16(i16); visit_i32(i32); visit_i64(i64); visit_i128(i128);
    visit_u8(u8); visit_u16(u16); visit_u32(u32); visit_u64(u64); visit_u128(u128); visit_f32(f32); visit_f64(f64); visit_char(char); }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        self.state.borrow_mut().reserve::<E>(value.len())?;
        bounded_error(self.inner.visit_str::<ValueError>(value))
    }
    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        self.state.borrow_mut().reserve::<E>(value.len())?;
        bounded_error(self.inner.visit_borrowed_str::<ValueError>(value))
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        self.state.borrow_mut().reserve::<E>(value.capacity())?;
        bounded_error(self.inner.visit_string::<ValueError>(value))
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_none()
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_unit()
    }
    fn visit_some<D: de::Deserializer<'de>>(self, inner: D) -> Result<Self::Value, D::Error> {
        self.inner.visit_some(Guard {
            inner,
            state: self.state,
            depth: self.depth,
        })
    }
    fn visit_newtype_struct<D: de::Deserializer<'de>>(
        self,
        inner: D,
    ) -> Result<Self::Value, D::Error> {
        self.inner.visit_newtype_struct(Guard {
            inner,
            state: self.state,
            depth: self.depth,
        })
    }
    fn visit_seq<A: SeqAccess<'de>>(self, inner: A) -> Result<Self::Value, A::Error> {
        let mut access = Access {
            inner,
            state: self.state,
            depth: self.depth,
            temporary_keys: false,
            count: 0,
            slots: 0,
            slot_size: 0,
        };
        let value = self.inner.visit_seq(&mut access)?;
        // The old vector is gone after its visitor returns. Keep a capacity bound
        // and per-entry Arc overhead for the run visitor, not every past growth.
        let live = if access.count == 0 {
            0
        } else {
            let capacity = access
                .count
                .next_power_of_two()
                .max(min_capacity(access.slot_size));
            capacity
                .saturating_mul(access.slot_size)
                .saturating_add(access.count.saturating_mul(32))
                .min(access.slots)
        };
        self.state.borrow_mut().release(access.slots - live);
        Ok(value)
    }
    fn visit_map<A: MapAccess<'de>>(self, inner: A) -> Result<Self::Value, A::Error> {
        self.inner.visit_map(Access {
            inner,
            state: self.state,
            depth: self.depth,
            temporary_keys: self.temporary_keys,
            count: 0,
            slots: 0,
            slot_size: 0,
        })
    }
}
struct Access<'a, 'b, A> {
    inner: A,
    temporary_keys: bool,
    state: &'a RefCell<Accounting<'b>>,
    depth: usize,
    count: usize,
    slots: usize,
    slot_size: usize,
}
fn min_capacity(size: usize) -> usize {
    if size == 1 {
        8
    } else if size <= 1024 {
        4
    } else {
        1
    }
}
impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for Access<'_, '_, A> {
    type Error = A::Error;
    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        if self.count >= crate::data::MAX_CELLS {
            return self
                .inner
                .next_element_seed(Reject::<S::Value>(std::marker::PhantomData));
        }
        // Three slots cover the old and doubled vector allocations during growth.
        self.slot_size = size_of::<S::Value>();
        let slots = self.slot_size.saturating_mul(if self.count == 0 {
            min_capacity(self.slot_size).max(3)
        } else {
            3
        });
        self.state.borrow_mut().reserve::<A::Error>(slots)?;
        self.slots += slots;
        let value = self.inner.next_element_seed(Seed {
            inner: seed,
            state: self.state,
            depth: self.depth,
        })?;
        self.count += usize::from(value.is_some());
        Ok(value)
    }
}
impl<'de, A: MapAccess<'de>> MapAccess<'de> for Access<'_, '_, A> {
    type Error = A::Error;
    fn next_key_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        let before = self.state.borrow().bytes;
        self.state.borrow_mut().reserve::<A::Error>(256)?;
        let result = self.inner.next_key_seed(Seed {
            inner: seed,
            state: self.state,
            depth: self.depth,
        });
        // Closed record fields and derived enum tags return discriminants, not
        // owned map keys. Release only this key visit; payloads remain charged.
        // Generic maps keep their entry/key charge for retained nodes and keys.
        if self.temporary_keys {
            let temporary = self.state.borrow().bytes - before;
            self.state.borrow_mut().release(temporary);
        }
        result
    }
    fn next_value_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        self.inner.next_value_seed(Seed {
            inner: seed,
            state: self.state,
            depth: self.depth,
        })
    }
}

struct Reject<T>(std::marker::PhantomData<T>);
impl<'de, T> DeserializeSeed<'de> for Reject<T> {
    type Value = T;
    fn deserialize<D: de::Deserializer<'de>>(self, _: D) -> Result<T, D::Error> {
        Err(de::Error::custom("Collection exceeds two million entries."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    #[derive(Debug)]
    struct NoAllocationError;
    impl fmt::Display for NoAllocationError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("refused")
        }
    }
    impl std::error::Error for NoAllocationError {}
    impl de::Error for NoAllocationError {
        fn custom<T: fmt::Display>(_: T) -> Self {
            Self
        }
    }
    #[test]
    fn initial_vector_capacity_and_later_growth_are_reserved_before_allocation() {
        let control = JobControl::default();
        for (headroom, count, largest_allowed) in [(31, 1, 0), (210, 9, 64)] {
            let state = RefCell::new(Accounting {
                bytes: MAX_OWNED_BYTES - headroom,
                control: &control,
                error: None,
                steps: 0,
            });
            let input = de::value::SeqDeserializer::<_, NoAllocationError>::new(0_u64..count);
            let (result, largest) = crate::data::measure_test_allocations(|| {
                Vec::<u64>::deserialize(Guard {
                    inner: input,
                    state: &state,
                    depth: 0,
                })
            });
            assert!(result.is_err());
            assert!(matches!(
                state.borrow().error,
                Some(AppError::SizeLimit { .. })
            ));
            assert!(
                largest <= largest_allowed,
                "allocated {largest} with {headroom} headroom"
            );
        }
    }
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Record {
        payload: String,
        tag: Tag,
    }
    #[derive(Debug, Deserialize)]
    enum Tag {
        Text(String),
    }
    fn parse_budgeted<T: for<'de> Deserialize<'de>>(
        text: &str,
        headroom: usize,
        control: &JobControl,
    ) -> (Result<T, serde_json::Error>, usize) {
        let baseline = MAX_OWNED_BYTES - headroom;
        let state = RefCell::new(Accounting {
            bytes: baseline,
            control,
            error: None,
            steps: 0,
        });
        let mut json = serde_json::Deserializer::from_str(text);
        let result = T::deserialize(Guard {
            inner: &mut json,
            state: &state,
            depth: 0,
        });
        let charged = state.borrow().bytes - baseline;
        (result, charged)
    }
    #[test]
    fn record_and_enum_identifiers_are_temporary_but_payloads_remain_owned() {
        let row = r#"{"payload":"owned payload text","tag":{"Text":"owned enum text"}}"#;
        let input = format!("[{}]", vec![row; 256].join(","));
        let (result, charged) =
            parse_budgeted::<Vec<Record>>(&input, 128 * 1024, &JobControl::default());
        let rows = result.expect("closed record keys must not accumulate as retained map entries");
        assert_eq!(rows.len(), 256);
        for row in rows {
            assert_eq!(row.payload, "owned payload text");
            let Tag::Text(value) = row.tag;
            assert_eq!(value, "owned enum text");
        }
        assert!(charged >= 256 * ("owned payload text".len() + "owned enum text".len()));
    }
    #[test]
    fn nested_generic_maps_retain_owned_keys_values_and_entry_storage() {
        use std::collections::BTreeMap;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct MapRecord {
            entries: BTreeMap<String, String>,
        }
        let key = "k".repeat(4096);
        let value = "v".repeat(4096);
        let input = serde_json::json!({"entries": { key.clone(): value.clone() }}).to_string();
        let (result, charged) = parse_budgeted::<MapRecord>(&input, 16384, &JobControl::default());
        assert_eq!(result.unwrap().entries[&key], value);
        assert!(
            charged >= 8192 + 256,
            "generic map storage was released: {charged}"
        );
        assert!(
            parse_budgeted::<MapRecord>(&input, 8192, &JobControl::default())
                .0
                .is_err()
        );
        let large = format!(
            r#"{{"payload":"{}","tag":{{"Text":"x"}}}}"#,
            "v".repeat(8192)
        );
        let (result, largest) = crate::data::measure_test_allocations(|| {
            parse_budgeted::<Record>(&large, 4096, &JobControl::default())
        });
        assert!(result.0.is_err());
        assert!(
            largest < 8192,
            "owned payload allocated before its guard: {largest}"
        );
    }
    #[test]
    fn transient_keys_preserve_strict_records_tags_and_cancellation() {
        for bad in [
            r#"["x",{"Text":"y"}]"#,
            r#"{"payload":"x","payload":"y","tag":{"Text":"z"}}"#,
            r#"{"payload":"x","other":0,"tag":{"Text":"z"}}"#,
            r#"{"payload":[],"tag":{"Text":"z"}}"#,
            r#"{"payload":"x","tag":{"Text":"y","Text":"z"}}"#,
            r#"{"payload":"x","tag":{"Unknown":"z"}}"#,
            r#"{"payload":"x","tag":["Text","z"]}"#,
        ] {
            assert!(
                parse_budgeted::<Record>(bad, 16384, &JobControl::default())
                    .0
                    .is_err(),
                "{bad}"
            );
        }
        let escaped = r#"{"paylo\u0061d":"x","tag":{"T\u0065xt":"y"}}"#;
        assert!(
            parse_budgeted::<Record>(escaped, 4096, &JobControl::default())
                .0
                .is_ok()
        );
        let long_key = format!(r#"{{"{}":0}}"#, "k".repeat(8192));
        let (result, largest) = crate::data::measure_test_allocations(|| {
            parse_budgeted::<Record>(&long_key, 4096, &JobControl::default())
        });
        assert!(result.0.is_err());
        assert!(largest < 8192);
        assert!(
            parse_budgeted::<Record>(escaped, 255, &JobControl::default())
                .0
                .is_err()
        );
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let control = JobControl::new(cancel, None);
        let input = format!(
            "[{}]",
            vec![r#"{"payload":"x","tag":{"Text":"y"}}"#; 1024].join(",")
        );
        assert!(
            parse_budgeted::<Vec<Record>>(&input, 4 * 1024 * 1024, &control)
                .0
                .is_err()
        );
    }
}
