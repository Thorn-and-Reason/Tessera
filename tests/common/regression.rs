use tessera::{
    analysis::*,
    data::{Cell, Column, ColumnId, Dataset},
    operation::JobControl,
    project::Project,
};
pub fn project() -> Project {
    Project::new(
        "Captured regression <&>",
        Dataset::new(vec![
            Column::numeric(
                ColumnId(1),
                "Outcome <Y>",
                [2., 5., 4., 8., 7., 11.].map(Some).to_vec(),
            )
            .unwrap(),
            Column::numeric(
                ColumnId(2),
                "Predictor & X",
                [1., 2., 3., 4., 5., 6.].map(Some).to_vec(),
            )
            .unwrap(),
            Column::text(
                ColumnId(3),
                "First factor",
                [" a ", "b", " a ", "b", " a ", "b"]
                    .map(|s| Some(s.into()))
                    .to_vec(),
            )
            .unwrap(),
            Column::text(
                ColumnId(4),
                "Second factor",
                ["north", "south", "south", "north", "north", "south"]
                    .map(|s| Some(s.into()))
                    .to_vec(),
            )
            .unwrap(),
        ])
        .unwrap(),
    )
    .unwrap()
}
pub fn request() -> AnalysisRequest {
    AnalysisRequest::Regression(RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![
            RegressionPredictor {
                column: ColumnId(2),
                role: PredictorRole::Numeric,
            },
            RegressionPredictor {
                column: ColumnId(3),
                role: PredictorRole::Categorical {
                    reference: Cell::Text(" a ".into()),
                },
            },
        ],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    })
}
pub fn captured() -> Project {
    let p = project();
    let r = analyze_with_budget(
        p.dataset(),
        &request(),
        &JobControl::default(),
        &[],
        p.owned_bytes().unwrap(),
    )
    .unwrap();
    p.with_run(r).unwrap()
}
