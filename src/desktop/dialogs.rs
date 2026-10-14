use super::state::PropertyChoice;
use super::{
    analyses, button, command, import_review, start, state::TEXT_LIMIT, theme, toggle, variables,
    DesktopState, Dialog, PendingAction, UiAction, UnsavedChoice,
};
use crate::{
    data::Budget,
    import::Delimiter,
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
    } else {
        state.cancel_survey_import();
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
            .set_title("Save project — choose a new filename for a different file")
            .add_filter("Tessera project", &["tessera"])
            .set_file_name(
                state
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "project.tessera".into()),
            )
            .save_file()
    });
    if let Some(path) = path {
        let expected = state.expected_save_stamp(&path);
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
        .set_title("Export data — choose a new filename; existing files are preserved")
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
                acknowledge_formula_risk: false,
            },
        );
    }
}
pub fn export_report(state: &mut DesktopState, jobs: &mut JobRunner) {
    state.modal_file_dialog = true;
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Export all results — choose a new filename")
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
pub fn export_survey_report(state: &mut DesktopState, jobs: &mut JobRunner) {
    let runs = match state.survey_export_runs() {
        Ok(runs) => runs,
        Err(error) => {
            state.fail(error);
            return;
        }
    };
    state.modal_file_dialog = true;
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Export survey report — choose a new filename")
        .add_filter("HTML report", &["html"])
        .set_file_name("survey-report.html")
        .save_file()
    {
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
pub fn export_selected_report(state: &mut DesktopState, jobs: &mut JobRunner) {
    let runs = match state.report_export_runs() {
        Ok(runs) => runs,
        Err(error) => {
            state.fail(error);
            return;
        }
    };
    state.modal_file_dialog = true;
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Export selected results — choose a new filename")
        .add_filter("HTML report", &["html"])
        .set_file_name("selected-results.html")
        .save_file()
    {
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
pub fn save_survey_recipe(state: &mut DesktopState, jobs: &mut JobRunner) {
    state.modal_file_dialog = true;
    let path = rfd::FileDialog::new()
        .set_title("Save survey recipe — choose a new filename")
        .add_filter("Survey recipe", &["tessera-recipe"])
        .set_file_name("survey.tessera-recipe")
        .save_file();
    state.modal_file_dialog = false;
    if let Some(path) = path {
        if let Err(error) = state.save_survey_recipe(jobs, path) {
            state.fail(error);
        }
    }
}
pub fn load_survey_recipe(state: &mut DesktopState, jobs: &mut JobRunner) {
    state.modal_file_dialog = true;
    let path = rfd::FileDialog::new()
        .add_filter("Survey recipe", &["tessera-recipe"])
        .pick_file();
    state.modal_file_dialog = false;
    if let Some(path) = path {
        if let Err(error) = state.load_survey_recipe(jobs, path) {
            state.fail(error);
        }
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
                Dialog::UserGuide => "User guide",
                Dialog::ErrorDetails => "Error details",
                Dialog::ExportDataConfirmation => "Spreadsheet formula warning",
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
                Dialog::Import => import_review::show(ui, state, jobs),
                Dialog::ExcelImport => super::state::excel::render(ui, state, jobs),
                Dialog::Prepare => super::preparation::show(ui, state, jobs),
                Dialog::About => super::about::show(ui, state),
                Dialog::UserGuide => super::help::show(ui),
                Dialog::ErrorDetails => {
                    if let Some(error) = state.error_details() {
                        ui.label(error);
                        ui.small("Details are limited to 32 KiB. A final … means the message was shortened.");
                        if ui.small_button("Copy error details").clicked() {
                            ui.ctx().copy_text(error.to_owned());
                        }
                    } else {
                        ui.label("No error details are available.");
                    }
                }
                Dialog::ExportDataConfirmation => {
                    ui.label("Some text or variable names begin with characters that spreadsheet applications may treat as formulas. Opening this CSV or TSV file directly could run those formulas.");
                    ui.label("The export preserves your original text exactly. In Excel or LibreOffice, use the text-import dialog and set every column to Text before loading the file. Do not enable external links or content.");
                    ui.label("Export the original data with this risk acknowledged?");
                }
                Dialog::FileOutcome => {
                    ui.strong(state.file_receipt_label());
                    if let Some(outcome) = state.shown_file_receipt() {
                        ui.label(match outcome.publication {
                            crate::files::Publication::Committed => "The file operation completed. Review any warning below.",
                            crate::files::Publication::Conflict => "The destination changed or already exists. Review the retained files, then choose a new filename.",
                            crate::files::Publication::Uncertain => "The final file state could not be verified. Check the destination and recovery files.",
                        });
                        path_detail(ui, "Destination", &outcome.destination);
                        if outcome.committed_stamp.is_some() {
                            ui.label("The prepared file was written to the destination.");
                        }
                        if let Some(warning) = &outcome.warning {
                            ui.label(state.bounded_file_warning(warning));
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
                    if let Some(path) = &state.path {
                        path_detail(ui, "Current project file", path);
                    } else {
                        ui.label("The current project has no saved file yet.");
                    }
                    ui.small("Exports and saves to a different file need a new filename. Existing files are preserved, even if the system file chooser offers Replace.");
                }
            });
        }
        if let Some(error) = state.error().filter(|_| dialog != Dialog::ErrorDetails) {
            ui.colored_label(ui.visuals().error_fg_color, theme::caption(error));
            ui.collapsing("Error details", |ui| {
                egui::ScrollArea::vertical().max_height(100.0).show(ui, |ui| { ui.label(error); });
                if ui.small_button("Copy error details").clicked() { ui.ctx().copy_text(error.to_owned()); }
            });
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
                Dialog::ExportDataConfirmation => {
                    if command(ui, state, "Acknowledge risk and export").clicked() {
                        if let Err(error) = state.confirm_data_export(jobs) { state.fail(error); }
                    }
                    if toggle(ui, state, "Cancel export").clicked() { close = true; }
                }
                Dialog::Import => {
                    if state.active {
                        let response = ui.add_enabled(!state.cancelled, egui::Button::new("Cancel import"));
                        super::procedure_forms::record(state, "Cancel import", &response);
                        if response.clicked() { state.request_cancel(jobs); }
                    }
                    let response = ui.add_enabled(!state.active, theme::primary(ui, "Import data"));
                    super::procedure_forms::record(state, "Import data", &response);
                    if response.clicked() { commit_import(state, jobs); }
                }
                Dialog::ExcelImport => super::state::excel::commit_button(ui, state, jobs),
                Dialog::FileOutcome if state.file_receipt_can_save_as() && ui.add_enabled(!state.active, egui::Button::new("Save as…")).clicked() => { save(state,jobs,true); }
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
            if dialog == Dialog::ExportDataConfirmation {
                state.export_confirmation = None;
            }
            if matches!(dialog, Dialog::Import | Dialog::ExcelImport) {
                state.dismiss_import(jobs);
            } else {
                state.dialog = None;
            }
        }
    }
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

pub(super) fn path_detail(ui: &mut egui::Ui, label: &str, path: &std::path::Path) {
    ui.label(theme::caption(format_args!("{label}: {}", path.display())));
    // OS paths are capped by the file domain; retain a guarded exact-copy action.
    if path.as_os_str().len() <= 64 * 1024 && ui.small_button(format!("Copy {label}")).clicked() {
        ui.ctx().copy_text(path.to_string_lossy().into_owned());
    }
}
