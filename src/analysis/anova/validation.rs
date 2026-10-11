//! Controlled semantic admission against current or captured column definitions.
use super::*;
use crate::{
    analysis::{
        exploration::ordering::{compare_cells, compare_text, sort_indices},
        validation::invalid_analysis,
        RowPredicate,
    },
    data::{Cell, ColumnId, ColumnKind},
    error::AppResult,
    operation::JobControl,
};
use std::{cmp::Ordering, collections::BTreeSet};
pub(super) fn nonblank(s: &str, c: &JobControl) -> AppResult<bool> {
    let mut nonblank = false;
    let mut next = 0;
    for (i, ch) in s.char_indices() {
        if i >= next {
            c.checkpoint(i, None)?;
            next = i.saturating_add(4096);
        }
        nonblank |= !ch.is_whitespace();
    }
    c.check_cancelled()?;
    Ok(nonblank)
}
fn scan(s: &str, c: &JobControl) -> AppResult<()> {
    for i in (0..s.len()).step_by(4096) {
        c.checkpoint(i, None)?;
    }
    c.check_cancelled()
}
fn typed(v: &Cell, k: ColumnKind, c: &JobControl) -> AppResult<()> {
    match (v, k) {
        (Cell::Number(n), ColumnKind::Number) if n.is_finite() => Ok(()),
        (Cell::Text(s), ColumnKind::Text) => scan(s, c),
        _ => Err(invalid_analysis()),
    }
}
pub(crate) fn request(
    r: &AnovaRequest,
    mut kind: impl FnMut(ColumnId) -> AppResult<ColumnKind>,
    control: &JobControl,
) -> AppResult<BTreeSet<ColumnId>> {
    let c = control.without_progress();
    let c = &c;
    c.checkpoint(0, None)?;
    r.admit()?;
    let mut ids = BTreeSet::new();
    let mut role = |id: ColumnId, numeric: bool| -> AppResult<ColumnKind> {
        c.checkpoint(0, None)?;
        if id.0 == 0 || !ids.insert(id) {
            return Err(invalid_analysis());
        }
        let k = kind(id)?;
        if numeric && k != ColumnKind::Number {
            return Err(invalid_analysis());
        }
        Ok(k)
    };
    let mut kinds = Vec::new();
    match &r.design {
        AnovaDesign::OneWay { outcome, factor } => {
            role(*outcome, true)?;
            kinds.push(role(*factor, false)?);
        }
        AnovaDesign::Factorial { outcome, factors } => {
            if !(2..=6).contains(&factors.len()) {
                return Err(invalid_analysis());
            }
            role(*outcome, true)?;
            for id in factors {
                kinds.push(role(*id, false)?);
            }
        }
        AnovaDesign::Repeated {
            subject,
            factors,
            measurements,
        } => {
            if r.frequency_weight.is_some() || factors.is_empty() || factors.len() > 6 {
                return Err(invalid_analysis());
            }
            role(*subject, false)?;
            let mut grid = 1usize;
            for (i, f) in factors.iter().enumerate() {
                if !nonblank(&f.name, c)? || f.levels.len() < 2 {
                    return Err(invalid_analysis());
                }
                grid = grid
                    .checked_mul(f.levels.len())
                    .filter(|n| *n <= MAX_GRID)
                    .ok_or_else(invalid_analysis)?;
                for previous in &factors[..i] {
                    if compare_text(&previous.name, &f.name, c)? == Ordering::Equal {
                        return Err(invalid_analysis());
                    }
                }
                for (j, s) in f.levels.iter().enumerate() {
                    if !nonblank(s, c)? {
                        return Err(invalid_analysis());
                    }
                    for previous in &f.levels[..j] {
                        if compare_text(previous, s, c)? == Ordering::Equal {
                            return Err(invalid_analysis());
                        }
                    }
                }
                kinds.push(ColumnKind::Text);
            }
            if measurements.len() != grid {
                return Err(invalid_analysis());
            }
            for id in measurements {
                role(*id, true)?;
            }
        }
    }
    drop(role);
    for (i, p) in r.filter.predicates.iter().enumerate() {
        c.checkpoint(i, None)?;
        let id = p.column();
        if id.0 == 0 {
            return Err(invalid_analysis());
        }
        let k = kind(id)?;
        match p {
            RowPredicate::Number { value, .. } if k != ColumnKind::Number || !value.is_finite() => {
                return Err(invalid_analysis())
            }
            RowPredicate::Text { value, .. } => {
                if k != ColumnKind::Text {
                    return Err(invalid_analysis());
                }
                scan(value, c)?;
            }
            _ => {}
        }
        ids.insert(id);
    }
    if let Some(id) = r.frequency_weight {
        if id.0 == 0 || kind(id)? != ColumnKind::Number {
            return Err(invalid_analysis());
        }
        ids.insert(id);
    }
    for (i, p) in r.planned_pairs.iter().enumerate() {
        c.checkpoint(i, None)?;
        target(p.factor, &p.scope, &kinds, &r.design, c)?;
        typed(&p.first, kinds[p.factor], c)?;
        typed(&p.second, kinds[p.factor], c)?;
        if compare_cells(&p.first, &p.second, c)? == Ordering::Equal {
            return Err(invalid_analysis());
        }
        declared(p.factor, &p.first, &r.design, c)?;
        declared(p.factor, &p.second, &r.design, c)?;
    }
    for (i, f) in r.all_pairs.iter().enumerate() {
        c.checkpoint(i, None)?;
        target(f.factor, &f.scope, &kinds, &r.design, c)?;
    }
    unique(r.planned_pairs.len(), c, |a, b| {
        pair_order(&r.planned_pairs[a], &r.planned_pairs[b], c)
    })?;
    unique(r.all_pairs.len(), c, |a, b| {
        let a = &r.all_pairs[a];
        let b = &r.all_pairs[b];
        let o = a.factor.cmp(&b.factor);
        if o == Ordering::Equal {
            scope_order(&a.scope, &b.scope, c)
        } else {
            Ok(o)
        }
    })?;
    Ok(ids)
}
fn declared(factor: usize, value: &Cell, d: &AnovaDesign, c: &JobControl) -> AppResult<()> {
    if let AnovaDesign::Repeated { factors, .. } = d {
        let Cell::Text(s) = value else {
            return Err(invalid_analysis());
        };
        for v in &factors[factor].levels {
            if compare_text(s, v, c)? == Ordering::Equal {
                return Ok(());
            }
        }
        return Err(invalid_analysis());
    }
    Ok(())
}
fn target(
    factor: usize,
    scope: &AnovaScope,
    kinds: &[ColumnKind],
    d: &AnovaDesign,
    c: &JobControl,
) -> AppResult<()> {
    if factor >= kinds.len() {
        return Err(invalid_analysis());
    }
    if let AnovaScope::Fixed(v) = scope {
        if v.len() != kinds.len() - 1 {
            return Err(invalid_analysis());
        }
        let mut ordinal = 0;
        for (i, k) in kinds.iter().enumerate() {
            if i != factor {
                typed(&v[ordinal], *k, c)?;
                declared(i, &v[ordinal], d, c)?;
                ordinal += 1;
            }
        }
    }
    Ok(())
}
pub(super) fn scope_order(a: &AnovaScope, b: &AnovaScope, c: &JobControl) -> AppResult<Ordering> {
    match (a, b) {
        (AnovaScope::Marginal, AnovaScope::Marginal) => Ok(Ordering::Equal),
        (AnovaScope::Marginal, _) => Ok(Ordering::Less),
        (_, AnovaScope::Marginal) => Ok(Ordering::Greater),
        (AnovaScope::Fixed(a), AnovaScope::Fixed(b)) => {
            for (a, b) in a.iter().zip(b) {
                let o = compare_cells(a, b, c)?;
                if o != Ordering::Equal {
                    return Ok(o);
                }
            }
            Ok(a.len().cmp(&b.len()))
        }
    }
}
fn pair_order(a: &AnovaPairRequest, b: &AnovaPairRequest, c: &JobControl) -> AppResult<Ordering> {
    let o = a.factor.cmp(&b.factor);
    if o != Ordering::Equal {
        return Ok(o);
    }
    let o = scope_order(&a.scope, &b.scope, c)?;
    if o != Ordering::Equal {
        return Ok(o);
    }
    let ordered = |p: &AnovaPairRequest| -> AppResult<bool> {
        Ok(compare_cells(&p.first, &p.second, c)? == Ordering::Less)
    };
    let (a1, a2) = if ordered(a)? {
        (&a.first, &a.second)
    } else {
        (&a.second, &a.first)
    };
    let (b1, b2) = if ordered(b)? {
        (&b.first, &b.second)
    } else {
        (&b.second, &b.first)
    };
    let o = compare_cells(a1, b1, c)?;
    if o != Ordering::Equal {
        return Ok(o);
    }
    compare_cells(a2, b2, c)
}
fn unique(
    n: usize,
    c: &JobControl,
    cmp: impl Fn(usize, usize) -> AppResult<Ordering>,
) -> AppResult<()> {
    let mut indices = (0..n).collect::<Vec<_>>();
    let mut scratch = vec![0; n];
    sort_indices(&mut indices, &mut scratch, c, &cmp)?;
    for p in indices.windows(2) {
        if cmp(p[0], p[1])? == Ordering::Equal {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}

fn check(ok: bool) -> AppResult<()> {
    if ok {
        Ok(())
    } else {
        Err(invalid_analysis())
    }
}
fn estimate(e: &crate::analysis::Estimate, c: &JobControl) -> AppResult<()> {
    match (&e.value, &e.reason) {
        (Some(v), None) => check(v.is_finite()),
        (None, Some(s)) => check(nonblank(s, c)?),
        _ => Err(invalid_analysis()),
    }
}
fn bounded(e: &crate::analysis::Estimate, lo: f64, hi: f64, c: &JobControl) -> AppResult<()> {
    estimate(e, c)?;
    check(e.value.is_none_or(|v| v >= lo && v <= hi))
}
fn failure(f: &AnovaFailure, c: &JobControl) -> AppResult<()> {
    check(nonblank(&f.reason, c)?)
}
fn energy(e: &AnovaEnergy, c: &JobControl) -> AppResult<()> {
    bounded(&e.value, 0., f64::MAX, c)?;
    match e.state {
        AnovaQuantityState::ExactZero => check(e.value.value == Some(0.)),
        AnovaQuantityState::Positive => check(e.value.value.is_none_or(|v| v > 0.)),
        AnovaQuantityState::Unavailable(_) => check(e.value.value.is_none()),
    }
}
fn test(t: &AnovaTest, c: &JobControl) -> AppResult<()> {
    bounded(&t.df1, 0., (1u64 << 53) as f64, c)?;
    bounded(&t.df2, 0., (1u64 << 53) as f64, c)?;
    bounded(&t.f, 0., f64::MAX, c)?;
    bounded(&t.p, f64::MIN_POSITIVE, 1., c)?;
    check(
        t.p.value.is_none()
            || (t.df1.value.is_some_and(|v| v > 0.) && t.df2.value.is_some_and(|v| v > 0.)),
    )?;
    if t.f.value == Some(0.) && t.p.value.is_some() {
        check(t.p.value == Some(1.))?;
    }
    Ok(())
}
fn count(rows: usize, n: u64, weighted: bool) -> AppResult<()> {
    check(
        n <= crate::analysis::MAX_FREQUENCY
            && n >= rows as u64
            && (rows == 0) == (n == 0)
            && (weighted || n == rows as u64),
    )
}
fn snapshot(info: &crate::analysis::RunColumnInfo, c: &JobControl) -> AppResult<()> {
    use crate::data::MissingRules;
    check(info.id.0 != 0 && nonblank(&info.name, c)?)?;
    for (i, ch) in info.name.char_indices() {
        if i % 4096 < 4 {
            c.checkpoint(i, None)?;
        }
        check(!ch.is_control())?;
    }
    scan(&info.description, c)?;
    match (&info.missing_rules, info.kind) {
        (MissingRules::None, _) => {}
        (MissingRules::Number { values, ranges }, ColumnKind::Number) => {
            for (i, v) in values.iter().enumerate() {
                c.checkpoint(i, None)?;
                check(v.is_finite())?;
            }
            for (i, (lo, hi)) in ranges.iter().enumerate() {
                c.checkpoint(i, None)?;
                check(lo.is_finite() && hi.is_finite() && lo <= hi)?;
            }
        }
        (MissingRules::Text(values), ColumnKind::Text) => {
            for s in values {
                scan(s, c)?;
            }
        }
        _ => return Err(invalid_analysis()),
    }
    for label in &info.value_labels {
        typed(&label.value, info.kind, c)?;
        scan(&label.label, c)?;
    }
    unique(info.value_labels.len(), c, |a, b| {
        compare_cells(&info.value_labels[a].value, &info.value_labels[b].value, c)
    })
}
fn present(v: &Cell, info: &crate::analysis::RunColumnInfo, c: &JobControl) -> AppResult<()> {
    use crate::data::MissingRules;
    typed(v, info.kind, c)?;
    match (&info.missing_rules, v) {
        (MissingRules::Number { values, ranges }, Cell::Number(n)) => {
            for (i, v) in values.iter().enumerate() {
                c.checkpoint(i, None)?;
                check(v != n)?;
            }
            for (i, (lo, hi)) in ranges.iter().enumerate() {
                c.checkpoint(i, None)?;
                check(n < lo || n > hi)?;
            }
        }
        (MissingRules::Text(values), Cell::Text(s)) => {
            for v in values {
                check(compare_text(s, v, c)? != Ordering::Equal)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn completed(r: &ComputedAnova, control: &JobControl) -> AppResult<()> {
    let quiet = control.without_progress();
    let c = &quiet;
    c.checkpoint(0, None)?;
    r.request.admit()?;
    check(
        r.revision.0 != 0
            && r.source_rows <= crate::data::MAX_CELLS
            && r.method
                == AnovaMethod {
                    numerical_version: 1,
                    basis_version: 1,
                },
    )?;
    let mut previous = None;
    for info in &r.columns {
        c.checkpoint(0, None)?;
        check(previous.is_none_or(|id| id < info.id))?;
        previous = Some(info.id);
        snapshot(info, c)?;
    }
    let info = |id: ColumnId| {
        r.columns
            .binary_search_by_key(&id, |i| i.id)
            .map(|i| &r.columns[i])
            .map_err(|_| invalid_analysis())
    };
    let ids = request(&r.request, |id| Ok(info(id)?.kind), c)?;
    check(ids.len() == r.columns.len())?;
    let weighted = r.request.frequency_weight.is_some();
    let repeated = matches!(r.request.design, AnovaDesign::Repeated { .. });
    let d = &r.diagnostics;
    check(
        [
            d.filtered_rows,
            d.missing_weight_rows,
            d.zero_weight_rows,
            d.complete_case_rows,
            r.fitted_rows,
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
            == Some(r.source_rows),
    )?;
    check(weighted || (d.missing_weight_rows == 0 && d.zero_weight_rows == 0))?;
    count(r.fitted_rows, r.frequency, weighted)?;
    count(d.complete_case_rows, r.incomplete_case_frequency, weighted)?;
    check(
        r.frequency
            .checked_add(r.incomplete_case_frequency)
            .is_some_and(|n| n <= crate::analysis::MAX_FREQUENCY),
    )?;
    let factor_ids = match &r.request.design {
        AnovaDesign::OneWay { factor, .. } => vec![*factor],
        AnovaDesign::Factorial { factors, .. } => factors.clone(),
        AnovaDesign::Repeated { .. } => vec![],
    };
    let factor_count = if let AnovaDesign::Repeated { factors, .. } = &r.request.design {
        factors.len()
    } else {
        factor_ids.len()
    };
    check(r.grid.factors.len() == factor_count)?;
    let mut grid = 1usize;
    for (ordinal, f) in r.grid.factors.iter().enumerate() {
        c.checkpoint(ordinal, None)?;
        check(nonblank(&f.name, c)?)?;
        if let AnovaDesign::Repeated { factors, .. } = &r.request.design {
            let declared = &factors[ordinal];
            check(
                f.column.is_none()
                    && compare_text(&f.name, &declared.name, c)? == Ordering::Equal
                    && f.values.len() == declared.levels.len(),
            )?;
            for (a, b) in f.values.iter().zip(&declared.levels) {
                let Cell::Text(a) = a else {
                    return Err(invalid_analysis());
                };
                check(compare_text(a, b, c)? == Ordering::Equal)?;
            }
        } else {
            let id = factor_ids[ordinal];
            let column = info(id)?;
            check(
                f.column == Some(id) && compare_text(&f.name, &column.name, c)? == Ordering::Equal,
            )?;
            check(f.values.len() <= MAX_GRID)?;
            for (i, v) in f.values.iter().enumerate() {
                present(v, column, c)?;
                check(!matches!(v,Cell::Number(n) if *n==0.&&n.to_bits()!=0))?;
                if i > 0 {
                    check(compare_cells(&f.values[i - 1], v, c)? == Ordering::Less)?;
                }
            }
        }
        grid = grid
            .checked_mul(f.values.len())
            .filter(|n| *n <= MAX_GRID)
            .ok_or_else(invalid_analysis)?;
    }
    check(r.grid.cells.len() == grid)?;
    check(
        r.observed_measurements
            == if repeated {
                r.fitted_rows
                    .checked_mul(grid)
                    .ok_or_else(invalid_analysis)?
            } else {
                r.fitted_rows
            },
    )?;
    let mut physical = 0usize;
    let mut frequency = 0u64;
    for cell in &r.grid.cells {
        c.checkpoint(physical, None)?;
        count(cell.physical_rows, cell.frequency, weighted)?;
        estimate(&cell.mean, c)?;
        if repeated {
            check(cell.physical_rows == r.fitted_rows && cell.frequency == r.frequency)?;
        } else {
            physical = physical
                .checked_add(cell.physical_rows)
                .ok_or_else(invalid_analysis)?;
            frequency = frequency
                .checked_add(cell.frequency)
                .ok_or_else(invalid_analysis)?;
        }
        if cell.frequency == 0 {
            check(cell.mean.value.is_none())?;
        }
    }
    if !repeated {
        check(physical == r.fitted_rows && frequency == r.frequency)?;
    }
    let complete =
        grid > 0 && r.frequency > 0 && r.grid.cells.iter().all(|cell| cell.frequency > 0);
    check(r.grid.complete == complete)?;
    let collapsed = r.grid.factors.iter().any(|f| f.values.len() < 2);
    match &r.outcome {
        AnovaOutcome::Unavailable(f) => {
            failure(f, c)?;
            match f.kind {
                AnovaFailureKind::EmptyCohort => check(r.fitted_rows == 0)?,
                AnovaFailureKind::CollapsedFactor => check(r.fitted_rows > 0 && collapsed)?,
                AnovaFailureKind::EmptyCells => {
                    check(r.fitted_rows > 0 && !collapsed && !complete)?
                }
                AnovaFailureKind::Numerical | AnovaFailureKind::Nonconvergence => {
                    check(r.fitted_rows > 0 && !collapsed && complete)?
                }
                _ => return Err(invalid_analysis()),
            }
        }
        AnovaOutcome::Fitted(m) => {
            check(r.fitted_rows > 0 && !collapsed && complete)?;
            model(r, m, repeated, c)?;
            scalar_relationships(r, m, repeated, c)?;
        }
    }
    families(r, c)
}
fn model(r: &ComputedAnova, m: &AnovaModelResult, repeated: bool, c: &JobControl) -> AppResult<()> {
    let k = r.grid.cells.len();
    let n = r.frequency;
    let effects = (1usize << r.grid.factors.len()) - 1;
    check(
        m.effects.len() == effects
            && m.total_df
                == if repeated {
                    r.observed_measurements.saturating_sub(1) as u64
                } else {
                    n - 1
                },
    )?;
    check(
        m.residual_df
            == if repeated {
                n.saturating_sub(1)
            } else {
                n.saturating_sub(k as u64)
            },
    )?;
    for e in [&m.error_ss, &m.model_ss, &m.total_ss] {
        energy(e, c)?;
    }
    check(repeated == m.subject_ss.is_some())?;
    if let Some(e) = &m.subject_ss {
        energy(e, c)?;
    }
    if !repeated {
        check(
            m.balanced
                == r.grid
                    .cells
                    .windows(2)
                    .all(|v| v[0].frequency == v[1].frequency),
        )?;
    }
    for (ordinal, e) in m.effects.iter().enumerate() {
        c.checkpoint(ordinal, None)?;
        let mask = ordinal + 1;
        let expected = (0..r.grid.factors.len())
            .filter(|i| mask & (1 << i) != 0)
            .collect::<Vec<_>>();
        check(e.factors == expected)?;
        let rank = e
            .factors
            .iter()
            .try_fold(1usize, |n, i| {
                n.checked_mul(r.grid.factors[*i].values.len() - 1)
            })
            .ok_or_else(invalid_analysis)?;
        check(e.dimension == rank)?;
        energy(&e.ss, c)?;
        energy(&e.error_ss, c)?;
        if matches!(e.ss.state, AnovaQuantityState::Unavailable(_)) {
            check(
                e.ms.value.is_none()
                    && e.eta_squared.value.is_none()
                    && e.partial_eta_squared.value.is_none(),
            )?;
        }
        if matches!(e.error_ss.state, AnovaQuantityState::Unavailable(_)) {
            check(e.error_ms.value.is_none() && e.partial_eta_squared.value.is_none())?;
        }
        if matches!(m.total_ss.state, AnovaQuantityState::Unavailable(_)) {
            check(e.eta_squared.value.is_none())?;
        }
        if matches!(e.ss.state, AnovaQuantityState::Positive) {
            check(e.ms.value.is_none_or(|v| v > 0.))?;
        }
        if matches!(e.error_ss.state, AnovaQuantityState::Positive) {
            check(e.error_ms.value.is_none_or(|v| v > 0.))?;
        }

        for value in [&e.ms, &e.error_ms] {
            bounded(value, 0., f64::MAX, c)?;
        }
        for value in [&e.eta_squared, &e.partial_eta_squared] {
            bounded(value, 0., 1., c)?;
        }
        test(&e.test, c)?;
        let df2 = if repeated {
            (n - 1)
                .checked_mul(rank as u64)
                .ok_or_else(invalid_analysis)?
        } else {
            m.residual_df
        };
        check(e.test.df1.value == Some(rank as f64) && e.test.df2.value == Some(df2 as f64))?;
        if let Some(f) = &e.failure {
            failure(f, c)?;
        }
        let inference_unavailable = matches!(
            e.error_ss.state,
            AnovaQuantityState::ExactZero | AnovaQuantityState::Unavailable(_)
        ) || matches!(e.ss.state, AnovaQuantityState::Unavailable(_))
            || df2 == 0;
        if inference_unavailable {
            check(e.test.f.value.is_none() && e.test.p.value.is_none())?;
        }
        if df2 == 0 {
            check(e.error_ms.value.is_none())?;
        }
        if matches!(e.ss.state, AnovaQuantityState::ExactZero)
            && matches!(e.error_ss.state, AnovaQuantityState::Positive)
            && df2 > 0
        {
            check(
                e.test.f.value.is_none_or(|v| v == 0.) && e.test.p.value.is_none_or(|v| v == 1.),
            )?;
        }
        check(repeated == e.gg.is_some())?;
        if repeated {
            check(e.eta_squared.value.is_none())?;
        }
        if let Some(g) = &e.gg {
            bounded(
                &g.epsilon,
                1. / rank as f64,
                if rank == 1 {
                    1.
                } else {
                    1f64.min((n - 1) as f64 / rank as f64)
                },
                c,
            )?;
            test(&g.test, c)?;
            if inference_unavailable {
                check(g.test.f.value.is_none() && g.test.p.value.is_none())?;
            }
            if let Some(f) = &g.failure {
                failure(f, c)?;
            }
            if rank == 1 {
                check(g.epsilon.value == Some(1.))?;
            }
            if n == 2 && matches!(g.covariance, AnovaQuantityState::Positive) {
                check(g.epsilon.value == Some(1. / rank as f64))?;
            }
            if let Some(epsilon) = g.epsilon.value {
                let corrected_df1 = if rank == 1 || n == 2 {
                    1.
                } else {
                    epsilon * rank as f64
                };
                let corrected_df2 = (n - 1) as f64 * corrected_df1;
                check(
                    g.test.df1.value == Some(corrected_df1)
                        && g.test.df2.value == Some(corrected_df2),
                )?;
            } else {
                check(
                    g.test.df1.value.is_none()
                        && g.test.df2.value.is_none()
                        && g.test.p.value.is_none(),
                )?;
            }
            check(g.test.f == e.test.f)?;
            if matches!(
                g.covariance,
                AnovaQuantityState::ExactZero | AnovaQuantityState::Unavailable(_)
            ) && rank > 1
            {
                check(g.epsilon.value.is_none() && g.test.p.value.is_none())?;
            }
        }
    }
    Ok(())
}

mod families;
mod numerical;
use families::families;
use numerical::scalar_relationships;
#[cfg(test)]
mod tests;
