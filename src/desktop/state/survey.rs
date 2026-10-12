//! Revision-bound survey navigation and complete-batch publication.
use super::*;
use crate::operation::JobControl;
use crate::survey::{SurveyChoice, SurveyRequest, SurveyReview, SurveyRole};
use std::mem::size_of;

#[derive(Debug, Clone, Default)]
pub(super) struct SurveyState {
    pub(super) review: Option<Arc<SurveyReview>>,
    pub(super) choices: Vec<SurveyChoice>,
    pub(super) runs: Vec<RunId>,
    pub(super) enhancements: super::survey_improvements::SurveyEnhancements,
    pub(super) entry_pending: bool,
    pub(super) import_requested: bool,
    import_picker_pending: bool,
}
impl DesktopState {
    /// Resolve existing edit guards; the next idle poll starts inspection or requests import.
    pub fn enter_survey(&mut self) -> AppResult<()> {
        self.idle()?;
        let mut next = self.apply_action(UiAction::Navigate(Workspace::Analyses))?;
        if next.metadata_has_changes() {
            next.pending_workspace = Some(Workspace::Analyses);
            next.dialog = Some(Dialog::PropertyChanges);
        }
        next.survey.entry_pending = true;
        if next.dialog.is_none() && next.project.dataset().column_count() == 0 {
            next.survey.entry_pending = false;
            next.survey.import_requested = true;
            next.survey.import_picker_pending = true;
        }
        *self = next;
        Ok(())
    }
    pub fn service_survey(&mut self, jobs: &mut JobRunner) -> AppResult<()> {
        if !self.survey.entry_pending
            || self.active
            || self.dialog.is_some()
            || self.modal_file_dialog
            || self.pending.is_some()
        {
            return Ok(());
        }
        if self.cell_editor.is_some() || self.metadata_has_changes() {
            return Ok(());
        }
        if self.project.dataset().column_count() == 0 {
            self.survey.entry_pending = false;
            self.survey.import_requested = true;
            self.survey.import_picker_pending = true;
            return Ok(());
        }
        self.survey.entry_pending = false;
        self.start_operation(
            jobs,
            JobOperation::InspectSurvey {
                project: Arc::clone(&self.project),
            },
        )?;
        Ok(())
    }
    pub fn survey_visible(&self) -> bool {
        self.survey.review.is_some() || self.survey.entry_pending || self.survey.import_requested
    }
    pub fn survey_import_requested(&self) -> bool {
        self.survey.import_requested
    }
    /// Consume the picker action while preserving continuation through preview and confirmation.
    pub fn take_survey_import_picker(&mut self) -> bool {
        std::mem::take(&mut self.survey.import_picker_pending)
    }
    pub fn leave_survey_review(&mut self) {
        self.survey.review = None;
        self.survey.choices = Vec::new();
        self.survey.entry_pending = false;
        self.survey.enhancements = Default::default();
    }
    pub fn survey_export_runs(&self) -> AppResult<Vec<RunId>> {
        if self.survey.runs.is_empty()
            || self.survey.runs.iter().any(|id| {
                !self.project.runs().iter().any(|run| {
                    run.id == *id && run.analysis.revision() == self.project.dataset().revision()
                })
            })
        {
            return Err(AppError::invalid(
                "survey report",
                "Create a survey report before exporting it.",
            ));
        }
        self.check_ui_copy(self.survey.runs.len() * size_of::<RunId>())?;
        Ok(self.survey.runs.clone())
    }
    /// Call when an import picker/preview is cancelled, or its replacement guard chooses Stay.
    pub fn cancel_survey_import(&mut self) {
        self.survey.import_requested = false;
        self.survey.import_picker_pending = false;
        self.survey.entry_pending = false;
    }
    pub fn survey_review(&self) -> Option<&Arc<SurveyReview>> {
        self.survey.review.as_ref()
    }
    pub fn survey_choices(&self) -> &[SurveyChoice] {
        &self.survey.choices
    }
    pub fn survey_run_ids(&self) -> &[RunId] {
        &self.survey.runs
    }
    /// Cheap readiness from worker findings and UI-owned roles; no response scan.
    pub fn survey_ready(&self) -> bool {
        !self.active
            && !self.modal_file_dialog
            && self.dialog.is_none()
            && self.pending.is_none()
            && self.cell_editor.is_none()
            && !self.metadata_has_changes()
            && self
                .survey
                .review
                .as_ref()
                .is_some_and(|review| review.revision == self.project.dataset().revision())
            && !self
                .survey
                .choices
                .iter()
                .any(|c| c.role == SurveyRole::Unresolved)
            && self
                .survey
                .choices
                .iter()
                .any(|c| matches!(c.role, SurveyRole::Categories | SurveyRole::NumericMeasure))
    }
    pub fn set_survey_role(&mut self, column: ColumnId, role: SurveyRole) -> AppResult<()> {
        self.idle()?;
        let review = self
            .survey
            .review
            .as_ref()
            .ok_or_else(|| AppError::invalid("survey", "Review the variables again."))?;
        if review.revision != self.project.dataset().revision() {
            return Err(AppError::Conflict(
                "The data changed. Review the variables again.".into(),
            ));
        }
        if role == SurveyRole::NumericMeasure
            && self.project.dataset().column(column)?.kind() != ColumnKind::Number
        {
            return Err(AppError::invalid(
                "survey",
                "Numeric measures require a numeric variable.",
            ));
        }
        let index = self
            .survey
            .choices
            .iter()
            .position(|c| c.column == column)
            .ok_or_else(|| AppError::invalid("survey", "The variable is not in this review."))?;
        self.check_ui_copy(self.survey.choices.len() * size_of::<SurveyChoice>())?;
        let mut choices = self.survey.choices.clone();
        choices[index].role = role;
        self.survey.choices = choices;
        Ok(())
    }
    pub fn submit_survey(&mut self, jobs: &mut JobRunner) -> AppResult<()> {
        if !self.survey_ready() {
            return Err(AppError::invalid("survey", "Choose a reporting role or exclude each variable, and include at least one variable."));
        }
        self.check_ui_copy(
            size_of::<SurveyRequest>() + self.survey.choices.len() * size_of::<SurveyChoice>(),
        )?;
        let request = SurveyRequest {
            revision: self.project.dataset().revision(),
            choices: self.survey.choices.clone(),
        };
        self.start_operation(
            jobs,
            JobOperation::SurveyPresented {
                project: Arc::clone(&self.project),
                request,
                presentation: self.survey.enhancements.presentation,
            },
        )
    }
    pub(super) fn invalidate_survey(&mut self) {
        self.survey.review = None;
        self.survey.choices = Vec::new();
        self.survey.runs = Vec::new();
        self.survey.enhancements = Default::default();
    }
    pub(super) fn survey_review_bytes(&self) -> usize {
        self.survey.review.as_ref().map_or(0, |r| {
            r.owned_bytes()
                .unwrap_or(usize::MAX)
                .saturating_add(2 * size_of::<usize>())
        })
    }
    pub(super) fn shared_survey_review_bytes(&self, next: &Self) -> usize {
        match (&self.survey.review, &next.survey.review) {
            (Some(a), Some(b)) if Arc::ptr_eq(a, b) => self.survey_review_bytes(),
            _ => 0,
        }
    }
    pub(super) fn survey_owned_bytes(&self) -> usize {
        self.survey_review_bytes()
            .saturating_add(self.survey.choices.capacity() * size_of::<SurveyChoice>())
            .saturating_add(self.survey.runs.capacity() * size_of::<RunId>())
            .saturating_add(self.survey.enhancements.owned_bytes())
    }
    pub(super) fn accept_survey_review(&mut self, review: Arc<SurveyReview>) -> AppResult<()> {
        if review.revision != self.project.dataset().revision() {
            return Err(AppError::Conflict(
                "The data changed. Review the variables again.".into(),
            ));
        }
        self.check_ui_copy(
            review
                .owned_bytes()?
                .saturating_add(2 * size_of::<usize>())
                .saturating_add(
                    review.columns.len() * (size_of::<SurveyChoice>() + size_of::<usize>()),
                ),
        )?;
        let choices = review
            .columns
            .iter()
            .map(|c| SurveyChoice {
                column: c.column,
                role: c.suggested_role,
            })
            .collect();
        self.survey.review = Some(review);
        self.survey.choices = choices;
        self.survey_review_order();
        self.workspace = Workspace::Analyses;
        self.status.clear();
        Ok(())
    }
    pub(super) fn accept_survey_report(
        &mut self,
        candidate: Arc<Project>,
        control: &JobControl,
    ) -> AppResult<()> {
        control.check_cancelled()?;
        let prior = self.project.runs();
        if !std::ptr::eq(candidate.dataset(), self.project.dataset())
            || candidate.dataset().revision() != self.project.dataset().revision()
            || candidate.runs().len() <= prior.len()
            || !candidate
                .runs()
                .iter()
                .zip(prior)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return Err(AppError::Conflict(
                "The report does not match the current project. Review the variables again.".into(),
            ));
        }
        let suffix = &candidate.runs()[prior.len()..];
        if suffix.iter().enumerate().any(|(index, r)| {
            r.id.0 != self.project.next_run_id().0.saturating_add(index as u64)
                || r.analysis.revision() != self.project.dataset().revision()
        }) {
            return Err(AppError::Conflict(
                "The report contains an invalid saved analysis.".into(),
            ));
        }
        let bytes = Project::retained_owned_bytes_iter_controlled(
            [self.project.as_ref(), candidate.as_ref()].into_iter(),
            Some(control),
        )?;
        Budget::check_datasets(
            &[self.project.dataset()],
            bytes
                .saturating_add(self.extra())
                .saturating_add(suffix.len() * size_of::<RunId>()),
        )?;
        self.ensure_frequency_invalidation()?;
        let runs: Vec<_> = suffix.iter().map(|r| r.id).collect();
        self.clear_frequency_search()?;
        self.selected_run = runs.first().copied();
        self.project = candidate;
        self.survey.runs = runs;
        self.prepare_chart_roles();
        self.workspace = Workspace::Results;
        self.status = "Survey report created.".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edited() -> DesktopState {
        let mut state = DesktopState::from_project(
            Project::new(
                "Survey",
                Dataset::new(vec![crate::data::Column::numeric(
                    ColumnId(1),
                    "Value",
                    vec![Some(1.)],
                )
                .unwrap()])
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let mut metadata = state
            .project
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .clone();
        metadata.name = "Edited".into();
        state.metadata_editor = Some(MetadataEditor {
            column: ColumnId(1),
            metadata,
            kind: ColumnKind::Number,
        });
        state
    }
    #[test]
    fn survey_property_apply_discard_resume_and_stay_stops() {
        for choice in [
            PropertyChoice::Apply,
            PropertyChoice::Discard,
            PropertyChoice::Stay,
        ] {
            let mut state = edited();
            let before = Arc::clone(&state.project);
            let mut jobs = JobRunner::default();
            state.enter_survey().unwrap();
            assert_eq!(state.dialog, Some(Dialog::PropertyChanges));
            state.service_survey(&mut jobs).unwrap();
            assert!(!jobs.is_running());
            assert!(state.pending.is_none());
            assert!(state
                .start_operation(
                    &mut jobs,
                    JobOperation::InspectSurvey {
                        project: Arc::clone(&state.project)
                    }
                )
                .is_err());
            state = state
                .apply_action(UiAction::ResolveProperties(choice))
                .unwrap();
            state.service_survey(&mut jobs).unwrap();
            assert_eq!(jobs.is_running(), choice != PropertyChoice::Stay);
            if choice == PropertyChoice::Stay {
                assert!(state.metadata_has_changes());
                assert!(Arc::ptr_eq(&before, &state.project));
            } else {
                assert!(!state.metadata_has_changes());
                assert_eq!(
                    state.project.dataset().column(ColumnId(1)).unwrap().name(),
                    if choice == PropertyChoice::Apply {
                        "Edited"
                    } else {
                        "Value"
                    }
                );
            }
            jobs.cancel();
        }
    }
    #[test]
    fn survey_ui_copy_counts_shared_review_once_and_owned_vectors() {
        let mut state = edited();
        state.metadata_editor = None;
        let review = Arc::new(
            crate::survey::inspect_with_budget(
                state.project.dataset(),
                &JobControl::default(),
                &[],
                0,
            )
            .unwrap(),
        );
        state.accept_survey_review(Arc::clone(&review)).unwrap();
        let next = state.clone();
        assert!(Arc::ptr_eq(next.survey_review().unwrap(), &review));
        assert_eq!(
            state.shared_survey_review_bytes(&next),
            state.survey_review_bytes()
        );
        assert_eq!(
            state.retained_ui_bytes() - state.copied_ui_bytes(),
            state.survey_review_bytes()
        );
        let (_, baseline) = state.project_baseline().unwrap();
        let live = baseline + state.extra();
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES - live;
        let before = state.survey.choices.clone();
        assert!(matches!(
            state.set_survey_role(ColumnId(1), SurveyRole::Categories),
            Err(AppError::SizeLimit { .. })
        ));
        assert_eq!(state.survey.choices, before);
    }
    #[test]
    fn survey_publication_budget_failure_preserves_project_and_selection() {
        let mut state = edited();
        state.metadata_editor = None;
        let request = SurveyRequest {
            revision: state.project.dataset().revision(),
            choices: vec![SurveyChoice {
                column: ColumnId(1),
                role: SurveyRole::NumericMeasure,
            }],
        };
        let candidate = Arc::new(
            crate::survey::generate_with_budget(
                &state.project,
                &request,
                &JobControl::default(),
                &[],
                0,
            )
            .unwrap(),
        );
        let before = Arc::clone(&state.project);
        let (_, baseline) = state.project_baseline().unwrap();
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES - baseline - state.extra();
        assert!(matches!(
            state.accept_survey_report(candidate, &JobControl::default()),
            Err(AppError::SizeLimit { .. })
        ));
        assert!(Arc::ptr_eq(&before, &state.project));
        assert!(state.survey.runs.is_empty());
        assert!(state.selected_run.is_none());
    }
    #[test]
    fn survey_publication_rejects_prior_prefix_even_with_shared_dataset() {
        let mut state = edited();
        state.metadata_editor = None;
        let request = SurveyRequest {
            revision: state.project.dataset().revision(),
            choices: vec![SurveyChoice {
                column: ColumnId(1),
                role: SurveyRole::NumericMeasure,
            }],
        };
        let base = Arc::clone(&state.project);
        let candidate = crate::survey::generate_with_budget(
            &state.project,
            &request,
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        state
            .accept_survey_report(Arc::new(candidate), &JobControl::default())
            .unwrap();
        let before = Arc::clone(&state.project);
        // Rebuilt runs have identical IDs/results, but must not substitute prior Arc owners.
        let rebuilt =
            crate::survey::generate_with_budget(&base, &request, &JobControl::default(), &[], 0)
                .unwrap();
        let rebuilt =
            crate::survey::generate_with_budget(&rebuilt, &request, &JobControl::default(), &[], 0)
                .unwrap();
        assert!(std::ptr::eq(rebuilt.dataset(), before.dataset()));
        assert!(rebuilt.runs().len() > before.runs().len());
        assert!(state
            .accept_survey_report(Arc::new(rebuilt), &JobControl::default())
            .is_err());
        assert!(Arc::ptr_eq(&before, &state.project));
    }
}
