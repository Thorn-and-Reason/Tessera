use super::draft::AnalysisDraft;
mod accounting;
mod analysis_ux;
mod chart_roles;
#[path = "excel.rs"]
pub(crate) mod excel;
mod figure_edit;
mod file_feedback;
mod frequency_search;
mod group_lookup;
mod operations;
mod property_drafts;
mod survey;
mod survey_improvements;
mod undo;
mod worksheet;
use crate::{
    analysis::{DescribeRequest, GroupLookupResult, MissingPolicy, RowFilter, Statistic},
    data::{parse_number, Budget, Cell, ColumnId, ColumnKind, ColumnMetadata, Dataset, RevisionId},
    error::{AppError, AppResult},
    files::{FileStamp, SaveOutcome},
    import::{Delimiter, ImportColumnChoice, ImportPreview},
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner, JobTask, RetainedWork},
    operation::{JobToken, Progress, RunId},
    project::{AnalysisRun, Project},
};
use eframe::egui::{Id, Rect};
use group_lookup::LookupState;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

// Bounded editable controls may change during a worker job within this reservation.
// Dataset chunks, project history and import previews are accounted separately.
// Includes both live frame outputs, mode selector and bounded guided explanations.
pub const UI_WORK_BYTES: usize = 4 * 1024 * 1024 + 512 * 1024;
pub const MAX_DRAFT_BYTES: usize = 512 * 1024;
pub const TEXT_LIMIT: usize = 8192;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workspace {
    Data,
    Prepare,
    Variables,
    Analyses,
    Results,
}
impl Workspace {
    pub fn label(self) -> &'static str {
        match self {
            Self::Data => "Data",
            Self::Prepare => "Prepare",
            Self::Variables => "Variables",
            Self::Analyses => "Analyses",
            Self::Results => "Results",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    About,
    UserGuide,
    ErrorDetails,
    Prepare,
    Properties,
    Settings,
    AddVariable,
    Import,
    ExcelImport,
    Unsaved,
    FileOutcome,
    ExportDataConfirmation,
    PropertyChanges,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyChoice {
    Apply,
    Discard,
    Stay,
}
#[derive(Debug, Clone)]
pub enum PendingAction {
    New,
    Open(PathBuf),
    Import(PathBuf, Delimiter),
    ImportExcel(PathBuf),
    Close,
}
#[derive(Debug, Clone, Copy)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}
#[derive(Debug, Clone)]
pub enum UiAction {
    SetAnalysisMode(super::AnalysisMode),
    Undo,
    Redo,
    Navigate(Workspace),
    SelectColumn(ColumnId),
    SelectCell {
        column: ColumnId,
        row: usize,
    },
    MoveCell {
        columns: i32,
        rows: i32,
        edit: bool,
    },
    PasteCells(String),
    ResolveProperties(PropertyChoice),
    SelectRun(RunId),
    EditCell {
        column: ColumnId,
        row: usize,
        value: Cell,
    },
    BeginCell {
        column: ColumnId,
        row: usize,
    },
    CommitCell,
    CancelCell,
    AppendRow,
    AddColumn {
        name: String,
        kind: ColumnKind,
    },
    Metadata {
        column: ColumnId,
        metadata: ColumnMetadata,
    },
    SetDraft(AnalysisDraft),
    Request(PendingAction),
    ResolveUnsaved(UnsavedChoice),
}
#[derive(Debug, Clone)]
pub struct CellEditor {
    pub column: ColumnId,
    pub row: usize,
    pub text: String,
    pub missing: bool,
    pub focus: bool,
}
#[derive(Debug, Clone)]
pub struct MetadataEditor {
    pub column: ColumnId,
    pub metadata: ColumnMetadata,
    pub kind: ColumnKind,
}
#[derive(Debug, Clone)]
pub struct DesktopState {
    project: Arc<Project>,
    survey: survey::SurveyState,
    undo: Vec<Arc<crate::project::DataSnapshot>>,
    redo: Vec<Arc<crate::project::DataSnapshot>>,
    budget_memo: std::cell::RefCell<Option<accounting::ProjectBaseline>>,
    document: u64,
    generation: u64,
    saved: Option<(RevisionId, RunId)>,
    pub(crate) path: Option<PathBuf>,
    pub(crate) stamp: Option<FileStamp>,
    workspace: Workspace,
    analysis_mode: super::AnalysisMode,
    pub(crate) selected_column: Option<ColumnId>,
    selected_run: Option<RunId>,
    pub(crate) report_selection: Vec<RunId>,
    pub(crate) draft: AnalysisDraft,
    pub(crate) active_regression_reference: Option<ColumnId>,
    pub(crate) lookup: LookupState,
    chart_roles: Option<chart_roles::ChartRoles>,
    pub(crate) figure_editor: Option<super::figure_editor::FigureEditor>,
    pub(crate) figure_work_bytes: usize,
    pub(crate) selected_figure: Option<crate::figures::FigureKey>,
    pub(crate) frequency_search: frequency_search::FrequencyState,
    pub(crate) procedure_search: String,
    pub(crate) group_values: Option<Arc<GroupLookupResult>>,
    pub(crate) cell_editor: Option<CellEditor>,
    pub(crate) metadata_editor: Option<MetadataEditor>,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) return_focus: Option<Id>,
    pub(crate) properties_open: bool,
    pub(crate) preparation: super::preparation::PreparationForm,
    pub(crate) recovery_entries: Option<Vec<crate::recovery::RecoveryEntry>>,
    pub(crate) recovery_finished: Option<(JobToken, bool)>,
    pub(crate) variable_search: String,
    pub(crate) history_search: String,
    pub(crate) selected_cell: Option<(ColumnId, usize)>,
    pub(crate) column_widths: BTreeMap<ColumnId, f32>,
    pub(crate) pending_column: Option<ColumnId>,
    pub(crate) pending_selection: Option<(ColumnId, usize)>,
    pending_property_action: Option<PendingAction>,
    pending_new_variable: Option<(String, ColumnKind)>,
    pending_workspace: Option<Workspace>,
    pending_save_as: Option<bool>,
    pending: Option<PendingAction>,
    pub(crate) pending_ready: bool,
    cancel_for_pending: bool,
    save_requested: bool,
    pub(crate) active: bool,
    pub(super) active_recipe_bytes: usize,
    pub(crate) operation_name: &'static str,
    pub(crate) progress: Option<Progress>,
    pub(crate) cancelled: bool,
    abandoned_import: Option<JobToken>,
    pub(crate) modal_file_dialog: bool,
    pub(crate) shutdown: bool,
    error: Option<String>,
    pub(crate) status: String,
    pub(crate) preview: Option<Arc<ImportPreview>>,
    pub(crate) excel: excel::ExcelImportState,
    pub(crate) choices: Arc<Vec<ImportColumnChoice>>,
    pub(crate) import_path: Option<PathBuf>,
    pub(crate) import_delimiter: Delimiter,
    preview_delimiter: Delimiter,
    pub(crate) file_outcome: Option<SaveOutcome>,
    pub(crate) recovery_outcome: Option<SaveOutcome>,
    file_outcome_kind: file_feedback::FileOutcomeKind,
    show_recovery_outcome: bool,
    pub(crate) export_confirmation: Option<(PathBuf, Delimiter, JobToken)>,
    pub(crate) controls: BTreeMap<String, Rect>,
    pub(crate) control_ids: BTreeMap<String, Id>,
    pub(crate) search: String,
    pub(crate) picker: bool,
    pub(crate) new_name: String,
    pub(crate) new_kind: ColumnKind,
}
impl DesktopState {
    pub fn new() -> AppResult<Self> {
        let mut state = Self::from_project(Project::new("Untitled", Dataset::new(vec![])?)?)?;
        state.saved = Some(state.identity());
        Ok(state)
    }
    pub fn from_project(project: Project) -> AppResult<Self> {
        Budget::check_datasets(&[project.dataset()], project.owned_bytes()? + UI_WORK_BYTES)?;
        let selected_column = project.dataset().columns().first().map(|c| c.id());
        Ok(Self {
            project: Arc::new(project),
            survey: Default::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            budget_memo: Default::default(),
            document: 1,
            generation: 0,
            saved: None,
            path: None,
            stamp: None,
            workspace: Workspace::Data,
            analysis_mode: super::AnalysisMode::Guided,
            selected_column,
            selected_run: None,
            report_selection: Vec::new(),
            active_regression_reference: None,
            draft: AnalysisDraft::Describe(DescribeRequest {
                columns: vec![],
                statistics: vec![
                    Statistic::Mean,
                    Statistic::SampleStdDev,
                    Statistic::Minimum,
                    Statistic::Maximum,
                ],
                missing_policy: MissingPolicy::PerVariable,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            lookup: LookupState::default(),
            chart_roles: None,
            figure_editor: None,
            figure_work_bytes: 0,
            selected_figure: None,
            frequency_search: Default::default(),
            procedure_search: String::new(),
            group_values: None,
            cell_editor: None,
            metadata_editor: None,
            dialog: None,
            return_focus: None,
            properties_open: false,
            preparation: Default::default(),
            recovery_entries: None,
            recovery_finished: None,
            variable_search: String::new(),
            history_search: String::new(),
            selected_cell: None,
            column_widths: BTreeMap::new(),
            pending_column: None,
            pending_selection: None,
            pending_property_action: None,
            pending_new_variable: None,
            pending_workspace: None,
            pending_save_as: None,
            pending: None,
            pending_ready: false,
            cancel_for_pending: false,
            save_requested: false,
            active: false,
            active_recipe_bytes: 0,
            operation_name: "",
            progress: None,
            cancelled: false,
            abandoned_import: None,
            modal_file_dialog: false,
            shutdown: false,
            error: None,
            status: String::new(),
            preview: None,
            excel: Default::default(),
            choices: Arc::new(vec![]),
            import_path: None,
            import_delimiter: Delimiter::Comma,
            preview_delimiter: Delimiter::Comma,
            file_outcome: None,
            recovery_outcome: None,
            file_outcome_kind: file_feedback::FileOutcomeKind::Project,
            show_recovery_outcome: false,
            export_confirmation: None,
            controls: BTreeMap::new(),
            control_ids: BTreeMap::new(),
            search: String::new(),
            picker: false,
            new_name: String::new(),
            new_kind: ColumnKind::Number,
        })
    }
    pub fn project(&self) -> &Arc<Project> {
        &self.project
    }
    pub fn workspace(&self) -> Workspace {
        self.workspace
    }
    pub fn analysis_mode(&self) -> super::AnalysisMode {
        self.analysis_mode
    }
    pub fn document_id(&self) -> u64 {
        self.document
    }
    pub fn pending(&self) -> Option<&PendingAction> {
        self.pending.as_ref()
    }
    pub fn save_requested(&self) -> bool {
        self.save_requested
    }
    pub fn dialog(&self) -> Option<Dialog> {
        self.dialog
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn control_rect(&self, label: &str) -> Option<Rect> {
        self.controls.get(label).copied()
    }
    pub fn control_id(&self, label: &str) -> Option<Id> {
        self.control_ids.get(label).copied()
    }
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.identity())
    }
    fn identity(&self) -> (RevisionId, RunId) {
        (
            self.project.dataset().revision(),
            self.project.next_run_id(),
        )
    }
    pub fn token(&self) -> JobToken {
        JobToken {
            document: self.document,
            generation: self.generation,
            revision: self.project.dataset().revision(),
        }
    }
    pub fn selected_run(&self) -> Option<&Arc<AnalysisRun>> {
        self.selected_run
            .and_then(|id| self.project.runs().iter().find(|r| r.id == id))
    }
    pub fn selected_run_is_stale(&self) -> bool {
        self.selected_run()
            .is_some_and(|run| run.analysis.revision() != self.project.dataset().revision())
    }
    pub fn draft(&self) -> &AnalysisDraft {
        &self.draft
    }
    fn idle(&self) -> AppResult<()> {
        if self.active {
            Err(AppError::Conflict(
                "Wait for the current operation or cancel it.".into(),
            ))
        } else {
            Ok(())
        }
    }
    fn publish_dataset(&mut self, dataset: Dataset) -> AppResult<()> {
        self.ensure_frequency_invalidation()?;
        let project = self
            .project
            .with_dataset_with_budget(dataset, &[], self.extra())?;
        self.adopt_prepared_project(project)
    }
    pub fn apply_action(&self, action: UiAction) -> AppResult<Self> {
        if matches!(action, UiAction::Undo | UiAction::Redo) {
            return self.restore_history(matches!(action, UiAction::Redo));
        }
        self.check_ui_copy(
            self.copied_ui_bytes().saturating_mul(3).saturating_add(
                (self.undo.len() + self.redo.len())
                    .saturating_mul(std::mem::size_of::<Arc<crate::project::DataSnapshot>>()),
            ),
        )?;
        self.preflight_state_copy(&action)?;
        if matches!(action, UiAction::Request(PendingAction::Close))
            && matches!(self.pending, Some(PendingAction::Close))
        {
            return Ok(self.clone());
        }
        match &action {
            UiAction::MoveCell {
                columns,
                rows,
                edit,
            } => return self.move_cell(*columns, *rows, *edit),
            UiAction::PasteCells(text) => return self.paste_cells(text),
            UiAction::ResolveProperties(choice) => return self.resolve_properties(*choice),
            UiAction::CommitCell => {
                self.idle()?;
                if let Some(edit) = &self.cell_editor {
                    let value = if edit.missing {
                        Cell::Missing
                    } else {
                        match self.project.dataset().column(edit.column)?.kind() {
                            ColumnKind::Number => Cell::Number(parse_number(&edit.text)?),
                            ColumnKind::Text => {
                                self.check_ui_copy(edit.text.capacity())?;
                                Cell::Text(edit.text.clone())
                            }
                        }
                    };
                    return self.apply_action(UiAction::EditCell {
                        column: edit.column,
                        row: edit.row,
                        value,
                    });
                }
            }
            _ => {}
        }
        let mut next = self.clone();
        if !matches!(action, UiAction::SetAnalysisMode(_)) {
            next.error = None;
        }
        match action {
            UiAction::SetAnalysisMode(mode) => next.analysis_mode = mode,
            UiAction::Undo | UiAction::Redo => unreachable!(),
            UiAction::Navigate(view) => {
                if view != Workspace::Data && self.cell_editor.is_some() {
                    next = self.apply_action(UiAction::CommitCell)?;
                }
                if view != self.workspace && next.metadata_has_changes() {
                    next.pending_workspace = Some(view);
                    next.dialog = Some(Dialog::PropertyChanges);
                } else {
                    next.workspace = view;
                }
            }
            UiAction::SelectColumn(id) => {
                self.project.dataset().column(id)?;
                if self.selected_column != Some(id) {
                    if self.metadata_has_changes() {
                        next.pending_column = Some(id);
                        next.dialog = Some(Dialog::PropertyChanges);
                    } else {
                        if self.cell_editor.is_some() {
                            next = self.apply_action(UiAction::CommitCell)?;
                        }
                        next.selected_column = Some(id);
                        next.selected_cell = None;
                        next.metadata_editor = None;
                    }
                }
            }
            UiAction::SelectCell { column, row } => {
                self.project.dataset().column(column)?;
                if row >= self.project.dataset().row_count() {
                    return Err(AppError::invalid("row", "Row is out of range."));
                }
                next = self.apply_action(UiAction::SelectColumn(column))?;
                if next.dialog == Some(Dialog::PropertyChanges) {
                    next.pending_selection = Some((column, row));
                } else {
                    if self
                        .cell_editor
                        .as_ref()
                        .is_some_and(|e| (e.column, e.row) != (column, row))
                    {
                        next = next.apply_action(UiAction::CommitCell)?;
                    }
                    next.selected_cell = Some((column, row));
                }
            }
            UiAction::MoveCell { .. }
            | UiAction::PasteCells(_)
            | UiAction::ResolveProperties(_) => unreachable!(),
            UiAction::SelectRun(id) => {
                if !self.project.runs().iter().any(|r| r.id == id) {
                    return Err(AppError::invalid("history", "Run no longer exists."));
                }
                if next.selected_run != Some(id) {
                    next.clear_frequency_search()?;
                }
                next.selected_run = Some(id);
                next.prepare_chart_roles();
            }
            UiAction::SetDraft(draft) => {
                if draft.heap_bytes()? > MAX_DRAFT_BYTES {
                    return Err(AppError::SizeLimit {
                        limit: MAX_DRAFT_BYTES,
                    });
                }
                next.invalidate_for_draft(&draft)?;
                next.draft = draft;
                if !matches!(
                    &next.draft,
                    AnalysisDraft::Regression(_) | AnalysisDraft::Anova(_)
                ) || next
                    .draft
                    .group_roles(next.active_regression_reference)
                    .is_none()
                {
                    next.active_regression_reference = None;
                }
            }
            UiAction::EditCell { column, row, value } => {
                self.idle()?;
                let data = self.project.dataset().edit_cell_with_budget(
                    column,
                    row,
                    value,
                    &[],
                    self.project.owned_bytes()? + self.extra(),
                )?;
                next.publish_dataset(data)?;
                next.cell_editor = None;
            }
            UiAction::AppendRow => {
                self.idle()?;
                let data = self
                    .project
                    .dataset()
                    .append_row_with_budget(&[], self.project.owned_bytes()? + self.extra())?;
                next.publish_dataset(data)?;
            }
            UiAction::AddColumn { name, kind } => {
                self.idle()?;
                if self.metadata_has_changes() {
                    next.pending_new_variable = Some((name, kind));
                    next.dialog = Some(Dialog::PropertyChanges);
                    return Ok(next);
                }
                let id = self.project.next_column_id();
                let data = self
                    .project
                    .dataset()
                    .append_empty_column_with_id_with_budget(
                        id,
                        &name,
                        kind,
                        &[],
                        self.project.owned_bytes()? + self.extra(),
                    )?;
                next.publish_dataset(data)?;
                next.selected_column = Some(id);
                next.selected_cell = None;
                next.metadata_editor = None;
            }
            UiAction::Metadata { column, metadata } => {
                self.idle()?;
                let data = self.project.dataset().with_column_metadata_with_budget(
                    column,
                    metadata,
                    &[],
                    self.project.owned_bytes()? + self.extra(),
                )?;
                next.publish_dataset(data)?;
                if !next.metadata_has_changes() {
                    next.metadata_editor = None;
                }
            }
            UiAction::BeginCell { column, row } => {
                self.idle()?;
                if self.selected_cell != Some((column, row)) {
                    next = self.apply_action(UiAction::SelectCell { column, row })?;
                    if next.dialog == Some(Dialog::PropertyChanges) {
                        return Ok(next);
                    }
                }
                let c = self.project.dataset().column(column)?;
                if c.kind() == ColumnKind::Text
                    && c.text_value(row)?
                        .is_some_and(|s| s.len() > MAX_DRAFT_BYTES)
                {
                    return Err(AppError::SizeLimit {
                        limit: MAX_DRAFT_BYTES,
                    });
                }
                let copy = match c.kind() {
                    ColumnKind::Text => c.text_value(row)?.map_or(0, str::len),
                    ColumnKind::Number => 512,
                };
                self.check_ui_copy(copy)?;
                let value = c.cell(row)?;
                next.cell_editor = Some(CellEditor {
                    column,
                    row,
                    missing: matches!(value, Cell::Missing),
                    text: match value {
                        Cell::Number(v) => v.to_string(),
                        Cell::Text(v) => v,
                        Cell::Missing => String::new(),
                    },
                    focus: true,
                });
            }
            UiAction::CommitCell => {}
            UiAction::CancelCell => next.cell_editor = None,
            UiAction::Request(action) => {
                if self.metadata_has_changes() {
                    next.pending_property_action = Some(action);
                    next.dialog = Some(Dialog::PropertyChanges);
                    return Ok(next);
                }
                if self.cell_editor.is_some() {
                    next = self.apply_action(UiAction::CommitCell)?;
                }
                next.pending = Some(action);
                next.cancel_for_pending = self.active;
                next.pending_ready = !next.is_dirty();
                if !self.active && !next.pending_ready {
                    next.dialog = Some(Dialog::Unsaved);
                }
            }
            UiAction::ResolveUnsaved(choice) => match choice {
                UnsavedChoice::Cancel => {
                    next.cancel_survey_import();
                    next.pending = None;
                    next.pending_ready = false;
                    next.save_requested = false;
                    next.cancel_for_pending = false;
                    next.dialog = None;
                }
                UnsavedChoice::Save => {
                    next.save_requested = true;
                    next.dialog = None;
                }
                UnsavedChoice::Discard => {
                    next.dialog = None;
                    next.pending_ready = true;
                }
            },
        }
        Ok(next)
    }
    pub fn set_cell_text(&mut self, text: String) -> AppResult<()> {
        if text.len() > MAX_DRAFT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        if let Some(edit) = &mut self.cell_editor {
            edit.text = text;
            edit.missing = false;
        }
        Ok(())
    }
    pub(crate) fn act(&mut self, action: UiAction) {
        match self.apply_action(action) {
            Ok(next) => *self = next,
            Err(error) => self.fail(error),
        }
    }
    pub(crate) fn fail(&mut self, error: AppError) {
        if matches!(error, AppError::Cancelled) {
            self.status = "Operation cancelled.".into();
            return;
        }
        self.error = Some(file_feedback::bounded_error(error));
    }
    fn replace_new(&mut self) -> AppResult<()> {
        self.ensure_frequency_invalidation()?;
        let document = self
            .document
            .checked_add(1)
            .ok_or(AppError::IdentityExhausted)?;
        let outcome = self.file_outcome.take();
        let recovery_outcome = self.recovery_outcome.take();
        let outcome_kind = self.file_outcome_kind;
        let mut state = Self::new()?;
        state.document = document;
        state.analysis_mode = self.analysis_mode;
        state.inherit_frequency_generation(self.frequency_query_generation())?;
        state.file_outcome = outcome;
        state.recovery_outcome = recovery_outcome;
        state.file_outcome_kind = outcome_kind;
        *self = state;
        Ok(())
    }
    fn adopt(
        &mut self,
        project: Arc<Project>,
        path: Option<PathBuf>,
        stamp: Option<FileStamp>,
    ) -> AppResult<()> {
        self.ensure_frequency_invalidation()?;
        let document = self
            .document
            .checked_add(1)
            .ok_or(AppError::IdentityExhausted)?;
        self.project = project;
        self.survey = Default::default();
        self.preparation = Default::default();
        self.recovery_entries = None;
        self.undo.clear();
        self.redo.clear();
        self.document = document;
        self.path = path;
        self.stamp = stamp;
        self.saved = self.stamp.as_ref().map(|_| self.identity());
        self.selected_column = self.project.dataset().columns().first().map(|c| c.id());
        self.selected_run = self.project.runs().last().map(|r| r.id);
        self.report_selection.clear();
        self.figure_editor = None;
        self.figure_work_bytes = 0;
        self.selected_figure = None;
        self.prepare_chart_roles();
        self.draft = AnalysisDraft::Describe(DescribeRequest {
            columns: vec![],
            statistics: vec![
                Statistic::Mean,
                Statistic::SampleStdDev,
                Statistic::Minimum,
                Statistic::Maximum,
            ],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        self.clear_lookup();
        self.clear_frequency_search()?;
        self.error = None;
        self.procedure_search.clear();
        self.search.clear();
        self.variable_search.clear();
        self.history_search.clear();
        self.column_widths.clear();
        self.selected_cell = None;
        self.pending_column = None;
        self.pending_selection = None;
        self.pending_property_action = None;
        self.pending_new_variable = None;
        self.pending_workspace = None;
        self.pending_save_as = None;
        self.cell_editor = None;
        self.metadata_editor = None;
        self.preview = None;
        self.excel = Default::default();
        self.choices = Arc::new(vec![]);
        self.dialog = None;
        self.pending = None;
        self.pending_ready = false;
        self.workspace = Workspace::Data;
        self.status.clear();
        Ok(())
    }
    pub(crate) fn process_pending(&mut self, jobs: &mut JobRunner) {
        if self.active {
            if self.cancel_for_pending && !self.cancelled {
                jobs.cancel();
                self.cancelled = true;
            }
            return;
        }
        if !self.pending_ready {
            return;
        }
        let Some(action) = self.pending.take() else {
            return;
        };
        self.pending_ready = false;
        let result = match action {
            PendingAction::New => self.replace_new(),
            PendingAction::Open(path) => self.start_operation(jobs, JobOperation::Open { path }),
            PendingAction::Import(path, delimiter) => {
                self.import_path = Some(path.clone());
                self.import_delimiter = delimiter;
                self.start_operation(jobs, JobOperation::PreviewPath { path, delimiter })
            }
            PendingAction::ImportExcel(path) => {
                self.import_path = Some(path.clone());
                self.start_operation(jobs, JobOperation::InspectExcelPath { path })
            }
            PendingAction::Close => {
                jobs.begin_shutdown();
                self.shutdown = true;
                Ok(())
            }
        };
        if let Err(error) = result {
            self.fail(error);
        }
    }
    pub(crate) fn edit_import_choice(
        &mut self,
        index: usize,
        choice: ImportColumnChoice,
    ) -> AppResult<()> {
        self.idle()?;
        let bytes = crate::import::choice_bytes(&self.choices)?;
        Budget::check_datasets(
            &[self.project.dataset()],
            self.project.owned_bytes()? + self.extra() + bytes + choice.name.capacity() + 1024,
        )?;
        let mut choices = self.choices.as_ref().clone();
        choices[index] = choice;
        self.choices = Arc::new(choices);
        Ok(())
    }
    pub(crate) fn take_save_requested(&mut self) -> bool {
        let requested = self.save_requested;
        self.save_requested = false;
        requested
    }
}
