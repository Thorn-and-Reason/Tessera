//! Loaded results use their captured roles and definitions.
use super::{theme, DesktopState, UiAction, Workspace};
use crate::{
    analysis::*,
    data::{Cell, ColumnId},
    project::AnalysisRun,
};
use eframe::egui;
pub(super) fn name(run: &AnalysisRun, id: ColumnId) -> &str {
    run.analysis
        .columns()
        .iter()
        .find(|c| c.id == id)
        .map_or("Unavailable variable", |c| c.name.as_str())
}
fn value(ui: &mut egui::Ui, label: &str, e: &Estimate) {
    contextual_value(ui, label, e, "");
}
/// Identity band: which data revision produced this output, and whether it is
/// still the current one.
pub(super) fn revision_status(ui: &mut egui::Ui, revision: u64, stale: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("Data revision {revision}")).color(theme::muted(ui)));
        if stale {
            ui.label(
                egui::RichText::new("Data changed since this run")
                    .color(ui.visuals().warn_fg_color),
            );
        }
    });
}
pub(super) fn contextual_value(ui: &mut egui::Ui, label: &str, e: &Estimate, pair: &str) {
    let exact = e.value.map_or_else(
        || theme::caption(e.reason.as_deref().unwrap_or("Unavailable")),
        |v| v.to_string(),
    );
    let shown = e.value.map_or_else(
        || "Unavailable".into(),
        |v| {
            if label == "Two-sided p" && v < 0.001 {
                "<0.001".into()
            } else {
                theme::compact_number(v)
            }
        },
    );
    let response = ui.add(
        egui::Label::new(egui::RichText::new(shown).monospace())
            .sense(egui::Sense::focusable_noninteractive()),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            true,
            format!("{pair}{label}: {exact}"),
        )
    });
    response.on_hover_text(exact);
}
fn estimate(ui: &mut egui::Ui, label: &str, e: &Estimate) {
    ui.label(label);
    value(ui, label, e);
    ui.end_row();
}
fn sample(ui: &mut egui::Ui, id: &str, s: &SampleSummary) {
    egui::Grid::new(id).striped(true).show(ui, |ui| {
        ui.label("N");
        ui.monospace(s.n.to_string());
        ui.end_row();
        estimate(ui, "Mean", &s.mean);
        estimate(ui, "Sample SD", &s.sample_sd);
    });
}
fn statistics(ui: &mut egui::Ui, s: &TStatistics, c: ConfidenceLevel) {
    let confidence = match c {
        ConfidenceLevel::P90 => "90%",
        ConfidenceLevel::P95 => "95%",
        ConfidenceLevel::P99 => "99%",
    };
    ui.label(format!("{confidence} confidence interval · two-sided p"));
    egui::Grid::new("t-statistics")
        .striped(true)
        .show(ui, |ui| {
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
                estimate(ui, label, e);
            }
        });
}
pub(super) fn show(ui: &mut egui::Ui, state: &mut DesktopState, run: &AnalysisRun) {
    egui::ScrollArea::both()
        .id_salt("analysis-output")
        .show(ui, |ui| {
            let a = &run.analysis;
            if let CompletedAnalysis::Regression(v) = a {
                super::regression_results::show(ui, state, run.id, v);
                super::experience::guidance(ui, state.analysis_mode(), run.analysis.request());
                let panel = ui.collapsing("Run settings and provenance", |ui| {
                    super::regression_results::settings(ui, state, v)
                });
                state.controls.insert(
                    "Run settings and provenance".into(),
                    panel.header_response.rect,
                );
                if super::button(ui, state, "Use this run in Analyses").clicked() {
                    state.load_selected_request();
                    state.act(UiAction::Navigate(Workspace::Analyses));
                }
                return;
            }
            ui.heading(format!(
                "Saved result: {} · run {}",
                crate::export::report_analyses::method(a),
                run.id.0
            ));
            revision_status(ui, a.revision().0, state.selected_run_is_stale());
            match a {
                CompletedAnalysis::Reliability(v) => super::reliability_results::show(ui, state, v),
                CompletedAnalysis::Anova(v) => super::anova_results::show(ui, v),
                CompletedAnalysis::Regression(_) => unreachable!("regression rendered above"),
                CompletedAnalysis::Pearson(v) => pearson(ui, run, v),
                CompletedAnalysis::OneSampleT(v) => one_sample(ui, run, v),
                CompletedAnalysis::PairedT(v) => paired(ui, run, v),
                CompletedAnalysis::IndependentT(v) => independent(ui, run, v),
                CompletedAnalysis::Frequencies(v) => {
                    super::exploration_results::frequencies(ui, state, run, v)
                }
                CompletedAnalysis::Histogram(v) => {
                    super::exploration_results::histogram(ui, state, run, v)
                }
                CompletedAnalysis::Scatter(v) => {
                    super::exploration_results::scatter(ui, state, run, v)
                }
                CompletedAnalysis::Boxplot(v) => {
                    super::exploration_results::boxplot(ui, state, run, v)
                }
                CompletedAnalysis::Describe(_) => {
                    unreachable!("descriptives use the existing tables")
                }
            }
            super::experience::guidance(ui, state.analysis_mode(), run.analysis.request());
            let panel = ui.collapsing("Run settings and provenance", |ui| settings(ui, state, run));
            state.controls.insert(
                "Run settings and provenance".into(),
                panel.header_response.rect,
            );
            if super::button(ui, state, "Use this run in Analyses").clicked() {
                state.load_selected_request();
                state.act(UiAction::Navigate(Workspace::Analyses));
            }
        });
}
fn pearson(ui: &mut egui::Ui, run: &AnalysisRun, v: &ComputedPearson) {
    ui.label(match v.request.missing_policy {
        CorrelationMissingPolicy::PairwiseComplete => "Pairwise complete observations",
        CorrelationMissingPolicy::CompleteCases => "Complete cases across every selected variable",
    });
    ui.collapsing("Method", |ui| {
        ui.label(crate::export::report_analyses::PEARSON_METHOD);
        ui.label("Frequency weights mean literal replication of observations.");
    });
    ui.horizontal(|ui| {
        for (heading, width) in [
            ("First", 140.0),
            ("Second", 140.0),
            ("N", 70.0),
            ("r", 80.0),
            ("Two-sided p", 100.0),
            ("Missing rows", 100.0),
            ("Complete-case excluded rows", 180.0),
        ] {
            ui.add_sized(
                [width, 24.0],
                egui::Label::new(egui::RichText::new(heading).strong()),
            );
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("pearson-table")
        .max_height(360.0)
        .show_rows(ui, 24.0, v.rows.len(), |ui, range| {
            for index in range {
                let r = &v.rows[index];
                let pair = format!(
                    "{} / {} · ",
                    theme::caption_limit(name(run, r.first), 64),
                    theme::caption_limit(name(run, r.second), 64)
                );
                ui.push_id(index, |ui| {
                    ui.horizontal(|ui| {
                        for id in [r.first, r.second] {
                            ui.add_sized(
                                [140.0, 24.0],
                                egui::Label::new(theme::caption(name(run, id))).truncate(),
                            )
                            .on_hover_text(name(run, id));
                        }
                        count(ui, 70.0, &pair, "N", r.n);
                        ui.allocate_ui(egui::vec2(80.0, 24.0), |ui| {
                            contextual_value(ui, "r", &r.r, &pair)
                        });
                        ui.allocate_ui(egui::vec2(100.0, 24.0), |ui| {
                            contextual_value(ui, "Two-sided p", &r.p, &pair)
                        });
                        count(ui, 100.0, &pair, "Missing rows", r.missing_rows);
                        count(
                            ui,
                            180.0,
                            &pair,
                            "Complete-case excluded rows",
                            r.complete_case_excluded_rows,
                        );
                    })
                });
            }
        });
}
fn count(ui: &mut egui::Ui, width: f32, pair: &str, metric: &str, n: impl std::fmt::Display) {
    let response = ui.add_sized([width, 24.0], egui::Label::new(n.to_string()));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            true,
            format!("{pair}{metric}: {n}"),
        )
    });
}
fn one_sample(ui: &mut egui::Ui, run: &AnalysisRun, v: &ComputedOneSampleT) {
    ui.label(theme::caption(format_args!(
        "Difference: {} mean minus reference {}",
        name(run, v.request.variable),
        v.request.reference_mean
    )));
    sample(ui, "one-sample", &v.sample);
    ui.label(format!("Missing outcome rows: {}", v.missing_rows));
    ui.label("Effect size: difference divided by sample SD");
    statistics(ui, &v.statistics, v.request.confidence);
}
fn paired(ui: &mut egui::Ui, run: &AnalysisRun, v: &ComputedPairedT) {
    ui.label(theme::caption(format_args!(
        "First: {} · Second: {}",
        name(run, v.request.first),
        name(run, v.request.second)
    )));
    ui.label("Difference: First minus Second · both values must be present");
    sample(ui, "first-sample", &v.first);
    sample(ui, "second-sample", &v.second);
    egui::Grid::new("difference-sd").show(ui, |ui| {
        estimate(ui, "SD of paired differences", &v.difference_sd)
    });
    ui.label("Effect size: difference divided by SD of paired differences");
    statistics(ui, &v.statistics, v.request.confidence);
}
fn independent(ui: &mut egui::Ui, run: &AnalysisRun, v: &ComputedIndependentT) {
    ui.label(theme::caption(format_args!(
        "Outcome: {} · Grouping variable: {}",
        name(run, v.request.outcome),
        name(run, v.request.group)
    )));
    ui.label("Difference: First group minus Second group");
    for (i, s) in v.groups.iter().enumerate() {
        ui.strong(if i == 0 {
            "First group"
        } else {
            "Second group"
        });
        match &v.request.groups[i] {
            Cell::Text(s) => {
                ui.label(theme::caption(format_args!("{s:?}")));
            }
            Cell::Number(n) => {
                ui.monospace(n.to_string());
            }
            Cell::Missing => {
                ui.label("Missing");
            }
        }
        sample(
            ui,
            if i == 0 {
                "first-group"
            } else {
                "second-group"
            },
            s,
        );
    }
    ui.label(format!(
        "Excluded rows: {} missing groups; {} other groups; {} missing outcomes in selected groups",
        v.exclusions.missing_group_rows,
        v.exclusions.other_group_rows,
        v.exclusions.missing_outcome_rows
    ));
    ui.label("Effect size: difference divided by pooled sample SD");
    statistics(ui, &v.statistics, v.request.confidence);
}
fn settings(ui: &mut egui::Ui, state: &mut DesktopState, run: &AnalysisRun) {
    if let CompletedAnalysis::Anova(r) = &run.analysis {
        super::anova_results::settings(ui, &r.request, &r.columns);
        return;
    }
    if matches!(
        run.analysis,
        CompletedAnalysis::Frequencies(_)
            | CompletedAnalysis::Histogram(_)
            | CompletedAnalysis::Scatter(_)
            | CompletedAnalysis::Boxplot(_)
    ) {
        super::exploration_results::settings(ui, state, run);
        return;
    }
    if let CompletedAnalysis::Reliability(r) = &run.analysis {
        super::reliability_results::settings(ui, state, r);
        return;
    }
    let a = &run.analysis;
    if let Some(id) = a.request().frequency_weight() {
        ui.label(theme::caption(format_args!(
            "Frequency weight: {} · repeated observations",
            name(run, id)
        )));
    } else {
        ui.label("Unweighted observations");
    }
    for pred in &a.request().filter().predicates {
        match pred {
            RowPredicate::Number {
                column,
                comparison,
                value,
            } => {
                ui.label(theme::caption(format_args!(
                    "{} {} {}",
                    name(run, *column),
                    comparison_label(*comparison),
                    value
                )));
            }
            RowPredicate::Text {
                column,
                value,
                equal,
            } => {
                ui.label(theme::caption(format_args!(
                    "{} {} {}",
                    name(run, *column),
                    if *equal { "=" } else { "!=" },
                    value
                )));
            }
            RowPredicate::Missing { column, missing } => {
                ui.label(theme::caption(format_args!(
                    "{} is {}",
                    name(run, *column),
                    if *missing { "missing" } else { "present" }
                )));
            }
        }
    }
    let d = a.diagnostics();
    ui.label(format!(
        "Excluded rows: {} filtered; {} missing weights; {} zero weights; {} incomplete cases",
        d.filtered_rows, d.missing_weight_rows, d.zero_weight_rows, d.complete_case_rows
    ));
    for c in a.columns() {
        ui.label(theme::caption(format_args!(
            "{} · {}",
            c.name, c.description
        )));
        ui.label(theme::caption(format_args!(
            "Missing definitions: {:?}",
            c.missing_rules
        )));
        for label in &c.value_labels {
            ui.label(theme::caption(format_args!(
                "{:?}: {}",
                label.value, label.label
            )));
        }
    }
    for source in state.project().provenance() {
        ui.label(theme::caption(source));
    }
}
fn comparison_label(c: NumericComparison) -> &'static str {
    match c {
        NumericComparison::Equal => "=",
        NumericComparison::NotEqual => "!=",
        NumericComparison::Less => "<",
        NumericComparison::LessOrEqual => "<=",
        NumericComparison::Greater => ">",
        NumericComparison::GreaterOrEqual => ">=",
    }
}
