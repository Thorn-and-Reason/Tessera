//! Retained workbook selection and explicit import decisions.
use std::path::Path;

pub(crate) fn is_excel_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_xlsx_paths_use_the_excel_import_flow() {
        assert!(is_excel_path(Path::new("book.xlsx")));
        assert!(is_excel_path(Path::new("book.XLSX")));
        for path in ["book.xls", "book.xlsm", "book.csv", "book.tsv", "xlsx"] {
            assert!(!is_excel_path(Path::new(path)));
        }
    }
}

use super::{DesktopState, Dialog, TEXT_LIMIT};
use crate::{
    data::{ColumnKind, MissingRules},
    error::{AppError, AppResult},
    import::{
        choice_bytes,
        excel::{ExcelDecisions, ExcelPreview, ExcelSelection, ExcelWorkbook},
        ImportWarning,
    },
    jobs::{JobOperation, JobRunner},
};
use eframe::egui;
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone, Default)]
pub(crate) struct ExcelImportState {
    pub workbook: Option<Arc<ExcelWorkbook>>,
    pub preview: Option<Arc<ExcelPreview>>,
    pub selection: Option<ExcelSelection>,
    pub decisions: ExcelDecisions,
    pub path: Option<PathBuf>,
}
impl ExcelImportState {
    pub fn owned_bytes(&self) -> usize {
        let workbook = self
            .workbook
            .as_ref()
            .map_or(0, |w| w.owned_bytes() + 2 * std::mem::size_of::<usize>());
        let preview = self.preview.as_ref().map_or(0, |p| {
            p.preview_only_bytes()
                + 2 * std::mem::size_of::<usize>()
                + if self
                    .workbook
                    .as_ref()
                    .is_some_and(|w| Arc::ptr_eq(w, p.workbook()))
                {
                    0
                } else {
                    p.workbook().owned_bytes()
                }
        });
        workbook.saturating_add(preview)
    }
    fn current_preview(&self) -> Option<&Arc<ExcelPreview>> {
        self.preview.as_ref().filter(|preview| {
            self.selection
                .is_some_and(|s| same_selection(s, preview.selection()))
                && self
                    .workbook
                    .as_ref()
                    .is_some_and(|w| Arc::ptr_eq(w, preview.workbook()))
        })
    }
}
fn same_selection(a: ExcelSelection, b: ExcelSelection) -> bool {
    a.worksheet_index == b.worksheet_index && a.range == b.range && a.header == b.header
}
fn default_selection(workbook: &ExcelWorkbook, worksheet_index: u32) -> Option<ExcelSelection> {
    let sheet = workbook
        .metadata()
        .sheets
        .iter()
        .find(|s| s.index == worksheet_index && s.kind == 0)?;
    let mut range = sheet.dimensions.unwrap_or([0, 0, 0, 0]);
    let cells = u64::from(range[2] - range[0] + 1) * u64::from(range[3] - range[1] + 1);
    if cells > crate::data::MAX_CELLS as u64 {
        range[2] = range[2].min(range[0].saturating_add(19));
        range[3] = range[3].min(range[1].saturating_add(19));
    }
    Some(ExcelSelection {
        worksheet_index,
        range,
        header: true,
    })
}
impl DesktopState {
    pub(crate) fn accept_excel_workbook(&mut self, workbook: Arc<ExcelWorkbook>) -> AppResult<()> {
        self.check_ui_copy(
            workbook
                .owned_bytes()
                .saturating_add(self.import_path.as_ref().map_or(0, PathBuf::capacity))
                .saturating_add(1024),
        )?;
        let selection = workbook
            .metadata()
            .sheets
            .iter()
            .find(|s| s.kind == 0 && s.visibility == 0)
            .or_else(|| workbook.metadata().sheets.iter().find(|s| s.kind == 0))
            .and_then(|s| default_selection(&workbook, s.index));
        self.excel = ExcelImportState {
            workbook: Some(workbook),
            selection,
            path: self.import_path.clone(),
            ..Default::default()
        };
        self.preview = None;
        self.choices = Arc::new(vec![]);
        self.dialog = Some(Dialog::ExcelImport);
        Ok(())
    }
    pub(crate) fn accept_excel_preview(&mut self, preview: Arc<ExcelPreview>) -> AppResult<()> {
        if !self
            .excel
            .workbook
            .as_ref()
            .is_some_and(|w| Arc::ptr_eq(w, preview.workbook()))
        {
            return Err(AppError::Conflict(
                "The selected workbook changed. Preview it again.".into(),
            ));
        }
        let shared = self.excel.workbook.as_ref().map_or(0, |w| w.owned_bytes());
        let extra = self
            .project()
            .owned_bytes()?
            .checked_add(self.extra())
            .and_then(|v| v.checked_sub(shared))
            .ok_or(AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?;
        let choices = preview.default_choices_with_budget(&[self.project().dataset()], extra)?;
        self.excel.selection = Some(preview.selection());
        self.excel.preview = Some(preview);
        self.excel.decisions = Default::default();
        self.choices = Arc::new(choices);
        self.dialog = Some(Dialog::ExcelImport);
        Ok(())
    }
    fn excel_commit_operation(&self) -> AppResult<JobOperation> {
        let preview = self.excel.current_preview().ok_or_else(|| {
            AppError::invalid(
                "Excel import",
                "Preview the selected range before importing.",
            )
        })?;
        let warnings = preview.warnings();
        let decisions = self.excel.decisions;
        if warnings.cached_formula_cells > 0 && !decisions.acknowledge_cached_formulas {
            return Err(AppError::invalid(
                "Excel import",
                "Confirm that cached formula values may be out of date.",
            ));
        }
        if warnings.missing_formula_cache_cells > 0 && !decisions.allow_missing_formula_caches {
            return Err(AppError::invalid(
                "Excel import",
                "Choose whether formulas without cached values become missing.",
            ));
        }
        if warnings.fictitious_date_cells > 0 && !decisions.allow_fictitious_dates_as_missing {
            return Err(AppError::invalid(
                "Excel import",
                "Choose whether Excel's invalid 1900 leap day becomes missing.",
            ));
        }
        for (index, choice) in self.choices.iter().enumerate() {
            if preview.precision_acknowledgement_required(index, choice.kind)
                && !choice.acknowledge_precision_loss
            {
                return Err(AppError::invalid(
                    "Excel import",
                    if preview.precision_acknowledgement_required(index, ColumnKind::Text) {
                        "Accept Excel stored numeric rounding above the sample rows."
                    } else {
                        "Open Column types and names and confirm precision loss for the marked text-to-number conversion."
                    },
                ));
            }
        }
        self.check_ui_copy(choice_bytes(&self.choices)?)?;
        Ok(JobOperation::CommitExcel {
            preview: Arc::clone(preview),
            choices: self.choices.as_ref().clone(),
            decisions,
        })
    }
}

pub(crate) fn render(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    let Some(workbook) = state.excel.workbook.as_ref().cloned() else {
        return;
    };
    let Some(mut selection) = state.excel.selection else {
        ui.label("This workbook has no importable worksheets.");
        return;
    };
    ui.add_enabled_ui(!state.active, |ui| {
        let selected = workbook
            .metadata()
            .sheets
            .iter()
            .find(|s| s.index == selection.worksheet_index);
        egui::ComboBox::new("excel-sheet", "Worksheet")
            .selected_text(selected.map_or("Choose worksheet", |s| s.name.as_str()))
            .show_ui(ui, |ui| {
                for sheet in &workbook.metadata().sheets {
                    let suffix = if sheet.kind != 0 {
                        " (not a worksheet)"
                    } else if sheet.visibility != 0 {
                        " (hidden)"
                    } else {
                        ""
                    };
                    let label = format!("{}{}", sheet.name, suffix);
                    if ui
                        .add_enabled(
                            sheet.kind == 0,
                            egui::Button::selectable(
                                selection.worksheet_index == sheet.index,
                                label,
                            ),
                        )
                        .clicked()
                    {
                        if let Some(next) = default_selection(&workbook, sheet.index) {
                            selection = next;
                        }
                    }
                }
            });
        if let Some(sheet) = workbook
            .metadata()
            .sheets
            .iter()
            .find(|s| s.index == selection.worksheet_index)
        {
            if sheet.visibility != 0 {
                ui.small("Hidden worksheet");
            }

        }
        let whole_sheet = workbook.metadata().sheets.iter().find(|sheet| sheet.index == selection.worksheet_index).and_then(|sheet| sheet.dimensions) == Some(selection.range);
        ui.small(format!("{}: rows {}–{} · columns {}–{}", if whole_sheet { "Entire worksheet selected" } else { "Selected range" }, selection.range[0] + 1, selection.range[2] + 1, selection.range[1] + 1, selection.range[3] + 1));
        let limited_default = workbook.metadata().sheets.iter().find(|sheet| sheet.index == selection.worksheet_index).and_then(|sheet| sheet.dimensions).is_some_and(|bounds| {
            default_selection(&workbook, selection.worksheet_index).is_some_and(|default| default.range != bounds)
        });
        if limited_default {
            ui.colored_label(ui.visuals().warn_fg_color, "This worksheet exceeds the import limit. Only a small range is selected; review the custom range before importing.");
        }
        egui::CollapsingHeader::new("Custom row and column range")
            .default_open(limited_default)
            .show(ui, |ui| {
        let mut range = selection.range.map(|v| v + 1);
        egui::Grid::new("excel-rectangle").show(ui, |ui| {
            for (index, label, ceiling) in [
                (0, "First row", 1_048_576),
                (2, "Last row", 1_048_576),
                (1, "First column", 16_384),
                (3, "Last column", 16_384),
            ] {
                let label = ui.label(label);
                ui.add(egui::DragValue::new(&mut range[index]).range(1..=ceiling))
                    .labelled_by(label.id);
                ui.end_row();
            }
        });
        selection.range = range.map(|v| v.saturating_sub(1));
            });
        ui.checkbox(
            &mut selection.header,
            "First selected row contains variable names",
        );
        state.excel.selection = Some(selection);
        let response = ui.button("Review worksheet");
        super::super::procedure_forms::record(state, "Review worksheet", &response);
        super::super::procedure_forms::record(state, "Preview selected range", &response);
        if response.clicked() {
            super::super::start(
                state,
                jobs,
                JobOperation::PreviewExcel {
                    workbook: Arc::clone(&workbook),
                    selection,
                },
            );
        }
    });
    let Some(preview) = state.excel.current_preview().cloned() else {
        return;
    };
    ui.separator();
    ui.label(format!(
        "{} rows · {} variables",
        preview.row_count(),
        preview.columns().len()
    ));
    ui.add_enabled_ui(!state.active, |ui| {
        let warnings = preview.warnings();
        if warnings.cached_formula_cells > 0 {
            ui.checkbox(&mut state.excel.decisions.acknowledge_cached_formulas, "Use cached formula values; they may be out of date");
        }
        if warnings.missing_formula_cache_cells > 0 {
            ui.checkbox(&mut state.excel.decisions.allow_missing_formula_caches, "Import formulas without cached values as missing");
        }
        if warnings.fictitious_date_cells > 0 {
            ui.checkbox(&mut state.excel.decisions.allow_fictitious_dates_as_missing, "Import Excel's invalid 1900 leap day as missing");
        }
        let stored_numeric = || (0..preview.columns().len()).filter(|&index| preview.precision_acknowledgement_required(index, ColumnKind::Text));
        if stored_numeric().next().is_some() {
            ui.label(format!("{} variables contain stored Excel numbers that may be rounded. Tessera can import the stored values, but cannot recover original digits, even when importing as text.", stored_numeric().count()));
            let mut accepted = stored_numeric().all(|index| state.choices[index].acknowledge_precision_loss);
            let response = ui.checkbox(&mut accepted, "Accept Excel stored numeric rounding");
            super::super::procedure_forms::record(state, "Accept Excel stored numeric rounding", &response);
            for index in stored_numeric() {
                super::super::procedure_forms::record(state, &format!("Excel numeric rounding variable {}", index + 1), &response);
            }
            if response.changed() {
                for index in stored_numeric() {
                    let mut choice = state.choices[index].clone();
                    choice.acknowledge_precision_loss = accepted;
                    if let Err(error) = state.edit_import_choice(index, choice) { state.fail(error); break; }
                }
            }
        }
        let conversion_warning = state.choices.iter().enumerate().any(|(index, choice)| preview.precision_acknowledgement_required(index, choice.kind) && !preview.precision_acknowledgement_required(index, ColumnKind::Text));
        egui::CollapsingHeader::new("Column types and names")
            .open(conversion_warning.then_some(true))
            .show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("excel-columns").max_height(220.).show_rows(ui, 95., state.choices.len(), |ui, range| {
            for index in range {
                ui.push_id(index, |ui| {
                    let original = &state.choices[index];
                    let mut choice = original.clone();
                    let old_kind = choice.kind;
                    ui.horizontal(|ui| {
                        let label = ui.label(format!("Variable {} name", index + 1));
                        ui.add(egui::TextEdit::singleline(&mut choice.name).char_limit(TEXT_LIMIT).desired_width(170.)).labelled_by(label.id);
                        super::super::variables::kind_picker(ui, "excel-kind", &mut choice.kind);
                    });
                    if old_kind != choice.kind { choice.missing_rules = MissingRules::None; }
                    if preview.precision_acknowledgement_required(index, choice.kind) && !preview.precision_acknowledgement_required(index, ColumnKind::Text) {
                        let response = ui.checkbox(&mut choice.acknowledge_precision_loss, "Allow text-to-number precision loss");
                        super::super::procedure_forms::record(state, &format!("Excel numeric rounding variable {}", index + 1), &response);
                        response.on_hover_text("Keep this identifier as text to preserve its exact digits. Converting to a number may round those digits.");
                    }
                    if choice.kind == ColumnKind::Text {
                        let mut missing = matches!(&choice.missing_rules, MissingRules::Text(v) if v.iter().any(String::is_empty));
                        if ui.checkbox(&mut missing, "Empty text is missing").changed() {
                            choice.missing_rules = if missing { MissingRules::Text(vec![String::new()]) } else { MissingRules::None };
                        }
                    }
                    if preview.columns()[index].warnings.iter().any(|warning| matches!(warning, ImportWarning::InvalidHeader | ImportWarning::DuplicateHeader)) {
                        ui.small("Enter a unique, nonblank variable name.");
                    }
                    let old = &state.choices[index];
                    if choice.name != old.name || choice.kind != old.kind || choice.missing_rules != old.missing_rules || choice.acknowledge_precision_loss != old.acknowledge_precision_loss {
                        if let Err(error) = state.edit_import_choice(index, choice) { state.fail(error); }
                    }
                });
            }
        });
        });
    });
    egui::CollapsingHeader::new("Sample rows")
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                egui::Grid::new("excel-sample")
                    .striped(true)
                    .show(ui, |ui| {
                        for choice in state.choices.iter() {
                            ui.add_sized(
                                [140.0, 22.0],
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "{} · {}",
                                        choice.name,
                                        super::super::variables::kind_label(choice.kind)
                                    ))
                                    .strong(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&choice.name);
                        }
                        ui.end_row();
                        for row in preview.sample_rows() {
                            for value in row {
                                ui.add_sized(
                                    [140.0, 22.0],
                                    egui::Label::new(super::super::theme::caption(value))
                                        .truncate(),
                                )
                                .on_hover_text(value);
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
pub(crate) fn commit_button(ui: &mut egui::Ui, state: &mut DesktopState, jobs: &mut JobRunner) {
    if state.active {
        let response = ui.add_enabled(!state.cancelled, egui::Button::new("Cancel import"));
        super::super::procedure_forms::record(state, "Cancel import", &response);
        if response.clicked() {
            state.cancel_survey_import();
            jobs.cancel();
            state.cancelled = true;
        }
    }
    let enabled = !state.active && state.excel.current_preview().is_some();
    let response = ui.add_enabled(enabled, egui::Button::new("Import Excel data"));
    super::super::procedure_forms::record(state, "Import Excel data", &response);
    if response.clicked() {
        match state.excel_commit_operation() {
            Ok(operation) => super::super::start(state, jobs, operation),
            Err(error) => state.fail(error),
        }
    }
}

#[cfg(test)]
#[path = "excel/tests.rs"]
mod workflow_tests;
