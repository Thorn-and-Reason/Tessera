use super::column::Chunks;
use super::*;
use crate::operation::JobControl;
use std::{mem::size_of, sync::Arc};

impl Dataset {
    pub fn edit_cell(&self, id: ColumnId, row: usize, value: Cell) -> AppResult<Self> {
        self.edit_cell_with_budget(id, row, value, &[], 0)
    }
    pub fn edit_cell_with_budget(
        &self,
        id: ColumnId,
        row: usize,
        value: Cell,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        let index = self
            .columns
            .iter()
            .position(|c| c.id() == id)
            .ok_or_else(|| AppError::invalid("column", "Column no longer exists."))?;
        let column = &self.columns[index];
        if row >= self.rows {
            return Err(AppError::invalid("row", "Row no longer exists."));
        }
        let compatible = matches!((column.kind(), &value), (ColumnKind::Number, Cell::Number(v)) if v.is_finite())
            || matches!(
                (column.kind(), &value),
                (ColumnKind::Text, Cell::Text(_)) | (_, Cell::Missing)
            );
        if !compatible {
            return Err(AppError::invalid(
                format!("column {}, row {}", id.0, row + 1),
                "Enter a finite value of the column type.",
            ));
        }
        let revision = self.revision.next()?;
        let chunk_index = row / CHUNK_SIZE;
        let offset = row % CHUNK_SIZE;
        let extra = self
            .copy_overhead()?
            .checked_add(extra_bytes)
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?
            .checked_add(chunk_copy_bytes(column, chunk_index)?)
            .and_then(|n| n.checked_add(chunk_buffer_bytes(column, chunk_index)))
            .and_then(|n| {
                n.checked_add(match &value {
                    Cell::Text(s) => s.capacity(),
                    _ => 0,
                })
            })
            .ok_or(AppError::SizeLimit {
                limit: MAX_OWNED_BYTES,
            })?;
        Budget::context_bytes(self, retained, None, extra)?;
        let chunks = match (&column.chunks, value) {
            (Chunks::Number(old), value) => {
                let mut new = old.clone();
                let mut chunk = old[chunk_index].to_vec();
                chunk[offset] = match value {
                    Cell::Number(v) => Some(v),
                    _ => None,
                };
                new[chunk_index] = Arc::from(chunk);
                Chunks::Number(new)
            }
            (Chunks::Text(old), value) => {
                let mut new = old.clone();
                let mut chunk = old[chunk_index].to_vec();
                chunk[offset] = match value {
                    Cell::Text(v) => Some(v),
                    _ => None,
                };
                new[chunk_index] = Arc::from(chunk);
                Chunks::Text(new)
            }
        };
        let mut columns: Vec<_> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| {
                if i == index {
                    Column {
                        id: column.id,
                        metadata: Arc::clone(&column.metadata),
                        chunks: match column.kind() {
                            ColumnKind::Number => Chunks::Number(vec![]),
                            ColumnKind::Text => Chunks::Text(vec![]),
                        },
                        len: column.len,
                    }
                } else {
                    column.clone()
                }
            })
            .collect();
        columns[index].chunks = chunks;
        self.finish_with_budget(columns, self.rows, revision, retained, extra_bytes)
    }
    pub fn append_row(&self) -> AppResult<Self> {
        self.append_row_with_budget(&[], 0)
    }
    pub fn append_row_with_budget(
        &self,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        if self.columns.is_empty() {
            return Err(AppError::invalid(
                "dataset",
                "Add a column before adding rows.",
            ));
        }
        let rows = self
            .rows
            .checked_add(1)
            .ok_or(AppError::SizeLimit { limit: MAX_CELLS })?;
        check_dimensions(rows, self.columns.len())?;
        let revision = self.revision.next()?;
        let mut counter = super::budget::AllocationCounter::default();
        counter.add(self.copy_overhead()?)?;
        counter.add(extra_bytes)?;
        for column in &self.columns {
            if !self.rows.is_multiple_of(CHUNK_SIZE) {
                counter.add(chunk_copy_bytes(column, self.rows / CHUNK_SIZE)?)?;
            }
            let chunk_len = if self.rows.is_multiple_of(CHUNK_SIZE) {
                1
            } else {
                self.rows % CHUNK_SIZE + 1
            };
            counter.add(
                chunk_len
                    * match column.kind() {
                        ColumnKind::Number => size_of::<Option<f64>>(),
                        ColumnKind::Text => size_of::<Option<String>>(),
                    },
            )?;
            counter.add(
                match column.kind() {
                    ColumnKind::Number => size_of::<Option<f64>>(),
                    ColumnKind::Text => size_of::<Option<String>>(),
                } + 4 * size_of::<usize>(),
            )?;
        }
        Budget::context_bytes(self, retained, None, counter.bytes)?;
        let columns = self
            .columns
            .iter()
            .map(|column| {
                let chunks = match &column.chunks {
                    Chunks::Number(old) => Chunks::Number(append_missing(old)),
                    Chunks::Text(old) => Chunks::Text(append_missing(old)),
                };
                Column {
                    id: column.id,
                    metadata: Arc::clone(&column.metadata),
                    chunks,
                    len: rows,
                }
            })
            .collect();
        self.finish_with_budget(columns, rows, revision, retained, extra_bytes)
    }
    pub fn append_empty_column_with_budget(
        &self,
        name: &str,
        kind: ColumnKind,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        self.append_empty_column_with_id_with_budget(
            self.next_column_id()?,
            name,
            kind,
            retained,
            extra_bytes,
        )
    }
    pub fn append_empty_column_with_id_with_budget(
        &self,
        id: ColumnId,
        name: &str,
        kind: ColumnKind,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        check_dimensions(self.rows, self.columns.len() + 1)?;
        if id.0 == 0 || self.columns.iter().any(|column| column.id() == id) {
            return Err(AppError::invalid(
                "column",
                "New column identity must be positive and unused.",
            ));
        }
        self.revision.next()?;
        ColumnMetadata::validate_name(name)?;
        if self.columns.iter().any(|column| column.name() == name) {
            return Err(AppError::invalid("name", "Column names must be unique."));
        }
        let mut counter = AllocationCounter::default();
        counter.add(extra_bytes)?;
        counter.add(self.copy_overhead()?)?;
        counter.array::<Column>(1)?;
        counter.add(size_of::<Column>() + size_of::<ColumnMetadata>() + 2 * size_of::<usize>())?;
        counter.add(name.len())?;
        counter.array::<usize>(self.rows.div_ceil(CHUNK_SIZE) * 4)?;
        match kind {
            ColumnKind::Number => {
                counter.array::<Option<f64>>(self.rows)?;
                counter.array::<Option<f64>>(self.rows)?;
            }
            ColumnKind::Text => {
                counter.array::<Option<String>>(self.rows)?;
                counter.array::<Option<String>>(self.rows)?;
                counter.array::<Option<String>>(self.rows.min(CHUNK_SIZE))?;
            }
        }
        Budget::context_bytes(self, retained, None, counter.bytes)?;
        let column = match kind {
            ColumnKind::Number => Column::numeric(id, name, vec![None; self.rows])?,
            ColumnKind::Text => Column::text(id, name, vec![None; self.rows])?,
        };
        self.append_column_with_budget(column, retained, extra_bytes)
    }
    pub fn append_column(&self, column: Column) -> AppResult<Self> {
        self.append_column_with_budget(column, &[], 0)
    }
    pub fn append_column_with_budget(
        &self,
        column: Column,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        let rows = if self.columns.is_empty() {
            column.len()
        } else {
            self.rows
        };
        check_dimensions(rows, self.columns.len() + 1)?;
        if column.len() != rows
            || self
                .columns
                .iter()
                .any(|c| c.id() == column.id() || c.name() == column.name())
        {
            return Err(AppError::invalid(
                "column",
                "New column must have a unique ID/name and matching length.",
            ));
        }
        let revision = self.revision.next()?;
        let mut counter = AllocationCounter::default();
        counter.add(extra_bytes)?;
        counter.add(self.copy_overhead()?)?;
        counter.add(size_of::<Column>())?;
        counter.add(column.metadata().validation_work()?)?;
        Budget::context_bytes(self, retained, Some(&column), counter.bytes)?;
        let mut columns = Vec::with_capacity(self.columns.len() + 1);
        columns.extend(self.columns.iter().cloned());
        columns.push(column);
        self.finish_with_budget(columns, rows, revision, retained, extra_bytes)
    }
    pub fn replace_column(&self, column: Column) -> AppResult<Self> {
        self.replace_column_with_budget(column, &[], 0)
    }
    pub fn replace_column_with_budget(
        &self,
        column: Column,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        let index = self
            .columns
            .iter()
            .position(|c| c.id() == column.id())
            .ok_or_else(|| AppError::invalid("column", "Column no longer exists."))?;
        if column.len() != self.rows
            || self
                .columns
                .iter()
                .enumerate()
                .any(|(i, c)| i != index && c.name() == column.name())
        {
            return Err(AppError::invalid(
                "column",
                "Replacement must have a unique name and matching length.",
            ));
        }
        let revision = self.revision.next()?;
        let mut counter = AllocationCounter::default();
        counter.add(extra_bytes)?;
        counter.add(self.copy_overhead()?)?;
        counter.add(column.metadata().validation_work()?)?;
        Budget::context_bytes(self, retained, Some(&column), counter.bytes)?;
        let mut replacement = Some(column);
        let columns = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, original)| {
                if i == index {
                    replacement.take().expect("validated replacement index")
                } else {
                    original.clone()
                }
            })
            .collect();
        self.finish_with_budget(columns, self.rows, revision, retained, extra_bytes)
    }
    pub fn with_column_metadata_with_budget(
        &self,
        id: ColumnId,
        metadata: ColumnMetadata,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        let column = self.column(id)?;
        let mut counter = AllocationCounter::default();
        counter.add(extra_bytes)?;
        counter.add(self.copy_overhead()?)?;
        counter.add(metadata.heap_bytes()?)?;
        counter.add(metadata.validation_work()?)?;
        counter.add(size_of::<ColumnMetadata>() + size_of::<Column>() + 2 * size_of::<usize>())?;
        counter.array::<usize>(column.len().div_ceil(CHUNK_SIZE) * 2)?;
        Budget::context_bytes(self, retained, None, counter.bytes)?;
        self.replace_column_with_budget(column.with_metadata(metadata)?, retained, extra_bytes)
    }
    pub fn convert_column_with_budget(
        &self,
        id: ColumnId,
        kind: ColumnKind,
        control: &JobControl,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        let column = self.column(id)?;
        control.checkpoint(0, Some(column.len().saturating_mul(3)))?;
        let mut counter = AllocationCounter::default();
        counter.add(extra_bytes)?;
        counter.add(self.copy_overhead()?)?;
        let baseline = Budget::context_bytes(self, retained, None, counter.bytes)?;
        if kind == column.kind() {
            return Ok(self.clone());
        }
        let revision = self.revision.next()?;
        let converted = column.convert_controlled(kind, control, baseline)?;
        control.checkpoint(
            column.len().saturating_mul(3),
            Some(column.len().saturating_mul(3)),
        )?;
        let mut replacement = Some(converted);
        let columns = self
            .columns
            .iter()
            .map(|column| {
                if column.id() == id {
                    replacement.take().expect("validated conversion column")
                } else {
                    column.clone()
                }
            })
            .collect();
        let result =
            self.finish_with_budget(columns, self.rows, revision, retained, extra_bytes)?;
        control.checkpoint(
            column.len().saturating_mul(3),
            Some(column.len().saturating_mul(3)),
        )?;
        Ok(result)
    }
    pub(super) fn copy_overhead(&self) -> AppResult<usize> {
        let mut counter = super::budget::AllocationCounter::default();
        counter.add(size_of::<Dataset>())?;
        counter.array::<Column>(self.columns.len())?;
        counter.array::<u64>(self.columns.len().saturating_mul(8))?;
        if !self.columns.is_empty() {
            counter.add(64)?;
        }
        counter.add(self.columns.iter().try_fold(0, |peak, column| {
            column
                .metadata()
                .validation_work()
                .map(|bytes| peak.max(bytes))
        })?)?;
        for column in &self.columns {
            let count = match &column.chunks {
                Chunks::Number(c) => c.len(),
                Chunks::Text(c) => c.len(),
            };
            counter.array::<Arc<[Option<f64>]>>(count)?;
        }
        Ok(counter.bytes)
    }
    pub(super) fn finish_with_budget(
        &self,
        columns: Vec<Column>,
        rows: usize,
        revision: RevisionId,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        let next = Self {
            columns,
            rows,
            revision,
        };
        Budget::replacement_bytes(self, &next, retained, extra_bytes)?;
        next.validate()?;
        Ok(next)
    }
}
fn append_missing<T: Clone>(old: &[Arc<[Option<T>]>]) -> Vec<Arc<[Option<T>]>> {
    let last = old.last().filter(|last| last.len() < CHUNK_SIZE);
    let mut new = Vec::with_capacity(old.len() + usize::from(last.is_none()));
    new.extend(old.iter().cloned());
    if let Some(last) = last {
        let mut chunk = Vec::with_capacity(last.len() + 1);
        chunk.extend_from_slice(last);
        chunk.push(None);
        let index = new.len() - 1;
        new[index] = Arc::from(chunk);
    } else {
        new.push(Arc::from(vec![None]));
    }
    new
}
pub(super) fn chunk_buffer_bytes(column: &Column, index: usize) -> usize {
    match &column.chunks {
        Chunks::Number(chunks) => chunks[index].len() * size_of::<Option<f64>>(),
        Chunks::Text(chunks) => chunks[index].len() * size_of::<Option<String>>(),
    }
}
pub(super) fn chunk_copy_bytes(column: &Column, index: usize) -> AppResult<usize> {
    let mut counter = super::budget::AllocationCounter::default();
    counter.add(2 * size_of::<usize>())?;
    match &column.chunks {
        Chunks::Number(chunks) => counter.array::<Option<f64>>(chunks[index].len())?,
        Chunks::Text(chunks) => {
            counter.array::<Option<String>>(chunks[index].len())?;
            for value in chunks[index].iter().flatten() {
                counter.add(value.len())?;
            }
        }
    }
    Ok(counter.bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn edit_shares_untouched_chunks_and_metadata() {
        let first = Column::numeric(ColumnId(1), "A", vec![Some(1.0); CHUNK_SIZE + 1]).unwrap();
        let second =
            Column::text(ColumnId(2), "B", vec![Some("text".into()); CHUNK_SIZE + 1]).unwrap();
        let before = Dataset::new(vec![first, second]).unwrap();
        let after = before.edit_cell(ColumnId(1), 0, Cell::Number(2.0)).unwrap();
        let Chunks::Number(old) = &before.columns[0].chunks else {
            panic!()
        };
        let Chunks::Number(new) = &after.columns[0].chunks else {
            panic!()
        };
        assert!(!Arc::ptr_eq(&old[0], &new[0]));
        assert!(Arc::ptr_eq(&old[1], &new[1]));
        let Chunks::Text(old) = &before.columns[1].chunks else {
            panic!()
        };
        let Chunks::Text(new) = &after.columns[1].chunks else {
            panic!()
        };
        assert!(Arc::ptr_eq(&old[0], &new[0]));
        assert!(Arc::ptr_eq(
            &before.columns[0].metadata,
            &after.columns[0].metadata
        ));
        let shared_bytes = Budget::dataset_bytes(&[&before, &after], 0).unwrap();
        assert!(
            shared_bytes
                < Budget::dataset_bytes(&[&before], 0).unwrap()
                    + Budget::dataset_bytes(&[&after], 0).unwrap()
        );
        let appended = before.append_row().unwrap();
        let Chunks::Number(new) = &appended.columns[0].chunks else {
            panic!()
        };
        let Chunks::Number(old) = &before.columns[0].chunks else {
            panic!()
        };
        assert!(Arc::ptr_eq(&old[0], &new[0]));
        let metadata = ColumnMetadata {
            display_decimals: 7,
            ..before.columns[0].metadata().clone()
        };
        let changed = before.columns[0].with_metadata(metadata).unwrap();
        let Chunks::Number(new) = &changed.chunks else {
            panic!()
        };
        assert!(Arc::ptr_eq(&old[0], &new[0]));
    }
    struct TrackingAllocator;
    std::thread_local! {
        static TRACK_ALLOCATIONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static MAX_ALLOCATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    #[global_allocator]
    static ALLOCATOR: TrackingAllocator = TrackingAllocator;
    // Tests observe only allocation sizes on their own thread; System owns the memory.
    unsafe impl std::alloc::GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            record_allocation(layout.size());
            unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
            unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
            record_allocation(size);
            unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, ptr, layout, size) }
        }
    }
    fn record_allocation(bytes: usize) {
        let _ = TRACK_ALLOCATIONS.try_with(|active| {
            if active.get() {
                let _ = MAX_ALLOCATION.try_with(|size| size.set(size.get().max(bytes)));
            }
        });
    }
    pub(crate) fn measured<T>(action: impl FnOnce() -> T) -> (T, usize) {
        MAX_ALLOCATION.with(|size| size.set(0));
        TRACK_ALLOCATIONS.with(|active| active.set(true));
        let result = action();
        TRACK_ALLOCATIONS.with(|active| active.set(false));
        (result, MAX_ALLOCATION.with(std::cell::Cell::get))
    }
    #[test]
    fn near_ceiling_append_refuses_before_column_vector_growth() {
        let columns = (1..=9)
            .map(|id| Column::numeric(ColumnId(id), &format!("C{id}"), vec![]).unwrap())
            .collect();
        let before = Dataset::new(columns).unwrap();
        let added = Column::numeric(ColumnId(10), "C10", vec![]).unwrap();
        let old_estimate = Budget::context_bytes(
            &before,
            &[],
            Some(&added),
            before.copy_overhead().unwrap() + size_of::<Column>(),
        )
        .unwrap();
        let description = "d".repeat(MAX_OWNED_BYTES - old_estimate);
        let mut columns = before.columns.clone();
        columns[0] = columns[0]
            .with_metadata(ColumnMetadata {
                description,
                ..columns[0].metadata().clone()
            })
            .unwrap();
        let before = Dataset::new(columns).unwrap();
        let (result, largest_allocation) = measured(|| before.append_column(added));
        assert!(
            largest_allocation <= 10 * size_of::<Column>(),
            "allocated {largest_allocation} bytes beyond planned capacity"
        );
        if let Ok(after) = result {
            Budget::check_datasets(&[&before, &after], 0).unwrap();
        }
        let mut columns = before.columns.clone();
        let old = columns[0].metadata();
        let mut description = String::with_capacity(old.description.len() + 1);
        description.push_str(&old.description);
        description.push('d');
        let metadata = ColumnMetadata {
            name: old.name.clone(),
            description,
            measurement: old.measurement,
            missing_rules: old.missing_rules.clone(),
            value_labels: old.value_labels.clone(),
            display_decimals: old.display_decimals,
        };
        columns[0].metadata = Arc::new(metadata);
        let full = Dataset::new(columns).unwrap();
        let added = Column::numeric(ColumnId(10), "C10", vec![]).unwrap();
        let (refused, largest_allocation) = measured(|| full.append_column(added));
        assert!(matches!(refused, Err(AppError::SizeLimit { .. })));
        assert!(
            largest_allocation < 10 * size_of::<Column>(),
            "allocated {largest_allocation} bytes before refusing growth"
        );
        assert_eq!(before.column_count(), 9);
        assert_eq!(before.revision(), RevisionId(1));
    }
    #[test]
    fn appending_column_and_chunk_references_uses_exact_target_capacity() {
        let before = Dataset::new(
            (1..=9)
                .map(|id| Column::numeric(ColumnId(id), &format!("C{id}"), vec![]).unwrap())
                .collect(),
        )
        .unwrap();
        let after = before
            .append_column(Column::numeric(ColumnId(10), "C10", vec![]).unwrap())
            .unwrap();
        assert_eq!(after.columns.capacity(), 10);
        let column = Column::numeric(ColumnId(1), "N", vec![None; CHUNK_SIZE * 9]).unwrap();
        let before = Dataset::new(vec![column]).unwrap();
        let after = before.append_row().unwrap();
        let Chunks::Number(chunks) = &after.columns[0].chunks else {
            panic!()
        };
        assert_eq!(chunks.capacity(), 10);
    }
    #[test]
    fn text_missingness_scan_borrows_values_without_allocating() {
        let column = Column::text(
            ColumnId(1),
            "T",
            vec![Some("x".repeat(8192)), None, Some("".into())],
        )
        .unwrap();
        let column = column
            .with_metadata(ColumnMetadata {
                missing_rules: MissingRules::Text(vec!["x".repeat(8192)]),
                ..column.metadata().clone()
            })
            .unwrap();
        let (missing, allocated) = measured(|| {
            [
                column.is_missing(0).unwrap(),
                column.is_missing(1).unwrap(),
                column.is_missing(2).unwrap(),
            ]
        });
        assert_eq!(missing, [true, true, false]);
        assert_eq!(allocated, 0);
    }
}
