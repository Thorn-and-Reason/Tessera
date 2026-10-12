use super::state::PropertyChoice;
use super::{
    analyses, button, command, start,
    state::{MAX_DRAFT_BYTES, TEXT_LIMIT},
    theme, toggle, variables, DesktopState, Dialog, PendingAction, UiAction, UnsavedChoice,
};
use crate::{
    data::{Budget, ColumnKind, MissingRules},
    import::{Delimiter, ImportWarning},
    jobs::{JobOperation, JobRunner},
};
use eframe::egui;
use std::sync::Arc;
pub fn open(state: &mut DesktopState) {
    state.modal_file_dialog = true;
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Tessera project", &["tessera"])
        .pick_file()
    {
        state.act(UiAction::Request(PendingAction::Open(path)));
    }
}
pub fn import(state: &mut DesktopState) {
    state.modal_file_dialog = true;
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Data", &["xlsx", "csv", "tsv"])
        .pick_file()
    {
        if super::state::excel::is_excel_path(&path) {
            state.act(UiAction::Request(PendingAction::ImportExcel(path)));
            return;
        }
        let delimiter = if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tsv"))
        {
            Delimiter::Tab
        } else {
            Delimiter::Comma
        };
        state.act(UiAction::Request(PendingAction::Import(path, delimiter)));
    }
}
pub fn save(state: &mut DesktopState, jobs: &mut JobRunner, save_as: bool) {
    if !state.prepare_save(save_as) {
        return;
    }
    let selected = if !save_as { state.path.clone() } else { None };
    let path = selected.or_else(|| {
        state.modal_file_dialog = true;
        rfd::FileDialog::new()
            .add_filter("Tessera project", &["tessera"])
            .set_file_name("project.tessera")
            .save_file()
    });
    if let Some(path) = path {
        let expected = if !save_as && state.path.as_ref() == Some(&path) {
            state.stamp.clone()
        } else {
            None
        };
        start(
            state,
            jobs,
            JobOperation::Save {
                project: Arc::clone(state.project()),
                path,
                expected,
            },
        );
    } else if state.pending().is_some() {
        state.dialog = Some(Dialog::Unsaved);
    }
}
pub fn export_data(state: &mut DesktopState, jobs: &mut JobRunner, delimiter: Delimiter) {
    state.modal_file_dialog = true;
    let extension = if delimiter == Delimiter::Comma {
        "csv"
    } else {
        "tsv"
    };
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Data", &[extension])
        .set_file_name(format!("data.{extension}"))
        .save_file()
    {
        start(
            state,
            jobs,
            JobOperation::ExportData {
                project: Arc::clone(state.project()),
                path,
                delimiter,
            },
        );
    }
}
pub fn export_report(state: &mut DesktopState, jobs: &mut JobRunner) {
    state.modal_file_dialog = true;
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("HTML report", &["html"])
        .set_file_name("report.html")
        .save_file()
    {
        // All retained runs are exported, preserving history rather than replacing it.
        let bytes = state.project().owned_bytes().and_then(|owned| {
            Budget::check_datasets(
                &[state.project().dataset()],
                owned
                    + state.extra()
                    + state.project().runs().len() * std::mem::size_of::<crate::operation::RunId>(),
            )
        });
        if let Err(error) = bytes {
            state.fail(error);
            return;
        }
        let runs = state.project().runs().iter().map(|r| r.id).collect();
        start(
            state,
            jobs,
            JobOperation::ExportReport {
                project: Arc::clone(state.project()),
                path,
                runs,
            },
        );
    }
}
pub fn show(ctx: &egui::Context, state: &mut DesktopState, jobs: &mut JobRunner) {
    let Some(dialog) = state.dialog else {
        return;
    };
    if matches!(dialog, Dialog::Unsaved | Dialog::PropertyChanges) {
        // This decision has no child popups; background menus must yield to it.
        egui::Popup::close_all(ctx);
    }
    let mut close = false;
    let response = egui::Modal::new(egui::Id::new("workspace-dialog")).show(ctx, |ui| {
        ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 560.0));
        ui.set_max_height((ctx.content_rect().height() - 64.0).max(160.0));
        // Title band: identity on the left, one named dismiss control on the right.
        ui.horizontal(|ui| {
            ui.heading(match dialog {
                Dialog::Properties => "Variable properties", Dialog::Settings => "Analysis settings",
                Dialog::AddVariable => "Add variable", Dialog::Unsaved => "Save changes?",
                Dialog::PropertyChanges => "Apply property changes?", Dialog::Import => "Import preview",
                Dialog::FileOutcome => "File details", Dialog::ExcelImport => "Excel import",
                Dialog::Prepare => "Prepare data",
                Dialog::About => "About Tessera",
            });
            if !matches!(dialog, Dialog::Unsaved | Dialog::PropertyChanges) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let response = ui.add(theme::quiet(ui, "×"));
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close dialog"));
                    state.controls.insert("Close dialog".into(), response.rect);
                    state.control_ids.insert("Close dialog".into(), response.id);
                    if response.on_hover_text("Close dialog · Escape").clicked() { close = true; }
                });
            }
        });
        ui.separator();
        if dialog == Dialog::Properties {
            variables::properties(ui, state, jobs);
        } else if dialog == Dialog::Settings {
            analyses::settings(ui, state, jobs);
        } else {
        egui::ScrollArea::vertical()
            .id_salt("dialog-body")
            .max_height((ctx.content_rect().height() - 220.0).max(100.0))
            .show(ui, |ui| match dialog {
                Dialog::Properties => variables::properties(ui, state, jobs),
                Dialog::Settings => analyses::settings(ui, state, jobs),
                Dialog::AddVariable => {
                    let label=ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut state.new_name).id_salt("new-variable-name").char_limit(TEXT_LIMIT)).labelled_by(label.id);
                    variables::kind_picker(ui, "new-kind", &mut state.new_kind);
                }
                Dialog::Unsaved => {
                    ui.label(theme::caption(state.project().title()));
                    ui.label("Save your project before continuing, or discard its changes.");
                }
                Dialog::PropertyChanges => {
                    ui.label("This variable has unapplied changes. Apply them before continuing, discard them, or stay to keep editing.");
                }
                Dialog::Import => import_preview(ui, state, jobs),
                Dialog::ExcelImport => super::state::excel::render(ui, state, jobs),
                Dialog::Prepare => super::preparation::show(ui, state, jobs),
                Dialog::About => super::about::show(ui, state),
                Dialog::FileOutcome => {
                    if let Some(outcome) = &state.file_outcome {
                        ui.label(match outcome.publication {
                            crate::files::Publication::Committed => "File saved",
                            crate::files::Publication::Conflict => "The destination changed. Your recovery files are preserved.",
                            crate::files::Publication::Uncertain => "The final file state could not be verified. Check the destination and recovery files.",
                        });
                        path_detail(ui, "Destination", &outcome.destination);
                        if outcome.committed_stamp.is_some() {
                            ui.label("The prepared file was written to the destination.");
                        }
                        if let Some(warning) = &outcome.warning {
                            ui.label(theme::caption(warning));
                        }
                        if let Some(recovery) = &outcome.recovery {
                            path_detail(ui, "Recovery", &recovery.directory);
                            for (label, path) in [
                                ("Saved revision", &recovery.frozen_revision),
                                ("Displaced file", &recovery.displaced),
                                ("Prepared file", &recovery.prepared),
                            ] {
                                if let Some(path) = path {
                                    path_detail(ui, label, path);
                                }
                            }
                        }
                    }
                }
            });
        }
        if let Some(error) = state.error() {
            ui.colored_label(ui.visuals().error_fg_color, theme::caption(error));
        }
        ui.separator();
        // Action band: one right-aligned cluster, primary action rightmost.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match dialog {
                Dialog::AddVariable => {
                    if ui.add_enabled(!state.active, theme::primary(ui, "Add variable")).clicked() {
                        state.act(UiAction::AddColumn { name: state.new_name.clone(), kind: state.new_kind });
                        if state.error().is_none() && state.dialog == Some(Dialog::AddVariable) { state.new_name.clear(); close=true; }
                    }
                }
                Dialog::Unsaved => {
                    if command(ui, state, "Save changes").clicked() { state.act(UiAction::ResolveUnsaved(UnsavedChoice::Save)); }
                    if button(ui, state, "Discard").clicked() { state.act(UiAction::ResolveUnsaved(UnsavedChoice::Discard)); }
                    if toggle(ui, state, "Cancel").clicked() { state.act(UiAction::ResolveUnsaved(UnsavedChoice::Cancel)); }
                }
                Dialog::PropertyChanges => {
                    if command(ui, state, "Apply").clicked() { state.act(UiAction::ResolveProperties(PropertyChoice::Apply)); }
                    if button(ui, state, "Discard").clicked() { state.act(UiAction::ResolveProperties(PropertyChoice::Discard)); }
                    if toggle(ui, state, "Stay").clicked() { state.act(UiAction::ResolveProperties(PropertyChoice::Stay)); }
                }
                Dialog::Import => {
                    let response = ui.add_enabled(!state.active, theme::primary(ui, "Import data"));
                    super::procedure_forms::record(state, "Import data", &response);
                    if response.clicked() { commit_import(state, jobs); }
                }
                Dialog::ExcelImport => super::state::excel::commit_button(ui, state, jobs),
                Dialog::FileOutcome if ui.add_enabled(!state.active, egui::Button::new("Save as…")).clicked() => { save(state,jobs,true); }
                _ => {}
            });
        });
    });
    if close || response.should_close() {
        if dialog == Dialog::PropertyChanges {
            state.act(UiAction::ResolveProperties(PropertyChoice::Stay));
        } else if dialog == Dialog::Unsaved {
            state.act(UiAction::ResolveUnsaved(UnsavedChoice::Cancel));
        } else {
            state.dialog = None;
        }
        if let Some(id) = state.return_focus.take() {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }
}
fn import_preview(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let Some(preview) = state.preview.as_ref().cloned() else {
        return;
    };
    ui.label(format!(
        "{} rows · {} variables",
        preview.row_count(),
        preview.columns().len()
    ));
    let previous = state.import_delimiter;
    ui.add_enabled_ui(!state.active, |ui| {
        egui::ComboBox::new("delimiter", "Delimiter")
            .selected_text(if previous == Delimiter::Comma {
                "Comma"
            } else {
                "Tab"
            })
            .show_ui(ui, |ui| {
                theme::selectable_value(ui, &mut state.import_delimiter, Delimiter::Comma, "Comma");
                theme::selectable_value(ui, &mut state.import_delimiter, Delimiter::Tab, "Tab");
            });
    });
    if previous != state.import_delimiter {
        if let Some(path) = state.import_path.clone() {
            start(
                state,
                jobs,
                JobOperation::PreviewPath {
                    path,
                    delimiter: state.import_delimiter,
                },
            );
        }
        return;
    }
    ui.add_enabled_ui(!state.active,|ui|{
        // Only visible preview choices are rendered; source samples are already bounded.
        egui::ScrollArea::vertical().id_salt("import-columns").max_height(220.0).show_rows(ui,100.0,state.choices.len(),|ui,range|{
            for index in range{ui.push_id(index,|ui|{
                let original=&state.choices[index];
                if original.name.len()>MAX_DRAFT_BYTES{ui.label("This header exceeds the editor's text limit.");return;}
                let mut choice=original.clone();
                let old_kind=choice.kind;
                ui.horizontal(|ui|{let name_label=ui.label(format!("Variable {} name",index+1));ui.add(egui::TextEdit::singleline(&mut choice.name).char_limit(TEXT_LIMIT).desired_width(170.0)).labelled_by(name_label.id);variables::kind_picker(ui,"kind",&mut choice.kind);});
                if old_kind!=choice.kind {choice.missing_rules=MissingRules::None;}
                if preview.columns()[index].warnings.contains(&ImportWarning::PrecisionRisk){ui.checkbox(&mut choice.acknowledge_precision_loss,"Allow numeric precision loss").on_hover_text("Keep this identifier as text to preserve its exact digits. Converting to a number may round those digits.");}
                if choice.kind==ColumnKind::Text{let mut blank_missing=matches!(&choice.missing_rules,MissingRules::Text(v) if v.iter().any(String::is_empty));if ui.checkbox(&mut blank_missing,"Empty text is missing").changed(){choice.missing_rules=if blank_missing{MissingRules::Text(vec![String::new()])}else{MissingRules::None};}}
                if preview.columns()[index].warnings.iter().any(|w|matches!(w,ImportWarning::InvalidHeader|ImportWarning::DuplicateHeader)){ui.label("Enter a unique, nonblank name");}
                let original=&state.choices[index];
                if choice.name!=original.name||choice.kind!=original.kind||choice.missing_rules!=original.missing_rules||choice.acknowledge_precision_loss!=original.acknowledge_precision_loss {
                    if let Err(error)=state.edit_import_choice(index,choice){state.fail(error);}
                }
                ui.separator();
            });}
        });
    });
    egui::CollapsingHeader::new("Sample rows")
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                egui::Grid::new("import-sample")
                    .striped(true)
                    .show(ui, |ui| {
                        for row in preview.sample_rows() {
                            for value in row {
                                ui.label(theme::caption(value));
                            }
                            ui.end_row();
                        }
                    });
            });
            if preview.samples_truncated() {
                ui.small("Long sample values are shortened.");
            }
        });
}
fn commit_import(state: &mut DesktopState, jobs: &mut JobRunner) {
    let Some(preview) = state.preview.as_ref().cloned() else {
        return;
    };
    {
        let extra = state.project().owned_bytes().and_then(|bytes| {
            crate::import::choice_bytes(&state.choices)
                .map(|choices| bytes + state.extra() + choices)
        });
        let check =
            extra.and_then(|bytes| Budget::check_datasets(&[state.project().dataset()], bytes));
        match check {
            Ok(()) => start(
                state,
                jobs,
                JobOperation::CommitImport {
                    preview,
                    choices: state.choices.as_ref().clone(),
                },
            ),
            Err(error) => state.fail(error),
        }
    }
}

fn path_detail(ui: &mut egui::Ui, label: &str, path: &std::path::Path) {
    ui.label(theme::caption(format_args!("{label}: {}", path.display())));
    // OS paths are capped by the file domain; retain a guarded exact-copy action.
    if path.as_os_str().len() <= 64 * 1024 && ui.small_button(format!("Copy {label}")).clicked() {
        ui.ctx().copy_text(path.to_string_lossy().into_owned());
    }
}
