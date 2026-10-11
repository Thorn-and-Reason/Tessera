//! Bounded choices discovered from the complete nonmissing grouping column.
use super::model::{checked_product, checked_sum};
use crate::{
    data::{Budget, Cell, ColumnId, ColumnKind, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
    search::{NameQuery, SEARCH_SCRATCH_BYTES},
};
use std::mem::size_of;
const CHOICES: usize = 256;
const OUTPUT_BYTES: usize = 1024 * 1024;
const DRAFT_BYTES: usize = 512 * 1024;
#[derive(Debug, Clone, PartialEq)]
pub struct GroupLookupRequest {
    pub column: ColumnId,
    pub role_epoch: u64,
    pub query: String,
    pub selected: [Option<Cell>; 2],
}
#[derive(Debug, Clone, PartialEq)]
pub struct GroupChoice {
    pub value: Cell,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GroupLookupResult {
    pub column: ColumnId,
    pub role_epoch: u64,
    pub query: String,
    pub choices: Vec<GroupChoice>,
    pub selected_present: [bool; 2],
    pub more: bool,
}
fn cell_heap(value: &Cell) -> usize {
    match value {
        Cell::Text(text) => text.capacity(),
        _ => 0,
    }
}
impl GroupLookupRequest {
    pub fn heap_bytes(&self) -> AppResult<usize> {
        checked_sum(
            std::iter::once(self.query.capacity())
                .chain(self.selected.iter().flatten().map(cell_heap)),
        )
    }
}
impl GroupLookupResult {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            size_of::<Self>(),
            self.query.capacity(),
            checked_product(self.choices.capacity(), size_of::<GroupChoice>())?,
        ])?;
        for choice in &self.choices {
            bytes = checked_sum([bytes, cell_heap(&choice.value), choice.label.capacity()])?;
        }
        Ok(bytes)
    }
}
#[derive(Clone, Copy)]
enum Value<'a> {
    Number(f64),
    Text(&'a str),
}
impl Value<'_> {
    fn equals(self, cell: &Cell) -> bool {
        match (self, cell) {
            (Self::Number(a), Cell::Number(b)) => a == *b,
            (Self::Text(a), Cell::Text(b)) => a == b,
            _ => false,
        }
    }
    fn owned(self) -> Cell {
        match self {
            Self::Number(v) => Cell::Number(v),
            Self::Text(v) => Cell::Text(v.to_owned()),
        }
    }
    fn bytes(self) -> usize {
        match self {
            Self::Number(_) => 0,
            Self::Text(v) => v.len(),
        }
    }
}
/// `extra_bytes` includes the live request, retained lookup output and other caller work.
pub fn group_values_with_budget(
    dataset: &Dataset,
    request: &GroupLookupRequest,
    control: &JobControl,
    retained: &[&Dataset],
    extra_bytes: usize,
) -> AppResult<GroupLookupResult> {
    control.check_cancelled()?;
    let request_bytes = request.heap_bytes()?;
    if request_bytes > DRAFT_BYTES {
        return Err(AppError::SizeLimit { limit: DRAFT_BYTES });
    }
    let column = dataset.column(request.column)?;
    for selected in request.selected.iter().flatten() {
        if !matches!((column.kind(),selected), (ColumnKind::Number,Cell::Number(v)) if v.is_finite())
            && !matches!((column.kind(), selected), (ColumnKind::Text, Cell::Text(_)))
        {
            return Err(AppError::invalid(
                "Group search",
                "Selected groups must match the grouping variable's type.",
            ));
        }
    }
    let base = checked_sum([extra_bytes, SEARCH_SCRATCH_BYTES, 384])?;
    let mut output = checked_sum([
        size_of::<GroupLookupResult>(),
        request.query.len(),
        checked_product(CHOICES, size_of::<GroupChoice>())?,
    ])?;
    let check = |output| {
        Budget::dataset_bytes_iter(
            std::iter::once(dataset).chain(retained.iter().copied()),
            checked_sum([base, output])?,
        )
        .map(|_| ())
    };
    check(output)?;
    let matcher = NameQuery::new(&request.query)?;
    let mut result = GroupLookupResult {
        column: request.column,
        role_epoch: request.role_epoch,
        query: request.query.clone(),
        choices: Vec::with_capacity(CHOICES),
        selected_present: [false; 2],
        more: false,
    };
    for row in 0..column.len() {
        if row % 4096 == 0 {
            control.checkpoint(row, Some(column.len()))?;
        }
        if column.is_missing(row)? {
            continue;
        }
        let value = match column.kind() {
            ColumnKind::Text => Value::Text(column.text_value(row)?.expect("nonmissing text")),
            ColumnKind::Number => match column.cell(row)? {
                Cell::Number(v) => Value::Number(v),
                _ => continue,
            },
        };
        for (index, selected) in request.selected.iter().enumerate() {
            if selected.as_ref().is_some_and(|cell| value.equals(cell)) {
                result.selected_present[index] = true;
            }
        }
        if result
            .choices
            .iter()
            .any(|choice| value.equals(&choice.value))
        {
            continue;
        }
        // ponytail: borrowed linear metadata scan; add a precharged label index if large label tables dominate lookup time.
        let mut label = None;
        for (index, entry) in column.metadata().value_labels.iter().enumerate() {
            if index % 4096 == 0 {
                control.check_cancelled()?;
            }
            if value.equals(&entry.value) {
                label = Some(entry.label.as_str());
                break;
            }
        }
        let numeric;
        let raw = match value {
            Value::Text(v) => v,
            Value::Number(v) => {
                numeric = {
                    use std::fmt::Write;
                    let mut text = String::with_capacity(384);
                    write!(&mut text, "{v}").expect("string write");
                    text
                };
                &numeric
            }
        };
        if !matcher.matches_worker(raw, control)?
            && !match label {
                Some(text) => matcher.matches_worker(text, control)?,
                None => false,
            }
        {
            continue;
        }
        let mut counter = CaptionCounter(0);
        caption(&mut counter, value, raw, label).map_err(|_| AppError::SizeLimit {
            limit: OUTPUT_BYTES,
        })?;
        let caption_bytes = counter.0;
        let growth = checked_sum([value.bytes(), caption_bytes])?;
        if growth > OUTPUT_BYTES || value.bytes() > DRAFT_BYTES {
            return Err(AppError::invalid("Group search", "A group value is too large to retain. Shorten the source value before selecting it."));
        }
        if result.choices.len() == CHOICES || checked_sum([output, growth])? > OUTPUT_BYTES {
            result.more = true;
            continue;
        }
        check(checked_sum([output, growth])?)?;
        let mut caption_text = String::with_capacity(caption_bytes);
        caption(&mut caption_text, value, raw, label).expect("string write");
        output = checked_sum([output, growth])?;
        result.choices.push(GroupChoice {
            value: value.owned(),
            label: caption_text,
        });
    }
    control.checkpoint(column.len(), Some(column.len()))?;
    Ok(result)
}

struct CaptionCounter(usize);
impl std::fmt::Write for CaptionCounter {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0 = self.0.checked_add(text.len()).ok_or(std::fmt::Error)?;
        Ok(())
    }
}
fn caption(
    output: &mut impl std::fmt::Write,
    value: Value<'_>,
    raw: &str,
    label: Option<&str>,
) -> std::fmt::Result {
    if let Some(label) = label {
        write!(output, "{label} — ")?;
    }
    match value {
        Value::Text(text) => write!(output, "text {text:?}"),
        Value::Number(_) => write!(output, "number {raw}"),
    }
}
