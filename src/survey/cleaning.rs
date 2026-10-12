//! Explicit, reversible survey preparation with a bounded preview.
use crate::{
    data::{AllocationCounter, Budget, Cell, Column, ColumnId, ColumnKind, Dataset, RevisionId},
    error::{AppError, AppResult},
    operation::JobControl,
    preparation::{RecodeRule, Step, MAX_RECODE_RULES},
};
use serde::{Deserialize, Serialize};
use std::mem::size_of;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleaningSpec {
    pub trim_whitespace: bool,
    pub rules: Vec<RecodeRule>,
}
impl CleaningSpec {
    pub fn validate(&self) -> AppResult<()> {
        validate_spec(self.trim_whitespace, &self.rules)
    }
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.add(size_of::<Self>())?;
        n.array::<RecodeRule>(self.rules.capacity())?;
        for r in &self.rules {
            for cell in [&r.from, &r.to] {
                if let Cell::Text(s) = cell {
                    n.add(s.capacity())?;
                }
            }
        }
        Ok(n.bytes)
    }
    pub fn budget(&self, control: &JobControl) -> AppResult<usize> {
        control.check_cancelled()?;
        self.validate()?;
        self.owned_bytes()
    }
}
pub(crate) fn validate_spec(trim: bool, rules: &[RecodeRule]) -> AppResult<()> {
    if rules.len() > MAX_RECODE_RULES || (!trim && rules.is_empty()) {
        return Err(invalid("Choose whitespace trimming or at least one exact replacement; at most 256 replacements are supported."));
    }
    for (i, r) in rules.iter().enumerate() {
        for cell in [&r.from, &r.to] {
            match cell {
                Cell::Number(n) if !n.is_finite() => {
                    return Err(invalid("Replacement numbers must be finite."))
                }
                Cell::Text(s) if s.len() > 4096 => {
                    return Err(invalid("Replacement text is limited to 4096 bytes."))
                }
                _ => {}
            }
        }
        if r.from == Cell::Missing
            || rules[..i].iter().any(|p| p.from == r.from)
            || (trim && matches!(&r.from,Cell::Text(s) if s.trim()!=s))
        {
            return Err(invalid("Replacement sources must be unique present values, without surrounding whitespace when trimming."));
        }
    }
    Ok(())
}
fn invalid(message: &str) -> AppError {
    AppError::invalid("cleaning", message)
}
#[derive(Debug, Clone)]
pub struct CleaningRequest {
    pub revision: RevisionId,
    pub source: ColumnId,
    pub name: String,
    pub spec: CleaningSpec,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CleaningExample {
    pub row: usize,
    pub before: Cell,
    pub after: Cell,
}
#[derive(Debug, Clone)]
pub struct CleaningPreview {
    pub revision: RevisionId,
    pub source: ColumnId,
    pub changed: usize,
    pub examples: Vec<CleaningExample>,
}
impl CleaningPreview {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut n = AllocationCounter::default();
        n.add(size_of::<Self>())?;
        n.array::<CleaningExample>(self.examples.capacity())?;
        for example in &self.examples {
            for cell in [&example.before, &example.after] {
                if let Cell::Text(s) = cell {
                    n.add(s.capacity())?;
                }
            }
        }
        Ok(n.bytes)
    }
}
/// Keeps derived names within preparation's byte limit without changing source headers.
pub fn reviewed_name(dataset: &Dataset, source: ColumnId) -> AppResult<String> {
    reviewed_name_with_control(dataset, source, &JobControl::default())
}
pub fn reviewed_name_with_control(
    dataset: &Dataset,
    source: ColumnId,
    control: &JobControl,
) -> AppResult<String> {
    control.check_cancelled()?;
    let column = dataset.column(source)?;
    let mut end = column.name().len().min(990);
    while !column.name().is_char_boundary(end) {
        end -= 1;
    }
    let base = format!("{} (reviewed)", &column.name()[..end]);
    let mut name = base.clone();
    let mut suffix = 2usize;
    loop {
        let mut found = false;
        for column in dataset.columns() {
            control.check_cancelled()?;
            if column.name() == name {
                found = true;
                break;
            }
        }
        if !found {
            break;
        }
        name = format!("{base} {suffix}");
        suffix = suffix
            .checked_add(1)
            .ok_or(crate::error::AppError::IdentityExhausted)?;
    }
    Ok(name)
}
impl CleaningRequest {
    pub fn step(&self) -> AppResult<Step> {
        self.spec.validate()?;
        if self.source.0 == 0 || self.name.len() > 1024 {
            return Err(invalid("Choose a source and a name of at most 1024 bytes."));
        }
        crate::data::ColumnMetadata::validate_name(&self.name)?;
        Ok(Step::CleanText {
            source: self.source,
            name: self.name.clone(),
            trim_whitespace: self.spec.trim_whitespace,
            rules: self.spec.rules.clone(),
        })
    }
}
pub fn preview_with_budget(
    dataset: &Dataset,
    request: &CleaningRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<CleaningPreview> {
    control.check_cancelled()?;
    request.spec.validate()?;
    if request.name.len() > 1024 || dataset.columns().iter().any(|c| c.name() == request.name) {
        return Err(invalid(
            "Choose a unique new variable name of at most 1024 bytes.",
        ));
    }
    crate::data::ColumnMetadata::validate_name(&request.name)?;
    if request.revision != dataset.revision() {
        return Err(invalid("The data changed. Review cleaning again."));
    }
    let col = dataset.column(request.source)?;
    validate_column(col, request.spec.trim_whitespace, &request.spec.rules)?;
    let mut bytes = AllocationCounter::default();
    bytes.add(extra)?;
    bytes.add(request.spec.budget(control)?)?;
    bytes.add(request.name.capacity())?;
    bytes.add(size_of::<CleaningPreview>())?;
    bytes.array::<CleaningExample>(8)?;
    bytes.bytes = Budget::context_bytes_controlled(dataset, retained, None, bytes.bytes, control)?;
    let mut examples = Vec::with_capacity(8);
    let mut changed = 0;
    for row in 0..col.len() {
        control.checkpoint(row, Some(col.len()))?;
        let before = raw(col, row)?;
        let after = transformed(
            col,
            row,
            request.spec.trim_whitespace,
            &request.spec.rules,
            control,
        )?;
        if before != after {
            changed += 1;
            if examples.len() < 8 {
                for v in [before, after] {
                    if let Value::Text(s) = v {
                        bytes.add(s.len())?;
                    }
                }
                examples.push(CleaningExample {
                    row,
                    before: owned(before, control)?,
                    after: owned(after, control)?,
                });
            }
        }
    }
    control.check_cancelled()?;
    Ok(CleaningPreview {
        revision: dataset.revision(),
        source: request.source,
        changed,
        examples,
    })
}
pub(crate) fn validate_column(
    col: &Column,
    trim_whitespace: bool,
    rules: &[RecodeRule],
) -> AppResult<()> {
    if (trim_whitespace && col.kind() != ColumnKind::Text)
        || rules.iter().any(|r| {
            !matches!(
                (value(&r.from), col.kind()),
                (Value::Text(_), ColumnKind::Text) | (Value::Number(_), ColumnKind::Number)
            ) || !matches!(
                (value(&r.to), col.kind()),
                (Value::Missing, _)
                    | (Value::Text(_), ColumnKind::Text)
                    | (Value::Number(_), ColumnKind::Number)
            )
        })
    {
        return Err(invalid(
            "Replacement values must match the source type; trimming requires text.",
        ));
    }
    Ok(())
}
#[derive(Clone, Copy, PartialEq)]
enum Value<'a> {
    Missing,
    Number(f64),
    Text(&'a str),
}
fn raw(col: &Column, row: usize) -> AppResult<Value<'_>> {
    Ok(match col.kind() {
        ColumnKind::Text => col
            .text_value(row)?
            .map(Value::Text)
            .unwrap_or(Value::Missing),
        ColumnKind::Number => match col.cell(row)? {
            Cell::Number(n) => Value::Number(n),
            _ => Value::Missing,
        },
    })
}
fn value(cell: &Cell) -> Value<'_> {
    match cell {
        Cell::Missing => Value::Missing,
        Cell::Number(n) => Value::Number(*n),
        Cell::Text(s) => Value::Text(s),
    }
}
fn trim<'a>(s: &'a str, c: &JobControl) -> AppResult<&'a str> {
    let mut start = s.len();
    for (count, (i, ch)) in s.char_indices().enumerate() {
        if count % 1024 == 0 {
            c.check_cancelled()?;
        }
        if !ch.is_whitespace() {
            start = i;
            break;
        }
    }
    let mut end = start;
    for (count, (i, ch)) in s.char_indices().rev().enumerate() {
        if count % 1024 == 0 {
            c.check_cancelled()?;
        }
        if !ch.is_whitespace() {
            end = i + ch.len_utf8();
            break;
        }
    }
    Ok(&s[start..end])
}
fn transformed<'a>(
    col: &'a Column,
    row: usize,
    trim_whitespace: bool,
    rules: &'a [RecodeRule],
    c: &JobControl,
) -> AppResult<Value<'a>> {
    if col.is_missing_controlled(row, c)? {
        return Ok(Value::Missing);
    }
    let mut v = raw(col, row)?;
    if trim_whitespace {
        if let Value::Text(s) = v {
            v = Value::Text(trim(s, c)?);
        }
    }
    for r in rules {
        c.check_cancelled()?;
        if v == value(&r.from) {
            return Ok(value(&r.to));
        }
    }
    Ok(v)
}
fn owned(v: Value<'_>, c: &JobControl) -> AppResult<Cell> {
    Ok(match v {
        Value::Missing => Cell::Missing,
        Value::Number(n) => Cell::Number(n),
        Value::Text(s) => {
            let mut text = String::with_capacity(s.len());
            let mut start = 0;
            while start < s.len() {
                c.check_cancelled()?;
                let mut end = (start + 4096).min(s.len());
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                text.push_str(&s[start..end]);
                start = end;
            }
            Cell::Text(text)
        }
    })
}
pub(crate) fn clean_cell(
    col: &Column,
    row: usize,
    trim: bool,
    rules: &[RecodeRule],
    c: &JobControl,
) -> AppResult<Cell> {
    owned(transformed(col, row, trim, rules, c)?, c)
}
pub(crate) fn clean_text_len(
    col: &Column,
    row: usize,
    trim: bool,
    rules: &[RecodeRule],
    c: &JobControl,
) -> AppResult<usize> {
    Ok(match transformed(col, row, trim, rules, c)? {
        Value::Text(s) => s.len(),
        _ => 0,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_mid_preview_and_apply_preserves_original() {
        let data = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Answer",
            vec![Some(" a ".into()); 100],
        )
        .unwrap()])
        .unwrap();
        let request = CleaningRequest {
            revision: data.revision(),
            source: ColumnId(1),
            name: "Clean".into(),
            spec: CleaningSpec {
                trim_whitespace: true,
                rules: vec![],
            },
        };
        for apply in [false, true] {
            let control = JobControl::default();
            let canceller = control.clone();
            crate::operation::CHECKPOINT_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |p| {
                    if p.done == 10 {
                        canceller.cancel();
                    }
                }))
            });
            let cancelled = if apply {
                let p = crate::project::Project::new("Survey", data.clone()).unwrap();
                crate::preparation::prepare_with_budget(
                    &p,
                    &request.step().unwrap(),
                    &control,
                    &[],
                    0,
                )
                .map(|_| ())
            } else {
                preview_with_budget(&data, &request, &control, &[], 0).map(|_| ())
            };
            crate::operation::CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
            assert!(matches!(cancelled, Err(AppError::Cancelled)));
            assert_eq!(data.column_count(), 1);
        }
    }
    #[test]
    fn long_unicode_strings_trim_and_copy_without_corruption() {
        let text = format!(
            "{}{}{}",
            "\u{2003}".repeat(10000),
            "ê".repeat(10000),
            "\u{2003}".repeat(10000)
        );
        let c = JobControl::default();
        let trimmed = trim(&text, &c).unwrap();
        assert_eq!(trimmed, "ê".repeat(10000));
        assert_eq!(
            owned(Value::Text(trimmed), &c).unwrap(),
            Cell::Text("ê".repeat(10000))
        );
        assert_eq!(trim("\u{2003}\u{2003}", &c).unwrap(), "");
    }
}
