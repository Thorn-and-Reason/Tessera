#![cfg(feature = "desktop")]
use std::sync::Arc;
use tessera::{
    data::{Column, ColumnId, Dataset},
    desktop::{DesktopState, UiAction},
    jobs::{JobEvent, JobOutput},
    operation::JobControl,
    preparation::{self, Arithmetic, Operand, Step},
    project::Project,
};
fn state() -> DesktopState {
    DesktopState::from_project(
        Project::new(
            "Test",
            Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "X",
                vec![Some(2.), Some(4.)],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn compute(s: &DesktopState) -> Project {
    preparation::prepare_with_budget(
        s.project(),
        &Step::Compute {
            name: "Double".into(),
            left: Operand::Column(ColumnId(1)),
            operator: Arithmetic::Multiply,
            right: Operand::Constant(2.),
        },
        &JobControl::default(),
        &[],
        0,
    )
    .unwrap()
}
#[test]
fn prepared_publication_is_undoable_and_preserves_stale_result_contract() {
    let mut s = state();
    let token = s.token();
    let prepared = compute(&s);
    s.handle_event(JobEvent::Completed {
        token,
        cancelled: false,
        result: Ok(JobOutput::Prepared(Arc::new(prepared))),
    })
    .unwrap();
    assert_eq!(s.project().dataset().column_count(), 2);
    assert!(s.can_undo());
    let next = s.apply_action(UiAction::Undo).unwrap();
    assert_eq!(next.project().dataset().column_count(), 1);
    assert!(next.project().dataset().revision() > s.project().dataset().revision());
}
#[test]
fn stale_and_cancelled_preparation_never_change_current_data() {
    for cancelled in [false, true] {
        let mut s = state();
        let mut token = s.token();
        if !cancelled {
            token.generation += 1;
        }
        let prepared = compute(&s);
        let before = Arc::clone(s.project());
        s.handle_event(JobEvent::Completed {
            token,
            cancelled,
            result: Ok(JobOutput::Prepared(Arc::new(prepared))),
        })
        .unwrap();
        assert!(Arc::ptr_eq(&before, s.project()));
        assert!(!s.can_undo());
    }
}
#[test]
fn recovered_work_is_dirty_and_has_no_user_save_identity() {
    let mut s = state();
    let token = s.token();
    let recovered = compute(&s);
    s.handle_event(JobEvent::Completed {
        token,
        cancelled: false,
        result: Ok(JobOutput::Recovered(Arc::new(recovered))),
    })
    .unwrap();
    assert!(s.is_dirty());
    assert_eq!(s.project().dataset().column_count(), 2);
    assert!(!s.can_undo());
}
