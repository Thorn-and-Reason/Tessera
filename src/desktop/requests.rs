//! One checked clone boundary for draft computation, reruns and retained settings.
mod requirements;
use super::{draft::AnalysisDraft, state::MAX_DRAFT_BYTES, DesktopState};
use crate::{
    analysis::{AnalysisRequest, AnalysisRequestRef},
    error::{AppError, AppResult},
    operation::RunId,
};
enum RequestSource<'a> {
    Draft(&'a AnalysisDraft),
    Saved(AnalysisRequestRef<'a>),
}
impl DesktopState {
    fn checked_request_copy(
        &self,
        source: RequestSource<'_>,
        text_bytes: usize,
        draft_bound: bool,
    ) -> AppResult<AnalysisRequest> {
        let (heap, scratch) = match source {
            RequestSource::Draft(d) => (d.heap_bytes()?, d.validation_scratch_bytes()?),
            RequestSource::Saved(r) => (r.heap_bytes()?, r.validation_scratch_bytes()?),
        };
        if draft_bound
            && heap
                .checked_add(text_bytes)
                .is_none_or(|n| n > MAX_DRAFT_BYTES)
        {
            return Err(AppError::SizeLimit {
                limit: MAX_DRAFT_BYTES,
            });
        }
        let copy = heap
            .checked_add(text_bytes)
            .and_then(|n| n.checked_add(scratch))
            .ok_or(AppError::SizeLimit {
                limit: crate::data::MAX_OWNED_BYTES,
            })?;
        self.check_ui_copy(copy)?;
        match source {
            RequestSource::Draft(d) => {
                if let Some(message) = self.draft_requirement_for(d) {
                    return Err(AppError::invalid("Analysis settings", message));
                }
                d.build_request(self.project().dataset()).map_err(|error| match error {
                    AppError::InvalidInput { location, .. } if location == "analysis" =>
                        AppError::invalid("Analysis settings", "Check that variable roles and statistics are distinct, and that filters and groups match the selected variables."),
                    other => other,
                })
            }
            RequestSource::Saved(r) => Ok(r.clone_owned()),
        }
    }
    pub fn checked_run_request(&self, id: RunId) -> AppResult<AnalysisRequest> {
        let run = self
            .project()
            .runs()
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| AppError::invalid("history", "Run no longer exists."))?;
        let r =
            self.checked_request_copy(RequestSource::Saved(run.analysis.request()), 0, false)?;
        r.validate_current(
            self.project().dataset(),
            &[],
            self.project_baseline()?.0 + self.extra() + r.heap_bytes()?,
        )?;
        Ok(r)
    }
    pub fn checked_draft_request(&self) -> AppResult<AnalysisRequest> {
        self.checked_request_copy(RequestSource::Draft(&self.draft), 0, true)
    }
    pub(crate) fn load_selected_request(&mut self) {
        if let Some(run) = self.selected_run() {
            // Float64's decimal display fits within 512 bytes, including subnormal values.
            let text = usize::from(matches!(
                run.analysis.request(),
                AnalysisRequestRef::OneSampleT(_)
            )) * 512;
            let conversion = match run.analysis.request() {
                AnalysisRequestRef::Regression(r) => {
                    r.predictors.len().saturating_mul(std::mem::size_of::<
                        super::draft::regression::RegressionPredictorDraft,
                    >())
                }
                AnalysisRequestRef::Reliability(r) => {
                    match super::draft::reliability::ReliabilityDraft::conversion_bytes(r) {
                        Ok(n) => n,
                        Err(e) => {
                            self.fail(e);
                            return;
                        }
                    }
                }
                AnalysisRequestRef::Anova(r) => {
                    match super::draft::anova::AnovaDraft::conversion_bytes(r) {
                        Ok(n) => n,
                        Err(e) => {
                            self.fail(e);
                            return;
                        }
                    }
                }
                _ => 0,
            };
            match self.checked_request_copy(
                RequestSource::Saved(run.analysis.request()),
                text.saturating_add(conversion),
                true,
            ) {
                Ok(r) => {
                    let draft = AnalysisDraft::from_request(r);
                    let active = match &draft {
                        AnalysisDraft::Regression(r) => r.predictors.iter().find(|p| matches!(p.role, super::draft::regression::RegressionDraftRole::Categorical { .. })).map(|p| p.column),
                        AnalysisDraft::Anova(r) => r.independent_factor(0),
                        _ => None,
                    };
                    if let Err(e) = self.set_draft_reference(draft, active) {
                        self.fail(e);
                    }
                }
                Err(e) => self.fail(e),
            }
        }
    }
}

#[cfg(test)]
use super::UiAction;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{Cell, ColumnId, ColumnKind},
        operation::JobControl,
        project_io,
    };
    #[test]
    fn retained_settings_are_loaded_without_reinterpreting_current_types() {
        let p = project_io::decode(
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            &JobControl::default(),
        )
        .unwrap();
        let current = p
            .dataset()
            .convert_column_with_budget(
                ColumnId(1),
                ColumnKind::Text,
                &JobControl::default(),
                &[],
                p.owned_bytes().unwrap(),
            )
            .unwrap();
        let p = p.with_dataset(current).unwrap();
        let mut state = DesktopState::from_project(p).unwrap();
        for id in 1..=5 {
            state.act(UiAction::SelectRun(RunId(id)));
            state.load_selected_request();
            assert!(state.error().is_none());
            assert!(state
                .draft
                .matches_saved(state.selected_run().unwrap().analysis.request()));
            assert!(state.checked_run_request(RunId(id)).is_err());
            assert!(state.checked_draft_request().is_err());
        }
    }
    #[test]
    fn matching_settings_allocates_nothing_and_live_lookup_counts_before_request_clone() {
        let p = project_io::decode(
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            &JobControl::default(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(p).unwrap();
        state.act(UiAction::SelectRun(RunId(5)));
        state.load_selected_request();
        let (_, largest) = crate::data::measure_test_allocations(|| {
            assert!(state
                .draft
                .matches_saved(state.selected_run().unwrap().analysis.request()))
        });
        assert_eq!(largest, 0);
        let lookup = crate::analysis::GroupLookupResult {
            column: ColumnId(3),
            role_epoch: 1,
            query: String::new(),
            choices: vec![crate::analysis::GroupChoice {
                value: Cell::Text("x".repeat(1000)),
                label: String::new(),
            }],
            selected_present: [false, false],
            more: false,
        };
        let before = state.extra();
        let bytes = lookup.owned_bytes().unwrap() + 2 * std::mem::size_of::<usize>();
        state.group_values = Some(std::sync::Arc::new(lookup));
        assert_eq!(state.extra() - before, bytes);
        // The helper includes live result storage before cloning a saved recipe.
        let request = state.checked_run_request(RunId(5)).unwrap();
        assert_eq!(request, state.selected_run().unwrap().analysis.request());
    }
}

#[cfg(test)]
mod clone_budget_tests {
    use super::*;
    #[test]
    fn every_draft_and_saved_variant_refuses_before_cloning_when_live_headroom_is_exhausted() {
        let p = crate::project_io::decode(
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            &crate::operation::JobControl::default(),
        )
        .unwrap();
        let state = DesktopState::from_project(p).unwrap();
        for run in state.project().runs() {
            let recipe = run.analysis.request();
            let draft = AnalysisDraft::from_request(recipe.clone_owned());
            let (_, largest) =
                crate::data::measure_test_allocations(|| assert!(draft.matches_saved(recipe)));
            assert_eq!(largest, 0);
            let (result, largest) = crate::data::measure_test_allocations(|| {
                state.checked_request_copy(
                    RequestSource::Saved(recipe),
                    crate::data::MAX_OWNED_BYTES,
                    false,
                )
            });
            assert!(matches!(result, Err(AppError::SizeLimit { .. })));
            assert_eq!(largest, 0);
            let (result, largest) = crate::data::measure_test_allocations(|| {
                state.checked_request_copy(
                    RequestSource::Draft(&draft),
                    crate::data::MAX_OWNED_BYTES,
                    false,
                )
            });
            assert!(matches!(result, Err(AppError::SizeLimit { .. })));
            assert_eq!(largest, 0);
        }
    }
}

#[cfg(test)]
mod workflow_budget_tests {
    use super::*;
    #[test]
    fn native_run_button_and_shortcut_refuse_exhausted_clone_budget() {
        use eframe::egui::{self, Event, Modifiers};
        let p = crate::project_io::decode(
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            &crate::operation::JobControl::default(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(p).unwrap();
        state.act(UiAction::SelectRun(RunId(3)));
        state.load_selected_request();
        state.act(UiAction::Navigate(super::super::Workspace::Analyses));
        let project = std::sync::Arc::clone(state.project());
        let ctx = egui::Context::default();
        super::super::configure_context(&ctx);
        let mut jobs = crate::jobs::JobRunner::default();
        let frame = |state: &mut DesktopState, jobs: &mut crate::jobs::JobRunner, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 780.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| super::super::render(ui, state, jobs),
            );
            output.textures_delta.clear();
        };
        frame(&mut state, &mut jobs, vec![]);
        let pos = state.control_rect("Run analysis").unwrap().center();
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
        for pressed in [true, false] {
            frame(
                &mut state,
                &mut jobs,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(!jobs.is_running());
        assert!(
            state.control_rect("Run analysis").is_some(),
            "the fixed footer stays visible when copying the draft is refused"
        );
        assert!(
            state.error().is_some(),
            "the button must explain the clone-budget refusal"
        );
        frame(
            &mut state,
            &mut jobs,
            vec![Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            }],
        );
        assert!(!jobs.is_running());
        assert!(state.error().is_some());
        assert!(std::sync::Arc::ptr_eq(&project, state.project()));
    }
    #[test]
    fn run_shortcut_and_retained_settings_share_before_clone_refusal() {
        let p = crate::project_io::decode(
            &include_bytes!("../../examples/mixed-analysis.tessera")[..],
            &crate::operation::JobControl::default(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(p).unwrap();
        state.act(UiAction::SelectRun(RunId(5)));
        state.load_selected_request();
        let original = state.draft.clone();
        // Synthetic live-worker reservation proves the boundary before allocating a copy.
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES;
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.checked_draft_request());
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.checked_run_request(RunId(5)));
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
        state.load_selected_request();
        assert_eq!(state.draft, original);
        assert!(state.error().is_some());
    }
}

#[cfg(test)]
mod regression_budget_tests {
    use super::*;
    use crate::{
        analysis::*,
        data::{Cell, Column, ColumnId, Dataset, MAX_OWNED_BYTES},
        operation::JobControl,
        project::Project,
    };
    #[test]
    fn saved_conversion_precharges_destination_predictor_vector_at_real_caller() {
        let columns = (1..=61)
            .map(|id| {
                Column::numeric(
                    ColumnId(id),
                    &format!("v{id}"),
                    vec![Some(1.), Some(2.), Some(3.), Some(5.)],
                )
                .unwrap()
            })
            .collect();
        let data = Dataset::new(columns).unwrap();
        let request = AnalysisRequest::Regression(RegressionRequest {
            outcome: ColumnId(1),
            predictors: (2..=61)
                .map(|id| RegressionPredictor {
                    column: ColumnId(id),
                    role: PredictorRole::Numeric,
                })
                .collect(),
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        let result = analyze_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[],
            request.heap_bytes().unwrap(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(
            Project::new("Conversion", data)
                .unwrap()
                .with_run(result)
                .unwrap(),
        )
        .unwrap();
        state.act(UiAction::SelectRun(RunId(1)));
        let saved = state.selected_run().unwrap().analysis.request();
        let destination =
            60 * std::mem::size_of::<super::super::draft::regression::RegressionPredictorDraft>();
        let headroom =
            saved.heap_bytes().unwrap() + saved.validation_scratch_bytes().unwrap() + destination
                - 1;
        let baseline = state.project_baseline().unwrap().1;
        state.active_recipe_bytes = MAX_OWNED_BYTES - baseline - state.extra() - headroom;
        let original = state.draft.clone();
        let (_, largest) = crate::data::measure_test_allocations(|| state.load_selected_request());
        assert!(state.error().is_some());
        assert_eq!(state.draft, original);
        assert!(
            largest < destination,
            "copied request before conversion admission: {largest}"
        );
    }
    #[test]
    fn warm_regression_history_lookup_and_worker_refuse_before_request_or_draft_copy() {
        let data = Dataset::new(vec![
            Column::numeric(
                ColumnId(1),
                "y",
                vec![Some(2.), Some(4.), Some(5.), Some(8.)],
            )
            .unwrap(),
            Column::numeric(
                ColumnId(2),
                "x",
                vec![Some(1.), Some(2.), Some(3.), Some(4.)],
            )
            .unwrap(),
            Column::text(
                ColumnId(3),
                "g",
                vec![
                    Some("a".into()),
                    Some("b".into()),
                    Some("a".into()),
                    Some("b".into()),
                ],
            )
            .unwrap(),
        ])
        .unwrap();
        let request = AnalysisRequest::Regression(RegressionRequest {
            outcome: ColumnId(1),
            predictors: vec![RegressionPredictor {
                column: ColumnId(2),
                role: PredictorRole::Numeric,
            }],
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        });
        let result = analyze_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[],
            request.heap_bytes().unwrap(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(
            Project::new("Live budget", data)
                .unwrap()
                .with_run(result)
                .unwrap(),
        )
        .unwrap();
        state.act(UiAction::SelectRun(RunId(1)));
        state.load_selected_request();
        state.project_baseline().unwrap();
        let original = state.draft.clone();
        state.group_values = Some(std::sync::Arc::new(GroupLookupResult {
            column: ColumnId(3),
            role_epoch: 0,
            query: String::new(),
            choices: vec![GroupChoice {
                value: Cell::Text("z".repeat(10000)),
                label: "lookup".into(),
            }],
            selected_present: [false; 2],
            more: false,
        }));
        state.active_recipe_bytes = MAX_OWNED_BYTES;
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.checked_draft_request());
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
        let (result, largest) =
            crate::data::measure_test_allocations(|| state.checked_run_request(RunId(1)));
        assert!(matches!(result, Err(AppError::SizeLimit { .. })));
        assert_eq!(largest, 0);
        state.load_selected_request();
        assert_eq!(state.draft, original);
        assert_eq!(state.project().runs().len(), 1);
        assert!(state.error().is_some());
    }
}

#[cfg(test)]
mod anova_budget_tests {
    use super::*;
    use crate::{
        analysis::*,
        data::{Cell, Column, ColumnId, Dataset, MAX_OWNED_BYTES},
        operation::JobControl,
        project::Project,
    };
    fn state() -> DesktopState {
        let mut cols = vec![Column::numeric(
            ColumnId(1),
            "Subject",
            (0..8).map(|i| Some(i as f64)).collect(),
        )
        .unwrap()];
        for id in 2..=65 {
            cols.push(
                Column::numeric(
                    ColumnId(id),
                    &format!("M{id}"),
                    (0..8).map(|i| Some((i * id) as f64)).collect(),
                )
                .unwrap(),
            );
        }
        let data = Dataset::new(cols).unwrap();
        let request = AnalysisRequest::Anova(AnovaRequest {
            design: AnovaDesign::Repeated {
                subject: ColumnId(1),
                factors: vec![AnovaWithinFactor {
                    name: "Exact declared factor".into(),
                    levels: (0..64).map(|i| format!("Level {i}")).collect(),
                }],
                measurements: (2..=65).map(ColumnId).collect(),
            },
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
            planned_pairs: vec![],
            all_pairs: vec![],
        });
        let result = analyze_with_budget(
            &data,
            &request,
            &JobControl::default(),
            &[],
            std::mem::size_of::<AnalysisRequest>() + request.heap_bytes().unwrap(),
        )
        .unwrap();
        let mut s = DesktopState::from_project(
            Project::new("Conversion", data)
                .unwrap()
                .with_run(result)
                .unwrap(),
        )
        .unwrap();
        s.act(UiAction::SelectRun(RunId(1)));
        s
    }
    #[test]
    fn repeated_saved_conversion_precharges_destination_mapping_before_request_clone() {
        let mut state = state();
        let saved = state.selected_run().unwrap().analysis.request();
        let AnalysisRequestRef::Anova(r) = saved else {
            panic!()
        };
        let destination = super::super::draft::anova::AnovaDraft::conversion_bytes(r).unwrap();
        assert_eq!(destination, 64 * std::mem::size_of::<Option<ColumnId>>());
        let headroom =
            saved.heap_bytes().unwrap() + saved.validation_scratch_bytes().unwrap() + destination
                - 1;
        let baseline = state.project_baseline().unwrap().1;
        state.active_recipe_bytes = MAX_OWNED_BYTES - baseline - state.extra() - headroom;
        let original = state.draft.clone();
        let project = std::sync::Arc::clone(state.project());
        let (_, largest) = crate::data::measure_test_allocations(|| state.load_selected_request());
        assert!(state.error().is_some());
        assert_eq!(state.draft, original);
        assert!(
            largest < destination,
            "copied before destination-vector admission: {largest}"
        );
        assert!(std::sync::Arc::ptr_eq(&project, state.project()));
    }
    #[test]
    fn warm_anova_lookup_and_worker_reservations_refuse_run_rerun_and_load_before_copy() {
        let mut state = state();
        state.load_selected_request();
        state.project_baseline().unwrap();
        let original = state.draft.clone();
        let project = std::sync::Arc::clone(state.project());
        state.group_values = Some(std::sync::Arc::new(GroupLookupResult {
            column: ColumnId(1),
            role_epoch: 0,
            query: String::new(),
            choices: vec![GroupChoice {
                value: Cell::Text("z".repeat(10000)),
                label: "retained output".into(),
            }],
            selected_present: [false; 2],
            more: false,
        }));
        state.active_recipe_bytes = MAX_OWNED_BYTES;
        for saved in [false, true] {
            let (result, largest) = crate::data::measure_test_allocations(|| {
                if saved {
                    state.checked_run_request(RunId(1))
                } else {
                    state.checked_draft_request()
                }
            });
            assert!(matches!(result, Err(AppError::SizeLimit { .. })));
            assert_eq!(largest, 0);
        }
        state.load_selected_request();
        assert_eq!(state.draft, original);
        assert!(std::sync::Arc::ptr_eq(&project, state.project()));
        assert_eq!(state.project().runs().len(), 1);
    }
}
