//! Count-only, physical-row cohorts. No replicated rows or numerical placeholders.
use super::*;
use crate::{
    analysis::{
        exploration::ordering::{compare_text, sort_indices},
        model::{checked_product, checked_sum},
        selection::{select_rows, Eligible},
        support,
        validation::invalid_analysis,
        RunDiagnostics,
    },
    data::{Budget, Cell, ColumnId, ColumnKind, Dataset},
    error::{AppError, AppResult},
    operation::JobControl,
};
use std::{cmp::Ordering, mem::size_of};
#[derive(Debug)]
pub(crate) struct AnovaRow {
    pub row: usize,
    pub weight: u64,
    pub cell: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnovaCellCount {
    pub physical_rows: usize,
    pub frequency: u64,
}
#[derive(Debug)]
pub(crate) struct AnovaCohort {
    pub rows: Vec<AnovaRow>,
    pub factors: Vec<AnovaFactorLevels>,
    pub cells: Vec<AnovaCellCount>,
    pub diagnostics: RunDiagnostics,
    pub frequency: u64,
    pub incomplete_case_frequency: u64,
    pub observed_measurements: usize,
    pub failure: Option<AnovaFailure>,
}
impl AnovaCohort {
    pub(crate) fn owned_bytes(&self) -> AppResult<usize> {
        let mut bytes = checked_sum([
            size_of::<Self>(),
            super::accounting::array::<AnovaRow>(self.rows.capacity())?,
            super::accounting::array::<AnovaFactorLevels>(self.factors.capacity())?,
            super::accounting::array::<AnovaCellCount>(self.cells.capacity())?,
            self.failure.as_ref().map_or(0, |f| f.reason.capacity()),
        ])?;
        for factor in &self.factors {
            bytes = checked_sum([
                bytes,
                factor.name.capacity(),
                super::accounting::array::<Cell>(factor.values.capacity())?,
            ])?;
            for value in &factor.values {
                bytes = checked_sum([bytes, super::accounting::cell_bytes(value)])?;
            }
        }
        Ok(bytes)
    }
}
fn compare_rows(
    d: &Dataset,
    id: ColumnId,
    a: usize,
    b: usize,
    c: &JobControl,
) -> AppResult<Ordering> {
    let col = d.column(id)?;
    match col.kind() {
        ColumnKind::Text => compare_text(
            col.text_value(a)?.ok_or_else(invalid_analysis)?,
            col.text_value(b)?.ok_or_else(invalid_analysis)?,
            c,
        ),
        ColumnKind::Number => {
            let (Cell::Number(a), Cell::Number(b)) = (col.cell(a)?, col.cell(b)?) else {
                return Err(invalid_analysis());
            };
            a.partial_cmp(&b).ok_or_else(invalid_analysis)
        }
    }
}
fn copy_value(d: &Dataset, id: ColumnId, row: usize, c: &JobControl) -> AppResult<Cell> {
    let col = d.column(id)?;
    if col.kind() == ColumnKind::Text {
        Ok(Cell::Text(support::clone_text(
            col.text_value(row)?.ok_or_else(invalid_analysis)?,
            c,
        )?))
    } else {
        let Cell::Number(v) = col.cell(row)? else {
            return Err(invalid_analysis());
        };
        Ok(Cell::Number(if v == 0. { 0. } else { v }))
    }
}
fn value_bytes(d: &Dataset, id: ColumnId, row: usize) -> AppResult<usize> {
    Ok(d.column(id)?
        .text_value(row)
        .ok()
        .flatten()
        .map_or(0, str::len))
}
fn failure(kind: AnovaFailureKind, reason: &str) -> AnovaFailure {
    AnovaFailure {
        kind,
        reason: reason.into(),
    }
}
pub(crate) fn select(
    d: &Dataset,
    r: &AnovaRequest,
    control: &JobControl,
    extra_bytes: usize,
) -> AppResult<AnovaCohort> {
    let quiet = control.without_progress();
    let c = &quiet;
    c.checkpoint(0, None)?;
    r.validate_current(d, &[], extra_bytes, c)?;
    let n = d.row_count();
    // Full-length selection, sort indexes and final rows coexist. Copies are charged separately below.
    let scratch = checked_sum([
        checked_product(
            n,
            size_of::<Eligible>() + size_of::<AnovaRow>() + 3 * size_of::<usize>(),
        )?,
        r.validation_scratch_bytes()?,
        checked_product(
            MAX_GRID,
            size_of::<AnovaCellCount>() + size_of::<AnovaFactorLevels>() + 6 * size_of::<Cell>(),
        )?,
        1024,
    ])?;
    let baseline = Budget::dataset_bytes(&[d], checked_sum([extra_bytes, scratch])?)?;
    let mut complete = Vec::new();
    let mut factor_ids = Vec::new();
    match &r.design {
        AnovaDesign::OneWay { outcome, factor } => {
            complete.extend([*outcome, *factor]);
            factor_ids.push(*factor);
        }
        AnovaDesign::Factorial { outcome, factors } => {
            complete.push(*outcome);
            complete.extend(factors);
            factor_ids.extend(factors);
        }
        AnovaDesign::Repeated {
            subject,
            measurements,
            ..
        } => {
            let mut ids = Vec::with_capacity(n);
            let mut sorting = vec![0; n];
            for row in 0..n {
                c.checkpoint(row, None)?;
                if !r.filter.matches(d, row, c)? {
                    continue;
                }
                if d.column(*subject)?.is_missing(row)? {
                    return Err(AppError::invalid(format!("subject, row {}",row+1),"Every retained row needs a subject ID before incomplete profiles are excluded."));
                }
                ids.push(row);
            }
            sort_indices(&mut ids, &mut sorting, c, |a, b| {
                compare_rows(d, *subject, a, b, c)
            })?;
            for pair in ids.windows(2) {
                c.check_cancelled()?;
                if compare_rows(d, *subject, pair[0], pair[1], c)? == Ordering::Equal {
                    return Err(AppError::invalid(
                        format!("subject, rows {} and {}", pair[0] + 1, pair[1] + 1),
                        "Retained subject IDs must be unique, including incomplete profiles.",
                    ));
                }
            }
            complete.extend(measurements);
        }
    }
    let (eligible, diagnostics) = select_rows(d, &r.filter, r.frequency_weight, &complete, c, n)?;
    let frequency = eligible
        .iter()
        .filter(|e| e.complete)
        .map(|e| e.weight)
        .sum();
    let incomplete_case_frequency = eligible
        .iter()
        .filter(|e| !e.complete)
        .map(|e| e.weight)
        .sum();
    let mut rows = Vec::with_capacity(eligible.len());
    for e in &eligible {
        c.checkpoint(e.row, None)?;
        if e.complete {
            rows.push(AnovaRow {
                row: e.row,
                weight: e.weight,
                cell: 0,
            });
        }
    }
    let mut factors = Vec::with_capacity(6);
    let mut copy_bytes = 0usize;
    let mut grid = 1usize;
    if let AnovaDesign::Repeated {
        factors: declared, ..
    } = &r.design
    {
        for f in declared {
            copy_bytes = checked_sum([copy_bytes, f.name.len()])?;
            for s in &f.levels {
                copy_bytes = checked_sum([copy_bytes, s.len()])?;
            }
        }
        Budget::check_datasets(&[], checked_sum([baseline, copy_bytes])?)?;
        for f in declared {
            let mut values = Vec::with_capacity(f.levels.len());
            for s in &f.levels {
                values.push(Cell::Text(support::clone_text(s, c)?));
            }
            grid = checked_product(grid, values.len())?;
            factors.push(AnovaFactorLevels {
                column: None,
                name: support::clone_text(&f.name, c)?,
                values,
            });
        }
    } else {
        let mut indices = (0..rows.len()).collect::<Vec<_>>();
        let mut sorting = vec![0; rows.len()];
        for id in &factor_ids {
            sort_indices(&mut indices, &mut sorting, c, |a, b| {
                compare_rows(d, *id, rows[a].row, rows[b].row, c)
            })?;
            let mut unique = Vec::with_capacity(MAX_GRID);
            let mut previous = None;
            for index in &indices {
                c.checkpoint(*index, None)?;
                let row = rows[*index].row;
                let new = match previous {
                    None => true,
                    Some(p) => compare_rows(d, *id, p, row, c)? != Ordering::Equal,
                };
                if new {
                    if unique.len() == MAX_GRID {
                        return Err(invalid_analysis());
                    }
                    unique.push(row);
                }
                previous = Some(row);
            }
            grid = checked_product(grid, unique.len())?;
            if grid > MAX_GRID {
                return Err(invalid_analysis());
            }
            copy_bytes = checked_sum([copy_bytes, d.column(*id)?.metadata().name.len()])?;
            for row in &unique {
                copy_bytes = checked_sum([copy_bytes, value_bytes(d, *id, *row)?])?;
            }
            Budget::check_datasets(&[], checked_sum([baseline, copy_bytes])?)?;
            let mut values = Vec::with_capacity(unique.len());
            for row in unique {
                values.push(copy_value(d, *id, row, c)?);
            }
            factors.push(AnovaFactorLevels {
                column: Some(*id),
                name: support::clone_text(&d.column(*id)?.metadata().name, c)?,
                values,
            });
        }
    }
    let mut cells = vec![
        AnovaCellCount {
            physical_rows: 0,
            frequency: 0
        };
        grid
    ];
    for row in &mut rows {
        c.checkpoint(row.row, None)?;
        let mut cell = 0;
        if !matches!(r.design, AnovaDesign::Repeated { .. }) {
            for (id, f) in factor_ids.iter().zip(&factors) {
                let mut found = None;
                for (i, v) in f.values.iter().enumerate() {
                    let col = d.column(*id)?;
                    let equal = match v {
                        Cell::Text(s) => {
                            compare_text(
                                col.text_value(row.row)?.ok_or_else(invalid_analysis)?,
                                s,
                                c,
                            )? == Ordering::Equal
                        }
                        Cell::Number(v) => col.cell(row.row)? == Cell::Number(*v),
                        _ => false,
                    };
                    if equal {
                        found = Some(i);
                        break;
                    }
                }
                cell = checked_sum([
                    checked_product(cell, f.values.len())?,
                    found.ok_or_else(invalid_analysis)?,
                ])?;
            }
            row.cell = cell;
            cells[cell].physical_rows += 1;
            cells[cell].frequency += row.weight;
        }
    }
    let repeated = matches!(r.design, AnovaDesign::Repeated { .. });
    if repeated {
        for cell in &mut cells {
            cell.physical_rows = rows.len();
            cell.frequency = frequency;
        }
    }
    Budget::check_datasets(
        &[],
        checked_sum([
            baseline,
            copy_bytes,
            super::accounting::family_bytes(r, &factors, c)?,
        ])?,
    )?;
    let observed_measurements = if repeated {
        checked_product(rows.len(), grid)?
    } else {
        rows.len()
    };
    let unavailable = if rows.is_empty() {
        Some(failure(
            AnovaFailureKind::EmptyCohort,
            "No complete observations remain.",
        ))
    } else if factors.iter().any(|f| f.values.len() < 2) {
        Some(failure(
            AnovaFailureKind::CollapsedFactor,
            "A realised factor has fewer than two levels.",
        ))
    } else if cells.iter().any(|cell| cell.frequency == 0) {
        Some(failure(
            AnovaFailureKind::EmptyCells,
            "Inference requires a complete crossed grid; at least one realised cell is empty.",
        ))
    } else {
        None
    };
    Ok(AnovaCohort {
        rows,
        factors,
        cells,
        diagnostics,
        frequency,
        incomplete_case_frequency,
        observed_measurements,
        failure: unavailable,
    })
}
#[cfg(test)]
mod tests;
