use super::validation;
use super::*;
use crate::{
    analysis::{selection::select_rows, support::capture_columns, *},
    data::{Cell, Column, ColumnId, ColumnKind, Dataset, RevisionId, ValueLabel},
    error::AppError,
    operation::JobControl,
};
mod accounting;
mod cohort;
mod contract;

fn numeric_request() -> RegressionRequest {
    RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Numeric,
        }],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    }
}
fn dataset() -> Dataset {
    Dataset::new(vec![
        Column::numeric(
            ColumnId(1),
            "Y",
            vec![Some(1.), Some(3.), Some(5.), Some(7.)],
        )
        .unwrap(),
        Column::numeric(
            ColumnId(2),
            "X",
            vec![Some(0.), Some(1.), Some(2.), Some(3.)],
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Group",
            ["a", "b", "a", "b"].map(|v| Some(v.into())).to_vec(),
        )
        .unwrap(),
    ])
    .unwrap()
}
fn category_request(reference: Cell) -> RegressionRequest {
    RegressionRequest {
        predictors: vec![RegressionPredictor {
            column: ColumnId(3),
            role: PredictorRole::Categorical { reference },
        }],
        ..numeric_request()
    }
}
fn selected(
    dataset: &Dataset,
    request: &RegressionRequest,
) -> (Vec<crate::analysis::selection::Eligible>, RunDiagnostics) {
    let mut columns = vec![request.outcome];
    columns.extend(request.predictors.iter().map(|p| p.column));
    select_rows(
        dataset,
        &request.filter,
        request.frequency_weight,
        &columns,
        &JobControl::default(),
        dataset.row_count(),
    )
    .unwrap()
}
fn captured(dataset: &Dataset, request: RegressionRequest) -> ComputedRegression {
    let ids = validation::request(&request, |id| Ok(dataset.column(id)?.kind())).unwrap();
    let (rows, diagnostics) = selected(dataset, &request);
    let (levels, terms, failure) = coding::discover(
        dataset,
        &request,
        &rows,
        &JobControl::default(),
        coding_baseline(dataset, &request, &rows),
    )
    .unwrap();
    ComputedRegression {
        revision: dataset.revision(),
        request,
        columns: capture_columns(dataset, &ids, &JobControl::default()).unwrap(),
        source_rows: dataset.row_count(),
        diagnostics,
        fitted_rows: rows.iter().filter(|r| r.complete).count(),
        frequency: rows.iter().filter(|r| r.complete).map(|r| r.weight).sum(),
        incomplete_case_frequency: rows.iter().filter(|r| !r.complete).map(|r| r.weight).sum(),
        levels,
        terms,
        method: RegressionMethod {
            numerical_version: 1,
            rank_threshold: 1e-12,
            condition_warning: 1e8,
        },
        outcome: RegressionOutcome::Unavailable(
            failure.expect("fixture requires a real coding failure"),
        ),
    }
}
fn available(value: f64) -> Estimate {
    Estimate {
        value: Some(value),
        reason: None,
    }
}
fn unavailable() -> Estimate {
    Estimate {
        value: None,
        reason: Some("Residual variance is zero.".into()),
    }
}
fn fitted_fixture() -> ComputedRegression {
    let data = dataset();
    let request = numeric_request();
    let ids = validation::request(&request, |id| Ok(data.column(id)?.kind())).unwrap();
    let empty_extent = DiagnosticExtent {
        range: [unavailable(), unavailable()],
        undefined_rows: 4,
        undefined_frequency: 4,
    };
    ComputedRegression {
        revision: RevisionId(1),
        request,
        columns: capture_columns(&data, &ids, &JobControl::default()).unwrap(),
        source_rows: 4,
        diagnostics: RunDiagnostics::default(),
        fitted_rows: 4,
        frequency: 4,
        incomplete_case_frequency: 0,
        levels: vec![],
        terms: vec![
            RegressionTerm::Intercept,
            RegressionTerm::Numeric {
                column: ColumnId(2),
            },
        ],
        method: RegressionMethod {
            numerical_version: 1,
            rank_threshold: 1e-12,
            condition_warning: 1e8,
        },
        outcome: RegressionOutcome::Fitted(Box::new(RegressionFit {
            rank: 2,
            model_df: 1,
            residual_df: 2,
            total_df: 3,
            condition: available(1.),
            condition_warned: false,
            outcome_scale: available(5_f64.sqrt()),
            term_scales: vec![available(1.), available(1.25_f64.sqrt())],
            coefficients: [1., 2.]
                .map(|v| RegressionCoefficient {
                    estimate: available(v),
                    standard_error: available(0.),
                    t: unavailable(),
                    p: unavailable(),
                    interval: [available(v), available(v)],
                })
                .to_vec(),
            statistics: RegressionStatistics {
                sse: available(0.),
                sst: available(20.),
                model_ss: available(20.),
                residual_standard_error: available(0.),
                r_squared: available(1.),
                adjusted_r_squared: available(1.),
                f: unavailable(),
                f_p: unavailable(),
            },
            residual_diagnostics: RegressionDiagnostics {
                residual_mean: available(0.),
                residual_sample_sd: available(0.),
                residual_range: [available(0.), available(0.)],
                fitted_range: [available(1.), available(7.)],
                leverage: DiagnosticExtent {
                    range: [available(0.3), available(0.7)],
                    undefined_rows: 0,
                    undefined_frequency: 0,
                },
                studentised: empty_extent.clone(),
                cooks_distance: empty_extent,
                residual_points: [1., 3., 5., 7.]
                    .into_iter()
                    .enumerate()
                    .map(|(row, v)| RegressionResidualPoint {
                        cohort_ordinal: row,
                        row,
                        frequency: 1,
                        fitted: available(v),
                        residual: available(0.),
                        leverage: available(if row == 0 || row == 3 { 0.7 } else { 0.3 }),
                        studentised: unavailable(),
                        cooks_distance: unavailable(),
                    })
                    .collect(),
                residual_reduced: false,
                qq_points: [-1.15, -318. / 1000., 318. / 1000., 1.15]
                    .into_iter()
                    .enumerate()
                    .map(|(row, q)| RegressionQqPoint {
                        rank: row as u64,
                        row,
                        frequency: 1,
                        theoretical: available(q),
                        residual: available(0.),
                    })
                    .collect(),
                qq_reduced: false,
                theoretical_range: [available(-1.15), available(1.15)],
            },
        })),
    }
}

fn coding_baseline(
    data: &Dataset,
    request: &RegressionRequest,
    rows: &Vec<crate::analysis::selection::Eligible>,
) -> usize {
    crate::data::Budget::dataset_bytes(
        &[data],
        request.heap_bytes().unwrap()
            + rows.capacity() * std::mem::size_of::<crate::analysis::selection::Eligible>(),
    )
    .unwrap()
}
