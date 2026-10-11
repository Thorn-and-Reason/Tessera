use crate::{
    analysis::{MissingPolicy, NumericComparison, PlotData, RowPredicate, Statistic},
    data::{Cell, MissingRules},
    error::AppResult,
    operation::{JobControl, RunId},
    project::{AnalysisRun, Project},
};
use std::io::Write;
pub(super) fn text(writer: &mut dyn Write, value: &str) -> AppResult<()> {
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        let escaped = match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => {
                if index - start >= 4096 {
                    writer.write_all(&value.as_bytes()[start..index])?;
                    start = index;
                }
                continue;
            }
        };
        writer.write_all(&value.as_bytes()[start..index])?;
        writer.write_all(escaped.as_bytes())?;
        start = index + ch.len_utf8();
    }
    writer.write_all(&value.as_bytes()[start..])?;
    Ok(())
}
pub(super) fn write(
    writer: &mut dyn Write,
    project: &Project,
    selected: &[RunId],
    control: &JobControl,
) -> AppResult<()> {
    writer.write_all(b"<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>")?;
    text(writer, project.title())?;
    writer.write_all(b"</title><style>")?;
    writer.write_all(include_bytes!("report.css"))?;
    writer.write_all(b"</style></head><body><main><header aria-label=\"Tessera by Thorn &amp; Reason\"><p>Tessera by Thorn &amp; Reason</p></header><h1>")?;
    text(writer, project.title())?;
    write!(
        writer,
        "</h1><p>Data revision {}. {} observation rows; {} variables.</p>",
        project.dataset().revision().0,
        project.dataset().row_count(),
        project.dataset().column_count()
    )?;
    writer.write_all(b"<p>Results are retained analyses. Numeric exports preserve Float64 values; display rounding does not alter calculations. Frequency weights repeat observations in the estimates. These results do not apply survey-design adjustments or establish causal effects.</p>")?;
    if selected.is_empty() {
        writer.write_all(b"<p>No analysis runs were selected.</p>")?;
    }
    for (index, id) in selected.iter().enumerate() {
        control.checkpoint(index, Some(selected.len()))?;
        // Caller validates all IDs and duplicates before creating a transaction.
        if let Some(run) = project.runs().iter().find(|run| run.id == *id) {
            run_report(writer, run, project, control)?;
            figures(writer, run, project, control)?;
        }
    }
    if !project.provenance().is_empty() {
        writer.write_all(b"<h2>Provenance</h2><ul>")?;
        for entry in project.provenance() {
            writer.write_all(b"<li>")?;
            text(writer, entry)?;
            writer.write_all(b"</li>")?;
        }
        writer.write_all(b"</ul>")?;
    }
    writer.write_all(b"</main></body></html>\n")?;
    Ok(())
}
fn figures(
    writer: &mut dyn Write,
    run: &AnalysisRun,
    project: &Project,
    control: &JobControl,
) -> AppResult<()> {
    let keys = crate::figures::figure_keys(run);
    for key in keys.iter().chain(
        project
            .figures()
            .iter()
            .filter(|record| record.key.run == run.id.0 && record.key.variant != 0)
            .map(|record| &record.key),
    ) {
        control.check_cancelled()?;
        let settings = project.figure_settings(key);
        let svg = match crate::figures::render::svg(
            run,
            key,
            &settings,
            run.analysis.revision() != project.dataset().revision(),
            control,
        ) {
            Ok(svg) => svg,
            Err(crate::error::AppError::InvalidInput { location, message })
                if location == "figure" =>
            {
                writer.write_all(b"<p>Drawing unavailable: ")?;
                text(writer, &message)?;
                writer.write_all(b" Exact captured values remain in the tables.</p>")?;
                continue;
            }
            Err(error) => return Err(error),
        };
        writer.write_all(b"<figure class=\"report-figure\">")?;
        writer.write_all(svg.as_bytes())?;
        writer.write_all(b"</figure>")?;
    }
    Ok(())
}

fn stat(statistic: Statistic) -> &'static str {
    match statistic {
        Statistic::Mean => "Mean",
        Statistic::SampleStdDev => "Sample standard deviation",
        Statistic::Variance => "Sample variance",
        Statistic::Minimum => "Minimum",
        Statistic::Maximum => "Maximum",
        Statistic::Sum => "Sum",
        Statistic::Range => "Range",
        Statistic::StdErrorMean => "Standard error of the mean",
    }
}
pub(super) fn column_name(
    writer: &mut dyn Write,
    run: &AnalysisRun,
    id: crate::data::ColumnId,
) -> AppResult<()> {
    if let Some(info) = run.analysis.columns().iter().find(|info| info.id == id) {
        text(writer, &info.name)?;
    } else {
        write!(writer, "Column {}", id.0)?;
    }
    Ok(())
}
fn run_report(
    writer: &mut dyn Write,
    run: &AnalysisRun,
    project: &Project,
    control: &JobControl,
) -> AppResult<()> {
    let crate::analysis::CompletedAnalysis::Describe(a) = &run.analysis else {
        return super::report_analyses::write(writer, run, project, control);
    };
    write!(writer,"<section aria-labelledby=\"run-{}\"><h2 id=\"run-{}\">Analysis run {}</h2><p>Calculated from data revision {}. ",run.id.0,run.id.0,run.id.0,a.revision.0)?;
    if a.revision != project.dataset().revision() {
        writer.write_all(b"Historical results: the current data revision differs. ")?;
    }
    writer.write_all(b"Names and definitions below are those captured when this analysis ran.</p><h3>Settings</h3><p>Missing-value policy: ")?;
    writer.write_all(match a.request.missing_policy {
        MissingPolicy::PerVariable => b"per variable",
        MissingPolicy::CompleteCase => b"complete case across selected variables",
    })?;
    writer.write_all(b". Frequency weight: ")?;
    if let Some(id) = a.request.frequency_weight {
        column_name(writer, run, id)?;
    } else {
        writer.write_all(b"none (each observation has weight one)")?;
    }
    writer.write_all(b".</p><p>Filter: ")?;
    if a.request.filter.predicates.is_empty() {
        writer.write_all(b"all rows")?;
    }
    for (index, predicate) in a.request.filter.predicates.iter().enumerate() {
        if index != 0 {
            writer.write_all(b" AND ")?;
        }
        column_name(writer, run, predicate.column())?;
        match predicate {
            RowPredicate::Number {
                comparison, value, ..
            } => {
                text(
                    writer,
                    match comparison {
                        NumericComparison::Equal => " = ",
                        NumericComparison::NotEqual => " != ",
                        NumericComparison::Less => " < ",
                        NumericComparison::LessOrEqual => " <= ",
                        NumericComparison::Greater => " > ",
                        NumericComparison::GreaterOrEqual => " >= ",
                    },
                )?;
                super::number(writer, *value)?;
            }
            RowPredicate::Text { value, equal, .. } => {
                writer.write_all(if *equal {
                    b" equals "
                } else {
                    b" does not equal "
                })?;
                text(writer, value)?;
            }
            RowPredicate::Missing { missing, .. } => writer.write_all(if *missing {
                b" is missing"
            } else {
                b" is present"
            })?,
        }
    }
    let d = &a.diagnostics;
    write!(writer,".</p><p>Excluded observation rows: {} by filter, {} with missing weights, {} with zero weights, and {} by complete-case selection.</p>",d.filtered_rows,d.missing_weight_rows,d.zero_weight_rows,d.complete_case_rows)?;
    writer.write_all(b"<table><caption>Descriptive statistics; N values are summed frequencies</caption><thead><tr><th scope=\"col\">Variable</th><th scope=\"col\">Valid N</th><th scope=\"col\">Missing N</th><th scope=\"col\">Other complete-case exclusions</th>")?;
    for statistic in &a.request.statistics {
        writer.write_all(b"<th scope=\"col\">")?;
        text(writer, stat(*statistic))?;
        writer.write_all(b"</th>")?;
    }
    writer.write_all(b"</tr></thead><tbody>")?;
    for row in &a.rows {
        writer.write_all(b"<tr><th scope=\"row\">")?;
        column_name(writer, run, row.column)?;
        write!(
            writer,
            "</th><td>{}</td><td>{}</td><td>{}</td>",
            row.valid_n, row.missing_n, row.complete_case_excluded_n
        )?;
        for statistic in &a.request.statistics {
            writer.write_all(b"<td>")?;
            if let Some(estimate) = row.estimates.get(statistic) {
                if let Some(value) = estimate.value {
                    super::number(writer, value)?;
                } else {
                    writer.write_all(b"Unavailable")?;
                    if let Some(reason) = &estimate.reason {
                        writer.write_all(b": ")?;
                        text(writer, reason)?;
                    }
                }
            } else {
                writer.write_all(b"Not selected")?;
            }
            writer.write_all(b"</td>")?;
        }
        writer.write_all(b"</tr>")?;
    }
    writer.write_all(b"</tbody></table><h3>Variable definitions at analysis time</h3><dl>")?;
    for info in &a.columns {
        writer.write_all(b"<dt>")?;
        text(writer, &info.name)?;
        write!(writer, " (column {}, {:?})</dt><dd>", info.id.0, info.kind)?;
        text(writer, &info.description)?;
        writer.write_all(b"<p>Missing rules: ")?;
        missing(writer, &info.missing_rules)?;
        writer.write_all(b".</p>")?;
        if !info.value_labels.is_empty() {
            writer.write_all(b"<ul>")?;
            for label in &info.value_labels {
                writer.write_all(b"<li>")?;
                match &label.value {
                    Cell::Number(value) => super::number(writer, *value)?,
                    Cell::Text(value) => text(writer, value)?,
                    Cell::Missing => writer.write_all(b"Missing")?,
                };
                writer.write_all(b": ")?;
                text(writer, &label.label)?;
                writer.write_all(b"</li>")?;
            }
            writer.write_all(b"</ul>")?;
        }
        writer.write_all(b"</dd>")?;
    }
    writer.write_all(b"</dl>")?;
    for plot in &a.plots {
        control.checkpoint(0, None)?;
        plot_report(writer, plot)?;
    }
    writer.write_all(b"</section>")?;
    Ok(())
}
pub(super) fn missing(writer: &mut dyn Write, rules: &MissingRules) -> AppResult<()> {
    match rules {
        MissingRules::None => writer.write_all(b"empty cells only")?,
        MissingRules::Text(values) => {
            writer.write_all(b"empty cells and exact text tokens ")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                writer.write_all(b"&quot;")?;
                text(writer, value)?;
                writer.write_all(b"&quot;")?;
            }
        }
        MissingRules::Number { values, ranges } => {
            writer.write_all(b"empty cells")?;
            for value in values {
                writer.write_all(b", ")?;
                super::number(writer, *value)?;
            }
            for (lo, hi) in ranges {
                writer.write_all(b", inclusive range ")?;
                super::number(writer, *lo)?;
                writer.write_all(b" to ")?;
                super::number(writer, *hi)?;
            }
        }
    }
    Ok(())
}
fn plot_report(writer: &mut dyn Write, plot: &PlotData) -> AppResult<()> {
    writer.write_all(b"<p>")?;
    text(writer, &plot.label)?;
    write!(writer," Horizontal position: source observation row. Vertical position: value. Each point represents one retained observation row; frequency is recorded in the table. Showing {} of {} eligible observation rows. ",plot.points.len(),plot.source_rows)?;
    if plot.reduced {
        writer.write_all(
            b"Plot points were reduced for display; statistics use all eligible observations.",
        )?;
    }
    writer.write_all(b"</p><details><summary>Plot data and frequency weights</summary><table><caption>Exact plotted values</caption><thead><tr><th scope=\"col\">Source row (starting at 1)</th><th scope=\"col\">Value</th><th scope=\"col\">Frequency</th></tr></thead><tbody>")?;
    for point in &plot.points {
        write!(writer, "<tr><th scope=\"row\">{}</th><td>", point.row + 1)?;
        super::number(writer, point.value)?;
        write!(writer, "</td><td>{}</td></tr>", point.frequency)?;
    }
    writer.write_all(b"</tbody></table></details>")?;
    Ok(())
}
