//! Standalone rendering of a frozen regression capture; shell dispatch is separate.
use super::{chart_drawing, theme, DesktopState};
use crate::{
    analysis::*,
    charts::{CategoryCaption, ChartScale},
    data::ColumnId,
    operation::RunId,
};
use eframe::egui::{self, Pos2, Stroke};
mod plots;
#[cfg(test)]
mod tests;

pub(super) fn show(
    ui: &mut egui::Ui,
    state: &mut DesktopState,
    run_id: RunId,
    r: &ComputedRegression,
) {
    ui.push_id(run_id, |ui| {
        ui.heading(format!("Linear regression · run {}", run_id.0));
        super::result_tables::revision_status(ui, r.revision.0, state.selected_run_is_stale());
        ui.label(format!(
            "Fitted source rows M: {} · literal replicated frequency N: {} · source rows: {}",
            r.fitted_rows, r.frequency, r.source_rows
        ));
        ui.label(format!(
            "Complete-case excluded frequency: {}",
            r.incomplete_case_frequency
        ));
        ui.collapsing("Method", |ui| {
            ui.label(crate::export::report_regression::METHOD);
        });
        ui.label("Complete captured text and exact values: Export report…");
        match &r.outcome {
            RegressionOutcome::Unavailable(f) => failure(ui, r, f),
            RegressionOutcome::Fitted(f) => {
                coefficients(ui, r, f);
                fit_statistics(ui, f);
                diagnostic_summary(ui, &f.residual_diagnostics);
                plots::show(ui, state, run_id, r, &f.residual_diagnostics);
            }
        }
    });
}
fn name(r: &ComputedRegression, id: ColumnId) -> &str {
    r.columns
        .iter()
        .find(|c| c.id == id)
        .map_or("Unavailable variable", |c| c.name.as_str())
}
fn term(r: &ComputedRegression, index: usize) -> String {
    let label = match &r.terms[index] {
        RegressionTerm::Intercept => "Intercept".into(),
        RegressionTerm::Numeric { column } => {
            theme::caption(format_args!("{} (numeric slope)", name(r, *column)))
        }
        RegressionTerm::Contrast {
            column,
            level,
            reference,
        } => theme::caption(format_args!(
            "{}: {} versus reference {}",
            name(r, *column),
            CategoryCaption(level),
            CategoryCaption(reference)
        )),
    };
    format!("Term {} · {label}", index + 1)
}
fn estimate(e: &Estimate) -> String {
    e.value.map_or_else(
        || theme::caption(e.reason.as_deref().unwrap_or("Unavailable")),
        |v| v.to_string(),
    )
}
fn context(ui: &mut egui::Ui, text: String) {
    let response = ui.add(
        egui::Label::new(&text)
            .sense(egui::Sense::focusable_noninteractive())
            .truncate(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
    response.on_hover_text(&text);
}
fn value(ui: &mut egui::Ui, label: &str, e: &Estimate) {
    ui.horizontal(|ui| {
        ui.label(theme::caption(label));
        super::result_tables::contextual_value(ui, label, e, "");
    });
}
fn rows(ui: &mut egui::Ui, id: &str, len: usize, mut row: impl FnMut(&mut egui::Ui, usize)) {
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(270.)
        .show_rows(ui, 28., len, |ui, range| {
            for i in range {
                ui.push_id(i, |ui| {
                    row(ui, i);
                });
            }
        });
}
fn failure(ui: &mut egui::Ui, r: &ComputedRegression, f: &RegressionFailure) {
    ui.heading("Fit unavailable");
    context(ui, theme::caption(&f.reason));
    ui.label(format!(
        "Expected terms: {} · Numerical rank: {}",
        f.expected_terms
            .map_or_else(|| "unavailable".into(), |p| p.to_string()),
        f.rank
            .map_or_else(|| "unavailable".into(), |p| p.to_string())
    ));
    for index in &f.implicated_terms {
        if *index < r.terms.len() {
            context(ui, format!("Implicated {}", term(r, *index)));
        }
    }
    rows(ui, "failed-terms", r.terms.len(), |ui, i| {
        context(ui, term(r, i));
    });
}
fn coefficients(ui: &mut egui::Ui, r: &ComputedRegression, f: &RegressionFit) {
    ui.heading("Coefficients");
    ui.label(format!(
        "Expanded terms p: {} · numerical rank: {} · model df: {} · residual df: {} · total df: {}",
        r.terms.len(),
        f.rank,
        f.model_df,
        f.residual_df,
        f.total_df
    ));
    ui.label(format!(
        "{} confidence intervals · unadjusted two-sided tests",
        crate::export::report_analyses::confidence(r.request.confidence)
    ));
    if f.residual_df == 0 {
        ui.label("There are no residual degrees of freedom. Coefficients may be available; residual-variance inference is unavailable.");
    } else if f.statistics.residual_standard_error.value == Some(0.) {
        ui.label("Zero residual variance: point intervals are an algebraic limit and do not establish error-free population effects. Tests are unavailable.");
    }
    if f.condition_warned {
        ui.label("Condition warning: coefficients and inference may be sensitive to small input changes.");
    }
    rows(ui, "regression-coefficients", r.terms.len(), |ui, i| {
        ui.horizontal(|ui| {
            let label = term(r, i);
            context(ui, label.clone());
            let c = &f.coefficients[i];
            for (metric, e) in [
                ("Coefficient", &c.estimate),
                ("Standard error", &c.standard_error),
                ("t", &c.t),
                ("Two-sided p", &c.p),
                ("Interval lower", &c.interval[0]),
                ("Interval upper", &c.interval[1]),
                ("Captured term scale", &f.term_scales[i]),
            ] {
                value(ui, &format!("{label} · {metric}"), e);
            }
        });
    });
}
fn fit_statistics(ui: &mut egui::Ui, f: &RegressionFit) {
    ui.heading("Model fit");
    let s = &f.statistics;
    for (label, e) in [
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
        value(ui, label, e);
    }
}
fn diagnostic_summary(ui: &mut egui::Ui, d: &RegressionDiagnostics) {
    ui.heading("Residual diagnostics");
    ui.collapsing("Diagnostic definitions", |ui| {
        ui.label(crate::export::report_regression::DIAGNOSTICS);
    });
    for (label, e) in [
        ("Residual mean", &d.residual_mean),
        ("Residual sample SD (N−1)", &d.residual_sample_sd),
        ("Residual minimum", &d.residual_range[0]),
        ("Residual maximum", &d.residual_range[1]),
        ("Fitted minimum", &d.fitted_range[0]),
        ("Fitted maximum", &d.fitted_range[1]),
        ("Theoretical quantile minimum", &d.theoretical_range[0]),
        ("Theoretical quantile maximum", &d.theoretical_range[1]),
    ] {
        value(ui, label, e);
    }
    for (label, extent) in [
        ("Per-observation leverage", &d.leverage),
        ("Internally studentised residual", &d.studentised),
        (
            "Cook’s distance (one replicated observation)",
            &d.cooks_distance,
        ),
    ] {
        value(ui, &format!("{label} · minimum"), &extent.range[0]);
        value(ui, &format!("{label} · maximum"), &extent.range[1]);
        ui.label(format!(
            "{label} · undefined source rows: {} · undefined frequency: {}",
            extent.undefined_rows, extent.undefined_frequency
        ));
    }
}
pub(super) fn settings(ui: &mut egui::Ui, _state: &DesktopState, r: &ComputedRegression) {
    ui.push_id((r.revision, "regression-settings"), |ui| {
        ui.heading("Captured settings");
        ui.label(theme::caption(format_args!("Outcome: {}", name(r, r.request.outcome))));
        ui.label(format!("{} confidence · required intercept · complete cases", crate::export::report_analyses::confidence(r.request.confidence)));
        ui.label(theme::caption(format_args!("Numerical version: {} · centred/scaled design and response · rank threshold: {} · condition warning strictly above {}", r.method.numerical_version, r.method.rank_threshold, r.method.condition_warning)));
        rows(ui, "regression-predictors", r.request.predictors.len(), |ui, i| {
            let p = &r.request.predictors[i];
            context(ui, match &p.role { PredictorRole::Numeric => theme::caption(format_args!("Predictor {} · {}: numeric", i + 1, name(r, p.column))), PredictorRole::Categorical { reference } => theme::caption(format_args!("Predictor {} · {}: categorical · reference {}", i + 1, name(r, p.column), CategoryCaption(reference))) });
        });
        for (i, levels) in r.levels.iter().enumerate() {
            ui.push_id(i, |ui| { rows(ui, "regression-levels", levels.values.len(), |ui, j| { context(ui, theme::caption(format_args!("{} · Level {}: {}", name(r, levels.column), j + 1, CategoryCaption(&levels.values[j])))); }); });
        }
        ui.label(r.request.frequency_weight.map_or_else(|| "Unweighted observations".into(), |id| theme::caption(format_args!("Frequency weight: {} · literal repeated observations", name(r, id)))));
        if !r.request.filter.predicates.is_empty() { ui.label("Filter: all listed conditions must hold"); }
        rows(ui, "regression-filter", r.request.filter.predicates.len(), |ui, i| { context(ui, filter(r, &r.request.filter.predicates[i])); });
        let d = &r.diagnostics;
        ui.label(format!("Excluded source rows: {} filtered; {} missing weights; {} zero weights; {} incomplete cases", d.filtered_rows, d.missing_weight_rows, d.zero_weight_rows, d.complete_case_rows));
        definitions(ui, r);
    });
}
fn filter(r: &ComputedRegression, p: &RowPredicate) -> String {
    match p {
        RowPredicate::Number {
            column,
            comparison,
            value,
        } => theme::caption(format_args!(
            "{} {} {}",
            name(r, *column),
            match comparison {
                NumericComparison::Equal => "=",
                NumericComparison::NotEqual => "!=",
                NumericComparison::Less => "<",
                NumericComparison::LessOrEqual => "<=",
                NumericComparison::Greater => ">",
                NumericComparison::GreaterOrEqual => ">=",
            },
            value
        )),
        RowPredicate::Text {
            column,
            value,
            equal,
        } => theme::caption(format_args!(
            "{} {} {:?}",
            name(r, *column),
            if *equal { "equals" } else { "does not equal" },
            value
        )),
        RowPredicate::Missing { column, missing } => theme::caption(format_args!(
            "{} is {}",
            name(r, *column),
            if *missing { "missing" } else { "present" }
        )),
    }
}

fn definitions(ui: &mut egui::Ui, r: &ComputedRegression) {
    let Some(total) = r.columns.iter().try_fold(0usize, |n, c| {
        n.checked_add(2)?.checked_add(c.value_labels.len())
    }) else {
        ui.label("Captured definitions unavailable in this view · complete definitions remain in the report");
        return;
    };
    egui::ScrollArea::both()
        .id_salt("regression-definitions")
        .max_height(270.)
        .show_rows(ui, 28., total, |ui, range| {
            let mut offset = 0;
            for column in &r.columns {
                let end = offset + 2 + column.value_labels.len();
                for index in range.start.max(offset)..range.end.min(end) {
                    ui.push_id(index, |ui| {
                        let text = match index - offset {
                            0 => theme::caption(format_args!(
                                "Column {} · {} ({:?}) · {}",
                                column.id.0, column.name, column.kind, column.description
                            )),
                            1 => theme::caption(format_args!(
                                "{} · Missing definitions: {:?}",
                                column.name, column.missing_rules
                            )),
                            i => {
                                let label = &column.value_labels[i - 2];
                                theme::caption(format_args!(
                                    "{} · Value label {} · {}: {}",
                                    column.name,
                                    i - 1,
                                    CategoryCaption(&label.value),
                                    label.label
                                ))
                            }
                        };
                        context(ui, text);
                    });
                }
                offset = end;
                if offset >= range.end {
                    break;
                }
            }
        });
}
