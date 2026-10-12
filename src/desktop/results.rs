use super::{analyses::statistic_label, button, dialogs, start, theme, DesktopState, UiAction};
use crate::{
    analysis::{MissingPolicy, PlotData, RowPredicate, Statistic},
    jobs::{JobOperation, JobRunner},
    project::AnalysisRun,
};
use eframe::egui::{self, Align2, Pos2, Sense};
use std::sync::Arc;
fn history_label(run: &AnalysisRun, revision: crate::data::RevisionId) -> String {
    let variables = run
        .analysis
        .columns()
        .iter()
        .take(3)
        .map(|c| theme::caption_limit(&c.name, 24))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Run {} · {} · {}{} · {}",
        run.id.0,
        crate::export::report_analyses::method(&run.analysis),
        variables,
        if run.analysis.columns().len() > 3 {
            ", …"
        } else {
            ""
        },
        if run.analysis.revision() == revision {
            "Current data"
        } else {
            "Data changed"
        }
    )
}
pub fn history(ui: &mut egui::Ui, state: &mut DesktopState) {
    let project = Arc::clone(state.project());
    let revision = project.dataset().revision();
    let history = egui::ComboBox::from_id_salt("run-history")
        .width(ui.available_width().min(300.))
        .truncate()
        .selected_text(
            state
                .selected_run()
                .map_or("History".into(), |r| history_label(r, revision)),
        )
        .show_ui(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.history_search)
                    .hint_text("Search saved results")
                    .char_limit(super::state::TEXT_LIMIT),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search saved results")
            });
            super::procedure_forms::record(state, "Search saved results", &response);
            let query = match theme::NameQuery::new(&state.history_search) {
                Ok(query) => query,
                Err(error) => {
                    state.fail(error);
                    return;
                }
            };
            let matches = || {
                project.runs().iter().rev().filter(|run| {
                    query.matches(&history_label(run, revision))
                        || run
                            .analysis
                            .columns()
                            .iter()
                            .any(|c| query.matches(&c.name))
                })
            };
            let count = matches().count();
            if count == 0 {
                ui.label("No matching saved results");
            }
            egui::ScrollArea::vertical()
                .max_height(280.)
                .show_rows(ui, 28., count, |ui, range| {
                    for run in matches().skip(range.start).take(range.len()) {
                        let label = history_label(run, revision);
                        if theme::selectable_label(
                            ui,
                            state.selected_run().is_some_and(|r| r.id == run.id),
                            &label,
                        )
                        .on_hover_text(&label)
                        .clicked()
                        {
                            state.act(UiAction::SelectRun(run.id));
                            ui.close();
                        }
                    }
                });
        });
    history.response.widget_info(|| egui::WidgetInfo {
        current_text_value: Some(
            state
                .selected_run()
                .map_or("No saved result selected".into(), |run| {
                    history_label(run, revision)
                }),
        ),
        ..egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "History")
    });
    super::procedure_forms::record(state, "History", &history.response);
    if let Some(run) = state.selected_run() {
        history.response.on_hover_text(history_label(run, revision));
    }
}
fn saved_list(ui: &mut egui::Ui, state: &mut DesktopState) {
    let project = Arc::clone(state.project());
    let revision = project.dataset().revision();
    ui.strong("Saved results");
    ui.small(format!("{} saved analyses", project.runs().len()));
    let response = ui.add(
        egui::TextEdit::singleline(&mut state.history_search)
            .hint_text("Search saved results")
            .char_limit(super::state::TEXT_LIMIT),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search saved results")
    });
    super::procedure_forms::record(state, "Search saved results", &response);
    let query = match theme::NameQuery::new(&state.history_search) {
        Ok(query) => query,
        Err(error) => {
            state.fail(error);
            return;
        }
    };
    let matches = || {
        project.runs().iter().rev().filter(|run| {
            query.matches(&history_label(run, revision))
                || run
                    .analysis
                    .columns()
                    .iter()
                    .any(|column| query.matches(&column.name))
        })
    };
    let count = matches().count();
    if count == 0 {
        ui.label("No matching saved results");
    }
    egui::ScrollArea::vertical()
        .id_salt("saved-results-list")
        .show_rows(ui, 40., count, |ui, range| {
            for run in matches().skip(range.start).take(range.len()) {
                let method = format!(
                    "{} · Run {}",
                    crate::export::report_analyses::method(&run.analysis),
                    run.id.0
                );
                let variables = run
                    .analysis
                    .columns()
                    .iter()
                    .take(3)
                    .map(|column| theme::caption_limit(&column.name, 24))
                    .collect::<Vec<_>>()
                    .join(", ");
                let subtitle = format!(
                    "{variables} · {}",
                    if run.analysis.revision() == revision {
                        "Current data"
                    } else {
                        "Data changed"
                    }
                );
                let response = ui
                    .push_id(run.id.0, |ui| {
                        theme::list_row(
                            ui,
                            state
                                .selected_run()
                                .is_some_and(|selected| selected.id == run.id),
                            &method,
                            Some(&subtitle),
                        )
                    })
                    .inner;
                if response
                    .on_hover_text(history_label(run, revision))
                    .clicked()
                {
                    state.act(UiAction::SelectRun(run.id));
                }
            }
        });
}

pub fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    if state.project().runs().is_empty() {
        ui.heading("Results");
        output(ui, state);
        return;
    }
    let wide = ui.available_width() >= 760. && !state.project().runs().is_empty();
    if !wide {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Results");
            history(ui, state);
        });
    }
    ui.horizontal_wrapped(|ui| {
        if wide {
            ui.heading("Results");
        }
        if button(ui, state, "New analysis").clicked() {
            state.act(UiAction::Navigate(super::Workspace::Analyses));
        }
        let edit = ui.add_enabled(
            !state.active && state.selected_run().is_some(),
            egui::Button::new("Edit selected analysis"),
        );
        super::procedure_forms::record(state, "Edit selected analysis", &edit);
        super::procedure_forms::record(state, "Use this run in Analyses", &edit);
        if edit.clicked() {
            state.load_selected_request();
            if state.error().is_none() {
                state.act(UiAction::Navigate(super::Workspace::Analyses));
            }
        }
        if ui
            .add_enabled(
                !state.active && !state.project().runs().is_empty(),
                theme::primary(ui, "Export all results…"),
            )
            .on_hover_text("Export every saved result, including its captured settings and complete tables, as an HTML report")
            .clicked()
        {
            dialogs::export_report(state, jobs);
        }
        if ui
            .add_enabled(
                !state.active && state.selected_run().is_some(),
                egui::Button::new("Rerun"),
            )
            .on_hover_text("Run the saved settings against the current data")
            .clicked()
        {
            if let Some(run) = state.selected_run() {
                match state.checked_run_request(run.id) {
                    Ok(request) => start(
                        state,
                        jobs,
                        JobOperation::Analyze {
                            project: Arc::clone(state.project()),
                            request,
                        },
                    ),
                    Err(error) => state.fail(error),
                }
            }
        }
    });
    ui.separator();
    if state.selected_run().is_some() {
        super::figure_editor::toolbar(ui, state, jobs);
    }
    if wide {
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(230., height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| saved_list(ui, state),
            );
            ui.separator();
            ui.vertical(|ui| {
                output(ui, state);
            });
        });
    } else {
        output(ui, state);
    }
}
pub fn output(ui: &mut egui::Ui, state: &mut DesktopState) {
    egui::Frame::new()
        .fill(ui.visuals().panel_fill)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            super::output_style::apply(ui);
            output_contents(ui, state);
        });
}
fn output_contents(ui: &mut egui::Ui, state: &mut DesktopState) {
    let Some(run) = state.selected_run().cloned() else {
        ui.add_space(24.);
        ui.heading("Your results will appear here");
        ui.label("Open Analyse, choose what you want to find out, select your variables, and run the analysis. Each result keeps its original data and settings.");
        if button(ui, state, "Go to Analyses").clicked() {
            state.act(UiAction::Navigate(super::Workspace::Analyses));
        }
        return;
    };
    let crate::analysis::CompletedAnalysis::Describe(a) = &run.analysis else {
        super::result_tables::show(ui, state, &run);
        return;
    };
    egui::ScrollArea::both().id_salt("analysis-output").show(ui,|ui|{
        ui.heading(format!("Saved result: Descriptive statistics · run {}",run.id.0));
        super::result_tables::revision_status(ui, a.revision.0, state.selected_run_is_stale());
        super::experience::guidance(ui, state.analysis_mode(), run.analysis.request());
        ui.add_space(10.0);
        egui::Grid::new("descriptive-table").striped(true).min_col_width(84.0).show(ui,|ui|{
            ui.strong("Variable");ui.strong("Valid N");ui.strong("Missing N");
            if a.request.missing_policy==MissingPolicy::CompleteCase{ui.strong("Excluded N");}
            for statistic in &a.request.statistics{ui.strong(statistic_label(*statistic));}ui.end_row();
            for row in &a.rows{
                ui.label(theme::caption(name(&run,row.column)));ui.monospace(row.valid_n.to_string());ui.monospace(row.missing_n.to_string());
                if a.request.missing_policy==MissingPolicy::CompleteCase{ui.monospace(row.complete_case_excluded_n.to_string());}
                for statistic in &a.request.statistics{
                    if let Some(estimate)=row.estimates.get(statistic){
                        let exact=estimate.value.map_or_else(||theme::caption(estimate.reason.as_deref().unwrap_or("Unavailable")),|value|value.to_string());
                        let response=ui.monospace(estimate.value.map_or_else(||"Unavailable".into(),theme::compact_number));
                        response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Label,true,theme::caption(format_args!("{}: {}",statistic_label(*statistic),exact))));
                        response.on_hover_text(exact);
                    }
                }ui.end_row();
            }
        });
        for row in &a.rows{for (statistic,estimate) in &row.estimates{if let Some(reason)=&estimate.reason{ui.label(theme::caption(format_args!("{} · {}: {}",name(&run,row.column),statistic_label(*statistic),reason)));}}}
        ui.add_space(12.0);
        for plot in &a.plots{
            ui.strong(theme::caption(format_args!("{} · distribution",name(&run,plot.column))));
            ui.small(theme::caption(&plot.label));
            if plot.reduced{ui.small(format!("{} of {} eligible rows displayed",plot.points.len(),plot.source_rows));}
            let mean=a.rows.iter().find(|r|r.column==plot.column).and_then(|r|r.estimates.get(&Statistic::Mean)).and_then(|e|e.value);
            draw_plot(ui,plot,mean);
            ui.push_id(plot.column.0,|ui|{ui.collapsing("Observation table",|ui|{
                ui.horizontal(|ui|{ui.label("Source row");ui.label("Value");ui.label("Frequency");});
                egui::ScrollArea::vertical().id_salt("observations").max_height(220.0).show_rows(ui,24.0,plot.points.len(),|ui,range|{
                    for point in &plot.points[range]{ui.horizontal(|ui|{ui.monospace((point.row+1).to_string());ui.monospace(point.value.to_string());ui.monospace(point.frequency.to_string());});}
                });
            });});ui.add_space(12.0);
        }
        let provenance=ui.collapsing("Run settings and provenance",|ui|{
            ui.label(match a.request.missing_policy{MissingPolicy::PerVariable=>"Missing values: per variable",MissingPolicy::CompleteCase=>"Missing values: complete cases"});
            if let Some(weight)=a.request.frequency_weight{ui.label(theme::caption(format_args!("Frequency weight: {} · repeated observations",name(&run,weight))));}else{ui.label("Unweighted observations");}
            for predicate in &a.request.filter.predicates{ui.label(match predicate{
                RowPredicate::Number{column,comparison,value}=>theme::caption(format_args!("{} {} {}",name(&run,*column),match comparison{crate::analysis::NumericComparison::Equal=>"=",crate::analysis::NumericComparison::NotEqual=>"!=",crate::analysis::NumericComparison::Less=>"<",crate::analysis::NumericComparison::LessOrEqual=>"<=",crate::analysis::NumericComparison::Greater=>">",crate::analysis::NumericComparison::GreaterOrEqual=>">="},value)),
                RowPredicate::Text{column,value,equal}=>theme::caption(format_args!("{} {} {:?}",name(&run,*column),if *equal{"="}else{"!="},value)),
                RowPredicate::Missing{column,missing}=>theme::caption(format_args!("{} is {}",name(&run,*column),if *missing{"missing"}else{"present"})),
            });}
            let d=&a.diagnostics;
            ui.label(format!("Excluded rows: {} filtered; {} missing weights; {} zero weights; {} incomplete cases.",d.filtered_rows,d.missing_weight_rows,d.zero_weight_rows,d.complete_case_rows));
            for column in &a.columns{ui.label(theme::caption(format_args!("{} · {}",column.name,column.description)));
                ui.label(theme::caption(format_args!("Missing definitions: {:?}",column.missing_rules)));
                for label in &column.value_labels{ui.label(theme::caption(format_args!("{:?}: {}",label.value,label.label)));}
            }
            for source in state.project().provenance(){ui.label(theme::caption(source));}
        });
        state.controls.insert("Run settings and provenance".into(),provenance.header_response.rect);
    });
}
fn name(run: &AnalysisRun, id: crate::data::ColumnId) -> &str {
    run.analysis
        .columns()
        .iter()
        .find(|c| c.id == id)
        .map_or("Unavailable variable", |c| c.name.as_str())
}
fn draw_plot(ui: &mut egui::Ui, plot: &PlotData, mean: Option<f64>) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().clamp(240.0, 800.0), 176.0),
        Sense::hover(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, theme::caption(&plot.label))
    });
    if plot.points.is_empty() {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No eligible observations",
            egui::TextStyle::Body.resolve(ui.style()),
            theme::text(ui),
        );
        return;
    }
    let stacks = match crate::charts::stacked_rows(
        plot.points.iter().map(|point| point.value),
        &crate::operation::JobControl::default(),
    ) {
        Ok(stacks) => stacks,
        Err(_) => {
            ui.label("Drawing unavailable · exact values remain in the observation table");
            return;
        }
    };
    let largest_stack = stacks.iter().map(|point| point[1]).fold(1.0, f64::max);
    let min = plot
        .points
        .iter()
        .map(|p| p.value)
        .fold(mean.unwrap_or(f64::INFINITY), f64::min);
    let max = plot
        .points
        .iter()
        .map(|p| p.value)
        .fold(mean.unwrap_or(f64::NEG_INFINITY), f64::max);
    let scale = min.abs().max(max.abs()).max(1.0);
    let lo = min / scale;
    let hi = max / scale;
    let x = |value: f64| {
        rect.left()
            + 30.0
            + if hi == lo {
                0.5
            } else {
                ((value / scale - lo) / (hi - lo)) as f32
            } * (rect.width() - 60.0)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0., theme::surface(ui));
    let base = rect.bottom() - 26.0;
    painter.line_segment(
        [
            Pos2::new(rect.left() + 30.0, base),
            Pos2::new(rect.right() - 30.0, base),
        ],
        egui::Stroke::new(1.0, theme::line(ui)),
    );
    painter.text(
        Pos2::new(x(min), base + 10.0),
        Align2::CENTER_CENTER,
        min.to_string(),
        egui::TextStyle::Small.resolve(ui.style()),
        theme::text(ui),
    );
    if min != max {
        painter.text(
            Pos2::new(x(max), base + 10.0),
            Align2::CENTER_CENTER,
            max.to_string(),
            egui::TextStyle::Small.resolve(ui.style()),
            theme::text(ui),
        );
    }
    painter.text(
        Pos2::new(rect.left() + 30.0, rect.top() + 10.0),
        Align2::LEFT_CENTER,
        format!(
            "Retained source rows · largest stack {}",
            largest_stack as usize
        ),
        egui::TextStyle::Small.resolve(ui.style()),
        theme::text(ui),
    );
    let spacing = ((base - rect.top() - 32.0) / largest_stack as f32).min(9.0);
    let radius = (spacing * 0.4).clamp(1.0, 3.5);
    for point in &stacks {
        let position = Pos2::new(x(point[0]), base - point[1] as f32 * spacing);
        painter.circle_filled(position, radius, theme::accent(ui));
        painter.circle_stroke(position, radius, egui::Stroke::new(0.5, theme::surface(ui)));
    }
    if let Some(mean) = mean {
        let marker = [
            Pos2::new(x(mean), rect.top() + 35.0),
            Pos2::new(x(mean), base),
        ];
        painter.line_segment(marker, egui::Stroke::new(4.0, theme::text(ui)));
        painter.line_segment(marker, egui::Stroke::new(2.0, theme::GOLD));
        painter.text(
            Pos2::new(x(mean), rect.top() + 25.0),
            Align2::CENTER_CENTER,
            format!("Mean {}", theme::compact_number(mean)),
            egui::TextStyle::Small.resolve(ui.style()),
            theme::text(ui),
        );
    }
    ui.small("Each dot represents one retained source row; stack height counts rows, not frequency weights.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::*,
        data::{Column, ColumnId, Dataset, RevisionId},
        operation::JobControl,
        project::Project,
    };

    #[test]
    fn descriptive_dots_stack_equal_values_and_count_rows_not_weights() {
        let plot = PlotData {
            column: ColumnId(1),
            points: [1., 1., 2.]
                .into_iter()
                .enumerate()
                .map(|(row, value)| PlotPoint {
                    row,
                    value,
                    frequency: 99,
                })
                .collect(),
            source_rows: 3,
            reduced: false,
            label: "Retained observations".into(),
        };
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw_plot(ui, &plot, None));
        output.textures_delta.clear();
        let dots: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) if circle.fill != egui::Color32::TRANSPARENT => {
                    Some(circle.center)
                }
                _ => None,
            })
            .collect();
        assert_eq!(dots.len(), 3);
        assert_eq!(dots[0].x, dots[1].x);
        assert!(dots[1].y < dots[0].y);
        assert_eq!(dots[0].y, dots[2].y);
        assert!(dots[2].x > dots[0].x);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text().contains("largest stack 2"))));
    }

    #[test]
    fn new_family_history_uses_bounded_captured_unicode_names_and_stale_revision() {
        let long_name = format!("Sgôr ŵ {} final-name", "Ŷ".repeat(20_000));
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), &long_name, [1., 3., 4., 7.].map(Some).to_vec()).unwrap(),
            Column::numeric(
                ColumnId(2),
                "Second item",
                [2., 4., 3., 8.].map(Some).to_vec(),
            )
            .unwrap(),
            Column::text(
                ColumnId(3),
                "Captured factor",
                ["A", "A", "B", "B"].map(|s| Some(s.into())).to_vec(),
            )
            .unwrap(),
        ])
        .unwrap();
        let mut project = Project::new("History", data).unwrap();
        let requests = [
            AnalysisRequest::Regression(RegressionRequest {
                outcome: ColumnId(1),
                predictors: vec![RegressionPredictor {
                    column: ColumnId(2),
                    role: PredictorRole::Numeric,
                }],
                confidence: ConfidenceLevel::P95,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            AnalysisRequest::Anova(AnovaRequest {
                design: AnovaDesign::OneWay {
                    outcome: ColumnId(1),
                    factor: ColumnId(3),
                },
                confidence: ConfidenceLevel::P95,
                filter: RowFilter::default(),
                frequency_weight: None,
                planned_pairs: vec![],
                all_pairs: vec![],
            }),
            AnalysisRequest::Reliability(ReliabilityRequest {
                items: vec![ColumnId(1), ColumnId(2)],
                reversals: vec![],
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
        ];
        for request in &requests {
            let result = analyze_with_budget(
                project.dataset(),
                request,
                &JobControl::default(),
                &[],
                1024 * 1024,
            )
            .unwrap();
            project = project.with_run(result).unwrap();
        }
        for (run, method) in
            project
                .runs()
                .iter()
                .zip(["Linear regression", "ANOVA", "Reliability"])
        {
            let label = history_label(run, RevisionId(2));
            assert!(label.contains(method), "{label}");
            assert!(label.contains("Sgôr ŵ"), "{label}");
            assert!(label.ends_with("Data changed"));
            assert!(
                label.len() < 256,
                "history accessibility text stays bounded"
            );
            assert_eq!(run.analysis.columns()[0].name, long_name);
            assert!(theme::NameQuery::new("final-name")
                .unwrap()
                .matches(&run.analysis.columns()[0].name));
        }
    }
}
