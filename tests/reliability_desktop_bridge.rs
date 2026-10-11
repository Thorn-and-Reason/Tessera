#![cfg(feature = "desktop")]
use tessera::{
    analysis::*,
    data::{Column, ColumnId, Dataset},
    desktop::{
        draft::{AnalysisDraft, Procedure},
        DesktopState,
    },
    operation::JobControl,
    project::Project,
};
#[test]
fn reliability_saved_request_uses_concrete_draft_with_chooser_exposure() {
    let data = Dataset::new(vec![
        Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(2.), Some(3.)]).unwrap(),
        Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(4.), Some(6.)]).unwrap(),
    ])
    .unwrap();
    let request = AnalysisRequest::Reliability(ReliabilityRequest {
        items: vec![ColumnId(1), ColumnId(2)],
        filter: RowFilter::default(),
        frequency_weight: None,
        reversals: vec![],
    });
    let draft = AnalysisDraft::from_request(request.clone());
    assert_eq!(draft.procedure(), Procedure::Reliability);
    assert!(Procedure::ALL.contains(&Procedure::Reliability));
    assert!(draft.matches_saved(request.borrowed()));
    assert_eq!(draft.build_request(&data).unwrap(), request);
    let c = JobControl::default();
    let a = analyze_with_budget(
        &data,
        &request,
        &c,
        &[],
        std::mem::size_of::<AnalysisRequest>() + request.heap_bytes().unwrap(),
    )
    .unwrap();
    let p = Project::new("History", data).unwrap().with_run(a).unwrap();
    let state = DesktopState::from_project(p).unwrap();
    assert_eq!(
        state
            .checked_run_request(tessera::operation::RunId(1))
            .unwrap(),
        request
    );
}
#[test]
fn stale_and_cancelled_reliability_terminal_events_cannot_publish() {
    for stale in [false, true] {
        let data = Dataset::new(vec![
            Column::numeric(ColumnId(1), "First", vec![Some(1.), Some(2.)]).unwrap(),
            Column::numeric(ColumnId(2), "Second", vec![Some(2.), Some(4.)]).unwrap(),
        ])
        .unwrap();
        let r = AnalysisRequest::Reliability(ReliabilityRequest {
            items: vec![ColumnId(1), ColumnId(2)],
            filter: RowFilter::default(),
            frequency_weight: None,
            reversals: vec![],
        });
        let a = analyze_with_budget(
            &data,
            &r,
            &JobControl::default(),
            &[],
            std::mem::size_of::<AnalysisRequest>() + r.heap_bytes().unwrap(),
        )
        .unwrap();
        let mut state = DesktopState::from_project(Project::new("Events", data).unwrap()).unwrap();
        let mut token = state.token();
        if stale {
            token.generation += 1;
        }
        state
            .handle_event(tessera::jobs::JobEvent::Completed {
                token,
                cancelled: !stale,
                result: Ok(tessera::jobs::JobOutput::Analyzed(a)),
            })
            .unwrap();
        assert!(state.project().runs().is_empty());
    }
}
