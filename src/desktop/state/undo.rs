use super::*;
use crate::project::DataSnapshot;
const MAX_HISTORY: usize = 32;
#[cfg(test)]
thread_local! { static HISTORY_ALLOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
impl DesktopState {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() && self.history_controls_available()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty() && self.history_controls_available()
    }
    fn history_controls_available(&self) -> bool {
        !self.active
            && !self.modal_file_dialog
            && self.dialog.is_none()
            && self.cell_editor.is_none()
            && !self.metadata_has_changes()
    }
    pub(crate) fn adopt_prepared_project(&mut self, project: Project) -> AppResult<()> {
        self.adopt_prepared_project_arc(Arc::new(project))
    }
    pub(crate) fn adopt_prepared_project_arc(&mut self, project: Arc<Project>) -> AppResult<()> {
        self.ensure_frequency_invalidation()?;
        self.preflight_history_publication(&project, self.undo.len().min(MAX_HISTORY - 1) + 1)?;
        #[cfg(test)]
        HISTORY_ALLOCATIONS.with(|count| count.set(count.get() + 1));
        let snapshot = Arc::new(
            self.project
                .data_snapshot_with_budget(&[project.dataset()], self.extra())?,
        );
        let mut next = self.clone();
        next.redo.clear();
        let keep = self.undo.len().min(MAX_HISTORY - 1);
        let mut undo = Vec::with_capacity(keep + 1);
        undo.extend(self.undo.iter().skip(self.undo.len() - keep).cloned());
        undo.push(snapshot);
        next.undo = undo;
        next.project = project;
        next.invalidate_survey();
        // Old and new states coexist until immutable adoption completes.
        self.check_history_transition(&next)?;
        next.selected_cell = next.selected_cell.filter(|(id, row)| {
            *row < next.project.dataset().row_count() && next.project.dataset().column(*id).is_ok()
        });
        if next
            .selected_column
            .is_some_and(|id| next.project.dataset().column(id).is_err())
        {
            next.selected_column = next.project.dataset().columns().first().map(|c| c.id());
        }
        next.clear_lookup();
        next.clear_frequency_search()?;
        *self = next;
        Ok(())
    }
    pub(super) fn restore_history(&self, redo: bool) -> AppResult<Self> {
        self.idle()?;
        if !self.history_controls_available() {
            return Err(AppError::Conflict(
                "Finish editing before using undo or redo.".into(),
            ));
        }
        let source = if redo { &self.redo } else { &self.undo };
        let Some(snapshot) = source.last() else {
            self.check_ui_copy(self.history_copy_reservation(0))?;
            return Ok(self.clone());
        };
        let destination_len = if redo {
            self.undo.len() + 1
        } else {
            self.redo.len() + 1
        };
        let reservation = self.history_copy_reservation(destination_len);
        self.check_ui_copy(reservation)?;
        let control = crate::operation::JobControl::default();
        let target_bytes = Budget::dataset_bytes(&[self.project.dataset(), snapshot.dataset()], 0)?;
        let active_bytes = Budget::dataset_bytes(&[self.project.dataset()], 0)?;
        let extra = self
            .extra()
            .saturating_add(reservation)
            .checked_sub(target_bytes - active_bytes)
            .ok_or(AppError::IdentityExhausted)?;
        let project =
            self.project
                .with_restored_dataset_and_steps(snapshot, &control, &[], extra)?;
        self.preflight_history_publication(&project, destination_len)?;
        #[cfg(test)]
        HISTORY_ALLOCATIONS.with(|count| count.set(count.get() + 1));
        let current = Arc::new(
            self.project
                .data_snapshot_with_budget(&[project.dataset()], self.extra())?,
        );
        let mut next = self.clone();
        let destination = if redo { &self.undo } else { &self.redo };
        let mut restored = Vec::with_capacity(destination_len);
        restored.extend(destination.iter().cloned());
        restored.push(current);
        if redo {
            next.redo.pop();
            next.undo = restored;
        } else {
            next.undo.pop();
            next.redo = restored;
        }
        next.project = Arc::new(project);
        next.invalidate_survey();
        self.check_history_transition(&next)?;
        next.error = None;
        next.cell_editor = None;
        next.metadata_editor = None;
        next.selected_cell = None;
        next.selected_column = next.project.dataset().columns().first().map(|c| c.id());
        next.pending_column = None;
        next.pending_selection = None;
        next.column_widths
            .retain(|id, _| next.project.dataset().column(*id).is_ok());
        next.clear_lookup();
        next.clear_frequency_search()?;
        next.status = if redo { "Redone." } else { "Undone." }.into();
        Ok(next)
    }
    fn history_copy_reservation(&self, destination_len: usize) -> usize {
        self.copied_ui_bytes()
            .saturating_add(UI_WORK_BYTES)
            .saturating_add(crate::operation::JobControl::DEFAULT_HEAP_BYTES)
            .saturating_add(
                (self.undo.len() + self.redo.len() + destination_len)
                    .saturating_mul(std::mem::size_of::<Arc<DataSnapshot>>()),
            )
            .saturating_add(std::mem::size_of::<DataSnapshot>() + 2 * std::mem::size_of::<usize>())
            .saturating_add(
                self.project
                    .steps()
                    .len()
                    .saturating_mul(std::mem::size_of::<Arc<crate::project::RecordedStep>>()),
            )
    }
    fn preflight_history_publication(
        &self,
        project: &Project,
        destination_len: usize,
    ) -> AppResult<()> {
        let history_datasets = std::iter::once(self.project.dataset())
            .chain(self.undo.iter().chain(&self.redo).map(|s| s.dataset()));
        let history_data = Budget::dataset_bytes_iter(history_datasets, 0)?
            .saturating_sub(Budget::dataset_bytes(&[self.project.dataset()], 0)?);
        let extra = self
            .extra()
            .saturating_sub(history_data)
            .saturating_add(self.history_copy_reservation(destination_len));
        let metadata = Project::retained_owned_bytes(&[&self.project, project])?;
        Budget::dataset_bytes_iter(
            [self.project.dataset(), project.dataset()]
                .into_iter()
                .chain(self.undo.iter().chain(&self.redo).map(|s| s.dataset())),
            metadata.saturating_add(extra),
        )
        .map(|_| ())
    }

    fn check_history_transition(&self, next: &Self) -> AppResult<()> {
        let histories = self
            .undo
            .iter()
            .chain(&self.redo)
            .chain(&next.undo)
            .chain(&next.redo);
        let mut bytes = Project::retained_owned_bytes(&[&self.project, &next.project])?
            .saturating_add(self.extra().saturating_sub(self.history_bytes()?))
            .saturating_add(
                next.extra()
                    .saturating_sub(next.history_bytes()?)
                    .saturating_sub(next.recovery_owner_bytes())
                    .saturating_sub(self.shared_survey_review_bytes(next)),
            )
            .saturating_add(
                (self.undo.capacity()
                    + self.redo.capacity()
                    + next.undo.capacity()
                    + next.redo.capacity())
                    * std::mem::size_of::<Arc<DataSnapshot>>(),
            );
        for (index, snapshot) in histories.clone().enumerate() {
            if histories
                .clone()
                .take(index)
                .any(|s| Arc::ptr_eq(s, snapshot))
            {
                continue;
            }
            bytes = bytes.saturating_add(snapshot.owned_bytes()?);
            for (step_index, record) in snapshot.steps().iter().enumerate() {
                let active = [self.project.steps(), next.project.steps()]
                    .into_iter()
                    .any(|steps| {
                        steps
                            .get(step_index)
                            .is_some_and(|r| Arc::ptr_eq(r, record))
                    });
                let prior = histories.clone().take(index).any(|s| {
                    s.steps()
                        .get(step_index)
                        .is_some_and(|r| Arc::ptr_eq(r, record))
                });
                let local = snapshot.steps()[..step_index]
                    .iter()
                    .any(|r| Arc::ptr_eq(r, record));
                if !active && !prior && !local {
                    bytes = bytes.saturating_add(record.owned_bytes()?);
                }
            }
        }
        Budget::dataset_bytes_iter(
            [self.project.dataset(), next.project.dataset()]
                .into_iter()
                .chain(histories.map(|s| s.dataset())),
            bytes,
        )
        .map(|_| ())
    }
    pub(crate) fn history_bytes(&self) -> AppResult<usize> {
        if self.undo.is_empty() && self.redo.is_empty() {
            return Ok((self.undo.capacity() + self.redo.capacity())
                .saturating_mul(std::mem::size_of::<Arc<DataSnapshot>>()));
        }
        let datasets = std::iter::once(self.project.dataset())
            .chain(self.undo.iter().chain(&self.redo).map(|s| s.dataset()));
        let union = Budget::dataset_bytes_iter(datasets, 0)?;
        let active = Budget::dataset_bytes(&[self.project.dataset()], 0)?;
        let mut bytes = union
            .checked_sub(active)
            .ok_or(AppError::IdentityExhausted)?;
        bytes = bytes.saturating_add(
            (self.undo.capacity() + self.redo.capacity())
                * std::mem::size_of::<Arc<DataSnapshot>>(),
        );
        for (index, snapshot) in self.undo.iter().chain(&self.redo).enumerate() {
            bytes = bytes.saturating_add(snapshot.owned_bytes()?);
            for (step_index, record) in snapshot.steps().iter().enumerate() {
                let in_active = self
                    .project
                    .steps()
                    .get(step_index)
                    .is_some_and(|r| Arc::ptr_eq(r, record));
                let in_prior = self.undo.iter().chain(&self.redo).take(index).any(|s| {
                    s.steps()
                        .get(step_index)
                        .is_some_and(|r| Arc::ptr_eq(r, record))
                });
                let in_snapshot = snapshot.steps()[..step_index]
                    .iter()
                    .any(|r| Arc::ptr_eq(r, record));
                if !in_active && !in_prior && !in_snapshot {
                    bytes = bytes.saturating_add(record.owned_bytes()?);
                }
            }
        }
        Ok(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peak_limit_refuses_before_history_snapshot_and_state_copy() {
        let mut state = DesktopState::new().unwrap();
        state.new_name = "x".repeat(8192);
        let candidate = Arc::clone(&state.project);
        let (_, baseline) = state.project_baseline().unwrap();
        let live = baseline + state.extra();
        state.active_recipe_bytes = crate::data::MAX_OWNED_BYTES - live - 64;
        state.check_ui_copy(0).unwrap();
        HISTORY_ALLOCATIONS.with(|count| count.set(0));
        assert!(matches!(
            state.adopt_prepared_project_arc(candidate),
            Err(AppError::SizeLimit { .. })
        ));
        HISTORY_ALLOCATIONS.with(|count| assert_eq!(count.get(), 0));
        assert!(state.undo.is_empty());
        assert_eq!(state.new_name.len(), 8192);
    }
    #[test]
    fn shared_snapshot_dataset_is_counted_once() {
        let dataset = Dataset::new(vec![crate::data::Column::text(
            ColumnId(1),
            "Text",
            vec![Some("x".repeat(65536))],
        )
        .unwrap()])
        .unwrap();
        let mut state =
            DesktopState::from_project(Project::new("Budget", dataset).unwrap()).unwrap();
        let snapshot = Arc::new(state.project.data_snapshot_with_budget(&[], 0).unwrap());
        state.undo.push(snapshot);
        assert!(state.history_bytes().unwrap() < 1024);
        let cloned = state.clone();
        assert!(cloned.history_bytes().unwrap() < 1024);
        assert_eq!(
            cloned.extra() - cloned.history_bytes().unwrap(),
            state.extra() - state.history_bytes().unwrap()
        );
    }
}
