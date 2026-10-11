//! Cancellable bounded pages from immutable complete frequency output.
use super::super::{
    exploration::ordering::compare_cells,
    model::{checked_product, checked_sum},
    validation::invalid_analysis,
    ComputedFrequencies,
};
use crate::{
    data::{Budget, Cell, ColumnId, ColumnKind, MAX_CELLS},
    error::{AppError, AppResult},
    operation::{JobControl, RunId},
    search::{NameQuery, SEARCH_SCRATCH_BYTES},
};
use std::{cmp::Ordering, fmt::Write, mem::size_of};
const PAGE_ROWS: usize = 256;
const DRAFT_BYTES: usize = 512 * 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrequencySearchRequest {
    pub run: RunId,
    pub column: ColumnId,
    pub query: String,
    /// Offset in matching categories, rather than the unfiltered category list.
    pub offset: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrequencySearchResult {
    pub request: FrequencySearchRequest,
    /// Indexes into the captured complete table, in its stored order.
    pub indices: Vec<usize>,
    pub matched: usize,
    pub next_offset: Option<usize>,
}
impl FrequencySearchRequest {
    pub fn heap_bytes(&self) -> AppResult<usize> {
        checked_sum([self.query.capacity()])
    }
}
impl FrequencySearchResult {
    pub fn owned_bytes(&self) -> AppResult<usize> {
        checked_sum([
            size_of::<Self>(),
            self.request.heap_bytes()?,
            checked_product(self.indices.capacity(), size_of::<usize>())?,
        ])
    }
}
/// Caller charges all live inputs, immutable results and history through `extra_bytes`.
/// Project validates run association before dispatch; this leaf has no project reference.
pub fn frequency_rows_with_budget(
    result: &ComputedFrequencies,
    request: &FrequencySearchRequest,
    control: &JobControl,
    extra_bytes: usize,
) -> AppResult<FrequencySearchResult> {
    control.check_cancelled()?;
    if request.run.0 == 0
        || request.column.0 == 0
        || result.revision.0 == 0
        || result.source_rows > MAX_CELLS
        || request.offset > MAX_CELLS
        || request.query.chars().take(8193).count() > 8192
    {
        return Err(invalid_analysis());
    }
    if request.heap_bytes()? > DRAFT_BYTES {
        return Err(AppError::SizeLimit { limit: DRAFT_BYTES });
    }
    let mut tables = result
        .tables
        .iter()
        .filter(|table| table.column == request.column);
    let table = tables.next().ok_or_else(invalid_analysis)?;
    if tables.next().is_some()
        || table.categories.len() > MAX_CELLS
        || request.offset > table.categories.len()
    {
        return Err(invalid_analysis());
    }
    let mut columns = result
        .columns
        .iter()
        .filter(|info| info.id == request.column);
    let info = columns.next().ok_or_else(invalid_analysis)?;
    if columns.next().is_some() || !result.request.columns.contains(&request.column) {
        return Err(invalid_analysis());
    }
    let page_capacity = table.categories.len().min(PAGE_ROWS);
    // Query clone, KMP construction/boundary ring and returned page are additional live storage.
    Budget::check_datasets(
        &[],
        checked_sum([
            extra_bytes,
            size_of::<FrequencySearchResult>(),
            request.query.len(),
            SEARCH_SCRATCH_BYTES,
            checked_product(page_capacity, size_of::<usize>())?,
        ])?,
    )?;
    let matcher = NameQuery::new(&request.query)?;
    let mut indices = Vec::with_capacity(page_capacity);
    let mut matched = 0;
    let mut previous: Option<&Cell> = None;
    for (index, category) in table.categories.iter().enumerate() {
        if index.is_multiple_of(4096) {
            control.checkpoint(index, Some(table.categories.len()))?;
        }
        if !matches!((&category.value,info.kind),(Cell::Number(v),ColumnKind::Number) if v.is_finite())
            && !matches!(
                (&category.value, info.kind),
                (Cell::Text(_), ColumnKind::Text)
            )
        {
            return Err(invalid_analysis());
        }
        if let Some(previous) = previous {
            if compare_cells(previous, &category.value, control)? != Ordering::Less {
                return Err(invalid_analysis());
            }
        }
        previous = Some(&category.value);
        let label = category
            .value_label_index
            .map(|index| info.value_labels.get(index).ok_or_else(invalid_analysis))
            .transpose()?;
        if let Some(label) = label {
            if compare_cells(&label.value, &category.value, control)? != Ordering::Equal {
                return Err(invalid_analysis());
            }
        }
        let numeric;
        let raw = match &category.value {
            Cell::Text(text) => text.as_str(),
            Cell::Number(value) => {
                numeric = number_text(*value)?;
                numeric.as_str()?
            }
            Cell::Missing => return Err(invalid_analysis()),
        };
        if matcher.matches_worker(raw, control)?
            || label
                .map(|label| matcher.matches_worker(&label.label, control))
                .transpose()?
                .unwrap_or(false)
        {
            if matched >= request.offset && indices.len() < PAGE_ROWS {
                indices.push(index);
            }
            matched += 1;
        }
    }
    let end = checked_sum([request.offset, indices.len()])?;
    control.checkpoint(table.categories.len(), Some(table.categories.len()))?;
    Ok(FrequencySearchResult {
        request: request.clone(),
        indices,
        matched,
        next_offset: (end < matched).then_some(end),
    })
}
// Float64 Display needs at most 327 ASCII bytes, including subnormal decimal notation.
struct NumberText {
    bytes: [u8; 384],
    len: usize,
}
impl Write for NumberText {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self
            .len
            .checked_add(text.len())
            .filter(|end| *end <= self.bytes.len())
            .ok_or(std::fmt::Error)?;
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}
impl NumberText {
    fn as_str(&self) -> AppResult<&str> {
        std::str::from_utf8(&self.bytes[..self.len]).map_err(|_| invalid_analysis())
    }
}
fn number_text(value: f64) -> AppResult<NumberText> {
    let mut result = NumberText {
        bytes: [0; 384],
        len: 0,
    };
    write!(&mut result, "{value}").map_err(|_| invalid_analysis())?;
    Ok(result)
}
