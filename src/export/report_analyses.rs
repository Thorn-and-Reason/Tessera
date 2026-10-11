//! Procedure-specific tables stream historical values without cloning their recipes.
use super::report::{column_name, missing, text};
use crate::{
    analysis::*,
    data::Cell,
    error::AppResult,
    operation::JobControl,
    project::{AnalysisRun, Project},
};
use std::io::Write;
pub(crate) const PEARSON_METHOD: &str = "Pearson r measures linear association. The two-sided test of zero population correlation assumes independent observations and a bivariate normal distribution. P-values are unadjusted across pairs.";
pub(super) fn estimate(w: &mut dyn Write, e: &Estimate) -> AppResult<()> {
    if let Some(v) = e.value {
        super::number(w, v)?
    } else {
        text(w, e.reason.as_deref().unwrap_or("Unavailable"))?
    }
    Ok(())
}
fn p_value(w: &mut dyn Write, e: &Estimate) -> AppResult<()> {
    if e.value.is_some_and(|v| v < 0.001) {
        text(w, "<0.001")?;
    } else {
        estimate(w, e)?;
    }
    Ok(())
}
fn row(w: &mut dyn Write, label: &str, e: &Estimate) -> AppResult<()> {
    w.write_all(b"<tr><th scope=\"row\">")?;
    text(w, label)?;
    w.write_all(b"</th><td>")?;
    if label == "Two-sided p" {
        p_value(w, e)?;
    } else {
        estimate(w, e)?;
    }
    w.write_all(b"</td></tr>")?;
    Ok(())
}
fn sample(w: &mut dyn Write, s: &SampleSummary) -> AppResult<()> {
    write!(w, "<p>N: {}.</p><table><tbody>", s.n)?;
    row(w, "Mean", &s.mean)?;
    row(w, "Sample SD", &s.sample_sd)?;
    w.write_all(b"</tbody></table>")?;
    Ok(())
}
fn statistics(w: &mut dyn Write, s: &TStatistics, c: ConfidenceLevel) -> AppResult<()> {
    write!(
        w,
        "<h3>Inference</h3><p>{} confidence interval; two-sided p.</p><table><tbody>",
        confidence(c)
    )?;
    for (label, e) in [
        ("Difference", &s.difference),
        ("Standard error", &s.standard_error),
        ("t", &s.t),
        ("df", &s.df),
        ("Two-sided p", &s.p),
        ("Interval lower", &s.interval[0]),
        ("Interval upper", &s.interval[1]),
        ("Effect size", &s.effect_size),
    ] {
        row(w, label, e)?;
    }
    w.write_all(b"</tbody></table>")?;
    Ok(())
}
pub(crate) fn confidence(c: ConfidenceLevel) -> &'static str {
    match c {
        ConfidenceLevel::P90 => "90%",
        ConfidenceLevel::P95 => "95%",
        ConfidenceLevel::P99 => "99%",
    }
}
pub(crate) fn method(a: &CompletedAnalysis) -> &'static str {
    match a {
        CompletedAnalysis::Anova(_) => "ANOVA",
        CompletedAnalysis::Reliability(_) => "Reliability",
        CompletedAnalysis::Regression(_) => "Linear regression",
        CompletedAnalysis::Describe(_) => "Descriptive statistics",
        CompletedAnalysis::Frequencies(_) => "Frequencies",
        CompletedAnalysis::Histogram(_) => "Histogram",
        CompletedAnalysis::Scatter(_) => "Scatterplot",
        CompletedAnalysis::Boxplot(_) => "Boxplot",
        CompletedAnalysis::Pearson(_) => "Pearson correlation",
        CompletedAnalysis::OneSampleT(_) => "One-sample t-test",
        CompletedAnalysis::PairedT(_) => "Paired t-test",
        CompletedAnalysis::IndependentT(v) => match v.request.method {
            IndependentMethod::Welch => "Independent t-test · Welch",
            IndependentMethod::Pooled => "Independent t-test · pooled equal variance",
        },
    }
}
pub(super) fn cell(w: &mut dyn Write, c: &Cell) -> AppResult<()> {
    match c {
        Cell::Number(v) => super::number(w, *v),
        Cell::Text(v) => {
            w.write_all(b"<span style=\"white-space:pre-wrap\">")?;
            text(w, "\"")?;
            for ch in v.chars() {
                for escaped in ch.escape_debug() {
                    let mut bytes = [0; 4];
                    text(w, escaped.encode_utf8(&mut bytes))?;
                }
            }
            text(w, "\"")?;
            w.write_all(b"</span>")?;
            Ok(())
        }
        Cell::Missing => text(w, "Missing"),
    }
}
pub(super) fn write(
    w: &mut dyn Write,
    run: &AnalysisRun,
    p: &Project,
    control: &JobControl,
) -> AppResult<()> {
    let a = &run.analysis;
    if let CompletedAnalysis::Anova(v) = a {
        if a.revision() != p.dataset().revision() {
            w.write_all(b"<p>Historical results: the current data revision differs.</p>")?;
        }
        return super::report_anova::write(w, run.id, v, control);
    }
    if let CompletedAnalysis::Regression(v) = a {
        if a.revision() != p.dataset().revision() {
            w.write_all(b"<p>Historical results: the current data revision differs.</p>")?;
        }
        return super::report_regression::tables(w, run.id, v, control);
    }
    if let CompletedAnalysis::Reliability(v) = a {
        if a.revision() != p.dataset().revision() {
            w.write_all(b"<p>Historical results: the current data revision differs.</p>")?;
        }
        return super::report_reliability::write(w, run.id, v, control);
    }
    control.checkpoint(0, None)?;
    write!(
        w,
        "<section aria-labelledby=\"run-{}\"><h2 id=\"run-{}\">Analysis run {} · ",
        run.id.0, run.id.0, run.id.0
    )?;
    text(w, method(a))?;
    write!(
        w,
        "</h2><p>Calculated from data revision {}. ",
        a.revision().0
    )?;
    if a.revision() != p.dataset().revision() {
        w.write_all(b"Historical results: the current data revision differs. ")?;
    }
    w.write_all(b"Names and definitions below were captured when this analysis ran.</p>")?;
    match a {
        CompletedAnalysis::Anova(_)
        | CompletedAnalysis::Regression(_)
        | CompletedAnalysis::Reliability(_) => {
            unreachable!("family rendered above")
        }
        CompletedAnalysis::Pearson(v) => {
            w.write_all(b"<p>")?;
            text(w, PEARSON_METHOD)?;
            w.write_all(b"</p>")?;
            w.write_all(match v.request.missing_policy{CorrelationMissingPolicy::PairwiseComplete=>b"<p>Pairwise complete observations: each pair uses rows present in both variables.</p>",CorrelationMissingPolicy::CompleteCases=>b"<p>Complete cases: rows must be present in every selected variable.</p>"})?;
            w.write_all(b"<table><caption>Ordered variable pairs</caption><thead><tr><th>First</th><th>Second</th><th>N</th><th>r</th><th>Two-sided p</th><th>Missing rows</th><th>Complete-case excluded rows</th></tr></thead><tbody>")?;
            for r in &v.rows {
                control.checkpoint(0, None)?;
                w.write_all(b"<tr><th scope=\"row\">")?;
                column_name(w, run, r.first)?;
                w.write_all(b"</th><td>")?;
                column_name(w, run, r.second)?;
                write!(w, "</td><td>{}</td><td>", r.n)?;
                estimate(w, &r.r)?;
                w.write_all(b"</td><td>")?;
                p_value(w, &r.p)?;
                write!(
                    w,
                    "</td><td>{}</td><td>{}</td></tr>",
                    r.missing_rows, r.complete_case_excluded_rows
                )?;
            }
            w.write_all(b"</tbody></table>")?;
        }
        CompletedAnalysis::OneSampleT(v) => {
            w.write_all(b"<p>Difference: ")?;
            column_name(w, run, v.request.variable)?;
            w.write_all(b" mean minus reference ")?;
            super::number(w, v.request.reference_mean)?;
            write!(
                w,
                ". Missing outcome rows: {}. Effect size: difference divided by sample SD.</p>",
                v.missing_rows
            )?;
            sample(w, &v.sample)?;
            statistics(w, &v.statistics, v.request.confidence)?;
        }
        CompletedAnalysis::PairedT(v) => {
            w.write_all(b"<p>Difference: First (")?;
            column_name(w, run, v.request.first)?;
            w.write_all(b") minus Second (")?;
            column_name(w, run, v.request.second)?;
            w.write_all(b"). Both values must be present. Effect size: difference divided by SD of paired differences.</p><h3>First</h3>")?;
            sample(w, &v.first)?;
            w.write_all(b"<h3>Second</h3>")?;
            sample(w, &v.second)?;
            w.write_all(b"<table><tbody>")?;
            row(w, "SD of paired differences", &v.difference_sd)?;
            w.write_all(b"</tbody></table>")?;
            statistics(w, &v.statistics, v.request.confidence)?;
        }
        CompletedAnalysis::IndependentT(v) => {
            w.write_all(b"<p>Outcome: ")?;
            column_name(w, run, v.request.outcome)?;
            w.write_all(b". Grouping variable: ")?;
            column_name(w, run, v.request.group)?;
            w.write_all(b". Difference: First group minus Second group. Effect size: difference divided by pooled sample SD.</p>")?;
            for (i, s) in v.groups.iter().enumerate() {
                write!(w, "<h3>{} group: ", if i == 0 { "First" } else { "Second" })?;
                cell(w, &v.request.groups[i])?;
                w.write_all(b"</h3>")?;
                sample(w, s)?;
            }
            write!(w,"<p>Excluded observation rows: {} missing groups; {} other groups; {} missing outcomes in selected groups.</p>",v.exclusions.missing_group_rows,v.exclusions.other_group_rows,v.exclusions.missing_outcome_rows)?;
            statistics(w, &v.statistics, v.request.confidence)?;
        }
        CompletedAnalysis::Frequencies(v) => {
            super::report_exploration::frequencies(w, run, v, control)?
        }
        CompletedAnalysis::Histogram(v) => {
            super::report_exploration::histogram_tables(w, run, v, control)?
        }
        CompletedAnalysis::Scatter(v) => {
            super::report_exploration::scatter_tables(w, run, v, control)?
        }
        CompletedAnalysis::Boxplot(v) => {
            super::report_exploration::boxplot_tables(w, run, v, control)?
        }
        CompletedAnalysis::Describe(_) => unreachable!("descriptives use the existing report"),
    }
    w.write_all(b"<h3>Settings and captured definitions</h3><p>Frequency weight: ")?;
    if let Some(id) = a.request().frequency_weight() {
        column_name(w, run, id)?;
        w.write_all(b"; literal repeated observations")?;
    } else {
        w.write_all(b"none (each observation has weight one)")?;
    }
    w.write_all(b".</p><p>Filter: ")?;
    if a.request().filter().predicates.is_empty() {
        w.write_all(b"all rows")?;
    }
    for pred in &a.request().filter().predicates {
        column_name(w, run, pred.column())?;
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
                super::number(w, *value)?;
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
                text(w, value)?;
            }
            RowPredicate::Missing { missing, .. } => {
                text(
                    w,
                    if *missing {
                        " is missing"
                    } else {
                        " is present"
                    },
                )?;
            }
        }
        w.write_all(b"; ")?;
    }
    let d = a.diagnostics();
    write!(w,".</p><p>Excluded observation rows: {} filtered; {} missing weights; {} zero weights; {} incomplete cases.</p><dl>",d.filtered_rows,d.missing_weight_rows,d.zero_weight_rows,d.complete_case_rows)?;
    for c in a.columns() {
        w.write_all(b"<dt>")?;
        text(w, &c.name)?;
        w.write_all(b"</dt><dd>")?;
        text(w, &c.description)?;
        w.write_all(b". Missing definitions: ")?;
        missing(w, &c.missing_rules)?;
        for label in &c.value_labels {
            w.write_all(b"; ")?;
            cell(w, &label.value)?;
            w.write_all(b": ")?;
            text(w, &label.label)?;
        }
        w.write_all(b"</dd>")?;
    }
    w.write_all(b"</dl></section>")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_p_threshold_does_not_change_other_estimates() {
        for (value, shown) in [(0.0005, "&lt;0.001"), (0.001, "0.001"), (0.01, "0.01")] {
            let e = Estimate {
                value: Some(value),
                reason: None,
            };
            let mut out = Vec::new();
            row(&mut out, "Two-sided p", &e).unwrap();
            assert!(String::from_utf8(out).unwrap().contains(shown));
            let mut out = Vec::new();
            row(&mut out, "Difference", &e).unwrap();
            assert!(!String::from_utf8(out).unwrap().contains("&lt;0.001"));
        }
        let mut out = Vec::new();
        row(
            &mut out,
            "Two-sided p",
            &Estimate {
                value: None,
                reason: Some("Tail < limit".into()),
            },
        )
        .unwrap();
        assert!(String::from_utf8(out).unwrap().contains("Tail &lt; limit"));
    }
}
