//! Passive captured output; no dataset or numerical engine enters this renderer.
use super::theme;
fn value(ui: &mut eframe::egui::Ui, label: &str, e: &crate::analysis::Estimate, context: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        super::result_tables::contextual_value(ui, label, e, context);
    });
}
use crate::{analysis::*, charts::CategoryCaption, data::ColumnId};
use eframe::egui;
struct QuotedText<'a>(&'a str);
impl std::fmt::Display for QuotedText<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
fn caption(ui: &mut egui::Ui, text: impl std::fmt::Display) {
    ui.label(theme::caption(text));
}
fn name(columns: &[RunColumnInfo], id: ColumnId) -> &str {
    columns
        .iter()
        .find(|v| v.id == id)
        .map_or("Unavailable variable", |v| v.name.as_str())
}
fn rows(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    n: usize,
    lines: usize,
    mut emit: impl FnMut(&mut egui::Ui, usize),
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let line = ui
        .text_style_height(&egui::TextStyle::Body)
        .max(ui.text_style_height(&egui::TextStyle::Monospace))
        + ui.spacing().item_spacing.y;
    let height = lines as f32 * line - ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(250.)
        .show_rows(ui, height, n, |ui, range| {
            for i in range {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), height),
                    egui::Layout::top_down(egui::Align::Min),
                    |row| {
                        row.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        emit(row, i);
                    },
                );
            }
        })
}
fn scope(ui: &mut egui::Ui, s: &AnovaScope) {
    match s {
        AnovaScope::Marginal => {
            ui.label("Equal-cell marginal averaging over other factors");
        }
        AnovaScope::Fixed(v) => {
            ui.label("Fixed slice: other factors in captured order");
            for (i, v) in v.iter().enumerate() {
                caption(ui, format_args!("Factor {}: {}", i + 1, CategoryCaption(v)));
            }
        }
    }
}
fn pair(ui: &mut egui::Ui, p: &AnovaPairRequest, factor: &str) {
    caption(
        ui,
        format_args!(
            "{}: {} minus {}",
            factor,
            CategoryCaption(&p.first),
            CategoryCaption(&p.second)
        ),
    );
    scope(ui, &p.scope);
}
pub(super) fn settings(ui: &mut egui::Ui, r: &AnovaRequest, columns: &[RunColumnInfo]) {
    ui.label("Captured ANOVA settings · read-only");
    ui.label("Complete names and captured definitions: Export report…");
    match &r.design {
        AnovaDesign::OneWay { outcome, factor } => {
            ui.label("One-way independent");
            caption(ui, format_args!("Outcome: {}", name(columns, *outcome)));
            caption(
                ui,
                format_args!("Categorical factor: {}", name(columns, *factor)),
            );
        }
        AnovaDesign::Factorial { outcome, factors } => {
            ui.label("Full factorial independent · Type III equal-cell hypotheses");
            caption(ui, format_args!("Outcome: {}", name(columns, *outcome)));
            for (i, id) in factors.iter().enumerate() {
                caption(ui, format_args!("Factor {}: {}", i + 1, name(columns, *id)));
            }
        }
        AnovaDesign::Repeated {
            subject,
            factors,
            measurements,
        } => {
            ui.label("Repeated measures · complete subject profiles");
            caption(
                ui,
                format_args!("Subject-ID column: {}", name(columns, *subject)),
            );
            for factor in factors {
                caption(ui, format_args!("Within factor: {}", factor.name));
                for label in &factor.levels {
                    caption(ui, format_args!("Level: {}", QuotedText(label)));
                }
            }
            rows(ui, "anova-mapping", measurements.len(), 1, |ui, i| {
                caption(
                    ui,
                    format_args!("Measurement {}: {}", i + 1, name(columns, measurements[i])),
                )
            });
        }
    }
    caption(
        ui,
        format_args!(
            "Confidence: {}",
            match r.confidence {
                ConfidenceLevel::P90 => "90%",
                ConfidenceLevel::P95 => "95%",
                ConfidenceLevel::P99 => "99%",
            }
        ),
    );
    if let Some(id) = r.frequency_weight {
        caption(
            ui,
            format_args!(
                "Literal independent frequency weight: {}",
                name(columns, id)
            ),
        );
    } else {
        ui.label("Unweighted");
    }
    let subject = match &r.design {
        AnovaDesign::Repeated { subject, .. } => Some(*subject),
        _ => None,
    };
    rows(
        ui,
        "anova-filters",
        r.filter.predicates.len(),
        3,
        |ui, i| {
            let p = &r.filter.predicates[i];
            caption(
                ui,
                format_args!("Filter {}: {}", i + 1, name(columns, p.column())),
            );
            if Some(p.column()) == subject {
                ui.label("Subject-ID filter value omitted");
            } else {
                match p {
                    RowPredicate::Number {
                        comparison, value, ..
                    } => caption(
                        ui,
                        format_args!(
                            "{} {value}",
                            match comparison {
                                NumericComparison::Equal => "Equals",
                                NumericComparison::NotEqual => "Not equal to",
                                NumericComparison::Less => "Less than",
                                NumericComparison::LessOrEqual => "At most",
                                NumericComparison::Greater => "Greater than",
                                NumericComparison::GreaterOrEqual => "At least",
                            }
                        ),
                    ),
                    RowPredicate::Text { value, equal, .. } => {
                        ui.label(if *equal {
                            "Equals captured text"
                        } else {
                            "Does not equal captured text"
                        });
                        caption(ui, QuotedText(value));
                    }
                    RowPredicate::Missing { missing, .. } => {
                        ui.label(if *missing { "Missing" } else { "Present" });
                    }
                }
            }
        },
    );
    ui.collapsing("Planned pair settings", |ui| {
        rows(
            ui,
            "anova-pair-settings",
            r.planned_pairs.len(),
            8,
            |ui, i| {
                caption(ui, format_args!("Member {}", i + 1));
                pair(
                    ui,
                    &r.planned_pairs[i],
                    factor_name(r, columns, r.planned_pairs[i].factor),
                );
            },
        );
    });
    for (i, f) in r.all_pairs.iter().enumerate() {
        caption(
            ui,
            format_args!("All-pairs family {} · factor {}", i + 1, f.factor + 1),
        );
        scope(ui, &f.scope);
    }
    ui.label("Complete captured text and exact values: Export report…");
}
pub(super) fn show(ui: &mut egui::Ui, r: &ComputedAnova) {
    ui.collapsing("Method", |ui| {
        ui.label(crate::export::report_anova::METHOD);
    });
    caption(ui,format_args!("Source rows: {}; retained physical rows / subjects: {}; frequency N: {}; observed measurements: {}",r.source_rows,r.fitted_rows,r.frequency,r.observed_measurements));
    caption(
        ui,
        format_args!(
            "Excluded: {} filtered; {} missing weights; {} zero weights; {} incomplete",
            r.diagnostics.filtered_rows,
            r.diagnostics.missing_weight_rows,
            r.diagnostics.zero_weight_rows,
            r.diagnostics.complete_case_rows
        ),
    );
    match &r.outcome {
        AnovaOutcome::Unavailable(f) => caption(ui, &f.reason),
        AnovaOutcome::Fitted(m) => {
            caption(
                ui,
                format_args!(
                    "Residual df: {}; total df: {}; balanced: {}",
                    m.residual_df, m.total_df, m.balanced
                ),
            );
            for (name, e) in [
                ("Model SS", &m.model_ss.value),
                ("Total SS", &m.total_ss.value),
                ("Error SS", &m.error_ss.value),
            ] {
                value(ui, name, e, "");
            }
            if let Some(e) = &m.subject_ss {
                value(ui, "Subject SS", &e.value, "");
            }
            for e in &m.effects {
                theme::section(ui, &theme::caption(EffectName(&r.grid.factors, &e.factors)));
                value(ui, "F", &e.test.f, "");
                value(ui, "p", &e.test.p, "");
                if let Some(f) = &e.failure {
                    caption(ui, &f.reason);
                }
                ui.collapsing(
                    theme::caption(EffectName(&r.grid.factors, &e.factors)),
                    |ui| {
                        for f in &e.factors {
                            caption(ui, &r.grid.factors[*f].name);
                        }
                        for (name, v) in [
                            ("SS", &e.ss.value),
                            ("MS", &e.ms),
                            ("Error SS", &e.error_ss.value),
                            ("Error MS", &e.error_ms),
                            ("df1", &e.test.df1),
                            ("df2", &e.test.df2),
                            ("F", &e.test.f),
                            ("p", &e.test.p),
                            ("Eta squared", &e.eta_squared),
                            ("Partial eta squared", &e.partial_eta_squared),
                        ] {
                            value(ui, name, v, "");
                        }
                        if let Some(f) = &e.failure {
                            caption(ui, &f.reason);
                        }
                        if let Some(g) = &e.gg {
                            ui.label("Greenhouse–Geisser corrected inference");
                            for (name, v) in [
                                ("Epsilon", &g.epsilon),
                                ("Corrected df1", &g.test.df1),
                                ("Corrected df2", &g.test.df2),
                                ("Corrected F", &g.test.f),
                                ("Corrected p", &g.test.p),
                            ] {
                                value(ui, name, v, "");
                            }
                            if let Some(f) = &g.failure {
                                caption(ui, &f.reason);
                            }
                        }
                    },
                );
            }
        }
    }
    ui.collapsing("Cell summaries and factor levels", |ui| {
        for (i, factor) in r.grid.factors.iter().enumerate() {
            caption(ui, format_args!("Factor {}: {}", i + 1, factor.name));
            for v in &factor.values {
                caption(ui, CategoryCaption(v));
            }
        }
        rows(ui, "anova-cells", r.grid.cells.len(), 2, |ui, i| {
            let cell = &r.grid.cells[i];
            caption(
                ui,
                format_args!(
                    "Cell {}: {} physical rows; {} frequency / subjects",
                    i + 1,
                    cell.physical_rows,
                    cell.frequency
                ),
            );
            value(ui, "Mean", &cell.mean, "");
        });
    });
    ui.label("First minus second; pointwise intervals; Holm adjustment within each fixed family.");
    for (i, f) in r.families.iter().enumerate() {
        ui.collapsing(format!("Comparison family {} · m={}", i + 1, f.m), |ui| {
            match &f.kind {
                AnovaFamilyKind::Planned => {
                    ui.label("Planned pairs");
                }
                AnovaFamilyKind::AllPairs(v) => {
                    caption(ui, format_args!("All pairs · factor {}", v.factor + 1));
                    scope(ui, &v.scope);
                }
            }
            if let Some(v) = &f.adjustment_failure {
                caption(ui, &v.reason);
            }
            rows(ui, ("anova-members", i), f.members.len(), 17, |ui, j| {
                let v = &f.members[j];
                caption(ui, format_args!("Member {}", v.ordinal + 1));
                pair(ui, &v.request, &r.grid.factors[v.request.factor].name);
                match &v.state {
                    AnovaComparisonState::Available => {}
                    AnovaComparisonState::Unavailable(f)
                    | AnovaComparisonState::AbsentRequestedLevel(f) => caption(ui, &f.reason),
                }
                for (name, e) in [
                    ("Difference", &v.estimate),
                    ("SE", &v.standard_error),
                    ("t", &v.t),
                    ("df", &v.df),
                    ("Raw p", &v.raw_p),
                    ("Holm p", &v.adjusted_p),
                    ("Lower pointwise bound", &v.interval[0]),
                    ("Upper pointwise bound", &v.interval[1]),
                ] {
                    value(ui, name, e, "");
                }
            });
        });
    }
    ui.label("Independent eta squared: effect SS / total SS; partial eta squared: effect SS / (effect SS + its error SS). Repeated eta squared is withheld. GG corrects df and p; SS, MS and effect sizes retain uncorrected definitions.");
    ui.collapsing("Captured settings", |ui| {
        settings(ui, &r.request, &r.columns)
    });
}
#[cfg(test)]
mod tests;

struct EffectName<'a>(&'a [AnovaFactorLevels], &'a [usize]);
impl std::fmt::Display for EffectName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, j) in self.1.iter().enumerate() {
            if i > 0 {
                f.write_str(" × ")?;
            }
            f.write_str(&self.0[*j].name)?;
        }
        Ok(())
    }
}
fn factor_name<'a>(r: &'a AnovaRequest, columns: &'a [RunColumnInfo], ordinal: usize) -> &'a str {
    match &r.design {
        AnovaDesign::OneWay { factor, .. } => name(columns, *factor),
        AnovaDesign::Factorial { factors, .. } => factors
            .get(ordinal)
            .map_or("Unavailable factor", |id| name(columns, *id)),
        AnovaDesign::Repeated { factors, .. } => factors
            .get(ordinal)
            .map_or("Unavailable factor", |v| v.name.as_str()),
    }
}
