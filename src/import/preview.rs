use super::{
    check_bytes, grow,
    records::{Field, Scanner},
    Delimiter, ImportColumnChoice, ImportColumnPreview, ImportPreview, ImportWarning,
    MAX_INPUT_BYTES,
};
use crate::{
    data::{parse_number, Budget, ColumnKind, Dataset, MissingRules, MAX_CELLS},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::{collections::HashSet, mem::size_of, sync::Arc};

const SAMPLE_ROWS: usize = 20;
const SAMPLE_BYTES: usize = 64 * 1024;
const SAMPLE_CHARS: usize = 120;

impl ImportPreview {
    pub fn columns(&self) -> &[ImportColumnPreview] {
        &self.columns
    }
    pub fn sample_rows(&self) -> &[Vec<String>] {
        &self.samples
    }
    pub fn samples_truncated(&self) -> bool {
        self.samples_truncated
    }
    pub fn row_count(&self) -> usize {
        self.rows
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Retained input, metadata and bounded sample storage; not process RSS.
    pub fn owned_bytes(&self) -> usize {
        self.owned_bytes
    }
    /// Resolve header warnings; assumes no other retained application storage.
    pub fn default_choices(&self) -> AppResult<Vec<ImportColumnChoice>> {
        self.default_choices_with_budget(&[], 0)
    }
    /// Count current datasets, results and pending work before cloning header names.
    pub fn default_choices_with_budget(
        &self,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Vec<ImportColumnChoice>> {
        let mut bytes = check_bytes(
            Budget::dataset_bytes(retained, extra_bytes)?,
            self.owned_bytes,
        )?;
        bytes = check_bytes(bytes, self.columns.len() * size_of::<ImportColumnChoice>())?;
        for column in &self.columns {
            bytes = check_bytes(bytes, column.header.len())?;
        }
        Ok(self
            .columns
            .iter()
            .map(|column| ImportColumnChoice {
                name: column.header.clone(),
                kind: column.inferred_kind,
                missing_rules: MissingRules::None,
                acknowledge_precision_loss: false,
            })
            .collect())
    }
}

pub fn preview(
    bytes: Arc<[u8]>,
    delimiter: Delimiter,
    control: &JobControl,
) -> AppResult<ImportPreview> {
    preview_with_budget(bytes, delimiter, control, &[], 0)
}
/// Include the current project, retained revisions and non-dataset work in the job's budget.
/// Callers must count retained results/pending jobs in `extra_bytes`.
pub fn preview_with_budget(
    bytes: Arc<[u8]>,
    delimiter: Delimiter,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<ImportPreview> {
    control.checkpoint(0, Some(bytes.len()))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(AppError::SizeLimit {
            limit: MAX_INPUT_BYTES,
        });
    }
    let retained_bytes = Budget::dataset_bytes(retained, extra_bytes)?;
    let owned = check_bytes(
        bytes.len(),
        size_of::<ImportPreview>() + 2 * size_of::<usize>(),
    )?;
    let base = check_bytes(retained_bytes, owned)?;
    let mut scanner = Scanner::new(&bytes, delimiter, control, base)?;
    let (mut columns, live) = headers(&mut scanner, base)?;
    mark_headers(&mut columns, live)?;
    let (samples, rows, truncated) = scan_data(&mut scanner, &mut columns, live)?;
    control.checkpoint(bytes.len(), Some(bytes.len()))?;
    let mut result = ImportPreview {
        bytes,
        delimiter,
        columns,
        samples,
        rows,
        samples_truncated: truncated,
        owned_bytes: 0,
    };
    result.owned_bytes = retained_preview_bytes(&result)?;
    check_bytes(retained_bytes, result.owned_bytes)?;
    Ok(result)
}

fn scan_data(
    scanner: &mut Scanner<'_>,
    columns: &mut [ImportColumnPreview],
    live: usize,
) -> AppResult<(Vec<Vec<String>>, usize, bool)> {
    // Reserve the sample ceiling before decoding any data. Samples never hold complete large fields.
    scanner.budget_bytes = check_bytes(live, SAMPLE_BYTES)?;
    let mut samples = Vec::new();
    let mut sample = Vec::new();
    let mut sample_bytes = 0;
    let mut sample_live = 0;
    let mut truncated = false;
    let mut rows = 0;
    while let Some(field) = scanner.next()? {
        rows = observe(&field, columns)?;
        if rows <= SAMPLE_ROWS {
            collect_sample(
                &field,
                &mut sample,
                &mut sample_live,
                sample_bytes,
                &mut truncated,
            );
        } else {
            truncated = true;
        }
        if field.end_record {
            if sample.len() == columns.len() {
                sample_bytes += sample_live + size_of::<Vec<String>>();
                samples.push(std::mem::take(&mut sample));
            } else {
                sample = Vec::new();
            }
            sample_live = 0;
        }
    }
    for column in columns {
        if !column.seen_nonblank {
            column.inferred_kind = ColumnKind::Text;
        }
    }
    Ok((samples, rows, truncated))
}

fn observe(field: &Field<'_>, columns: &mut [ImportColumnPreview]) -> AppResult<usize> {
    validate_width(field, columns.len())?;
    let rows = field.row - 1;
    if rows
        .checked_mul(columns.len())
        .is_none_or(|n| n > MAX_CELLS)
    {
        return Err(AppError::SizeLimit { limit: MAX_CELLS });
    }
    infer(&mut columns[field.column - 1], &field.text);
    Ok(rows)
}

fn headers(scanner: &mut Scanner<'_>, base: usize) -> AppResult<(Vec<ImportColumnPreview>, usize)> {
    let mut columns = Vec::new();
    let mut live = base;
    while let Some(field) = scanner.next()? {
        if columns.len() >= MAX_CELLS {
            return Err(AppError::SizeLimit { limit: MAX_CELLS });
        }
        push_header(&mut columns, &mut live, field.text)?;
        scanner.budget_bytes = live;
        if field.end_record {
            return Ok((columns, live));
        }
    }
    Err(AppError::invalid("row 1, field 1", "Enter a header row."))
}
fn push_header(
    columns: &mut Vec<ImportColumnPreview>,
    live: &mut usize,
    text: std::borrow::Cow<'_, str>,
) -> AppResult<()> {
    let owned_bytes = match &text {
        std::borrow::Cow::Owned(value) => value.capacity(),
        std::borrow::Cow::Borrowed(_) => 0,
    };
    // The decoded owned field is already live when the descriptor vector grows.
    *live = check_bytes(*live, owned_bytes)?;
    grow(columns, live)?;
    let copied_bytes = if owned_bytes == 0 { text.len() } else { 0 };
    *live = check_bytes(*live, copied_bytes + 3 * size_of::<ImportWarning>())?;
    columns.push(ImportColumnPreview {
        header: text.into_owned(),
        inferred_kind: ColumnKind::Number,
        warnings: Vec::with_capacity(3),
        seen_nonblank: false,
    });
    Ok(())
}
fn mark_headers(columns: &mut [ImportColumnPreview], live: usize) -> AppResult<()> {
    // Hash table and duplicate indexes exist only during header validation.
    check_bytes(
        live,
        columns.len().checked_mul(128).ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?,
    )?;
    let mut names = HashSet::with_capacity(columns.len());
    let duplicates: Vec<_> = columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| (!names.insert(column.header.as_str())).then_some(index))
        .collect();
    drop(names);
    for column in columns.iter_mut() {
        if column.header.trim().is_empty() || column.header.chars().any(char::is_control) {
            column.warnings.push(ImportWarning::InvalidHeader);
        }
    }
    for index in duplicates {
        columns[index].warnings.push(ImportWarning::DuplicateHeader);
    }
    Ok(())
}
fn infer(column: &mut ImportColumnPreview, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    column.seen_nonblank = true;
    let number = parse_number(text);
    let risk = precision_risk(text, number.is_err());
    if risk && !column.warnings.contains(&ImportWarning::PrecisionRisk) {
        column.warnings.push(ImportWarning::PrecisionRisk);
    }
    if number.is_err() || risk {
        column.inferred_kind = ColumnKind::Text;
    }
}
/// Float64-exact large integers are still identifiers at risk during later editing/export.
pub(super) fn precision_risk(text: &str, conversion_failed: bool) -> bool {
    let text = text.trim();
    if let Ok(number) = text.parse::<f64>() {
        if number.is_finite() {
            return conversion_failed
                || (number.fract() == 0.0 && number.abs() > 9007199254740992.0);
        }
    }
    let digits = text.trim_start_matches(['+', '-']);
    digits.len() > 16 && digits.bytes().all(|byte| byte.is_ascii_digit())
}
pub(super) fn validate_width(field: &Field<'_>, columns: usize) -> AppResult<()> {
    if field.column > columns || (field.end_record && field.column != columns) {
        Err(field.invalid("Each data row must have the same number of fields as the header."))
    } else {
        Ok(())
    }
}
fn collect_sample(
    field: &Field<'_>,
    sample: &mut Vec<String>,
    live: &mut usize,
    retained: usize,
    truncated: &mut bool,
) {
    if sample.len() + 1 != field.column {
        return;
    }
    let end = field
        .text
        .char_indices()
        .nth(SAMPLE_CHARS)
        .map_or(field.text.len(), |(index, _)| index);
    *truncated |= end < field.text.len();
    let old_capacity = sample.capacity();
    let next_capacity = if sample.len() == old_capacity {
        old_capacity.saturating_mul(2).max(4)
    } else {
        old_capacity
    };
    let needed = end + (next_capacity - old_capacity) * size_of::<String>();
    // Check descriptor capacity before growth, as well as the copied field text.
    if retained + *live + needed + size_of::<Vec<String>>() * SAMPLE_ROWS * 2 > SAMPLE_BYTES {
        *truncated = true;
        return;
    }
    if next_capacity > old_capacity {
        sample.reserve_exact(next_capacity - sample.len());
    }
    sample.push(field.text[..end].to_owned());
    *live += end + (sample.capacity() - old_capacity) * size_of::<String>();
}
fn retained_preview_bytes(preview: &ImportPreview) -> AppResult<usize> {
    let mut bytes = check_bytes(
        preview.bytes.len(),
        size_of::<ImportPreview>() + 2 * size_of::<usize>(),
    )?;
    bytes = check_bytes(
        bytes,
        preview.columns.capacity() * size_of::<ImportColumnPreview>(),
    )?;
    for column in &preview.columns {
        bytes = check_bytes(
            bytes,
            column.header.capacity() + column.warnings.capacity() * size_of::<ImportWarning>(),
        )?;
    }
    bytes = check_bytes(bytes, preview.samples.capacity() * size_of::<Vec<String>>())?;
    for row in &preview.samples {
        bytes = check_bytes(bytes, row.capacity() * size_of::<String>())?;
        for value in row {
            bytes = check_bytes(bytes, value.capacity())?;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    // Catches vector growth that excludes the already-owned current decoded header.
    #[test]
    fn escaped_header_is_charged_before_descriptor_growth() {
        let source = format!("\"{}\"\" header\"", "x".repeat(1024));
        let control = JobControl::default();
        let mut scanner = Scanner::new(source.as_bytes(), Delimiter::Comma, &control, 0).unwrap();
        let field = scanner.next().unwrap().unwrap();
        assert!(matches!(&field.text, Cow::Owned(_)));
        let mut columns = Vec::new();
        let growth_bytes = 4 * size_of::<ImportColumnPreview>();
        let mut live = crate::data::MAX_OWNED_BYTES - field.text.len() - growth_bytes / 2;
        assert!(matches!(
            push_header(&mut columns, &mut live, field.text),
            Err(AppError::SizeLimit { .. })
        ));
        assert_eq!(
            columns.capacity(),
            0,
            "Rejected work must not grow the descriptor vector."
        );
    }

    #[test]
    fn sample_vector_never_allocates_above_ceiling() {
        let mut sample = Vec::new();
        let mut live = 0;
        let mut truncated = false;
        for column in 1..=3000 {
            let field = Field {
                text: Cow::Borrowed(""),
                row: 2,
                column,
                end_record: column == 3000,
            };
            collect_sample(&field, &mut sample, &mut live, 0, &mut truncated);
            assert!(sample.capacity() * size_of::<String>() <= SAMPLE_BYTES);
        }
        assert!(truncated);
        assert!(sample.len() < 3000);
    }
}
