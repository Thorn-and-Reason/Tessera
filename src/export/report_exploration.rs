//! Complete captured tables and SVG; display reductions never supply summaries or bounds.
use super::{
    number,
    report::{column_name, text},
    report_analyses::estimate,
};
use crate::{
    analysis::*,
    charts::ChartScale,
    data::{Cell, ColumnId},
    error::AppResult,
    operation::JobControl,
    project::AnalysisRun,
};
use std::io::Write;
pub(crate) const FREQUENCY_METHOD: &str = "Each variable is counted independently. Percent uses valid plus missing frequency; valid percent uses valid frequency. Numeric cumulative percentages follow ascending values.";
pub(crate) const HISTOGRAM_METHOD: &str = "Bins use the complete cohort. Intervals include the left boundary; the final interval also includes the maximum. Constant data has one point-mass bin.";
pub(crate) const SCATTER_METHOD: &str = "Complete pairs only. Each marker is one source row; frequency records literal replication. Full-cohort ranges include undisplayed values. Reduced points are deterministic evenly spaced eligible rows, including both endpoints.";
pub(crate) const BOXPLOT_METHOD: &str = "Quartiles use frequency-replicated Hyndman–Fan type 7. Whiskers are actual observations within inclusive 1.5-IQR fences. All summaries and outlier counts use the complete cohort. Reduced outliers follow group, value and source-row order.";
pub(crate) fn bin_setting(bins: HistogramBins) -> String {
    match bins {
        HistogramBins::Automatic => "Automatic (Sturges)".into(),
        HistogramBins::Count(n) => n.to_string(),
    }
}
fn method(w: &mut dyn Write, s: &str) -> AppResult<()> {
    w.write_all(b"<p>")?;
    text(w, s)?;
    w.write_all(b"</p>")?;
    Ok(())
}
fn start_table(w: &mut dyn Write, title: &str, headings: &[&str]) -> AppResult<()> {
    w.write_all(b"<table><caption>")?;
    text(w, title)?;
    w.write_all(b"</caption><thead><tr>")?;
    for h in headings {
        w.write_all(b"<th scope=\"col\">")?;
        text(w, h)?;
        w.write_all(b"</th>")?;
    }
    w.write_all(b"</tr></thead><tbody>")?;
    Ok(())
}
fn end_table(w: &mut dyn Write) -> AppResult<()> {
    w.write_all(b"</tbody></table>")?;
    Ok(())
}
fn estimates(w: &mut dyn Write, values: &[&Estimate]) -> AppResult<()> {
    for e in values {
        w.write_all(b"<td>")?;
        estimate(w, e)?;
        w.write_all(b"</td>")?;
    }
    Ok(())
}
fn escaped_caption(w: &mut dyn Write, value: &Cell) -> AppResult<()> {
    struct Html<'a> {
        writer: &'a mut dyn Write,
        error: Option<crate::error::AppError>,
    }
    impl std::fmt::Write for Html<'_> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            text(self.writer, s).map_err(|e| {
                self.error = Some(e);
                std::fmt::Error
            })
        }
    }
    w.write_all(b"<span style=\"white-space:pre-wrap\">")?;
    let mut html = Html {
        writer: w,
        error: None,
    };
    let result = std::fmt::write(
        &mut html,
        format_args!("{}", crate::charts::CategoryCaption(value)),
    );
    if let Some(error) = html.error {
        return Err(error);
    }
    result
        .map_err(|_| crate::error::AppError::invalid("report", "Could not format the category."))?;
    w.write_all(b"</span>")?;
    Ok(())
}
fn category(
    w: &mut dyn Write,
    run: &AnalysisRun,
    id: ColumnId,
    value: &Cell,
    index: Option<usize>,
) -> AppResult<()> {
    escaped_caption(w, value)?;
    if let Some(label) = index.and_then(|i| {
        run.analysis
            .columns()
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.value_labels.get(i))
    }) {
        text(w, " · ")?;
        text(w, &label.label)?;
    }
    Ok(())
}
pub(super) fn frequencies(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedFrequencies,
    c: &JobControl,
) -> AppResult<()> {
    method(w, FREQUENCY_METHOD)?;
    for t in &v.tables {
        c.check_cancelled()?;
        w.write_all(b"<h3>")?;
        column_name(w, run, t.column)?;
        w.write_all(b"</h3>")?;
        write!(w,"<p>Valid rows: {}; valid frequency: {}. Missing rows: {}; missing frequency: {}. Missing percent: ",t.valid_rows,t.valid_n,t.missing_rows,t.missing_n)?;
        estimate(w, &t.missing_percent)?;
        w.write_all(b".</p>")?;
        start_table(
            w,
            "Complete frequency table",
            &[
                "Typed value and label",
                "Rows",
                "Frequency",
                "Percent",
                "Valid percent",
                "Cumulative frequency",
                "Cumulative valid percent",
            ],
        )?;
        for r in &t.categories {
            c.check_cancelled()?;
            w.write_all(b"<tr><th scope=\"row\">")?;
            category(w, run, t.column, &r.value, r.value_label_index)?;
            write!(w, "</th><td>{}</td><td>{}</td>", r.rows, r.frequency)?;
            estimates(w, &[&r.percent, &r.valid_percent])?;
            w.write_all(b"<td>")?;
            if let Some(n) = r.cumulative_frequency {
                write!(w, "{n}")?;
            } else {
                w.write_all(b"Not applicable")?;
            }
            w.write_all(b"</td><td>")?;
            if let Some(e) = &r.cumulative_percent {
                estimate(w, e)?;
            } else {
                w.write_all(b"Not applicable")?;
            }
            w.write_all(b"</td></tr>")?;
        }
        end_table(w)?;
    }
    Ok(())
}
fn bounds(range: &[Estimate; 2]) -> Option<(f64, f64)> {
    Some((range[0].value?, range[1].value?))
}
use super::report_charts::{left as svg_left, right as svg_right};
fn svg_start(
    w: &mut dyn Write,
    run: &AnalysisRun,
    title: &str,
    desc: &str,
    xs: &ChartScale,
    ys: &ChartScale,
) -> AppResult<()> {
    super::report_charts::svg_description_start(w, run.id, "", title, desc)?;
    if let CompletedAnalysis::Scatter(v) = &run.analysis {
        for (label, e) in [
            (" X minimum: ", &v.ranges[0][0]),
            (" X maximum: ", &v.ranges[0][1]),
            (" Y minimum: ", &v.ranges[1][0]),
            (" Y maximum: ", &v.ranges[1][1]),
        ] {
            w.write_all(label.as_bytes())?;
            if let Some(value) = e.value {
                number(w, value)?;
            }
        }
    }
    super::report_charts::svg_plot_area(w, xs, ys)
}

fn axes(
    w: &mut dyn Write,
    run: &AnalysisRun,
    x: Option<ColumnId>,
    y: Option<ColumnId>,
    xs: &ChartScale,
    ys: &ChartScale,
) -> AppResult<()> {
    let caption = |id: Option<ColumnId>, fallback: &'static str| match id {
        Some(id) => run
            .analysis
            .columns()
            .iter()
            .find(|c| c.id == id)
            .map_or_else(
                || std::borrow::Cow::Owned(format!("Column {}", id.0)),
                |c| std::borrow::Cow::Borrowed(c.name.as_str()),
            ),
        None => std::borrow::Cow::Borrowed(fallback),
    };
    super::report_charts::axes(
        w,
        &caption(x, "Group index (starting at 1)"),
        &caption(y, "Frequency"),
        xs,
        ys,
    )
}

fn svg_end(w: &mut dyn Write, shown: usize, total: usize, reduced: bool) -> AppResult<()> {
    write!(
        w,
        "</svg><figcaption>Showing {shown} of {total} eligible observation rows. "
    )?;
    if reduced {
        w.write_all(b"Drawing values were reduced; counts and bounds use the complete cohort. ")?;
    }
    w.write_all(b"Exact values follow in the table.</figcaption></figure>")?;
    Ok(())
}
#[cfg(test)]
pub(super) fn histogram(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedHistogram,
    c: &JobControl,
) -> AppResult<()> {
    histogram_output(w, run, v, c, true)
}
pub(super) fn histogram_tables(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedHistogram,
    c: &JobControl,
) -> AppResult<()> {
    histogram_output(w, run, v, c, false)
}
fn histogram_output(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedHistogram,
    c: &JobControl,
    draw: bool,
) -> AppResult<()> {
    method(w, HISTOGRAM_METHOD)?;
    write!(w,"<p>Requested bins: {}. Actual bins: {}. Valid rows: {}; frequency: {}. Missing rows: {}; missing frequency: {}.</p>",bin_setting(v.request.bins),v.bins.len(),v.valid_rows,v.n,v.missing_rows,v.missing_n)?;
    if let Some(reason) = &v.bin_adjustment {
        method(w, reason)?;
    }
    if draw {
        if let Some((lo, hi)) = bounds(&v.range) {
            let xs = ChartScale::new(lo, hi)?;
            let ys = ChartScale::new(
                0.,
                v.bins.iter().map(|b| b.frequency).max().unwrap_or(0) as f64,
            )?;
            svg_start(
                w,
                run,
                "Histogram",
                "Stored full-cohort bins; precise boundaries and counts follow.",
                &xs,
                &ys,
            )?;
            axes(w, run, Some(v.request.variable), None, &xs, &ys)?;
            for b in &v.bins {
                c.check_cancelled()?;
                let left =
                    svg_left(&ys) + (svg_right(&xs) - svg_left(&ys)) * xs.position(b.left)?;
                let right =
                    svg_left(&ys) + (svg_right(&xs) - svg_left(&ys)) * xs.position(b.right)?;
                let height = 260. * ys.position(b.frequency as f64)?;
                if lo == hi {
                    write!(
                        w,
                        "<path d=\"M{left} 280 v-{}\" stroke=\"#86631F\" stroke-width=\"8\"/>",
                        height
                    )?;
                } else {
                    write!(w,"<rect x=\"{left}\" y=\"{}\" width=\"{}\" height=\"{height}\" fill=\"#86631F\" stroke=\"#F2EBDC\" class=\"chart-bar\"/>",280.-height,right-left)?;
                }
            }
            svg_end(w, v.valid_rows, v.valid_rows, false)?;
        } else {
            estimate(w, &v.range[0])?;
        }
    }
    start_table(
        w,
        "Complete histogram bins",
        &[
            "Left boundary",
            "Right boundary",
            "Inclusion",
            "Rows",
            "Frequency",
        ],
    )?;
    for (i, b) in v.bins.iter().enumerate() {
        c.check_cancelled()?;
        w.write_all(b"<tr><td>")?;
        number(w, b.left)?;
        w.write_all(b"</td><td>")?;
        number(w, b.right)?;
        write!(
            w,
            "</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            if b.left == b.right {
                "Point mass"
            } else if i + 1 == v.bins.len() {
                "[left, right]"
            } else {
                "[left, right)"
            },
            b.rows,
            b.frequency
        )?;
    }
    end_table(w)
}
#[cfg(test)]
pub(super) fn scatter(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedScatter,
    c: &JobControl,
) -> AppResult<()> {
    scatter_output(w, run, v, c, true)
}
pub(super) fn scatter_tables(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedScatter,
    c: &JobControl,
) -> AppResult<()> {
    scatter_output(w, run, v, c, false)
}
fn scatter_output(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedScatter,
    c: &JobControl,
    draw: bool,
) -> AppResult<()> {
    method(w, SCATTER_METHOD)?;
    write!(
        w,
        "<p>Valid rows: {}; frequency: {}. Missing-pair rows: {}; missing-pair frequency: {}.</p>",
        v.valid_rows, v.n, v.missing_rows, v.missing_n
    )?;
    if draw {
        if let (Some((xl, xh)), Some((yl, yh))) = (bounds(&v.ranges[0]), bounds(&v.ranges[1])) {
            let xs = ChartScale::new(xl, xh)?;
            let ys = ChartScale::new(yl, yh)?;
            svg_start(
                w,
                run,
                "Scatterplot",
                "One marker per retained source row; axes use complete-pair full-cohort extrema.",
                &xs,
                &ys,
            )?;
            axes(w, run, Some(v.request.x), Some(v.request.y), &xs, &ys)?;
            for p in &v.points {
                c.check_cancelled()?;
                write!(w,"<circle cx=\"{}\" cy=\"{}\" r=\"3\" fill=\"#C59B43\" class=\"chart-point\"><title>Source row {}; frequency {}</title></circle>",svg_left(&ys)+(svg_right(&xs)-svg_left(&ys))*xs.position(p.x)?,280.-260.*ys.position(p.y)?,p.row+1,p.frequency)?;
            }
            svg_end(w, v.points.len(), v.valid_rows, v.reduced)?;
        } else {
            estimate(w, &v.ranges[0][0])?;
        }
    }
    if !draw {
        write!(
            w,
            "<p>Showing {} of {} eligible observation rows. Exact values follow in the table.</p>",
            v.points.len(),
            v.valid_rows
        )?;
        if v.reduced {
            method(
                w,
                "Drawing points were reduced; summaries and bounds use the complete cohort.",
            )?;
        }
    }
    start_table(
        w,
        "Exact retained scatterplot values",
        &["Source row (starting at 1)", "X", "Y", "Frequency"],
    )?;
    for p in &v.points {
        c.check_cancelled()?;
        write!(w, "<tr><th scope=\"row\">{}</th><td>", p.row + 1)?;
        number(w, p.x)?;
        w.write_all(b"</td><td>")?;
        number(w, p.y)?;
        write!(w, "</td><td>{}</td></tr>", p.frequency)?;
    }
    end_table(w)
}
#[cfg(test)]
pub(super) fn boxplot(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedBoxplot,
    c: &JobControl,
) -> AppResult<()> {
    boxplot_output(w, run, v, c, true)
}
pub(super) fn boxplot_tables(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedBoxplot,
    c: &JobControl,
) -> AppResult<()> {
    boxplot_output(w, run, v, c, false)
}
fn boxplot_output(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedBoxplot,
    c: &JobControl,
    draw: bool,
) -> AppResult<()> {
    method(w, BOXPLOT_METHOD)?;
    write!(w,"<p>Missing group rows: {}; missing group frequency: {}. Missing outcome rows with present groups: {}; missing outcome frequency: {}.</p>",v.missing_group_rows,v.missing_group_n,v.missing_outcome_rows,v.missing_outcome_n)?;
    if let Some(reason) = &v.empty_reason {
        method(w, reason)?;
    }
    if draw {
        box_svg(w, run, v, c)?;
    }
    if !draw && v.reduced {
        method(
            w,
            "Outlier drawing was reduced; summaries use the complete cohort.",
        )?;
    }
    start_table(
        w,
        "Complete boxplot group summaries",
        &[
            "Group",
            "Rows",
            "Frequency",
            "Minimum",
            "Q1",
            "Median",
            "Q3",
            "Maximum",
            "Lower observed whisker",
            "Upper observed whisker",
            "Outlier rows",
            "Outlier frequency",
        ],
    )?;
    for (i, g) in v.groups.iter().enumerate() {
        c.check_cancelled()?;
        write!(w, "<tr><th scope=\"row\">{}: ", i + 1)?;
        if let (Some(id), Some(value)) = (v.request.group, &g.value) {
            category(w, run, id, value, g.value_label_index)?;
        } else {
            w.write_all(b"All eligible outcomes")?;
        }
        write!(w, "</th><td>{}</td><td>{}</td>", g.rows, g.n)?;
        estimates(
            w,
            &[
                &g.minimum,
                &g.q1,
                &g.median,
                &g.q3,
                &g.maximum,
                &g.whiskers[0],
                &g.whiskers[1],
            ],
        )?;
        write!(
            w,
            "<td>{}</td><td>{}</td></tr>",
            g.outlier_rows, g.outlier_n
        )?;
    }
    end_table(w)?;
    start_table(
        w,
        "Exact retained outliers",
        &[
            "Group index (starting at 1)",
            "Source row (starting at 1)",
            "Value",
            "Frequency",
        ],
    )?;
    for p in &v.points {
        c.check_cancelled()?;
        write!(
            w,
            "<tr><th scope=\"row\">{}</th><td>{}</td><td>",
            p.group_index + 1,
            p.row + 1
        )?;
        number(w, p.value)?;
        write!(w, "</td><td>{}</td></tr>", p.frequency)?;
    }
    end_table(w)
}
fn box_svg(
    w: &mut dyn Write,
    run: &AnalysisRun,
    v: &ComputedBoxplot,
    c: &JobControl,
) -> AppResult<()> {
    let lo = v
        .groups
        .iter()
        .filter_map(|g| g.minimum.value)
        .reduce(f64::min);
    let hi = v
        .groups
        .iter()
        .filter_map(|g| g.maximum.value)
        .reduce(f64::max);
    let (Some(lo), Some(hi)) = (lo, hi) else {
        return Ok(());
    };
    let xs = ChartScale::new(1., v.groups.len() as f64)?;
    let ys = ChartScale::new(lo, hi)?;
    svg_start(w,run,"Boxplot","Full-cohort quartiles and actual observed whiskers. Quartiles and whiskers have separate order chains; unavailable whiskers remain explained in the table.",&xs,&ys)?;
    axes(w, run, None, Some(v.request.outcome), &xs, &ys)?;
    for (i, g) in v.groups.iter().enumerate() {
        c.check_cancelled()?;
        let x = svg_left(&ys) + (svg_right(&xs) - svg_left(&ys)) * xs.position((i + 1) as f64)?;
        if let (Some(q1), Some(median), Some(q3)) = (g.q1.value, g.median.value, g.q3.value) {
            let bottom = 280. - 260. * ys.position(q1)?;
            let top = 280. - 260. * ys.position(q3)?;
            let mid = 280. - 260. * ys.position(median)?;
            write!(w,"<rect x=\"{}\" y=\"{top}\" width=\"16\" height=\"{}\" fill=\"#F2EBDC\" stroke=\"#263449\" class=\"chart-box\"/><path d=\"M{} {mid} h16\" stroke=\"#263449\" stroke-width=\"5\" class=\"chart-median-edge\"/><path d=\"M{} {mid} h16\" stroke=\"#C59B43\" stroke-width=\"3\" class=\"chart-median\"/>",x-8.,bottom-top,x-8.,x-8.)?;
            for whisker in &g.whiskers {
                if let Some(value) = whisker.value {
                    let y = 280. - 260. * ys.position(value)?;
                    write!(
                        w,
                        "<path d=\"M{x} {mid} V{y} M{} {y} h12\" stroke=\"#263449\"/>",
                        x - 6.
                    )?;
                }
            }
        }
    }
    for p in &v.points {
        c.check_cancelled()?;
        write!(
            w,
            "<circle cx=\"{}\" cy=\"{}\" r=\"3\" fill=\"none\" stroke=\"#263449\"/>",
            svg_left(&ys)
                + (svg_right(&xs) - svg_left(&ys)) * xs.position((p.group_index + 1) as f64)?,
            280. - 260. * ys.position(p.value)?
        )?;
    }
    let total: usize = v.groups.iter().map(|g| g.outlier_rows).sum();
    w.write_all(b"</svg><figcaption>Outlier drawing: ")?;
    write!(w, "{} of {} outlier source rows. ", v.points.len(), total)?;
    if v.reduced {
        w.write_all(b"Outlier drawing reduced. ")?;
    }
    w.write_all(b"Full group summaries and precise outliers follow.</figcaption></figure>")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_category_report_copy_can_cancel_and_does_not_publish() {
        use crate::{
            data::{Column, Dataset},
            operation::CHECKPOINT_HOOK,
            project::Project,
        };
        let data = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Text",
            vec![Some("ŵ<&>\t".repeat(100_000))],
        )
        .unwrap()])
        .unwrap();
        let request = AnalysisRequest::Frequencies(FrequenciesRequest {
            columns: vec![ColumnId(1)],
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        let result =
            analyze_with_budget(&data, &request, &JobControl::default(), &[], 4096).unwrap();
        let p = Project::new("Long text", data)
            .unwrap()
            .with_run(result)
            .unwrap();
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "tessera-v3-cancel-report-{}.html",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let control = JobControl::default();
        let cancel = control.clone();
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |progress| {
                if progress.done >= 65536 {
                    cancel.cancel();
                }
            }))
        });
        let result =
            crate::export::export_report(&path, &p, &[crate::operation::RunId(1)], &control);
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert!(matches!(result, Err(crate::error::AppError::Cancelled)));
        assert!(!path.exists());
    }
    #[test]
    fn precise_svg_ticks_fit_reserved_margin_and_raw_tables_and_description_remain_exact() {
        use crate::{
            data::{Column, Dataset},
            operation::RunId,
        };
        let lower = f64::from_bits(f64::MAX.to_bits() - 1);
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), "X", vec![Some(lower), Some(f64::MAX)]).unwrap(),
            Column::numeric(ColumnId(2), "Y", vec![Some(lower), Some(f64::MAX)]).unwrap(),
        ])
        .unwrap();
        let result = analyze_with_budget(
            &data,
            &AnalysisRequest::Scatter(ScatterRequest {
                x: ColumnId(1),
                y: ColumnId(2),
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        let run = AnalysisRun {
            id: RunId(1),
            analysis: result,
        };
        let CompletedAnalysis::Scatter(v) = &run.analysis else {
            panic!()
        };
        let mut bytes = Vec::new();
        scatter(&mut bytes, &run, v, &JobControl::default()).unwrap();
        let html = String::from_utf8(bytes).unwrap();
        assert!(html.contains("M184 20 V280 H636"));
        assert!(html.contains("<circle cx=\"184\" cy=\"280\""));
        assert!(html.contains("<circle cx=\"636\" cy=\"20\""));
        let ticks = html
            .split("<text")
            .filter(|part| part.split('>').next().unwrap().contains("font-size=\"11\""))
            .map(|part| {
                part.split('>')
                    .nth(1)
                    .unwrap()
                    .split("</text")
                    .next()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(ticks.len(), 4);
        assert!(ticks.iter().all(|label| label.len() <= 26));
        assert_eq!(
            ticks
                .iter()
                .filter(|s| **s == "1.79769313486231571e308")
                .count(),
            2
        );
        let exact = format!("{:.0}", f64::MAX);
        assert!(html.contains(&format!("X maximum: {exact}")));
        assert!(html.contains(&format!("<td>{exact}</td>")));
    }
}
