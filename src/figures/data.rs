//! Figure geometry uses captured results, never the current dataset.
use super::{CategoryOrder, FigureKind, FigureSettings, FrequencyMetric};
use crate::{
    analysis::*,
    charts::CategoryCaption,
    data::{Cell, ColumnId},
    error::{AppError, AppResult},
    operation::JobControl,
    project::AnalysisRun,
};
pub(super) struct Data {
    pub title: String,
    pub x: String,
    pub y: String,
    pub note: String,
    pub visible_note: String,
    pub bounds: [[f64; 2]; 2],
    pub marks: Vec<Mark>,
    pub categories: Vec<String>,
    pub categorical: bool,
}
pub(super) enum Mark {
    Point(f64, f64),
    Bin(f64, f64, f64),
    Bar(usize, f64),
    Box(usize, f64, f64, f64, [Option<f64>; 2]),
}
fn unavailable() -> AppError {
    AppError::invalid(
        "figure",
        "This captured result has no available drawing values.",
    )
}
fn range(e: &[Estimate; 2]) -> AppResult<[f64; 2]> {
    Ok([
        e[0].value.ok_or_else(unavailable)?,
        e[1].value.ok_or_else(unavailable)?,
    ])
}
fn name(run: &AnalysisRun, id: ColumnId) -> AppResult<String> {
    let name = run
        .analysis
        .columns()
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.name.as_str());
    if name.is_some_and(|name| name.len() > 4096) {
        return Err(AppError::invalid(
            "figure",
            "The captured variable name is too long for a report figure.",
        ));
    }
    Ok(name.map_or_else(|| format!("Column {}", id.0), str::to_owned))
}
fn category(run: &AnalysisRun, id: ColumnId, value: &Cell, index: Option<usize>) -> String {
    let raw = CategoryCaption(value).to_string();
    index
        .and_then(|i| {
            run.analysis
                .columns()
                .iter()
                .find(|c| c.id == id)
                .and_then(|c| c.value_labels.get(i))
        })
        .map_or(raw.clone(), |l| format!("{} · {}", l.label, raw))
}
fn preflight(
    run: &AnalysisRun,
    kind: &FigureKind,
    s: &FigureSettings,
    c: &JobControl,
) -> AppResult<()> {
    let (count, bytes) = match (&run.analysis, kind) {
        (CompletedAnalysis::Frequencies(v), FigureKind::Frequency(id)) => {
            let t = v
                .tables
                .iter()
                .find(|t| t.column.0 == *id)
                .ok_or_else(unavailable)?;
            let mut bytes = 0usize;
            for (i, r) in t.categories.iter().enumerate() {
                c.checkpoint(i, Some(t.categories.len()))?;
                bytes = bytes.saturating_add(match &r.value {
                    Cell::Text(t) => t.len(),
                    _ => 32,
                });
                if let Some(label) = r.value_label_index.and_then(|i| {
                    v.columns
                        .iter()
                        .find(|col| col.id == t.column)
                        .and_then(|col| col.value_labels.get(i))
                }) {
                    bytes = bytes.saturating_add(label.label.len());
                }
            }
            (t.categories.len(), bytes)
        }
        (CompletedAnalysis::Boxplot(v), FigureKind::Boxplot) => {
            let mut bytes = 0usize;
            for (i, g) in v.groups.iter().enumerate() {
                c.checkpoint(i, Some(v.groups.len()))?;
                bytes = bytes.saturating_add(match &g.value {
                    Some(Cell::Text(t)) => t.len(),
                    _ => 32,
                });
                if let Some(label) = g.value_label_index.and_then(|i| {
                    v.columns
                        .iter()
                        .find(|col| Some(col.id) == v.request.group)
                        .and_then(|col| col.value_labels.get(i))
                }) {
                    bytes = bytes.saturating_add(label.label.len());
                }
            }
            (v.groups.len(), bytes)
        }
        _ => (0, 0),
    };
    let available = if matches!(kind, FigureKind::Frequency(_)) && s.horizontal {
        s.height_mm * 72. / 25.4
    } else {
        s.width_mm * 72. / 25.4
    };
    if count as f64 * s.font_size * 1.4 > available {
        return Err(AppError::invalid("figure","This figure is too small for all categories. Increase its dimensions or reduce the font size; no categories were omitted."));
    }
    let text = bytes
        .saturating_add(s.title.len())
        .saturating_add(s.subtitle.len())
        .saturating_add(s.caption.len())
        .saturating_add(s.x_label.len())
        .saturating_add(s.y_label.len());
    if text > 16_384 {
        return Err(AppError::invalid("figure","The complete category labels exceed the figure's text budget. Shorten their wording or use the exact frequency table; no categories were omitted."));
    }
    Ok(())
}
pub(super) fn captured(
    run: &AnalysisRun,
    kind: &FigureKind,
    s: &FigureSettings,
    c: &JobControl,
) -> AppResult<Data> {
    c.check_cancelled()?;
    preflight(run, kind, s, c)?;
    let mut d = Data {
        title: String::new(),
        x: String::new(),
        y: String::new(),
        note: String::new(),
        visible_note: String::new(),
        bounds: [[0., 1.], [0., 1.]],
        marks: Vec::new(),
        categories: Vec::new(),
        categorical: false,
    };
    match (&run.analysis, kind) {
        (CompletedAnalysis::Histogram(v), FigureKind::Histogram) => {
            d.title = "Histogram".into();
            d.x = name(run, v.request.variable)?;
            d.y = "Frequency".into();
            d.bounds = [
                range(&v.range)?,
                [
                    0.,
                    v.bins.iter().map(|b| b.frequency).max().unwrap_or(0) as f64,
                ],
            ];
            d.note=format!("Valid: {} source rows; frequency {}. Missing: {} rows; frequency {}. Full-cohort bins.",v.valid_rows,v.n,v.missing_rows,v.missing_n);
            if let Some(reason) = &v.bin_adjustment {
                d.note.push(' ');
                d.note.push_str(reason);
            }
            for b in &v.bins {
                c.check_cancelled()?;
                d.marks.push(Mark::Bin(b.left, b.right, b.frequency as f64));
            }
        }
        (CompletedAnalysis::Scatter(v), FigureKind::Scatter) => {
            d.title = "Scatterplot".into();
            d.x = name(run, v.request.x)?;
            d.y = name(run, v.request.y)?;
            d.bounds = [range(&v.ranges[0])?, range(&v.ranges[1])?];
            d.note=format!("Showing {} of {} complete-pair source rows; frequency {}. Missing: {} rows; frequency {}. Axes use full-cohort bounds.{}",v.points.len(),v.valid_rows,v.n,v.missing_rows,v.missing_n,if v.reduced{" Drawing reduced."}else{""});
            for p in &v.points {
                c.check_cancelled()?;
                d.marks.push(Mark::Point(p.x, p.y));
            }
        }
        (CompletedAnalysis::Boxplot(v), FigureKind::Boxplot) => {
            d.title = "Boxplot".into();
            d.x = match v.request.group {
                Some(id) => name(run, id)?,
                None => "Group".into(),
            };
            d.y = name(run, v.request.outcome)?;
            d.categorical = true;
            let lo = v
                .groups
                .iter()
                .filter_map(|g| g.minimum.value)
                .reduce(f64::min)
                .ok_or_else(unavailable)?;
            let hi = v
                .groups
                .iter()
                .filter_map(|g| g.maximum.value)
                .reduce(f64::max)
                .ok_or_else(unavailable)?;
            d.bounds = [[0., v.groups.len() as f64], [lo, hi]];
            for (i, g) in v.groups.iter().enumerate() {
                c.check_cancelled()?;
                d.categories.push(match (v.request.group, &g.value) {
                    (Some(id), Some(value)) => category(run, id, value, g.value_label_index),
                    _ => "All eligible outcomes".into(),
                });
                if let (Some(q1), Some(m), Some(q3)) = (g.q1.value, g.median.value, g.q3.value) {
                    d.marks.push(Mark::Box(
                        i,
                        q1,
                        m,
                        q3,
                        [g.whiskers[0].value, g.whiskers[1].value],
                    ));
                }
            }
            for p in &v.points {
                c.check_cancelled()?;
                d.marks
                    .push(Mark::Point(p.group_index as f64 + 0.5, p.value));
            }
            d.note=format!("Full-cohort quartiles and observed whiskers. Outliers: {} of {} source rows drawn. Missing group: {} rows, frequency {}; missing outcome: {} rows, frequency {}.{}",v.points.len(),v.groups.iter().map(|g|g.outlier_rows).sum::<usize>(),v.missing_group_rows,v.missing_group_n,v.missing_outcome_rows,v.missing_outcome_n,if v.reduced{" Outlier drawing reduced."}else{""});
        }
        (CompletedAnalysis::Frequencies(v), FigureKind::Frequency(id)) => {
            let t = v
                .tables
                .iter()
                .find(|t| t.column.0 == *id)
                .ok_or_else(unavailable)?;
            if t.categories.is_empty() {
                return Err(unavailable());
            }
            d.title = "Frequency distribution".into();
            d.x = name(run, t.column)?;
            d.y = match s.metric {
                FrequencyMetric::Count => "Frequency",
                FrequencyMetric::ValidPercent => "Valid frequency (%)",
                FrequencyMetric::AllPercent => "All frequency (%)",
            }
            .into();
            d.categorical = true;
            let mut rows: Vec<_> = t.categories.iter().collect();
            match s.order {
                CategoryOrder::Original => {}
                CategoryOrder::Value => rows.sort_by(|a, b| match (&a.value, &b.value) {
                    (Cell::Number(a), Cell::Number(b)) => a.total_cmp(b),
                    _ => CategoryCaption(&a.value)
                        .to_string()
                        .cmp(&CategoryCaption(&b.value).to_string()),
                }),
                CategoryOrder::Frequency => rows.sort_by_key(|r| std::cmp::Reverse(r.frequency)),
            }
            let mut max: f64 = 0.;
            for (i, r) in rows.iter().enumerate() {
                c.check_cancelled()?;
                let value = match s.metric {
                    FrequencyMetric::Count => r.frequency as f64,
                    FrequencyMetric::ValidPercent => {
                        r.valid_percent.value.ok_or_else(unavailable)?
                    }
                    FrequencyMetric::AllPercent => r.percent.value.ok_or_else(unavailable)?,
                };
                max = max.max(value);
                d.categories
                    .push(category(run, t.column, &r.value, r.value_label_index));
                d.marks.push(Mark::Bar(i, value));
            }
            d.bounds = [[0., rows.len() as f64], [0., max]];
            d.note=format!("Valid: {} rows; frequency {}. Missing: {} rows; frequency {}. {}",t.valid_rows,t.valid_n,t.missing_rows,t.missing_n,match s.metric{FrequencyMetric::Count=>"Counts are literal frequencies; missing responses excluded from bars.",FrequencyMetric::ValidPercent=>"Denominator: valid frequency; missing responses excluded.",FrequencyMetric::AllPercent=>"Denominator: valid plus missing frequency; missing responses excluded from bars."});
        }
        (CompletedAnalysis::Regression(v), FigureKind::Residual | FigureKind::Qq) => {
            let RegressionOutcome::Fitted(f) = &v.outcome else {
                return Err(unavailable());
            };
            let p = &f.residual_diagnostics;
            let qq = matches!(kind, FigureKind::Qq);
            d.title = if qq {
                "Normal Q–Q plot"
            } else {
                "Residual versus fitted values"
            }
            .into();
            d.x = if qq {
                "Theoretical normal quantile"
            } else {
                "Fitted value"
            }
            .into();
            d.y = "Raw residual".into();
            d.bounds = [
                range(if qq {
                    &p.theoretical_range
                } else {
                    &p.fitted_range
                })?,
                range(&p.residual_range)?,
            ];
            if qq {
                for point in &p.qq_points {
                    c.check_cancelled()?;
                    if let (Some(x), Some(y)) = (point.theoretical.value, point.residual.value) {
                        d.marks.push(Mark::Point(x, y));
                    }
                }
            } else {
                for point in &p.residual_points {
                    c.check_cancelled()?;
                    if let (Some(x), Some(y)) = (point.fitted.value, point.residual.value) {
                        d.marks.push(Mark::Point(x, y));
                    }
                }
            }
            d.note=format!("Fitted cohort: {} source rows; literal replicated frequency {}. Showing {} retained diagnostic points. Full-cohort bounds.{}",v.fitted_rows,v.frequency,d.marks.len(),if if qq{p.qq_reduced}else{p.residual_reduced}{" Drawing reduced."}else{""});
        }
        (CompletedAnalysis::Describe(v), FigureKind::Distribution(id)) => {
            let p = v
                .plots
                .iter()
                .find(|p| p.column.0 == *id)
                .ok_or_else(unavailable)?;
            let row = v
                .rows
                .iter()
                .find(|r| r.column == p.column)
                .ok_or_else(unavailable)?;
            let lo = row.estimates.get(&Statistic::Minimum).and_then(|e| e.value);
            let hi = row.estimates.get(&Statistic::Maximum).and_then(|e| e.value);
            // shortcut: older reduced captures lack extrema; rerun with minimum and maximum before exporting.
            let bounds=match(lo,hi){(Some(lo),Some(hi))=>[lo,hi],_ if !p.reduced=>[p.points.iter().map(|p|p.value).reduce(f64::min).ok_or_else(unavailable)?,p.points.iter().map(|p|p.value).reduce(f64::max).ok_or_else(unavailable)?],_=>return Err(AppError::invalid("figure","Rerun descriptives with minimum and maximum to retain full-cohort drawing bounds."))};
            d.title = "Observed distribution".into();
            d.x = name(run, p.column)?;
            d.y = "Retained rows".into();
            let points = crate::charts::stacked_rows(p.points.iter().map(|point| point.value), c)?;
            let max_stack = points.iter().map(|point| point[1]).fold(0., f64::max);
            d.bounds = [bounds, [0., max_stack + 1.]];
            d.marks
                .extend(points.into_iter().map(|[x, y]| Mark::Point(x, y)));
            d.note=format!("{} Showing {} of {} eligible source rows. Valid frequency {}; missing frequency {}.{}",p.label,p.points.len(),p.source_rows,row.valid_n,row.missing_n,if p.reduced{" Drawing reduced."}else{""});
        }
        _ => {
            return Err(AppError::invalid(
                "figure",
                "The figure does not belong to this captured analysis.",
            ))
        }
    }
    if matches!(kind, FigureKind::Distribution(_)) {
        d.note
            .push_str(" Each dot is one retained source row; equal values stack vertically.");
    }
    d.visible_note = match (&run.analysis, kind) {
        (CompletedAnalysis::Histogram(v), _) => concise(v.valid_rows, v.n, v.missing_rows, None),
        (CompletedAnalysis::Scatter(v), _) => concise(
            v.valid_rows,
            v.n,
            v.missing_rows,
            v.reduced.then_some(v.points.len()),
        ),
        (CompletedAnalysis::Boxplot(v), _) => concise(
            v.groups.iter().map(|g| g.rows).sum(),
            v.groups.iter().map(|g| g.n).sum(),
            v.missing_group_rows + v.missing_outcome_rows,
            v.reduced.then_some(v.points.len()),
        ),
        (CompletedAnalysis::Frequencies(v), FigureKind::Frequency(id)) => {
            let t = v
                .tables
                .iter()
                .find(|t| t.column.0 == *id)
                .ok_or_else(unavailable)?;
            let mut note = concise(t.valid_rows, t.valid_n, t.missing_rows, None);
            match s.metric {
                FrequencyMetric::Count => {}
                FrequencyMetric::ValidPercent => {
                    note.push_str(" Percent denominator: valid frequency.")
                }
                FrequencyMetric::AllPercent => {
                    note.push_str(" Percent denominator: valid plus missing frequency.")
                }
            };
            note
        }
        (CompletedAnalysis::Regression(v), _) => {
            let RegressionOutcome::Fitted(f) = &v.outcome else {
                return Err(unavailable());
            };
            let reduced = if matches!(kind, FigureKind::Qq) {
                f.residual_diagnostics.qq_reduced
            } else {
                f.residual_diagnostics.residual_reduced
            };
            concise(
                v.fitted_rows,
                v.frequency,
                v.diagnostics.complete_case_rows,
                reduced.then_some(d.marks.len()),
            )
        }
        (CompletedAnalysis::Describe(v), FigureKind::Distribution(id)) => {
            let p = v
                .plots
                .iter()
                .find(|p| p.column.0 == *id)
                .ok_or_else(unavailable)?;
            let row = v
                .rows
                .iter()
                .find(|r| r.column == p.column)
                .ok_or_else(unavailable)?;
            let mut note = concise(
                p.source_rows,
                row.valid_n,
                0,
                p.reduced.then_some(p.points.len()),
            );
            if row.missing_n > 0 {
                note.push_str(&format!(" Missing frequency: {}.", row.missing_n));
            }
            note.push_str(" Each dot is one retained source row; equal values stack vertically.");
            note
        }
        _ => String::new(),
    };
    let diag = match &run.analysis {
        CompletedAnalysis::Histogram(v) => Some(&v.diagnostics),
        CompletedAnalysis::Scatter(v) => Some(&v.diagnostics),
        CompletedAnalysis::Boxplot(v) => Some(&v.diagnostics),
        CompletedAnalysis::Frequencies(v) => Some(&v.diagnostics),
        CompletedAnalysis::Regression(v) => Some(&v.diagnostics),
        CompletedAnalysis::Describe(v) => Some(&v.diagnostics),
        _ => None,
    };
    let weight = match &run.analysis {
        CompletedAnalysis::Histogram(v) => v.request.frequency_weight,
        CompletedAnalysis::Scatter(v) => v.request.frequency_weight,
        CompletedAnalysis::Boxplot(v) => v.request.frequency_weight,
        CompletedAnalysis::Frequencies(v) => v.request.frequency_weight,
        CompletedAnalysis::Regression(v) => v.request.frequency_weight,
        CompletedAnalysis::Describe(v) => v.request.frequency_weight,
        _ => None,
    };
    if let Some(id) = weight {
        d.note.push_str(&format!(
            " Frequency weights are literal replication using {}.",
            name(run, id)?
        ));
    }
    if let Some(diag) = diag {
        d.note.push_str(&format!(
            " Excluded rows: {} filtered, {} missing weight, {} zero weight, {} incomplete cases.",
            diag.filtered_rows,
            diag.missing_weight_rows,
            diag.zero_weight_rows,
            diag.complete_case_rows
        ));
    }
    Ok(d)
}

fn concise(rows: usize, frequency: u64, missing: usize, reduced: Option<usize>) -> String {
    let mut note = format!("n = {rows} source rows");
    if frequency != rows as u64 {
        note.push_str(&format!("; frequency {frequency}"));
    }
    note.push('.');
    if missing > 0 {
        note.push_str(&format!(" Missing: {missing} rows."));
    }
    if let Some(shown) = reduced {
        note.push_str(&format!(
            " Drawing reduced: {shown} retained points; bounds use the full cohort."
        ));
    }
    note
}
#[cfg(test)]
mod caption_tests {
    use super::*;
    #[test]
    fn visible_caption_is_short_but_full_provenance_remains() {
        let dataset = crate::data::Dataset::new(vec![crate::data::Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), Some(2.), Some(3.)],
        )
        .unwrap()])
        .unwrap();
        let result = analyze_with_budget(
            &dataset,
            &AnalysisRequest::Boxplot(BoxplotRequest {
                outcome: ColumnId(1),
                group: None,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        let run = AnalysisRun {
            id: crate::operation::RunId(1),
            analysis: result,
        };
        let d = captured(
            &run,
            &FigureKind::Boxplot,
            &FigureSettings::default(),
            &JobControl::default(),
        )
        .unwrap();
        assert_eq!(d.visible_note, "n = 3 source rows.");
        assert!(d
            .note
            .contains("Full-cohort quartiles and observed whiskers"));
        assert!(d.note.contains("0 filtered"));
        assert!(d.note.contains("Missing group: 0"));
    }
    #[test]
    fn distribution_stacks_equal_values_without_moving_the_data_axis() {
        let values = vec![
            Some(-0.),
            Some(0.),
            Some(1.),
            Some(1. + f64::EPSILON),
            Some(1.),
        ];
        let dataset = crate::data::Dataset::new(vec![crate::data::Column::numeric(
            ColumnId(1),
            "Score",
            values.clone(),
        )
        .unwrap()])
        .unwrap();
        let c = JobControl::default();
        let result = analyze_with_budget(
            &dataset,
            &AnalysisRequest::Describe(DescribeRequest {
                columns: vec![ColumnId(1)],
                statistics: vec![Statistic::Minimum, Statistic::Maximum],
                missing_policy: MissingPolicy::PerVariable,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &c,
            &[],
            4096,
        )
        .unwrap();
        let run = AnalysisRun {
            id: crate::operation::RunId(1),
            analysis: result,
        };
        let d = captured(
            &run,
            &FigureKind::Distribution(1),
            &FigureSettings::default(),
            &c,
        )
        .unwrap();
        let points: Vec<_> = d
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Point(x, y) => Some([*x, *y]),
                _ => None,
            })
            .collect();
        assert_eq!(
            points,
            vec![
                [-0., 1.],
                [0., 2.],
                [1., 1.],
                [1. + f64::EPSILON, 1.],
                [1., 2.]
            ]
        );
        assert_eq!(d.bounds[1], [0., 3.]);
        assert!(d.y.contains("Retained"));
        assert!(d.visible_note.contains("Each dot"));
    }
    #[test]
    fn weighted_missing_and_reduced_disclosures_are_never_hidden() {
        let note = concise(10, 25, 2, Some(4));
        assert!(note.contains("frequency 25"));
        assert!(note.contains("Missing: 2 rows"));
        assert!(note.contains("Drawing reduced: 4 retained points"));
        assert!(note.contains("full cohort"));
        assert!(!concise(10, 10, 0, None).contains("Missing"));
    }
}
