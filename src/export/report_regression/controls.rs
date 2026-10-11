//! Actual accepted direct-API captures shared by standalone output tests.
use crate::{
    analysis::*,
    data::{Cell, Column, ColumnId, Dataset},
    operation::JobControl,
};
pub(crate) fn cases() -> [&'static str; 10] {
    [
        "ordinary",
        "categorical",
        "huge_n",
        "tiny",
        "overflow",
        "exact_zero",
        "no_df",
        "absent_reference",
        "one_level",
        "unidentified",
    ]
}
pub(crate) fn run(case: &str) -> (Dataset, ComputedRegression) {
    let scale = match case {
        "tiny" => 1e-200,
        "overflow" => 1e155,
        _ => 1.,
    };
    let (x, y) = if case == "no_df" {
        (vec![0., 1.], vec![1., 2.])
    } else {
        (vec![0., 1., 2., 4.], vec![1., 3., 2., 6.])
    };
    let w = if case == "huge_n" {
        vec![(1u64 << 51) as f64; 4]
    } else {
        vec![2., 3., 5., 7.]
    };
    let rows = x.len();
    let mut columns = vec![
        Column::numeric(
            ColumnId(1),
            "Captured outcome <Y>",
            y.iter()
                .map(|y| Some(if case == "exact_zero" { 0. } else { y * scale }))
                .collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Captured predictor & X",
            x.iter()
                .map(|x| Some(if case == "unidentified" { 1. } else { *x }))
                .collect(),
        )
        .unwrap(),
    ];
    let categorical = matches!(case, "categorical" | "absent_reference" | "one_level");
    if categorical {
        columns.push(
            Column::text(
                ColumnId(3),
                "Captured group <G>",
                ["", " B<& ", "", " B<& "]
                    .map(|v| {
                        Some(if case == "one_level" {
                            "".into()
                        } else {
                            v.into()
                        })
                    })
                    .to_vec(),
            )
            .unwrap(),
        );
    }
    let weighted = matches!(case, "categorical" | "huge_n");
    if weighted {
        columns.push(
            Column::numeric(
                ColumnId(4),
                "Captured weight",
                w.into_iter().map(Some).collect(),
            )
            .unwrap(),
        );
    }
    let data = Dataset::new(columns).unwrap();
    let request = RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![RegressionPredictor {
            column: ColumnId(if categorical { 3 } else { 2 }),
            role: if categorical {
                PredictorRole::Categorical {
                    reference: Cell::Text(if case == "absent_reference" {
                        "<missing>".into()
                    } else {
                        "".into()
                    }),
                }
            } else {
                PredictorRole::Numeric
            },
        }],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: weighted.then_some(ColumnId(4)),
    };
    assert_eq!(data.row_count(), rows);
    let result = regression_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    result.validate().unwrap();
    (data, result)
}
pub(crate) fn many(n: usize) -> (Dataset, ComputedRegression) {
    let data = Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Captured outcome <Y>",
            (0..n)
                .map(|i| Some(2. + 0.5 * i as f64 + (i % 3) as f64 / 4.))
                .collect(),
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "Captured predictor & X",
            (0..n).map(|i| Some(i as f64)).collect(),
        )
        .unwrap(),
    ])
    .unwrap();
    let request = RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Numeric,
        }],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    };
    let result = regression_with_budget(
        &data,
        &request,
        &JobControl::default(),
        &[],
        request.heap_bytes().unwrap(),
    )
    .unwrap();
    result.validate().unwrap();
    (data, result)
}
