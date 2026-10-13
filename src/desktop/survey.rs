//! Advisory survey review; only visible variable metadata is read while drawing.
use super::{
    button, command, paged_rows, procedure_forms::record, DesktopState, UiAction, Workspace,
};
use crate::{
    data::ColumnKind,
    jobs::JobRunner,
    survey::{SurveyFinding, SurveyRole},
};
use eframe::egui;
use std::sync::Arc;

pub(super) fn entry(ui: &mut egui::Ui, state: &mut DesktopState) {
    ui.add_enabled_ui(!state.active, |ui| {
        if button(ui, state, "Instant survey report").clicked() {
            if let Err(error) = state.enter_survey() {
                state.fail(error);
            }
        }
    });
}
pub(super) fn show(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    egui::Panel::bottom("survey-review-footer").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(state.survey_ready(), |ui| {
                if command(ui, state, "Create report").clicked() {
                    if let Err(error) = state.submit_survey(jobs) {
                        state.fail(error);
                    }
                }
                if button(ui, state, "Save report recipe").clicked() {
                    super::dialogs::save_survey_recipe(state, jobs);
                }
            });
            ui.add_enabled_ui(!state.active, |ui| {
                if button(ui, state, "Load report recipe").clicked() {
                    super::dialogs::load_survey_recipe(state, jobs);
                }
                if state.survey_recipe().is_some()
                    && command(ui, state, "Apply recipe and create report").clicked()
                {
                    if let Err(error) = state.replay_survey_recipe(jobs) {
                        state.fail(error);
                    }
                }
            });
            if button(ui, state, "Manual analysis").clicked() {
                state.leave_survey_review();
                state.cancel_survey_import();
            }
            for (label, workspace) in [
                ("Prepare data", Workspace::Prepare),
                ("Variable definitions", Workspace::Variables),
            ] {
                if button(ui, state, label).clicked() {
                    state.leave_survey_review();
                    state.act(UiAction::Navigate(workspace));
                }
            }
        });
    });
    let Some(review) = state.survey_review().cloned() else {
        ui.heading("Instant survey report");
        ui.label("Import data, then review the suggested reporting roles.");
        return;
    };
    let project = Arc::clone(state.project());
    ui.heading("Instant survey report");
    recipe_summary(ui, state);
    ui.label("Suggestions need your review. Excluded variables remain in the data. Personal-data warnings are advisory and may miss sensitive values.");
    ui.label("Valid and missing counts follow current missing rules. Blank text may still count as valid; use Variable definitions to review those rules.");
    ui.label(format!(
        "Possible duplicate rows: {}. Rows are retained.",
        review.duplicate_rows
    ));
    let unresolved = state
        .survey_choices()
        .iter()
        .filter(|c| c.role == SurveyRole::Unresolved)
        .count();
    ui.weak(format!(
        "{unresolved} actions still need your choice. Include at least one variable."
    ));
    presentation(ui, state);
    cleaning(ui, state, jobs);
    let height = (ui.available_height() - 32.).max(50.);
    let controls = paged_rows::show_rows(
        ui,
        "survey-variable-review",
        "survey variables",
        146.,
        review.columns.len(),
        (height, 50.),
        |ui, range| {
            for row in range {
                let index = state.survey_order().get(row).copied().unwrap_or(row);
                let finding = &review.columns[index];
                let Ok(column) = project.dataset().column(finding.column) else {
                    continue;
                };
                ui.push_id(finding.column.0, |ui| {
                    if state.active {
                        ui.disable();
                    }
                    ui.set_min_height(146.);
                    ui.add(
                        egui::Label::new(egui::RichText::new(column.name()).strong()).truncate(),
                    );
                    ui.label(format!(
                        "{} valid · {} missing · Suggestion: {}",
                        finding.valid,
                        finding.missing,
                        finding.suggested_role.label()
                    ));
                    let role = state
                        .survey_choices()
                        .get(index)
                        .map_or(SurveyRole::Unresolved, |choice| choice.role);
                    let mut selected = role;
                    let response = egui::ComboBox::from_id_salt("role")
                        .selected_text(role.label())
                        .show_ui(ui, |ui| {
                            for choice in [
                                SurveyRole::Unresolved,
                                SurveyRole::Exclude,
                                SurveyRole::Categories,
                                SurveyRole::NumericMeasure,
                            ] {
                                if choice == SurveyRole::NumericMeasure
                                    && column.kind() != ColumnKind::Number
                                {
                                    continue;
                                }
                                let label = if choice == SurveyRole::Categories
                                    && finding.findings.contains(&SurveyFinding::MultipleAnswers)
                                {
                                    "Count responses (complete cells)"
                                } else {
                                    choice.label()
                                };
                                let response = ui.selectable_value(&mut selected, choice, label);
                                record(
                                    state,
                                    &format!("Survey role {}: {}", finding.column.0, label),
                                    &response,
                                );
                            }
                        })
                        .response;
                    let label = format!("Survey role {}", finding.column.0);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::ComboBox,
                            !state.active,
                            format!("Reporting role for {}", column.name()),
                        )
                    });
                    record(state, &label, &response);
                    if selected != role {
                        if let Err(error) = state.set_survey_role(finding.column, selected) {
                            state.fail(error);
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        for warning in &finding.findings {
                            let response = ui
                                .menu_button(warning.label(), |ui| {
                                    ui.set_max_width(360.);
                                    ui.label(warning.explanation());
                                })
                                .response;
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    !state.active,
                                    format!(
                                        "{} for {}: show explanation",
                                        warning.label(),
                                        column.name()
                                    ),
                                )
                            });
                            record(
                                state,
                                &format!(
                                    "Survey warning {}: {}",
                                    finding.column.0,
                                    warning.label()
                                ),
                                &response,
                            );
                        }
                    });
                    ui.separator();
                });
            }
        },
    );
    controls.record(state);
}

fn recipe_summary(ui: &mut egui::Ui, state: &mut DesktopState) {
    let Some(recipe) = state.survey_recipe().cloned() else {
        return;
    };
    ui.label(super::theme::caption(format_args!(
        "Loaded recipe: {}",
        recipe.name
    )));
    ui.label("Apply recipe uses these saved settings.");
    ui.weak("Create report uses the current review below.");
    ui.label(match recipe.presentation {
        crate::survey::SurveyPresentation::Report => "Saved report style: Report colours",
        crate::survey::SurveyPresentation::HighContrast => "Saved report style: High contrast",
    });
    let controls = paged_rows::show_rows(
        ui,
        "loaded-recipe-settings",
        "saved recipe variables",
        158.,
        recipe.columns.len(),
        (158., 158.),
        |ui, range| {
            for index in range {
                let column = &recipe.columns[index];
                ui.push_id(index, |ui| {
                    ui.set_min_height(158.);
                    ui.label(super::theme::caption(format_args!(
                        "{}: {}",
                        super::theme::caption_limit(&column.name, 96),
                        column.role.label()
                    )));
                    if let Some(cleaning) = &column.cleaning {
                        ui.label(if cleaning.trim_whitespace {
                            "Remove surrounding spaces before replacements"
                        } else {
                            "Keep surrounding spaces"
                        });
                        let controls = paged_rows::show_rows(
                            ui,
                            "saved-replacements",
                            "saved replacements",
                            20.,
                            cleaning.rules.len(),
                            (44., 20.),
                            |ui, range| {
                                for rule in &cleaning.rules[range] {
                                    ui.label(super::theme::caption(format_args!(
                                        "{} → {}",
                                        cell_caption(&rule.from),
                                        cell_caption(&rule.to)
                                    )));
                                }
                            },
                        );
                        controls.record(state);
                    } else {
                        ui.label("No response-label changes");
                    }
                });
            }
        },
    );
    controls.record(state);
    ui.separator();
}

fn presentation(ui: &mut egui::Ui, state: &mut DesktopState) {
    let mut selected = state.survey_presentation();
    ui.add_enabled_ui(!state.active, |ui| {
        egui::ComboBox::from_id_salt("survey-presentation")
            .selected_text(match selected {
                crate::survey::SurveyPresentation::Report => "Report colours",
                crate::survey::SurveyPresentation::HighContrast => "High contrast",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut selected,
                    crate::survey::SurveyPresentation::Report,
                    "Report colours",
                );
                ui.selectable_value(
                    &mut selected,
                    crate::survey::SurveyPresentation::HighContrast,
                    "High contrast",
                );
            });
    });
    if selected != state.survey_presentation() {
        if let Err(error) = state.set_survey_presentation(selected) {
            state.fail(error);
        }
    }
}
fn cleaning(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let header=egui::CollapsingHeader::new("Optional: prepare response labels").show(ui,|ui| {
        ui.label("Preview exact changes, then create a reviewed copy. Original responses remain available. Trimming happens before replacements.");
        let mut draft=match state.copy_survey_cleaning_draft(){Ok(draft)=>draft,Err(error)=>{state.fail(error);return;}};
        let original=draft.clone();
        let project=Arc::clone(state.project());
        ui.add_enabled_ui(!state.active,|ui| {
            egui::ComboBox::from_id_salt("survey-clean-variable").selected_text(draft.column.and_then(|id|project.dataset().column(id).ok()).map_or("Choose a text variable",|c|c.name())).show_ui(ui,|ui| {
                for column in project.dataset().columns().iter().filter(|c|c.kind()==ColumnKind::Text){ui.selectable_value(&mut draft.column,Some(column.id()),column.name());}
            });
            ui.checkbox(&mut draft.trim,"Remove surrounding spaces");
            if draft.column!=original.column {draft.rules=Vec::new();draft.replacement=false;draft.from=String::new();draft.to=String::new();}
            ui.checkbox(&mut draft.replacement,"Edit an exact replacement");
            if draft.replacement {
                ui.horizontal_wrapped(|ui|{ui.label("From");ui.add(egui::TextEdit::singleline(&mut draft.from).char_limit(1024));});
                ui.checkbox(&mut draft.missing,"Treat this response as missing");
                if !draft.missing{ui.horizontal_wrapped(|ui|{ui.label("To");ui.add(egui::TextEdit::singleline(&mut draft.to).char_limit(1024));});}
            }
        });
        if draft!=original {
            if let Err(error)=state.set_survey_cleaning_draft(draft){state.fail(error);}
        }else{drop(draft);}drop(original);
        ui.add_enabled_ui(!state.active&&state.survey_cleaning_draft().replacement,|ui|{if button(ui,state,"Add replacement").clicked(){if let Err(error)=state.stage_survey_replacement(){state.fail(error);}}});
        ui.weak("Add each replacement to the list. Preview includes listed replacements and any replacement still being edited.");
        egui::ScrollArea::vertical().id_salt("survey-cleaning-rules").max_height(100.).show(ui,|ui|{
            let rules=match state.copy_survey_cleaning_draft(){Ok(draft)=>draft.rules,Err(error)=>{state.fail(error);Vec::new()}};
            for (index,rule) in rules.iter().enumerate(){ui.horizontal_wrapped(|ui|{ui.label(super::theme::caption(format_args!("{} → {}",cell_caption(&rule.from),cell_caption(&rule.to))));if button(ui,state,&format!("Remove replacement {}",index+1)).clicked(){if let Err(error)=state.remove_survey_replacement(index){state.fail(error);}}});}
        });
        ui.add_enabled_ui(!state.active,|ui|{if button(ui,state,"Preview label changes").clicked(){if let Err(error)=state.preview_survey_cleaning_draft(jobs){state.fail(error);}}});
        if let Some(preview)=state.survey_cleaning_preview().cloned(){
            ui.label(format!("{} cells would change. {} examples shown.",preview.changed,preview.examples.len()));
            for example in &preview.examples{ui.label(super::theme::caption(format_args!("Row {}: {} → {}",example.row+1,cell_caption(&example.before),cell_caption(&example.after))));}
            ui.add_enabled_ui(!state.active,|ui|{if command(ui,state,"Apply reviewed copy").clicked(){if let Err(error)=state.apply_survey_cleaning(jobs){state.fail(error);}}});
        }
    });
    record(state, "Prepare response labels", &header.header_response);
}
fn cell_caption(cell: &crate::data::Cell) -> String {
    match cell {
        crate::data::Cell::Text(text) => super::theme::caption(text),
        crate::data::Cell::Missing => "Missing".into(),
        crate::data::Cell::Number(value) => super::theme::caption(value),
    }
}
