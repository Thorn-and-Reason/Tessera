use super::{
    check_bytes, grow,
    records::{Field, Scanner},
    Delimiter, ImportColumnChoice, ImportColumnPreview, ImportColumnReview,
    ImportConversionExample, ImportConversionNote, ImportPreview, ImportWarning, MAX_INPUT_BYTES,
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
// Three complete examples, their stored values and bounded formatting.
pub(crate) const REVIEW_WORK_BYTES: usize = 8 * 1024;

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
    /// Bounded source-to-stored examples for the currently selected import decisions.
    ///
    /// Examples are drawn from the retained sample rows only, but each conversion uses the
    /// same numeric parser and blank handling as `commit_preview`.
    pub fn review_choices(
        &self,
        choices: &[ImportColumnChoice],
    ) -> AppResult<Vec<ImportColumnReview>> {
        if choices.len() != self.columns.len() {
            return Err(AppError::invalid(
                "import columns",
                "Choose a type and unique name for every column.",
            ));
        }
        let baseline = check_bytes(self.owned_bytes, super::choice_bytes(choices)?)?;
        let mut work = choices
            .len()
            .checked_mul(size_of::<ImportColumnReview>() + REVIEW_WORK_BYTES)
            .ok_or(AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?;
        for choice in choices {
            work = check_bytes(work, choice.name.len())?;
        }
        check_bytes(baseline, work)?;
        let mut reviews = Vec::with_capacity(choices.len());
        for (index, choice) in choices.iter().enumerate() {
            reviews.push(self.review_choice(index, choice)?);
        }
        Ok(reviews)
    }

    /// Build the conversion review for a single rendered column. This avoids allocating
    /// review copies for every column while the import dialog virtualizes the list.
    pub fn review_choice(
        &self,
        index: usize,
        choice: &ImportColumnChoice,
    ) -> AppResult<ImportColumnReview> {
        if index >= self.columns.len() {
            return Err(AppError::invalid(
                "import columns",
                "Choose a type and unique name for every column.",
            ));
        }
        super::validate_missing(&choice.missing_rules, choice.kind, "Conversion preview")?;
        let baseline = check_bytes(
            self.owned_bytes,
            super::choice_bytes(std::slice::from_ref(choice))?,
        )?;
        check_bytes(check_bytes(baseline, choice.name.len())?, REVIEW_WORK_BYTES)?;
        let mut notes = Vec::new();
        if self.samples.len() < self.rows.min(SAMPLE_ROWS) {
            push_note(&mut notes, ImportConversionNote::ShortenedOrOmittedSamples);
        }
        if choice.kind == ColumnKind::Number
            && self.columns[index]
                .warnings
                .contains(&ImportWarning::PrecisionRisk)
        {
            push_note(&mut notes, ImportConversionNote::NumericPrecisionMayRound);
        }
        let mut examples = Vec::new();
        for (row, complete) in self.samples.iter().zip(&self.sample_complete) {
            let Some(source) = row.get(index) else {
                push_note(&mut notes, ImportConversionNote::ShortenedOrOmittedSamples);
                continue;
            };
            if !complete.get(index).copied().unwrap_or(false) {
                push_note(&mut notes, ImportConversionNote::ShortenedOrOmittedSamples);
                continue;
            }
            match review_example(source, choice, &mut notes) {
                Ok(example) => examples.push(example),
                Err(_)
                    if choice.kind == ColumnKind::Number
                        && !choice.acknowledge_precision_loss
                        && super::import_number(source, true).is_ok() =>
                {
                    // Retain the warning without inventing a stored value or suppressing
                    // unrelated valid examples. Commit still requires acknowledgement.
                    push_note(&mut notes, ImportConversionNote::NumericPrecisionMayRound);
                    continue;
                }
                Err(error) => return Err(error),
            }
            if examples.len() == 3 {
                break;
            }
        }
        Ok(ImportColumnReview {
            name: choice.name.clone(),
            kind: choice.kind,
            examples,
            notes,
        })
    }
}

fn review_example(
    source: &str,
    choice: &ImportColumnChoice,
    notes: &mut Vec<ImportConversionNote>,
) -> AppResult<ImportConversionExample> {
    match choice.kind {
        ColumnKind::Number => review_number_example(source, choice, notes),
        ColumnKind::Text => Ok(review_text_example(source, choice, notes)),
    }
}

fn review_number_example(
    source: &str,
    choice: &ImportColumnChoice,
    notes: &mut Vec<ImportConversionNote>,
) -> AppResult<ImportConversionExample> {
    if source.trim().is_empty() {
        push_note(notes, ImportConversionNote::BlankNumberImportsAsMissing);
        return Ok(ImportConversionExample {
            source: source.to_owned(),
            stored: "Missing".into(),
            marked_missing: true,
        });
    }
    let number = super::import_number(source, choice.acknowledge_precision_loss)?;
    let stored = number.to_string();
    if source.trim() != stored {
        push_note(
            notes,
            ImportConversionNote::KeepAsTextToPreserveExactCharacters,
        );
    }
    Ok(ImportConversionExample {
        source: source.to_owned(),
        stored,
        marked_missing: matches!(&choice.missing_rules, MissingRules::Number { values, ranges }
            if values.contains(&number) || ranges.iter().any(|(lo, hi)| *lo <= number && number <= *hi)),
    })
}

fn review_text_example(
    source: &str,
    choice: &ImportColumnChoice,
    notes: &mut Vec<ImportConversionNote>,
) -> ImportConversionExample {
    let marked_missing = matches!(
        &choice.missing_rules,
        MissingRules::Text(tokens) if tokens.iter().any(|token| token == source)
    );
    if marked_missing && source.is_empty() {
        push_note(notes, ImportConversionNote::EmptyTextMarkedMissing);
    }
    ImportConversionExample {
        source: source.to_owned(),
        stored: source.to_owned(),
        marked_missing,
    }
}

fn push_note(notes: &mut Vec<ImportConversionNote>, note: ImportConversionNote) {
    if !notes.contains(&note) {
        notes.push(note);
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
    let (samples, sample_complete, rows, truncated) = scan_data(&mut scanner, &mut columns, live)?;
    control.checkpoint(bytes.len(), Some(bytes.len()))?;
    let mut result = ImportPreview {
        bytes,
        delimiter,
        columns,
        samples,
        sample_complete,
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
) -> AppResult<(Vec<Vec<String>>, Vec<Vec<bool>>, usize, bool)> {
    // Reserve the sample ceiling before decoding any data. Samples never hold complete large fields.
    scanner.budget_bytes = check_bytes(live, SAMPLE_BYTES)?;
    let mut samples = Vec::new();
    let mut sample_complete_rows = Vec::new();
    let mut sample = Vec::new();
    let mut sample_complete = Vec::new();
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
                &mut sample_complete,
                &mut sample_live,
                sample_bytes,
                &mut truncated,
            );
        } else {
            truncated = true;
        }
        if field.end_record {
            if sample.len() == columns.len() {
                sample_bytes += sample_live + size_of::<Vec<String>>() + size_of::<Vec<bool>>();
                samples.push(std::mem::take(&mut sample));
                sample_complete_rows.push(std::mem::take(&mut sample_complete));
            } else {
                sample = Vec::new();
                sample_complete = Vec::new();
            }
            sample_live = 0;
        }
    }
    for column in columns {
        if !column.seen_nonblank {
            column.inferred_kind = ColumnKind::Text;
        }
    }
    Ok((samples, sample_complete_rows, rows, truncated))
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
    sample_complete: &mut Vec<bool>,
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
    let complete = end == field.text.len();
    *truncated |= !complete;
    let old_capacity = sample.capacity();
    let next_capacity = if sample.len() == old_capacity {
        old_capacity.saturating_mul(2).max(4)
    } else {
        old_capacity
    };
    let needed = end
        + (next_capacity - old_capacity) * size_of::<String>()
        + (next_capacity - old_capacity) * size_of::<bool>();
    // Check descriptor capacity before growth, as well as the copied field text.
    if retained + *live + needed + size_of::<Vec<String>>() * SAMPLE_ROWS * 2 > SAMPLE_BYTES {
        *truncated = true;
        return;
    }
    if next_capacity > old_capacity {
        sample.reserve_exact(next_capacity - sample.len());
        sample_complete.reserve_exact(next_capacity - sample_complete.len());
    }
    sample.push(field.text[..end].to_owned());
    sample_complete.push(complete);
    *live += end
        + (sample.capacity() - old_capacity) * size_of::<String>()
        + (sample_complete.capacity() - old_capacity) * size_of::<bool>();
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
    bytes = check_bytes(
        bytes,
        preview.sample_complete.capacity() * size_of::<Vec<bool>>(),
    )?;
    for row in &preview.samples {
        bytes = check_bytes(bytes, row.capacity() * size_of::<String>())?;
        for value in row {
            bytes = check_bytes(bytes, value.capacity())?;
        }
    }
    for row in &preview.sample_complete {
        bytes = check_bytes(bytes, row.capacity() * size_of::<bool>())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[test]
    fn precision_blocked_sample_preserves_other_valid_conversion_examples() {
        let control = JobControl::default();
        let preview = preview(
            Arc::from(&b"Identifier\n99999999999999999999\n1\n2\n"[..]),
            Delimiter::Comma,
            &control,
        )
        .unwrap();
        let mut choices = preview.default_choices().unwrap();
        choices[0].kind = ColumnKind::Number;
        let review = preview.review_choice(0, &choices[0]).unwrap();
        assert!(review
            .notes
            .contains(&ImportConversionNote::NumericPrecisionMayRound));
        assert_eq!(
            review
                .examples
                .iter()
                .map(|example| example.stored.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        assert!(super::super::commit_preview(&preview, &choices, &control).is_err());
        choices[0].acknowledge_precision_loss = true;
        assert_eq!(
            preview
                .review_choice(0, &choices[0])
                .unwrap()
                .examples
                .len(),
            3
        );
        for invalid in ["invalid", "inf", "NaN"] {
            let source = format!("Number\n{invalid}\n1\n");
            let invalid_preview =
                super::preview(Arc::from(source.as_bytes()), Delimiter::Comma, &control).unwrap();
            let mut invalid_choices = invalid_preview.default_choices().unwrap();
            invalid_choices[0].kind = ColumnKind::Number;
            assert!(invalid_preview
                .review_choice(0, &invalid_choices[0])
                .is_err());
        }
    }

    #[test]
    fn reviewed_numeric_missing_values_and_ranges_match_committed_data() {
        let control = JobControl::default();
        let preview = preview(
            Arc::from(&b"Score\n99\n5\n12\n"[..]),
            Delimiter::Comma,
            &control,
        )
        .unwrap();
        let mut choices = preview.default_choices().unwrap();
        choices[0].missing_rules = MissingRules::Number {
            values: vec![99.0],
            ranges: vec![(1.0, 5.0)],
        };
        let review = preview.review_choice(0, &choices[0]).unwrap();
        let data = super::super::commit_preview(&preview, &choices, &control).unwrap();
        for (row, example) in review.examples.iter().enumerate() {
            assert_eq!(
                example.marked_missing,
                data.columns()[0].is_missing(row).unwrap(),
                "{example:?}"
            );
        }
        assert_eq!(review.examples[0].stored, "99");
        assert_eq!(review.examples[1].stored, "5");
    }

    #[test]
    fn incomplete_conversion_samples_are_disclosed_without_invented_examples() {
        let control = JobControl::default();
        let text = format!("Text\n{}\n", "x".repeat(SAMPLE_CHARS + 1));
        let preview = preview(Arc::from(text.as_bytes()), Delimiter::Comma, &control).unwrap();
        let choice = preview.default_choices().unwrap().remove(0);
        let review = preview.review_choice(0, &choice).unwrap();
        assert!(review.examples.is_empty());
        assert!(
            !review.notes.is_empty(),
            "shortened samples need an explanation"
        );

        let headers = (0..3000)
            .map(|i| format!("C{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let row = std::iter::repeat_n("x", 3000).collect::<Vec<_>>().join(",");
        let source = format!("{headers}\n{row}\n");
        let wide =
            super::preview(Arc::from(source.as_bytes()), Delimiter::Comma, &control).unwrap();
        let choices = wide.default_choices().unwrap();
        let review = wide.review_choice(2999, &choices[2999]).unwrap();
        assert!(review.examples.is_empty());
        assert!(
            !review.notes.is_empty(),
            "omitted samples are not blank values"
        );
    }

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
        let mut complete = Vec::new();
        let mut live = 0;
        let mut truncated = false;
        for column in 1..=3000 {
            let field = Field {
                text: Cow::Borrowed(""),
                row: 2,
                column,
                end_record: column == 3000,
            };
            collect_sample(
                &field,
                &mut sample,
                &mut complete,
                &mut live,
                0,
                &mut truncated,
            );
            assert!(sample.capacity() * size_of::<String>() <= SAMPLE_BYTES);
        }
        assert!(truncated);
        assert!(sample.len() < 3000);
    }
}
