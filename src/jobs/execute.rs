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
                .map(JobOutput::RecoveryRetired),
            JobOperation::DiscardRecovery { store, entry } => store
                .discard_entry(entry, control, &datasets, extra)
                .map(JobOutput::RecoveryDiscarded),
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
            JobOperation::InspectSurvey { project } => {
                crate::survey::inspect_with_budget(project.dataset(), control, &datasets, extra)
                    .map(|review| JobOutput::SurveyReview(Arc::new(review)))
            }
            JobOperation::SurveyPresented {
                project,
                request,
                presentation,
            } => crate::survey::generate_presented_with_budget(
                project,
                request,
                *presentation,
                control,
                &datasets,
                extra - project.owned_bytes_controlled(control)? - request.owned_bytes()?,
            )
            .map(|p| JobOutput::SurveyReport(Arc::new(p))),
            JobOperation::PreviewSurveyCleaning { project, request } => {
                crate::survey::cleaning::preview_with_budget(
                    project.dataset(),
                    request,
                    control,
                    &datasets,
                    extra,
                )
                .map(|p| JobOutput::SurveyCleaningPreview(Arc::new(p)))
            }
            JobOperation::ApplySurveyCleaning { project, request } => {
                let p = preparation::prepare_with_budget(
                    project,
                    &request.step()?,
                    control,
                    &datasets,
                    extra - project.owned_bytes_controlled(control)?,
                )?;
                let mut retained = datasets.clone();
                retained.push(p.dataset());
                let review = crate::survey::inspect_with_budget(
                    p.dataset(),
                    control,
                    &retained,
                    extra.saturating_add(p.owned_bytes_controlled(control)?),
                )?;
                Ok(JobOutput::SurveyCleaned {
                    project: Arc::new(p),
                    request: request.clone(),
                    review: Arc::new(review),
                })
            }
            JobOperation::SaveSurveyRecipe {
                source,
                choices,
                cleanings,
                presentation,
                path,
                ..
            } => {
                let recipe = crate::survey::recipe::SurveyRecipe::capture(
                    source.dataset(),
                    choices,
                    cleanings,
                    *presentation,
                    "Survey report",
                    control,
                    &datasets,
                    extra,
                )?;
                crate::survey::recipe::save_with_budget(
                    path, &recipe, None, control, &datasets, extra,
                )
                .map(JobOutput::SurveyRecipeSaved)
            }
            JobOperation::LoadSurveyRecipe { project, path } => {
                let (recipe, _) =
                    crate::survey::recipe::load_with_budget(path, control, &datasets, extra)?;
                recipe.bind(project.dataset(), control)?;
                Ok(JobOutput::SurveyRecipe(Arc::new(recipe)))
            }
            JobOperation::ReplaySurveyRecipe { project, recipe } => {
                crate::survey::recipe::replay_with_budget(
                    project,
                    recipe,
                    control,
                    &datasets,
                    extra - project.owned_bytes_controlled(control)?,
                )
                .map(|p| JobOutput::SurveyRecipeReport(Arc::new(p)))
            }
            JobOperation::SurveyReport { project, request } => crate::survey::generate_with_budget(
                project,
                request,
                control,
                &datasets,
                extra - project.owned_bytes_controlled(control)? - request.owned_bytes()?,
            )
            .map(|project| JobOutput::SurveyReport(Arc::new(project))),
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
                acknowledge_formula_risk,
            } => {
                if !acknowledge_formula_risk
                    && export::has_formula_like_text(project.dataset(), control)?
                {
                    Budget::check_datasets(&datasets, add(extra, path.capacity())?)?;
                    control.check_cancelled()?;
                    return Ok(JobOutput::ExportDataConfirmation {
                        path: path.clone(),
                        delimiter: *delimiter,
                    });
                }
                export::export_data_with_budget(
                    path,
                    project.dataset(),
                    *delimiter,
                    control,
                    &datasets,
                    extra,
                )
                .map(JobOutput::Exported)
            }
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
