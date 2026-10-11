#![cfg(feature = "desktop")]
use tessera::{
    data::{Cell, Column, ColumnId, ColumnKind, Dataset},
    desktop::{DesktopState, UiAction},
    project::Project,
};
fn state() -> DesktopState {
    DesktopState::from_project(
        Project::new(
            "Undo",
            Dataset::new(vec![
                Column::numeric(ColumnId(1), "Value", vec![Some(1.0)]).unwrap()
            ])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn edit(state: &DesktopState, value: f64) -> DesktopState {
    state
        .apply_action(UiAction::EditCell {
            column: ColumnId(1),
            row: 0,
            value: Cell::Number(value),
        })
        .unwrap()
}
fn value(state: &DesktopState) -> Cell {
    state
        .project()
        .dataset()
        .column(ColumnId(1))
        .unwrap()
        .cell(0)
        .unwrap()
}
#[test]
fn undo_redo_restores_contents_with_fresh_revision_and_branch_clears_redo() {
    let first = state();
    assert!(!first.can_undo());
    let second = edit(&first, 2.0);
    let revision = second.project().dataset().revision();
    let undone = second.apply_action(UiAction::Undo).unwrap();
    assert_eq!(value(&undone), Cell::Number(1.0));
    assert!(undone.project().dataset().revision() > revision);
    let redone = undone.apply_action(UiAction::Redo).unwrap();
    assert_eq!(value(&redone), Cell::Number(2.0));
    assert!(redone.project().dataset().revision() > undone.project().dataset().revision());
    assert!(!edit(&undone, 3.0).can_redo());
    assert_eq!(value(&second), Cell::Number(2.0));
}
#[test]
fn history_is_bounded_and_failed_edit_preserves_history() {
    let mut current = state();
    for i in 1..=40 {
        current = edit(&current, i as f64);
    }
    assert!(current
        .apply_action(UiAction::EditCell {
            column: ColumnId(999),
            row: 0,
            value: Cell::Missing
        })
        .is_err());
    let mut count = 0;
    while current.can_undo() {
        current = current.apply_action(UiAction::Undo).unwrap();
        count += 1;
    }
    assert_eq!(count, 32);
    assert_eq!(value(&current), Cell::Number(8.0));
}
#[test]
fn undone_column_identity_is_never_reused() {
    let first = state()
        .apply_action(UiAction::AddColumn {
            name: "Old".into(),
            kind: ColumnKind::Number,
        })
        .unwrap();
    let old = first.project().dataset().columns()[1].id();
    let undone = first.apply_action(UiAction::Undo).unwrap();
    let branched = undone
        .apply_action(UiAction::AddColumn {
            name: "New".into(),
            kind: ColumnKind::Number,
        })
        .unwrap();
    assert!(branched.project().dataset().columns()[1].id() > old);
}
#[test]
fn undo_keeps_analysis_and_run_watermark_and_results_stale() {
    use tessera::analysis::{describe, DescribeRequest, MissingPolicy, RowFilter, Statistic};
    use tessera::operation::JobControl;
    let initial = state();
    let result = describe(
        initial.project().dataset(),
        &DescribeRequest {
            columns: vec![ColumnId(1)],
            statistics: vec![Statistic::Mean],
            missing_policy: MissingPolicy::PerVariable,
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        &JobControl::default(),
    )
    .unwrap();
    let with_run = DesktopState::from_project(initial.project().with_run(result).unwrap()).unwrap();
    let run = with_run.project().runs()[0].id;
    let current = edit(&with_run, 2.0)
        .apply_action(UiAction::SelectRun(run))
        .unwrap();
    let undone = current.apply_action(UiAction::Undo).unwrap();
    assert_eq!(undone.project().runs().len(), 1);
    assert_eq!(
        undone.project().next_run_id(),
        current.project().next_run_id()
    );
    assert!(undone.selected_run_is_stale());
}
#[test]
fn unfinished_cell_editor_blocks_restore_without_losing_history() {
    let current = edit(&state(), 2.0)
        .apply_action(UiAction::BeginCell {
            column: ColumnId(1),
            row: 0,
        })
        .unwrap();
    assert!(!current.can_undo());
    assert!(current.apply_action(UiAction::Undo).is_err());
    let finished = current.apply_action(UiAction::CancelCell).unwrap();
    assert!(finished.can_undo());
}
#[test]
fn paste_row_column_and_metadata_changes_are_captured_once_each() {
    let initial = state()
        .apply_action(UiAction::SelectCell {
            column: ColumnId(1),
            row: 0,
        })
        .unwrap();
    let pasted = initial
        .apply_action(UiAction::PasteCells("7".into()))
        .unwrap();
    let restored = pasted.apply_action(UiAction::Undo).unwrap();
    assert_eq!(value(&restored), Cell::Number(1.0));
    assert!(!restored.can_undo());
    let row = restored.apply_action(UiAction::AppendRow).unwrap();
    assert_eq!(row.project().dataset().row_count(), 2);
    let restored = row.apply_action(UiAction::Undo).unwrap();
    assert_eq!(restored.project().dataset().row_count(), 1);
    assert!(!restored.can_undo());
    let mut metadata = restored
        .project()
        .dataset()
        .column(ColumnId(1))
        .unwrap()
        .metadata()
        .clone();
    metadata.description = "Updated".into();
    let updated = restored
        .apply_action(UiAction::Metadata {
            column: ColumnId(1),
            metadata,
        })
        .unwrap();
    let restored = updated.apply_action(UiAction::Undo).unwrap();
    assert_eq!(
        restored
            .project()
            .dataset()
            .column(ColumnId(1))
            .unwrap()
            .metadata()
            .description,
        ""
    );
    assert!(!restored.can_undo());
}
#[test]
fn exhausted_restore_preserves_history_and_active_contents() {
    use tessera::data::RevisionId;
    let initial = DesktopState::from_project(
        Project::new(
            "Limit",
            Dataset::from_revision(
                vec![Column::numeric(ColumnId(1), "Value", vec![Some(1.0)]).unwrap()],
                RevisionId(u64::MAX - 1),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let edited = edit(&initial, 2.0);
    assert!(edited.can_undo());
    assert!(edited.apply_action(UiAction::Undo).is_err());
    assert!(edited.can_undo());
    assert_eq!(value(&edited), Cell::Number(2.0));
}
