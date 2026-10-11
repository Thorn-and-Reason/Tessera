//! Validate captured facts independently of the active dataset and numerical kernel.
use super::*;
use crate::{
    analysis::{
        exploration::numeric::display_index,
        validation::{invalid_analysis, validate_filter_weight, validate_snapshot},
        Estimate, RunColumnInfo, MAX_FREQUENCY, MAX_PLOT_POINTS,
    },
    data::{Cell, ColumnId, ColumnKind, MissingRules, MAX_CELLS},
    error::AppResult,
};
mod numerical;

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

pub(crate) fn request(
    r: &RegressionRequest,
    kind: impl Fn(ColumnId) -> AppResult<ColumnKind>,
) -> AppResult<BTreeSet<ColumnId>> {
    if r.predictors.is_empty() || r.outcome.0 == 0 || kind(r.outcome)? != ColumnKind::Number {
        return Err(invalid_analysis());
    }
    let mut ids = BTreeSet::from([r.outcome]);
    for predictor in &r.predictors {
        if predictor.column.0 == 0 || !ids.insert(predictor.column) {
            return Err(invalid_analysis());
        }
        let actual = kind(predictor.column)?;
        match &predictor.role {
            PredictorRole::Numeric if actual != ColumnKind::Number => {
                return Err(invalid_analysis())
            }
            PredictorRole::Categorical { reference } => typed(reference, actual)?,
            _ => {}
        }
    }
    validate_filter_weight(&r.filter, r.frequency_weight, kind, &mut ids)?;
    Ok(ids)
}
fn typed(cell: &Cell, kind: ColumnKind) -> AppResult<()> {
    if matches!((cell, kind), (Cell::Number(v), ColumnKind::Number) if v.is_finite())
        || matches!((cell, kind), (Cell::Text(_), ColumnKind::Text))
    {
        Ok(())
    } else {
        Err(invalid_analysis())
    }
}
fn compare(a: &Cell, b: &Cell) -> Option<Ordering> {
    match (a, b) {
        (Cell::Number(a), Cell::Number(b)) => a.partial_cmp(b),
        (Cell::Text(a), Cell::Text(b)) => Some(a.cmp(b)),
        _ => None,
    }
}
fn present(cell: &Cell, info: &RunColumnInfo) -> AppResult<()> {
    typed(cell, info.kind)?;
    let missing = match (&info.missing_rules, cell) {
        (MissingRules::Number { values, ranges }, Cell::Number(v)) => {
            values.contains(v) || ranges.iter().any(|(lo, hi)| lo <= v && v <= hi)
        }
        (MissingRules::Text(values), Cell::Text(v)) => values.contains(v),
        _ => false,
    };
    if missing {
        Err(invalid_analysis())
    } else {
        Ok(())
    }
}
pub(super) fn completed(r: &ComputedRegression) -> AppResult<()> {
    if r.revision.0 == 0
        || r.source_rows > MAX_CELLS
        || r.method.numerical_version != 1
        || r.method.rank_threshold != 1e-12
        || r.method.condition_warning != 1e8
    {
        return Err(invalid_analysis());
    }
    if r.columns.windows(2).any(|pair| pair[0].id >= pair[1].id) {
        return Err(invalid_analysis());
    }
    let infos = r
        .columns
        .iter()
        .map(|c| (c.id, c))
        .collect::<BTreeMap<_, _>>();
    if infos.len() != r.columns.len() {
        return Err(invalid_analysis());
    }
    for info in &r.columns {
        validate_snapshot(info)?;
    }
    let ids = request(&r.request, |id| {
        infos.get(&id).map(|c| c.kind).ok_or_else(invalid_analysis)
    })?;
    if !ids.iter().eq(infos.keys()) {
        return Err(invalid_analysis());
    }
    cohort(r)?;
    let failure = match &r.outcome {
        RegressionOutcome::Unavailable(f) => Some(f),
        _ => None,
    };
    expansion(r, &infos, failure)?;
    match &r.outcome {
        RegressionOutcome::Unavailable(f) => failed(r, f),
        RegressionOutcome::Fitted(f) => fit(r, f),
    }
}
fn count(rows: usize, n: u64, weighted: bool) -> AppResult<()> {
    if rows > MAX_CELLS
        || n > MAX_FREQUENCY
        || (rows == 0) != (n == 0)
        || n < rows as u64
        || (!weighted && n != rows as u64)
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn cohort(r: &ComputedRegression) -> AppResult<()> {
    let d = &r.diagnostics;
    let weighted = r.request.frequency_weight.is_some();
    if (!weighted && (d.missing_weight_rows != 0 || d.zero_weight_rows != 0))
        || [
            d.filtered_rows,
            d.missing_weight_rows,
            d.zero_weight_rows,
            d.complete_case_rows,
            r.fitted_rows,
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
            != Some(r.source_rows)
    {
        return Err(invalid_analysis());
    }
    count(r.fitted_rows, r.frequency, weighted)?;
    count(d.complete_case_rows, r.incomplete_case_frequency, weighted)?;
    if r.frequency
        .checked_add(r.incomplete_case_frequency)
        .is_none_or(|n| n > MAX_FREQUENCY)
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn expansion(
    r: &ComputedRegression,
    infos: &BTreeMap<ColumnId, &RunColumnInfo>,
    failure: Option<&RegressionFailure>,
) -> AppResult<()> {
    if r.terms.first() != Some(&RegressionTerm::Intercept) || r.terms.len() > MAX_TERMS {
        return Err(invalid_analysis());
    }
    if failure.is_some_and(|f| matches!(f.kind, RegressionFailureKind::EmptyCohort)) {
        return if r.fitted_rows == 0 && r.levels.is_empty() && r.terms.len() == 1 {
            Ok(())
        } else {
            Err(invalid_analysis())
        };
    }
    if r.fitted_rows == 0 {
        return Err(invalid_analysis());
    }
    let (mut levels, mut terms) = (r.levels.iter(), r.terms[1..].iter());
    let mut expanded = 1;
    for p in &r.request.predictors {
        match &p.role {
            PredictorRole::Numeric => {
                if terms.next() != Some(&RegressionTerm::Numeric { column: p.column }) {
                    return Err(invalid_analysis());
                }
                expanded += 1;
            }
            PredictorRole::Categorical { reference } => {
                let discovered = levels.next().ok_or_else(invalid_analysis)?;
                ordered_levels(discovered, p.column, infos[&p.column])?;
                expanded += discovered.values.len() - 1;
                if category_terms(p.column, &discovered.values, reference, &mut terms, failure)? {
                    return if levels.next().is_none()
                        && terms.next().is_none()
                        && expanded <= MAX_TERMS
                    {
                        Ok(())
                    } else {
                        Err(invalid_analysis())
                    };
                }
            }
        }
        if expanded > MAX_TERMS {
            return Err(invalid_analysis());
        }
    }
    if levels.next().is_some()
        || terms.next().is_some()
        || failure.is_some_and(|f| {
            matches!(
                f.kind,
                RegressionFailureKind::AbsentReference { .. }
                    | RegressionFailureKind::NoContrast { .. }
            )
        })
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn ordered_levels(
    levels: &RegressionLevels,
    column: ColumnId,
    info: &RunColumnInfo,
) -> AppResult<()> {
    if levels.column != column || levels.values.is_empty() || levels.values.len() > MAX_TERMS {
        return Err(invalid_analysis());
    }
    for (index, value) in levels.values.iter().enumerate() {
        present(value, info)?;
        if matches!(value, Cell::Number(v) if *v == 0. && v.to_bits() != 0)
            || (index > 0 && compare(&levels.values[index - 1], value) != Some(Ordering::Less))
        {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}
fn category_terms(
    column: ColumnId,
    values: &[Cell],
    reference: &Cell,
    terms: &mut std::slice::Iter<'_, RegressionTerm>,
    failure: Option<&RegressionFailure>,
) -> AppResult<bool> {
    let reference_present = values.contains(reference);
    let early = failure.is_some_and(|f| matches!(f.kind, RegressionFailureKind::AbsentReference { column: id } | RegressionFailureKind::NoContrast { column: id } if id == column));
    if early {
        let truthful = match failure.ok_or_else(invalid_analysis)?.kind {
            RegressionFailureKind::AbsentReference { .. } => !reference_present,
            RegressionFailureKind::NoContrast { .. } => reference_present && values.len() == 1,
            _ => false,
        };
        return if truthful {
            Ok(true)
        } else {
            Err(invalid_analysis())
        };
    }
    if !reference_present || values.len() < 2 {
        return Err(invalid_analysis());
    }
    for value in values.iter().filter(|v| *v != reference) {
        match terms.next() {
            Some(RegressionTerm::Contrast {
                column: id,
                level,
                reference: saved,
            }) if *id == column && level == value && saved == reference => {}
            _ => return Err(invalid_analysis()),
        }
    }
    Ok(false)
}
fn failed(r: &ComputedRegression, f: &RegressionFailure) -> AppResult<()> {
    if f.reason.trim().is_empty() {
        return Err(invalid_analysis());
    }
    let p = r.terms.len();
    let early = matches!(
        f.kind,
        RegressionFailureKind::EmptyCohort
            | RegressionFailureKind::AbsentReference { .. }
            | RegressionFailureKind::NoContrast { .. }
    );
    if early {
        return if f.expected_terms.is_none() && f.rank.is_none() && f.implicated_terms.is_empty() {
            Ok(())
        } else {
            Err(invalid_analysis())
        };
    }
    if f.expected_terms != Some(p)
        || f.rank.is_some_and(|rank| rank > p || rank > r.fitted_rows)
        || f.implicated_terms.iter().any(|i| *i >= p)
        || f.implicated_terms.iter().collect::<BTreeSet<_>>().len() != f.implicated_terms.len()
    {
        return Err(invalid_analysis());
    }
    match f.kind {
        RegressionFailureKind::Unidentified
            if f.rank.is_some_and(|rank| rank >= p)
                || (f.rank.is_none() && r.fitted_rows >= p && r.frequency >= p as u64) =>
        {
            return Err(invalid_analysis())
        }
        RegressionFailureKind::Nonconvergence
            if f.rank.is_some() || !f.implicated_terms.is_empty() =>
        {
            return Err(invalid_analysis())
        }
        _ => {}
    }
    if f.rank.is_none() && !f.implicated_terms.is_empty() {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn estimate(e: &Estimate) -> AppResult<()> {
    match (&e.value, &e.reason) {
        (Some(v), None) if v.is_finite() => Ok(()),
        (None, Some(reason)) if !reason.trim().is_empty() => Ok(()),
        _ => Err(invalid_analysis()),
    }
}
fn bounded(e: &Estimate, lo: f64, hi: f64) -> AppResult<()> {
    estimate(e)?;
    if e.value.is_some_and(|v| v < lo || v > hi) {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn fit(r: &ComputedRegression, f: &RegressionFit) -> AppResult<()> {
    let p = r.terms.len();
    if p < 2
        || f.rank != p
        || p > r.fitted_rows
        || r.frequency < p as u64
        || f.model_df != (p - 1) as u64
        || f.residual_df != r.frequency - p as u64
        || f.total_df != r.frequency - 1
        || f.coefficients.len() != p
        || f.term_scales.len() != p
    {
        return Err(invalid_analysis());
    }
    bounded(&f.condition, 1., f64::MAX)?;
    if f.condition.value.is_none()
        || f.condition_warned
            != f.condition
                .value
                .is_some_and(|v| v > r.method.condition_warning)
    {
        return Err(invalid_analysis());
    }
    bounded(&f.outcome_scale, f64::from_bits(1), f64::MAX)?;
    for scale in &f.term_scales {
        bounded(scale, f64::from_bits(1), f64::MAX)?;
    }
    for coefficient in &f.coefficients {
        coefficient_fields(coefficient, f.residual_df)?;
    }
    statistics(f)?;
    diagnostics(r, &f.residual_diagnostics)?;
    numerical::relationships(r, f)
}
fn coefficient_fields(c: &RegressionCoefficient, residual_df: u64) -> AppResult<()> {
    estimate(&c.estimate)?;
    if c.estimate.value.is_none() {
        return Err(invalid_analysis());
    }
    bounded(&c.standard_error, 0., f64::MAX)?;
    estimate(&c.t)?;
    bounded(&c.p, f64::from_bits(1), 1.)?;
    ordered_range(&c.interval)?;
    if c.interval[0]
        .value
        .zip(c.interval[1].value)
        .is_some_and(|(lo, hi)| c.estimate.value.is_some_and(|v| v < lo || v > hi))
    {
        return Err(invalid_analysis());
    }
    if residual_df == 0
        && [
            &c.standard_error,
            &c.t,
            &c.p,
            &c.interval[0],
            &c.interval[1],
        ]
        .iter()
        .any(|e| e.value.is_some())
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn statistics(f: &RegressionFit) -> AppResult<()> {
    let s = &f.statistics;
    for e in [
        &s.sse,
        &s.sst,
        &s.model_ss,
        &s.residual_standard_error,
        &s.f,
    ] {
        bounded(e, 0., f64::MAX)?;
    }
    bounded(&s.r_squared, 0., 1.)?;
    bounded(&s.adjusted_r_squared, -f64::MAX, 1.)?;
    bounded(&s.f_p, f64::from_bits(1), 1.)?;
    if f.residual_df == 0
        && [
            &s.residual_standard_error,
            &s.adjusted_r_squared,
            &s.f,
            &s.f_p,
        ]
        .iter()
        .any(|e| e.value.is_some())
    {
        return Err(invalid_analysis());
    }
    if s.sst.value == Some(0.)
        && [&s.r_squared, &s.adjusted_r_squared, &s.f, &s.f_p]
            .iter()
            .any(|e| e.value.is_some())
    {
        return Err(invalid_analysis());
    }
    if s.residual_standard_error.value == Some(0.) {
        if s.sse.value != Some(0.) || s.f.value.is_some() || s.f_p.value.is_some() {
            return Err(invalid_analysis());
        }
        for c in &f.coefficients {
            if c.standard_error.value != Some(0.)
                || c.t.value.is_some()
                || c.p.value.is_some()
                || c.interval[0].value != c.estimate.value
                || c.interval[1].value != c.estimate.value
            {
                return Err(invalid_analysis());
            }
        }
    }
    Ok(())
}
fn ordered_range(range: &[Estimate; 2]) -> AppResult<()> {
    for e in range {
        estimate(e)?;
    }
    if range[0]
        .value
        .zip(range[1].value)
        .is_some_and(|(lo, hi)| lo > hi)
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn inside(e: &Estimate, range: &[Estimate; 2]) -> AppResult<()> {
    estimate(e)?;
    if e.value.is_some_and(|v| {
        range[0].value.is_some_and(|lo| v < lo) || range[1].value.is_some_and(|hi| v > hi)
    }) {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn extent(e: &DiagnosticExtent, r: &ComputedRegression, nonnegative: bool) -> AppResult<()> {
    ordered_range(&e.range)?;
    let weighted = r.request.frequency_weight.is_some();
    if e.undefined_rows > r.fitted_rows || e.undefined_frequency > r.frequency {
        return Err(invalid_analysis());
    }
    count(e.undefined_rows, e.undefined_frequency, weighted)?;
    count(
        r.fitted_rows - e.undefined_rows,
        r.frequency - e.undefined_frequency,
        weighted,
    )?;
    if (e.undefined_rows == r.fitted_rows) != e.range.iter().all(|v| v.value.is_none()) {
        return Err(invalid_analysis());
    }
    if nonnegative {
        for value in &e.range {
            bounded(value, 0., f64::MAX)?;
        }
    }
    Ok(())
}
fn diagnostics(r: &ComputedRegression, d: &RegressionDiagnostics) -> AppResult<()> {
    estimate(&d.residual_mean)?;
    bounded(&d.residual_sample_sd, 0., f64::MAX)?;
    for range in [&d.residual_range, &d.fitted_range, &d.theoretical_range] {
        ordered_range(range)?;
    }
    extent(&d.leverage, r, true)?;
    extent(&d.studentised, r, false)?;
    extent(&d.cooks_distance, r, true)?;
    let shown = r.fitted_rows.min(MAX_PLOT_POINTS);
    if d.residual_points.len() != shown
        || d.residual_reduced != (r.fitted_rows > MAX_PLOT_POINTS)
        || d.qq_points.len() != r.frequency.min(MAX_PLOT_POINTS as u64) as usize
        || d.qq_reduced != (r.frequency > MAX_PLOT_POINTS as u64)
    {
        return Err(invalid_analysis());
    }
    let mut rows = BTreeMap::new();
    let (mut previous, mut n) = (None, 0_u64);
    for (index, point) in d.residual_points.iter().enumerate() {
        if point.cohort_ordinal != display_index(r.fitted_rows, shown, index)?
            || point.row >= r.source_rows
            || previous.is_some_and(|row| row >= point.row)
        {
            return Err(invalid_analysis());
        }
        count(1, point.frequency, r.request.frequency_weight.is_some())?;
        n = n
            .checked_add(point.frequency)
            .ok_or_else(invalid_analysis)?;
        inside(&point.fitted, &d.fitted_range)?;
        inside(&point.residual, &d.residual_range)?;
        for (value, extent) in [
            (&point.leverage, &d.leverage),
            (&point.studentised, &d.studentised),
            (&point.cooks_distance, &d.cooks_distance),
        ] {
            inside(value, &extent.range)?;
        }
        rows.insert(point.row, point);
        previous = Some(point.row);
    }
    if n > r.frequency
        || r.frequency - n < (r.fitted_rows - shown) as u64
        || (!d.residual_reduced && n != r.frequency)
    {
        return Err(invalid_analysis());
    }
    qq(r, d, &rows)?;
    if !d.residual_reduced {
        diagnostic_counts(r, d)?;
        full_ranges(d)?;
    }
    Ok(())
}
fn full_ranges(d: &RegressionDiagnostics) -> AppResult<()> {
    for (field, range) in [(0, &d.residual_range), (1, &d.fitted_range)] {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut complete = true;
        for p in &d.residual_points {
            let value = if field == 0 { &p.residual } else { &p.fitted };
            if let Some(v) = value.value {
                lo = lo.min(v);
                hi = hi.max(v);
            } else {
                complete = false;
            }
        }
        if (complete && (range[0].value != Some(lo) || range[1].value != Some(hi)))
            || range[0].value.is_some_and(|v| v != lo)
            || range[1].value.is_some_and(|v| v != hi)
        {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}
fn qq(
    r: &ComputedRegression,
    d: &RegressionDiagnostics,
    rows: &BTreeMap<usize, &RegressionResidualPoint>,
) -> AppResult<()> {
    let shown = d.qq_points.len();
    let exact_zero =
        matches!(&r.outcome, RegressionOutcome::Fitted(f) if f.statistics.sse.value == Some(0.));
    let maximum_frequency = r
        .fitted_rows
        .checked_sub(1)
        .and_then(|other_rows| r.frequency.checked_sub(other_rows as u64))
        .ok_or_else(invalid_analysis)?;
    let mut previous: Option<&RegressionQqPoint> = None;
    let mut identities = BTreeMap::new();
    for (index, point) in d.qq_points.iter().enumerate() {
        let rank = if shown == 1 {
            0
        } else {
            (index as u128 * u128::from(r.frequency - 1) / (shown - 1) as u128) as u64
        };
        if point.rank != rank || point.row >= r.source_rows || point.frequency > maximum_frequency {
            return Err(invalid_analysis());
        }
        count(1, point.frequency, r.request.frequency_weight.is_some())?;
        inside(&point.theoretical, &d.theoretical_range)?;
        inside(&point.residual, &d.residual_range)?;
        if let Some(p) = previous {
            if p.rank >= point.rank
                || p.theoretical
                    .value
                    .zip(point.theoretical.value)
                    .is_some_and(|(a, b)| a >= b)
                || p.residual
                    .value
                    .zip(point.residual.value)
                    .is_some_and(|(a, b)| a > b || (exact_zero && a == b && p.row > point.row))
                || (p.row == point.row && point.rank - p.rank >= point.frequency)
            {
                return Err(invalid_analysis());
            }
        }
        if let Some(p) = rows.get(&point.row) {
            if p.frequency != point.frequency || p.residual != point.residual {
                return Err(invalid_analysis());
            }
        } else if !d.residual_reduced {
            return Err(invalid_analysis());
        }
        if let Some((frequency, residual, first_rank)) = identities.get(&point.row) {
            if *frequency != point.frequency
                || *residual != &point.residual
                || previous.is_none_or(|p| p.row != point.row)
                || point.rank - first_rank >= point.frequency
            {
                return Err(invalid_analysis());
            }
        }
        identities
            .entry(point.row)
            .or_insert((point.frequency, &point.residual, point.rank));
        previous = Some(point);
    }
    if d.qq_points
        .first()
        .is_some_and(|p| p.theoretical != d.theoretical_range[0])
        || d.qq_points
            .last()
            .is_some_and(|p| p.theoretical != d.theoretical_range[1])
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn diagnostic_counts(r: &ComputedRegression, d: &RegressionDiagnostics) -> AppResult<()> {
    for (field, extent) in [
        (0, &d.leverage),
        (1, &d.studentised),
        (2, &d.cooks_distance),
    ] {
        let mut rows = 0;
        let mut n = 0_u64;
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &d.residual_points {
            let value = match field {
                0 => &p.leverage,
                1 => &p.studentised,
                _ => &p.cooks_distance,
            };
            if let Some(v) = value.value {
                lo = lo.min(v);
                hi = hi.max(v);
            } else {
                rows += 1;
                n += p.frequency;
            }
        }
        if rows != extent.undefined_rows
            || n != extent.undefined_frequency
            || (rows < r.fitted_rows
                && (extent.range[0].value != Some(lo) || extent.range[1].value != Some(hi)))
        {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}
