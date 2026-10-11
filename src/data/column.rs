use super::{
    budget::{AllocationCounter, MAX_OWNED_BYTES},
    ColumnId, CHUNK_SIZE, MAX_CELLS,
};
use crate::{
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::{collections::HashSet, mem::size_of, sync::Arc};

#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Number(f64),
    Text(String),
    Missing,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Number,
    Text,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Measurement {
    #[default]
    Unknown,
    Nominal,
    Ordinal,
    Scale,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub enum MissingRules {
    #[default]
    None,
    Number {
        values: Vec<f64>,
        ranges: Vec<(f64, f64)>,
    },
    Text(Vec<String>),
}
#[derive(Debug, Clone, PartialEq)]
pub struct ValueLabel {
    pub value: Cell,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnMetadata {
    pub name: String,
    pub description: String,
    pub measurement: Measurement,
    pub missing_rules: MissingRules,
    pub value_labels: Vec<ValueLabel>,
    pub display_decimals: u8,
}
impl ColumnMetadata {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            measurement: Measurement::Unknown,
            missing_rules: MissingRules::None,
            value_labels: vec![],
            display_decimals: 2,
        }
    }
    pub(crate) fn validate_name(name: &str) -> AppResult<()> {
        if name.trim().is_empty() || name.chars().any(char::is_control) {
            return Err(AppError::invalid(
                "name",
                "Enter a nonblank name without control characters.",
            ));
        }
        Ok(())
    }
    fn validate(&self, kind: ColumnKind) -> AppResult<()> {
        Self::validate_name(&self.name)?;
        if self.display_decimals > 16 {
            return Err(AppError::invalid(
                "decimals",
                "Display decimals must be between 0 and 16.",
            ));
        }
        match &self.missing_rules {
            MissingRules::None => {}
            MissingRules::Number { values, ranges } if kind == ColumnKind::Number => {
                if values.iter().any(|v| !v.is_finite())
                    || ranges
                        .iter()
                        .any(|(lo, hi)| !lo.is_finite() || !hi.is_finite() || lo > hi)
                {
                    return Err(AppError::invalid(
                        "missing rules",
                        "Missing numbers and ordered range endpoints must be finite.",
                    ));
                }
            }
            MissingRules::Text(_) if kind == ColumnKind::Text => {}
            _ => {
                return Err(AppError::invalid(
                    "missing rules",
                    "Missing rules must match the column type.",
                ));
            }
        }
        let mut numbers = HashSet::new();
        let mut texts = HashSet::new();
        for label in &self.value_labels {
            let unique = match &label.value {
                Cell::Number(v) if kind == ColumnKind::Number && v.is_finite() => {
                    numbers.insert(if *v == 0.0 { 0 } else { v.to_bits() })
                }
                Cell::Text(v) if kind == ColumnKind::Text => texts.insert(v),
                _ => {
                    return Err(AppError::invalid(
                        "value labels",
                        "Labels require finite values of the column type.",
                    ));
                }
            };
            if !unique {
                return Err(AppError::invalid(
                    "value labels",
                    "Label values must be unique.",
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn validation_work(&self) -> AppResult<usize> {
        let mut counter = AllocationCounter::default();
        if !self.value_labels.is_empty() {
            // Reserve temporary uniqueness tables, including their capacity rounding.
            counter.array::<u64>(self.value_labels.len().saturating_mul(8))?;
            counter.add(64)?;
        }
        Ok(counter.bytes)
    }
    pub(crate) fn heap_bytes(&self) -> AppResult<usize> {
        let mut counter = AllocationCounter::default();
        counter.add(self.name.capacity())?;
        counter.add(self.description.capacity())?;
        match &self.missing_rules {
            MissingRules::None => {}
            MissingRules::Number { values, ranges } => {
                counter.array::<f64>(values.capacity())?;
                counter.array::<(f64, f64)>(ranges.capacity())?;
            }
            MissingRules::Text(values) => {
                counter.array::<String>(values.capacity())?;
                for value in values {
                    counter.add(value.capacity())?;
                }
            }
        }
        counter.array::<ValueLabel>(self.value_labels.capacity())?;
        for label in &self.value_labels {
            counter.add(label.label.capacity())?;
            if let Cell::Text(value) = &label.value {
                counter.add(value.capacity())?;
            }
        }
        Ok(counter.bytes)
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Chunks {
    Number(Vec<Arc<[Option<f64>]>>),
    Text(Vec<Arc<[Option<String>]>>),
}
#[derive(Debug, Clone)]
pub struct Column {
    pub(crate) id: ColumnId,
    pub(crate) metadata: Arc<ColumnMetadata>,
    pub(crate) chunks: Chunks,
    pub(crate) len: usize,
}
impl Column {
    pub fn numeric(id: ColumnId, name: &str, values: Vec<Option<f64>>) -> AppResult<Self> {
        Self::check_input(
            id,
            name,
            values.len(),
            values.capacity().checked_mul(size_of::<Option<f64>>()),
        )?;
        if values.iter().flatten().any(|v| !v.is_finite()) {
            return Err(AppError::invalid(
                "column",
                "Numeric values must be finite.",
            ));
        }
        let len = values.len();
        let chunks = values.chunks(CHUNK_SIZE).map(Arc::from).collect();
        Ok(Self {
            id,
            metadata: Arc::new(ColumnMetadata::new(name)),
            chunks: Chunks::Number(chunks),
            len,
        })
    }
    pub fn text(id: ColumnId, name: &str, values: Vec<Option<String>>) -> AppResult<Self> {
        let mut counter = AllocationCounter::default();
        counter.array::<Option<String>>(values.capacity())?;
        for value in values.iter().flatten() {
            counter.add(value.capacity())?;
        }
        Self::check_input(id, name, values.len(), Some(counter.bytes))?;
        let len = values.len();
        let mut iter = values.into_iter();
        let chunks = (0..len.div_ceil(CHUNK_SIZE))
            .map(|_| Arc::from(iter.by_ref().take(CHUNK_SIZE).collect::<Vec<_>>()))
            .collect();
        Ok(Self {
            id,
            metadata: Arc::new(ColumnMetadata::new(name)),
            chunks: Chunks::Text(chunks),
            len,
        })
    }
    fn check_input(id: ColumnId, name: &str, len: usize, bytes: Option<usize>) -> AppResult<()> {
        if id.0 == 0 {
            return Err(AppError::invalid("id", "Column ID must be positive."));
        }
        if len > MAX_CELLS {
            return Err(AppError::SizeLimit { limit: MAX_CELLS });
        }
        ColumnMetadata::validate_name(name)?;
        let mut counter = AllocationCounter::default();
        counter.add(bytes.ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })?)?;
        counter.add(name.len())?;
        counter.add(size_of::<ColumnMetadata>() + size_of::<Column>() + 2 * size_of::<usize>())?;
        counter.array::<usize>(len.div_ceil(CHUNK_SIZE) * 4)?;
        Ok(())
    }
    pub fn id(&self) -> ColumnId {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.metadata.name
    }
    pub fn metadata(&self) -> &ColumnMetadata {
        &self.metadata
    }
    pub fn kind(&self) -> ColumnKind {
        match self.chunks {
            Chunks::Number(_) => ColumnKind::Number,
            Chunks::Text(_) => ColumnKind::Text,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn cell(&self, row: usize) -> AppResult<Cell> {
        if row >= self.len {
            return Err(AppError::invalid("row", "Row no longer exists."));
        }
        let (chunk, offset) = (row / CHUNK_SIZE, row % CHUNK_SIZE);
        Ok(match &self.chunks {
            Chunks::Number(chunks) => chunks[chunk][offset].map_or(Cell::Missing, Cell::Number),
            Chunks::Text(chunks) => chunks[chunk][offset]
                .clone()
                .map_or(Cell::Missing, Cell::Text),
        })
    }
    pub fn text_value(&self, row: usize) -> AppResult<Option<&str>> {
        if row >= self.len {
            return Err(AppError::invalid("row", "Row no longer exists."));
        }
        match &self.chunks {
            Chunks::Text(chunks) => Ok(chunks[row / CHUNK_SIZE][row % CHUNK_SIZE].as_deref()),
            Chunks::Number(_) => Err(AppError::invalid(
                "column",
                "This column stores numbers, not text.",
            )),
        }
    }
    pub fn is_missing(&self, row: usize) -> AppResult<bool> {
        self.is_missing_core(row, None)
    }
    pub fn is_missing_controlled(&self, row: usize, control: &JobControl) -> AppResult<bool> {
        self.is_missing_core(row, Some(control))
    }
    fn is_missing_core(&self, row: usize, control: Option<&JobControl>) -> AppResult<bool> {
        missing_checkpoint(control, 0)?;
        if row >= self.len {
            return Err(AppError::invalid("row", "Row no longer exists."));
        }
        let (chunk, offset) = (row / CHUNK_SIZE, row % CHUNK_SIZE);
        let missing = match &self.chunks {
            Chunks::Number(chunks) => match chunks[chunk][offset] {
                None => true,
                Some(value) => numeric_missing(value, &self.metadata.missing_rules, control)?,
            },
            Chunks::Text(chunks) => match &chunks[chunk][offset] {
                None => true,
                Some(value) => text_missing(value, &self.metadata.missing_rules, control)?,
            },
        };
        if let Some(control) = control {
            control.check_cancelled()?;
        }
        Ok(missing)
    }
    // ponytail: linear label lookup; index labels if metadata-heavy conversions become a measured bottleneck.
    fn label_at(&self, row: usize) -> Option<usize> {
        let (chunk, offset) = (row / CHUNK_SIZE, row % CHUNK_SIZE);
        self.metadata
            .value_labels
            .iter()
            .position(|label| match (&self.chunks, &label.value) {
                (Chunks::Number(chunks), Cell::Number(value)) => {
                    chunks[chunk][offset].as_ref() == Some(value)
                }
                (Chunks::Text(chunks), Cell::Text(value)) => {
                    chunks[chunk][offset].as_ref() == Some(value)
                }
                _ => false,
            })
    }
    pub fn with_metadata(&self, metadata: ColumnMetadata) -> AppResult<Self> {
        let mut counter = AllocationCounter::default();
        counter.add(metadata.heap_bytes()?)?;
        counter.add(metadata.validation_work()?)?;
        counter.add(size_of::<ColumnMetadata>() + size_of::<Column>() + 2 * size_of::<usize>())?;
        counter.array::<usize>(self.len.div_ceil(CHUNK_SIZE) * 2)?;
        super::Budget::check_column(self, counter.bytes)?;
        metadata.validate(self.kind())?;
        Ok(Self {
            metadata: Arc::new(metadata),
            ..self.clone()
        })
    }
    pub(crate) fn validate(&self) -> AppResult<()> {
        if self.id.0 == 0 {
            return Err(AppError::invalid("id", "Column ID must be positive."));
        }
        self.metadata.validate(self.kind())?;
        match &self.chunks {
            Chunks::Number(chunks)
                if chunks
                    .iter()
                    .flat_map(|c| c.iter())
                    .flatten()
                    .any(|v| !v.is_finite()) =>
            {
                Err(AppError::invalid(
                    "column",
                    "Numeric values must be finite.",
                ))
            }
            _ => Ok(()),
        }
    }
}

impl Column {
    fn conversion_error(&self, row: usize, error: AppError) -> AppError {
        match error {
            AppError::InvalidInput { message, .. } => {
                AppError::invalid(format!("column {}, row {}", self.id.0, row + 1), message)
            }
            error => error,
        }
    }
    pub fn convert(&self, kind: ColumnKind) -> AppResult<Self> {
        if kind == self.kind() {
            return Ok(self.clone());
        }
        let baseline = super::Budget::column_bytes(self, 0)?;
        self.convert_controlled(kind, &JobControl::default(), baseline)
    }
    pub(crate) fn convert_controlled(
        &self,
        kind: ColumnKind,
        control: &JobControl,
        baseline: usize,
    ) -> AppResult<Self> {
        let total = self.len * 3;
        control.checkpoint(0, Some(total))?;
        let mut counter = AllocationCounter { bytes: baseline };
        counter.add(size_of::<ColumnMetadata>() + size_of::<Column>() + 2 * size_of::<usize>())?;
        counter.array::<usize>(self.len.div_ceil(CHUNK_SIZE) * 4)?;
        counter.add(self.metadata.validation_work()?)?;
        match kind {
            ColumnKind::Text => counter.array::<Option<String>>(self.len)?,
            ColumnKind::Number => counter.array::<Option<f64>>(self.len)?,
        }
        counter.add(
            self.len.min(CHUNK_SIZE)
                * match kind {
                    ColumnKind::Text => size_of::<Option<String>>(),
                    ColumnKind::Number => size_of::<Option<f64>>(),
                },
        )?;
        conversion_scratch(&counter)?;
        counter.add(converted_metadata_bytes(
            self.metadata(),
            control,
            counter.bytes,
            total,
        )?)?;
        for (row, value) in (0..self.len).enumerate() {
            if row.is_multiple_of(256) {
                control.checkpoint(row, Some(total))?;
            }
            conversion_scratch(&counter)?;
            match &self.chunks {
                Chunks::Number(chunks) => {
                    if let Some(value) = chunks[value / CHUNK_SIZE][value % CHUNK_SIZE] {
                        counter.add(number_text(value).capacity())?;
                    }
                }
                Chunks::Text(chunks) => {
                    if let Some(value) = &chunks[value / CHUNK_SIZE][value % CHUNK_SIZE] {
                        parse_number(value).map_err(|error| self.conversion_error(row, error))?;
                    }
                }
            }
        }
        conversion_scratch(&counter)?;
        control.checkpoint(self.len, Some(total))?;
        let metadata = convert_metadata(self.metadata(), kind, control, self.len, total)?;
        let chunks = match &self.chunks {
            Chunks::Number(chunks) => {
                let mut converted = Vec::with_capacity(chunks.len());
                for (index, chunk) in chunks.iter().enumerate() {
                    let mut values = Vec::with_capacity(chunk.len());
                    for (offset, value) in chunk.iter().enumerate() {
                        let row = index * CHUNK_SIZE + offset;
                        if offset.is_multiple_of(256) {
                            control.checkpoint(self.len + row, Some(total))?;
                        }
                        values.push(value.map(number_text));
                    }
                    converted.push(Arc::from(values));
                }
                Chunks::Text(converted)
            }
            Chunks::Text(chunks) => {
                let mut converted = Vec::with_capacity(chunks.len());
                for (index, chunk) in chunks.iter().enumerate() {
                    let mut values = Vec::with_capacity(chunk.len());
                    for (offset, value) in chunk.iter().enumerate() {
                        let row = index * CHUNK_SIZE + offset;
                        if offset.is_multiple_of(256) {
                            control.checkpoint(self.len + row, Some(total))?;
                        }
                        values.push(
                            value
                                .as_ref()
                                .map(|value| parse_number(value))
                                .transpose()?,
                        );
                    }
                    converted.push(Arc::from(values));
                }
                Chunks::Number(converted)
            }
        };
        let column = Self {
            id: self.id,
            len: self.len,
            chunks,
            metadata: Arc::new(metadata),
        };
        column.validate()?;
        for row in 0..self.len {
            if row.is_multiple_of(256) {
                control.checkpoint(self.len * 2 + row, Some(total))?;
            }
            if self.is_missing(row)? != column.is_missing(row)?
                || self.label_at(row) != column.label_at(row)
            {
                return Err(AppError::invalid(format!("column {}, row {}", self.id.0, row + 1),
                    "Conversion would change missingness or value labels. Revise those definitions before changing type."));
            }
        }
        control.checkpoint(total, Some(total))?;
        Ok(column)
    }
}
fn missing_checkpoint(control: Option<&JobControl>, done: usize) -> AppResult<()> {
    if let Some(control) = control {
        control.checkpoint(done, None)?;
    }
    Ok(())
}
fn numeric_missing(
    value: f64,
    rules: &MissingRules,
    control: Option<&JobControl>,
) -> AppResult<bool> {
    if let MissingRules::Number { values, ranges } = rules {
        for (i, candidate) in values.iter().enumerate() {
            if i.is_multiple_of(64) {
                missing_checkpoint(control, i)?;
            }
            if *candidate == value {
                return Ok(true);
            }
        }
        for (i, (lo, hi)) in ranges.iter().enumerate() {
            if i.is_multiple_of(64) {
                missing_checkpoint(control, i)?;
            }
            if *lo <= value && value <= *hi {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn text_missing(
    value: &str,
    rules: &MissingRules,
    control: Option<&JobControl>,
) -> AppResult<bool> {
    if let MissingRules::Text(values) = rules {
        for (i, candidate) in values.iter().enumerate() {
            if i.is_multiple_of(64) {
                missing_checkpoint(control, i)?;
            }
            if missing_text_equal(candidate, value, control)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn missing_text_equal(a: &str, b: &str, control: Option<&JobControl>) -> AppResult<bool> {
    if a.len() != b.len() {
        return Ok(false);
    }
    for (i, (a, b)) in a
        .as_bytes()
        .chunks(4096)
        .zip(b.as_bytes().chunks(4096))
        .enumerate()
    {
        missing_checkpoint(control, i * 4096)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn parse_number(text: &str) -> AppResult<f64> {
    let text = text.trim();
    let number = text
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| AppError::invalid("number", "Enter a finite number."))?;
    if !integer_is_exact(text, number) {
        return Err(AppError::invalid(
            "number",
            "This integer cannot be stored exactly as Float64. Keep the column as text.",
        ));
    }
    Ok(number)
}
// Ordinary fractional decimals retain Float64 semantics; integer lexemes must retain every digit.
fn integer_is_exact(text: &str, number: f64) -> bool {
    if let Ok(integer) = text.parse::<i128>() {
        let magnitude = integer.unsigned_abs();
        return number.abs() < u128::MAX as f64 && number.abs() as u128 == magnitude;
    }
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let mantissa = mantissa.trim_start_matches(['+', '-']);
    let digits = mantissa.bytes().filter(|byte| *byte != b'.');
    let Some(first) = digits.clone().position(|byte| byte != b'0') else {
        return true;
    };
    let last = digits
        .clone()
        .enumerate()
        .filter(|(_, byte)| *byte != b'0')
        .map(|(index, _)| index)
        .last()
        .unwrap_or(first);
    let count = digits.clone().count();
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let Ok(exponent) = exponent.parse::<i64>() else {
        return false;
    };
    let scale = i128::from(exponent) - fraction as i128 + (count - last - 1) as i128;
    if scale < 0 {
        return true;
    }
    let significant = last - first + 1;
    if significant as i128 + scale > 309 {
        return false;
    }
    let normalized = digits
        .skip(first)
        .take(significant)
        .chain(std::iter::repeat_n(b'0', scale as usize));
    if let Some(integer) = normalized.clone().try_fold(0_u128, |value, byte| {
        value.checked_mul(10)?.checked_add(u128::from(byte - b'0'))
    }) {
        return number.abs() < u128::MAX as f64 && number.abs() as u128 == integer;
    }
    normalized.eq(exact_integer_digits(number))
}
fn exact_integer_digits(number: f64) -> Vec<u8> {
    let bits = number.abs().to_bits();
    let significand = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let exponent = ((bits >> 52) & 2047) as i32 - 1023 - 52;
    let mut exact = Vec::with_capacity(309);
    exact.extend(significand.to_string().bytes());
    for _ in 0..exponent {
        let mut carry = 0;
        for digit in exact.iter_mut().rev() {
            let doubled = (*digit - b'0') * 2 + carry;
            *digit = b'0' + doubled % 10;
            carry = doubled / 10;
        }
        if carry != 0 {
            exact.insert(0, b'0' + carry);
        }
    }
    exact
}
fn number_text(number: f64) -> String {
    let text = number.to_string();
    if number.abs() < 9007199254740992.0 || parse_number(&text).is_ok() {
        return text;
    }
    let mut exact = exact_integer_digits(number);
    if number.is_sign_negative() {
        exact.insert(0, b'-');
    }
    String::from_utf8(exact).expect("integer representation contains only ASCII digits and sign")
}

fn conversion_scratch(counter: &AllocationCounter) -> AppResult<()> {
    let mut peak = AllocationCounter {
        bytes: counter.bytes,
    };
    // ponytail: reserve 4 KiB for bounded Float64 formatting/parsing scratch; tighter budgets are refused conservatively.
    peak.add(4096)
}
fn converted_metadata_bytes(
    old: &ColumnMetadata,
    control: &JobControl,
    baseline: usize,
    total: usize,
) -> AppResult<usize> {
    let mut counter = AllocationCounter { bytes: baseline };
    counter.add(old.name.len())?;
    counter.add(old.description.len())?;
    match &old.missing_rules {
        MissingRules::None => {}
        MissingRules::Number { values, ranges } if ranges.is_empty() => {
            counter.array::<String>(values.len())?;
            for (index, value) in values.iter().enumerate() {
                if index.is_multiple_of(256) {
                    control.checkpoint(0, Some(total))?;
                }
                conversion_scratch(&counter)?;
                counter.add(number_text(*value).capacity())?;
            }
        }
        MissingRules::Number { .. } => {
            return Err(AppError::invalid(
                "conversion",
                "Remove numeric missing ranges before converting to text.",
            ));
        }
        MissingRules::Text(values) => {
            counter.array::<f64>(values.len())?;
            for (index, value) in values.iter().enumerate() {
                if index.is_multiple_of(256) {
                    control.checkpoint(0, Some(total))?;
                }
                conversion_scratch(&counter)?;
                parse_number(value)?;
            }
        }
    }
    counter.array::<ValueLabel>(old.value_labels.len())?;
    for (index, label) in old.value_labels.iter().enumerate() {
        if index.is_multiple_of(256) {
            control.checkpoint(0, Some(total))?;
        }
        conversion_scratch(&counter)?;
        counter.add(label.label.len())?;
        match &label.value {
            Cell::Number(value) => counter.add(number_text(*value).capacity())?,
            Cell::Text(value) => {
                parse_number(value)?;
            }
            Cell::Missing => {
                return Err(AppError::invalid(
                    "conversion",
                    "A missing cell cannot have a value label.",
                ));
            }
        }
    }
    Ok(counter.bytes - baseline)
}

fn convert_metadata(
    old: &ColumnMetadata,
    target: ColumnKind,
    control: &JobControl,
    done: usize,
    total: usize,
) -> AppResult<ColumnMetadata> {
    let missing_rules = match &old.missing_rules {
        MissingRules::None => MissingRules::None,
        MissingRules::Number { values, ranges } if ranges.is_empty() => {
            let mut tokens = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                if index.is_multiple_of(256) {
                    control.checkpoint(done, Some(total))?;
                }
                tokens.push(number_text(*value));
            }
            MissingRules::Text(tokens)
        }
        MissingRules::Number { .. } => {
            return Err(AppError::invalid(
                "conversion",
                "Remove numeric missing ranges before converting to text.",
            ))
        }
        MissingRules::Text(values) => {
            let mut numbers = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                if index.is_multiple_of(256) {
                    control.checkpoint(done, Some(total))?;
                }
                numbers.push(parse_number(value)?);
            }
            MissingRules::Number {
                values: numbers,
                ranges: vec![],
            }
        }
    };
    let mut value_labels = Vec::with_capacity(old.value_labels.len());
    for (index, label) in old.value_labels.iter().enumerate() {
        if index.is_multiple_of(256) {
            control.checkpoint(done, Some(total))?;
        }
        let value = match &label.value {
            Cell::Number(value) => Cell::Text(number_text(*value)),
            Cell::Text(value) => Cell::Number(parse_number(value)?),
            Cell::Missing => {
                return Err(AppError::invalid(
                    "conversion",
                    "A missing cell cannot have a value label.",
                ))
            }
        };
        value_labels.push(ValueLabel {
            value,
            label: label.label.clone(),
        });
    }
    let metadata = ColumnMetadata {
        name: old.name.clone(),
        description: old.description.clone(),
        measurement: old.measurement,
        display_decimals: old.display_decimals,
        missing_rules,
        value_labels,
    };
    metadata.validate(target)?;
    Ok(metadata)
}

#[cfg(test)]
mod controlled_tests;
