//! Single-worker operation ownership and publication.
use super::*;
impl DesktopState {
    pub fn start_operation(
        &mut self,
        jobs: &mut JobRunner,
        operation: JobOperation,
    ) -> AppResult<()> {
        self.idle()?;
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
                        self.save_requested = false;
                        self.import_delimiter = self.preview_delimiter;
                        return Err(error);
                    }
                };
                if let Some(outcome) = output.file_outcome() {
                    let verb = match outcome.publication {
                        crate::files::Publication::Committed
                            if matches!(output, JobOutput::Saved(_)) =>
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
                        self.adopt(Arc::new(project), None, None)?;
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
