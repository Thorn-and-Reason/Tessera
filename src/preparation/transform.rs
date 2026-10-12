use super::*;
use crate::data::{Column, Measurement, CHUNK_SIZE, MAX_CELLS};
use std::cmp::Ordering;
pub(super) fn apply(
    p: &Project,
    s: &Step,
    c: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<Dataset> {
    admit_output(p, s, c, retained, extra)?;
    let d = p.dataset();
    let result = match s {
        Step::CleanText {
            source,
            name,
            trim_whitespace,
            rules,
        } => {
            let col = d.column(*source)?;
            let mut cells = Vec::with_capacity(col.len());
            for row in 0..col.len() {
                c.checkpoint(row, Some(col.len()))?;
                cells.push(crate::survey::cleaning::clean_cell(
                    col,
                    row,
                    *trim_whitespace,
                    rules,
                    c,
                )?);
            }
            let output = column_from_cells(p.next_column_id(), name, col.kind(), cells)?;
            d.append_column_with_budget(
                output,
                retained,
                extra + p.owned_bytes()? + s.owned_bytes()?,
            )?
        }
        Step::Compute {
            name,
            left,
            operator,
            right,
        } => {
            let output = computed_column(p.next_column_id(), name, d, left, *operator, right, c)?;
            d.append_column_with_budget(
                output,
                retained,
                extra + p.owned_bytes()? + s.owned_bytes()?,
            )?
        }
        Step::Mean { name, columns } => {
            let output = mean_column(p.next_column_id(), name, d, columns, c)?;
            d.append_column_with_budget(
                output,
                retained,
                extra + p.owned_bytes()? + s.owned_bytes()?,
            )?
        }
        Step::Recode {
            source,
            name,
            kind,
            rules,
            unmatched,
        } => {
            let output = recoded_column(
                p.next_column_id(),
                name,
                d.column(*source)?,
                *kind,
                rules,
                *unmatched,
                c,
            )?;
            d.append_column_with_budget(
                output,
                retained,
                extra + p.owned_bytes()? + s.owned_bytes()?,
            )?
        }
        Step::Sort { source, descending } => {
            let mut indices = (0..d.row_count()).collect::<Vec<_>>();
            stable_sort(&mut indices, d.column(*source)?, *descending, c)?;
            rows(d, &indices, c)?
        }
        Step::Filter { source, predicate } => {
            rows(d, &matching_rows(d.column(*source)?, predicate, c)?, c)?
        }
        Step::DatasetEdit => return Err(invalid("Direct edits cannot be replayed.")),
    };
    c.check_cancelled()?;
    Ok(result)
}
fn admit_output(
    p: &Project,
    s: &Step,
    c: &JobControl,
    retained: &[&Dataset],
    extra: usize,
) -> AppResult<()> {
    let d = p.dataset();
    let creates = matches!(
        s,
        Step::Compute { .. } | Step::Mean { .. } | Step::Recode { .. } | Step::CleanText { .. }
    );
    let count = d
        .column_count()
        .checked_add(usize::from(creates))
        .ok_or(AppError::SizeLimit { limit: MAX_CELLS })?;
    if d.row_count()
        .checked_mul(count)
        .is_none_or(|n| n > MAX_CELLS)
    {
        return Err(AppError::SizeLimit { limit: MAX_CELLS });
    }
    let mut budget = AllocationCounter::default();
    budget.add(extra)?;
    budget.add(p.owned_bytes()?)?;
    budget.add(s.owned_bytes()?)?;
    budget.array::<Column>(count.saturating_mul(2))?;
    budget.array::<usize>(d.row_count().saturating_mul(2))?;
    budget.add(4096)?;
    if let Step::Mean { columns, .. } = s {
        budget.array::<&Column>(columns.len())?;
    }
    if creates {
        let (name, kind) = match s {
            Step::Compute { name, .. } | Step::Mean { name, .. } => (name, ColumnKind::Number),
            Step::Recode { name, kind, .. } => (name, *kind),
            Step::CleanText { source, name, .. } => (name, d.column(*source)?.kind()),
            _ => unreachable!(),
        };
        if d.columns().iter().any(|col| col.name() == name) {
            return Err(invalid("The new variable name must be unique."));
        }
        p.next_column_id().next()?;
        reserve_column(
            &mut budget,
            d.row_count(),
            kind,
            recode_text_bytes(d, s, c)?,
            name.len() + 256,
        )?;
    } else {
        for col in d.columns() {
            let text = column_text_bytes(col, c)?;
            reserve_column(
                &mut budget,
                col.len(),
                col.kind(),
                text,
                col.metadata().heap_bytes()? + col.metadata().validation_work()? + 256,
            )?;
        }
    }
    Budget::dataset_bytes_iter(
        std::iter::once(d).chain(retained.iter().copied()),
        budget.bytes,
    )
    .map(|_| ())
}
fn column_text_bytes(col: &Column, c: &JobControl) -> AppResult<usize> {
    let mut bytes = AllocationCounter::default();
    if col.kind() == ColumnKind::Text {
        for row in 0..col.len() {
            c.checkpoint(row, Some(col.len()))?;
            bytes.add(col.text_value(row)?.map_or(0, str::len))?;
        }
    }
    Ok(bytes.bytes)
}
fn recode_text_bytes(d: &Dataset, s: &Step, c: &JobControl) -> AppResult<usize> {
    if let Step::CleanText {
        source,
        trim_whitespace,
        rules,
        ..
    } = s
    {
        let col = d.column(*source)?;
        let mut bytes = AllocationCounter::default();
        for row in 0..col.len() {
            c.checkpoint(row, Some(col.len()))?;
            bytes.add(crate::survey::cleaning::clean_text_len(
                col,
                row,
                *trim_whitespace,
                rules,
                c,
            )?)?;
        }
        return Ok(bytes.bytes);
    }
    let Step::Recode {
        source,
        rules,
        unmatched,
        ..
    } = s
    else {
        return Ok(0);
    };
    let col = d.column(*source)?;
    let mut bytes = AllocationCounter::default();
    for row in 0..col.len() {
        c.checkpoint(row, Some(col.len()))?;
        if col.is_missing_controlled(row, c)? {
            continue;
        }
        if let Some(rule) = matching_rule(col, row, rules)? {
            bytes.add(cell_heap(&rule.to))?;
        } else if *unmatched == Unmatched::Preserve && col.kind() == ColumnKind::Text {
            bytes.add(col.text_value(row)?.map_or(0, str::len))?;
        }
    }
    Ok(bytes.bytes)
}
fn computed_column(
    id: ColumnId,
    name: &str,
    d: &Dataset,
    left: &Operand,
    operator: Arithmetic,
    right: &Operand,
    c: &JobControl,
) -> AppResult<Column> {
    if matches!(right,Operand::Constant(v)if *v==0.) && operator == Arithmetic::Divide {
        return Err(invalid("Division by zero cannot be computed."));
    }
    let mut values = Vec::with_capacity(d.row_count());
    for row in 0..d.row_count() {
        c.checkpoint(row, Some(d.row_count()))?;
        values.push(
            match (number(d, left, row, c)?, number(d, right, row, c)?) {
                (Some(a), Some(b)) => Some(arithmetic(a, b, operator)?),
                _ => None,
            },
        );
    }
    let col = Column::numeric(id, name, values)?;
    let mut meta = col.metadata().clone();
    meta.measurement = Measurement::Scale;
    col.with_metadata(meta)
}
fn mean_column(
    id: ColumnId,
    name: &str,
    d: &Dataset,
    columns: &[ColumnId],
    c: &JobControl,
) -> AppResult<Column> {
    let sources = columns
        .iter()
        .map(|id| d.column(*id))
        .collect::<AppResult<Vec<_>>>()?;
    let mut values = Vec::with_capacity(d.row_count());
    for row in 0..d.row_count() {
        c.checkpoint(row, Some(d.row_count()))?;
        let mut items = [0.; MAX_MEAN_ITEMS];
        let mut complete = true;
        let mut scale = 0_f64;
        for (i, source) in sources.iter().enumerate() {
            if source.is_missing_controlled(row, c)? {
                complete = false;
                break;
            }
            let Cell::Number(value) = source.cell(row)? else {
                return Err(invalid("Domain averages require numeric values."));
            };
            if !value.is_finite() {
                return Err(invalid("Domain averages require finite values."));
            }
            items[i] = value;
            scale = scale.max(value.abs());
        }
        if !complete {
            values.push(None);
            continue;
        }
        // Scale before summing to avoid overflow; compensate cancellation between items.
        let (mut sum, mut correction) = (0_f64, 0_f64);
        if scale != 0. {
            for value in &items[..sources.len()] {
                let value = value / scale;
                let next = sum + value;
                correction += if sum.abs() >= value.abs() {
                    (sum - next) + value
                } else {
                    (value - next) + sum
                };
                sum = next;
            }
        }
        let mean = ((sum + correction) / sources.len() as f64).clamp(-1., 1.) * scale;
        if !mean.is_finite() {
            return Err(invalid("Domain average produced a nonfinite result."));
        }
        values.push(Some(mean));
    }
    let col = Column::numeric(id, name, values)?;
    let meta = crate::data::ColumnMetadata {
        measurement: Measurement::Scale,
        ..col.metadata().clone()
    };
    col.with_metadata(meta)
}
fn arithmetic(a: f64, b: f64, operator: Arithmetic) -> AppResult<f64> {
    let n = match operator {
        Arithmetic::Add => a + b,
        Arithmetic::Subtract => a - b,
        Arithmetic::Multiply => a * b,
        Arithmetic::Divide => {
            if b == 0. {
                return Err(invalid("Division by zero cannot be computed."));
            }
            a / b
        }
    };
    if !n.is_finite() {
        return Err(invalid("Computation produced a nonfinite result."));
    }
    Ok(n)
}
fn recoded_column(
    id: ColumnId,
    name: &str,
    col: &Column,
    kind: ColumnKind,
    rules: &[RecodeRule],
    unmatched: Unmatched,
    c: &JobControl,
) -> AppResult<Column> {
    let mut cells = Vec::with_capacity(col.len());
    for row in 0..col.len() {
        c.checkpoint(row, Some(col.len()))?;
        cells.push(if col.is_missing_controlled(row, c)? {
            Cell::Missing
        } else if let Some(rule) = matching_rule(col, row, rules)? {
            rule.to.clone()
        } else if unmatched == Unmatched::Preserve {
            col.cell(row)?
        } else {
            Cell::Missing
        });
    }
    column_from_cells(id, name, kind, cells)
}
fn matching_rows(
    col: &Column,
    predicate: &FilterPredicate,
    c: &JobControl,
) -> AppResult<Vec<usize>> {
    let mut indices = Vec::with_capacity(col.len());
    for row in 0..col.len() {
        c.checkpoint(row, Some(col.len()))?;
        let missing = col.is_missing_controlled(row, c)?;
        let keep = match predicate {
            FilterPredicate::Missing => missing,
            FilterPredicate::Present => !missing,
            FilterPredicate::Compare { comparison, value } => {
                !missing && compare_result(compare_value(col, row, value)?, *comparison)
            }
        };
        if keep {
            indices.push(row);
        }
    }
    Ok(indices)
}
fn reserve_column(
    n: &mut AllocationCounter,
    rows: usize,
    kind: ColumnKind,
    text: usize,
    metadata: usize,
) -> AppResult<()> {
    let slot = match kind {
        ColumnKind::Number => size_of::<Option<f64>>(),
        ColumnKind::Text => size_of::<Option<String>>(),
    };
    n.add(
        rows.checked_mul(slot + size_of::<Cell>())
            .and_then(|v| v.checked_mul(2))
            .ok_or(AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?,
    )?;
    n.add(text.checked_mul(2).ok_or(AppError::SizeLimit {
        limit: crate::data::MAX_OWNED_BYTES,
    })?)?;
    n.add(rows.div_ceil(CHUNK_SIZE).saturating_mul(128))?;
    n.add(metadata)?;
    Ok(())
}
fn matching_rule<'a>(
    col: &Column,
    row: usize,
    rules: &'a [RecodeRule],
) -> AppResult<Option<&'a RecodeRule>> {
    for rule in rules {
        if compare_value(col, row, &rule.from)? == Ordering::Equal {
            return Ok(Some(rule));
        }
    }
    Ok(None)
}
fn number(d: &Dataset, o: &Operand, row: usize, c: &JobControl) -> AppResult<Option<f64>> {
    match o {
        Operand::Constant(v) => Ok(Some(*v)),
        Operand::Column(id) => {
            let col = d.column(*id)?;
            if col.is_missing_controlled(row, c)? {
                return Ok(None);
            }
            match col.cell(row)? {
                Cell::Number(n) => Ok(Some(n)),
                _ => Err(invalid("Compute requires numeric values.")),
            }
        }
    }
}
fn column_from_cells(
    id: ColumnId,
    name: &str,
    kind: ColumnKind,
    cells: Vec<Cell>,
) -> AppResult<Column> {
    match kind {
        ColumnKind::Number => {
            let values = cells
                .into_iter()
                .map(|cell| match cell {
                    Cell::Number(v) => Ok(Some(v)),
                    Cell::Missing => Ok(None),
                    _ => Err(invalid("Wrong numeric recode output.")),
                })
                .collect::<AppResult<Vec<_>>>()?;
            Column::numeric(id, name, values)
        }
        ColumnKind::Text => {
            let values = cells
                .into_iter()
                .map(|cell| match cell {
                    Cell::Text(v) => Ok(Some(v)),
                    Cell::Missing => Ok(None),
                    _ => Err(invalid("Wrong text recode output.")),
                })
                .collect::<AppResult<Vec<_>>>()?;
            Column::text(id, name, values)
        }
    }
}
fn rows(d: &Dataset, indices: &[usize], c: &JobControl) -> AppResult<Dataset> {
    let mut columns = Vec::with_capacity(d.column_count());
    for old in d.columns() {
        let col = match old.kind() {
            ColumnKind::Number => {
                let mut values = Vec::with_capacity(indices.len());
                for &row in indices {
                    c.check_cancelled()?;
                    values.push(match old.cell(row)? {
                        Cell::Number(v) => Some(v),
                        _ => None,
                    });
                }
                Column::numeric(old.id(), old.name(), values)?
            }
            ColumnKind::Text => {
                let mut values = Vec::with_capacity(indices.len());
                for &row in indices {
                    c.check_cancelled()?;
                    values.push(old.text_value(row)?.map(str::to_owned));
                }
                Column::text(old.id(), old.name(), values)?
            }
        };
        columns.push(col.with_metadata(old.metadata().clone())?);
    }
    c.check_cancelled()?;
    Dataset::from_revision(columns, d.revision().next()?)
}
fn compare_value(col: &Column, row: usize, value: &Cell) -> AppResult<Ordering> {
    match value {
        Cell::Number(v) => match col.cell(row)? {
            Cell::Number(a) => Ok(a.partial_cmp(v).unwrap_or(Ordering::Equal)),
            _ => Err(invalid("Cannot compare mismatched values.")),
        },
        Cell::Text(v) => Ok(col
            .text_value(row)?
            .ok_or_else(|| invalid("Cannot compare missing text."))?
            .cmp(v)),
        Cell::Missing => Err(invalid("Use a missing test.")),
    }
}
fn compare_result(o: Ordering, c: NumericComparison) -> bool {
    match c {
        NumericComparison::Equal => o == Ordering::Equal,
        NumericComparison::NotEqual => o != Ordering::Equal,
        NumericComparison::Less => o == Ordering::Less,
        NumericComparison::LessOrEqual => o != Ordering::Greater,
        NumericComparison::Greater => o == Ordering::Greater,
        NumericComparison::GreaterOrEqual => o != Ordering::Less,
    }
}
fn row_order(
    col: &Column,
    a: usize,
    b: usize,
    descending: bool,
    c: &JobControl,
) -> AppResult<Ordering> {
    let (am, bm) = (
        col.is_missing_controlled(a, c)?,
        col.is_missing_controlled(b, c)?,
    );
    if am || bm {
        return Ok(am.cmp(&bm));
    }
    let o = match col.kind() {
        ColumnKind::Number => match (col.cell(a)?, col.cell(b)?) {
            (Cell::Number(x), Cell::Number(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => return Err(invalid("Invalid numeric sort value.")),
        },
        ColumnKind::Text => col.text_value(a)?.cmp(&col.text_value(b)?),
    };
    Ok(if descending { o.reverse() } else { o })
}
// A fallible merge sort checks cancellation during comparisons and keeps tied source rows ordered.
fn stable_sort(
    indices: &mut [usize],
    col: &Column,
    descending: bool,
    c: &JobControl,
) -> AppResult<()> {
    let mut scratch = vec![0; indices.len()];
    let mut width = 1;
    while width < indices.len() {
        for start in (0..indices.len()).step_by(width * 2) {
            let mid = (start + width).min(indices.len());
            let end = (mid + width).min(indices.len());
            let (mut a, mut b) = (start, mid);
            for slot in &mut scratch[start..end] {
                c.check_cancelled()?;
                let take_a = a < mid
                    && (b == end
                        || row_order(col, indices[a], indices[b], descending, c)?
                            != Ordering::Greater);
                *slot = if take_a {
                    let x = indices[a];
                    a += 1;
                    x
                } else {
                    let x = indices[b];
                    b += 1;
                    x
                };
            }
        }
        indices.copy_from_slice(&scratch);
        width = width.saturating_mul(2);
    }
    Ok(())
}
