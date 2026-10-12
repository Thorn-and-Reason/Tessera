//! Explicit review decisions and atomic preparation/report publication.
use super::*;
use crate::operation::JobControl;
use crate::survey::{
    cleaning::{CleaningPreview, CleaningRequest, CleaningSpec},
    recipe::SurveyRecipe,
    SurveyChoice, SurveyPresentation, SurveyRole,
};
use std::mem::size_of;

#[derive(Debug, Clone, Default)]
pub(super) struct SurveyEnhancements {
    pub(super) presentation: SurveyPresentation,
    order: Vec<usize>,
    source: Option<Arc<Project>>,
    cleanings: Vec<(ColumnId, ColumnId, CleaningSpec)>,
    pending: Option<Arc<CleaningRequest>>,
    preview: Option<Arc<CleaningPreview>>,
    recipe: Option<Arc<SurveyRecipe>>,
    draft: CleaningDraft,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleaningDraft {
    pub column: Option<ColumnId>,
    pub trim: bool,
    pub replacement: bool,
    pub from: String,
    pub to: String,
    pub missing: bool,
    pub rules: Vec<crate::preparation::RecodeRule>,
}
impl CleaningDraft {
    fn owned_bytes(&self) -> usize {
        self.from
            .capacity()
            .saturating_add(self.to.capacity())
            .saturating_add(
                self.rules
                    .capacity()
                    .saturating_mul(size_of::<crate::preparation::RecodeRule>()),
            )
            .saturating_add(
                self.rules
                    .iter()
                    .flat_map(|r| [&r.from, &r.to])
                    .map(|c| match c {
                        crate::data::Cell::Text(s) => s.capacity(),
                        _ => 0,
                    })
                    .fold(0usize, usize::saturating_add),
            )
    }
}
impl SurveyEnhancements {
    pub(super) fn owned_bytes(&self) -> usize {
        self.draft
            .owned_bytes()
            .saturating_add(self.order.capacity() * size_of::<usize>())
            .saturating_add(self.cleanings.capacity().saturating_mul(size_of::<(
                ColumnId,
                ColumnId,
                CleaningSpec,
            )>()))
            .saturating_add(
                self.cleanings
                    .iter()
                    .map(|(_, _, s)| s.owned_bytes().unwrap_or(usize::MAX))
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(self.source.as_ref().map_or(0, |p| {
                p.owned_bytes()
                    .unwrap_or(usize::MAX)
                    .saturating_add(Budget::dataset_bytes(&[p.dataset()], 0).unwrap_or(usize::MAX))
            }))
            .saturating_add(self.pending.as_ref().map_or(0, |r| {
                r.spec
                    .owned_bytes()
                    .unwrap_or(usize::MAX)
                    .saturating_add(r.name.capacity())
                    .saturating_add(size_of::<CleaningRequest>())
            }))
            .saturating_add(
                self.preview
                    .as_ref()
                    .map_or(0, |p| p.owned_bytes().unwrap_or(usize::MAX)),
            )
            .saturating_add(
                self.recipe
                    .as_ref()
                    .map_or(0, |r| r.owned_bytes().unwrap_or(usize::MAX)),
            )
            .saturating_add(size_of::<[usize; 8]>())
    }
}
impl DesktopState {
    pub fn survey_cleaning_draft(&self) -> &CleaningDraft {
        &self.survey.enhancements.draft
    }
    pub fn copy_survey_cleaning_draft(&self) -> AppResult<CleaningDraft> {
        self.check_ui_copy(
            self.survey
                .enhancements
                .draft
                .owned_bytes()
                .saturating_mul(2),
        )?;
        Ok(self.survey.enhancements.draft.clone())
    }
    pub fn set_survey_cleaning_draft(&mut self, draft: CleaningDraft) -> AppResult<()> {
        self.idle()?;
        if draft.from.len() > 4096
            || draft.to.len() > 4096
            || draft.rules.len() > crate::preparation::MAX_RECODE_RULES
        {
            return Err(AppError::invalid(
                "cleaning",
                "Replacement labels are limited to 4096 bytes.",
            ));
        }
        self.check_ui_copy(draft.owned_bytes().saturating_mul(2))?;
        if !draft.rules.is_empty() {
            CleaningSpec {
                trim_whitespace: draft.trim,
                rules: draft.rules.clone(),
            }
            .validate()?;
        }
        self.survey.enhancements.draft = draft;
        self.survey.enhancements.pending = None;
        self.survey.enhancements.preview = None;
        Ok(())
    }
    pub fn stage_survey_replacement(&mut self) -> AppResult<()> {
        let mut draft = self.copy_survey_cleaning_draft()?;
        if !draft.replacement || draft.rules.len() >= crate::preparation::MAX_RECODE_RULES {
            return Err(AppError::invalid(
                "cleaning",
                "Choose an exact replacement; at most 256 are supported.",
            ));
        }
        self.check_ui_copy(
            draft.owned_bytes().saturating_add(
                (draft.rules.len() + 1)
                    .saturating_mul(size_of::<crate::preparation::RecodeRule>())
                    .saturating_mul(2),
            ),
        )?;
        draft.rules.push(crate::preparation::RecodeRule {
            from: crate::data::Cell::Text(std::mem::take(&mut draft.from)),
            to: if draft.missing {
                crate::data::Cell::Missing
            } else {
                crate::data::Cell::Text(std::mem::take(&mut draft.to))
            },
        });
        draft.missing = false;
        draft.replacement = false;
        self.set_survey_cleaning_draft(draft)
    }
    pub fn remove_survey_replacement(&mut self, index: usize) -> AppResult<()> {
        let mut draft = self.copy_survey_cleaning_draft()?;
        if index >= draft.rules.len() {
            return Err(AppError::invalid(
                "cleaning",
                "That replacement no longer exists.",
            ));
        }
        draft.rules.remove(index);
        self.set_survey_cleaning_draft(draft)
    }
    pub fn preview_survey_cleaning_draft(&mut self, jobs: &mut JobRunner) -> AppResult<()> {
        let draft = &self.survey.enhancements.draft;
        let source = draft
            .column
            .ok_or_else(|| AppError::invalid("cleaning", "Choose a variable to prepare."))?;
        self.check_ui_copy(
            draft
                .owned_bytes()
                .saturating_mul(2)
                .saturating_add(size_of::<crate::preparation::RecodeRule>()),
        )?;
        let mut rules = draft.rules.clone();
        if draft.replacement {
            rules.push(crate::preparation::RecodeRule {
                from: crate::data::Cell::Text(draft.from.clone()),
                to: if draft.missing {
                    crate::data::Cell::Missing
                } else {
                    crate::data::Cell::Text(draft.to.clone())
                },
            });
        }
        let name = crate::survey::cleaning::reviewed_name(self.project.dataset(), source)?;
        self.preview_survey_cleaning(
            jobs,
            CleaningRequest {
                revision: self.project.dataset().revision(),
                source,
                name,
                spec: CleaningSpec {
                    trim_whitespace: draft.trim,
                    rules,
                },
            },
        )
    }
    pub(super) fn survey_review_order(&mut self) {
        self.survey.enhancements.order = (0..self.survey.choices.len())
            .filter(|&i| self.survey.choices[i].role == SurveyRole::Unresolved)
            .chain(
                (0..self.survey.choices.len())
                    .filter(|&i| self.survey.choices[i].role != SurveyRole::Unresolved),
            )
            .collect();
    }
    pub fn survey_order(&self) -> &[usize] {
        &self.survey.enhancements.order
    }
    pub fn survey_presentation(&self) -> SurveyPresentation {
        self.survey.enhancements.presentation
    }
    pub fn set_survey_presentation(&mut self, presentation: SurveyPresentation) -> AppResult<()> {
        self.idle()?;
        self.survey.enhancements.presentation = presentation;
        Ok(())
    }
    pub fn survey_cleaning_preview(&self) -> Option<&Arc<CleaningPreview>> {
        self.survey.enhancements.preview.as_ref()
    }
    pub fn survey_recipe(&self) -> Option<&Arc<SurveyRecipe>> {
        self.survey.enhancements.recipe.as_ref()
    }
    pub fn preview_survey_cleaning(
        &mut self,
        jobs: &mut JobRunner,
        request: CleaningRequest,
    ) -> AppResult<()> {
        self.idle()?;
        if self.survey.review.is_none()
            || request.revision != self.project.dataset().revision()
            || self
                .survey
                .enhancements
                .cleanings
                .iter()
                .any(|(source, derived, _)| *source == request.source || *derived == request.source)
        {
            return Err(AppError::invalid(
                "cleaning",
                "Review an original variable once before preparing it.",
            ));
        }
        request.spec.validate()?;
        self.check_ui_copy(
            request
                .spec
                .owned_bytes()?
                .saturating_add(request.name.capacity())
                .saturating_add(size_of::<CleaningRequest>()),
        )?;
        let pending = Arc::new(request.clone());
        self.start_operation(
            jobs,
            JobOperation::PreviewSurveyCleaning {
                project: Arc::clone(&self.project),
                request,
            },
        )?;
        self.survey.enhancements.pending = Some(pending);
        self.survey.enhancements.preview = None;
        Ok(())
    }
    pub(super) fn accept_survey_cleaning_preview(
        &mut self,
        preview: Arc<CleaningPreview>,
    ) -> AppResult<()> {
        let request = self
            .survey
            .enhancements
            .pending
            .as_ref()
            .ok_or_else(|| AppError::Conflict("The cleaning request expired.".into()))?;
        if preview.revision != self.project.dataset().revision()
            || preview.revision != request.revision
            || preview.source != request.source
        {
            return Err(AppError::Conflict(
                "The data changed. Preview again.".into(),
            ));
        }
        self.check_ui_copy(preview.owned_bytes()?)?;
        self.survey.enhancements.preview = Some(preview);
        Ok(())
    }
    pub fn apply_survey_cleaning(&mut self, jobs: &mut JobRunner) -> AppResult<()> {
        let request = self
            .survey
            .enhancements
            .pending
            .as_ref()
            .ok_or_else(|| AppError::invalid("cleaning", "Preview the changes first."))?;
        let preview = self
            .survey
            .enhancements
            .preview
            .as_ref()
            .ok_or_else(|| AppError::invalid("cleaning", "Preview the changes first."))?;
        if request.revision != self.project.dataset().revision()
            || preview.revision != request.revision
            || preview.source != request.source
        {
            return Err(AppError::Conflict(
                "The data changed. Preview again.".into(),
            ));
        }
        self.check_ui_copy(
            request
                .spec
                .owned_bytes()?
                .saturating_add(request.name.capacity()),
        )?;
        self.start_operation(
            jobs,
            JobOperation::ApplySurveyCleaning {
                project: Arc::clone(&self.project),
                request: (**request).clone(),
            },
        )
    }
    pub(super) fn accept_survey_cleaned(
        &mut self,
        project: Arc<Project>,
        request: CleaningRequest,
        review: Arc<crate::survey::SurveyReview>,
        control: &JobControl,
    ) -> AppResult<()> {
        control.check_cancelled()?;
        let expected_step = request.step()?;
        let pending = self
            .survey
            .enhancements
            .pending
            .as_ref()
            .ok_or_else(|| AppError::Conflict("The cleaning preview expired.".into()))?;
        if pending.source != request.source
            || pending.revision != request.revision
            || pending.name != request.name
            || pending.spec != request.spec
            || self.survey.enhancements.preview.is_none()
        {
            return Err(AppError::Conflict(
                "Preview the same changes before applying them.".into(),
            ));
        }
        if request.revision != self.project.dataset().revision()
            || project.dataset().column_count() != self.project.dataset().column_count() + 1
            || project.runs().len() != self.project.runs().len()
            || project.steps().len() != self.project.steps().len() + 1
            || !project
                .steps()
                .iter()
                .zip(self.project.steps())
                .all(|(a, b)| Arc::ptr_eq(a, b))
            || project.steps().last().is_none_or(|record| {
                record.source_revision != request.revision || record.step != expected_step
            })
            || !self
                .project
                .dataset()
                .columns()
                .iter()
                .zip(project.dataset().columns())
                .all(|(a, b)| {
                    a.id() == b.id() && a.metadata() == b.metadata() && a.shares_values_with(b)
                })
            || !project
                .runs()
                .iter()
                .zip(self.project.runs())
                .all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return Err(AppError::Conflict(
                "The prepared variable no longer matches this review.".into(),
            ));
        }
        self.check_ui_copy(
            self.survey_owned_bytes()
                .saturating_add(self.survey.choices.len() * size_of::<SurveyChoice>())
                .saturating_add(request.spec.owned_bytes()?),
        )?;
        let mut enhancements = self.survey.enhancements.clone();
        let old_choices = self.survey.choices.clone();
        enhancements
            .source
            .get_or_insert_with(|| Arc::clone(&self.project));
        let derived = project
            .dataset()
            .columns()
            .last()
            .ok_or(AppError::IdentityExhausted)?
            .id();
        enhancements
            .cleanings
            .push((request.source, derived, request.spec));
        enhancements.pending = None;
        enhancements.preview = None;
        let mut next = self.clone();
        next.adopt_prepared_project_arc(project)?;
        next.survey.enhancements = enhancements;
        // Reinspection recomputes missing counts; reviewed actions transfer only to the derived column.
        next.accept_survey_review(review)?;
        for choice in old_choices {
            let id = if choice.column == request.source {
                derived
            } else {
                choice.column
            };
            next.set_survey_role(id, choice.role)?;
        }
        next.set_survey_role(request.source, SurveyRole::Exclude)?;
        next.survey_review_order();
        next.check_ui_copy(0)?;
        control.check_cancelled()?;
        *self = next;
        Ok(())
    }
    pub fn save_survey_recipe(
        &mut self,
        jobs: &mut JobRunner,
        path: std::path::PathBuf,
    ) -> AppResult<()> {
        if !self.survey_ready() {
            return Err(AppError::invalid(
                "recipe",
                "Resolve the reporting actions before saving.",
            ));
        }
        self.check_ui_copy(
            self.survey_owned_bytes()
                .saturating_add(self.project.dataset().column_count() * size_of::<SurveyChoice>()),
        )?;
        let source = self
            .survey
            .enhancements
            .source
            .as_ref()
            .unwrap_or(&self.project);
        let choices = source
            .dataset()
            .columns()
            .iter()
            .map(|column| {
                let effective = self
                    .survey
                    .enhancements
                    .cleanings
                    .iter()
                    .find(|(id, _, _)| *id == column.id())
                    .map_or(column.id(), |(_, derived, _)| *derived);
                SurveyChoice {
                    column: column.id(),
                    role: self
                        .survey
                        .choices
                        .iter()
                        .find(|c| c.column == effective)
                        .map_or(SurveyRole::Unresolved, |c| c.role),
                }
            })
            .collect();
        let cleanings = self
            .survey
            .enhancements
            .cleanings
            .iter()
            .map(|(id, _, s)| (*id, s.clone()))
            .collect();
        self.start_operation(
            jobs,
            JobOperation::SaveSurveyRecipe {
                project: Arc::clone(&self.project),
                source: Arc::clone(source),
                choices,
                cleanings,
                presentation: self.survey.enhancements.presentation,
                path,
            },
        )
    }
    pub fn load_survey_recipe(
        &mut self,
        jobs: &mut JobRunner,
        path: std::path::PathBuf,
    ) -> AppResult<()> {
        self.start_operation(
            jobs,
            JobOperation::LoadSurveyRecipe {
                project: Arc::clone(&self.project),
                path,
            },
        )?;
        self.survey.enhancements.recipe = None;
        Ok(())
    }
    pub(super) fn accept_survey_recipe(&mut self, recipe: Arc<SurveyRecipe>) -> AppResult<()> {
        recipe.bind(self.project.dataset(), &JobControl::default())?;
        self.check_ui_copy(recipe.owned_bytes()?)?;
        self.survey.enhancements.recipe = Some(recipe);
        Ok(())
    }
    pub fn replay_survey_recipe(&mut self, jobs: &mut JobRunner) -> AppResult<()> {
        let recipe = self
            .survey
            .enhancements
            .recipe
            .as_ref()
            .ok_or_else(|| AppError::invalid("recipe", "Load a matching recipe first."))?;
        self.start_operation(
            jobs,
            JobOperation::ReplaySurveyRecipe {
                project: Arc::clone(&self.project),
                recipe: Arc::clone(recipe),
            },
        )
    }
    pub(super) fn accept_survey_recipe_report(
        &mut self,
        project: Arc<Project>,
        control: &JobControl,
    ) -> AppResult<()> {
        control.check_cancelled()?;
        let prior = self.project.runs();
        if project.runs().len() <= prior.len()
            || project.dataset().column_count() < self.project.dataset().column_count()
            || !self
                .project
                .dataset()
                .columns()
                .iter()
                .zip(project.dataset().columns())
                .all(|(a, b)| {
                    a.id() == b.id() && a.metadata() == b.metadata() && a.shares_values_with(b)
                })
            || !project
                .runs()
                .iter()
                .zip(prior)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return Err(AppError::Conflict(
                "The report no longer matches this project.".into(),
            ));
        }
        let runs: Vec<_> = project.runs()[prior.len()..].iter().map(|r| r.id).collect();
        if project.runs()[prior.len()..]
            .iter()
            .enumerate()
            .any(|(i, r)| {
                r.id.0 != self.project.next_run_id().0.saturating_add(i as u64)
                    || r.analysis.revision() != project.dataset().revision()
            })
        {
            return Err(AppError::Conflict("Invalid recipe report.".into()));
        }
        if std::ptr::eq(project.dataset(), self.project.dataset()) {
            return self.accept_survey_report(project, control);
        }
        self.check_ui_copy(runs.capacity() * size_of::<RunId>())?;
        let mut next = self.clone();
        next.adopt_prepared_project_arc(project)?;
        next.survey.runs = runs;
        next.selected_run = next.survey.runs.first().copied();
        next.prepare_chart_roles();
        next.workspace = Workspace::Results;
        next.status = "Survey report created.".into();
        control.check_cancelled()?;
        *self = next;
        Ok(())
    }
}
