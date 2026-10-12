//! Revision-bound, advisory review of survey variables. Source values are never changed.
pub mod cleaning;
pub mod presentation;
pub mod recipe;
use crate::{
    data::{
        AllocationCounter, Budget, Cell, Column, ColumnId, ColumnKind, Dataset, Measurement,
        RevisionId,
    },
    error::{AppError, AppResult},
    operation::JobControl,
};
pub use presentation::SurveyPresentation;
use std::{cmp::Ordering, mem::size_of};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurveyRole {
    Unresolved,
    Exclude,
    Categories,
    NumericMeasure,
}
impl SurveyRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unresolved => "Choose an action",
            Self::Exclude => "Leave out",
            Self::Categories => "Count responses",
            Self::NumericMeasure => "Summarise a measurement",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurveyChoice {
    pub column: ColumnId,
    pub role: SurveyRole,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurveyFinding {
    Empty,
    AllMissing,
    EmptyCells,
    ExplicitMissingRules,
    CandidateMissingToken,
    LabelVariants,
    MultipleAnswers,
    PersonalDataCandidate,
    FreeTextCandidate,
    HighCardinality,
}
impl SurveyFinding {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Empty => "There are no response rows to summarise.",
            Self::AllMissing => "Current missing rules leave no valid responses for this question.",
            Self::EmptyCells => "Check whether blank text should count as an answer or as missing.",
            Self::ExplicitMissingRules => "These counts use the missing-value definitions already set for this variable.",
            Self::CandidateMissingToken => "Some answers may mean missing; choose their meaning before changing them.",
            Self::LabelVariants => "Similar answers may differ in spaces or letter case; review any merge before applying it.",
            Self::MultipleAnswers => "Choose whether to count each complete cell as one answer; answers are not split automatically.",
            Self::PersonalDataCandidate => "This variable may identify a respondent. Review whether it belongs in the report.",
            Self::FreeTextCandidate => "Long written answers may need a separate qualitative review.",
            Self::HighCardinality => "There are many different answers; a frequency table or chart may be difficult to read.",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "No responses",
            Self::AllMissing => "All responses missing",
            Self::EmptyCells => "Blank or empty cells",
            Self::ExplicitMissingRules => "Missing rules applied",
            Self::CandidateMissingToken => "Possible missing tokens",
            Self::LabelVariants => "Possible label variants",
            Self::MultipleAnswers => "Possible multiple answers",
            Self::PersonalDataCandidate => "Possible personal data",
            Self::FreeTextCandidate => "Possible free text",
            Self::HighCardinality => "Many distinct values",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurveyColumnReview {
    pub column: ColumnId,
    pub suggested_role: SurveyRole,
    pub valid: usize,
    pub missing: usize,
    /// Counts above 30 are represented by 31 ("more than 30").
    pub distinct_values: usize,
    pub findings: Vec<SurveyFinding>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurveyReview {
    pub revision: RevisionId,
    pub columns: Vec<SurveyColumnReview>,
    pub duplicate_rows: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurveyRequest {
    pub revision: RevisionId,
    pub choices: Vec<SurveyChoice>,
}
impl SurveyReview {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = AllocationCounter::default();
        bytes.add(size_of::<Self>())?;
        bytes.array::<SurveyColumnReview>(self.columns.capacity())?;
        for column in &self.columns {
            bytes.array::<SurveyFinding>(column.findings.capacity())?;
        }
        Ok(bytes.bytes)
    }
}
impl SurveyRequest {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = AllocationCounter::default();
        bytes.add(size_of::<Self>())?;
        bytes.array::<SurveyChoice>(self.choices.capacity())?;
        Ok(bytes.bytes)
    }
    pub fn validate(&self, dataset: &Dataset, control: &JobControl) -> AppResult<()> {
        control.check_cancelled()?;
        if self.revision != dataset.revision() {
            return Err(AppError::invalid(
                "survey",
                "The data changed. Review the variables again.",
            ));
        }
        let mut included = false;
        for (index, choice) in self.choices.iter().enumerate() {
            control.check_cancelled()?;
            let column = dataset.column(choice.column)?;
            for earlier in &self.choices[..index] {
                control.check_cancelled()?;
                if earlier.column == choice.column {
                    return Err(AppError::invalid(
                        "survey",
                        "Each variable can appear only once.",
                    ));
                }
            }
            match choice.role {
                SurveyRole::Unresolved => {
                    return Err(AppError::invalid(
                        "survey",
                        "Choose a reporting role or exclude this variable.",
                    ))
                }
                SurveyRole::NumericMeasure if column.kind() != ColumnKind::Number => {
                    return Err(AppError::invalid(
                        "survey",
                        "Numeric measures require a numeric variable.",
                    ))
                }
                SurveyRole::Exclude => {}
                _ => included = true,
            }
        }
        if !included {
            return Err(AppError::invalid(
                "survey",
                "Include at least one supported variable to create a report.",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Raw<'a> {
    Missing,
    Number(f64),
    Text(&'a str),
}
fn raw(column: &Column, row: usize) -> AppResult<Raw<'_>> {
    if column.kind() == ColumnKind::Text {
        return Ok(column.text_value(row)?.map_or(Raw::Missing, Raw::Text));
    }
    Ok(match column.cell(row)? {
        Cell::Number(n) => Raw::Number(n),
        _ => Raw::Missing,
    })
}
fn compare(left: Raw<'_>, right: Raw<'_>, control: &JobControl) -> AppResult<Ordering> {
    control.check_cancelled()?;
    Ok(match (left, right) {
        (Raw::Missing, Raw::Missing) => Ordering::Equal,
        (Raw::Missing, _) => Ordering::Less,
        (_, Raw::Missing) => Ordering::Greater,
        (Raw::Number(a), Raw::Number(b)) => {
            if a == b {
                Ordering::Equal
            } else {
                a.total_cmp(&b)
            }
        }
        (Raw::Text(a), Raw::Text(b)) => compare_iter(a.bytes(), b.bytes(), control)?,
        (Raw::Number(_), Raw::Text(_)) => Ordering::Less,
        (Raw::Text(_), Raw::Number(_)) => Ordering::Greater,
    })
}
fn compare_iter<T: Ord>(
    mut a: impl Iterator<Item = T>,
    mut b: impl Iterator<Item = T>,
    control: &JobControl,
) -> AppResult<Ordering> {
    let mut count = 0;
    loop {
        if count % crate::data::CHUNK_SIZE == 0 {
            control.check_cancelled()?;
        }
        match (a.next(), b.next()) {
            (None, None) => return Ok(Ordering::Equal),
            (None, _) => return Ok(Ordering::Less),
            (_, None) => return Ok(Ordering::Greater),
            (Some(a), Some(b)) if a != b => return Ok(a.cmp(&b)),
            _ => count += 1,
        }
    }
}
// Bottom-up merging makes sorting cancellable without allocating in comparisons.
fn sort_rows(
    rows: &mut Vec<usize>,
    scratch: &mut Vec<usize>,
    control: &JobControl,
    cmp: impl Fn(usize, usize) -> AppResult<Ordering>,
) -> AppResult<()> {
    let len = rows.len();
    let mut width = 1;
    while width < len {
        for start in (0..len).step_by(width * 2) {
            let middle = (start + width).min(len);
            let end = (middle + width).min(len);
            let (mut a, mut b) = (start, middle);
            for destination in &mut scratch[start..end] {
                control.check_cancelled()?;
                if a < middle && (b == end || cmp(rows[a], rows[b])? != Ordering::Greater) {
                    *destination = rows[a];
                    a += 1;
                } else {
                    *destination = rows[b];
                    b += 1;
                }
            }
        }
        std::mem::swap(rows, scratch);
        width *= 2;
    }
    Ok(())
}
fn trimmed<'a>(text: &'a str, control: &JobControl) -> AppResult<&'a str> {
    let mut start = text.len();
    for (count, (index, ch)) in text.char_indices().enumerate() {
        if count % crate::data::CHUNK_SIZE == 0 {
            control.check_cancelled()?;
        }
        if !ch.is_whitespace() {
            start = index;
            break;
        }
    }
    let mut end = start;
    for (count, (index, ch)) in text[start..].char_indices().rev().enumerate() {
        if count % crate::data::CHUNK_SIZE == 0 {
            control.check_cancelled()?;
        }
        if !ch.is_whitespace() {
            end = start + index + ch.len_utf8();
            break;
        }
    }
    Ok(&text[start..end])
}
fn normal(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().flat_map(char::to_lowercase)
}
fn header_candidate(name: &str, words: &[&str], control: &JobControl) -> AppResult<bool> {
    let mut start = 0;
    for (count, (index, ch)) in name.char_indices().enumerate() {
        if count % crate::data::CHUNK_SIZE == 0 {
            control.check_cancelled()?;
        }
        if !ch.is_alphanumeric() {
            if words
                .iter()
                .any(|s| name[start..index].eq_ignore_ascii_case(s))
            {
                return Ok(true);
            }
            start = index + ch.len_utf8();
        }
    }
    control.check_cancelled()?;
    Ok(words.iter().any(|s| name[start..].eq_ignore_ascii_case(s)))
}
fn value_candidates(text: &str, control: &JobControl) -> AppResult<(bool, bool, bool)> {
    let (mut at, mut dot_after_at, mut space_after_at) = (None, false, false);
    let (mut digits, mut free, mut multi) = (!text.is_empty(), text.len() > 100, false);
    for (count, (index, ch)) in text.char_indices().enumerate() {
        if count % crate::data::CHUNK_SIZE == 0 {
            control.check_cancelled()?;
        }
        digits &= ch.is_ascii_digit();
        free |= ch == '\n';
        multi |= matches!(ch, ';' | '|' | ',');
        if ch == '@' && at.is_none() {
            at = Some(index);
        } else if at.is_some() {
            dot_after_at |= ch == '.';
            space_after_at |= ch.is_whitespace();
        }
    }
    control.check_cancelled()?;
    let email = at.is_some_and(|i| i > 0) && dot_after_at && !space_after_at;
    let identifier = digits && (text.len() >= 15 || (text.len() > 1 && text.starts_with('0')));
    let bytes = text.as_bytes();
    let timestamp = bytes.len() >= 10
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes[..4].iter().all(u8::is_ascii_digit);
    Ok((email || identifier || timestamp, free, multi))
}

/// Inspect on a worker thread. Heuristics are advisory and cannot identify all personal data.
pub fn inspect_with_budget(
    dataset: &Dataset,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<SurveyReview> {
    control.check_cancelled()?;
    let mut budget = AllocationCounter {
        bytes: Budget::context_bytes_controlled(dataset, retained, None, extra_bytes, control)?,
    };
    budget.add(size_of::<SurveyReview>())?;
    budget.array::<SurveyColumnReview>(dataset.column_count())?;
    // Every column has at most one entry for each of the ten typed findings.
    budget.array::<SurveyFinding>(dataset.column_count().checked_mul(10).ok_or(
        AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        },
    )?)?;
    budget.array::<usize>(dataset.row_count())?;
    budget.array::<usize>(dataset.row_count())?;
    let mut columns = Vec::with_capacity(dataset.column_count());
    let mut rows = Vec::with_capacity(dataset.row_count());
    let mut scratch = Vec::with_capacity(dataset.row_count());
    for row in 0..dataset.row_count() {
        if row.is_multiple_of(crate::data::CHUNK_SIZE) {
            control.checkpoint(row, None)?;
        }
        rows.push(row);
        scratch.push(0);
    }
    for (index, column) in dataset.columns().iter().enumerate() {
        control.checkpoint(index, Some(dataset.column_count()))?;
        let mut findings = Vec::with_capacity(10);
        let (mut valid, mut missing, mut empty, mut explicit) = (0, 0, 0, 0);
        let mut personal = header_candidate(
            column.name(),
            &[
                "id",
                "identifier",
                "respondent",
                "name",
                "firstname",
                "lastname",
                "email",
                "mail",
                "timestamp",
                "time",
                "date",
                "address",
                "phone",
            ],
            control,
        )?;
        let mut free = header_candidate(
            column.name(),
            &["comment", "comments", "feedback", "freetext", "essay"],
            control,
        )?;
        let (mut multi, mut token) = (false, false);
        for row in 0..dataset.row_count() {
            control.check_cancelled()?;
            let value = raw(column, row)?;
            if column.is_missing_controlled(row, control)? {
                missing += 1;
                if matches!(value, Raw::Missing) {
                    empty += 1;
                } else {
                    explicit += 1;
                }
            } else {
                valid += 1;
            }
            if let Raw::Text(text) = value {
                // Text work is borrowed; long responses never enter findings or scratch storage.
                let (private_value, free_text, multiple) = value_candidates(text, control)?;
                personal |= private_value;
                free |= free_text;
                multi |= multiple;
                let trimmed_text = trimmed(text, control)?;
                if trimmed_text.is_empty() {
                    empty += 1;
                }
                token |= ["n/a", "na", "null", "none", "missing", "-"]
                    .iter()
                    .any(|s| trimmed_text.eq_ignore_ascii_case(s))
                    && !column.is_missing_controlled(row, control)?;
            }
        }
        sort_rows(&mut rows, &mut scratch, control, |a, b| {
            compare(raw(column, a)?, raw(column, b)?, control)
        })?;
        let (mut distinct, mut previous) = (0, None);
        for &row in &rows {
            control.check_cancelled()?;
            if column.is_missing_controlled(row, control)? {
                continue;
            }
            if match previous {
                None => true,
                Some(p) => compare(raw(column, p)?, raw(column, row)?, control)? != Ordering::Equal,
            } {
                distinct = (distinct + 1).min(31);
            }
            previous = Some(row);
        }
        let mut variants = false;
        if column.kind() == ColumnKind::Text {
            sort_rows(&mut rows, &mut scratch, control, |a, b| {
                Ok(match (column.text_value(a)?, column.text_value(b)?) {
                    (Some(a), Some(b)) => compare_iter(
                        normal(trimmed(a, control)?),
                        normal(trimmed(b, control)?),
                        control,
                    )?,
                    (None, None) => Ordering::Equal,
                    (None, _) => Ordering::Less,
                    (_, None) => Ordering::Greater,
                })
            })?;
            for pair in rows.windows(2) {
                control.check_cancelled()?;
                if let (Some(a), Some(b)) =
                    (column.text_value(pair[0])?, column.text_value(pair[1])?)
                {
                    if compare_iter(a.bytes(), b.bytes(), control)? != Ordering::Equal
                        && compare_iter(
                            normal(trimmed(a, control)?),
                            normal(trimmed(b, control)?),
                            control,
                        )? == Ordering::Equal
                    {
                        variants = true;
                    }
                }
            }
        }
        for (present, finding) in [
            (dataset.row_count() == 0, SurveyFinding::Empty),
            (valid == 0 && missing > 0, SurveyFinding::AllMissing),
            (empty > 0, SurveyFinding::EmptyCells),
            (explicit > 0, SurveyFinding::ExplicitMissingRules),
            (token, SurveyFinding::CandidateMissingToken),
            (variants, SurveyFinding::LabelVariants),
            (multi, SurveyFinding::MultipleAnswers),
            (personal, SurveyFinding::PersonalDataCandidate),
            (free, SurveyFinding::FreeTextCandidate),
            (distinct > 30, SurveyFinding::HighCardinality),
        ] {
            if present {
                findings.push(finding);
            }
        }
        let suggested_role = if valid == 0 || personal || free {
            SurveyRole::Exclude
        } else if multi {
            SurveyRole::Unresolved
        } else if column.kind() == ColumnKind::Number
            && column.metadata().measurement == Measurement::Scale
        {
            SurveyRole::NumericMeasure
        } else if distinct > 30
            || (column.kind() == ColumnKind::Number
                && column.metadata().measurement == Measurement::Unknown)
        {
            SurveyRole::Unresolved
        } else {
            SurveyRole::Categories
        };
        columns.push(SurveyColumnReview {
            column: column.id(),
            suggested_role,
            valid,
            missing,
            distinct_values: distinct,
            findings,
        });
    }
    sort_rows(&mut rows, &mut scratch, control, |a, b| {
        for column in dataset.columns() {
            control.check_cancelled()?;
            let order = compare(raw(column, a)?, raw(column, b)?, control)?;
            if order != Ordering::Equal {
                return Ok(order);
            }
        }
        Ok(Ordering::Equal)
    })?;
    let mut duplicate_rows = 0;
    for pair in rows.windows(2) {
        control.check_cancelled()?;
        let mut equal = true;
        for column in dataset.columns() {
            control.check_cancelled()?;
            if compare(raw(column, pair[0])?, raw(column, pair[1])?, control)? != Ordering::Equal {
                equal = false;
                break;
            }
        }
        if equal {
            duplicate_rows += 1;
        }
    }
    control.checkpoint(dataset.column_count(), Some(dataset.column_count()))?;
    Ok(SurveyReview {
        revision: dataset.revision(),
        columns,
        duplicate_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_after_a_completed_column_discards_the_review() {
        let dataset = Dataset::new(vec![
            Column::numeric(ColumnId(1), "A", vec![Some(1.); 100]).unwrap(),
            Column::numeric(ColumnId(2), "B", vec![Some(2.); 100]).unwrap(),
        ])
        .unwrap();
        let control = JobControl::default();
        let cancellation = control.clone();
        crate::operation::CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done == 1 && progress.total == Some(2) {
                    cancellation.cancel();
                }
            }))
        });
        let result = inspect_with_budget(&dataset, &control, &[], 0);
        crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(dataset.revision(), RevisionId(1));
        assert_eq!(dataset.row_count(), 100);
    }
    #[test]
    fn cancellation_during_precharged_index_fill_discards_the_review() {
        let dataset = Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "Choice",
            vec![Some(1.); 8192],
        )
        .unwrap()])
        .unwrap();
        let control = JobControl::default();
        let cancellation = control.clone();
        crate::operation::CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done == 4096 && progress.total.is_none() {
                    cancellation.cancel();
                }
            }))
        });
        let result = inspect_with_budget(&dataset, &control, &[], 0);
        crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(AppError::Cancelled)));
        assert_eq!(dataset.row_count(), 8192);
    }
}

mod generate;
pub use generate::{generate_presented_with_budget, generate_with_budget};
