//! Fixed captured comparison membership and scalar dependencies.
use super::*;
fn member_present(r: &ComputedAnova, p: &AnovaPairRequest, c: &JobControl) -> AppResult<bool> {
    let contains = |factor: usize, value: &Cell| -> AppResult<bool> {
        for v in &r.grid.factors[factor].values {
            if compare_cells(v, value, c)? == Ordering::Equal {
                return Ok(true);
            }
        }
        Ok(false)
    };
    if !contains(p.factor, &p.first)? || !contains(p.factor, &p.second)? {
        return Ok(false);
    }
    if let AnovaScope::Fixed(values) = &p.scope {
        let mut j = 0;
        for i in 0..r.grid.factors.len() {
            if i != p.factor {
                if !contains(i, &values[j])? {
                    return Ok(false);
                }
                j += 1;
            }
        }
    }
    Ok(true)
}
fn same_pair(a: &AnovaPairRequest, b: &AnovaPairRequest, c: &JobControl) -> AppResult<bool> {
    Ok(a.factor == b.factor
        && scope_order(&a.scope, &b.scope, c)? == Ordering::Equal
        && compare_cells(&a.first, &b.first, c)? == Ordering::Equal
        && compare_cells(&a.second, &b.second, c)? == Ordering::Equal)
}
pub(super) fn families(r: &ComputedAnova, c: &JobControl) -> AppResult<()> {
    let planned = usize::from(!r.request.planned_pairs.is_empty());
    check(r.families.len() == planned + r.request.all_pairs.len())?;
    for (i, f) in r.families.iter().enumerate() {
        c.checkpoint(i, None)?;
        check(f.m == f.members.len())?;
        let mut expected = Vec::new();
        if i < planned {
            check(f.kind == AnovaFamilyKind::Planned && f.m == r.request.planned_pairs.len())?;
            for (j, p) in r.request.planned_pairs.iter().enumerate() {
                check(same_pair(p, &f.members[j].request, c)?)?;
            }
        } else {
            let request = &r.request.all_pairs[i - planned];
            let AnovaFamilyKind::AllPairs(saved) = &f.kind else {
                return Err(invalid_analysis());
            };
            check(
                saved.factor == request.factor
                    && scope_order(&saved.scope, &request.scope, c)? == Ordering::Equal,
            )?;
            let levels = &r.grid.factors[request.factor].values;
            let m = levels
                .len()
                .checked_mul(levels.len().saturating_sub(1))
                .ok_or_else(invalid_analysis)?
                / 2;
            check(m == f.m)?;
            for a in 0..levels.len() {
                for b in a + 1..levels.len() {
                    expected.push((a, b));
                }
            }
            for (j, (a, b)) in expected.iter().enumerate() {
                let p = &f.members[j].request;
                check(
                    p.factor == request.factor
                        && scope_order(&p.scope, &request.scope, c)? == Ordering::Equal
                        && compare_cells(&p.first, &levels[*a], c)? == Ordering::Equal
                        && compare_cells(&p.second, &levels[*b], c)? == Ordering::Equal,
                )?;
            }
        }
        let raw_complete = f.members.iter().all(|m| m.raw_p.value.is_some());
        if raw_complete {
            check(f.adjustment_failure.is_none())?;
        } else {
            let reason = f.adjustment_failure.as_ref().ok_or_else(invalid_analysis)?;
            failure(reason, c)?;
            check(reason.kind == AnovaFailureKind::IncompleteFamily)?;
        }
        for (ordinal, m) in f.members.iter().enumerate() {
            c.checkpoint(ordinal, None)?;
            check(m.ordinal == ordinal)?;
            comparison(r, m, i < planned, c)?;
            if !raw_complete {
                check(m.adjusted_p.value.is_none())?;
            } else {
                check(
                    m.adjusted_p
                        .value
                        .is_some_and(|p| p >= m.raw_p.value.unwrap()),
                )?;
            }
        }
        if raw_complete {
            let mut order = (0..f.m).collect::<Vec<_>>();
            let mut scratch = vec![0; f.m];
            sort_indices(&mut order, &mut scratch, c, |a, b| {
                Ok(f.members[a]
                    .raw_p
                    .value
                    .unwrap()
                    .total_cmp(&f.members[b].raw_p.value.unwrap())
                    .then(a.cmp(&b)))
            })?;
            let mut adjusted = 0f64;
            for (j, index) in order.iter().enumerate() {
                c.checkpoint(j, None)?;
                adjusted = adjusted
                    .max((f.m - j) as f64 * f.members[*index].raw_p.value.unwrap())
                    .min(1.);
                check(f.members[*index].adjusted_p.value == Some(adjusted))?;
            }
        }
    }
    Ok(())
}
fn comparison(
    r: &ComputedAnova,
    m: &AnovaComparison,
    planned: bool,
    c: &JobControl,
) -> AppResult<()> {
    for e in [&m.estimate, &m.t, &m.interval[0], &m.interval[1]] {
        estimate(e, c)?;
    }
    bounded(&m.standard_error, 0., f64::MAX, c)?;
    bounded(&m.df, 0., crate::analysis::MAX_FREQUENCY as f64, c)?;
    for e in [&m.raw_p, &m.adjusted_p] {
        bounded(e, f64::MIN_POSITIVE, 1., c)?;
    }
    let present = member_present(r, &m.request, c)?;
    if matches!(r.outcome, AnovaOutcome::Unavailable(_)) {
        check(!matches!(m.state, AnovaComparisonState::Available))?;
        check(
            [
                &m.standard_error,
                &m.t,
                &m.raw_p,
                &m.adjusted_p,
                &m.interval[0],
                &m.interval[1],
            ]
            .iter()
            .all(|e| e.value.is_none()),
        )?;
    }

    if let Some(df) = m.df.value {
        let expected = if matches!(r.request.design, AnovaDesign::Repeated { .. }) {
            r.frequency.saturating_sub(1)
        } else {
            r.frequency.saturating_sub(r.grid.cells.len() as u64)
        };
        check(df == expected as f64)?;
    }
    match m.variance {
        AnovaQuantityState::Positive => check(m.standard_error.value.is_none_or(|v| v > 0.))?,
        AnovaQuantityState::Unavailable(_) => check(
            [
                &m.standard_error,
                &m.t,
                &m.raw_p,
                &m.interval[0],
                &m.interval[1],
            ]
            .iter()
            .all(|e| e.value.is_none()),
        )?,
        AnovaQuantityState::ExactZero => {}
    }

    match &m.state {
        AnovaComparisonState::Available => {
            check(present)?;
        }
        AnovaComparisonState::AbsentRequestedLevel(f) => {
            failure(f, c)?;
            check(
                !matches!(r.request.design, AnovaDesign::Repeated { .. })
                    && !present
                    && f.kind == AnovaFailureKind::AbsentRequestedLevel,
            )?;
            if !planned {
                let values = &r.grid.factors[m.request.factor].values;
                let mut first = false;
                let mut second = false;
                for v in values {
                    first |= compare_cells(v, &m.request.first, c)? == Ordering::Equal;
                    second |= compare_cells(v, &m.request.second, c)? == Ordering::Equal;
                }
                check(first && second && matches!(m.request.scope, AnovaScope::Fixed(_)))?;
            }
        }
        AnovaComparisonState::Unavailable(f) => {
            failure(f, c)?;
            check(present && f.kind != AnovaFailureKind::AbsentRequestedLevel)?;
            check(
                [
                    &m.standard_error,
                    &m.t,
                    &m.raw_p,
                    &m.adjusted_p,
                    &m.interval[0],
                    &m.interval[1],
                ]
                .iter()
                .all(|e| e.value.is_none()),
            )?;
        }
    }
    if !present {
        check(
            [
                &m.estimate,
                &m.standard_error,
                &m.t,
                &m.df,
                &m.raw_p,
                &m.adjusted_p,
                &m.interval[0],
                &m.interval[1],
            ]
            .iter()
            .all(|e| e.value.is_none()),
        )?;
    }
    check(
        m.raw_p.value.is_none()
            || (m.t.value.is_some()
                && m.df.value.is_some_and(|v| v > 0.)
                && matches!(m.variance, AnovaQuantityState::Positive)),
    )?;
    if matches!(m.variance, AnovaQuantityState::ExactZero) {
        check(
            m.standard_error.value == Some(0.)
                && m.t.value.is_none()
                && m.raw_p.value.is_none()
                && m.interval[0].value == m.estimate.value
                && m.interval[1].value == m.estimate.value,
        )?;
    }
    if let (Some(lo), Some(hi)) = (m.interval[0].value, m.interval[1].value) {
        check(lo <= hi && m.estimate.value.is_some_and(|v| v >= lo && v <= hi))?;
        let centre = m.estimate.value.ok_or_else(invalid_analysis)?;
        let scale = lo.abs().max(hi.abs()).max(centre.abs());
        if scale > 0. {
            let magnitude = (lo / scale).abs() + (hi / scale).abs() + 2. * (centre / scale).abs();
            check(
                ((lo / scale + hi / scale) / 2. - centre / scale).abs()
                    <= 8. * f64::EPSILON * magnitude,
            )?;
        }
    } else {
        check(m.interval[0].value.is_none() && m.interval[1].value.is_none())?;
    }
    Ok(())
}
