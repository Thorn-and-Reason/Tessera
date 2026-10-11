use super::*;

impl JobTask {
    #[cfg(test)]
    pub(super) fn execute(&self, control: &JobControl) -> AppResult<JobOutput> {
        self.execute_preflighted(control)
    }
    pub(super) fn execute_preflighted(&self, control: &JobControl) -> AppResult<JobOutput> {
        control.checkpoint(0, None)?;
        let extra = self.preflight_controlled(Some(control))?;
        let mut datasets = Vec::with_capacity(self.dataset_count());
        datasets.extend(self.datasets());
        match &self.operation {
            JobOperation::DiscoverRecovery { store } => {
                // Discovery is bounded to 128 entries; reserve path/lock and hash scratch before I/O.
                Budget::check_datasets(
                    &datasets,
                    add(extra, crate::recovery::DISCOVERY_WORK_BYTES)?,
                )?;
                let entries = store.list_with_control(control)?;
                let mut bytes = entries.capacity() * size_of::<RecoveryEntry>();
                for entry in &entries {
                    bytes = add(bytes, entry.owned_bytes())?;
                }
                Budget::check_datasets(&datasets, add(extra, bytes)?)?;
                Ok(JobOutput::RecoveryEntries(entries))
            }

            JobOperation::RetireCheckpoint { store } => store
                .discard_owned_with_budget(control, &datasets, extra)
                .map(|()| JobOutput::RecoveryRetired),
            JobOperation::DiscardRecovery { store, entry } => store
                .discard_entry(entry, control, &datasets, extra)
                .map(|()| JobOutput::RecoveryDiscarded),
            JobOperation::Prepare { project, step } => preparation::prepare_with_budget(
                project,
                step,
                control,
                &datasets,
                extra - project.owned_bytes_controlled(control)?,
            )
            .map(|project| JobOutput::Prepared(Arc::new(project))),
            JobOperation::Checkpoint {
                project,
                store,
                sequence,
            } => store
                .checkpoint_with_budget(
                    project,
                    *sequence,
                    control,
                    &datasets,
                    extra - project.owned_bytes_controlled(control)?,
                )
                .map(JobOutput::Checkpoint),
            JobOperation::Recover { store, entry } => store
                .recover_with_budget(entry, control, &datasets, extra)
                .map(|project| JobOutput::Recovered(Arc::new(project))),
            JobOperation::InspectExcelPath { path } => {
                let config = excel::config(control)?;
                excel_import::inspect_with_budget(path, &config, control, &datasets, extra)
                    .map(|workbook| JobOutput::ExcelWorkbook(Arc::new(workbook)))
            }
            JobOperation::PreviewExcel {
                workbook,
                selection,
            } => {
                let config = excel::config(control)?;
                excel_import::preview_with_budget(
                    workbook.clone(),
                    *selection,
                    &config,
                    control,
                    &datasets,
                    extra,
                )
                .map(|preview| JobOutput::ExcelPreview(Arc::new(preview)))
            }
            JobOperation::CommitExcel {
                preview,
                choices,
                decisions,
            } => excel_import::commit_preview_with_budget(
                preview, choices, *decisions, control, &datasets, extra,
            )
            .map(JobOutput::Imported),
            JobOperation::PreviewPath { path, delimiter } => {
                let bytes = read_import(path, control, &datasets, extra)?;
                import::preview_with_budget(bytes, *delimiter, control, &datasets, extra)
                    .map(|preview| JobOutput::Preview(Arc::new(preview)))
            }
            JobOperation::CommitImport { preview, choices } => {
                import::commit_preview_with_budget(preview, choices, control, &datasets, extra)
                    .map(JobOutput::Imported)
            }
            JobOperation::Open { path } => files::read_project_with_budget(
                path, control, &datasets, extra,
            )
            .map(|(project, stamp)| JobOutput::Opened {
                project: Arc::new(project),
                path: path.clone(),
                stamp,
            }),
            JobOperation::Analyze { project, request } => {
                analysis::analyze_with_budget(project.dataset(), request, control, &datasets, extra)
                    .map(JobOutput::Analyzed)
            }
            JobOperation::FrequencySearch { project, request } => {
                let run = project
                    .runs()
                    .binary_search_by_key(&request.run, |r| r.id)
                    .ok()
                    .and_then(|i| project.runs().get(i))
                    .ok_or_else(|| {
                        AppError::invalid(
                            "frequency search",
                            "The saved frequency run does not exist.",
                        )
                    })?;
                let metadata = if self.retained.prepare_frequency_metadata {
                    match self
                        .retained
                        .frequency_metadata
                        .iter()
                        .find(|cache| cache.matches(run))
                    {
                        Some(cache) => Some(Arc::clone(cache)),
                        None => Some(frequency_view::prepare(run, control, &datasets, extra)?),
                    }
                } else {
                    None
                };
                let additional = metadata
                    .as_ref()
                    .filter(|cache| {
                        !self
                            .retained
                            .frequency_metadata
                            .iter()
                            .any(|prior| Arc::ptr_eq(prior, cache))
                    })
                    .map_or(Ok(0), |cache| cache.owned_bytes())?;
                let rows = project.frequency_rows_with_budget(
                    request,
                    control,
                    &datasets,
                    add(
                        extra - project.owned_bytes()? - request.heap_bytes()?,
                        additional,
                    )?,
                )?;
                Ok(JobOutput::FrequencyRows(FrequencyRowsOutput {
                    rows,
                    metadata,
                }))
            }
            JobOperation::GroupLookup { project, request } => analysis::group_values_with_budget(
                project.dataset(),
                request,
                control,
                &datasets,
                extra,
            )
            .map(|result| JobOutput::GroupValues(Arc::new(result))),
            JobOperation::ConvertColumn { project, id, kind } => project
                .dataset()
                .convert_column_with_budget(*id, *kind, control, &datasets, extra)
                .map(JobOutput::Converted),
            JobOperation::Save {
                project,
                path,
                expected,
            } => files::save_project_with_budget(
                path,
                project,
                expected.as_ref(),
                control,
                &datasets,
                extra - project.owned_bytes()?,
            )
            .map(JobOutput::Saved),
            JobOperation::ExportData {
                project,
                path,
                delimiter,
            } => export::export_data_with_budget(
                path,
                project.dataset(),
                *delimiter,
                control,
                &datasets,
                extra,
            )
            .map(JobOutput::Exported),
            JobOperation::ExportFigures {
                project,
                directory,
                keys,
                format,
            } => figures::export_batch(
                project,
                directory,
                keys,
                *format,
                control,
                &datasets,
                extra - project.owned_bytes()?,
            )
            .map(|outcomes| JobOutput::FiguresExported { outcomes }),
            JobOperation::ExportFigure {
                project,
                path,
                key,
                format,
            } => export::figures::export_figure(
                path,
                project,
                key,
                *format,
                control,
                &datasets,
                extra - project.owned_bytes()?,
            )
            .map(JobOutput::Exported),
            JobOperation::ExportReport {
                project,
                path,
                runs,
            } => export::export_report_with_budget(
                path,
                project,
                runs,
                control,
                &datasets,
                extra - project.owned_bytes()?,
            )
            .map(JobOutput::Exported),
        }
    }
}
