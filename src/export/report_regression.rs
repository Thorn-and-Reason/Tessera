//! Frozen regression output. No dataset, draft or historical-family dispatch is consulted.
use super::{
    number,
    report::{missing, text},
    report_analyses::{cell, confidence, estimate},
    report_charts,
};
use crate::{
    analysis::*,
    charts::ChartScale,
    data::{Cell, ColumnId},
    error::AppResult,
    operation::{JobControl, RunId},
};
use std::io::Write;
mod cancellation;
#[cfg(test)]
pub(crate) mod controls;
#[cfg(test)]
mod tests;

pub(crate) const METHOD: &str = "Additive ordinary least squares with a required intercept and complete cases. Categorical predictors use K−1 treatment contrasts against the captured reference. Frequency weights represent literal replication. Coefficient tests are two-sided and unadjusted. The omnibus F test tests all non-intercept coefficients against zero. Inference assumes independent observations, a correctly specified linear mean, common error variance and normally distributed errors; these results do not establish causal effects.";
pub(crate) const DIAGNOSTICS: &str = "Residual summaries use the complete frequency-weighted cohort. Residual sample SD uses N−1; model residual standard error uses N−p. Leverage refers to one replicated observation. Internally studentised residual: e/(s sqrt(1−h)). Cook’s distance: e²h/[p s²(1−h)²], for deletion of one replicated observation. The raw-residual Q–Q plot uses deterministic integer ranks, normal quantiles at (rank+0.5)/N for zero-based ranks, and symmetric lower-tail evaluation. Drawings do not change the fitted cohort or establish normality.";

#[cfg(test)]
pub(super) fn write(
    w: &mut dyn Write,
    run_id: RunId,
    result: &ComputedRegression,
    control: &JobControl,
) -> AppResult<()> {
    cancellation::write(w, control, |w, control| {
        output(w, run_id, result, control, true)
    })
}
pub(super) fn tables(
    w: &mut dyn Write,
    run_id: RunId,
    result: &ComputedRegression,
    control: &JobControl,
) -> AppResult<()> {
    cancellation::write(w, control, |w, control| {
        output(w, run_id, result, control, false)
    })
}

fn output(
    w: &mut dyn Write,
    run_id: RunId,
    r: &ComputedRegression,
    c: &JobControl,
    draw: bool,
) -> AppResult<()> {
    write!(w, "<section aria-labelledby=\"run-{}\"><h2 id=\"run-{}\">Analysis run {} · Linear regression</h2><p>Calculated from data revision {}. Names and definitions were captured when this analysis ran.</p>", run_id.0, run_id.0, run_id.0, r.revision.0)?;
    paragraph(w, METHOD)?;
    settings(w, r, c)?;
    write!(w, "<h3>Cohort</h3><p>Source rows: {}. Fitted source rows M: {}; literal replicated frequency N: {}. Complete-case excluded frequency: {}.</p><p>Excluded source rows: {} by filter; {} with missing weights; {} with zero weights; {} incomplete cases.</p>", r.source_rows, r.fitted_rows, r.frequency, r.incomplete_case_frequency, r.diagnostics.filtered_rows, r.diagnostics.missing_weight_rows, r.diagnostics.zero_weight_rows, r.diagnostics.complete_case_rows)?;
    coding(w, r, c)?;
    match &r.outcome {
        RegressionOutcome::Unavailable(f) => failed(w, r, f, c)?,
        RegressionOutcome::Fitted(f) => {
            fitted(w, r, f, c)?;
            diagnostic_summary(w, &f.residual_diagnostics)?;
            if draw {
                plots(w, run_id, r, &f.residual_diagnostics, c)?;
            } else {
                let d = &f.residual_diagnostics;
                plot_heading(w, run_id, "residual", "Residual versus fitted values", r)?;
                plot_notice(
                    w,
                    d.residual_points.len(),
                    r.fitted_rows as u64,
                    d.residual_reduced,
                    "fitted source rows",
                )?;
                residual_table(w, r, d, c)?;
                plot_heading(w, run_id, "qq", "Normal Q–Q plot of raw residuals", r)?;
                plot_notice(
                    w,
                    d.qq_points.len(),
                    r.frequency,
                    d.qq_reduced,
                    "replicated observation ranks",
                )?;
                qq_table(w, d, c)?;
            }
        }
    }
    definitions(w, r, c)?;
    w.write_all(b"</section>")?;
    c.check_cancelled()
}
fn paragraph(w: &mut dyn Write, s: &str) -> AppResult<()> {
    w.write_all(b"<p>")?;
    text(w, s)?;
    w.write_all(b"</p>")?;
    Ok(())
}
fn column(w: &mut dyn Write, r: &ComputedRegression, id: ColumnId) -> AppResult<()> {
    if let Some(info) = r.columns.iter().find(|v| v.id == id) {
        text(w, &info.name)?;
    } else {
        write!(w, "Column {}", id.0)?;
    }
    Ok(())
}
fn category(
    w: &mut dyn Write,
    r: &ComputedRegression,
    id: ColumnId,
    value: &Cell,
    c: &JobControl,
) -> AppResult<()> {
    cell(w, value)?;
    if let Some(info) = r.columns.iter().find(|v| v.id == id) {
        for label in &info.value_labels {
            c.check_cancelled()?;
            if crate::analysis::exploration::ordering::compare_cells(&label.value, value, c)?
                == std::cmp::Ordering::Equal
            {
                text(w, " · ")?;
                text(w, &label.label)?;
                break;
            }
        }
    }
    Ok(())
}
fn term(
    w: &mut dyn Write,
    r: &ComputedRegression,
    t: &RegressionTerm,
    c: &JobControl,
) -> AppResult<()> {
    match t {
        RegressionTerm::Intercept => w.write_all(b"Intercept")?,
        RegressionTerm::Numeric { column: id } => {
            column(w, r, *id)?;
            w.write_all(b" (numeric slope)")?;
        }
        RegressionTerm::Contrast {
            column: id,
            level,
            reference,
        } => {
            column(w, r, *id)?;
            w.write_all(b": ")?;
            category(w, r, *id, level, c)?;
            w.write_all(b" versus reference ")?;
            category(w, r, *id, reference, c)?;
        }
    }
    Ok(())
}
fn table(w: &mut dyn Write, title: &str, headings: &[&str]) -> AppResult<()> {
    w.write_all(b"<div role=\"region\" aria-label=\"")?;
    text(w, title)?;
    w.write_all(b"\" tabindex=\"0\" style=\"max-width:100%;overflow-x:auto\"><table><caption>")?;
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
    w.write_all(b"</tbody></table></div>")?;
    Ok(())
}
fn estimates(w: &mut dyn Write, values: &[&Estimate]) -> AppResult<()> {
    for value in values {
        w.write_all(b"<td>")?;
        estimate(w, value)?;
        w.write_all(b"</td>")?;
    }
    Ok(())
}
fn row(w: &mut dyn Write, label: &str, values: &[&Estimate]) -> AppResult<()> {
    w.write_all(b"<tr><th scope=\"row\">")?;
    text(w, label)?;
    w.write_all(b"</th>")?;
    estimates(w, values)?;
    w.write_all(b"</tr>")?;
    Ok(())
}
fn settings(w: &mut dyn Write, r: &ComputedRegression, c: &JobControl) -> AppResult<()> {
    w.write_all(b"<h3>Settings</h3><p>Outcome: ")?;
    column(w, r, r.request.outcome)?;
    write!(
        w,
        ". Confidence interval: {}. Required intercept. Frequency weight: ",
        confidence(r.request.confidence)
    )?;
    if let Some(id) = r.request.frequency_weight {
        column(w, r, id)?;
        w.write_all(b"; literal repeated observations")?;
    } else {
        w.write_all(b"none (each observation has weight one)")?;
    }
    w.write_all(b".</p><p>Filter: ")?;
    if r.request.filter.predicates.is_empty() {
        w.write_all(b"all rows")?;
    }
    for (index, pred) in r.request.filter.predicates.iter().enumerate() {
        c.check_cancelled()?;
        if index != 0 {
            w.write_all(b" AND ")?;
        }
        column(w, r, pred.column())?;
        match pred {
            RowPredicate::Number {
                comparison, value, ..
            } => {
                text(
                    w,
                    match comparison {
                        NumericComparison::Equal => " = ",
                        NumericComparison::NotEqual => " != ",
                        NumericComparison::Less => " < ",
                        NumericComparison::LessOrEqual => " <= ",
                        NumericComparison::Greater => " > ",
                        NumericComparison::GreaterOrEqual => " >= ",
                    },
                )?;
                number(w, *value)?;
            }
            RowPredicate::Text { value, equal, .. } => {
                text(
                    w,
                    if *equal {
                        " equals "
                    } else {
                        " does not equal "
                    },
                )?;
                w.write_all(b"<span style=\"white-space:pre-wrap\">&quot;")?;
                text(w, value)?;
                w.write_all(b"&quot;</span>")?;
            }
            RowPredicate::Missing { missing, .. } => text(
                w,
                if *missing {
                    " is missing"
                } else {
                    " is present"
                },
            )?,
        }
    }
    write!(w, ".</p><p>Numerical version: {}. Centred/scaled design and response; rank assessed from the scaled design. Rank threshold: ", r.method.numerical_version)?;
    number(w, r.method.rank_threshold)?;
    w.write_all(b". Condition warning threshold: ")?;
    number(w, r.method.condition_warning)?;
    w.write_all(b" (warning strictly above the threshold).</p>")?;
    Ok(())
}
fn coding(w: &mut dyn Write, r: &ComputedRegression, c: &JobControl) -> AppResult<()> {
    table(
        w,
        "Treatment coding and ordered predictor roles",
        &[
            "Predictor",
            "Role",
            "Final complete-cohort levels (in order)",
        ],
    )?;
    for predictor in &r.request.predictors {
        c.check_cancelled()?;
        w.write_all(b"<tr><th scope=\"row\">")?;
        column(w, r, predictor.column)?;
        w.write_all(b"</th><td>")?;
        match &predictor.role {
            PredictorRole::Numeric => w.write_all(b"Numeric")?,
            PredictorRole::Categorical { reference } => {
                w.write_all(b"Categorical; reference: ")?;
                category(w, r, predictor.column, reference, c)?;
            }
        }
        w.write_all(b"</td><td>")?;
        if let Some(levels) = r.levels.iter().find(|v| v.column == predictor.column) {
            if levels.values.is_empty() {
                w.write_all(b"No levels retained")?;
            }
            for (index, value) in levels.values.iter().enumerate() {
                c.check_cancelled()?;
                if index != 0 {
                    w.write_all(b"; ")?;
                }
                category(w, r, predictor.column, value, c)?;
            }
        } else {
            text(
                w,
                if matches!(predictor.role, PredictorRole::Numeric) {
                    "Not applicable"
                } else {
                    "Not discovered"
                },
            )?;
        }
        w.write_all(b"</td></tr>")?;
    }
    end_table(w)
}
fn failed(
    w: &mut dyn Write,
    r: &ComputedRegression,
    f: &RegressionFailure,
    c: &JobControl,
) -> AppResult<()> {
    w.write_all(b"<h3>Fit unavailable</h3><p>")?;
    text(w, &f.reason)?;
    w.write_all(b"</p><p>Failure kind: ")?;
    match &f.kind {
        RegressionFailureKind::EmptyCohort => text(w, "empty cohort")?,
        RegressionFailureKind::AbsentReference { column: id } => {
            text(w, "absent reference for ")?;
            column(w, r, *id)?;
        }
        RegressionFailureKind::NoContrast { column: id } => {
            text(w, "no contrast for ")?;
            column(w, r, *id)?;
        }
        RegressionFailureKind::Unidentified => text(w, "unidentified design")?,
        RegressionFailureKind::Nonconvergence => text(w, "nonconvergence")?,
        RegressionFailureKind::Numerical => text(w, "numerical failure")?,
    }
    w.write_all(b". Expected terms: ")?;
    if let Some(p) = f.expected_terms {
        write!(w, "{p}")?;
    } else {
        w.write_all(b"unavailable")?;
    }
    w.write_all(b". Numerical rank: ")?;
    if let Some(rank) = f.rank {
        write!(w, "{rank}")?;
    } else {
        w.write_all(b"unavailable")?;
    }
    w.write_all(b".</p>")?;
    if !f.implicated_terms.is_empty() {
        w.write_all(b"<p>Implicated terms:</p><ul>")?;
        for index in &f.implicated_terms {
            c.check_cancelled()?;
            write!(w, "<li>Term {}: ", index + 1)?;
            if let Some(t) = r.terms.get(*index) {
                term(w, r, t, c)?;
            } else {
                w.write_all(b"unavailable")?;
            }
            w.write_all(b"</li>")?;
        }
        w.write_all(b"</ul>")?;
    }
    if !r.terms.is_empty() {
        table(
            w,
            "Captured expanded terms",
            &["Term index (starting at 1)", "Term"],
        )?;
        for (index, t) in r.terms.iter().enumerate() {
            c.check_cancelled()?;
            write!(w, "<tr><th scope=\"row\">{}</th><td>", index + 1)?;
            term(w, r, t, c)?;
            w.write_all(b"</td></tr>")?;
        }
        end_table(w)?;
    }
    Ok(())
}
fn fitted(
    w: &mut dyn Write,
    r: &ComputedRegression,
    f: &RegressionFit,
    c: &JobControl,
) -> AppResult<()> {
    write!(w, "<h3>Coefficients</h3><p>Expanded terms p: {}. Numerical rank: {}. Model df: {}; Residual df: {}; total df: {}. {} confidence intervals.</p>", r.terms.len(), f.rank, f.model_df, f.residual_df, f.total_df, confidence(r.request.confidence))?;
    if f.residual_df == 0 {
        paragraph(w, "There are no residual degrees of freedom. Coefficients may be available; residual-variance inference is unavailable.")?;
    } else if f.statistics.residual_standard_error.value == Some(0.) {
        paragraph(w, "Zero residual variance: coefficient intervals are point intervals at the algebraic zero-variance limit. They do not establish error-free population effects. Tests and residual-variance diagnostics are unavailable.")?;
    }
    if f.condition_warned {
        paragraph(w, "Condition warning: the full-rank scaled design exceeds the captured warning threshold. Coefficients and inference may be sensitive to small input changes.")?;
    }
    table(
        w,
        "Original-unit coefficients and unadjusted two-sided tests",
        &[
            "Term",
            "Coefficient",
            "Standard error",
            "t",
            "Two-sided p",
            "Interval lower",
            "Interval upper",
            "Captured term scale",
        ],
    )?;
    for (index, t) in r.terms.iter().enumerate() {
        c.check_cancelled()?;
        w.write_all(b"<tr><th scope=\"row\">")?;
        term(w, r, t, c)?;
        w.write_all(b"</th>")?;
        if let Some(coefficient) = f.coefficients.get(index) {
            estimates(
                w,
                &[
                    &coefficient.estimate,
                    &coefficient.standard_error,
                    &coefficient.t,
                    &coefficient.p,
                    &coefficient.interval[0],
                    &coefficient.interval[1],
                ],
            )?;
        }
        if let Some(scale) = f.term_scales.get(index) {
            estimates(w, &[scale])?;
        }
        w.write_all(b"</tr>")?;
    }
    end_table(w)?;
    table(w, "Model fit", &["Statistic", "Captured value or reason"])?;
    let s = &f.statistics;
    for (label, value) in [
        ("SSE", &s.sse),
        ("Centred SST", &s.sst),
        ("Model sum of squares", &s.model_ss),
        ("Residual standard error (N−p)", &s.residual_standard_error),
        ("R²", &s.r_squared),
        ("Adjusted R²", &s.adjusted_r_squared),
        ("Omnibus F", &s.f),
        ("F upper-tail p", &s.f_p),
        ("Scaled-design condition", &f.condition),
        ("Outcome scale", &f.outcome_scale),
    ] {
        row(w, label, &[value])?;
    }
    end_table(w)
}
fn diagnostic_summary(w: &mut dyn Write, d: &RegressionDiagnostics) -> AppResult<()> {
    w.write_all(b"<h3>Residual diagnostics</h3>")?;
    paragraph(w, DIAGNOSTICS)?;
    table(
        w,
        "Full-cohort residual summary and plotting bounds",
        &["Statistic", "Captured value or reason"],
    )?;
    for (label, value) in [
        ("Residual mean", &d.residual_mean),
        ("Residual sample SD (N−1)", &d.residual_sample_sd),
        ("Residual minimum", &d.residual_range[0]),
        ("Residual maximum", &d.residual_range[1]),
        ("Fitted minimum", &d.fitted_range[0]),
        ("Fitted maximum", &d.fitted_range[1]),
        ("Theoretical quantile minimum", &d.theoretical_range[0]),
        ("Theoretical quantile maximum", &d.theoretical_range[1]),
    ] {
        row(w, label, &[value])?;
    }
    end_table(w)?;
    table(
        w,
        "Full-cohort diagnostic extents",
        &[
            "Diagnostic",
            "Minimum",
            "Maximum",
            "undefined source rows",
            "Undefined frequency",
        ],
    )?;
    for (label, extent) in [
        ("Per-observation leverage", &d.leverage),
        ("Internally studentised residual", &d.studentised),
        (
            "Cook’s distance (one replicated observation)",
            &d.cooks_distance,
        ),
    ] {
        w.write_all(b"<tr><th scope=\"row\">")?;
        text(w, label)?;
        w.write_all(b"</th>")?;
        estimates(w, &[&extent.range[0], &extent.range[1]])?;
        write!(
            w,
            "<td>{}</td><td>{}</td></tr>",
            extent.undefined_rows, extent.undefined_frequency
        )?;
    }
    end_table(w)
}
fn bounds(range: &[Estimate; 2]) -> Option<(f64, f64)> {
    Some((range[0].value?, range[1].value?))
}
fn unavailable_drawing(w: &mut dyn Write, x: &[Estimate; 2], y: &[Estimate; 2]) -> AppResult<()> {
    paragraph(w, "Drawing unavailable because a full-cohort axis bound is unavailable. Exact retained values remain in the table.")?;
    for value in x.iter().chain(y) {
        if value.value.is_none() {
            w.write_all(b"<p>")?;
            estimate(w, value)?;
            w.write_all(b"</p>")?;
        }
    }
    Ok(())
}
fn marker(
    w: &mut dyn Write,
    x: &Estimate,
    y: &Estimate,
    xs: &ChartScale,
    ys: &ChartScale,
) -> AppResult<()> {
    if let (Some(x), Some(y)) = (x.value, y.value) {
        write!(
            w,
            "<circle cx=\"{}\" cy=\"{}\" r=\"3\" fill=\"#263449\"/>",
            report_charts::left(ys)
                + (report_charts::right(xs) - report_charts::left(ys)) * xs.position(x)?,
            280. - 260. * ys.position(y)?
        )?;
    }
    Ok(())
}
fn plot_heading(
    w: &mut dyn Write,
    run_id: RunId,
    kind: &str,
    title: &str,
    r: &ComputedRegression,
) -> AppResult<()> {
    write!(w, "<div role=\"group\" aria-labelledby=\"plot-heading-{}-{kind}\"><h3 id=\"plot-heading-{}-{kind}\">", run_id.0, run_id.0)?;
    text(w, title)?;
    w.write_all(b" for ")?;
    column(w, r, r.request.outcome)?;
    w.write_all(b"</h3>")?;
    Ok(())
}
fn plot_notice(
    w: &mut dyn Write,
    shown: usize,
    total: u64,
    reduced: bool,
    unit: &str,
) -> AppResult<()> {
    write!(w, "<p>Showing {shown} of {total} {unit}. ")?;
    if reduced {
        w.write_all(
            b"Drawing values were reduced; summaries and bounds use the complete cohort. ",
        )?;
    }
    w.write_all(b"</p>")?;
    Ok(())
}
fn plot_end(w: &mut dyn Write, r: &ComputedRegression) -> AppResult<()> {
    write!(w, "</svg><figcaption>Outcome: ")?;
    column(w, r, r.request.outcome)?;
    w.write_all(b". Markers omit pairs with unavailable coordinates; their independent values and reasons remain in the exact table.</figcaption></figure>")?;
    Ok(())
}
fn plots(
    w: &mut dyn Write,
    run_id: RunId,
    r: &ComputedRegression,
    d: &RegressionDiagnostics,
    c: &JobControl,
) -> AppResult<()> {
    residual_plot(w, run_id, r, d, c)?;
    qq_plot(w, run_id, r, d, c)
}
fn residual_plot(
    w: &mut dyn Write,
    run_id: RunId,
    r: &ComputedRegression,
    d: &RegressionDiagnostics,
    c: &JobControl,
) -> AppResult<()> {
    plot_heading(w, run_id, "residual", "Residual versus fitted values", r)?;
    plot_notice(
        w,
        d.residual_points.len(),
        r.fitted_rows as u64,
        d.residual_reduced,
        "fitted source rows",
    )?;
    if let (Some((xl, xh)), Some((yl, yh))) = (bounds(&d.fitted_range), bounds(&d.residual_range)) {
        let xs = ChartScale::new(xl, xh)?;
        let ys = ChartScale::new(yl, yh)?;
        report_charts::svg_start(w, run_id, "residual", "Residual versus fitted values", "Raw residuals against fitted values. Full-cohort bounds and exact retained source rows follow.", &xs, &ys)?;
        report_charts::axes(w, "Fitted value", "Raw residual", &xs, &ys)?;
        for point in &d.residual_points {
            c.check_cancelled()?;
            marker(w, &point.fitted, &point.residual, &xs, &ys)?;
        }
        plot_end(w, r)?;
    } else {
        unavailable_drawing(w, &d.fitted_range, &d.residual_range)?;
    }
    residual_table(w, r, d, c)
}
fn residual_table(
    w: &mut dyn Write,
    r: &ComputedRegression,
    d: &RegressionDiagnostics,
    c: &JobControl,
) -> AppResult<()> {
    write!(w, "<p>Fitted values and raw residuals for ")?;
    column(w, r, r.request.outcome)?;
    w.write_all(b".</p><details><summary>Exact retained residual diagnostics</summary>")?;
    table(
        w,
        "Exact retained residual diagnostics",
        &[
            "Cohort ordinal (starting at 1)",
            "Source row (starting at 1)",
            "Frequency",
            "Fitted value",
            "Raw residual",
            "Per-observation leverage",
            "Internally studentised residual",
            "Cook’s distance",
        ],
    )?;
    for point in &d.residual_points {
        c.check_cancelled()?;
        write!(
            w,
            "<tr><th scope=\"row\">{}</th><td>{}</td><td>{}</td>",
            point.cohort_ordinal + 1,
            point.row + 1,
            point.frequency
        )?;
        estimates(
            w,
            &[
                &point.fitted,
                &point.residual,
                &point.leverage,
                &point.studentised,
                &point.cooks_distance,
            ],
        )?;
        w.write_all(b"</tr>")?;
    }
    end_table(w)?;
    w.write_all(b"</details></div>")?;
    Ok(())
}
fn qq_plot(
    w: &mut dyn Write,
    run_id: RunId,
    r: &ComputedRegression,
    d: &RegressionDiagnostics,
    c: &JobControl,
) -> AppResult<()> {
    plot_heading(w, run_id, "qq", "Normal Q–Q plot of raw residuals", r)?;
    plot_notice(
        w,
        d.qq_points.len(),
        r.frequency,
        d.qq_reduced,
        "replicated observation ranks",
    )?;
    if let (Some((xl, xh)), Some((yl, yh))) =
        (bounds(&d.theoretical_range), bounds(&d.residual_range))
    {
        let xs = ChartScale::new(xl, xh)?;
        let ys = ChartScale::new(yl, yh)?;
        report_charts::svg_start(w, run_id, "qq", "Normal Q–Q plot of raw residuals", "Theoretical normal quantiles against raw residuals. Exact integer replicated ranks and source rows follow.", &xs, &ys)?;
        report_charts::axes(w, "Theoretical normal quantile", "Raw residual", &xs, &ys)?;
        for point in &d.qq_points {
            c.check_cancelled()?;
            marker(w, &point.theoretical, &point.residual, &xs, &ys)?;
        }
        plot_end(w, r)?;
    } else {
        unavailable_drawing(w, &d.theoretical_range, &d.residual_range)?;
    }
    qq_table(w, d, c)
}
fn qq_table(w: &mut dyn Write, d: &RegressionDiagnostics, c: &JobControl) -> AppResult<()> {
    w.write_all("<details><summary>Exact retained Q–Q ranks</summary>".as_bytes())?;
    table(
        w,
        "Exact retained Q–Q ranks",
        &[
            "Replicated rank (starting at 1)",
            "Source row (starting at 1)",
            "Source-row frequency",
            "Theoretical normal quantile",
            "Raw residual",
        ],
    )?;
    for point in &d.qq_points {
        c.check_cancelled()?;
        write!(
            w,
            "<tr><th scope=\"row\">{}</th><td>{}</td><td>{}</td>",
            point.rank + 1,
            point.row + 1,
            point.frequency
        )?;
        estimates(w, &[&point.theoretical, &point.residual])?;
        w.write_all(b"</tr>")?;
    }
    end_table(w)?;
    w.write_all(b"</details></div>")?;
    Ok(())
}
fn definitions(w: &mut dyn Write, r: &ComputedRegression, c: &JobControl) -> AppResult<()> {
    w.write_all(b"<h3>Variable definitions at analysis time</h3><dl>")?;
    for info in &r.columns {
        c.check_cancelled()?;
        w.write_all(b"<dt>")?;
        text(w, &info.name)?;
        write!(w, " (column {}, {:?})</dt><dd>", info.id.0, info.kind)?;
        text(w, &info.description)?;
        w.write_all(b"<p>Missing definitions: ")?;
        missing(w, &info.missing_rules)?;
        w.write_all(b".</p>")?;
        if !info.value_labels.is_empty() {
            w.write_all(b"<ul>")?;
            for label in &info.value_labels {
                c.check_cancelled()?;
                w.write_all(b"<li>")?;
                cell(w, &label.value)?;
                w.write_all(b": ")?;
                text(w, &label.label)?;
                w.write_all(b"</li>")?;
            }
            w.write_all(b"</ul>")?;
        }
        w.write_all(b"</dd>")?;
    }
    w.write_all(b"</dl>")?;
    Ok(())
}
