mod budget;
mod column;
mod edit;
mod paste;
use crate::error::{AppError, AppResult};
pub(crate) use budget::AllocationCounter;
pub use budget::{Budget, MAX_OWNED_BYTES};
pub use column::{
    parse_number, Cell, Column, ColumnKind, ColumnMetadata, Measurement, MissingRules, ValueLabel,
};
#[cfg(test)]
pub(crate) use edit::tests::measured as measure_test_allocations;
use std::collections::HashSet;

pub const MAX_CELLS: usize = 2_000_000;
pub const CHUNK_SIZE: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColumnId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RevisionId(pub u64);
macro_rules! checked_next {
    ($id:ty) => {
        impl $id {
            pub fn next(self) -> AppResult<Self> {
                if self.0 == 0 {
                    return Err(AppError::IdentityExhausted);
                }
                self.0
                    .checked_add(1)
                    .map(Self)
                    .ok_or(AppError::IdentityExhausted)
            }
        }
    };
}
checked_next!(ColumnId);
checked_next!(RevisionId);

#[derive(Debug, Clone)]
pub struct Dataset {
    columns: Vec<Column>,
    rows: usize,
    revision: RevisionId,
}
impl Dataset {
    pub fn new(columns: Vec<Column>) -> AppResult<Self> {
        Self::from_revision(columns, RevisionId(1))
    }
    pub fn from_revision(columns: Vec<Column>, revision: RevisionId) -> AppResult<Self> {
        let rows = columns.first().map_or(0, Column::len);
        let dataset = Self {
            columns,
            rows,
            revision,
        };
        dataset.validate()?;
        Ok(dataset)
    }
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }
    pub fn row_count(&self) -> usize {
        self.rows
    }
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
    pub fn revision(&self) -> RevisionId {
        self.revision
    }
    pub fn column(&self, id: ColumnId) -> AppResult<&Column> {
        self.columns
            .iter()
            .find(|c| c.id() == id)
            .ok_or_else(|| AppError::invalid("column", "Column no longer exists."))
    }
    pub fn cell(&self, id: ColumnId, row: usize) -> AppResult<Cell> {
        self.column(id)?.cell(row)
    }
    pub fn next_column_id(&self) -> AppResult<ColumnId> {
        self.columns
            .iter()
            .map(|c| c.id())
            .max()
            .map_or(Ok(ColumnId(1)), ColumnId::next)
    }
    pub fn validate(&self) -> AppResult<()> {
        if self.revision.0 == 0 {
            return Err(AppError::invalid("revision", "Revision must be positive."));
        }
        check_dimensions(self.rows, self.columns.len())?;
        Budget::check_datasets(&[self], 0)?;
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for column in &self.columns {
            column.validate()?;
            if column.len() != self.rows {
                return Err(AppError::invalid("column", "Column lengths must match."));
            }
            if !ids.insert(column.id()) || !names.insert(column.name()) {
                return Err(AppError::invalid(
                    "column",
                    "Column IDs and names must be unique.",
                ));
            }
        }
        Ok(())
    }
}
fn check_dimensions(rows: usize, columns: usize) -> AppResult<()> {
    if columns > MAX_CELLS
        || rows
            .checked_mul(columns)
            .is_none_or(|cells| cells > MAX_CELLS)
    {
        return Err(AppError::SizeLimit { limit: MAX_CELLS });
    }
    Ok(())
}
