use super::*;
use crate::data::{ColumnKind, MissingRules, RevisionId, ValueLabel};

pub(super) fn available(value: f64) -> Estimate {
    Estimate {
        value: Some(value),
        reason: None,
    }
}
pub(super) fn unavailable(reason: &str) -> Estimate {
    Estimate {
        value: None,
        reason: Some(reason.into()),
    }
}
fn info(id: u64, name: &str, kind: ColumnKind) -> RunColumnInfo {
    RunColumnInfo {
        id: ColumnId(id),
        name: name.into(),
        kind,
        description: "Definition <img>".into(),
        missing_rules: MissingRules::None,
        value_labels: vec![],
    }
}
pub(super) fn failed() -> ComputedRegression {
    ComputedRegression {
        revision: RevisionId(4),
        request: RegressionRequest {
            outcome: ColumnId(1),
            predictors: vec![RegressionPredictor {
                column: ColumnId(2),
                role: PredictorRole::Numeric,
            }],
            confidence: ConfidenceLevel::P95,
            filter: RowFilter::default(),
            frequency_weight: None,
        },
        columns: vec![
            info(1, "Captured outcome <Y>", ColumnKind::Number),
            info(2, "Captured predictor & X", ColumnKind::Number),
        ],
        source_rows: 8,
        diagnostics: RunDiagnostics {
            filtered_rows: 1,
            missing_weight_rows: 1,
            zero_weight_rows: 1,
            complete_case_rows: 1,
        },
        fitted_rows: 4,
        frequency: 10,
        incomplete_case_frequency: 5,
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
        outcome: RegressionOutcome::Unavailable(RegressionFailure {
            kind: RegressionFailureKind::Numerical,
            reason: "Unavailable fit: reference <absent> & rejected".into(),
            expected_terms: None,
            rank: None,
            implicated_terms: vec![],
        }),
    }
}
pub(super) fn fitted() -> ComputedRegression {
    let coefficients = [1.25, 2.5]
        .map(|value| RegressionCoefficient {
            estimate: available(value),
            standard_error: available(0.125),
            t: available(10.),
            p: available(0.000125),
            interval: [available(value - 0.25), available(value + 0.25)],
        })
        .to_vec();
    let extent = DiagnosticExtent {
        range: [available(0.1), available(0.8)],
        undefined_rows: 1,
        undefined_frequency: 3,
    };
    ComputedRegression {
        outcome: RegressionOutcome::Fitted(Box::new(RegressionFit {
            rank: 2,
            model_df: 1,
            residual_df: 8,
            total_df: 9,
            condition: available(2.),
            condition_warned: false,
            outcome_scale: available(3.),
            term_scales: vec![available(1.), available(2.)],
            coefficients,
            statistics: RegressionStatistics {
                sse: available(2.),
                sst: available(10.),
                model_ss: available(8.),
                residual_standard_error: available(0.5),
                r_squared: available(0.8),
                adjusted_r_squared: available(0.775),
                f: available(32.),
                f_p: available(0.000475),
            },
            residual_diagnostics: RegressionDiagnostics {
                residual_mean: available(0.),
                residual_sample_sd: available(0.47),
                residual_range: [available(-0.5), available(0.5)],
                fitted_range: [available(1.), available(4.)],
                leverage: extent.clone(),
                studentised: extent.clone(),
                cooks_distance: extent,
                residual_points: (0..4)
                    .map(|i| RegressionResidualPoint {
                        cohort_ordinal: i,
                        row: i + 2,
                        frequency: [2, 1, 2, 5][i],
                        fitted: available((i + 1) as f64),
                        residual: available(if i % 2 == 0 { -0.5 } else { 0.5 }),
                        leverage: available(0.5),
                        studentised: available(1.),
                        cooks_distance: unavailable("Cook < domain"),
                    })
                    .collect(),
                residual_reduced: false,
                qq_points: vec![
                    RegressionQqPoint {
                        rank: 0,
                        row: 2,
                        frequency: 2,
                        theoretical: available(-1.64),
                        residual: available(-0.5),
                    },
                    RegressionQqPoint {
                        rank: 9,
                        row: 5,
                        frequency: 5,
                        theoretical: available(1.64),
                        residual: available(0.5),
                    },
                ],
                qq_reduced: true,
                theoretical_range: [available(-1.64), available(1.64)],
            },
        })),
        ..failed()
    }
}
pub(super) fn categorical() -> ComputedRegression {
    let reference = Cell::Text(" a<& ".into());
    let request = RegressionRequest {
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Categorical {
                reference: reference.clone(),
            },
        }],
        frequency_weight: Some(ColumnId(4)),
        filter: RowFilter {
            predicates: vec![
                RowPredicate::Text {
                    column: ColumnId(3),
                    value: "<filter>".into(),
                    equal: true,
                },
                RowPredicate::Missing {
                    column: ColumnId(1),
                    missing: false,
                },
            ],
        },
        ..failed().request
    };
    ComputedRegression {
        request,
        columns: vec![
            info(1, "Captured outcome <Y>", ColumnKind::Number),
            RunColumnInfo {
                value_labels: vec![ValueLabel {
                    value: reference.clone(),
                    label: "Label <script>".into(),
                }],
                ..info(2, "Group", ColumnKind::Text)
            },
            info(3, "Captured filter", ColumnKind::Text),
            info(4, "Captured weight", ColumnKind::Number),
        ],
        levels: vec![RegressionLevels {
            column: ColumnId(2),
            values: vec![
                Cell::Text("".into()),
                reference.clone(),
                Cell::Text("B".into()),
                Cell::Text(" b ".into()),
            ],
        }],
        terms: vec![
            RegressionTerm::Intercept,
            RegressionTerm::Contrast {
                column: ColumnId(2),
                level: Cell::Text("B".into()),
                reference,
            },
        ],
        ..failed()
    }
}
