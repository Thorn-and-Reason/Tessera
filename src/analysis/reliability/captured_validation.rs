//! Passive facts and publication dependencies; no present-data access or recomputation.
use super::{ReliabilityEnergyState as State, *};
use crate::{
    analysis::{
        model::checked_sum,
        validation::{invalid_analysis, validate_snapshot_controlled},
        Estimate, MAX_FREQUENCY,
    },
    data::{Budget, ColumnId, MAX_CELLS},
    error::AppResult,
    operation::JobControl,
};
use std::collections::BTreeMap;
fn check(ok: bool) -> AppResult<()> {
    if ok {
        Ok(())
    } else {
        Err(invalid_analysis())
    }
}
impl ReliabilityRun {
    pub fn validate_controlled(&self, control: &JobControl) -> AppResult<()> {
        control.check_cancelled()?;
        let quiet = control.without_progress();
        let c = &quiet;
        self.request.admit_controlled(c)?;
        Budget::check_datasets(
            &[],
            checked_sum([
                self.owned_bytes_controlled(Some(c))?,
                self.scratch_controlled(Some(c))?,
            ])?,
        )?;
        captured(self, c)?;
        c.check_cancelled()
    }
}
fn captured(r: &ReliabilityRun, c: &JobControl) -> AppResult<()> {
    check(
        r.revision.0 != 0 && r.source_rows <= MAX_CELLS && r.items.len() == r.request.items.len(),
    )?;
    let mut infos = BTreeMap::new();
    for (i, info) in r.columns.iter().enumerate() {
        if i.is_multiple_of(64) {
            c.checkpoint(i, None)?;
        }
        check(infos.insert(info.id, info).is_none())?;
        validate_snapshot_controlled(info, c)?;
    }
    let ids = super::validation::request(
        &r.request,
        |id: ColumnId| infos.get(&id).map(|v| v.kind).ok_or_else(invalid_analysis),
        c,
    )?;
    check(ids.len() == infos.len())?;
    counts(r)?;
    estimate(&r.raw_alpha, c)?;
    estimate(&r.standardized_alpha, c)?;
    check(
        r.raw_alpha.value.is_none_or(|v| v <= 1.)
            && r.standardized_alpha.value.is_none_or(|v| v <= 1.),
    )?;
    for (i, item) in r.items.iter().enumerate() {
        c.checkpoint(i, None)?;
        check(item.column == r.request.items[i])?;
        item_values(item, c)?;
        item_states(r, item)?;
    }
    full_states(r)?;
    for (i, item) in r.items.iter().enumerate() {
        c.checkpoint(i, None)?;
        deletion(r, i, item)?;
    }
    Ok(())
}
fn counts(r: &ReliabilityRun) -> AppResult<()> {
    let d = &r.diagnostics;
    check(
        [
            d.filtered_rows,
            d.missing_weight_rows,
            d.zero_weight_rows,
            d.complete_case_rows,
            r.complete_rows,
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
            == Some(r.source_rows),
    )?;
    let weighted = r.request.frequency_weight.is_some();
    check(weighted || (d.missing_weight_rows == 0 && d.zero_weight_rows == 0))?;
    count(r.complete_rows, r.frequency, weighted)?;
    count(d.complete_case_rows, r.incomplete_case_frequency, weighted)?;
    check(
        r.frequency
            .checked_add(r.incomplete_case_frequency)
            .is_some_and(|n| n <= MAX_FREQUENCY),
    )
}
fn count(rows: usize, n: u64, weighted: bool) -> AppResult<()> {
    check(
        n <= MAX_FREQUENCY
            && n >= rows as u64
            && (rows == 0) == (n == 0)
            && (weighted || n == rows as u64),
    )
}
fn estimate(e: &Estimate, c: &JobControl) -> AppResult<()> {
    match (&e.value, &e.reason) {
        (Some(v), None) => check(v.is_finite()),
        (None, Some(s)) => {
            check(s.len() <= 1024)?;
            let mut nonblank = false;
            for (i, ch) in s.char_indices() {
                if i.is_multiple_of(64) {
                    c.checkpoint(i, None)?;
                }
                nonblank |= !ch.is_whitespace();
            }
            check(nonblank)
        }
        _ => Err(invalid_analysis()),
    }
}
fn item_values(item: &ReliabilityItem, c: &JobControl) -> AppResult<()> {
    for e in [
        &item.mean,
        &item.sample_sd,
        &item.corrected_item_total,
        &item.raw_alpha_if_removed,
        &item.standardized_alpha_if_removed,
    ] {
        estimate(e, c)?;
    }
    check(
        item.sample_sd.value.is_none_or(|v| v >= 0.)
            && item
                .corrected_item_total
                .value
                .is_none_or(|v| (-1.0..=1.0).contains(&v))
            && item.raw_alpha_if_removed.value.is_none_or(|v| v <= 1.)
            && item
                .standardized_alpha_if_removed
                .value
                .is_none_or(|v| v <= 1.),
    )
}
fn item_states(r: &ReliabilityRun, item: &ReliabilityItem) -> AppResult<()> {
    check((r.frequency == 0) == item.mean.value.is_none())?;
    if r.frequency < 2 {
        return check(
            item.energy == State::Unavailable
                && item.remaining_energy == State::Unavailable
                && item.remaining_standardized_energy == State::Unavailable
                && [
                    &item.sample_sd,
                    &item.corrected_item_total,
                    &item.raw_alpha_if_removed,
                    &item.standardized_alpha_if_removed,
                ]
                .iter()
                .all(|e| e.value.is_none()),
        );
    }
    check(item.energy != State::Unavailable && item.remaining_energy != State::Unavailable)?;
    if r.complete_rows == 1 {
        check(item.energy == State::ExactZero && item.remaining_energy == State::ExactZero)?;
    }
    match item.energy {
        State::ExactZero => check(item.sample_sd.value == Some(0.))?,
        State::Positive => check(item.sample_sd.value.is_none_or(|v| v > 0.))?,
        State::Unresolved => check(item.sample_sd.value.is_none())?,
        State::Unavailable => return Err(invalid_analysis()),
    }
    check(
        item.corrected_item_total.value.is_none()
            || (item.energy == State::Positive && item.remaining_energy == State::Positive),
    )
}
fn standardized(
    state: State,
    items: impl Iterator<Item = State>,
    undefined: bool,
) -> AppResult<()> {
    let mut constant = false;
    let mut unresolved = false;
    for item in items {
        constant |= matches!(item, State::ExactZero | State::Unavailable);
        unresolved |= item == State::Unresolved;
    }
    if undefined || constant {
        check(state == State::Unavailable)
    } else if unresolved {
        check(state == State::Unresolved)
    } else {
        check(state != State::Unavailable)
    }
}
fn full_states(r: &ReliabilityRun) -> AppResult<()> {
    if r.frequency < 2 {
        return check(
            r.total_energy == State::Unavailable
                && r.standardized_total_energy == State::Unavailable
                && r.raw_alpha.value.is_none()
                && r.standardized_alpha.value.is_none(),
        );
    }
    check(r.total_energy != State::Unavailable)?;
    if r.complete_rows == 1 {
        check(r.total_energy == State::ExactZero)?;
    }
    standardized(
        r.standardized_total_energy,
        r.items.iter().map(|i| i.energy),
        false,
    )?;
    check(
        r.raw_alpha.value.is_none()
            || (r.total_energy == State::Positive
                && r.items
                    .iter()
                    .all(|i| matches!(i.energy, State::ExactZero | State::Positive))),
    )?;
    check(
        r.standardized_alpha.value.is_none()
            || (r.standardized_total_energy == State::Positive
                && r.items.iter().all(|i| i.energy == State::Positive)),
    )
}
fn deletion(r: &ReliabilityRun, index: usize, item: &ReliabilityItem) -> AppResult<()> {
    if r.frequency < 2 {
        return Ok(());
    }
    let retained = || {
        r.items
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, i)| i.energy)
    };
    let two = r.items.len() == 2;
    standardized(item.remaining_standardized_energy, retained(), two)?;
    check(
        item.raw_alpha_if_removed.value.is_none()
            || (!two
                && item.remaining_energy == State::Positive
                && retained().all(|s| matches!(s, State::ExactZero | State::Positive))),
    )?;
    check(
        item.standardized_alpha_if_removed.value.is_none()
            || (!two
                && item.remaining_standardized_energy == State::Positive
                && retained().all(|s| s == State::Positive)),
    )
}

#[cfg(test)]
mod tests;
