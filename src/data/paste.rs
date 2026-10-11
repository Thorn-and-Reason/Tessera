use super::{
    column::Chunks,
    edit::{chunk_buffer_bytes, chunk_copy_bytes},
    *,
};
use crate::{
    import::{Delimiter, Scanner},
    operation::JobControl,
};
use std::{mem::size_of, sync::Arc};

const MAX_PASTE_BYTES: usize = 512 * 1024;
const MAX_PASTE_CELLS: usize = 4096;

impl Dataset {
    /// Paste a bounded TSV rectangle atomically into existing cells.
    pub fn paste_tsv_with_budget(
        &self,
        start_column: ColumnId,
        start_row: usize,
        text: &str,
        retained: &[&Dataset],
        extra_bytes: usize,
    ) -> AppResult<Self> {
        if text.len() > MAX_PASTE_BYTES {
            return Err(AppError::invalid(
                "clipboard",
                "Paste at most 512 KiB and 4,096 cells at a time.",
            ));
        }
        let start = self
            .columns
            .iter()
            .position(|c| c.id() == start_column)
            .ok_or_else(|| AppError::invalid("column", "Column no longer exists."))?;
        if start_row >= self.rows || text.is_empty() {
            return Err(AppError::invalid(
                "clipboard",
                "Select an existing cell and copy a nonempty rectangle.",
            ));
        }
        let revision = self.revision.next()?;
        // Reserve parsing descriptors, decoded strings and transient field storage before scanning.
        let scratch = text.len() * 3 + MAX_PASTE_CELLS * size_of::<Cell>() * 2;
        let mut allocation = AllocationCounter::default();
        allocation.add(extra_bytes)?;
        allocation.add(self.copy_overhead()?)?;
        allocation.add(scratch)?;
        let baseline = Budget::context_bytes(self, retained, None, allocation.bytes)?;
        let control = JobControl::default();
        let mut scanner = Scanner::new(text.as_bytes(), Delimiter::Tab, &control, baseline)?;
        let (mut width, mut height, mut count) = (0, 0, 0);
        while let Some(field) = scanner.next()? {
            count += 1;
            if count > MAX_PASTE_CELLS {
                return Err(field.invalid("Paste at most 4,096 cells at a time."));
            }
            if field.column > self.columns.len() - start || field.row > self.rows - start_row {
                return Err(field.invalid(
                    "The pasted rectangle must fit within the existing rows and columns.",
                ));
            }
            if field.end_record {
                if width == 0 {
                    width = field.column;
                }
                if field.column != width {
                    return Err(
                        field.invalid("Every pasted row must have the same number of fields.")
                    );
                }
                height = field.row;
            }
            if field.row > 1 && field.column > width {
                return Err(field.invalid("Every pasted row must have the same number of fields."));
            }
            if self.columns[start + field.column - 1].kind() == ColumnKind::Number
                && !field.text.trim().is_empty()
            {
                parse_number(&field.text)
                    .map_err(|_| field.invalid("Enter a finite number in numeric columns."))?;
            }
        }
        if count == 0 {
            return Err(AppError::invalid("clipboard", "Copy a nonempty rectangle."));
        }
        for column in &self.columns[start..start + width] {
            for chunk in start_row / CHUNK_SIZE..=(start_row + height - 1) / CHUNK_SIZE {
                allocation.add(chunk_copy_bytes(column, chunk)?)?;
                allocation.add(chunk_buffer_bytes(column, chunk))?;
            }
        }
        let baseline = Budget::context_bytes(self, retained, None, allocation.bytes)?;
        let mut values = Vec::with_capacity(count);
        let mut scanner = Scanner::new(text.as_bytes(), Delimiter::Tab, &control, baseline)?;
        while let Some(field) = scanner.next()? {
            values.push(match self.columns[start + field.column - 1].kind() {
                ColumnKind::Number if field.text.trim().is_empty() => Cell::Missing,
                ColumnKind::Number => Cell::Number(parse_number(&field.text)?),
                ColumnKind::Text => Cell::Text(field.text.into_owned()),
            });
        }
        let mut columns = self.columns.clone();
        for offset in 0..width {
            let column = &mut columns[start + offset];
            match &mut column.chunks {
                Chunks::Number(chunks) => {
                    for (index, stored) in chunks
                        .iter_mut()
                        .enumerate()
                        .take((start_row + height - 1) / CHUNK_SIZE + 1)
                        .skip(start_row / CHUNK_SIZE)
                    {
                        let mut chunk = stored.to_vec();
                        for row in start_row.max(index * CHUNK_SIZE)
                            ..(start_row + height).min((index + 1) * CHUNK_SIZE)
                        {
                            chunk[row % CHUNK_SIZE] =
                                match values[(row - start_row) * width + offset] {
                                    Cell::Number(v) => Some(v),
                                    _ => None,
                                };
                        }
                        *stored = Arc::from(chunk);
                    }
                }
                Chunks::Text(chunks) => {
                    for (index, stored) in chunks
                        .iter_mut()
                        .enumerate()
                        .take((start_row + height - 1) / CHUNK_SIZE + 1)
                        .skip(start_row / CHUNK_SIZE)
                    {
                        let mut chunk = stored.to_vec();
                        for row in start_row.max(index * CHUNK_SIZE)
                            ..(start_row + height).min((index + 1) * CHUNK_SIZE)
                        {
                            let value = std::mem::replace(
                                &mut values[(row - start_row) * width + offset],
                                Cell::Missing,
                            );
                            chunk[row % CHUNK_SIZE] = match value {
                                Cell::Text(v) => Some(v),
                                _ => None,
                            };
                        }
                        *stored = Arc::from(chunk);
                    }
                }
            }
        }
        self.finish_with_budget(columns, self.rows, revision, retained, extra_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_budget_refuses_before_clipboard_or_chunk_allocation() {
        let dataset = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Text",
            vec![Some("original".into())],
        )
        .unwrap()])
        .unwrap();
        let (result, largest) = crate::data::measure_test_allocations(|| {
            dataset.paste_tsv_with_budget(
                ColumnId(1),
                0,
                "\"quoted\"\"field\"",
                &[],
                MAX_OWNED_BYTES,
            )
        });
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert!(largest < 1024, "allocated {largest} bytes before refusal");
    }

    #[test]
    fn paste_shares_untouched_chunks_and_metadata() {
        let dataset = Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "Number",
            vec![Some(1.0); CHUNK_SIZE + 1],
        )
        .unwrap()])
        .unwrap();
        let pasted = dataset
            .paste_tsv_with_budget(ColumnId(1), 0, "2\n3", &[], 0)
            .unwrap();
        let (Chunks::Number(old), Chunks::Number(new)) =
            (&dataset.columns[0].chunks, &pasted.columns[0].chunks)
        else {
            panic!()
        };
        assert!(!Arc::ptr_eq(&old[0], &new[0]));
        assert!(Arc::ptr_eq(&old[1], &new[1]));
        assert!(Arc::ptr_eq(
            &dataset.columns[0].metadata,
            &pasted.columns[0].metadata
        ));
    }
}
