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
                    "Confirm Excel numeric precision for each marked variable.",
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
            if let Some(bounds) = sheet.dimensions {
                ui.small(format!(
                    "Sheet extent: rows {}–{} · columns {}–{}",
                    bounds[0] + 1,
                    bounds[2] + 1,
                    bounds[1] + 1,
                    bounds[3] + 1
                ));
            }
        }
        ui.label("Choose the rectangle to import.");
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
        ui.checkbox(
            &mut selection.header,
            "First selected row contains variable names",
        );
        state.excel.selection = Some(selection);
        let response = ui.button("Preview selected range");
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
                    if preview.precision_acknowledgement_required(index, choice.kind) {
                        let numeric_source = preview.precision_acknowledgement_required(index, ColumnKind::Text);
                        let label = if numeric_source { "Accept Excel numeric rounding, including text imports" } else { "Allow numeric precision loss" };
                        let response = ui.checkbox(&mut choice.acknowledge_precision_loss, label);
                        super::super::procedure_forms::record(state, &format!("Excel numeric rounding variable {}", index + 1), &response);
                        response.on_hover_text(if numeric_source { "Numeric values may have lost original digits. Importing them as text cannot restore those digits." } else { "Keep this identifier as text to preserve its exact digits. Converting to a number may round those digits." });
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
    egui::CollapsingHeader::new("Sample rows")
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                egui::Grid::new("excel-sample")
                    .striped(true)
                    .show(ui, |ui| {
                        for row in preview.sample_rows() {
                            for value in row {
                                ui.label(super::super::theme::caption(value));
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
mod workflow_tests {
    use super::*;
    use crate::{
        jobs::{JobEvent, JobOutput},
        operation::{JobControl, JobToken},
    };
    use tessera_workbook::{
        metadata::{WorkbookInfo, Worksheet},
        protocol::{Cell as SourceCell, Value},
        supervisor::WorkbookSnapshot,
    };
    fn workbook() -> Arc<ExcelWorkbook> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/tessera-workbook/fixtures/tiny.xlsx");
        let snapshot =
            WorkbookSnapshot::prepare(&path, 0, &std::sync::atomic::AtomicBool::new(false))
                .unwrap();
        let metadata = WorkbookInfo {
            source_identity: snapshot.source_identity(),
            epoch_1904: false,
            sheets: vec![Worksheet {
                index: 0,
                name: "Data".into(),
                kind: 0,
                visibility: 0,
                dimensions: Some([0, 0, 1, 2]),
            }],
        };
        Arc::new(
            ExcelWorkbook::from_snapshot(snapshot, metadata, &JobControl::default(), &[], 0)
                .unwrap(),
        )
    }
    fn ready_state() -> DesktopState {
        let mut state = DesktopState::new().unwrap();
        state.import_path = Some(PathBuf::from("original.xlsx"));
        let workbook = workbook();
        state.accept_excel_workbook(Arc::clone(&workbook)).unwrap();
        let selection = ExcelSelection {
            worksheet_index: 0,
            range: [0, 0, 0, 2],
            header: false,
        };
        let cells = vec![
            SourceCell {
                row: 0,
                column: 0,
                flags: 33,
                value: Value::Number(9007199254740992.),
            },
            SourceCell {
                row: 0,
                column: 1,
                flags: 3,
                value: Value::Missing,
            },
            SourceCell {
                row: 0,
                column: 2,
                flags: 16,
                value: Value::ExcelDate {
                    serial: 60.,
                    epoch_1904: false,
                    duration: false,
                },
            },
        ];
        let preview =
            ExcelPreview::from_cells(workbook, selection, cells, &JobControl::default(), &[], 0)
                .unwrap();
        state.accept_excel_preview(Arc::new(preview)).unwrap();
        state
    }
    fn approve(state: &mut DesktopState) {
        state.excel.decisions = ExcelDecisions {
            acknowledge_cached_formulas: true,
            allow_missing_formula_caches: true,
            allow_fictitious_dates_as_missing: true,
        };
        let mut choice = state.choices[0].clone();
        choice.acknowledge_precision_loss = true;
        state.edit_import_choice(0, choice).unwrap();
    }
    #[test]
    fn numeric_source_still_needs_precision_approval_when_imported_as_text() {
        let mut state = ready_state();
        state.excel.decisions = ExcelDecisions {
            acknowledge_cached_formulas: true,
            allow_missing_formula_caches: true,
            allow_fictitious_dates_as_missing: true,
        };
        let mut choice = state.choices[0].clone();
        choice.kind = ColumnKind::Text;
        state.edit_import_choice(0, choice).unwrap();
        assert!(state.excel_commit_operation().is_err());
        approve(&mut state);
        let JobOperation::CommitExcel { choices, .. } = state.excel_commit_operation().unwrap()
        else {
            panic!()
        };
        assert_eq!(choices[0].kind, ColumnKind::Text);
    }
    #[test]
    fn every_formula_and_date_decision_is_required_and_range_edits_need_new_preview() {
        let mut state = ready_state();
        approve(&mut state);
        assert!(state.excel_commit_operation().is_ok());
        for index in 0..3 {
            let original = state.excel.decisions;
            match index {
                0 => state.excel.decisions.acknowledge_cached_formulas = false,
                1 => state.excel.decisions.allow_missing_formula_caches = false,
                _ => state.excel.decisions.allow_fictitious_dates_as_missing = false,
            }
            assert!(state.excel_commit_operation().is_err());
            state.excel.decisions = original;
        }
        state.excel.selection.as_mut().unwrap().header = true;
        assert!(state.excel_commit_operation().is_err());
        state.excel.selection = Some(state.excel.preview.as_ref().unwrap().selection());
        assert!(state.excel_commit_operation().is_ok());
    }
    #[test]
    fn workbook_and_preview_share_one_source_charge_and_clones_refuse_before_allocation() {
        let mut state = ready_state();
        let preview = state.excel.preview.as_ref().unwrap();
        let workbook = state.excel.workbook.as_ref().unwrap();
        let controls = 4 * std::mem::size_of::<usize>();
        assert_eq!(
            state.excel.owned_bytes(),
            workbook.owned_bytes() + preview.preview_only_bytes() + controls
        );
        approve(&mut state);
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
        let (result, allocated) =
            crate::data::measure_test_allocations(|| state.excel_commit_operation());
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(allocated, 0);
    }
    #[test]
    fn stale_failed_and_cancelled_events_keep_project_and_workbook_retryable() {
        let mut state = ready_state();
        approve(&mut state);
        let project = Arc::clone(state.project());
        let workbook = Arc::clone(state.excel.workbook.as_ref().unwrap());
        let preview = Arc::clone(state.excel.preview.as_ref().unwrap());
        state.active = true;
        state.operation_name = "Excel import";
        let token = state.token();
        let stale = JobToken {
            generation: token.generation + 1,
            ..token
        };
        state
            .handle_event(JobEvent::Completed {
                token: stale,
                cancelled: false,
                result: Err(AppError::invalid("Excel import", "stale error")),
            })
            .unwrap();
        assert!(state.active);
        assert!(state.error().is_none());
        state
            .handle_event(JobEvent::Completed {
                token,
                cancelled: true,
                result: Ok(JobOutput::Imported(
                    crate::data::Dataset::new(vec![]).unwrap(),
                )),
            })
            .unwrap();
        assert!(!state.active);
        assert!(Arc::ptr_eq(&project, state.project()));
        assert!(Arc::ptr_eq(
            &workbook,
            state.excel.workbook.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(&preview, state.excel.preview.as_ref().unwrap()));
        assert!(state.excel_commit_operation().is_ok());
        state
            .handle_event(JobEvent::Completed {
                token: state.token(),
                cancelled: false,
                result: Err(AppError::invalid("Excel import", "retryable failure")),
            })
            .unwrap_err();
        assert!(Arc::ptr_eq(&project, state.project()));
        assert!(state.excel_commit_operation().is_ok());
    }
    #[test]
    fn successful_import_adoption_releases_source_only_after_validation() {
        let mut state = ready_state();
        approve(&mut state);
        let source = Arc::downgrade(state.excel.workbook.as_ref().unwrap());
        state.operation_name = "Excel import";
        state.import_path = Some(PathBuf::from("failed-later-selection.xlsx"));
        state
            .handle_event(JobEvent::Completed {
                token: state.token(),
                cancelled: false,
                result: Ok(JobOutput::Imported(
                    crate::data::Dataset::new(vec![]).unwrap(),
                )),
            })
            .unwrap();
        assert_eq!(state.project().title(), "original");
        assert!(state.project().provenance()[0].contains("original.xlsx"));
        assert!(source.upgrade().is_none());
        assert!(state.excel.preview.is_none());
    }
    #[test]
    fn rendered_excel_preview_exposes_numeric_text_approval_and_active_cancel() {
        let mut state = ready_state();
        let mut choice = state.choices[0].clone();
        choice.kind = ColumnKind::Text;
        state.edit_import_choice(0, choice).unwrap();
        let ctx = egui::Context::default();
        super::super::super::configure_context(&ctx);
        let mut jobs = JobRunner::default();
        let frame = |state: &mut DesktopState, jobs: &mut JobRunner| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900., 800.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    render(ui, state, jobs);
                    commit_button(ui, state, jobs);
                },
            );
            output.textures_delta.clear();
        };
        frame(&mut state, &mut jobs);
        assert!(state.control_rect("Preview selected range").is_some());
        assert!(state
            .control_rect("Excel numeric rounding variable 1")
            .is_some());
        state.active = true;
        frame(&mut state, &mut jobs);
        assert!(state.control_rect("Cancel import").is_some());
    }
    #[test]
    fn excel_request_respects_property_and_unsaved_decisions() {
        use super::super::{MetadataEditor, PendingAction, PropertyChoice, UiAction};
        use crate::data::{Column, ColumnId, ColumnMetadata, Dataset};
        let project = crate::project::Project::new(
            "Prior",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Value", vec![Some(1.)]).unwrap()
            ])
            .unwrap(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(project).unwrap();
        state.metadata_editor = Some(MetadataEditor {
            column: ColumnId(1),
            metadata: ColumnMetadata::new("Edited"),
            kind: ColumnKind::Number,
        });
        let state = state
            .apply_action(UiAction::Request(PendingAction::ImportExcel(
                PathBuf::from("new.xlsx"),
            )))
            .unwrap();
        assert_eq!(state.dialog, Some(Dialog::PropertyChanges));
        assert!(state.pending().is_none());
        let state = state
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Discard))
            .unwrap();
        assert_eq!(state.dialog, Some(Dialog::Unsaved));
        assert!(matches!(
            state.pending(),
            Some(PendingAction::ImportExcel(_))
        ));
        assert_eq!(
            state
                .project()
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .name(),
            "Value"
        );
    }
    #[test]
    fn numeric_text_conversion_exposes_approval_only_when_number_is_selected() {
        let mut state = DesktopState::new().unwrap();
        let workbook = workbook();
        state.accept_excel_workbook(Arc::clone(&workbook)).unwrap();
        let preview = ExcelPreview::from_cells(
            workbook,
            ExcelSelection {
                worksheet_index: 0,
                range: [0; 4],
                header: false,
            },
            vec![SourceCell {
                row: 0,
                column: 0,
                flags: 0,
                value: Value::Text("9007199254740993".into()),
            }],
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        state.accept_excel_preview(Arc::new(preview)).unwrap();
        let ctx = egui::Context::default();
        super::super::super::configure_context(&ctx);
        let mut jobs = JobRunner::default();
        let frame = |state: &mut DesktopState, jobs: &mut JobRunner| {
            state.controls.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900., 800.),
                    )),
                    ..Default::default()
                },
                |ui| render(ui, state, jobs),
            );
            output.textures_delta.clear();
        };
        assert!(state.excel_commit_operation().is_ok());
        frame(&mut state, &mut jobs);
        assert!(state
            .control_rect("Excel numeric rounding variable 1")
            .is_none());
        let mut choice = state.choices[0].clone();
        choice.kind = ColumnKind::Number;
        state.edit_import_choice(0, choice).unwrap();
        assert!(state.excel_commit_operation().is_err());
        frame(&mut state, &mut jobs);
        assert!(state
            .control_rect("Excel numeric rounding variable 1")
            .is_some());
        let mut choice = state.choices[0].clone();
        choice.acknowledge_precision_loss = true;
        state.edit_import_choice(0, choice).unwrap();
        assert!(state.excel_commit_operation().is_ok());
        let mut choice = state.choices[0].clone();
        choice.kind = ColumnKind::Text;
        choice.acknowledge_precision_loss = false;
        state.edit_import_choice(0, choice).unwrap();
        assert!(state.excel_commit_operation().is_ok());
    }
}
