//! Current role validation and independent retained-result invariants.
use super::*;
use std::collections::BTreeSet;

pub(crate) fn validate_request(
    request: AnalysisRequestRef<'_>,
    kind: impl Fn(ColumnId) -> AppResult<ColumnKind>,
) -> AppResult<BTreeSet<ColumnId>> {
    validate_request_controlled(request, kind, &JobControl::default())
}
pub(crate) fn validate_request_controlled(
    request: AnalysisRequestRef<'_>,
    kind: impl Fn(ColumnId) -> AppResult<ColumnKind>,
    control: &JobControl,
) -> AppResult<BTreeSet<ColumnId>> {
    control.check_cancelled()?;
    match request {
        AnalysisRequestRef::Reliability(v) => {
            return reliability::validation::request(v, kind, control)
        }
        AnalysisRequestRef::Anova(v) => return anova::validation::request(v, kind, control),
        AnalysisRequestRef::Regression(v) => return regression::validation::request(v, kind),
        AnalysisRequestRef::Frequencies(v) => {
            return exploration::validation::request(
                exploration::model::RequestRef::Frequencies(v),
                kind,
            )
        }
        AnalysisRequestRef::Histogram(v) => {
            return exploration::validation::request(
                exploration::model::RequestRef::Histogram(v),
                kind,
            )
        }
        AnalysisRequestRef::Scatter(v) => {
            return exploration::validation::request(
                exploration::model::RequestRef::Scatter(v),
                kind,
            )
        }
        AnalysisRequestRef::Boxplot(v) => {
            return exploration::validation::request(
                exploration::model::RequestRef::Boxplot(v),
                kind,
            )
        }
        _ => {}
    }
    let mut ids = BTreeSet::new();
    let mut numeric = |id: ColumnId| -> AppResult<()> {
        if id.0 == 0 || kind(id)? != ColumnKind::Number || !ids.insert(id) {
            return Err(invalid_analysis());
        }
        Ok(())
    };
    match request {
        AnalysisRequestRef::Frequencies(_)
        | AnalysisRequestRef::Histogram(_)
        | AnalysisRequestRef::Scatter(_)
        | AnalysisRequestRef::Boxplot(_)
        | AnalysisRequestRef::Regression(_)
        | AnalysisRequestRef::Reliability(_)
        | AnalysisRequestRef::Anova(_) => unreachable!("handled above"),
        AnalysisRequestRef::Describe(v) => {
            if v.columns.is_empty()
                || v.statistics.is_empty()
                || v.statistics.iter().collect::<BTreeSet<_>>().len() != v.statistics.len()
            {
                return Err(invalid_analysis());
            }
            for id in &v.columns {
                numeric(*id)?;
            }
        }
        AnalysisRequestRef::Pearson(v) => {
            if v.columns.len() < 2 {
                return Err(invalid_analysis());
            }
            for id in &v.columns {
                numeric(*id)?;
            }
        }
        AnalysisRequestRef::OneSampleT(v) => {
            numeric(v.variable)?;
            if !v.reference_mean.is_finite() {
                return Err(invalid_analysis());
            }
        }
        AnalysisRequestRef::PairedT(v) => {
            numeric(v.first)?;
            numeric(v.second)?;
        }
        AnalysisRequestRef::IndependentT(v) => {
            numeric(v.outcome)?;
            if v.group.0 == 0 || v.outcome == v.group {
                return Err(invalid_analysis());
            }
            let group_kind = kind(v.group)?;
            if v.groups[0] == v.groups[1]
                || v.groups.iter().any(|value| !valid_group(value, group_kind))
            {
                return Err(invalid_analysis());
            }
            ids.insert(v.group);
        }
    }
    validate_filter_weight(
        request.filter(),
        request.frequency_weight(),
        &kind,
        &mut ids,
    )?;
    Ok(ids)
}

/// Shared authority for current and captured filter/weight references.
pub(crate) fn validate_filter_weight(
    filter: &RowFilter,
    frequency_weight: Option<ColumnId>,
    kind: impl Fn(ColumnId) -> AppResult<ColumnKind>,
    ids: &mut BTreeSet<ColumnId>,
) -> AppResult<()> {
    for predicate in &filter.predicates {
        let id = predicate.column();
        if id.0 == 0 {
            return Err(invalid_analysis());
        }
        let actual = kind(id)?;
        match predicate {
            RowPredicate::Number { value, .. }
                if actual != ColumnKind::Number || !value.is_finite() =>
            {
                return Err(invalid_analysis())
            }
            RowPredicate::Text { .. } if actual != ColumnKind::Text => {
                return Err(invalid_analysis())
            }
            _ => {}
        }
        ids.insert(id);
    }
    if let Some(id) = frequency_weight {
        if id.0 == 0 || kind(id)? != ColumnKind::Number {
            return Err(invalid_analysis());
        }
        ids.insert(id);
    }
    Ok(())
}
fn valid_group(value: &Cell, kind: ColumnKind) -> bool {
    matches!((value, kind), (Cell::Number(n), ColumnKind::Number) if n.is_finite())
        || matches!((value, kind), (Cell::Text(_), ColumnKind::Text))
}
fn estimate(value: &Estimate) -> AppResult<()> {
    match (&value.value, &value.reason) {
        (Some(n), None) if n.is_finite() => Ok(()),
        (None, Some(reason)) if !reason.trim().is_empty() => Ok(()),
        _ => Err(invalid_analysis()),
    }
}
fn bounded(value: &Estimate, lo: f64, hi: f64) -> AppResult<()> {
    estimate(value)?;
    if value.value.is_some_and(|n| n < lo || n > hi) {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn count(n: u64) -> AppResult<()> {
    if n > MAX_FREQUENCY {
        Err(invalid_analysis())
    } else {
        Ok(())
    }
}
fn source_counts(counts: impl IntoIterator<Item = usize>) -> AppResult<()> {
    if counts
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .is_none_or(|n| n > crate::data::MAX_CELLS)
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn sample(value: &SampleSummary) -> AppResult<()> {
    count(value.n)?;
    estimate(&value.mean)?;
    bounded(&value.sample_sd, 0.0, f64::MAX)?;
    if (value.n == 0 && value.mean.value.is_some())
        || (value.n < 2 && value.sample_sd.value.is_some())
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn t_statistics(
    value: &TStatistics,
    eligible: bool,
    minimum_df: f64,
    maximum_df: f64,
    exact_df: Option<f64>,
) -> AppResult<()> {
    for item in value.estimates() {
        estimate(item)?;
    }
    bounded(&value.standard_error, 0.0, f64::MAX)?;
    bounded(
        &value.df,
        f64::MIN_POSITIVE,
        maximum_df.max(f64::MIN_POSITIVE),
    )?;
    // Welch's ratio uses sums, products and division; allow 16 relative epsilons at its lower boundary.
    let df_tolerance = 16.0 * f64::EPSILON * minimum_df.max(1.0);
    if value
        .df
        .value
        .is_some_and(|df| df < minimum_df - df_tolerance)
    {
        return Err(invalid_analysis());
    }
    bounded(&value.p, f64::from_bits(1), 1.0)?;
    if value
        .df
        .value
        .is_some_and(|df| exact_df.is_some_and(|expected| df != expected))
        || (!eligible
            && [
                &value.standard_error,
                &value.t,
                &value.df,
                &value.p,
                &value.interval[0],
                &value.interval[1],
                &value.effect_size,
            ]
            .iter()
            .any(|e| e.value.is_some()))
        || (value.p.value.is_some() && value.df.value.is_none())
        || (value.standard_error.value == Some(0.0)
            && (value.t.value.is_some() || value.p.value.is_some()))
        || matches!((value.interval[0].value, value.interval[1].value), (Some(lo), Some(hi)) if lo > hi)
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn unweighted_source_total(n: u64, exclusions: impl IntoIterator<Item = usize>) -> AppResult<()> {
    let n = usize::try_from(n).map_err(|_| invalid_analysis())?;
    source_counts(std::iter::once(n).chain(exclusions))
}
pub(crate) fn validate_completed_controlled(
    result: &CompletedAnalysis,
    control: &JobControl,
) -> AppResult<()> {
    control.check_cancelled()?;
    match result {
        CompletedAnalysis::Reliability(v) => return v.validate_controlled(control),
        CompletedAnalysis::Anova(v) => return v.validate(control),
        CompletedAnalysis::Regression(v) => return v.validate(),
        CompletedAnalysis::Frequencies(v) => return v.validate(),
        CompletedAnalysis::Histogram(v) => return v.validate(),
        CompletedAnalysis::Scatter(v) => return v.validate(),
        CompletedAnalysis::Boxplot(v) => return v.validate(),
        _ => {}
    }
    let d = result.diagnostics();
    let unweighted = result.request().frequency_weight().is_none();
    if unweighted && (d.missing_weight_rows != 0 || d.zero_weight_rows != 0) {
        return Err(invalid_analysis());
    }
    if let CompletedAnalysis::Describe(value) = result {
        return value.validate();
    }
    if result.revision().0 == 0 {
        return Err(invalid_analysis());
    }
    let infos = result
        .columns()
        .iter()
        .map(|info| (info.id, info))
        .collect::<BTreeMap<_, _>>();
    if infos.len() != result.columns().len() {
        return Err(invalid_analysis());
    }
    for info in result.columns() {
        validate_snapshot(info)?;
    }
    let ids = validate_request(result.request(), |id| {
        infos
            .get(&id)
            .map(|info| info.kind)
            .ok_or_else(invalid_analysis)
    })?;
    if ids != infos.keys().copied().collect() {
        return Err(invalid_analysis());
    }
    source_counts([
        d.filtered_rows,
        d.missing_weight_rows,
        d.zero_weight_rows,
        d.complete_case_rows,
    ])?;
    match result {
        CompletedAnalysis::Pearson(v) => {
            let expected = v
                .request
                .columns
                .len()
                .checked_mul(v.request.columns.len() - 1)
                .map(|n| n / 2)
                .ok_or_else(invalid_analysis)?;
            if v.rows.len() != expected
                || (v.request.missing_policy == CorrelationMissingPolicy::PairwiseComplete
                    && d.complete_case_rows != 0)
            {
                return Err(invalid_analysis());
            }
            let mut index = 0;
            for (i, first) in v.request.columns.iter().enumerate() {
                for second in &v.request.columns[i + 1..] {
                    let row = &v.rows[index];
                    index += 1;
                    if row.first != *first || row.second != *second {
                        return Err(invalid_analysis());
                    }
                    count(row.n)?;
                    if unweighted {
                        unweighted_source_total(
                            row.n,
                            [
                                row.missing_rows,
                                row.complete_case_excluded_rows,
                                d.filtered_rows,
                            ],
                        )?;
                    }
                    bounded(&row.r, -1.0, 1.0)?;
                    bounded(&row.p, 0.0, 1.0)?;
                    source_counts([
                        d.filtered_rows,
                        d.missing_weight_rows,
                        d.zero_weight_rows,
                        row.missing_rows,
                        row.complete_case_excluded_rows,
                    ])?;
                    if (row.n < 2 && row.r.value.is_some())
                        || (row.n < 3 && row.p.value.is_some())
                        || (row.p.value.is_some() && row.r.value.is_none())
                        || (row.p.value == Some(0.0) && !matches!(row.r.value, Some(1.0 | -1.0)))
                        || (v.request.missing_policy == CorrelationMissingPolicy::PairwiseComplete
                            && row.complete_case_excluded_rows != 0)
                        || (v.request.missing_policy == CorrelationMissingPolicy::CompleteCases
                            && row
                                .missing_rows
                                .checked_add(row.complete_case_excluded_rows)
                                != Some(d.complete_case_rows))
                    {
                        return Err(invalid_analysis());
                    }
                }
            }
        }
        CompletedAnalysis::OneSampleT(v) => {
            sample(&v.sample)?;
            if unweighted {
                unweighted_source_total(v.sample.n, [v.missing_rows, d.filtered_rows])?;
            }
            if d.complete_case_rows != 0 {
                return Err(invalid_analysis());
            }
            source_counts([
                d.filtered_rows,
                d.missing_weight_rows,
                d.zero_weight_rows,
                v.missing_rows,
            ])?;
            t_statistics(
                &v.statistics,
                v.sample.n >= 2,
                1.0,
                v.sample.n.saturating_sub(1) as f64,
                Some(v.sample.n.saturating_sub(1) as f64),
            )?;
        }
        CompletedAnalysis::PairedT(v) => {
            sample(&v.first)?;
            sample(&v.second)?;
            if unweighted {
                unweighted_source_total(v.first.n, [d.complete_case_rows, d.filtered_rows])?;
            }
            bounded(&v.difference_sd, 0.0, f64::MAX)?;
            if v.first.n != v.second.n || (v.first.n < 2 && v.difference_sd.value.is_some()) {
                return Err(invalid_analysis());
            }
            t_statistics(
                &v.statistics,
                v.first.n >= 2,
                1.0,
                v.first.n.saturating_sub(1) as f64,
                Some(v.first.n.saturating_sub(1) as f64),
            )?;
        }
        CompletedAnalysis::IndependentT(v) => {
            for group in &v.groups {
                sample(group)?;
            }
            let total = v.groups[0]
                .n
                .checked_add(v.groups[1].n)
                .filter(|n| *n <= MAX_FREQUENCY)
                .ok_or_else(invalid_analysis)?;
            if d.complete_case_rows != 0 {
                return Err(invalid_analysis());
            }
            source_counts([
                d.filtered_rows,
                d.missing_weight_rows,
                d.zero_weight_rows,
                v.exclusions.missing_group_rows,
                v.exclusions.other_group_rows,
                v.exclusions.missing_outcome_rows,
            ])?;
            if unweighted {
                unweighted_source_total(
                    total,
                    [
                        v.exclusions.missing_group_rows,
                        v.exclusions.other_group_rows,
                        v.exclusions.missing_outcome_rows,
                        d.filtered_rows,
                    ],
                )?;
            }
            let df = total.saturating_sub(2) as f64;
            t_statistics(
                &v.statistics,
                v.groups.iter().all(|s| s.n >= 2),
                if v.request.method == IndependentMethod::Welch {
                    v.groups[0].n.min(v.groups[1].n).saturating_sub(1) as f64
                } else {
                    df
                },
                df,
                (v.request.method == IndependentMethod::Pooled).then_some(df),
            )?;
        }
        CompletedAnalysis::Describe(_)
        | CompletedAnalysis::Frequencies(_)
        | CompletedAnalysis::Histogram(_)
        | CompletedAnalysis::Scatter(_)
        | CompletedAnalysis::Boxplot(_)
        | CompletedAnalysis::Regression(_)
        | CompletedAnalysis::Reliability(_)
        | CompletedAnalysis::Anova(_) => unreachable!("handled above"),
    }
    Ok(())
}

impl ComputedAnalysis {
    pub(crate) fn validate(&self) -> AppResult<()> {
        if self.revision.0 == 0 {
            return Err(invalid_analysis());
        }
        let infos = self
            .columns
            .iter()
            .map(|info| (info.id, info))
            .collect::<BTreeMap<_, _>>();
        let selected = self
            .request
            .columns
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let stats = self
            .request
            .statistics
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if infos.len() != self.columns.len()
            || selected.is_empty()
            || selected.len() != self.request.columns.len()
            || stats.is_empty()
            || stats.len() != self.request.statistics.len()
        {
            return Err(invalid_analysis());
        }
        validate_references(&self.request, &infos, &selected)?;
        if self.rows.len() != selected.len() || self.plots.len() != selected.len() {
            return Err(invalid_analysis());
        }
        for info in &self.columns {
            validate_snapshot(info)?;
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.column != self.request.columns[index] {
                return Err(invalid_analysis());
            }
            validate_row(row, &stats)?;
            validate_plot(&self.plots[index], row)?;
        }
        Ok(())
    }
}
fn validate_references(
    request: &DescribeRequest,
    infos: &BTreeMap<ColumnId, &RunColumnInfo>,
    selected: &std::collections::BTreeSet<ColumnId>,
) -> AppResult<()> {
    let mut references = selected.clone();
    for id in selected {
        require_kind(infos, *id, ColumnKind::Number)?;
    }
    validate_filter_weight(
        &request.filter,
        request.frequency_weight,
        |id| {
            infos
                .get(&id)
                .map(|info| info.kind)
                .ok_or_else(invalid_analysis)
        },
        &mut references,
    )?;
    if references != infos.keys().copied().collect() {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn validate_row(
    row: &DescriptiveRow,
    stats: &std::collections::BTreeSet<Statistic>,
) -> AppResult<()> {
    if row
        .valid_n
        .checked_add(row.missing_n)
        .and_then(|n| n.checked_add(row.complete_case_excluded_n))
        .is_none_or(|n| n > MAX_FREQUENCY)
        || row
            .estimates
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            != *stats
    {
        return Err(invalid_analysis());
    }
    for estimate in row.estimates.values() {
        match (&estimate.value, &estimate.reason) {
            (Some(value), None) if value.is_finite() => {}
            (None, Some(reason)) if !reason.trim().is_empty() => {}
            _ => return Err(invalid_analysis()),
        }
    }
    Ok(())
}

pub(crate) fn invalid_analysis() -> AppError {
    AppError::invalid(
        "analysis",
        "The retained analysis has invalid settings, identity or numeric results.",
    )
}
fn require_kind(
    infos: &BTreeMap<ColumnId, &RunColumnInfo>,
    id: ColumnId,
    kind: ColumnKind,
) -> AppResult<()> {
    if infos.get(&id).is_none_or(|info| info.kind != kind) {
        return Err(invalid_analysis());
    }
    Ok(())
}
mod snapshot;
pub(super) use snapshot::{
    snapshot_scratch_bytes, validate_snapshot, validate_snapshot_controlled,
};

fn validate_plot(plot: &PlotData, row: &DescriptiveRow) -> AppResult<()> {
    if plot.column != row.column
        || plot.points.len() > MAX_PLOT_POINTS
        || plot.source_rows > crate::data::MAX_CELLS
        || plot.points.len() != plot.source_rows.min(MAX_PLOT_POINTS)
        || plot.reduced != (plot.source_rows > MAX_PLOT_POINTS)
    {
        return Err(invalid_analysis());
    }
    let mut rows = std::collections::BTreeSet::new();
    let mut frequency = 0_u64;
    for point in &plot.points {
        if !point.value.is_finite()
            || point.frequency == 0
            || point.row >= crate::data::MAX_CELLS
            || !rows.insert(point.row)
        {
            return Err(invalid_analysis());
        }
        frequency = frequency
            .checked_add(point.frequency)
            .filter(|n| *n <= row.valid_n)
            .ok_or_else(invalid_analysis)?;
    }
    Ok(())
}
