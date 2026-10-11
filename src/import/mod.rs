//! Bounded CSV/TSV import. A successful caller drops the immutable preview after publication;
//! cancellation or validation errors leave it available for another choice or attempt.
pub mod excel;
mod preview;
mod records;

use crate::{
    data::{
        parse_number, Budget, Column, ColumnId, ColumnKind, ColumnMetadata, Dataset, MissingRules,
        CHUNK_SIZE, MAX_OWNED_BYTES,
    },
    error::{AppError, AppResult},
    operation::JobControl,
};
pub use preview::{preview, preview_with_budget};
pub(crate) use records::Scanner;
use std::{collections::HashSet, mem::size_of, sync::Arc};

pub const MAX_INPUT_BYTES: usize = 128 * 1024 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    Comma,
    Tab,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportWarning {
    PrecisionRisk,
    InvalidHeader,
    DuplicateHeader,
}
#[derive(Debug)]
pub struct ImportColumnPreview {
    pub header: String,
    pub inferred_kind: ColumnKind,
    pub warnings: Vec<ImportWarning>,
    seen_nonblank: bool,
}
#[derive(Debug, Clone)]
pub struct ImportColumnChoice {
    pub name: String,
    pub kind: ColumnKind,
    pub missing_rules: MissingRules,
    pub acknowledge_precision_loss: bool,
}
#[derive(Debug)]
pub struct ImportPreview {
    bytes: Arc<[u8]>,
    delimiter: Delimiter,
    columns: Vec<ImportColumnPreview>,
    samples: Vec<Vec<String>>,
    rows: usize,
    samples_truncated: bool,
    owned_bytes: usize,
}

enum Values {
    Number(Vec<Option<f64>>),
    Text(Vec<Option<String>>),
}

pub fn commit_preview(
    preview: &ImportPreview,
    choices: &[ImportColumnChoice],
    control: &JobControl,
) -> AppResult<Dataset> {
    commit_preview_with_budget(preview, choices, control, &[], 0)
}
/// Count the current project/revisions and other job-owned storage before typed allocation.
/// This includes retained source and preview samples until the caller drops the preview.
pub fn commit_preview_with_budget(
    preview: &ImportPreview,
    choices: &[ImportColumnChoice],
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<Dataset> {
    control.checkpoint(0, Some(preview.bytes.len()))?;
    let base = check_bytes(
        Budget::dataset_bytes(retained, extra_bytes)?,
        preview.owned_bytes,
    )?;
    let mut live = validate_choices(preview, choices, base)?;
    live = construction_bytes(preview.rows, choices, live)?;
    let values = read_values(preview, choices, control, live)?;
    let columns = build_columns(values, choices, control)?;
    let dataset = Dataset::new(columns)?;
    // Recheck actual final ownership, including retained input and choices, before returning.
    let mut datasets = retained.to_vec();
    datasets.push(&dataset);
    Budget::check_datasets(
        &datasets,
        check_bytes(
            extra_bytes,
            check_bytes(preview.owned_bytes, choice_bytes(choices)?)?,
        )?,
    )?;
    control.checkpoint(preview.bytes.len(), Some(preview.bytes.len()))?;
    Ok(dataset)
}

fn read_values(
    preview: &ImportPreview,
    choices: &[ImportColumnChoice],
    control: &JobControl,
    mut live: usize,
) -> AppResult<Vec<Values>> {
    let mut values = allocate_values(preview.rows, choices);
    let mut scanner = Scanner::new(&preview.bytes, preview.delimiter, control, live)?;
    while let Some(field) = scanner.next()? {
        if field.row == 1 {
            continue;
        }
        preview::validate_width(&field, choices.len())?;
        let index = field.column - 1;
        match &mut values[index] {
            Values::Number(numbers) => {
                let number = if field.text.trim().is_empty() {
                    None
                } else {
                    Some(
                        import_number(&field.text, choices[index].acknowledge_precision_loss)
                            .map_err(|error| match error {
                                AppError::InvalidInput { message, .. } => field.invalid(&message),
                                error => error,
                            })?,
                    )
                };
                numbers.push(number);
            }
            Values::Text(texts) => {
                live = check_bytes(live, field.text.len())?;
                texts.push(Some(field.text.into_owned()));
            }
        }
        scanner.budget_bytes = live;
    }
    Ok(values)
}

fn validate_choices(
    preview: &ImportPreview,
    choices: &[ImportColumnChoice],
    base: usize,
) -> AppResult<usize> {
    if choices.len() != preview.columns.len() {
        return Err(AppError::invalid(
            "import columns",
            "Choose a type and unique name for every column.",
        ));
    }
    let live = check_bytes(base, choice_bytes(choices)?)?;
    check_bytes(live, choices.len() * 128)?;
    let mut names = HashSet::with_capacity(choices.len());
    for (index, choice) in choices.iter().enumerate() {
        let location = format!("row 1, field {}", index + 1);
        if choice.name.trim().is_empty()
            || choice.name.chars().any(char::is_control)
            || !names.insert(choice.name.as_str())
        {
            return Err(AppError::invalid(
                location,
                "Enter unique nonblank names without control characters.",
            ));
        }
        if choice.kind == ColumnKind::Number
            && !choice.acknowledge_precision_loss
            && preview.columns[index]
                .warnings
                .contains(&ImportWarning::PrecisionRisk)
        {
            return Err(AppError::invalid(
                location,
                "Keep large identifiers as text or acknowledge numeric precision loss.",
            ));
        }
        validate_missing(&choice.missing_rules, choice.kind, &location)?;
    }
    Ok(live)
}
fn validate_missing(rules: &MissingRules, kind: ColumnKind, location: &str) -> AppResult<()> {
    let valid = match rules {
        MissingRules::None => true,
        MissingRules::Text(_) => kind == ColumnKind::Text,
        MissingRules::Number { values, ranges } => {
            kind == ColumnKind::Number
                && values.iter().all(|n| n.is_finite())
                && ranges
                    .iter()
                    .all(|(lo, hi)| lo.is_finite() && hi.is_finite() && lo <= hi)
        }
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::invalid(
            location,
            "Missing rules must match the column type and use finite ordered numeric ranges.",
        ))
    }
}
fn import_number(text: &str, acknowledge: bool) -> AppResult<f64> {
    let safe = parse_number(text);
    if !acknowledge {
        return safe;
    }
    // Acknowledgement permits integer rounding, never invalid or infinite numbers.
    safe.or_else(|_| {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| AppError::invalid("number", "Enter a finite number."))
    })
}
fn construction_bytes(
    rows: usize,
    choices: &[ImportColumnChoice],
    mut live: usize,
) -> AppResult<usize> {
    live = check_bytes(
        live,
        choices.len() * (size_of::<Values>() + size_of::<Column>()),
    )?;
    live = check_bytes(live, size_of::<Dataset>())?;
    let chunks = rows.div_ceil(CHUNK_SIZE);
    for choice in choices {
        let width = match choice.kind {
            ColumnKind::Number => size_of::<Option<f64>>(),
            ColumnKind::Text => size_of::<Option<String>>(),
        };
        live = check_bytes(
            live,
            rows.checked_mul(width).ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?,
        )?;
        // Final metadata, Arc control blocks and both source/destination chunk-reference arrays.
        live = check_bytes(
            live,
            2 * (size_of::<ColumnMetadata>() + 2 * size_of::<usize>() + choice.name.len())
                + missing_bytes(&choice.missing_rules)?
                + chunks * 64,
        )?;
    }
    // Constructors retain one source vector while immutable chunks are assembled.
    let width = if choices.iter().any(|choice| choice.kind == ColumnKind::Text) {
        size_of::<Option<String>>()
    } else {
        size_of::<Option<f64>>()
    };
    live = check_bytes(live, rows * width)?;
    // Dataset uniqueness checks and ownership accounting use temporary sets.
    live = check_bytes(live, choices.len() * (chunks + 1) * 128)?;
    // Column::text moves strings, with at most one temporary chunk of descriptors.
    check_bytes(live, CHUNK_SIZE.min(rows) * size_of::<Option<String>>() * 2)
}
fn allocate_values(rows: usize, choices: &[ImportColumnChoice]) -> Vec<Values> {
    choices
        .iter()
        .map(|choice| match choice.kind {
            ColumnKind::Number => Values::Number(Vec::with_capacity(rows)),
            ColumnKind::Text => Values::Text(Vec::with_capacity(rows)),
        })
        .collect()
}
fn build_columns(
    values: Vec<Values>,
    choices: &[ImportColumnChoice],
    control: &JobControl,
) -> AppResult<Vec<Column>> {
    let mut columns = Vec::with_capacity(choices.len());
    for (index, (values, choice)) in values.into_iter().zip(choices).enumerate() {
        control.checkpoint(index, Some(choices.len()))?;
        let id = ColumnId(index as u64 + 1);
        let column = match values {
            Values::Number(values) => Column::numeric(id, &choice.name, values)?,
            Values::Text(values) => Column::text(id, &choice.name, values)?,
        };
        let metadata = ColumnMetadata {
            missing_rules: choice.missing_rules.clone(),
            ..column.metadata().clone()
        };
        columns.push(column.with_metadata(metadata)?);
    }
    Ok(columns)
}
fn missing_bytes(rules: &MissingRules) -> AppResult<usize> {
    match rules {
        MissingRules::None => Ok(0),
        MissingRules::Number { values, ranges } => check_bytes(
            values.capacity() * size_of::<f64>(),
            ranges.capacity() * size_of::<(f64, f64)>(),
        ),
        MissingRules::Text(tokens) => {
            let mut bytes = tokens.capacity() * size_of::<String>();
            for token in tokens {
                bytes = check_bytes(bytes, token.capacity())?;
            }
            Ok(bytes)
        }
    }
}
pub(crate) fn choice_bytes(choices: &[ImportColumnChoice]) -> AppResult<usize> {
    let mut bytes = std::mem::size_of_val(choices);
    for choice in choices {
        bytes = check_bytes(
            bytes,
            check_bytes(
                choice.name.capacity(),
                missing_bytes(&choice.missing_rules)?,
            )?,
        )?;
    }
    Ok(bytes)
}
fn check_bytes(bytes: usize, extra: usize) -> AppResult<usize> {
    bytes
        .checked_add(extra)
        .filter(|total| *total <= MAX_OWNED_BYTES)
        .ok_or(AppError::SizeLimit {
            limit: MAX_OWNED_BYTES,
        })
}
fn grow<T>(values: &mut Vec<T>, live: &mut usize) -> AppResult<()> {
    if values.len() == values.capacity() {
        let capacity = values.capacity().max(2) * 2;
        *live = check_bytes(*live, (capacity - values.capacity()) * size_of::<T>())?;
        values.reserve_exact(capacity - values.len());
    }
    Ok(())
}
