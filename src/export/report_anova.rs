//! Historical ANOVA tables consume captured values and never consult current data.
use super::{
    report::text,
    report_analyses::{cell, confidence, estimate, p_value},
};
use crate::{
    analysis::*,
    data::{Cell, ColumnId},
    error::AppResult,
    operation::{JobControl, RunId},
};
use std::io::Write;
#[path = "report_regression/cancellation.rs"]
mod cancellation;
pub(crate) const METHOD: &str = "Classical fixed-effects ANOVA uses the realised complete crossed grid. Independent factorial effects use Type III equal-cell marginal hypotheses in the full factorial model; main effects average cells equally, including when interactions exist. Frequency weights represent literal independent replication. Repeated measures use complete subject profiles, effect-specific subject-by-effect errors and Greenhouse–Geisser (GG) correction without a preliminary sphericity test. Classical inference assumes independent observations or subjects, the specified mean model and normally distributed errors or multivariate normal subject profiles; independent errors also require common variance. Uncorrected repeated multi-df inference additionally assumes sphericity. These results do not establish independence, ignorable missingness or causal effects.";
pub(crate) fn write(
    w: &mut dyn Write,
    id: RunId,
    r: &ComputedAnova,
    c: &JobControl,
) -> AppResult<()> {
    cancellation::write(w, c, |w, c| output(w, id, r, c))
}
fn paragraph(w: &mut dyn Write, s: &str) -> AppResult<()> {
    w.write_all(b"<p>")?;
    text(w, s)?;
    w.write_all(b"</p>")?;
    Ok(())
}
fn row(w: &mut dyn Write, name: &str, e: &Estimate) -> AppResult<()> {
    w.write_all(b"<tr><th scope=\"row\">")?;
    text(w, name)?;
    w.write_all(b"</th><td>")?;
    if name.ends_with(" p") {
        p_value(w, e)?;
    } else {
        estimate(w, e)?;
    }
    w.write_all(b"</td></tr>")?;
    Ok(())
}
fn energy(w: &mut dyn Write, name: &str, e: &AnovaEnergy) -> AppResult<()> {
    row(w, name, &e.value)?;
    w.write_all(b"<tr><th scope=\"row\">Energy state</th><td>")?;
    text(
        w,
        match e.state {
            AnovaQuantityState::Positive => "Positive carried energy",
            AnovaQuantityState::ExactZero => "Exact zero",
            AnovaQuantityState::Unavailable(_) => "Unavailable carried energy",
        },
    )?;
    w.write_all(b"</td></tr>")?;
    Ok(())
}
fn column(w: &mut dyn Write, r: &ComputedAnova, id: ColumnId) -> AppResult<()> {
    if let Some(v) = r.columns.iter().find(|v| v.id == id) {
        text(w, &v.name)
    } else {
        write!(w, "Column {}", id.0)?;
        Ok(())
    }
}
fn category(
    w: &mut dyn Write,
    r: &ComputedAnova,
    f: &AnovaFactorLevels,
    v: &Cell,
) -> AppResult<()> {
    cell(w, v)?;
    if let Some(info) = f
        .column
        .and_then(|id| r.columns.iter().find(|v| v.id == id))
    {
        if let Some(label) = info.value_labels.iter().find(|label| label.value == *v) {
            text(w, " · ")?;
            text(w, &label.label)?;
        }
    }
    Ok(())
}
fn scope(w: &mut dyn Write, s: &AnovaScope) -> AppResult<()> {
    match s {
        AnovaScope::Marginal => text(w, "Marginal: equal averaging over every other factor"),
        AnovaScope::Fixed(v) => {
            text(w, "Fixed slice (other factors in captured order): ")?;
            for (i, v) in v.iter().enumerate() {
                if i > 0 {
                    w.write_all(b"; ")?;
                }
                cell(w, v)?;
            }
            Ok(())
        }
    }
}
fn test(w: &mut dyn Write, t: &AnovaTest) -> AppResult<()> {
    for (label, e) in [
        ("Numerator df", &t.df1),
        ("Error df", &t.df2),
        ("F", &t.f),
        ("Upper-tail p", &t.p),
    ] {
        row(w, label, e)?;
    }
    Ok(())
}
fn output(w: &mut dyn Write, id: RunId, r: &ComputedAnova, c: &JobControl) -> AppResult<()> {
    write!(w,"<section aria-labelledby=\"run-{}\"><h2 id=\"run-{}\">Analysis run {} · ANOVA</h2><p>Calculated from data revision {}. Names, definitions and results were captured when this analysis ran.</p>",id.0,id.0,id.0,r.revision.0)?;
    paragraph(w, METHOD)?;
    write!(
        w,
        "<p>Numerical method version {}; basis version {}. {} pointwise confidence intervals.</p>",
        r.method.numerical_version,
        r.method.basis_version,
        confidence(r.request.confidence)
    )?;
    w.write_all(b"<h3>Captured settings</h3><p>Design: ")?;
    match &r.request.design {
        AnovaDesign::OneWay { outcome, factor } => {
            text(w, "One-way independent; outcome ")?;
            column(w, r, *outcome)?;
            text(w, "; categorical factor ")?;
            column(w, r, *factor)?;
        }
        AnovaDesign::Factorial { outcome, factors } => {
            text(w, "Full factorial independent; outcome ")?;
            column(w, r, *outcome)?;
            for id in factors {
                text(w, "; categorical factor ")?;
                column(w, r, *id)?;
            }
        }
        AnovaDesign::Repeated {
            subject,
            measurements,
            ..
        } => {
            text(w, "Repeated measures; subject-ID column ")?;
            column(w, r, *subject)?;
            text(
                w,
                " (personal ID values are omitted). Ordered measurement mapping: ",
            )?;
            for (i, id) in measurements.iter().enumerate() {
                c.checkpoint(i, None)?;
                if i > 0 {
                    text(w, "; ")?;
                }
                write!(w, "{}: ", i + 1)?;
                column(w, r, *id)?;
            }
        }
    }
    if let Some(id) = r.request.frequency_weight {
        text(w, "; literal frequency-weight column ")?;
        column(w, r, id)?;
    }
    w.write_all(b".</p>")?;
    filters(w, r, c)?;
    write!(w,"<h3>Cohort</h3><p>Source rows: {}. Fitted physical rows / retained subjects: {}. Independent replicated frequency / complete subjects N: {}. Incomplete-case frequency: {}. Measurements per complete subject K: {}. Observed measurements: {}.</p><p>Excluded source rows: {} by filter; {} with missing weights; {} with valid zero weights; {} incomplete cases.</p>",r.source_rows,r.fitted_rows,r.frequency,r.incomplete_case_frequency,match &r.request.design { AnovaDesign::Repeated{measurements,..}=>measurements.len(), _=>1 },r.observed_measurements,r.diagnostics.filtered_rows,r.diagnostics.missing_weight_rows,r.diagnostics.zero_weight_rows,r.diagnostics.complete_case_rows)?;
    paragraph(w,"The captured grid contains realised independent levels or the declared repeated measurement levels. Exact text identity is shown in quotes; whitespace escapes distinguish labels. Level labels do not merge identities. Grid cell order follows factor order, with the last factor varying fastest.")?;
    for (i, f) in r.grid.factors.iter().enumerate() {
        c.checkpoint(i, None)?;
        write!(w, "<h4>Factor {} · ", i + 1)?;
        text(w, &f.name)?;
        w.write_all(b"</h4><ol>")?;
        for v in &f.values {
            w.write_all(b"<li>")?;
            category(w, r, f, v)?;
            w.write_all(b"</li>")?;
        }
        w.write_all(b"</ol>")?;
    }
    write!(w,"<p>Complete crossed grid: {}.</p><table><caption>Captured cells</caption><thead><tr><th>Cell</th><th>Physical rows</th><th>Frequency / subjects</th><th>Mean</th></tr></thead><tbody>",r.grid.complete)?;
    for (i, v) in r.grid.cells.iter().enumerate() {
        c.checkpoint(i, None)?;
        write!(
            w,
            "<tr><th scope=\"row\">{}</th><td>{}</td><td>{}</td><td>",
            i + 1,
            v.physical_rows,
            v.frequency
        )?;
        estimate(w, &v.mean)?;
        w.write_all(b"</td></tr>")?;
    }
    w.write_all(b"</tbody></table>")?;
    match &r.outcome {
        AnovaOutcome::Unavailable(f) => paragraph(w, &f.reason)?,
        AnovaOutcome::Fitted(m) => {
            write!(w,"<h3>Model</h3><p>Balanced positive cell counts: {}. Residual df: {}. Total df: {}.</p><table><caption>Captured model energies</caption><tbody>",m.balanced,m.residual_df,m.total_df)?;
            for (name, v) in [
                ("Error SS", &m.error_ss),
                ("Model SS", &m.model_ss),
                ("Total SS", &m.total_ss),
            ] {
                energy(w, name, v)?;
            }
            if let Some(v) = &m.subject_ss {
                energy(w, "Subject SS", v)?;
            }
            w.write_all(b"</tbody></table>")?;
            for (i, e) in m.effects.iter().enumerate() {
                c.checkpoint(i, None)?;
                write!(w, "<h4>Effect {} · ", i + 1)?;
                for (j, f) in e.factors.iter().enumerate() {
                    if j > 0 {
                        text(w, " × ")?;
                    }
                    text(w, &r.grid.factors[*f].name)?;
                }
                write!(w,"</h4><p>Hypothesis dimension: {}.</p><table><caption>Uncorrected effect and error</caption><tbody>",e.dimension)?;
                energy(w, "Effect SS", &e.ss)?;
                row(w, "Effect MS", &e.ms)?;
                energy(w, "Effect-specific error SS", &e.error_ss)?;
                row(w, "Effect-specific error MS", &e.error_ms)?;
                test(w, &e.test)?;
                row(w, "Eta squared", &e.eta_squared)?;
                row(w, "Partial eta squared", &e.partial_eta_squared)?;
                w.write_all(b"</tbody></table>")?;
                if let Some(f) = &e.failure {
                    paragraph(w, &f.reason)?;
                }
                if let Some(g) = &e.gg {
                    w.write_all(b"<table><caption>Greenhouse-Geisser corrected inference (primary for dimension greater than one)</caption><tbody>")?;
                    row(w, "GG epsilon", &g.epsilon)?;
                    test(w, &g.test)?;
                    w.write_all(b"</tbody></table>")?;
                    if let Some(f) = &g.failure {
                        paragraph(w, &f.reason)?;
                    }
                }
            }
        }
    }
    paragraph(w,"Independent eta squared is effect SS / total SS. In unbalanced Type III models, effect SS generally do not partition model SS. Partial eta squared is effect SS / (effect SS + its error SS). Ordinary eta squared is withheld for repeated designs. GG changes degrees of freedom and p, while SS, MS and effect sizes retain their uncorrected definitions; dimension-one sphericity is automatic.")?;
    paragraph(w,"Pair differences are first requested level minus second. Marginal pairs average cells equally; fixed slices condition on every other factor. Intervals are pointwise, not simultaneous. Two-sided raw p and Holm-adjusted p are reported within each fixed family; an unavailable raw p leaves every adjusted p unavailable without removing any member.")?;
    for (i, f) in r.families.iter().enumerate() {
        c.checkpoint(i, None)?;
        write!(w, "<h3>Comparison family {} · ", i + 1)?;
        match &f.kind {
            AnovaFamilyKind::Planned => text(w, "Planned")?,
            AnovaFamilyKind::AllPairs(v) => {
                text(w, "All pairs for ")?;
                text(w, &r.grid.factors[v.factor].name)?;
                text(w, " · ")?;
                scope(w, &v.scope)?;
            }
        }
        write!(w, "</h3><p>Fixed family size m: {}.</p>", f.m)?;
        if let Some(v) = &f.adjustment_failure {
            paragraph(w, &v.reason)?;
        }
        for v in &f.members {
            c.checkpoint(v.ordinal, None)?;
            write!(w, "<h4>Member {} · ", v.ordinal + 1)?;
            text(w, &r.grid.factors[v.request.factor].name)?;
            text(w, ": ")?;
            cell(w, &v.request.first)?;
            text(w, " minus ")?;
            cell(w, &v.request.second)?;
            w.write_all(b"</h4><p>")?;
            scope(w, &v.request.scope)?;
            w.write_all(b"</p>")?;
            match &v.state {
                AnovaComparisonState::Available => {}
                AnovaComparisonState::AbsentRequestedLevel(f)
                | AnovaComparisonState::Unavailable(f) => paragraph(w, &f.reason)?,
            }
            w.write_all(b"<table><caption>Captured pair inference</caption><tbody>")?;
            for (name, e) in [
                ("Difference", &v.estimate),
                ("Standard error", &v.standard_error),
                ("t", &v.t),
                ("df", &v.df),
                ("Raw two-sided p", &v.raw_p),
                ("Holm-adjusted p", &v.adjusted_p),
                ("Pointwise lower bound", &v.interval[0]),
                ("Pointwise upper bound", &v.interval[1]),
            ] {
                row(w, name, e)?;
            }
            w.write_all(b"</tbody></table>")?;
        }
    }
    definitions(w, r, c)?;
    w.write_all(b"</section>")?;
    c.check_cancelled()
}

fn subject(r: &ComputedAnova) -> Option<ColumnId> {
    match &r.request.design {
        AnovaDesign::Repeated { subject, .. } => Some(*subject),
        _ => None,
    }
}
fn filters(w: &mut dyn Write, r: &ComputedAnova, c: &JobControl) -> AppResult<()> {
    w.write_all(b"<p>Captured filter: ")?;
    if r.request.filter.predicates.is_empty() {
        text(w, "all rows")?;
    }
    for (i, p) in r.request.filter.predicates.iter().enumerate() {
        c.checkpoint(i, None)?;
        if i > 0 {
            text(w, " AND ")?;
        }
        column(w, r, p.column())?;
        if Some(p.column()) == subject(r) {
            text(w, " (subject-ID predicate value omitted)")?;
            continue;
        }
        match p {
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
                quoted(w, value)?;
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
    w.write_all(b"</p>")?;
    Ok(())
}
fn definitions(w: &mut dyn Write, r: &ComputedAnova, c: &JobControl) -> AppResult<()> {
    w.write_all(b"<h3>Captured column definitions</h3><dl>")?;
    for info in &r.columns {
        c.checkpoint(0, None)?;
        w.write_all(b"<dt>")?;
        text(w, &info.name)?;
        w.write_all(b"</dt><dd>")?;
        text(w, &info.description)?;
        write!(
            w,
            "<p>Column {}; captured kind {}.</p>",
            info.id.0,
            match info.kind {
                crate::data::ColumnKind::Number => "Number",
                crate::data::ColumnKind::Text => "Text",
            }
        )?;
        if Some(info.id) == subject(r) {
            text(
                w,
                "Subject-ID value labels and missing-rule tokens omitted.",
            )?;
        } else {
            text(w, "Captured missing rules: ")?;
            super::report::missing(w, &info.missing_rules)?;
            for label in &info.value_labels {
                c.checkpoint(0, None)?;
                w.write_all(b"<p>")?;
                cell(w, &label.value)?;
                text(w, " → ")?;
                text(w, &label.label)?;
                w.write_all(b"</p>")?;
            }
        }
        w.write_all(b"</dd>")?;
    }
    w.write_all(b"</dl>")?;
    Ok(())
}

fn quoted(w: &mut dyn Write, value: &str) -> AppResult<()> {
    w.write_all(b"<span style=\"white-space:pre-wrap\">")?;
    text(w, "\"")?;
    for ch in value.chars() {
        for escaped in ch.escape_debug() {
            let mut bytes = [0; 4];
            text(w, escaped.encode_utf8(&mut bytes))?;
        }
    }
    text(w, "\"")?;
    w.write_all(b"</span>")?;
    Ok(())
}
