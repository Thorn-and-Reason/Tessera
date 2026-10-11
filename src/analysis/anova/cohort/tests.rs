use super::*;
use crate::{
    analysis::anova::*,
    data::{Cell, ColumnId, Dataset},
    operation::JobControl,
};
use crate::{
    analysis::{ConfidenceLevel, RowFilter, RowPredicate},
    data::Column,
};
fn request() -> AnovaRequest {
    AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: Some(ColumnId(3)),
        planned_pairs: vec![],
        all_pairs: vec![],
    }
}
fn weighted(y: Vec<Option<f64>>, w: Vec<Option<f64>>) -> Dataset {
    let n = y.len();
    Dataset::new(vec![
        Column::numeric(ColumnId(1), "y", y).unwrap(),
        Column::text(ColumnId(2), "g", vec![Some("a".into()); n]).unwrap(),
        Column::numeric(ColumnId(3), "w", w).unwrap(),
    ])
    .unwrap()
}
#[test]
fn weight_precedence() {
    let d = weighted(vec![None, Some(2.)], vec![Some(-1.), Some(1.)]);
    let mut r = request();
    assert!(select(&d, &r, &JobControl::default(), 0).is_err());
    r.filter.predicates.push(RowPredicate::Missing {
        column: ColumnId(1),
        missing: false,
    });
    assert_eq!(
        select(&d, &r, &JobControl::default(), 0)
            .unwrap()
            .rows
            .len(),
        1
    );
    let d = weighted(vec![None, None, Some(3.)], vec![None, Some(0.), Some(2.)]);
    let s = select(&d, &request(), &JobControl::default(), 0).unwrap();
    assert_eq!(s.diagnostics.missing_weight_rows, 1);
    assert_eq!(s.diagnostics.zero_weight_rows, 1);
    assert_eq!(s.frequency, 2);
    assert_eq!(s.rows.len(), 1);
    let d = weighted(
        vec![None, Some(1.)],
        vec![Some((1u64 << 53) as f64), Some(1.)],
    );
    assert!(select(&d, &request(), &JobControl::default(), 0).is_err());
}
#[test]
fn subject_identity_precedes_profile_deletion() {
    let d = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "id",
            vec![Some("same".into()), Some("same".into())],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "a", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(3), "b", vec![Some(1.), Some(2.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = None;
    r.design = AnovaDesign::Repeated {
        subject: ColumnId(1),
        factors: vec![AnovaWithinFactor {
            name: "Time".into(),
            levels: vec!["a".into(), "b".into()],
        }],
        measurements: vec![ColumnId(2), ColumnId(3)],
    };
    assert!(select(&d, &r, &JobControl::default(), 0).is_err());
    r.filter.predicates.push(RowPredicate::Missing {
        column: ColumnId(2),
        missing: false,
    });
    assert_eq!(
        select(&d, &r, &JobControl::default(), 0).unwrap().frequency,
        1
    );
}
#[test]
fn collapsed_empty_outcome() {
    let d = weighted(vec![Some(1.)], vec![Some(1.)]);
    let s = select(&d, &request(), &JobControl::default(), 0).unwrap();
    assert_eq!(s.failure.unwrap().kind, AnovaFailureKind::CollapsedFactor);
    let d = weighted(vec![None], vec![Some(1.)]);
    let s = select(&d, &request(), &JobControl::default(), 0).unwrap();
    assert_eq!(s.failure.unwrap().kind, AnovaFailureKind::EmptyCohort);
}
#[test]
fn checked_cohort_budget_before_allocation() {
    let d = weighted(vec![Some(1.)], vec![Some(1.)]);
    assert!(select(
        &d,
        &request(),
        &JobControl::default(),
        crate::data::MAX_OWNED_BYTES
    )
    .is_err());
}
#[test]
fn typed_labels_preserved() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "y", vec![Some(1.); 6]).unwrap(),
        Column::text(
            ColumnId(2),
            "g",
            ["a", "A", " a", "a ", "é", "e\u{301}"]
                .iter()
                .map(|s| Some((*s).into()))
                .collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = None;
    let s = select(&d, &r, &JobControl::default(), 0).unwrap();
    assert_eq!(s.factors[0].values.len(), 6);
    assert_eq!(s.cells.len(), 6);
    assert!(s.failure.is_none());
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "y", vec![Some(1.); 3]).unwrap(),
        Column::numeric(ColumnId(2), "g", vec![Some(-0.), Some(0.), Some(1.)]).unwrap(),
    ])
    .unwrap();
    let s = select(&d, &r, &JobControl::default(), 0).unwrap();
    assert_eq!(
        s.factors[0].values,
        vec![Cell::Number(0.), Cell::Number(1.)]
    );
    let Cell::Number(z) = s.factors[0].values[0] else {
        panic!()
    };
    assert_eq!(z.to_bits(), 0);
    assert_eq!(s.cells[0].physical_rows, 2);
}
#[test]
fn empty_crossed_cells_retained() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "y", vec![Some(1.), Some(2.)]).unwrap(),
        Column::text(ColumnId(2), "a", vec![Some("a".into()), Some("b".into())]).unwrap(),
        Column::text(ColumnId(3), "b", vec![Some("a".into()), Some("b".into())]).unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = None;
    r.design = AnovaDesign::Factorial {
        outcome: ColumnId(1),
        factors: vec![ColumnId(2), ColumnId(3)],
    };
    let s = select(&d, &r, &JobControl::default(), 0).unwrap();
    assert_eq!(
        s.cells.iter().map(|v| v.frequency).collect::<Vec<_>>(),
        vec![1, 0, 0, 1]
    );
    assert_eq!(s.failure.unwrap().kind, AnovaFailureKind::EmptyCells);
    assert_eq!(
        s.rows.iter().map(|r| r.cell).collect::<Vec<_>>(),
        vec![0, 3]
    );
}
#[test]
fn missing_subject_and_signed_zero_duplicates() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "id", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(2), "a", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(3), "b", vec![Some(1.), Some(2.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = None;
    r.design = AnovaDesign::Repeated {
        subject: ColumnId(1),
        factors: vec![AnovaWithinFactor {
            name: "Time".into(),
            levels: vec!["a".into(), "b".into()],
        }],
        measurements: vec![ColumnId(2), ColumnId(3)],
    };
    assert!(select(&d, &r, &JobControl::default(), 0).is_err());
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "id", vec![Some(-0.), Some(0.)]).unwrap(),
        Column::numeric(ColumnId(2), "a", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(3), "b", vec![Some(1.), Some(2.)]).unwrap(),
    ])
    .unwrap();
    assert!(select(&d, &r, &JobControl::default(), 0).is_err());
}
#[test]
fn declared_order_and_whole_profiles() {
    let d = Dataset::new(vec![
        Column::text(ColumnId(1), "id", vec![Some("a".into()), Some("b".into())]).unwrap(),
        Column::numeric(ColumnId(2), "a", vec![None, Some(2.)]).unwrap(),
        Column::numeric(ColumnId(3), "b", vec![Some(1.), Some(2.)]).unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = None;
    r.design = AnovaDesign::Repeated {
        subject: ColumnId(1),
        factors: vec![AnovaWithinFactor {
            name: " Time ".into(),
            levels: vec!["b".into(), "a ".into()],
        }],
        measurements: vec![ColumnId(3), ColumnId(2)],
    };
    let s = select(&d, &r, &JobControl::default(), 0).unwrap();
    assert_eq!(
        s.factors[0].values,
        vec![Cell::Text("b".into()), Cell::Text("a ".into())]
    );
    assert_eq!(s.rows.len(), 1);
    assert_eq!(s.observed_measurements, 2);
    assert_eq!(s.diagnostics.complete_case_rows, 1);
    assert_eq!(s.incomplete_case_frequency, 1);
}
#[test]
fn numeric_invalid_weight_boundaries() {
    for weight in [-1., 0.5, (1u64 << 53) as f64 + 2.] {
        let d = weighted(vec![None], vec![Some(weight)]);
        assert!(select(&d, &request(), &JobControl::default(), 0).is_err());
    }
}
#[test]
fn grid_limit_and_expanded_family_admission() {
    let data = |levels: usize| {
        Dataset::new(vec![
            Column::numeric(ColumnId(1), "y", vec![Some(1.); levels]).unwrap(),
            Column::numeric(
                ColumnId(2),
                "g",
                (0..levels).map(|i| Some(i as f64)).collect(),
            )
            .unwrap(),
        ])
        .unwrap()
    };
    let mut r = request();
    r.frequency_weight = None;
    let s = select(&data(64), &r, &JobControl::default(), 0).unwrap();
    assert_eq!(s.cells.len(), 64);
    assert!(select(&data(65), &r, &JobControl::default(), 0).is_err());
    r.planned_pairs = vec![AnovaPairRequest {
        factor: 0,
        first: Cell::Number(0.),
        second: Cell::Number(1.),
        scope: AnovaScope::Marginal,
    }];
    let d = data(64);
    let baseline = Budget::dataset_bytes(&[&d], 0).unwrap();
    assert!(select(
        &d,
        &r,
        &JobControl::default(),
        crate::data::MAX_OWNED_BYTES - baseline - 1024
    )
    .is_err());
}
#[test]
fn cancellation_inside_subject_id_comparison() {
    use crate::operation::CHECKPOINT_HOOK;
    let text = "ŵ".repeat(20000);
    let d = Dataset::new(vec![
        Column::text(
            ColumnId(1),
            "id",
            vec![Some(text.clone() + "a"), Some(text + "b")],
        )
        .unwrap(),
        Column::numeric(ColumnId(2), "a", vec![Some(1.); 2]).unwrap(),
        Column::numeric(ColumnId(3), "b", vec![Some(2.); 2]).unwrap(),
    ])
    .unwrap();
    let mut r = request();
    r.frequency_weight = None;
    r.design = AnovaDesign::Repeated {
        subject: ColumnId(1),
        factors: vec![AnovaWithinFactor {
            name: "Time".into(),
            levels: vec!["a".into(), "b".into()],
        }],
        measurements: vec![ColumnId(2), ColumnId(3)],
    };
    let c = JobControl::default();
    let cancel = c.clone();
    CHECKPOINT_HOOK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |p| {
            if p.total.is_none() && p.done >= 8192 {
                cancel.cancel()
            }
        }))
    });
    let result = select(&d, &r, &c, 0);
    CHECKPOINT_HOOK.with(|h| *h.borrow_mut() = None);
    assert!(matches!(result, Err(AppError::Cancelled)));
}
