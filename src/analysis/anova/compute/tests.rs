use super::*;
use crate::{
    analysis::{AnalysisRequest, AnovaFamilyRequest, AnovaScope, ConfidenceLevel, RowFilter},
    data::{measure_test_allocations, Column, ColumnId, MAX_OWNED_BYTES},
    operation::CHECKPOINT_HOOK,
};
use std::{cell::Cell as Counter, rc::Rc};
fn input() -> (Dataset, AnovaRequest) {
    let d = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            vec![Some(1.), Some(3.), Some(5.), Some(8.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Factor",
            vec![
                Some(" A ".into()),
                Some(" A ".into()),
                Some("B".into()),
                Some("B".into()),
            ],
        )
        .unwrap(),
    ])
    .unwrap();
    (
        d,
        AnovaRequest {
            design: AnovaDesign::OneWay {
                outcome: ColumnId(1),
                factor: ColumnId(2),
            },
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
            planned_pairs: vec![],
            all_pairs: vec![AnovaFamilyRequest {
                factor: 0,
                scope: AnovaScope::Marginal,
            }],
        },
    )
}
#[test]
fn reachable_orchestration_cancellation_releases_owners_and_never_returns_capture() {
    let (d, r) = input();
    let bytes = size_of::<AnovaRequest>() + r.heap_bytes().unwrap();
    let seen = Rc::new(Counter::new(0));
    let read = seen.clone();
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = Some(Box::new(move |_| read.set(read.get() + 1))));
    let capture = anova_with_budget(&d, &r, &JobControl::default(), &[], bytes).unwrap();
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    let total = seen.get();
    assert!(total > 30);
    for cut in [1, total / 4, total / 2, total - 1] {
        let c = JobControl::default();
        let cancel = c.clone();
        let n = Rc::new(Counter::new(0));
        let hook = n.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                let next = hook.get() + 1;
                hook.set(next);
                if next == cut {
                    cancel.cancel();
                }
            }))
        });
        let result = anova_with_budget(&d, &r, &c, &[], bytes);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "cut {cut}/{total}"
        );
        assert_eq!(n.get(), cut);
    }
    capture.validate(&JobControl::default()).unwrap();
}
#[test]
fn borrowed_request_and_central_envelope_are_charged_before_copy() {
    let (d, r) = input();
    let request_owner = size_of::<AnovaRequest>() + r.heap_bytes().unwrap();
    let control = JobControl::default();
    let (result, largest) =
        measure_test_allocations(|| anova_with_budget(&d, &r, &control, &[], request_owner - 1));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(largest, 0);
    let (result, largest) =
        measure_test_allocations(|| anova_with_budget(&d, &r, &control, &[], MAX_OWNED_BYTES));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert!(
        largest <= 64,
        "only existing dataset-owner accounting scratch is permitted: {largest}"
    );
    let central = AnalysisRequest::Anova(r.clone());
    assert!(size_of::<AnalysisRequest>() >= size_of::<AnovaRequest>());
    let result = crate::analysis::analyze_with_budget(
        &d,
        &central,
        &JobControl::default(),
        &[&d],
        size_of::<AnalysisRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    assert_eq!(
        result.owned_bytes().unwrap(),
        size_of::<CompletedAnalysis>() - size_of::<ComputedAnova>()
            + match &result {
                CompletedAnalysis::Anova(a) => a.owned_bytes().unwrap(),
                _ => panic!(),
            }
    );
}
#[test]
fn controlled_history_encode_decode_and_export_cancellation_reaches_semantic_work() {
    let (d, r) = input();
    let captured = anova_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        size_of::<AnovaRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    let project = crate::project::Project::new("ANOVA", d)
        .unwrap()
        .with_run(CompletedAnalysis::Anova(captured))
        .unwrap();
    let c = JobControl::default();
    let cancel = c.clone();
    let checkpoints = Rc::new(Counter::new(0));
    let read = checkpoints.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_| {
            let n = read.get() + 1;
            read.set(n);
            if n == 8 {
                cancel.cancel();
            }
        }))
    });
    let mut out = Vec::new();
    let result = crate::project_io::encode(&project, &mut out, &c);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
    assert!(out.is_empty());
    assert_eq!(checkpoints.get(), 8);
    let c = JobControl::default();
    let cancel = c.clone();
    let n = Rc::new(Counter::new(0));
    let hook = n.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_| {
            let count = hook.get() + 1;
            hook.set(count);
            if count == 8 {
                cancel.cancel();
            }
        }))
    });
    let result =
        project.with_run_with_budget_controlled(project.runs()[0].analysis.clone(), &[], 0, &c);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
    assert_eq!(project.runs().len(), 1);
}
#[test]
fn extracted_outcome_accounting_counts_heap_capacity_once() {
    let (d, r) = input();
    let mut result = anova_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        size_of::<AnovaRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    let before = result.owned_bytes().unwrap();
    let heap = accounting::outcome_heap_bytes(&result.outcome).unwrap();
    assert_eq!(
        accounting::outcome_bytes(&result.outcome).unwrap(),
        size_of::<AnovaOutcome>() + heap
    );
    let AnovaOutcome::Fitted(m) = &mut result.outcome else {
        panic!()
    };
    let old = m.effects.capacity();
    m.effects.reserve_exact(20);
    let growth = (m.effects.capacity() - old) * size_of::<AnovaEffect>();
    assert_eq!(result.owned_bytes().unwrap(), before + growth);
    assert_eq!(
        accounting::outcome_heap_bytes(&result.outcome).unwrap(),
        heap + growth
    );
}

#[test]
fn passive_public_decode_and_html_never_enter_orchestration() {
    let (d, r) = input();
    let captured = anova_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        size_of::<AnovaRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    let project = crate::project::Project::new("Passive", d)
        .unwrap()
        .with_run(CompletedAnalysis::Anova(captured))
        .unwrap();
    let mut json = Vec::new();
    crate::project_io::encode(&project, &mut json, &JobControl::default()).unwrap();
    let start = ENTRIES.with(|n| n.get());
    let reopened = crate::project_io::decode(json.as_slice(), &JobControl::default()).unwrap();
    let CompletedAnalysis::Anova(result) = &reopened.runs()[0].analysis else {
        panic!()
    };
    let mut html = Vec::new();
    crate::export::report_anova::write(
        &mut html,
        reopened.runs()[0].id,
        result,
        &JobControl::default(),
    )
    .unwrap();
    assert_eq!(ENTRIES.with(|n| n.get()), start);
    assert_eq!(project.runs(), reopened.runs());
}
#[test]
fn public_decode_cancellation_at_measured_checkpoints() {
    let (d, r) = input();
    let captured = anova_with_budget(
        &d,
        &r,
        &JobControl::default(),
        &[],
        size_of::<AnovaRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    let project = crate::project::Project::new("Decode", d)
        .unwrap()
        .with_run(CompletedAnalysis::Anova(captured))
        .unwrap();
    let mut bytes = Vec::new();
    crate::project_io::encode(&project, &mut bytes, &JobControl::default()).unwrap();
    let seen = Rc::new(Counter::new(0));
    let hook = seen.clone();
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = Some(Box::new(move |_| hook.set(hook.get() + 1))));
    crate::project_io::decode(bytes.as_slice(), &JobControl::default()).unwrap();
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    let total = seen.get();
    assert!(total > 10, "actual checkpoints {total}");
    eprintln!("public decode actual checkpoints: {total}");
    for cut in [1, 3, total / 3, 2 * total / 3, total - 1] {
        let c = JobControl::default();
        let cancel = c.clone();
        let seen = Rc::new(Counter::new(0));
        let hook = seen.clone();
        CHECKPOINT_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |_| {
                let next = hook.get() + 1;
                hook.set(next);
                if next == cut {
                    cancel.cancel();
                }
            }))
        });
        let result = crate::project_io::decode(bytes.as_slice(), &c);
        CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(
            matches!(result, Err(AppError::Cancelled)),
            "cut {cut}/{total}"
        );
        assert_eq!(seen.get(), cut);
    }
}
#[test]
fn convenience_request_quota_refusal_does_not_allocate_a_fallback_control() {
    let (d, mut r) = input();
    r.planned_pairs
        .reserve_exact(MAX_REQUEST_BYTES / std::mem::size_of::<AnovaPairRequest>() + 1);
    let request = AnalysisRequest::Anova(r);
    let (result, largest) = measure_test_allocations(|| request.validate_current(&d, &[], 0));
    assert!(matches!(
        result,
        Err(AppError::SizeLimit {
            limit: MAX_REQUEST_BYTES
        })
    ));
    assert_eq!(largest, 0);
}
