//! The single worker publishes only the exact current discovery identity.
use super::*;
use crate::analysis::GroupLookupRequest;
#[derive(Debug, Clone, Default)]
pub(crate) struct LookupState {
    epoch: u64,
    query: String,
    pending: Option<Arc<GroupLookupRequest>>,
    running: Option<(JobToken, Arc<GroupLookupRequest>)>,
    attempted: Option<(u64, RevisionId, u64, String)>,
    accepted: Option<JobToken>,
    worker_values: Option<Arc<GroupLookupResult>>,
}
impl LookupState {
    pub(super) fn copied_text_bytes(&self) -> usize {
        self.query.capacity().saturating_add(
            self.attempted
                .as_ref()
                .map_or(0, |(_, _, _, query)| query.capacity()),
        )
    }
    pub(super) fn pending_bytes(&self) -> usize {
        self.pending
            .as_ref()
            .map_or(0, |r| lookup_request_bytes(r))
            .saturating_add(
                self.running
                    .as_ref()
                    .map_or(0, |(_, r)| lookup_request_bytes(r)),
            )
    }
}
fn lookup_request_bytes(r: &GroupLookupRequest) -> usize {
    r.heap_bytes()
        .unwrap_or(usize::MAX)
        .saturating_add(std::mem::size_of::<GroupLookupRequest>())
        .saturating_add(2 * std::mem::size_of::<usize>())
}
impl DesktopState {
    pub fn active_regression_reference(&self) -> Option<ColumnId> {
        self.active_regression_reference
    }
    pub fn set_active_regression_reference(&mut self, active: Option<ColumnId>) -> AppResult<()> {
        if active.is_some()
            && (!matches!(
                &self.draft,
                AnalysisDraft::Regression(_) | AnalysisDraft::Anova(_)
            ) || self.draft.group_roles(active).is_none())
        {
            return Err(AppError::invalid(
                "reference",
                "Select a categorical predictor or ANOVA factor.",
            ));
        }
        if active != self.active_regression_reference {
            let epoch = self
                .lookup
                .epoch
                .checked_add(1)
                .ok_or(AppError::IdentityExhausted)?;
            self.clear_lookup();
            self.lookup.epoch = epoch;
            self.active_regression_reference = active;
        }
        Ok(())
    }
    pub(crate) fn set_draft_reference(
        &mut self,
        draft: AnalysisDraft,
        active: Option<ColumnId>,
    ) -> AppResult<()> {
        if active.is_some()
            && (!matches!(
                &draft,
                AnalysisDraft::Regression(_) | AnalysisDraft::Anova(_)
            ) || draft.group_roles(active).is_none())
        {
            return Err(AppError::invalid(
                "reference",
                "Select a categorical predictor or ANOVA factor.",
            ));
        }
        if draft.heap_bytes()? > MAX_DRAFT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        let changed = self.draft.procedure() != draft.procedure()
            || self.active_regression_reference != active
            || self.draft.group_roles(self.active_regression_reference)
                != draft.group_roles(active);
        let epoch = if changed {
            self.lookup
                .epoch
                .checked_add(1)
                .ok_or(AppError::IdentityExhausted)?
        } else {
            self.lookup.epoch
        };
        self.check_ui_copy(draft.heap_bytes()?)?;
        if changed {
            self.clear_lookup();
        }
        self.lookup.epoch = epoch;
        self.active_regression_reference = active;
        self.draft = draft;
        self.error = None;
        Ok(())
    }
    pub(super) fn capture_retained_lookup(&mut self) {
        self.lookup.worker_values = self.group_values.clone();
    }
    pub(super) fn release_retained_lookup(&mut self) {
        self.lookup.worker_values = None;
    }
    pub(super) fn retained_lookup_bytes(&self) -> usize {
        self.lookup
            .worker_values
            .as_ref()
            .filter(|held| {
                !self
                    .group_values
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(held, current))
            })
            .map_or(0, |held| {
                held.owned_bytes()
                    .unwrap_or(usize::MAX)
                    .saturating_add(2 * std::mem::size_of::<usize>())
            })
    }
    pub fn group_role_epoch(&self) -> u64 {
        self.lookup.epoch
    }
    pub fn group_query(&self) -> &str {
        &self.lookup.query
    }
    pub fn group_lookup(&self) -> Option<&Arc<GroupLookupResult>> {
        let token = self.lookup.accepted?;
        (token.document == self.document && token.revision == self.project.dataset().revision())
            .then_some(())?;
        let values = self.group_values.as_ref()?;
        self.lookup_matches(values).then_some(values)
    }
    fn lookup_matches(&self, result: &GroupLookupResult) -> bool {
        self.draft
            .group_roles(self.active_regression_reference)
            .is_some_and(|roles| roles.column == Some(result.column))
            && result.role_epoch == self.lookup.epoch
            && result.query == self.lookup.query
    }
    pub fn set_group_query(&mut self, query: String) -> AppResult<()> {
        if query.capacity() > MAX_DRAFT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        crate::search::NameQuery::new(&query)?;
        if self.lookup.query != query {
            self.lookup.query = query;
            self.group_values = None;
            self.lookup.accepted = None;
            self.lookup.pending = None;
            self.lookup.attempted = None;
        }
        Ok(())
    }
    pub(super) fn clear_lookup(&mut self) {
        self.group_values = None;
        self.lookup.accepted = None;
        self.lookup.pending = None;
        self.lookup.attempted = None;
    }
    pub(super) fn invalidate_for_draft(&mut self, draft: &AnalysisDraft) -> AppResult<()> {
        if self.draft.procedure() != draft.procedure()
            || self.draft.group_roles(self.active_regression_reference)
                != draft.group_roles(self.active_regression_reference)
        {
            let epoch = self
                .lookup
                .epoch
                .checked_add(1)
                .ok_or(AppError::IdentityExhausted)?;
            self.clear_lookup();
            self.lookup.epoch = epoch;
        }
        Ok(())
    }
    pub(super) fn preflight_state_copy(&self, action: &UiAction) -> AppResult<()> {
        if let UiAction::SetDraft(draft) = action {
            if draft.heap_bytes()? > MAX_DRAFT_BYTES {
                return Err(AppError::SizeLimit {
                    limit: MAX_DRAFT_BYTES,
                });
            }
            if (self.draft.procedure() != draft.procedure()
                || self.draft.group_roles(self.active_regression_reference)
                    != draft.group_roles(self.active_regression_reference))
                && self.lookup.epoch == u64::MAX
            {
                return Err(AppError::IdentityExhausted);
            }
        }
        // Selection and property resolution can overlap three nested state copies.
        self.check_ui_copy(self.retained_ui_bytes().saturating_mul(3))
    }
    pub(crate) fn checked_editable_draft(&self) -> AppResult<AnalysisDraft> {
        let bytes = self.draft.heap_bytes()?;
        if bytes > MAX_DRAFT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        self.check_ui_copy(bytes)?;
        Ok(self.draft.clone())
    }

    pub(super) fn accept_lookup(
        &mut self,
        token: JobToken,
        values: Arc<GroupLookupResult>,
    ) -> AppResult<()> {
        // Requiring a registered discovery prevents a same-token unrelated output from adoption.
        let valid = self.lookup.running.as_ref().is_some_and(|(t, r)| {
            *t == token
                && r.column == values.column
                && r.role_epoch == values.role_epoch
                && r.query == values.query
        });
        if valid && self.lookup_matches(&values) {
            Budget::check_datasets(
                &[self.project.dataset()],
                self.project.owned_bytes()? + self.extra() + values.owned_bytes()?,
            )?;
            self.group_values = Some(values);
            self.lookup.accepted = Some(token);
        }
        if self
            .lookup
            .running
            .as_ref()
            .is_some_and(|(t, _)| *t == token)
        {
            self.lookup.running = None;
        }
        Ok(())
    }
    pub(crate) fn service_lookup(&mut self, jobs: &mut JobRunner) {
        if self.pending.is_some() || self.shutdown {
            self.lookup.pending = None;
            return;
        }
        let role = self
            .draft
            .group_roles(self.active_regression_reference)
            .and_then(|roles| roles.column);
        if let Some((_, request)) = &self.lookup.running {
            if (role != Some(request.column)
                || request.role_epoch != self.lookup.epoch
                || request.query != self.lookup.query)
                && self.active
                && self.operation_name == "Group values"
            {
                jobs.cancel();
            }
        }
        if !self.active {
            self.lookup.running = None;
        }
        let Some(column) = role else {
            self.lookup.pending = None;
            return;
        };
        let identity = (
            self.document,
            self.project.dataset().revision(),
            self.lookup.epoch,
            self.lookup.query.as_str(),
        );
        let attempted = self
            .lookup
            .attempted
            .as_ref()
            .map(|(d, r, e, q)| (*d, *r, *e, q.as_str()));
        if attempted != Some(identity) {
            let result = self.prepare_lookup(column);
            match result {
                Ok(request) => {
                    self.lookup.pending = Some(Arc::new(request));
                    self.lookup.attempted = Some((
                        self.document,
                        self.project.dataset().revision(),
                        self.lookup.epoch,
                        self.lookup.query.clone(),
                    ));
                }
                Err(error) => {
                    self.fail(error);
                    return;
                }
            }
        }
        if self.active || jobs.is_running() {
            return;
        }
        if let Some(request) = self.lookup.pending.take() {
            let result = Budget::check_datasets(
                &[self.project.dataset()],
                self.project
                    .owned_bytes()
                    .unwrap_or(usize::MAX)
                    .saturating_add(self.extra())
                    .saturating_add(request.heap_bytes().unwrap_or(usize::MAX).saturating_mul(2)),
            );
            if let Err(error) = result {
                self.fail(error);
                return;
            }
            let Some(generation) = self.generation.checked_add(1) else {
                self.fail(AppError::IdentityExhausted);
                return;
            };
            self.lookup.running = Some((
                JobToken {
                    generation,
                    ..self.token()
                },
                Arc::clone(&request),
            ));
            match self.start_operation(
                jobs,
                JobOperation::GroupLookup {
                    project: Arc::clone(&self.project),
                    request: request.as_ref().clone(),
                },
            ) {
                Ok(()) => self.lookup.running = Some((self.token(), request)),
                Err(error) => {
                    self.lookup.running = None;
                    self.fail(error);
                }
            }
        }
    }
    fn prepare_lookup(&self, column: ColumnId) -> AppResult<GroupLookupRequest> {
        let selected = self
            .draft
            .group_roles(self.active_regression_reference)
            .expect("group role checked")
            .selected;
        let bytes = self.lookup.query.len()
            + selected
                .iter()
                .flatten()
                .map(|c| match c {
                    Cell::Text(s) => s.len(),
                    _ => 0,
                })
                .sum::<usize>();
        if bytes > MAX_DRAFT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        Budget::check_datasets(
            &[self.project.dataset()],
            self.project.owned_bytes()?
                + self.extra()
                + bytes
                + std::mem::size_of::<GroupLookupRequest>()
                + 2 * std::mem::size_of::<usize>(),
        )?;
        Ok(GroupLookupRequest {
            column,
            role_epoch: self.lookup.epoch,
            query: self.lookup.query.clone(),
            selected: selected.map(|cell| cell.cloned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{ConfidenceLevel, IndependentMethod};
    fn state() -> DesktopState {
        let p = crate::project_io::decode(
            &include_bytes!("../../../examples/mixed-analysis.tessera")[..],
            &crate::operation::JobControl::default(),
        )
        .unwrap();
        DesktopState::from_project(p).unwrap()
    }
    fn draft() -> AnalysisDraft {
        AnalysisDraft::IndependentT {
            outcome: Some(ColumnId(1)),
            group: Some(ColumnId(3)),
            groups: [
                Some(Cell::Text("absent".into())),
                Some(Cell::Text("other".into())),
            ],
            method: IndependentMethod::Welch,
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        }
    }
    fn register(state: &mut DesktopState) -> Arc<GroupLookupResult> {
        let request = Arc::new(state.prepare_lookup(ColumnId(3)).unwrap());
        state.lookup.running = Some((state.token(), request.clone()));
        Arc::new(GroupLookupResult {
            column: request.column,
            role_epoch: request.role_epoch,
            query: request.query.clone(),
            choices: vec![],
            selected_present: [false; 2],
            more: false,
        })
    }
    fn regression_draft() -> AnalysisDraft {
        use crate::analysis::*;
        AnalysisDraft::from_request(AnalysisRequest::Regression(RegressionRequest {
            outcome: ColumnId(1),
            predictors: vec![
                RegressionPredictor {
                    column: ColumnId(2),
                    role: PredictorRole::Numeric,
                },
                RegressionPredictor {
                    column: ColumnId(3),
                    role: PredictorRole::Categorical {
                        reference: Cell::Text("absent".into()),
                    },
                },
            ],
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        }))
    }
    #[test]
    fn successful_atomic_draft_edit_clears_prior_error() {
        let mut state = state();
        state.fail(AppError::IdentityExhausted);
        state
            .set_draft_reference(regression_draft(), Some(ColumnId(3)))
            .unwrap();
        assert!(state.error().is_none());
    }
    #[test]
    fn regression_epoch_exhaustion_and_role_changes_are_atomic() {
        let mut state = state()
            .apply_action(UiAction::SetDraft(regression_draft()))
            .unwrap();
        state
            .set_active_regression_reference(Some(ColumnId(3)))
            .unwrap();
        let original = state.draft.clone();
        state.lookup.epoch = u64::MAX;
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.set_active_regression_reference(None));
        assert!(matches!(result, Err(AppError::IdentityExhausted)));
        assert_eq!(largest, 0);
        assert_eq!(state.active_regression_reference, Some(ColumnId(3)));
        for change in 0..5 {
            let mut next = original.clone();
            if let AnalysisDraft::Regression(r) = &mut next {
                match change {
                    0 => r.predictors.remove(1),
                    1 => {
                        r.predictors[1].role =
                            super::super::super::draft::regression::RegressionDraftRole::Numeric;
                        r.predictors[1].clone()
                    }
                    2 => {
                        r.predictors[1].role = super::super::super::draft::regression::RegressionDraftRole::Categorical { reference: Some(Cell::Text("other".into())) };
                        r.predictors[1].clone()
                    }
                    _ => r.predictors[1].clone(),
                };
            }
            if change >= 3 {
                next = original.for_procedure(if change == 3 {
                    super::super::super::draft::Procedure::Pearson
                } else {
                    super::super::super::draft::Procedure::IndependentT
                });
            }
            let (result, largest) = crate::data::measure_test_allocations(|| {
                state.apply_action(UiAction::SetDraft(next))
            });
            assert!(matches!(result, Err(AppError::IdentityExhausted)));
            assert_eq!(largest, 0);
            assert_eq!(state.draft, original);
        }
        let mut next = original.clone();
        if let AnalysisDraft::Regression(r) = &mut next {
            r.predictors.swap(0, 1);
        }
        state = state.apply_action(UiAction::SetDraft(next)).unwrap();
        assert_eq!(state.group_role_epoch(), u64::MAX);
        assert_eq!(state.active_regression_reference(), Some(ColumnId(3)));
    }
    #[test]
    fn oversized_query_capacity_is_refused_before_matcher_or_assignment() {
        let mut state = state();
        let query = String::with_capacity(MAX_DRAFT_BYTES + 1);
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.set_group_query(query));
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
        assert_eq!(state.group_query(), "");
    }
    #[test]
    fn detached_worker_lookup_output_stays_charged_until_terminal_poll() {
        let mut state = state();
        let base = state.extra() - state.retained_ui_bytes();
        let values = Arc::new(GroupLookupResult {
            column: ColumnId(3),
            role_epoch: 0,
            query: String::new(),
            choices: vec![crate::analysis::GroupChoice {
                value: Cell::Text("x".repeat(1000)),
                label: String::new(),
            }],
            selected_present: [false; 2],
            more: false,
        });
        let bytes = values.owned_bytes().unwrap() + 2 * std::mem::size_of::<usize>();
        state.group_values = Some(values);
        state.capture_retained_lookup();
        assert_eq!(state.extra(), base + state.retained_ui_bytes() + bytes); // Arc aliases are charged once.
        state.set_group_query("new".into()).unwrap();
        assert_eq!(state.extra(), base + state.retained_ui_bytes() + bytes); // Worker still owns the old output.
        state.release_retained_lookup();
        assert_eq!(state.extra(), base + state.retained_ui_bytes());
    }
    #[test]
    fn exact_publication_requires_token_column_epoch_and_raw_query() {
        for changed in 0..5 {
            let mut state = state().apply_action(UiAction::SetDraft(draft())).unwrap();
            let values = register(&mut state);
            let token = state.token();
            match changed {
                0 => state.set_group_query(" ".into()).unwrap(),
                1 => {
                    let mut d = draft();
                    if let AnalysisDraft::IndependentT { group, .. } = &mut d {
                        *group = Some(ColumnId(2));
                    }
                    state = state.apply_action(UiAction::SetDraft(d)).unwrap();
                }
                2 => {
                    let mut d = draft();
                    if let AnalysisDraft::IndependentT { groups, .. } = &mut d {
                        groups[0] = Some(Cell::Text("replacement".into()));
                    }
                    state = state.apply_action(UiAction::SetDraft(d)).unwrap();
                }
                3 => state.document += 1,
                4 => state.generation += 1,
                _ => unreachable!(),
            }
            state
                .handle_event(JobEvent::Completed {
                    token,
                    cancelled: false,
                    result: Ok(JobOutput::GroupValues(values)),
                })
                .unwrap();
            assert!(state.group_lookup().is_none());
        }
        let mut state = state().apply_action(UiAction::SetDraft(draft())).unwrap();
        let values = register(&mut state);
        state
            .handle_event(JobEvent::Completed {
                token: state.token(),
                cancelled: false,
                result: Ok(JobOutput::GroupValues(values)),
            })
            .unwrap();
        assert!(state.group_lookup().is_some());
        state = state
            .apply_action(UiAction::EditCell {
                column: ColumnId(1),
                row: 0,
                value: Cell::Number(42.0),
            })
            .unwrap();
        assert!(state.group_lookup().is_none());
    }
    #[test]
    fn epoch_exhaustion_refuses_before_clone_and_nonroles_preserve_epoch() {
        let mut state = state().apply_action(UiAction::SetDraft(draft())).unwrap();
        let epoch = state.group_role_epoch();
        let mut d = draft();
        if let AnalysisDraft::IndependentT {
            confidence, method, ..
        } = &mut d
        {
            *confidence = ConfidenceLevel::P99;
            *method = IndependentMethod::Pooled;
        }
        state = state.apply_action(UiAction::SetDraft(d)).unwrap();
        assert_eq!(state.group_role_epoch(), epoch);
        state.lookup.epoch = u64::MAX;
        let d = state
            .draft
            .for_procedure(super::super::super::draft::Procedure::Pearson);
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.apply_action(UiAction::SetDraft(d)));
        assert!(matches!(result, Err(AppError::IdentityExhausted)));
        assert_eq!(largest, 0);
        assert_eq!(state.group_role_epoch(), u64::MAX);
    }
    #[test]
    fn newest_query_queues_without_cancelling_genuine_analysis() {
        let mut state = state().apply_action(UiAction::SetDraft(draft())).unwrap();
        let mut jobs = JobRunner::default();
        let request = state.checked_run_request(RunId(1)).unwrap();
        state
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(state.project()),
                    request,
                },
            )
            .unwrap();
        let token = state.token();
        for query in ["old", "newest"] {
            state.set_group_query(query.into()).unwrap();
            state.service_lookup(&mut jobs);
        }
        assert_eq!(state.token(), token);
        assert!(!state.cancelled);
        assert_eq!(state.lookup.pending.as_ref().unwrap().query, "newest");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        assert_eq!(state.project.runs().len(), 6);
        assert_eq!(state.group_lookup().unwrap().query, "newest");
    }
    #[test]
    fn live_lookup_clone_stays_charged_through_cancel_and_queued_query() {
        let mut state = state().apply_action(UiAction::SetDraft(draft())).unwrap();
        let (_, baseline) = state.project_baseline().unwrap(); // Warm before admission.
        let base_extra = state.extra();
        let initial_ui = state.retained_ui_bytes();
        let mut jobs = JobRunner::default();
        state.service_lookup(&mut jobs); // Actual cloned GroupLookup operation.
        assert!(jobs.is_running());
        assert!(state.active);
        let token = state.token();
        let request = &state.lookup.running.as_ref().unwrap().1;
        let worker_heap = request.heap_bytes().unwrap();
        let running_bytes = lookup_request_bytes(request);
        assert!(worker_heap > 0);
        // Fits exactly if the worker's second request payload is omitted.
        let copy = crate::data::MAX_OWNED_BYTES - baseline - base_extra - running_bytes;
        let (result, largest) = crate::data::measure_test_allocations(|| state.check_ui_copy(copy));
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
        assert_eq!(state.active_recipe_bytes, worker_heap);
        assert_eq!(
            state.extra(),
            base_extra - initial_ui + state.retained_ui_bytes() + running_bytes + worker_heap
        );
        assert_eq!(state.project_baseline().unwrap().1, baseline);
        state.set_group_query("newest".into()).unwrap();
        state.service_lookup(&mut jobs); // Cancel old discovery and queue replacement.
        let pending = state.lookup.pending.as_ref().unwrap();
        assert_eq!(pending.query, "newest");
        let pending_bytes = lookup_request_bytes(pending);
        assert_eq!(state.lookup.running.as_ref().unwrap().1.query, "");
        assert_eq!(
            state.extra(),
            base_extra - initial_ui
                + state.retained_ui_bytes()
                + running_bytes
                + pending_bytes
                + worker_heap
        );
        state
            .handle_event(JobEvent::CancellationRequested { token })
            .unwrap();
        assert_eq!(state.active_recipe_bytes, worker_heap);
        let stale = JobToken {
            generation: token.generation - 1,
            ..token
        };
        assert!(matches!(
            state.handle_event(JobEvent::Completed {
                token: stale,
                cancelled: true,
                result: Err(AppError::Cancelled),
            }),
            Err(AppError::Cancelled)
        ));
        assert!(state.active);
        assert_eq!(state.active_recipe_bytes, worker_heap);
        assert_eq!(
            state.extra(),
            base_extra - initial_ui
                + state.retained_ui_bytes()
                + running_bytes
                + pending_bytes
                + worker_heap
        );
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut completed = false;
        while jobs.is_running() {
            for event in jobs.poll() {
                let terminal =
                    matches!(&event, JobEvent::Completed { token: t, .. } if *t == token);
                let result = state.handle_event(event);
                assert!(result.is_ok() || matches!(result, Err(AppError::Cancelled)));
                if terminal {
                    completed = true;
                    assert!(!state.active);
                    assert_eq!(state.active_recipe_bytes, 0);
                } else {
                    assert_eq!(state.active_recipe_bytes, worker_heap);
                }
            }
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(completed);
        assert_eq!(state.lookup.pending.as_ref().unwrap().query, "newest");
        state.service_lookup(&mut jobs);
        let replacement = &state.lookup.running.as_ref().unwrap().1;
        assert_eq!(replacement.query, "newest");
        assert_eq!(state.active_recipe_bytes, replacement.heap_bytes().unwrap());
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        assert_eq!(state.active_recipe_bytes, 0);
        assert!(state.lookup.running.is_none());
        assert!(state.lookup.pending.is_none());
        assert_eq!(state.group_lookup().unwrap().query, "newest");
    }
    #[test]
    fn saved_absent_group_rerun_is_not_replaced() {
        let mut state = state().apply_action(UiAction::SetDraft(draft())).unwrap();
        let selected = state
            .draft
            .group_roles(None)
            .unwrap()
            .selected
            .map(|c| c.cloned());
        let mut jobs = JobRunner::default();
        state.service_lookup(&mut jobs);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        assert_eq!(
            state.draft.group_roles(None).unwrap().selected,
            [selected[0].as_ref(), selected[1].as_ref()]
        );
        let request = state.checked_draft_request().unwrap();
        let result = crate::analysis::analyze_with_budget(
            state.project.dataset(),
            &request,
            &crate::operation::JobControl::default(),
            &[],
            state.extra(),
        )
        .unwrap();
        let crate::analysis::CompletedAnalysis::IndependentT(result) = result else {
            panic!()
        };
        assert_eq!(result.groups[0].n, 0);
        assert_eq!(result.groups[1].n, 0);
        assert_eq!(state.project.runs().len(), 5);
    }
}

#[cfg(test)]
mod anova_tests;
