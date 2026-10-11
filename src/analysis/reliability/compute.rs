use super::{arithmetic::*, ReliabilityEnergyState as State, *};
use crate::{
    analysis::{
        moments::{self, BoundedSum, Moments},
        validation::invalid_analysis,
        Estimate,
    },
    data::{Column, ColumnKind, Dataset},
    error::AppResult,
    operation::JobControl,
};
mod controls;
mod raw;
mod standardized;
mod streaming;
use controls::{fill_row, finish, resolve_columns, validate_counts, Counter, Work};
fn centered(
    sums: &[moments::BoundedSum],
    aggregate: &moments::BoundedSum,
    row: &[f64],
    n: u64,
    counter: &mut Counter<'_>,
) -> AppResult<(Vec<moments::Scaled>, Vec<moments::Scaled>, moments::Scaled)> {
    let mut total = aggregate.clone();
    for &x in row {
        total.push(-x, n)?;
        counter.tick()?;
    }
    let full = total.scaled(n)?;
    let mut item = Vec::with_capacity(row.len());
    let mut deleted = Vec::with_capacity(row.len());
    for (sum, &x) in sums.iter().zip(row) {
        let mut centered = sum.clone();
        centered.push(-x, n)?;
        item.push(centered.scaled(n)?);
        let mut rest = total.clone();
        rest.add_sum(&centered, -1)?;
        deleted.push(rest.scaled(n)?);
        counter.tick()?;
    }
    Ok((item, deleted, full))
}
fn maxima(maximum: &mut Option<i32>, value: moments::Scaled) {
    if value.fraction != 0. {
        let v = norm(value.fraction, value.exponent);
        *maximum = Some(maximum.map_or(v.e, |old| old.max(v.e)));
    }
}
/// Trusted family-local pipeline: cohort is captured from this immutable source/request.
/// Crate privacy prevents external callers passing a cohort captured for another recipe.
pub(crate) fn compute(
    dataset: &Dataset,
    request: &ReliabilityRequest,
    cohort: &ReliabilityCohort,
    control: &JobControl,
) -> AppResult<ReliabilityComputed> {
    compute_core(dataset, request, cohort, control).map(|(result, _)| result)
}
pub(super) fn compute_core(
    dataset: &Dataset,
    request: &ReliabilityRequest,
    cohort: &ReliabilityCohort,
    control: &JobControl,
) -> AppResult<(ReliabilityComputed, Work)> {
    control.check_cancelled()?;
    request.admit_controlled(control)?;
    let admission = super::accounting::numerical_admission(dataset, request, cohort, control)?;
    let quiet = control.without_progress();
    let mut counter = Counter {
        control: &quiet,
        work: Work::default(),
        limit: admission.work,
    };
    let n = validate_counts(dataset, request, cohort, &mut counter)?;
    let columns = resolve_columns(dataset, request, &mut counter)?;
    let mut source = streaming::Source {
        cohort,
        columns,
        row: vec![0.; request.items.len()],
        n,
    };
    let (sums, mean) = streaming::item_sums(&mut source, &mut counter)?;
    if n < 2 {
        return small_sample(mean, n, counter);
    }
    let sums = streaming::aggregate(sums, &mut counter)?;
    let scales = streaming::find_scales(&mut source, &sums, &mut counter)?;
    let pairs = raw::reduce(&mut source, &sums, &scales, &mut counter)?;
    let energies = raw::energies(&pairs, &scales, &mut counter)?;
    let (alpha, deleted, prefix) = raw::alphas(&energies, &mut counter)?;
    let sd = raw::sample_sds(&energies, n, &mut counter)?;
    let two_point = streaming::two_point(&mut source, &sums, &mut counter)?;
    let corrected =
        raw::correlations(&pairs, &energies, &scales, two_point.as_ref(), &mut counter)?;
    let raw = raw::Publications {
        alpha,
        deleted,
        sd,
        corrected,
    };
    let standardized = standardized::reduce(&mut source, &sums, &energies, prefix, &mut counter)?;
    let publications = standardized::publish(&standardized, &energies.item_states, &mut counter)?;
    finish(mean, raw, publications, energies, counter)
}
fn small_sample(
    mean: Vec<Estimate>,
    n: u64,
    mut counter: Counter<'_>,
) -> AppResult<(ReliabilityComputed, Work)> {
    counter.finish()?;
    let k = mean.len();
    let p = Estimate::unavailable(if n == 0 { "N=0" } else { "N<2" });
    let mut items = Vec::with_capacity(k);
    for mean in mean {
        counter.finalize()?;
        items.push(ReliabilityComputedItem {
            mean,
            sample_sd: p.clone(),
            corrected_item_total: p.clone(),
            raw_alpha_if_removed: p.clone(),
            standardized_alpha_if_removed: p.clone(),
            energy: State::Unavailable,
            remaining_energy: State::Unavailable,
            remaining_standardized_energy: State::Unavailable,
        });
    }
    counter.finish()?;
    Ok((
        ReliabilityComputed {
            items,
            raw_alpha: p.clone(),
            standardized_alpha: p,
            total_energy: State::Unavailable,
            standardized_total_energy: State::Unavailable,
        },
        counter.work,
    ))
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use std::mem::size_of;
    #[test]
    fn phase_owner_layout_fits_accepted_fixed_allowance() {
        let records = size_of::<streaming::Source<'_>>()
            + size_of::<streaming::Sums>()
            + size_of::<streaming::Scales>()
            + size_of::<raw::Reductions>()
            + size_of::<raw::Energies>()
            + size_of::<raw::Publications>()
            + size_of::<standardized::Reductions>()
            + size_of::<standardized::Publications>()
            + size_of::<Vec<Estimate>>()
            + size_of::<Option<streaming::CenteredRow>>()
            + size_of::<Counter<'_>>()
            + size_of::<JobControl>()
            + size_of::<[usize; 8]>();
        // Full phase union retains totals; residual/reduction scratch adds at most two exact owners.
        let scratch = 2 * size_of::<BoundedSum>()
            + size_of::<[[u64; 35]; 2]>()
            + size_of::<[u64; 34]>()
            + size_of::<[moments::Scaled; 4]>()
            + size_of::<[Interval; 8]>()
            + size_of::<[Estimate; 2]>()
            + size_of::<ReliabilityComputed>();
        let full = records + scratch;
        // Earlier centered passes have three exact temporaries, before standardized owners exist.
        let centered = full + size_of::<BoundedSum>()
            - size_of::<standardized::Reductions>()
            - size_of::<standardized::Publications>();
        assert!(
            full <= 6640 && centered <= 6640,
            "full={full} centered={centered}"
        );
        eprintln!("Named fixed owners: records={records}, scratch={scratch}, full={full}, centered={centered}, unchanged allowance=6640");
    }
}
