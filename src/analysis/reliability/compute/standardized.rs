use super::{
    raw::Energies,
    streaming::{Source, Sums},
    *,
};
pub(super) struct Reductions {
    pub deleted: Vec<Pair>,
    pub total: Pair,
    pub z: Vec<Interval>,
    pub prefix: Vec<Interval>,
    pub bad: bool,
    pub saw_total: bool,
    pub saw_deleted: Vec<bool>,
}
pub(super) struct Publications {
    pub alpha: Estimate,
    pub deleted: Vec<Estimate>,
    pub deleted_states: Vec<State>,
    pub state: State,
}
pub(super) fn reduce(
    source: &mut Source<'_>,
    sums: &Sums,
    energies: &Energies,
    prefix: Vec<Interval>,
    counter: &mut Counter<'_>,
) -> AppResult<Reductions> {
    let k = source.row.len();
    let mut reductions = Reductions {
        deleted: (0..k).map(|_| Pair::default()).collect(),
        total: Pair::default(),
        z: vec![exact(0.); k],
        prefix,
        bad: false,
        saw_total: false,
        saw_deleted: vec![false; k],
    };
    counter.checkpoint()?;
    for e in &source.cohort.eligible {
        counter.source()?;
        if !e.complete {
            continue;
        }
        fill_row(&source.columns, e.row, &mut source.row, counter)?;
        normalize_row(source, sums, energies, &mut reductions, counter)?;
        if reductions.bad {
            continue;
        }
        reduce_row(&mut reductions, e.weight, counter)?;
    }
    Ok(reductions)
}
fn normalize_row(
    source: &Source<'_>,
    sums: &Sums,
    energies: &Energies,
    reductions: &mut Reductions,
    counter: &mut Counter<'_>,
) -> AppResult<()> {
    for i in 0..source.row.len() {
        reductions.z[i] = if energies.item_states[i] == State::Positive {
            idiv(
                interval_of(sums.items[i].residual(source.row[i], source.n)?, 0),
                isqrt(energies.item[i]),
            )
        } else {
            exact(0.)
        };
        // Exact z bound1 does not bound interval endpoints. Enforce a proved finite2 envelope.
        if abs_cmp(reductions.z[i].lo, norm(2., 0)).is_gt()
            || abs_cmp(reductions.z[i].hi, norm(2., 0)).is_gt()
        {
            reductions.bad = true;
        }
        reductions.prefix[i + 1] = iadd(reductions.prefix[i], reductions.z[i]);
        counter.tick()?;
    }
    Ok(())
}
fn reduce_row(
    reductions: &mut Reductions,
    weight: u64,
    counter: &mut Counter<'_>,
) -> AppResult<()> {
    let k = reductions.z.len();
    let t = isquare(reductions.prefix[k]);
    reductions.saw_total |= t.hi.f != 0.;
    reductions.total.push_weighted(t, weight)?;
    let mut suffix = exact(0.);
    for i in (0..k).rev() {
        let t = isquare(iadd(reductions.prefix[i], suffix));
        reductions.saw_deleted[i] |= t.hi.f != 0.;
        reductions.deleted[i].push_weighted(t, weight)?;
        suffix = iadd(suffix, reductions.z[i]);
        counter.tick()?;
    }
    Ok(())
}
fn full_state(
    reductions: &Reductions,
    states: &[State],
    interval: Interval,
    counter: &mut Counter<'_>,
) -> AppResult<State> {
    let mut all_positive = true;
    let mut any_zero = false;
    for state in states {
        counter.finalize()?;
        all_positive &= *state == State::Positive;
        any_zero |= *state == State::ExactZero;
    }
    counter.checkpoint()?;
    Ok(if any_zero {
        State::Unavailable
    } else if reductions.bad || !all_positive {
        State::Unresolved
    } else {
        energy_state(reductions.saw_total, interval)
    })
}
fn deletion_state(
    reductions: &Reductions,
    states: &[State],
    i: usize,
    counter: &mut Counter<'_>,
) -> AppResult<(State, Interval)> {
    counter.checkpoint()?;
    let mut constant = false;
    for (j, state) in states.iter().enumerate() {
        counter.classify()?;
        constant |= j != i && *state == State::ExactZero;
    }
    counter.checkpoint()?;
    let mut unresolved = false;
    for (j, state) in states.iter().enumerate() {
        counter.classify()?;
        unresolved |= j != i && *state != State::Positive;
    }
    counter.checkpoint()?;
    let interval = reductions.deleted[i].value(0)?;
    let state = if states.len() == 2 || constant {
        State::Unavailable
    } else if reductions.bad || unresolved {
        State::Unresolved
    } else {
        energy_state(reductions.saw_deleted[i], interval)
    };
    Ok((state, interval))
}
pub(super) fn publish(
    reductions: &Reductions,
    states: &[State],
    counter: &mut Counter<'_>,
) -> AppResult<Publications> {
    let k = states.len();
    let interval = reductions.total.value(0)?;
    let state = full_state(reductions, states, interval, counter)?;
    let alpha = publish_alpha(exact(k as f64), interval, k, state);
    let mut deleted_states = Vec::with_capacity(k);
    let mut deleted = Vec::with_capacity(k);
    for i in 0..k {
        let (state, interval) = deletion_state(reductions, states, i, counter)?;
        deleted_states.push(state);
        deleted.push(publish_alpha(exact((k - 1) as f64), interval, k - 1, state));
    }
    Ok(Publications {
        alpha,
        deleted,
        deleted_states,
        state,
    })
}
