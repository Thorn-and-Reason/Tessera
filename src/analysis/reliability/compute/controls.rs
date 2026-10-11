use super::*;
use crate::analysis::{model::checked_sum, MAX_FREQUENCY};
#[derive(Default, Debug, Clone, Copy)]
pub(in crate::analysis::reliability) struct Work {
    pub numerical: usize,
    pub source: usize,
    pub classification: usize,
    pub finalization: usize,
}
impl Work {
    pub fn total(self) -> AppResult<usize> {
        [
            self.numerical,
            self.source,
            self.classification,
            self.finalization,
        ]
        .into_iter()
        .try_fold(0usize, |n, x| n.checked_add(x).ok_or_else(invalid_analysis))
    }
}
pub(super) struct Counter<'a> {
    pub control: &'a JobControl,
    pub work: Work,
    pub limit: usize,
}
impl Counter<'_> {
    fn add(&mut self, kind: usize) -> AppResult<()> {
        let field = match kind {
            0 => &mut self.work.numerical,
            1 => &mut self.work.source,
            2 => &mut self.work.classification,
            _ => &mut self.work.finalization,
        };
        *field = field.checked_add(1).ok_or_else(invalid_analysis)?;
        let total = self.work.total()?;
        if total > self.limit {
            return Err(invalid_analysis());
        }
        if total.is_multiple_of(64) {
            self.checkpoint()?;
        }
        Ok(())
    }
    pub fn tick(&mut self) -> AppResult<()> {
        self.add(0)
    }
    pub fn source(&mut self) -> AppResult<()> {
        self.add(1)
    }
    pub fn classify(&mut self) -> AppResult<()> {
        self.add(2)
    }
    pub fn finalize(&mut self) -> AppResult<()> {
        self.add(3)
    }
    pub fn checkpoint(&self) -> AppResult<()> {
        self.control
            .checkpoint(self.work.total()?, Some(self.limit))
    }
    pub fn finish(&self) -> AppResult<()> {
        self.checkpoint()
    }
}
fn invalid_eligible(
    e: &crate::analysis::selection::Eligible,
    previous: Option<usize>,
    rows: usize,
    weighted: bool,
) -> bool {
    e.row >= rows
        || e.weight == 0
        || e.weight > MAX_FREQUENCY
        || previous.is_some_and(|row| row >= e.row)
        || (!weighted && e.weight != 1)
}
pub(super) fn validate_counts(
    d: &Dataset,
    r: &ReliabilityRequest,
    c: &ReliabilityCohort,
    counter: &mut Counter<'_>,
) -> AppResult<u64> {
    let mut previous = None;
    let mut complete = 0usize;
    let mut incomplete = 0usize;
    let mut n = 0u64;
    let mut missing_n = 0u64;
    for e in &c.eligible {
        counter.source()?;
        if invalid_eligible(e, previous, d.row_count(), r.frequency_weight.is_some()) {
            return Err(invalid_analysis());
        }
        previous = Some(e.row);
        if e.complete {
            complete = complete.checked_add(1).ok_or_else(invalid_analysis)?;
            n = n.checked_add(e.weight).ok_or_else(invalid_analysis)?;
        } else {
            incomplete = incomplete.checked_add(1).ok_or_else(invalid_analysis)?;
            missing_n = missing_n
                .checked_add(e.weight)
                .ok_or_else(invalid_analysis)?;
        }
    }
    let exclusions = checked_sum([
        c.diagnostics.filtered_rows,
        c.diagnostics.missing_weight_rows,
        c.diagnostics.zero_weight_rows,
        incomplete,
        complete,
    ])?;
    if complete != c.complete_rows
        || n != c.frequency
        || missing_n != c.incomplete_case_frequency
        || incomplete != c.diagnostics.complete_case_rows
        || exclusions != d.row_count()
        || n.checked_add(missing_n).is_none_or(|v| v > MAX_FREQUENCY)
    {
        return Err(invalid_analysis());
    }
    2u128
        .checked_mul((r.items.len() + 1) as u128)
        .and_then(|v| v.checked_mul(n as u128))
        .filter(|v| *v <= 1u128 << 66)
        .ok_or_else(invalid_analysis)?;
    Ok(n)
}
pub(super) fn resolve_columns<'a>(
    d: &'a Dataset,
    r: &'a ReliabilityRequest,
    counter: &mut Counter<'_>,
) -> AppResult<Vec<(&'a Column, Option<&'a ReliabilityReverse>)>> {
    let mut result = Vec::with_capacity(r.items.len());
    for id in &r.items {
        let mut column = None;
        for candidate in d.columns() {
            counter.source()?;
            if candidate.id() == *id {
                column = Some(candidate);
                break;
            }
        }
        let column = column.ok_or_else(invalid_analysis)?;
        if column.kind() != ColumnKind::Number {
            return Err(invalid_analysis());
        }
        let mut reverse = None;
        for candidate in &r.reversals {
            counter.source()?;
            if candidate.column == *id {
                reverse = Some(candidate);
                break;
            }
        }
        result.push((column, reverse));
    }
    Ok(result)
}
pub(super) fn fill_row(
    columns: &[(&Column, Option<&ReliabilityReverse>)],
    row: usize,
    buffer: &mut [f64],
    counter: &mut Counter<'_>,
) -> AppResult<()> {
    for (i, (column, reverse)) in columns.iter().enumerate() {
        counter.source()?;
        if reverse.is_some() {
            for _ in 0..3 {
                counter.source()?
            }
        }
        buffer[i] = super::super::scoring::score_column(column, *reverse, row)?;
    }
    Ok(())
}
pub(super) fn finish(
    mean: Vec<Estimate>,
    raw: super::raw::Publications,
    standardized: super::standardized::Publications,
    energies: super::raw::Energies,
    mut counter: Counter<'_>,
) -> AppResult<(ReliabilityComputed, Work)> {
    let mut items = Vec::with_capacity(mean.len());
    let values = mean
        .into_iter()
        .zip(raw.sd)
        .zip(raw.corrected)
        .zip(raw.deleted)
        .zip(standardized.deleted)
        .zip(energies.item_states)
        .zip(energies.deleted_states)
        .zip(standardized.deleted_states);
    for (values, remaining_standardized_energy) in values {
        let (values, remaining_energy) = values;
        let (values, energy) = values;
        let (
            (((mean, sample_sd), corrected_item_total), raw_alpha_if_removed),
            standardized_alpha_if_removed,
        ) = values;
        counter.finalize()?;
        items.push(ReliabilityComputedItem {
            mean,
            sample_sd,
            corrected_item_total,
            raw_alpha_if_removed,
            standardized_alpha_if_removed,
            energy,
            remaining_energy,
            remaining_standardized_energy,
        });
    }
    counter.finish()?;
    Ok((
        ReliabilityComputed {
            items,
            raw_alpha: raw.alpha,
            standardized_alpha: standardized.alpha,
            total_energy: energies.total_state,
            standardized_total_energy: standardized.state,
        },
        counter.work,
    ))
}
