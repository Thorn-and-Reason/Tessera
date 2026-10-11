use super::*;
use crate::{
    data::{Cell, Column, ColumnId, Dataset},
    error::AppError,
    operation::JobControl,
};

pub(crate) fn dataset() -> Dataset {
    Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Outcome",
            vec![Some(1.), Some(2.), Some(4.), Some(8.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Numeric",
            vec![Some(0.), Some(1.), Some(2.), Some(3.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(3),
            "Coded factor",
            vec![Some(0.), Some(1.), Some(0.), Some(1.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(4),
            "Text factor",
            ["", " ", "cŵn", "a"].map(|s| Some(s.into())).to_vec(),
        )
        .unwrap(),
    ])
    .unwrap()
}
fn draft(reference: Cell) -> RegressionDraft {
    RegressionDraft {
        outcome: Some(ColumnId(1)),
        predictors: vec![RegressionPredictorDraft {
            column: ColumnId(4),
            role: RegressionDraftRole::Categorical {
                reference: Some(reference),
            },
        }],
        ..RegressionDraft::default()
    }
}
#[test]
fn empty_and_incomplete_drafts_require_explicit_roles() {
    let data = dataset();
    assert!(RegressionDraft::default().build_request(&data).is_err());
    let mut d = draft(Cell::Text("".into()));
    assert!(d.build_request(&data).is_ok());
    d.predictors[0].role = RegressionDraftRole::Categorical { reference: None };
    assert!(d.build_request(&data).is_err());
    for reference in [Cell::Missing, Cell::Number(f64::NAN), Cell::Number(0.)] {
        d.predictors[0].role = RegressionDraftRole::Categorical {
            reference: Some(reference),
        };
        assert!(d.build_request(&data).is_err());
    }
    d.predictors[0].role = RegressionDraftRole::Numeric;
    assert!(d.build_request(&data).is_err());
}
#[test]
fn matches_saved_allocates_nothing_and_roles_borrow_references() {
    let d = draft(Cell::Text("ŵ".repeat(80_000)));
    let saved = d.build_request(&dataset()).unwrap();
    let (_, largest) = crate::data::measure_test_allocations(|| {
        assert!(d.matches_saved(&saved));
        let roles = d.group_roles(Some(ColumnId(4))).unwrap();
        assert_eq!(roles.column, Some(ColumnId(4)));
        assert!(roles.selected[1].is_none());
        let RegressionDraftRole::Categorical { reference } = &d.predictors[0].role else {
            panic!()
        };
        assert!(std::ptr::eq(
            roles.selected[0].unwrap(),
            reference.as_ref().unwrap()
        ));
    });
    assert_eq!(largest, 0);
    assert!(d.group_roles(Some(ColumnId(2))).is_none());
    let mut reordered = saved.clone();
    reordered.predictors[0].role = PredictorRole::Categorical {
        reference: Cell::Text("other".into()),
    };
    assert!(!d.matches_saved(&reordered));
    assert!(RegressionDraft::from_request(saved.clone()).matches_saved(&saved));
}
#[test]
fn heap_and_validation_scratch_charge_capacities_before_clone() {
    let mut d = draft(Cell::Text(String::with_capacity(200_000)));
    d.predictors.reserve_exact(12);
    d.filter.predicates.reserve_exact(9);
    let expected = d.predictors.capacity() * std::mem::size_of::<RegressionPredictorDraft>()
        + d.filter.predicates.capacity() * std::mem::size_of::<RowPredicate>()
        + 200_000;
    assert_eq!(d.heap_bytes().unwrap(), expected);
    assert_eq!(d.validation_scratch_bytes().unwrap(), 2 * 1024);
    d.frequency_weight = Some(ColumnId(2));
    assert_eq!(d.validation_scratch_bytes().unwrap(), 3 * 1024);
    d.predictors[0].role = RegressionDraftRole::Categorical {
        reference: Some(Cell::Text(String::with_capacity(
            super::super::super::state::MAX_DRAFT_BYTES + 1,
        ))),
    };
    let data = dataset();
    let (result, largest) = crate::data::measure_test_allocations(|| d.build_request(&data));
    assert!(matches!(result, Err(AppError::SizeLimit { .. })));
    assert_eq!(largest, 0);
}
#[test]
fn whole_column_choices_do_not_certify_final_cohort_reference() {
    let data = dataset().edit_cell(ColumnId(1), 0, Cell::Missing).unwrap();
    let request = draft(Cell::Text("".into())).build_request(&data).unwrap();
    let result = regression_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        result.outcome,
        RegressionOutcome::Unavailable(RegressionFailure {
            kind: RegressionFailureKind::AbsentReference {
                column: ColumnId(4)
            },
            ..
        })
    ));
}
#[test]
fn ordered_distinct_predictors_and_typed_references_round_trip() {
    let mut d = draft(Cell::Text("cŵn".into()));
    d.predictors.insert(
        0,
        RegressionPredictorDraft {
            column: ColumnId(2),
            role: RegressionDraftRole::Numeric,
        },
    );
    let request = d.build_request(&dataset()).unwrap();
    let mut changed = request.clone();
    changed.predictors.swap(0, 1);
    assert!(!d.matches_saved(&changed));
    d.predictors.push(d.predictors[0].clone());
    assert!(d.build_request(&dataset()).is_err());
    d.predictors.pop();
    d.predictors[0].column = ColumnId(1);
    assert!(d.build_request(&dataset()).is_err());
    assert!(RegressionDraft::from_request(request.clone()).matches_saved(&request));
}

#[test]
fn captured_conversion_moves_reference_strings_and_scratch_tracks_filter_roles() {
    let mut d = draft(Cell::Text("ŵ".repeat(100_000)));
    d.filter.predicates.push(RowPredicate::Text {
        column: ColumnId(4),
        value: String::with_capacity(1234),
        equal: false,
    });
    assert_eq!(d.validation_scratch_bytes().unwrap(), 3 * 1024);
    assert!(d.heap_bytes().unwrap() >= 201_234);
    let request = d.build_request(&dataset()).unwrap();
    assert!(request.heap_bytes().unwrap() <= d.heap_bytes().unwrap());
    assert_eq!(
        request.validation_scratch_bytes().unwrap(),
        d.validation_scratch_bytes().unwrap()
    );
    let PredictorRole::Categorical {
        reference: Cell::Text(reference),
    } = &request.predictors[0].role
    else {
        panic!()
    };
    let pointer = reference.as_ptr();
    let (converted, largest) =
        crate::data::measure_test_allocations(|| RegressionDraft::from_request(request));
    let RegressionDraftRole::Categorical {
        reference: Some(Cell::Text(reference)),
    } = &converted.predictors[0].role
    else {
        panic!()
    };
    assert_eq!(reference.as_ptr(), pointer);
    assert!(largest < reference.len());
}
