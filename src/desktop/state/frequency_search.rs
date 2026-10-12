//! Exact search identities queue on the existing worker; analysis always has priority.
use super::*;
use crate::analysis::{CompletedAnalysis, FrequencySearchRequest, FrequencySearchResult};
use crate::jobs::{FrequencyMetadata, FrequencyRowsOutput};
#[derive(Debug, Clone)]
pub(super) struct SearchIdentity {
    document: u64,
    generation: u64,
    pub(super) request: FrequencySearchRequest,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct FrequencyState {
    generation: u64,
    table_index: usize,
    metadata: Option<Arc<FrequencyMetadata>>,
    worker_metadata: Option<Arc<FrequencyMetadata>>,
    desired: Option<Arc<SearchIdentity>>,
    pending: bool,
    running: Option<(JobToken, Arc<SearchIdentity>)>,
    pub result: Option<Arc<FrequencySearchResult>>,
    worker_result: Option<Arc<FrequencySearchResult>>,
}
impl FrequencyState {
    fn metadata_bytes(&self) -> usize {
        let bytes = |cache: &FrequencyMetadata| {
            cache
                .owned_bytes()
                .unwrap_or(usize::MAX)
                .saturating_add(cache.orphaned_owner_bytes())
        };
        self.metadata
            .as_ref()
            .map_or(0, |c| bytes(c))
            .saturating_add(
                self.worker_metadata
                    .as_ref()
                    .filter(|c| !self.metadata.as_ref().is_some_and(|m| Arc::ptr_eq(c, m)))
                    .map_or(0, |c| bytes(c)),
            )
    }
    pub(super) fn owned_bytes(&self) -> usize {
        let request = |r: &SearchIdentity| {
            r.request.heap_bytes().unwrap_or(usize::MAX).saturating_add(
                std::mem::size_of::<SearchIdentity>() + 2 * std::mem::size_of::<usize>(),
            )
        };
        let page = |r: &FrequencySearchResult| {
            r.owned_bytes()
                .unwrap_or(usize::MAX)
                .saturating_add(2 * std::mem::size_of::<usize>())
        };
        self.metadata_bytes().saturating_add(
            self.desired
                .as_ref()
                .map_or(0, |r| request(r))
                .saturating_add(
                    self.running
                        .as_ref()
                        .filter(|(_, r)| !self.desired.as_ref().is_some_and(|d| Arc::ptr_eq(d, r)))
                        .map_or(0, |(_, r)| request(r)),
                )
                .saturating_add(self.result.as_ref().map_or(0, |r| page(r)))
                .saturating_add(
                    self.worker_result
                        .as_ref()
                        .filter(|r| !self.result.as_ref().is_some_and(|d| Arc::ptr_eq(d, r)))
                        .map_or(0, |r| page(r)),
                ),
        )
    }
}
impl DesktopState {
    pub(crate) fn frequency_table_index(&self) -> usize {
        self.frequency_search.table_index
    }
    pub(crate) fn frequency_metadata(&self) -> Option<&Arc<FrequencyMetadata>> {
        let run = self.selected_run()?;
        self.frequency_search
            .metadata
            .as_ref()
            .filter(|c| c.matches(run))
    }
    pub fn frequency_query_generation(&self) -> u64 {
        self.frequency_search.generation
    }
    pub fn frequency_request(&self) -> Option<&FrequencySearchRequest> {
        self.frequency_search.desired.as_ref().map(|r| &r.request)
    }
    pub fn frequency_rows(&self) -> Option<&Arc<FrequencySearchResult>> {
        let d = self.frequency_search.desired.as_ref()?;
        (d.document == self.document && self.selected_run == Some(d.request.run)).then_some(())?;
        self.frequency_search
            .result
            .as_ref()
            .filter(|r| r.request == d.request)
    }
    pub fn set_frequency_search(
        &mut self,
        column: ColumnId,
        query: String,
        offset: usize,
    ) -> AppResult<()> {
        let run = self
            .selected_run
            .ok_or_else(|| AppError::invalid("frequency search", "Select a frequency run."))?;
        if self.frequency_request().is_some_and(|r| {
            r.run == run && r.column == column && r.query == query && r.offset == offset
        }) {
            return Ok(());
        }
        let request = FrequencySearchRequest {
            run,
            column,
            query,
            offset,
        };
        let generation = self
            .frequency_search
            .generation
            .checked_add(1)
            .ok_or(AppError::IdentityExhausted)?;
        self.validate_frequency_request(&request)?;
        let table_index = self
            .selected_run()
            .and_then(|run| match &run.analysis {
                CompletedAnalysis::Frequencies(v) => {
                    v.tables.iter().position(|t| t.column == column)
                }
                _ => None,
            })
            .ok_or_else(|| AppError::invalid("frequency search", "Select a captured table."))?;
        Budget::check_datasets(
            &[self.project.dataset()],
            self.project
                .owned_bytes()?
                .saturating_add(self.extra())
                .saturating_add(request.heap_bytes()?)
                .saturating_add(std::mem::size_of::<SearchIdentity>() + 32),
        )?;
        self.frequency_search.desired = Some(Arc::new(SearchIdentity {
            document: self.document,
            generation,
            request,
        }));
        self.frequency_search.generation = generation;
        self.frequency_search.table_index = table_index;
        self.frequency_search.pending = true;
        self.frequency_search.result = None;
        Ok(())
    }
    fn validate_frequency_request(&self, request: &FrequencySearchRequest) -> AppResult<()> {
        if request.heap_bytes()? > MAX_DRAFT_BYTES {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        crate::search::NameQuery::new(&request.query)?;
        if request.query.chars().take(TEXT_LIMIT + 1).count() > TEXT_LIMIT {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        let Some(run) = self.selected_run() else {
            return Err(AppError::invalid(
                "frequency search",
                "Select a frequency run.",
            ));
        };
        let CompletedAnalysis::Frequencies(v) = &run.analysis else {
            return Err(AppError::invalid(
                "frequency search",
                "Select a frequency run.",
            ));
        };
        if run.id != request.run
            || !v
                .tables
                .iter()
                .any(|t| t.column == request.column && request.offset <= t.categories.len())
        {
            return Err(AppError::invalid(
                "frequency search",
                "Select a captured frequency table and page.",
            ));
        }
        Ok(())
    }
    pub(super) fn inherit_frequency_generation(&mut self, previous: u64) -> AppResult<()> {
        self.frequency_search.generation =
            previous.checked_add(1).ok_or(AppError::IdentityExhausted)?;
        Ok(())
    }
    pub(super) fn ensure_frequency_invalidation(&self) -> AppResult<()> {
        self.frequency_search
            .generation
            .checked_add(1)
            .ok_or(AppError::IdentityExhausted)?;
        Ok(())
    }
    pub(super) fn clear_frequency_search(&mut self) -> AppResult<()> {
        let generation = self
            .frequency_search
            .generation
            .checked_add(1)
            .ok_or(AppError::IdentityExhausted)?;
        self.frequency_search.generation = generation;
        self.frequency_search.desired = None;
        self.frequency_search.table_index = 0;
        self.frequency_search.metadata = None;
        self.frequency_search.pending = false;
        self.frequency_search.result = None;
        // Running request/page remains owned by the active worker until its terminal event.
        Ok(())
    }
    pub(super) fn prepare_frequency_search(
        &mut self,
        operation: &JobOperation,
    ) -> AppResult<Option<Arc<SearchIdentity>>> {
        let JobOperation::FrequencySearch { project, request } = operation else {
            return Ok(None);
        };
        if !Arc::ptr_eq(project, &self.project) {
            return Err(AppError::invalid(
                "frequency search",
                "Select the saved frequency run before searching.",
            ));
        }
        self.validate_frequency_request(request)?;
        Budget::check_datasets(
            &[self.project.dataset()],
            self.project
                .owned_bytes()?
                .saturating_add(self.extra())
                .saturating_add(request.heap_bytes()?.saturating_mul(2)),
        )?;
        self.set_frequency_search(request.column, request.query.clone(), request.offset)?;
        Ok(self.frequency_search.desired.clone())
    }
    pub(super) fn register_frequency_search(&mut self, identity: Option<Arc<SearchIdentity>>) {
        if let Some(identity) = identity {
            self.frequency_search.running = Some((self.token(), identity));
            self.frequency_search.pending = false;
        }
    }
    pub(super) fn capture_retained_frequency(&mut self) {
        self.frequency_search.worker_result = self.frequency_search.result.clone();
        self.frequency_search.worker_metadata = self.frequency_search.metadata.clone();
    }
    pub(super) fn release_retained_frequency(&mut self) {
        self.frequency_search.worker_result = None;
        self.frequency_search.worker_metadata = None;
    }
    pub(super) fn retain_frequency_search(&self, retained: &mut RetainedWork) {
        if let Some(result) = &self.frequency_search.result {
            retained.frequency_rows.push(Arc::clone(result));
        }
        retained.prepare_frequency_metadata = true;
        if let Some(cache) = &self.frequency_search.metadata {
            retained.frequency_metadata.push(Arc::clone(cache));
        }
        // Pages are represented above; identities are owned in the state, separately from task input.
        retained.extra_bytes = retained.extra_bytes.saturating_add(
            self.frequency_search
                .owned_bytes()
                .saturating_sub(self.frequency_search.metadata_bytes())
                .saturating_sub(self.frequency_search.result.as_ref().map_or(0, |r| {
                    r.owned_bytes()
                        .unwrap_or(usize::MAX)
                        .saturating_add(2 * std::mem::size_of::<usize>())
                })),
        );
    }
    pub(super) fn frequency_job_is_current(&self, token: JobToken) -> bool {
        self.frequency_search
            .running
            .as_ref()
            .is_some_and(|(t, r)| {
                *t == token
                    && r.document == self.document
                    && r.generation == self.frequency_search.generation
                    && self.selected_run == Some(r.request.run)
                    && self.frequency_request() == Some(&r.request)
            })
    }
    pub(super) fn accept_frequency_rows(
        &mut self,
        token: JobToken,
        output: FrequencyRowsOutput,
    ) -> AppResult<()> {
        let FrequencyRowsOutput {
            rows: result,
            metadata,
        } = output;
        let valid = self
            .frequency_search
            .running
            .as_ref()
            .is_some_and(|(t, r)| {
                *t == token
                    && r.document == self.document
                    && r.generation == self.frequency_search.generation
                    && r.request == result.request
            })
            && self.selected_run == Some(result.request.run)
            && self.frequency_request() == Some(&result.request);
        if valid {
            if metadata
                .as_ref()
                .is_some_and(|cache| !self.selected_run().is_some_and(|run| cache.matches(run)))
            {
                return Ok(());
            }
            let cache_bytes = metadata
                .as_ref()
                .filter(|cache| {
                    !self
                        .frequency_search
                        .metadata
                        .as_ref()
                        .is_some_and(|prior| Arc::ptr_eq(cache, prior))
                })
                .map_or(Ok(0), |cache| cache.owned_bytes())?;
            Budget::check_datasets(
                &[self.project.dataset()],
                self.project
                    .owned_bytes()?
                    .saturating_add(self.extra())
                    .saturating_add(result.owned_bytes()?)
                    .saturating_add(cache_bytes)
                    .saturating_add(2 * std::mem::size_of::<usize>()),
            )?;
            if let Some(metadata) = metadata {
                self.frequency_search.metadata = Some(metadata);
            }
            self.frequency_search.result = Some(Arc::new(result));
        }
        Ok(())
    }
    pub(crate) fn service_frequency_search(&mut self, jobs: &mut JobRunner) {
        if self.pending.is_some() || self.shutdown {
            self.frequency_search.pending = false;
            return;
        }
        if self.active {
            if self.operation_name == "Frequency rows"
                && self
                    .frequency_search
                    .running
                    .as_ref()
                    .is_some_and(|(_, r)| r.generation != self.frequency_search.generation)
            {
                jobs.cancel();
            }
            return;
        }
        self.frequency_search.running = None;
        if jobs.is_running() || !self.frequency_search.pending {
            return;
        }
        let Some(identity) = self.frequency_search.desired.clone() else {
            return;
        };
        if identity.document != self.document || self.selected_run != Some(identity.request.run) {
            self.frequency_search.pending = false;
            return;
        }
        let result = Budget::check_datasets(
            &[self.project.dataset()],
            self.project
                .owned_bytes()
                .unwrap_or(usize::MAX)
                .saturating_add(self.extra())
                .saturating_add(
                    identity
                        .request
                        .heap_bytes()
                        .unwrap_or(usize::MAX)
                        .saturating_mul(2),
                ),
        );
        if let Err(e) = result {
            self.frequency_search.pending = false;
            self.fail(e);
            return;
        }
        let result = self.start_operation(
            jobs,
            JobOperation::FrequencySearch {
                project: Arc::clone(&self.project),
                request: identity.request.clone(),
            },
        );
        if let Err(e) = result {
            self.frequency_search.pending = false;
            self.fail(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{analyze_with_budget, AnalysisRequest, FrequenciesRequest, RowFilter};
    use crate::data::Column;
    fn state() -> DesktopState {
        let data = Dataset::new(vec![Column::text(
            ColumnId(1),
            "Category",
            (0..600).map(|i| Some(format!("{i}"))).collect(),
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
            &crate::operation::JobControl::default(),
            &[],
            4096,
        )
        .unwrap();
        DesktopState::from_project(
            Project::new("Cache", data)
                .unwrap()
                .with_run(result)
                .unwrap(),
        )
        .unwrap()
        .apply_action(UiAction::SelectRun(RunId(1)))
        .unwrap()
    }
    #[test]
    fn worker_owned_page_and_identity_stay_charged_after_ui_replacement() {
        let mut state = state();
        let mut jobs = JobRunner::default();
        state
            .set_frequency_search(ColumnId(1), String::new(), 0)
            .unwrap();
        state.service_frequency_search(&mut jobs);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        let page = state.frequency_rows().unwrap().clone();
        let bytes = page.owned_bytes().unwrap();
        let cache = state.frequency_metadata().unwrap().clone();
        let cache_bytes = cache.owned_bytes().unwrap();
        state
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(&state.project),
                    request: AnalysisRequest::Frequencies(FrequenciesRequest {
                        group: None,
                        columns: vec![],
                        filter: RowFilter::default(),
                        frequency_weight: None,
                    }),
                },
            )
            .unwrap();
        assert!(Arc::ptr_eq(
            state.frequency_search.worker_result.as_ref().unwrap(),
            &page
        ));
        state.clear_frequency_search().unwrap();
        assert!(state.frequency_search.result.is_none());
        assert!(state.frequency_search.owned_bytes() >= bytes + cache_bytes);
        assert!(Arc::ptr_eq(
            state.frequency_search.worker_metadata.as_ref().unwrap(),
            &cache
        ));
        assert!(state.extra() >= UI_WORK_BYTES + bytes);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(state.frequency_search.worker_result.is_none());
        state
            .set_frequency_search(ColumnId(1), "A".into(), 0)
            .unwrap();
        state.service_frequency_search(&mut jobs);
        let running = state.frequency_search.running.as_ref().unwrap().1.clone();
        state
            .set_frequency_search(ColumnId(1), "B".into(), 0)
            .unwrap();
        assert!(Arc::ptr_eq(
            &running,
            &state.frequency_search.running.as_ref().unwrap().1
        ));
        assert!(state.frequency_search.owned_bytes() >= running.request.heap_bytes().unwrap());
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
    }
    #[test]
    fn identity_exhaustion_and_oversized_queries_leave_current_search_untouched() {
        let mut state = state();
        state
            .set_frequency_search(ColumnId(1), "A".into(), 0)
            .unwrap();
        let before = state.frequency_request().unwrap().clone();
        assert!(state
            .set_frequency_search(ColumnId(1), "x".repeat(TEXT_LIMIT + 1), 0)
            .is_err());
        assert_eq!(state.frequency_request(), Some(&before));
        state.frequency_search.generation = u64::MAX;
        assert!(matches!(
            state.set_frequency_search(ColumnId(1), "B".into(), 0),
            Err(AppError::IdentityExhausted)
        ));
        assert_eq!(state.frequency_request(), Some(&before));
        let project = Arc::clone(&state.project);
        let analysis = state.selected_run().unwrap().analysis.clone();
        assert!(matches!(
            state.handle_event(JobEvent::Completed {
                token: state.token(),
                cancelled: false,
                result: Ok(JobOutput::Analyzed(analysis))
            }),
            Err(AppError::IdentityExhausted)
        ));
        assert!(Arc::ptr_eq(&project, &state.project));
        assert_eq!(state.project.runs().len(), 1);
        assert!(matches!(
            state.replace_new(),
            Err(AppError::IdentityExhausted)
        ));
        assert!(Arc::ptr_eq(&project, &state.project));
    }
    #[test]
    fn latest_query_survives_explicit_analysis_cancellation_without_cancelling_itself() {
        let mut state = state();
        let mut jobs = JobRunner::default();
        state
            .start_operation(
                &mut jobs,
                JobOperation::Analyze {
                    project: Arc::clone(&state.project),
                    request: AnalysisRequest::Frequencies(FrequenciesRequest {
                        group: None,
                        columns: vec![ColumnId(1)],
                        filter: RowFilter::default(),
                        frequency_weight: None,
                    }),
                },
            )
            .unwrap();
        let token = state.token();
        for query in ["0", "5"] {
            state
                .set_frequency_search(ColumnId(1), query.into(), 0)
                .unwrap();
            state.service_frequency_search(&mut jobs);
        }
        assert_eq!(state.token(), token);
        assert!(!state.cancelled);
        assert!(state.active);
        jobs.cancel();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        assert_eq!(state.project.runs().len(), 1);
        assert_eq!(state.frequency_rows().unwrap().request.query, "5");
    }
    #[test]
    fn superseded_search_cancellation_does_not_restore_old_feedback() {
        let mut state = state();
        let mut jobs = JobRunner::default();
        state
            .set_frequency_search(ColumnId(1), "A".into(), 0)
            .unwrap();
        state.service_frequency_search(&mut jobs);
        let token = state.token();
        state
            .set_frequency_search(ColumnId(1), "B".into(), 0)
            .unwrap();
        state
            .handle_event(JobEvent::Completed {
                token,
                cancelled: true,
                result: Err(AppError::Cancelled),
            })
            .unwrap();
        assert!(state.error().is_none());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
    }
    #[test]
    fn invalid_search_run_is_refused_without_replacing_registered_query_or_page() {
        let mut state = state();
        let mut jobs = JobRunner::default();
        state
            .set_frequency_search(ColumnId(1), String::new(), 0)
            .unwrap();
        state.service_frequency_search(&mut jobs);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.is_running() {
            state.poll(&mut jobs);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        let before = state.frequency_request().unwrap().clone();
        let page = state.frequency_rows().unwrap().clone();
        let generation = state.frequency_query_generation();
        assert!(state
            .start_operation(
                &mut jobs,
                JobOperation::FrequencySearch {
                    project: Arc::clone(&state.project),
                    request: FrequencySearchRequest {
                        run: RunId(99),
                        column: ColumnId(1),
                        query: "new".into(),
                        offset: 0
                    }
                }
            )
            .is_err());
        assert_eq!(state.frequency_request(), Some(&before));
        assert_eq!(state.frequency_query_generation(), generation);
        assert!(Arc::ptr_eq(&page, state.frequency_rows().unwrap()));
        assert!(!jobs.is_running());
    }
}
