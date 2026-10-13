//! Single-worker operation ownership and publication.
use super::*;
impl DesktopState {
    pub(crate) fn confirm_data_export(&mut self, jobs: &mut JobRunner) -> AppResult<()> {
        self.idle()?;
        let Some((path, delimiter, token)) = self.export_confirmation.take() else {
            return Err(AppError::Conflict("Choose Export data again.".into()));
        };
        self.dialog = None;
        if token != self.token() {
            return Err(AppError::Conflict(
                "The project changed. Choose Export data again to review the current data.".into(),
            ));
        }
        self.start_operation(
            jobs,
            JobOperation::ExportData {
                project: Arc::clone(&self.project),
                path,
                delimiter,
                acknowledge_formula_risk: true,
            },
        )
    }
    pub fn start_operation(
        &mut self,
        jobs: &mut JobRunner,
        operation: JobOperation,
    ) -> AppResult<()> {
        self.idle()?;
        if let JobOperation::InspectSurvey { project }
        | JobOperation::SurveyReport { project, .. }
        | JobOperation::SurveyPresented { project, .. }
        | JobOperation::PreviewSurveyCleaning { project, .. }
        | JobOperation::ApplySurveyCleaning { project, .. }
        | JobOperation::SaveSurveyRecipe { project, .. }
        | JobOperation::LoadSurveyRecipe { project, .. }
        | JobOperation::ReplaySurveyRecipe { project, .. } = &operation
        {
            if !Arc::ptr_eq(project, &self.project) {
                return Err(AppError::Conflict(
                    "Review the current project only.".into(),
                ));
            }
            if self.metadata_has_changes() || self.cell_editor.is_some() {
                return Err(AppError::Conflict(
                    "Apply or discard pending edits before reviewing the survey.".into(),
                ));
            }
        }
        if let JobOperation::Prepare { project, .. } = &operation {
            if !Arc::ptr_eq(project, &self.project) {
                return Err(AppError::Conflict(
                    "Prepare the current project only.".into(),
                ));
            }
            if self.metadata_has_changes() || self.cell_editor.is_some() {
                return Err(AppError::Conflict(
                    "Finish editing before preparing data.".into(),
                ));
            }
        }
        if let (JobOperation::ConvertColumn { id, .. }, Some(editor)) =
            (&operation, &self.metadata_editor)
        {
            let column = self.project.dataset().column(editor.column)?;
            if editor.metadata != *column.metadata()
                || (editor.column != *id && editor.kind != column.kind())
            {
                return Err(AppError::invalid(
                    "properties",
                    "Apply or discard property changes before converting a variable.",
                ));
            }
        }
        self.check_ui_copy(0)?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(AppError::IdentityExhausted)?;
        let recipe_bytes = match &operation {
            JobOperation::Prepare { step, .. } => step.owned_bytes()?,
            JobOperation::CommitExcel { choices, .. } => crate::import::choice_bytes(choices)?,
            JobOperation::SurveyReport { request, .. } => request.owned_bytes()?,
            JobOperation::SurveyPresented { request, .. } => request.owned_bytes()?,
            JobOperation::Analyze { request, .. } => request.heap_bytes()?,
            JobOperation::FrequencySearch { request, .. } => request.heap_bytes()?,
            JobOperation::GroupLookup { request, .. } => request.heap_bytes()?,
            _ => 0,
        };
        let name = match &operation {
            JobOperation::Prepare { .. } => "Prepare data",
            JobOperation::Checkpoint { .. } => "Recovery checkpoint",
            JobOperation::Recover { .. } => "Recover work",
            JobOperation::DiscoverRecovery { .. } => "Find recoverable work",
            JobOperation::RetireCheckpoint { .. } => "Retire checkpoint",
            JobOperation::DiscardRecovery { .. } => "Discard recovery",
            JobOperation::InspectExcelPath { .. } => "Inspect Excel",
            JobOperation::PreviewExcel { .. } => "Excel preview",
            JobOperation::CommitExcel { .. } => "Excel import",
            JobOperation::PreviewPath { .. } => "Import preview",
            JobOperation::CommitImport { .. } => "Import",
            JobOperation::Open { .. } => "Open",
            JobOperation::InspectSurvey { .. } => "Review survey",
            JobOperation::SurveyReport { .. } => "Create survey report",
            JobOperation::SurveyPresented { .. } => "Create survey report",
            JobOperation::PreviewSurveyCleaning { .. } => "Preview survey cleaning",
            JobOperation::ApplySurveyCleaning { .. } => "Apply survey cleaning",
            JobOperation::SaveSurveyRecipe { .. } => "Save survey recipe",
            JobOperation::LoadSurveyRecipe { .. } => "Load survey recipe",
            JobOperation::ReplaySurveyRecipe { .. } => "Apply recipe and create report",
            JobOperation::Analyze { .. } => "Analysis",
            JobOperation::GroupLookup { .. } => "Group values",
            JobOperation::FrequencySearch { .. } => "Frequency rows",
            JobOperation::ConvertColumn { .. } => "Convert variable",
            JobOperation::Save { .. } => "Save",
            JobOperation::ExportData { .. } => "Export data",
            JobOperation::ExportReport { .. } => "Export report",
            JobOperation::ExportFigure { .. } => "Export figure",
            JobOperation::ExportFigures { .. } => "Export figures",
        };
        let mut retained = RetainedWork {
            projects: vec![Arc::clone(&self.project)],
            extra_bytes: UI_WORK_BYTES
                + self.history_bytes()?
                + self.retained_ui_bytes()
                + crate::import::choice_bytes(&self.choices)?
                + self.lookup.pending_bytes(),
            ..Default::default()
        };
        if let Some(result) = &self.group_values {
            retained.group_values.push(Arc::clone(result));
        }
        if let Some(preview) = &self.preview {
            retained.previews.push(Arc::clone(preview));
        }
        if let Some(workbook) = &self.excel.workbook {
            retained.excel_workbooks.push(Arc::clone(workbook));
        }
        if let Some(preview) = &self.excel.preview {
            retained.excel_previews.push(Arc::clone(preview));
        }
        let frequency_request = self.prepare_frequency_search(&operation)?;
        self.retain_frequency_search(&mut retained);
        jobs.start(
            JobTask {
                operation,
                retained,
            },
            JobToken {
                generation,
                ..self.token()
            },
        )?;
        self.generation = generation;
        self.register_frequency_search(frequency_request);
        self.capture_retained_frequency();
        self.active = true;
        self.active_recipe_bytes = recipe_bytes;
        self.capture_retained_lookup();
        self.operation_name = name;
        self.progress = None;
        self.error = None;
        self.cancelled = false;
        Ok(())
    }
    pub fn poll(&mut self, jobs: &mut JobRunner) {
        let events = jobs.poll();
        let control = jobs.completion_control().unwrap_or_default();
        for event in events {
            if let Err(error) = self.handle_event_controlled(event, &control) {
                self.fail(error);
            }
        }
        if let Err(error) = self.service_survey(jobs) {
            self.fail(error);
        }
        self.service_frequency_search(jobs);
        self.service_lookup(jobs);
        if !self.active
            && self.pending.is_some()
            && !self.pending_ready
            && !self.save_requested
            && self.dialog.is_none()
        {
            self.dialog = Some(Dialog::Unsaved);
        }
    }
    pub fn handle_event(&mut self, event: JobEvent) -> AppResult<()> {
        self.handle_event_controlled(event, &crate::operation::JobControl::default())
    }
    fn handle_event_controlled(
        &mut self,
        event: JobEvent,
        control: &crate::operation::JobControl,
    ) -> AppResult<()> {
        let current_import = event.token() == self.token()
            && matches!(
                self.operation_name,
                "Inspect Excel" | "Excel preview" | "Excel import" | "Import preview" | "Import"
            );
        let result = self.handle_event_inner_controlled(event, control);
        if result.is_err() && current_import {
            self.cancel_survey_import();
        }
        result
    }
    fn handle_event_inner_controlled(
        &mut self,
        event: JobEvent,
        control: &crate::operation::JobControl,
    ) -> AppResult<()> {
        let publish = event.may_publish(
            self.document,
            self.project.dataset().revision(),
            self.generation,
        );
        let saved = event.confirms_current_save(
            self.document,
            self.project.dataset().revision(),
            self.generation,
        );
        match event {
            JobEvent::Progress { token, progress } if token == self.token() => {
                self.progress = Some(progress)
            }
            JobEvent::CancellationRequested { token } if token == self.token() => {
                self.cancelled = true;
                self.status = "Cancelling…".into();
            }
            JobEvent::Completed { token, result, .. } => {
                let recovery_job = matches!(
                    self.operation_name,
                    "Recovery checkpoint"
                        | "Recover work"
                        | "Find recoverable work"
                        | "Retire checkpoint"
                        | "Discard recovery"
                );
                if recovery_job {
                    self.recovery_finished = Some((token, false));
                }
                if token == self.token() {
                    self.active = false;
                    self.active_recipe_bytes = 0;
                    self.release_retained_lookup();
                    self.release_retained_frequency();
                    self.cancel_for_pending = false;
                    self.progress = None;
                }
                let output = match result {
                    Ok(output) => output,
                    Err(error) => {
                        if token != self.token()
                            && matches!(
                                self.operation_name,
                                "Inspect Excel" | "Excel preview" | "Excel import"
                            )
                        {
                            return Ok(());
                        }
                        if self.operation_name == "Frequency rows"
                            && !self.frequency_job_is_current(token)
                        {
                            return Ok(());
                        }
                        if token == self.token() && matches!(error, AppError::Cancelled) {
                            self.status.clear();
                        }
                        if token == self.token()
                            && matches!(
                                self.operation_name,
                                "Inspect Excel"
                                    | "Excel preview"
                                    | "Excel import"
                                    | "Import preview"
                                    | "Import"
                            )
                        {
                            self.cancel_survey_import();
                        }
                        self.save_requested = false;
                        self.import_delimiter = self.preview_delimiter;
                        return Err(error);
                    }
                };
                if let Some(outcome) = output.file_outcome() {
                    let verb = match outcome.publication {
                        crate::files::Publication::Committed
                            if matches!(
                                output,
                                JobOutput::Saved(_) | JobOutput::SurveyRecipeSaved(_)
                            ) =>
                        {
                            "Saved"
                        }
                        crate::files::Publication::Committed => "Exported",
                        crate::files::Publication::Conflict => "File conflict:",
                        crate::files::Publication::Uncertain => "Check file:",
                    };
                    let filename = outcome
                        .destination
                        .file_name()
                        .map(std::path::Path::new)
                        .unwrap_or(&outcome.destination);
                    self.status =
                        super::super::theme::caption(format_args!("{verb} {}", filename.display()));
                    self.file_outcome = Some(outcome.clone());
                    if outcome.warning.is_some()
                        || outcome.publication != crate::files::Publication::Committed
                    {
                        self.dialog = Some(Dialog::FileOutcome);
                    }
                }
                if saved {
                    if let JobOutput::Saved(outcome) = &output {
                        self.path = Some(outcome.destination.clone());
                        self.stamp = outcome.committed_stamp.clone();
                        self.saved = Some(self.identity());
                        self.save_requested = false;
                        if self.pending.is_some() {
                            self.pending_ready = true;
                        }
                    }
                }
                if !publish {
                    return Ok(());
                }
                match output {
                    JobOutput::ExportDataConfirmation { path, delimiter } => {
                        self.export_confirmation = Some((path, delimiter, token));
                        self.dialog = Some(Dialog::ExportDataConfirmation);
                    }
                    JobOutput::SurveyCleaningPreview(preview) => {
                        self.accept_survey_cleaning_preview(preview)?
                    }
                    JobOutput::SurveyCleaned {
                        project,
                        request,
                        review,
                    } => self.accept_survey_cleaned(project, request, review, control)?,
                    JobOutput::SurveyRecipe(recipe) => self.accept_survey_recipe(recipe)?,
                    JobOutput::SurveyRecipeSaved(_) => {}
                    JobOutput::SurveyRecipeReport(project) => {
                        self.accept_survey_recipe_report(project, control)?
                    }
                    JobOutput::SurveyReview(review) => self.accept_survey_review(review)?,
                    JobOutput::SurveyReport(project) => {
                        self.accept_survey_report(project, control)?
                    }
                    JobOutput::Prepared(project) => {
                        self.adopt_prepared_project_arc(project)?;
                        self.metadata_editor = None;
                        self.status = "Data prepared.".into();
                    }
                    JobOutput::Recovered(project) => {
                        self.adopt(project, None, None)?;
                        self.recovery_finished = Some((token, true));
                        self.status = "Recovered work. Save to keep it.".into();
                    }
                    JobOutput::RecoveryEntries(entries) => {
                        self.recovery_entries = Some(entries);
                        self.recovery_finished = Some((token, true));
                    }
                    JobOutput::Checkpoint(outcome) => {
                        if outcome.publication != crate::files::Publication::Committed
                            || outcome.warning.is_some()
                        {
                            return Err(AppError::Conflict("Recovery checkpoint could not be confirmed. Your earlier checkpoint is preserved.".into()));
                        }
                        self.status.clear();
                        self.recovery_finished = Some((token, true));
                    }
                    JobOutput::RecoveryRetired | JobOutput::RecoveryDiscarded => {
                        self.status.clear();
                        self.recovery_finished = Some((token, true));
                    }
                    JobOutput::ExcelWorkbook(workbook) => self.accept_excel_workbook(workbook)?,
                    JobOutput::ExcelPreview(preview) => self.accept_excel_preview(preview)?,
                    JobOutput::Preview(preview) => {
                        let choices = preview.default_choices_with_budget(
                            &[self.project.dataset()],
                            self.project.owned_bytes()? + self.extra(),
                        )?;
                        self.preview_delimiter = self.import_delimiter;
                        self.preview = Some(preview);
                        self.choices = Arc::new(choices);
                        self.dialog = Some(Dialog::Import);
                    }
                    JobOutput::Imported(dataset) => {
                        let source_path = if self.operation_name == "Excel import" {
                            &self.excel.path
                        } else {
                            &self.import_path
                        };
                        let title = source_path
                            .as_ref()
                            .and_then(|p| p.file_stem())
                            .and_then(|s| s.to_str())
                            .unwrap_or("Imported data");
                        Budget::check_datasets(
                            &[self.project.dataset(), &dataset],
                            self.project.owned_bytes()? + self.extra() + title.len() + 1024,
                        )?;
                        let project = Project::new(title, dataset)?;
                        let provenance = source_path
                            .as_ref()
                            .map(|p| {
                                format!(
                                    "Imported from {}",
                                    p.file_name().unwrap_or_default().to_string_lossy()
                                )
                            })
                            .into_iter()
                            .collect();
                        let project = project.with_provenance_with_budget(
                            provenance,
                            &[self.project.dataset()],
                            self.project.owned_bytes()? + self.extra(),
                        )?;
                        let survey_import = self.survey.import_requested;
                        self.adopt(Arc::new(project), None, None)?;
                        self.survey.entry_pending = survey_import;
                    }
                    JobOutput::Opened {
                        project,
                        path,
                        stamp,
                    } => self.adopt(project, Some(path), Some(stamp))?,
                    JobOutput::Analyzed(analysis) => {
                        self.ensure_frequency_invalidation()?;
                        let id = self.project.next_run_id();
                        self.project = Arc::new(self.project.with_run_with_budget_controlled(
                            analysis,
                            &[],
                            self.extra(),
                            control,
                        )?);
                        self.clear_frequency_search()?;
                        self.selected_run = Some(id);
                        self.prepare_chart_roles();
                        self.workspace = Workspace::Results;
                    }
                    JobOutput::Converted(dataset) => {
                        self.publish_dataset(dataset)?;
                        self.metadata_editor = None;
                    }
                    JobOutput::GroupValues(values) => self.accept_lookup(token, values)?,
                    JobOutput::FrequencyRows(values) => {
                        self.accept_frequency_rows(token, values)?
                    }
                    JobOutput::FiguresExported { outcomes } => {
                        let completed = outcomes
                            .iter()
                            .filter(|o| o.committed_stamp.is_some())
                            .count();
                        self.status = format!(
                            "Exported {completed} figures: {}",
                            outcomes
                                .iter()
                                .filter(|o| o.committed_stamp.is_some())
                                .map(|o| o.destination.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        if let Some(outcome) = outcomes.iter().find(|o| {
                            o.publication != crate::files::Publication::Committed
                                || o.warning.is_some()
                        }) {
                            self.file_outcome = Some(outcome.clone());
                            self.dialog = Some(Dialog::FileOutcome);
                        }
                    }
                    JobOutput::Saved(_) | JobOutput::Exported(_) => {}
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_acknowledgement_cannot_outlive_document_revision_or_generation() {
        for identity in 0..3 {
            let mut state = DesktopState::new().unwrap();
            let mut stale = state.token();
            match identity {
                0 => stale.document += 1,
                1 => stale.revision = RevisionId(stale.revision.0 + 1),
                _ => stale.generation += 1,
            }
            state.export_confirmation =
                Some((PathBuf::from("unused.csv"), Delimiter::Comma, stale));
            state.dialog = Some(Dialog::ExportDataConfirmation);
            assert!(state
                .confirm_data_export(&mut JobRunner::default())
                .is_err());
            assert!(state.export_confirmation.is_none());
            assert!(!state.active);
        }
    }
    #[test]
    fn stale_or_cancelled_export_scan_never_opens_confirmation() {
        for cancelled in [false, true] {
            let mut state = DesktopState::new().unwrap();
            let mut token = state.token();
            if !cancelled {
                token.generation += 1;
            }
            state
                .handle_event(JobEvent::Completed {
                    token,
                    cancelled,
                    result: Ok(JobOutput::ExportDataConfirmation {
                        path: PathBuf::from("unused.csv"),
                        delimiter: Delimiter::Comma,
                    }),
                })
                .unwrap();
            assert!(state.export_confirmation.is_none());
            assert_ne!(state.dialog, Some(Dialog::ExportDataConfirmation));
        }
    }
    #[test]
    fn stale_cancellation_requests_cannot_change_current_feedback_or_release_owners() {
        let mut state = DesktopState::new().unwrap();
        state.active = true;
        state.generation = 2;
        state.active_recipe_bytes = 12345;
        state.status = "Running analysis".into();
        let project = Arc::clone(state.project());
        let current = state.token();
        for identity in 0..3 {
            let mut stale = current;
            match identity {
                0 => stale.document += 1,
                1 => stale.revision = RevisionId(stale.revision.0 + 1),
                _ => stale.generation += 1,
            }
            state
                .handle_event(JobEvent::CancellationRequested { token: stale })
                .unwrap();
            assert!(!state.cancelled);
            assert_eq!(state.status, "Running analysis");
            assert!(state.active);
            assert_eq!(state.active_recipe_bytes, 12345);
            assert!(Arc::ptr_eq(&project, state.project()));
        }
        state
            .handle_event(JobEvent::CancellationRequested { token: current })
            .unwrap();
        assert!(state.cancelled);
        assert_eq!(state.status, "Cancelling…");
        assert_eq!(state.active_recipe_bytes, 12345);
    }
}
