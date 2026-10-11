use tessera::{
    analysis::*,
    data::{Column, ColumnId, Dataset},
    operation::JobControl,
};
#[test]
fn retained_weight_values_are_checked_during_selection() {
    let d = Dataset::new(vec![
        Column::numeric(ColumnId(1), "y", vec![None]).unwrap(),
        Column::text(ColumnId(2), "g", vec![Some("a".into())]).unwrap(),
        Column::numeric(ColumnId(3), "w", vec![Some(-1.)]).unwrap(),
    ])
    .unwrap();
    let r = AnovaRequest {
        design: AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: Some(ColumnId(3)),
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    assert!(r
        .validate_current(&d, &[], 0, &JobControl::default())
        .is_ok());
}
