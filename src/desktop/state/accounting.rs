use super::*;
#[derive(Debug, Clone)]
pub(super) struct ProjectBaseline {
    owner: std::sync::Weak<Project>,
    project_bytes: usize,
    total_bytes: usize,
}
#[cfg(test)]
thread_local! {static ACCOUNTING_PASSES:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};}

impl DesktopState {
    pub(crate) fn project_baseline(&self) -> AppResult<(usize, usize)> {
        if let Some(memo) = self.budget_memo.borrow().as_ref() {
            if memo
                .owner
                .upgrade()
                .is_some_and(|owner| Arc::ptr_eq(&owner, &self.project))
            {
                return Ok((memo.project_bytes, memo.total_bytes));
            }
        }
        #[cfg(test)]
        ACCOUNTING_PASSES.with(|passes| passes.set(passes.get() + 1));
        let project_bytes = self.project.owned_bytes()?;
        let extra = self.extra();
        let total_bytes = Budget::dataset_bytes(
            &[self.project.dataset()],
            project_bytes
                .checked_add(extra)
                .ok_or(AppError::SizeLimit {
                    limit: crate::data::MAX_OWNED_BYTES,
                })?,
        )?
        .checked_sub(extra)
        .ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
        *self.budget_memo.borrow_mut() = Some(ProjectBaseline {
            owner: Arc::downgrade(&self.project),
            project_bytes,
            total_bytes,
        });
        Ok((project_bytes, total_bytes))
    }
    pub(crate) fn check_ui_copy(&self, copy_bytes: usize) -> AppResult<()> {
        let extra = self
            .extra()
            .checked_add(copy_bytes)
            .filter(|n| *n <= crate::data::MAX_OWNED_BYTES)
            .ok_or(AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?;
        let (_, baseline) = self.project_baseline()?;
        let total = baseline.checked_add(extra).ok_or(AppError::SizeLimit {
            limit: crate::data::MAX_OWNED_BYTES,
        })?;
        Budget::check_datasets(&[], total)
    }
    pub(super) fn retained_ui_bytes(&self) -> usize {
        let mut bytes = self.draft.heap_bytes().unwrap_or(usize::MAX);
        bytes = bytes.saturating_add(
            self.export_confirmation
                .as_ref()
                .map_or(0, |(path, _, _)| path.capacity()),
        );
        bytes = bytes.saturating_add(self.survey_owned_bytes());
        bytes = bytes.saturating_add(self.preparation.owned_bytes());
        if let Some(entries) = &self.recovery_entries {
            bytes = bytes.saturating_add(
                entries
                    .capacity()
                    .saturating_mul(std::mem::size_of::<crate::recovery::RecoveryEntry>()),
            );
            for (index, entry) in entries.iter().enumerate() {
                bytes = bytes.saturating_add(entry.path.capacity());
                if !entries[..index].iter().any(|prior| prior.same_owner(entry)) {
                    bytes = bytes.saturating_add(entry.owned_bytes().saturating_sub(
                        std::mem::size_of::<crate::recovery::RecoveryEntry>()
                            + entry.path.capacity(),
                    ));
                }
            }
        }
        bytes = bytes.saturating_add(self.figure_editor.as_ref().map_or(0, |e| e.owned_bytes()));
        bytes = bytes.saturating_add(self.figure_work_bytes);
        for text in [
            &self.procedure_search,
            &self.variable_search,
            &self.history_search,
            &self.search,
            &self.new_name,
            &self.status,
        ] {
            bytes = bytes.saturating_add(text.capacity());
        }
        bytes = bytes.saturating_add(self.error.as_ref().map_or(0, String::capacity));
        bytes = bytes.saturating_add(self.cell_editor.as_ref().map_or(0, |e| e.text.capacity()));
        bytes = bytes.saturating_add(
            self.metadata_editor
                .as_ref()
                .map_or(0, |e| e.metadata.heap_bytes().unwrap_or(usize::MAX)),
        );
        bytes = bytes.saturating_add(
            self.pending_new_variable
                .as_ref()
                .map_or(0, |(name, _)| name.capacity()),
        );
        bytes = bytes.saturating_add(self.lookup.copied_text_bytes());
        // BTreeMap nodes hold several entries; reserve conservatively for node rounding.
        bytes = bytes.saturating_add(self.column_widths.len().saturating_mul(1024));
        for name in self.controls.keys() {
            bytes = bytes.saturating_add(name.capacity()).saturating_add(1024);
        }
        for name in self.control_ids.keys() {
            bytes = bytes.saturating_add(name.capacity()).saturating_add(1024);
        }
        for path in [&self.path, &self.import_path, &self.excel.path]
            .into_iter()
            .flatten()
        {
            bytes = bytes.saturating_add(path.capacity());
        }
        for action in [&self.pending, &self.pending_property_action]
            .into_iter()
            .flatten()
        {
            if let PendingAction::Open(path)
            | PendingAction::Import(path, _)
            | PendingAction::ImportExcel(path) = action
            {
                bytes = bytes.saturating_add(path.capacity());
            }
        }
        bytes
    }
    pub(super) fn recovery_owner_bytes(&self) -> usize {
        let Some(entries) = &self.recovery_entries else {
            return 0;
        };
        entries
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                !entries[..*index]
                    .iter()
                    .any(|prior| prior.same_owner(entry))
            })
            .fold(0usize, |bytes, (_, entry)| {
                bytes.saturating_add(entry.owned_bytes().saturating_sub(
                    std::mem::size_of::<crate::recovery::RecoveryEntry>() + entry.path.capacity(),
                ))
            })
    }
    pub(super) fn copied_ui_bytes(&self) -> usize {
        self.retained_ui_bytes()
            .saturating_sub(self.recovery_owner_bytes())
            .saturating_sub(self.survey_review_bytes())
    }
    pub(crate) fn extra(&self) -> usize {
        self.frequency_search
            .owned_bytes()
            .saturating_add(self.history_bytes().unwrap_or(usize::MAX))
            .saturating_add(UI_WORK_BYTES)
            .saturating_add(self.retained_ui_bytes())
            .saturating_add(self.preview.as_ref().map_or(0, |p| p.owned_bytes()))
            .saturating_add(self.excel.owned_bytes())
            .saturating_add(self.group_values.as_ref().map_or(0, |v| {
                v.owned_bytes()
                    .unwrap_or(usize::MAX)
                    .saturating_add(2 * std::mem::size_of::<usize>())
            }))
            .saturating_add(self.lookup.pending_bytes())
            .saturating_add(self.active_recipe_bytes)
            .saturating_add(self.retained_lookup_bytes())
            .saturating_add(crate::import::choice_bytes(&self.choices).unwrap_or(usize::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analysis::*, data::Column, operation::JobControl};
    fn state() -> DesktopState {
        let data = Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "Value",
            (0..600).map(|i| Some(i as f64)).collect(),
        )
        .unwrap()])
        .unwrap();
        let result = analyze_with_budget(
            &data,
            &AnalysisRequest::Frequencies(FrequenciesRequest {
                group: None,
                columns: vec![ColumnId(1)],
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        let mut s = DesktopState::from_project(
            Project::new("Accounting", data)
                .unwrap()
                .with_run(result)
                .unwrap(),
        )
        .unwrap();
        s = s
            .apply_action(UiAction::SetDraft(
                s.draft
                    .for_procedure(super::super::super::draft::Procedure::Histogram),
            ))
            .unwrap()
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        s
    }
    fn frame(state: &mut DesktopState, ctx: &eframe::egui::Context, jobs: &mut JobRunner) {
        let mut output = ctx.run_ui(
            eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(1180., 780.),
                )),
                ..Default::default()
            },
            |ui| super::super::super::render(ui, state, jobs),
        );
        output.textures_delta.clear();
    }
    #[test]
    fn retained_property_search_and_pending_name_reduce_copy_headroom() {
        let mut state = state();
        let baseline = state.project_baseline().unwrap().1;
        let before = state.extra();
        state.variable_search = "v".repeat(8192);
        state.pending_new_variable = Some(("n".repeat(8192), ColumnKind::Text));
        assert!(state.extra() >= before + 16384);
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES - baseline - before - 1;
        assert!(matches!(
            state.check_ui_copy(0),
            Err(AppError::SizeLimit { .. })
        ));
    }
    #[test]
    fn rendered_warm_forms_reuse_owned_baseline_and_changed_project_recomputes_once() {
        let mut state = state();
        let ctx = eframe::egui::Context::default();
        super::super::super::configure_context(&ctx);
        let mut jobs = JobRunner::default();
        frame(&mut state, &ctx, &mut jobs);
        ACCOUNTING_PASSES.with(|p| p.set(0));
        for _ in 0..5 {
            frame(&mut state, &ctx, &mut jobs);
        }
        ACCOUNTING_PASSES.with(|p| assert_eq!(p.get(), 0));
        let old = state.project.clone();
        state = state
            .apply_action(UiAction::EditCell {
                column: ColumnId(1),
                row: 0,
                value: Cell::Number(-10.),
            })
            .unwrap();
        assert!(!Arc::ptr_eq(&old, &state.project));
        frame(&mut state, &ctx, &mut jobs);
        ACCOUNTING_PASSES.with(|p| assert_eq!(p.get(), 1));
        for _ in 0..5 {
            frame(&mut state, &ctx, &mut jobs);
        }
        ACCOUNTING_PASSES.with(|p| assert_eq!(p.get(), 1));
        let result = analyze_with_budget(
            state.project.dataset(),
            &AnalysisRequest::Histogram(HistogramRequest {
                variable: ColumnId(1),
                bins: HistogramBins::Automatic,
                filter: RowFilter::default(),
                frequency_weight: None,
            }),
            &JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        state
            .handle_event(JobEvent::Completed {
                token: state.token(),
                cancelled: false,
                result: Ok(JobOutput::Analyzed(result)),
            })
            .unwrap();
        frame(&mut state, &ctx, &mut jobs);
        assert_eq!(state.workspace(), Workspace::Results);
        // Results reuse frozen output; the next editable copy warms the new owner once.
        ACCOUNTING_PASSES.with(|p| assert_eq!(p.get(), 1));
        state.check_ui_copy(0).unwrap();
        ACCOUNTING_PASSES.with(|p| assert_eq!(p.get(), 2));
        let different_owner = Arc::new(state.project.as_ref().clone());
        state.adopt(different_owner, None, None).unwrap();
        state = state
            .apply_action(UiAction::Navigate(Workspace::Analyses))
            .unwrap();
        frame(&mut state, &ctx, &mut jobs);
        ACCOUNTING_PASSES.with(|p| assert_eq!(p.get(), 3));
        assert_eq!(
            state.project_baseline().unwrap().0,
            state.project.owned_bytes().unwrap()
        );
        assert_eq!(
            state.project_baseline().unwrap().1,
            Budget::dataset_bytes(
                &[state.project.dataset()],
                state.project.owned_bytes().unwrap()
            )
            .unwrap()
        );
    }
    #[test]
    fn warmed_baseline_rechecks_dynamic_worker_headroom_before_every_copy_and_render() {
        let mut state = state();
        state.check_ui_copy(0).unwrap();
        let draft = state.draft.clone();
        let project = state.project.clone();
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.checked_editable_draft());
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
        let ctx = eframe::egui::Context::default();
        super::super::super::configure_context(&ctx);
        let mut jobs = JobRunner::default();
        frame(&mut state, &ctx, &mut jobs);
        assert!(state.error().is_some());
        assert_eq!(state.draft, draft);
        assert!(Arc::ptr_eq(&project, &state.project));
        assert!(!jobs.is_running());
    }
}
