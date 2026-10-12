//! Stored identities and algebraic invariants; historical validation never reruns data.
use super::{model::RequestRef, *};
use crate::{
    analysis::{
        moments::inside_box_fences,
        validation::{invalid_analysis, validate_filter_weight, validate_snapshot},
        Estimate, RunColumnInfo, RunDiagnostics, MAX_FREQUENCY, MAX_PLOT_POINTS,
    },
    data::{Cell, ColumnId, ColumnKind, MissingRules, RevisionId, MAX_CELLS},
    error::AppResult,
};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

pub(crate) fn request(
    r: RequestRef<'_>,
    kind: impl Fn(ColumnId) -> AppResult<ColumnKind>,
) -> AppResult<BTreeSet<ColumnId>> {
    let mut ids = BTreeSet::new();
    let mut role = |id: ColumnId, numeric: bool| -> AppResult<()> {
        if id.0 == 0 || (numeric && kind(id)? != ColumnKind::Number) || !ids.insert(id) {
            return Err(invalid_analysis());
        }
        kind(id)?;
        Ok(())
    };
    match r {
        RequestRef::Frequencies(r) => {
            if r.columns.is_empty() {
                return Err(invalid_analysis());
            }
            for id in &r.columns {
                role(*id, false)?;
            }
            if let Some(id) = r.group {
                role(id, false)?;
            }
        }
        RequestRef::Histogram(r) => {
            if matches!(r.bins, HistogramBins::Count(n) if !(1..=128).contains(&n)) {
                return Err(invalid_analysis());
            }
            role(r.variable, true)?;
        }
        RequestRef::Scatter(r) => {
            role(r.x, true)?;
            role(r.y, true)?;
        }
        RequestRef::Boxplot(r) => {
            role(r.outcome, true)?;
            if let Some(id) = r.group {
                role(id, false)?;
            }
        }
    }
    validate_filter_weight(r.filter(), r.weight(), kind, &mut ids)?;
    Ok(ids)
}
fn common<'a>(
    revision: RevisionId,
    r: RequestRef<'_>,
    columns: &'a [RunColumnInfo],
    source_rows: usize,
    diagnostics: &RunDiagnostics,
) -> AppResult<BTreeMap<ColumnId, &'a RunColumnInfo>> {
    if revision.0 == 0
        || source_rows > MAX_CELLS
        || diagnostics.complete_case_rows != 0
        || (r.weight().is_none()
            && (diagnostics.missing_weight_rows != 0 || diagnostics.zero_weight_rows != 0))
    {
        return Err(invalid_analysis());
    }
    let infos = columns
        .iter()
        .map(|info| (info.id, info))
        .collect::<BTreeMap<_, _>>();
    if infos.len() != columns.len() {
        return Err(invalid_analysis());
    }
    for info in columns {
        validate_snapshot(info)?;
    }
    let ids = request(r, |id| {
        infos
            .get(&id)
            .map(|info| info.kind)
            .ok_or_else(invalid_analysis)
    })?;
    if !ids.iter().eq(infos.keys()) {
        return Err(invalid_analysis());
    }
    partition(
        source_rows,
        diagnostics,
        0,
        source_rows
            .checked_sub(excluded(diagnostics)?)
            .ok_or_else(invalid_analysis)?,
    )?;
    Ok(infos)
}
fn excluded(d: &RunDiagnostics) -> AppResult<usize> {
    row_sum([d.filtered_rows, d.missing_weight_rows, d.zero_weight_rows])
}
fn row_sum(rows: impl IntoIterator<Item = usize>) -> AppResult<usize> {
    rows.into_iter().try_fold(0usize, |a, b| {
        a.checked_add(b)
            .filter(|n| *n <= MAX_CELLS)
            .ok_or_else(invalid_analysis)
    })
}
fn frequency_sum(ns: impl IntoIterator<Item = u64>) -> AppResult<u64> {
    ns.into_iter().try_fold(0u64, |a, b| {
        a.checked_add(b)
            .filter(|n| *n <= MAX_FREQUENCY)
            .ok_or_else(invalid_analysis)
    })
}
fn partition(source: usize, d: &RunDiagnostics, valid: usize, missing: usize) -> AppResult<()> {
    if row_sum([excluded(d)?, valid, missing])? != source {
        return Err(invalid_analysis());
    }
    Ok(())
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
fn estimate(e: &Estimate) -> AppResult<()> {
    match (&e.value, &e.reason) {
        (Some(n), None) if n.is_finite() => Ok(()),
        (None, Some(reason)) if !reason.trim().is_empty() => Ok(()),
        _ => Err(invalid_analysis()),
    }
}
fn percent(e: &Estimate, n: u64, denominator: u64) -> AppResult<()> {
    estimate(e)?;
    if denominator == 0 {
        if e.value.is_some() {
            return Err(invalid_analysis());
        }
    } else {
        let expected = 100.0 * n as f64 / denominator as f64;
        // Permit only operation-order rounding; retained percentages still derive from exact counts.
        if e.value
            .is_none_or(|v| (v - expected).abs() > 8.0 * f64::EPSILON * expected.abs())
        {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}
fn range(estimates: &[Estimate; 2], rows: usize) -> AppResult<Option<[f64; 2]>> {
    for e in estimates {
        estimate(e)?;
    }
    match (rows, estimates[0].value, estimates[1].value) {
        (0, None, None) => Ok(None),
        (1.., Some(lo), Some(hi)) if lo <= hi && (rows != 1 || lo == hi) => Ok(Some([lo, hi])),
        _ => Err(invalid_analysis()),
    }
}
fn compare(a: &Cell, b: &Cell) -> Option<Ordering> {
    match (a, b) {
        (Cell::Number(a), Cell::Number(b)) => a.partial_cmp(b),
        (Cell::Text(a), Cell::Text(b)) => Some(a.cmp(b)),
        _ => None,
    }
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum LabelKey<'a> {
    Number(u64),
    Text(&'a str),
}
fn key(value: &Cell, kind: ColumnKind) -> AppResult<LabelKey<'_>> {
    match (value, kind) {
        (Cell::Number(v), ColumnKind::Number) if v.is_finite() => {
            Ok(LabelKey::Number(if *v == 0.0 { 0 } else { v.to_bits() }))
        }
        (Cell::Text(v), ColumnKind::Text) => Ok(LabelKey::Text(v)),
        _ => Err(invalid_analysis()),
    }
}
/// Borrowed keys and prefix-max ranges avoid category-by-label/missing-rule rescans.
struct RawMetadata<'a> {
    kind: ColumnKind,
    labels: BTreeMap<LabelKey<'a>, usize>,
    missing: BTreeSet<LabelKey<'a>>,
    ranges: Vec<(f64, f64)>,
}
impl<'a> RawMetadata<'a> {
    fn new(info: &'a RunColumnInfo) -> AppResult<Self> {
        let labels = info
            .value_labels
            .iter()
            .enumerate()
            .map(|(index, label)| Ok((key(&label.value, info.kind)?, index)))
            .collect::<AppResult<_>>()?;
        let (mut missing, mut ranges) = (BTreeSet::new(), Vec::new());
        match &info.missing_rules {
            MissingRules::None => {}
            MissingRules::Number {
                values,
                ranges: source,
            } => {
                for value in values {
                    missing.insert(LabelKey::Number(if *value == 0.0 {
                        0
                    } else {
                        value.to_bits()
                    }));
                }
                ranges.extend_from_slice(source);
                ranges.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                let mut hi = f64::NEG_INFINITY;
                for range in &mut ranges {
                    hi = hi.max(range.1);
                    range.1 = hi;
                }
            }
            MissingRules::Text(values) => {
                for value in values {
                    missing.insert(LabelKey::Text(value));
                }
            }
        }
        Ok(Self {
            kind: info.kind,
            labels,
            missing,
            ranges,
        })
    }
    fn present(&self, value: &Cell) -> AppResult<()> {
        let value_key = key(value, self.kind)?;
        let interval_missing = if let Cell::Number(value) = value {
            let end = self.ranges.partition_point(|(lo, _)| lo <= value);
            end > 0 && *value <= self.ranges[end - 1].1
        } else {
            false
        };
        if self.missing.contains(&value_key) || interval_missing {
            return Err(invalid_analysis());
        }
        Ok(())
    }
    fn category(&self, value: &Cell, index: Option<usize>) -> AppResult<()> {
        self.present(value)?;
        if self.labels.get(&key(value, self.kind)?).copied() != index {
            return Err(invalid_analysis());
        }
        Ok(())
    }
}
pub(super) fn frequencies(v: &ComputedFrequencies) -> AppResult<()> {
    let infos = common(
        v.revision,
        RequestRef::Frequencies(&v.request),
        &v.columns,
        v.source_rows,
        &v.diagnostics,
    )?;
    let weighted = v.request.frequency_weight.is_some();
    let rows = v
        .source_rows
        .checked_sub(excluded(&v.diagnostics)?)
        .ok_or_else(invalid_analysis)?;
    frequency_tables(&v.tables, &v.request.columns, &infos, rows, weighted)?;
    let total = frequency_sum([v.tables[0].valid_n, v.tables[0].missing_n])?;
    let Some(id) = v.request.group else {
        return if v.groups.is_empty() {
            Ok(())
        } else {
            Err(invalid_analysis())
        };
    };
    let raw = RawMetadata::new(infos[&id])?;
    let (mut group_rows, mut group_n) = (0, 0);
    let mut previous = None;
    let mut missing_seen = false;
    for group in &v.groups {
        count(group.rows, group.frequency, weighted)?;
        if group.rows == 0 {
            return Err(invalid_analysis());
        }
        if group.value == Cell::Missing {
            if missing_seen || group.value_label_index.is_some() {
                return Err(invalid_analysis());
            }
            missing_seen = true;
        } else {
            raw.category(&group.value, group.value_label_index)?;
            if missing_seen
                || previous.is_some_and(|p| compare(p, &group.value) != Some(Ordering::Less))
            {
                return Err(invalid_analysis());
            }
            previous = Some(&group.value);
        }
        frequency_tables(
            &group.tables,
            &v.request.columns,
            &infos,
            group.rows,
            weighted,
        )?;
        if frequency_sum([group.tables[0].valid_n, group.tables[0].missing_n])? != group.frequency {
            return Err(invalid_analysis());
        }
        group_rows = row_sum([group_rows, group.rows])?;
        group_n = frequency_sum([group_n, group.frequency])?;
    }
    if group_rows != rows || group_n != total {
        return Err(invalid_analysis());
    }
    for (index, overall) in v.tables.iter().enumerate() {
        if row_sum(v.groups.iter().map(|g| g.tables[index].missing_rows))? != overall.missing_rows
            || frequency_sum(v.groups.iter().map(|g| g.tables[index].missing_n))?
                != overall.missing_n
        {
            return Err(invalid_analysis());
        }
        let mut totals = vec![(0usize, 0u64); overall.categories.len()];
        for category in v.groups.iter().flat_map(|g| &g.tables[index].categories) {
            let position = overall
                .categories
                .binary_search_by(|c| compare(&c.value, &category.value).unwrap_or(Ordering::Equal))
                .map_err(|_| invalid_analysis())?;
            totals[position].0 = row_sum([totals[position].0, category.rows])?;
            totals[position].1 = frequency_sum([totals[position].1, category.frequency])?;
        }
        if overall
            .categories
            .iter()
            .zip(totals)
            .any(|(c, (rows, n))| rows != c.rows || n != c.frequency)
        {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}
fn frequency_tables(
    tables: &[FrequencyTable],
    ids: &[ColumnId],
    infos: &BTreeMap<ColumnId, &RunColumnInfo>,
    cohort_rows: usize,
    weighted: bool,
) -> AppResult<()> {
    if tables.len() != ids.len() {
        return Err(invalid_analysis());
    }

    let mut shared_total = None;
    for (table, id) in tables.iter().zip(ids) {
        if table.column != *id {
            return Err(invalid_analysis());
        }
        let info = infos[id];
        let raw = RawMetadata::new(info)?;
        count(table.valid_rows, table.valid_n, weighted)?;
        count(table.missing_rows, table.missing_n, weighted)?;
        if row_sum([table.valid_rows, table.missing_rows])? != cohort_rows {
            return Err(invalid_analysis());
        }
        let all = frequency_sum([table.valid_n, table.missing_n])?;
        if shared_total.is_some_and(|total| total != all) {
            return Err(invalid_analysis());
        }
        shared_total = Some(all);
        percent(&table.missing_percent, table.missing_n, all)?;
        let (mut rows, mut n) = (0, 0);
        let mut previous = None;
        for c in &table.categories {
            raw.category(&c.value, c.value_label_index)?;
            if previous.is_some_and(|p| compare(p, &c.value) != Some(Ordering::Less)) || c.rows == 0
            {
                return Err(invalid_analysis());
            }
            previous = Some(&c.value);
            count(c.rows, c.frequency, weighted)?;
            rows = row_sum([rows, c.rows])?;
            n = frequency_sum([n, c.frequency])?;
            percent(&c.percent, c.frequency, all)?;
            percent(&c.valid_percent, c.frequency, table.valid_n)?;
            match info.kind {
                ColumnKind::Number => {
                    if c.cumulative_frequency != Some(n) {
                        return Err(invalid_analysis());
                    }
                    percent(
                        c.cumulative_percent.as_ref().ok_or_else(invalid_analysis)?,
                        n,
                        table.valid_n,
                    )?;
                }
                ColumnKind::Text
                    if c.cumulative_frequency.is_some() || c.cumulative_percent.is_some() =>
                {
                    return Err(invalid_analysis())
                }
                ColumnKind::Text => {}
            }
        }
        if rows != table.valid_rows || n != table.valid_n {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}
fn cohort(
    source: usize,
    d: &RunDiagnostics,
    rows: usize,
    n: u64,
    missing_rows: usize,
    missing_n: u64,
    weighted: bool,
) -> AppResult<()> {
    partition(source, d, rows, missing_rows)?;
    count(rows, n, weighted)?;
    count(missing_rows, missing_n, weighted)?;
    frequency_sum([n, missing_n])?;
    Ok(())
}
pub(super) fn histogram(v: &ComputedHistogram) -> AppResult<()> {
    let infos = common(
        v.revision,
        RequestRef::Histogram(&v.request),
        &v.columns,
        v.source_rows,
        &v.diagnostics,
    )?;
    let weighted = v.request.frequency_weight.is_some();
    cohort(
        v.source_rows,
        &v.diagnostics,
        v.valid_rows,
        v.n,
        v.missing_rows,
        v.missing_n,
        weighted,
    )?;
    if v.bin_adjustment
        .as_ref()
        .is_some_and(|s| s.trim().is_empty())
    {
        return Err(invalid_analysis());
    }
    let Some([lo, hi]) = range(&v.range, v.valid_rows)? else {
        return if v.bins.is_empty() && v.bin_adjustment.is_none() {
            Ok(())
        } else {
            Err(invalid_analysis())
        };
    };
    let raw = RawMetadata::new(infos[&v.request.variable])?;
    raw.present(&Cell::Number(lo))?;
    raw.present(&Cell::Number(hi))?;
    let target = match v.request.bins {
        HistogramBins::Count(n) => usize::from(n),
        HistogramBins::Automatic => super::numeric::sturges_bins(v.n)?,
    };
    if v.bins.is_empty()
        || v.bins[0].rows == 0
        || v.bins.last().is_none_or(|bin| bin.rows == 0)
        || v.bins.len() > target
        || (lo == hi && v.bins.len() != 1)
        || (lo < hi && v.bins.len() < target && v.bin_adjustment.is_none())
        || v.bins[0].left != lo
        || v.bins.last().is_none_or(|b| b.right != hi)
    {
        return Err(invalid_analysis());
    }
    let (mut rows, mut n) = (0, 0);
    let mut previous = lo;
    for bin in &v.bins {
        if !bin.left.is_finite()
            || !bin.right.is_finite()
            || bin.left != previous
            || (lo < hi && bin.left >= bin.right)
            || bin.left > bin.right
        {
            return Err(invalid_analysis());
        }
        count(bin.rows, bin.frequency, weighted)?;
        rows = row_sum([rows, bin.rows])?;
        n = frequency_sum([n, bin.frequency])?;
        previous = bin.right;
    }
    if rows != v.valid_rows || n != v.n {
        return Err(invalid_analysis());
    }
    Ok(())
}
pub(super) fn scatter(v: &ComputedScatter) -> AppResult<()> {
    let infos = common(
        v.revision,
        RequestRef::Scatter(&v.request),
        &v.columns,
        v.source_rows,
        &v.diagnostics,
    )?;
    let weighted = v.request.frequency_weight.is_some();
    cohort(
        v.source_rows,
        &v.diagnostics,
        v.valid_rows,
        v.n,
        v.missing_rows,
        v.missing_n,
        weighted,
    )?;
    let raw = [
        RawMetadata::new(infos[&v.request.x])?,
        RawMetadata::new(infos[&v.request.y])?,
    ];
    let bounds = [
        range(&v.ranges[0], v.valid_rows)?,
        range(&v.ranges[1], v.valid_rows)?,
    ];
    for (axis, bound) in bounds.iter().enumerate() {
        if let Some([lo, hi]) = bound {
            raw[axis].present(&Cell::Number(*lo))?;
            raw[axis].present(&Cell::Number(*hi))?;
        }
    }
    if v.points.len() != v.valid_rows.min(MAX_PLOT_POINTS)
        || v.reduced != (v.valid_rows > MAX_PLOT_POINTS)
    {
        return Err(invalid_analysis());
    }
    let mut previous = None;
    let mut n = 0;
    let mut extrema = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
    for p in &v.points {
        if p.row >= v.source_rows || previous.is_some_and(|r| r >= p.row) {
            return Err(invalid_analysis());
        }
        previous = Some(p.row);
        count(1, p.frequency, weighted)?;
        n = frequency_sum([n, p.frequency])?;
        for (axis, value) in [p.x, p.y].into_iter().enumerate() {
            raw[axis].present(&Cell::Number(value))?;
            let [lo, hi] = bounds[axis].ok_or_else(invalid_analysis)?;
            if !value.is_finite() || value < lo || value > hi {
                return Err(invalid_analysis());
            }
            extrema[axis][0] = extrema[axis][0].min(value);
            extrema[axis][1] = extrema[axis][1].max(value);
        }
    }
    if n > v.n || v.n - n < (v.valid_rows - v.points.len()) as u64 || (!v.reduced && n != v.n) {
        return Err(invalid_analysis());
    }
    if !v.reduced
        && v.valid_rows > 0
        && (bounds[0] != Some(extrema[0]) || bounds[1] != Some(extrema[1]))
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
fn group(g: &BoxplotGroup, weighted: bool) -> AppResult<()> {
    estimate(&g.mean)?;
    if g.mean.value.is_some_and(|mean| {
        g.n == 0 || Some(mean) < g.minimum.value || Some(mean) > g.maximum.value
    }) {
        return Err(invalid_analysis());
    }
    count(g.rows, g.n, weighted)?;
    count(g.outlier_rows, g.outlier_n, weighted)?;
    if g.outlier_rows > g.rows || g.outlier_n > g.n {
        return Err(invalid_analysis());
    }
    count(g.rows - g.outlier_rows, g.n - g.outlier_n, weighted)?;
    let estimates = [
        &g.minimum,
        &g.whiskers[0],
        &g.q1,
        &g.median,
        &g.q3,
        &g.whiskers[1],
        &g.maximum,
    ];
    for e in estimates {
        estimate(e)?;
    }
    if g.n == 0 {
        if estimates.iter().any(|e| e.value.is_some()) {
            return Err(invalid_analysis());
        }
        return Ok(());
    }
    let values = estimates.map(|e| e.value);
    let summaries = [
        g.minimum.value,
        g.q1.value,
        g.median.value,
        g.q3.value,
        g.maximum.value,
    ];
    if summaries.iter().any(Option::is_none) || summaries.windows(2).any(|p| p[0] > p[1]) {
        return Err(invalid_analysis());
    }
    let q1 = g.q1.value.ok_or_else(invalid_analysis)?;
    let q3 = g.q3.value.ok_or_else(invalid_analysis)?;
    if g.whiskers.iter().all(|e| e.value.is_none()) {
        if g.outlier_rows != g.rows
            || g.outlier_n != g.n
            || inside_box_fences(g.minimum.value.ok_or_else(invalid_analysis)?, q1, q3)
            || inside_box_fences(g.maximum.value.ok_or_else(invalid_analysis)?, q1, q3)
        {
            return Err(invalid_analysis());
        }
        return Ok(());
    }
    if values.iter().any(Option::is_none)
        || g.outlier_rows == g.rows
        || g.outlier_n == g.n
        || [
            g.minimum.value,
            g.whiskers[0].value,
            g.whiskers[1].value,
            g.maximum.value,
        ]
        .windows(2)
        .any(|p| p[0] > p[1])
    {
        return Err(invalid_analysis());
    }
    if (g.outlier_rows == 0
        && (g.whiskers[0].value != g.minimum.value || g.whiskers[1].value != g.maximum.value))
        || (g.outlier_rows > 0
            && g.whiskers[0].value == g.minimum.value
            && g.whiskers[1].value == g.maximum.value)
        || (g.rows == 1 && values.iter().any(|v| *v != values[0]))
    {
        return Err(invalid_analysis());
    }
    for whisker in &g.whiskers {
        if !inside_box_fences(whisker.value.ok_or_else(invalid_analysis)?, q1, q3) {
            return Err(invalid_analysis());
        }
    }
    if (g.minimum.value != g.whiskers[0].value
        && inside_box_fences(g.minimum.value.ok_or_else(invalid_analysis)?, q1, q3))
        || (g.maximum.value != g.whiskers[1].value
            && inside_box_fences(g.maximum.value.ok_or_else(invalid_analysis)?, q1, q3))
    {
        return Err(invalid_analysis());
    }
    Ok(())
}
pub(super) fn boxplot(v: &ComputedBoxplot) -> AppResult<()> {
    let infos = common(
        v.revision,
        RequestRef::Boxplot(&v.request),
        &v.columns,
        v.source_rows,
        &v.diagnostics,
    )?;
    let weighted = v.request.frequency_weight.is_some();
    count(v.missing_group_rows, v.missing_group_n, weighted)?;
    count(v.missing_outcome_rows, v.missing_outcome_n, weighted)?;
    let missing_rows = row_sum([v.missing_group_rows, v.missing_outcome_rows])?;
    let missing_n = frequency_sum([v.missing_group_n, v.missing_outcome_n])?;
    let group_info = v.request.group.map(|id| infos[&id]);
    let raw_group = group_info.map(RawMetadata::new).transpose()?;
    let raw_outcome = RawMetadata::new(infos[&v.request.outcome])?;
    if group_info.is_none()
        && (v.groups.len() != 1 || v.missing_group_rows != 0 || v.missing_group_n != 0)
    {
        return Err(invalid_analysis());
    }
    let (mut rows, mut n, mut outlier_rows, mut outlier_n) = (0, 0, 0, 0);
    let mut previous = None;
    for g in &v.groups {
        group(g, weighted)?;
        for raw in [&g.minimum, &g.maximum, &g.whiskers[0], &g.whiskers[1]] {
            if let Some(value) = raw.value {
                raw_outcome.present(&Cell::Number(value))?;
            }
        }
        if group_info.is_some() {
            let value = g.value.as_ref().ok_or_else(invalid_analysis)?;
            raw_group
                .as_ref()
                .ok_or_else(invalid_analysis)?
                .category(value, g.value_label_index)?;
            if g.n == 0 || previous.is_some_and(|p| compare(p, value) != Some(Ordering::Less)) {
                return Err(invalid_analysis());
            }
            previous = Some(value);
        } else if g.value.is_some() || g.value_label_index.is_some() {
            return Err(invalid_analysis());
        }
        rows = row_sum([rows, g.rows])?;
        n = frequency_sum([n, g.n])?;
        outlier_rows = row_sum([outlier_rows, g.outlier_rows])?;
        outlier_n = frequency_sum([outlier_n, g.outlier_n])?;
    }
    partition(v.source_rows, &v.diagnostics, rows, missing_rows)?;
    frequency_sum([n, missing_n])?;
    if (n == 0) != v.empty_reason.is_some()
        || v.empty_reason.as_ref().is_some_and(|s| s.trim().is_empty())
        || v.points.len() != outlier_rows.min(MAX_PLOT_POINTS)
        || v.reduced != (outlier_rows > MAX_PLOT_POINTS)
    {
        return Err(invalid_analysis());
    }
    let mut seen = BTreeSet::new();
    let mut previous: Option<&BoxplotPoint> = None;
    let mut counts = vec![(0usize, 0u64, f64::INFINITY, f64::NEG_INFINITY); v.groups.len()];
    for p in &v.points {
        let g = v.groups.get(p.group_index).ok_or_else(invalid_analysis)?;
        raw_outcome.present(&Cell::Number(p.value))?;
        if inside_box_fences(
            p.value,
            g.q1.value.ok_or_else(invalid_analysis)?,
            g.q3.value.ok_or_else(invalid_analysis)?,
        ) {
            return Err(invalid_analysis());
        }
        if p.row >= v.source_rows
            || !seen.insert(p.row)
            || !p.value.is_finite()
            || p.value < g.minimum.value.ok_or_else(invalid_analysis)?
            || p.value > g.maximum.value.ok_or_else(invalid_analysis)?
        {
            return Err(invalid_analysis());
        }
        if previous.is_some_and(|q| {
            q.group_index > p.group_index
                || (q.group_index == p.group_index
                    && (q.value > p.value || (q.value == p.value && q.row >= p.row)))
        }) {
            return Err(invalid_analysis());
        }
        previous = Some(p);
        count(1, p.frequency, weighted)?;
        let (rows, n, lo, hi) = &mut counts[p.group_index];
        *rows = row_sum([*rows, 1])?;
        *n = frequency_sum([*n, p.frequency])?;
        *lo = lo.min(p.value);
        *hi = hi.max(p.value);
    }
    for (g, (rows, n, lo, hi)) in v.groups.iter().zip(counts) {
        if !v.reduced && g.n > 0 {
            let q1 = g.q1.value.ok_or_else(invalid_analysis)?;
            let q3 = g.q3.value.ok_or_else(invalid_analysis)?;
            for (bound, retained) in [(&g.minimum, lo), (&g.maximum, hi)] {
                if !inside_box_fences(bound.value.ok_or_else(invalid_analysis)?, q1, q3)
                    && bound.value != Some(retained)
                {
                    return Err(invalid_analysis());
                }
            }
        }
        if rows > g.outlier_rows
            || n > g.outlier_n
            || g.outlier_n - n < (g.outlier_rows - rows) as u64
            || (!v.reduced && (rows != g.outlier_rows || n != g.outlier_n))
        {
            return Err(invalid_analysis());
        }
    }
    Ok(())
}
