//! Analysis-mode state helpers live here when they need to share private `DesktopState` fields.
use super::*;

impl DesktopState {
    pub fn start_fresh_analysis(&mut self) -> AppResult<()> {
        self.idle()?;
        let mut next = self.apply_action(UiAction::SetDraft(AnalysisDraft::fresh(
            self.draft.procedure(),
        )))?;
        next.search.clear();
        next.procedure_search.clear();
        next.picker = false;
        next.leave_survey_review();
        next = next.apply_action(UiAction::Navigate(Workspace::Analyses))?;
        *self = next;
        Ok(())
    }
    pub fn toggle_report_run(&mut self, id: RunId) -> AppResult<()> {
        self.idle()?;
        if !self.project.runs().iter().any(|r| r.id == id) {
            return Err(AppError::invalid(
                "report",
                "This recorded result no longer exists.",
            ));
        }
        let included = self.report_selection.contains(&id);
        let count = if included {
            self.report_selection.len() - 1
        } else {
            self.report_selection.len() + 1
        };
        self.check_ui_copy(count * size_of::<RunId>())?;
        let mut selection = Vec::with_capacity(count);
        selection.extend(
            self.report_selection
                .iter()
                .copied()
                .filter(|&run| run != id),
        );
        if !included {
            selection.push(id);
        }
        self.report_selection = selection;
        Ok(())
    }
}

impl DesktopState {
    pub fn report_run_ids(&self) -> &[RunId] {
        &self.report_selection
    }
    pub fn report_export_runs(&self) -> AppResult<Vec<RunId>> {
        if self.report_selection.is_empty()
            || self
                .report_selection
                .iter()
                .any(|id| !self.project.runs().iter().any(|r| r.id == *id))
        {
            return Err(AppError::invalid(
                "report",
                "Select recorded results to include in the report.",
            ));
        }
        self.check_ui_copy(self.report_selection.len() * size_of::<RunId>())?;
        let mut runs = Vec::with_capacity(self.report_selection.len());
        runs.extend(
            self.project
                .runs()
                .iter()
                .filter(|r| self.report_selection.contains(&r.id))
                .map(|r| r.id),
        );
        Ok(runs)
    }
}
