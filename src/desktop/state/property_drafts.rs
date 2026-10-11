use super::*;

impl DesktopState {
    pub fn metadata_has_changes(&self) -> bool {
        self.metadata_editor.as_ref().is_some_and(|editor| {
            self.project
                .dataset()
                .column(editor.column)
                .is_ok_and(|column| {
                    column.metadata() != &editor.metadata || column.kind() != editor.kind
                })
        })
    }
    pub fn selected_column(&self) -> Option<ColumnId> {
        self.selected_column
    }
    pub(crate) fn prepare_save(&mut self, save_as: bool) -> bool {
        if self.cell_editor.is_some() {
            self.act(UiAction::CommitCell);
            if self.error.is_some() {
                return false;
            }
        }
        if self.metadata_has_changes() {
            self.pending_save_as = Some(save_as);
            self.dialog = Some(Dialog::PropertyChanges);
            return false;
        }
        true
    }
    pub(crate) fn take_property_save_requested(&mut self) -> Option<bool> {
        if self.dialog.is_none() {
            self.pending_save_as.take()
        } else {
            None
        }
    }
    pub(super) fn resolve_properties(&self, choice: PropertyChoice) -> AppResult<Self> {
        if self.dialog != Some(Dialog::PropertyChanges) {
            return Ok(self.clone());
        }
        let mut next = match choice {
            PropertyChoice::Apply if self.metadata_has_changes() => {
                let editor = self
                    .metadata_editor
                    .as_ref()
                    .ok_or_else(|| AppError::invalid("properties", "Select a variable first."))?;
                if self.project.dataset().column(editor.column)?.kind() != editor.kind {
                    return Err(AppError::invalid("type", "Stay and convert the variable type before leaving, or discard the type choice."));
                }
                self.check_ui_copy(editor.metadata.heap_bytes()?)?;
                self.apply_action(UiAction::Metadata {
                    column: editor.column,
                    metadata: editor.metadata.clone(),
                })?
            }
            PropertyChoice::Apply | PropertyChoice::Discard | PropertyChoice::Stay => self.clone(),
        };
        next.dialog = None;
        if choice != PropertyChoice::Stay {
            next.metadata_editor = None;
            if let Some(id) = next.pending_column.take() {
                next = next.apply_action(UiAction::SelectColumn(id))?;
            }
            if let Some((column, row)) = next.pending_selection.take() {
                next = next.apply_action(UiAction::SelectCell { column, row })?;
            }
            if let Some(action) = next.pending_property_action.take() {
                next = next.apply_action(UiAction::Request(action))?;
            }
            if let Some((name, kind)) = next.pending_new_variable.take() {
                next = next.apply_action(UiAction::AddColumn { name, kind })?;
                next.new_name.clear();
            }
        }
        next.pending_column = None;
        next.pending_selection = None;
        next.pending_property_action = None;
        next.pending_new_variable = None;
        if choice == PropertyChoice::Stay {
            next.pending_save_as = None;
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;
    fn edited() -> DesktopState {
        let mut state = DesktopState::from_project(
            Project::new(
                "Draft",
                Dataset::new(vec![
                    Column::numeric(ColumnId(1), "First", vec![Some(1.)]).unwrap(),
                    Column::numeric(ColumnId(2), "Second", vec![Some(2.)]).unwrap(),
                ])
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
    fn selection_preserves_draft_until_apply_discard_or_stay() {
        let original = edited();
        let same = original
            .apply_action(UiAction::SelectColumn(ColumnId(1)))
            .unwrap();
        assert!(same.metadata_has_changes());
        assert_eq!(same.dialog(), None);
        let prompted = original
            .apply_action(UiAction::SelectCell {
                column: ColumnId(2),
                row: 0,
            })
            .unwrap();
        assert_eq!(prompted.dialog(), Some(Dialog::PropertyChanges));
        assert_eq!(prompted.selected_column(), Some(ColumnId(1)));
        let stayed = prompted
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Stay))
            .unwrap();
        assert!(stayed.metadata_has_changes());
        assert_eq!(stayed.selected_column(), Some(ColumnId(1)));
        let applied = prompted
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Apply))
            .unwrap();
        assert_eq!(
            applied
                .project
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .name(),
            "Edited"
        );
        assert_eq!(applied.selected_cell(), Some((ColumnId(2), 0)));
        let discarded = prompted
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Discard))
            .unwrap();
        assert_eq!(
            discarded
                .project
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .name(),
            "First"
        );
        assert_eq!(discarded.selected_column(), Some(ColumnId(2)));
        assert_eq!(
            original
                .project
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .name(),
            "First"
        );
    }
    #[test]
    fn invalid_apply_leaves_draft_and_pending_selection_intact() {
        let mut state = edited();
        state
            .metadata_editor
            .as_mut()
            .unwrap()
            .metadata
            .name
            .clear();
        let state = state
            .apply_action(UiAction::SelectColumn(ColumnId(2)))
            .unwrap();
        assert!(state
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Apply))
            .is_err());
        assert_eq!(state.dialog(), Some(Dialog::PropertyChanges));
        assert_eq!(state.pending_column, Some(ColumnId(2)));
        assert!(state.metadata_has_changes());
    }
    #[test]
    fn document_close_resolves_property_draft_before_unsaved_decision() {
        let state = edited()
            .apply_action(UiAction::Request(PendingAction::Close))
            .unwrap();
        assert_eq!(state.dialog(), Some(Dialog::PropertyChanges));
        assert!(state.pending().is_none());
        let state = state
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Apply))
            .unwrap();
        assert_eq!(state.dialog(), Some(Dialog::Unsaved));
        assert!(matches!(state.pending(), Some(PendingAction::Close)));
        assert_eq!(
            state.project.dataset().column(ColumnId(1)).unwrap().name(),
            "Edited"
        );
    }
    #[test]
    fn adding_variable_does_not_replace_unapplied_editor() {
        let state = edited()
            .apply_action(UiAction::AddColumn {
                name: "Third".into(),
                kind: ColumnKind::Text,
            })
            .unwrap();
        assert_eq!(state.project.dataset().column_count(), 2);
        assert_eq!(state.dialog(), Some(Dialog::PropertyChanges));
        let state = state
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Apply))
            .unwrap();
        assert_eq!(state.project.dataset().column_count(), 3);
        assert_eq!(
            state.project.dataset().column(ColumnId(1)).unwrap().name(),
            "Edited"
        );
        assert_eq!(state.selected_column(), Some(ColumnId(3)));
    }
    fn converting() -> DesktopState {
        let mut state = edited();
        let metadata = state
            .project
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .clone();
        state.metadata_editor = Some(MetadataEditor {
            column: ColumnId(1),
            metadata,
            kind: ColumnKind::Text,
        });
        state.active = true;
        state.generation = 1;
        state.operation_name = "Convert variable";
        state
    }
    #[test]
    fn completed_conversion_allows_pending_property_decision_to_resume_close() {
        let original = converting();
        let converted = original
            .project
            .dataset()
            .convert_column_with_budget(
                ColumnId(1),
                ColumnKind::Text,
                &crate::operation::JobControl::default(),
                &[],
                0,
            )
            .unwrap();
        let mut waiting = original
            .apply_action(UiAction::Request(PendingAction::Close))
            .unwrap();
        assert_eq!(waiting.dialog(), Some(Dialog::PropertyChanges));
        assert!(waiting.pending().is_none());
        waiting
            .handle_event(JobEvent::Completed {
                token: waiting.token(),
                cancelled: false,
                result: Ok(JobOutput::Converted(converted)),
            })
            .unwrap();
        assert!(waiting.metadata_editor.is_none());
        assert_eq!(
            waiting
                .project
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .kind(),
            ColumnKind::Text
        );
        for choice in [PropertyChoice::Apply, PropertyChoice::Discard] {
            let resumed = waiting
                .apply_action(UiAction::ResolveProperties(choice))
                .unwrap();
            assert_eq!(resumed.dialog(), Some(Dialog::Unsaved));
            assert!(matches!(resumed.pending(), Some(PendingAction::Close)));
            assert!(resumed.pending_property_action.is_none());
        }
        let stayed = waiting
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Stay))
            .unwrap();
        assert!(stayed.pending().is_none());
        assert!(stayed.pending_property_action.is_none());
        assert_eq!(stayed.dialog(), None);
    }
    #[test]
    fn conversion_start_refuses_unapplied_metadata_before_starting_worker() {
        let mut state = converting();
        state.active = false;
        let editor = state.metadata_editor.as_mut().unwrap();
        editor.metadata.name = "Edited name".into();
        editor.metadata.description = "Unapplied description".into();
        let mut jobs = JobRunner::default();
        let result = state.start_operation(
            &mut jobs,
            JobOperation::ConvertColumn {
                project: Arc::clone(state.project()),
                id: ColumnId(1),
                kind: ColumnKind::Text,
            },
        );
        assert!(
            matches!(result, Err(AppError::InvalidInput { .. })),
            "conversion must resolve metadata edits before starting"
        );
        assert!(!jobs.is_running());
        assert!(!state.active);
        let editor = state.metadata_editor.as_ref().unwrap();
        assert_eq!(editor.metadata.name, "Edited name");
        assert_eq!(editor.metadata.description, "Unapplied description");
        assert_eq!(editor.kind, ColumnKind::Text);
        assert_eq!(
            state.project.dataset().column(ColumnId(1)).unwrap().kind(),
            ColumnKind::Number
        );
    }
    #[test]
    fn applying_metadata_preserves_the_unapplied_type_choice() {
        let mut state = converting();
        state.active = false;
        let editor = state.metadata_editor.as_mut().unwrap();
        editor.metadata.name = "Edited name".into();
        let metadata = editor.metadata.clone();
        let after = state
            .apply_action(UiAction::Metadata {
                column: ColumnId(1),
                metadata,
            })
            .unwrap();
        assert_eq!(
            after.project.dataset().column(ColumnId(1)).unwrap().name(),
            "Edited name"
        );
        assert_eq!(
            after.project.dataset().column(ColumnId(1)).unwrap().kind(),
            ColumnKind::Number
        );
        let editor = after
            .metadata_editor
            .as_ref()
            .expect("Apply must retain the unapplied type choice");
        assert_eq!(editor.kind, ColumnKind::Text);
        assert_eq!(editor.metadata.name, "Edited name");
        assert!(after.metadata_has_changes());
    }
    #[test]
    fn conversion_start_refuses_another_variables_unapplied_type_choice() {
        let mut state = converting();
        state.active = false;
        let mut jobs = JobRunner::default();
        let result = state.start_operation(
            &mut jobs,
            JobOperation::ConvertColumn {
                project: Arc::clone(state.project()),
                id: ColumnId(2),
                kind: ColumnKind::Text,
            },
        );
        assert!(
            matches!(result, Err(AppError::InvalidInput { .. })),
            "conversion must resolve the other variable's type draft before starting"
        );
        assert!(!jobs.is_running());
        assert!(!state.active);
        assert_eq!(state.metadata_editor.as_ref().unwrap().column, ColumnId(1));
        assert_eq!(
            state.metadata_editor.as_ref().unwrap().kind,
            ColumnKind::Text
        );
        assert_eq!(
            state.project.dataset().column(ColumnId(2)).unwrap().kind(),
            ColumnKind::Number
        );
    }
    #[test]
    fn cancelled_conversion_retains_type_draft_and_pending_close() {
        let original = converting();
        let mut waiting = original
            .apply_action(UiAction::Request(PendingAction::Close))
            .unwrap();
        waiting
            .handle_event(JobEvent::Completed {
                token: waiting.token(),
                cancelled: true,
                result: Err(AppError::Cancelled),
            })
            .unwrap_err();
        assert!(waiting
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Apply))
            .is_err());
        assert_eq!(waiting.dialog(), Some(Dialog::PropertyChanges));
        assert!(matches!(
            waiting.pending_property_action,
            Some(PendingAction::Close)
        ));
        assert_eq!(
            waiting.metadata_editor.as_ref().unwrap().kind,
            ColumnKind::Text
        );
        assert_eq!(
            waiting
                .project
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .kind(),
            ColumnKind::Number
        );
    }
    #[test]
    fn save_waits_for_property_decision_and_stay_cancels_save() {
        let mut state = edited();
        assert!(!state.prepare_save(true));
        assert_eq!(state.take_property_save_requested(), None);
        let mut stayed = state
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Stay))
            .unwrap();
        assert_eq!(stayed.take_property_save_requested(), None);
        let mut applied = state
            .apply_action(UiAction::ResolveProperties(PropertyChoice::Apply))
            .unwrap();
        assert_eq!(applied.take_property_save_requested(), Some(true));
        assert_eq!(
            applied
                .project
                .dataset()
                .column(ColumnId(1))
                .unwrap()
                .name(),
            "Edited"
        );
    }
}
