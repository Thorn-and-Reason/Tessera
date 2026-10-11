use super::{
    streaming::{CenteredRow, Scales, Source, Sums},
    *,
};
pub(super) struct Reductions {
    pub item: Vec<Pair>,
    pub deleted: Vec<Pair>,
    pub covariance: Vec<Pair>,
    pub total: Pair,
}
pub(super) struct Energies {
    pub item: Vec<Interval>,
    pub item_states: Vec<State>,
    pub total: Interval,
    pub total_state: State,
    pub deleted: Vec<Interval>,
    pub deleted_states: Vec<State>,
}
pub(super) struct Publications {
    pub alpha: Estimate,
    pub deleted: Vec<Estimate>,
    pub sd: Vec<Estimate>,
    pub corrected: Vec<Estimate>,
}
pub(super) fn reduce(
    source: &mut Source<'_>,
    sums: &Sums,
    scales: &Scales,
    counter: &mut Counter<'_>,
) -> AppResult<Reductions> {
    let k = source.row.len();
    let mut reductions = Reductions {
        item: (0..k).map(|_| Pair::default()).collect(),
        deleted: (0..k).map(|_| Pair::default()).collect(),
        covariance: (0..k).map(|_| Pair::default()).collect(),
        total: Pair::default(),
    };
    counter.checkpoint()?;
    for e in &source.cohort.eligible {
        counter.source()?;
        if !e.complete {
            continue;
        }
        fill_row(&source.columns, e.row, &mut source.row, counter)?;
        let (d, rest, t) = centered(&sums.items, &sums.total, &source.row, source.n, counter)?;
        reductions
            .total
            .push_weighted(isquare(interval_of(t, scales.total.unwrap_or(0))), e.weight)?;
        for i in 0..k {
            let a = interval_of(d[i], scales.item[i].unwrap_or(0));
            let b = interval_of(rest[i], scales.deleted[i].unwrap_or(0));
            reductions.item[i].push_weighted(isquare(a), e.weight)?;
            reductions.deleted[i].push_weighted(isquare(b), e.weight)?;
            reductions.covariance[i].push_weighted(imul(a, b), e.weight)?;
            counter.tick()?;
        }
    }
    Ok(reductions)
}
fn values(
    pairs: &[Pair],
    scales: &[Option<i32>],
    counter: &mut Counter<'_>,
) -> AppResult<Vec<Interval>> {
    let mut values = Vec::with_capacity(pairs.len());
    for (i, pair) in pairs.iter().enumerate() {
        counter.finalize()?;
        values.push(pair.value(2 * scales[i].unwrap_or(0))?);
    }
    Ok(values)
}
fn states(
    values: &[Interval],
    scales: &[Option<i32>],
    counter: &mut Counter<'_>,
) -> AppResult<Vec<State>> {
    let mut states = Vec::with_capacity(values.len());
    for (i, &value) in values.iter().enumerate() {
        counter.finalize()?;
        states.push(energy_state(scales[i].is_some(), value));
    }
    Ok(states)
}
pub(super) fn energies(
    pairs: &Reductions,
    scales: &Scales,
    counter: &mut Counter<'_>,
) -> AppResult<Energies> {
    let item = values(&pairs.item, &scales.item, counter)?;
    let item_states = states(&item, &scales.item, counter)?;
    let total = pairs.total.value(2 * scales.total.unwrap_or(0))?;
    let total_state = energy_state(scales.total.is_some(), total);
    let deleted = values(&pairs.deleted, &scales.deleted, counter)?;
    let deleted_states = states(&deleted, &scales.deleted, counter)?;
    Ok(Energies {
        item,
        item_states,
        total,
        total_state,
        deleted,
        deleted_states,
    })
}
pub(super) fn alphas(
    energies: &Energies,
    counter: &mut Counter<'_>,
) -> AppResult<(Estimate, Vec<Estimate>, Vec<Interval>)> {
    let k = energies.item.len();
    let mut prefix = vec![exact(0.); k + 1];
    for i in 0..k {
        prefix[i + 1] = iadd(prefix[i], energies.item[i]);
        counter.tick()?;
    }
    let mut unresolved = 0usize;
    for state in &energies.item_states {
        counter.finalize()?;
        unresolved += usize::from(*state == State::Unresolved);
    }
    let alpha = if unresolved > 0 {
        Estimate::unavailable("Unresolved item energy")
    } else {
        publish_alpha(prefix[k], energies.total, k, energies.total_state)
    };
    counter.checkpoint()?;
    let mut suffix = exact(0.);
    let mut deleted = vec![Estimate::unavailable("pending"); k];
    for i in (0..k).rev() {
        deleted[i] = if unresolved > usize::from(energies.item_states[i] == State::Unresolved) {
            Estimate::unavailable("Unresolved retained item energy")
        } else {
            publish_alpha(
                iadd(prefix[i], suffix),
                energies.deleted[i],
                k - 1,
                energies.deleted_states[i],
            )
        };
        suffix = iadd(suffix, energies.item[i]);
        counter.tick()?;
    }
    Ok((alpha, deleted, prefix))
}
pub(super) fn sample_sds(
    energies: &Energies,
    n: u64,
    counter: &mut Counter<'_>,
) -> AppResult<Vec<Estimate>> {
    let mut sd = Vec::with_capacity(energies.item.len());
    for (i, &energy) in energies.item.iter().enumerate() {
        counter.finalize()?;
        sd.push(publish_sd(energy, n, energies.item_states[i]));
    }
    Ok(sd)
}
pub(super) fn correlations(
    pairs: &Reductions,
    energies: &Energies,
    scales: &Scales,
    two_point: Option<&CenteredRow>,
    counter: &mut Counter<'_>,
) -> AppResult<Vec<Estimate>> {
    let mut corrected = Vec::with_capacity(energies.item.len());
    for i in 0..energies.item.len() {
        counter.finalize()?;
        let sign = two_point.map(|(d, rest, _)| {
            d[i].fraction.is_sign_negative() == rest[i].fraction.is_sign_negative()
        });
        let covariance = pairs.covariance[i]
            .value(scales.item[i].unwrap_or(0) + scales.deleted[i].unwrap_or(0))?;
        corrected.push(publish_correlation(
            covariance,
            energies.item[i],
            energies.deleted[i],
            energies.item_states[i],
            energies.deleted_states[i],
            sign,
        ));
    }
    Ok(corrected)
}
