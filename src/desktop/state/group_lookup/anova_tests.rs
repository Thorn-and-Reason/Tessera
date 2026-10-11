use super::*;
use crate::{analysis::*, desktop::draft::anova::*};
fn state() -> DesktopState {
    let project = crate::project_io::decode(
        &include_bytes!("../../../../examples/mixed-analysis.tessera")[..],
        &crate::operation::JobControl::default(),
    )
    .unwrap();
    let mut s = DesktopState::from_project(project).unwrap();
    s.set_draft_reference(
        AnalysisDraft::Anova(AnovaDraft {
            design: AnovaDesignDraft::Factorial {
                outcome: Some(ColumnId(1)),
                factors: vec![ColumnId(2), ColumnId(3)],
            },
            comparison_factor: 1,
            endpoints: [
                Some(Cell::Text("absent".into())),
                Some(Cell::Text("other".into())),
            ],
            ..Default::default()
        }),
        Some(ColumnId(3)),
    )
    .unwrap();
    s
}
#[test]
fn anova_active_role_exhaustion_and_removed_factor_refuse_atomically() {
    let mut s = state();
    let original = s.draft.clone();
    s.lookup.epoch = u64::MAX;
    let (result, largest) = crate::data::measure_test_allocations(|| {
        s.set_active_regression_reference(Some(ColumnId(2)))
    });
    assert!(matches!(result, Err(AppError::IdentityExhausted)));
    assert_eq!(largest, 0);
    assert_eq!(s.draft, original);
    assert_eq!(s.active_regression_reference, Some(ColumnId(3)));
    let mut changed = original.clone();
    let AnalysisDraft::Anova(d) = &mut changed else {
        panic!()
    };
    let AnovaDesignDraft::Factorial { factors, .. } = &mut d.design else {
        panic!()
    };
    factors.pop();
    let (result, largest) =
        crate::data::measure_test_allocations(|| s.apply_action(UiAction::SetDraft(changed)));
    assert!(matches!(result, Err(AppError::IdentityExhausted)));
    assert_eq!(largest, 0);
    assert_eq!(s.draft, original);
}
#[test]
fn anova_late_lookup_rejects_document_revision_generation_epoch_query_and_role() {
    for stale in 0..6 {
        let mut s = state();
        let token = s.token();
        let request = Arc::new(s.prepare_lookup(ColumnId(3)).unwrap());
        s.lookup.running = Some((token, Arc::clone(&request)));
        s.active = true;
        s.active_recipe_bytes = 9000;
        let mut output = GroupLookupResult {
            column: request.column,
            role_epoch: request.role_epoch,
            query: request.query.clone(),
            choices: vec![],
            selected_present: [false; 2],
            more: false,
        };
        match stale {
            0 => s.document += 1,
            1 => s.generation += 1,
            2 => {
                let data = s
                    .project
                    .dataset()
                    .edit_cell(ColumnId(1), 0, Cell::Number(-99.))
                    .unwrap();
                s.project = Arc::new(s.project.with_dataset(data).unwrap());
            }
            3 => output.role_epoch += 1,
            4 => output.query = "late query".into(),
            _ => output.column = ColumnId(2),
        }
        let current_token = s.token();
        let project = Arc::clone(s.project());
        s.handle_event(JobEvent::Completed {
            token,
            cancelled: false,
            result: Ok(JobOutput::GroupValues(Arc::new(output))),
        })
        .unwrap();
        assert!(s.group_lookup().is_none());
        assert!(Arc::ptr_eq(&project, s.project()));
        assert_eq!(
            s.active_recipe_bytes,
            if current_token == token { 0 } else { 9000 }
        );
    }
}
#[test]
fn anova_current_lookup_and_distinct_retained_worker_output_are_charged_once() {
    let mut s = state();
    let token = s.token();
    let request = Arc::new(s.prepare_lookup(ColumnId(3)).unwrap());
    s.lookup.running = Some((token, Arc::clone(&request)));
    let old = Arc::new(GroupLookupResult {
        column: request.column,
        role_epoch: request.role_epoch,
        query: request.query.clone(),
        choices: vec![GroupChoice {
            value: Cell::Text("old".repeat(1000)),
            label: "old label".into(),
        }],
        selected_present: [false; 2],
        more: false,
    });
    s.accept_lookup(token, Arc::clone(&old)).unwrap();
    assert!(s.group_lookup().is_some());
    s.capture_retained_lookup();
    let baseline = s.extra() - s.retained_ui_bytes();
    let old_bytes = old.owned_bytes().unwrap() + 2 * std::mem::size_of::<usize>();
    s.set_group_query("new".into()).unwrap();
    assert_eq!(s.extra(), baseline + s.retained_ui_bytes());
    let new = Arc::new(GroupLookupResult {
        query: "new".into(),
        choices: vec![GroupChoice {
            value: Cell::Text("new".repeat(2000)),
            label: "new label".into(),
        }],
        ..(*old).clone()
    });
    let new_bytes = new.owned_bytes().unwrap() + 2 * std::mem::size_of::<usize>();
    s.group_values = Some(new);
    assert_eq!(s.extra(), baseline + s.retained_ui_bytes() + new_bytes);
    s.release_retained_lookup();
    assert_eq!(
        s.extra(),
        baseline + s.retained_ui_bytes() + new_bytes - old_bytes
    );
}
