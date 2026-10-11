use super::*;
use crate::{
    analysis::{
        anova::{cohort, contrasts, independent, repeated, validation},
        support,
    },
    data::Column,
};
pub(super) fn capture(design: usize, scale: f64, absent: bool) -> ComputedAnova {
    capture_confidence(design, scale, absent, ConfidenceLevel::P95)
}
pub(super) fn capture_confidence(
    design: usize,
    scale: f64,
    absent: bool,
    confidence: ConfidenceLevel,
) -> ComputedAnova {
    let (d, model) = if design >= 2 {
        let k = if design == 2 { 3 } else { 4 };
        let mut columns =
            vec![
                Column::numeric(ColumnId(1), "id", (0..5).map(|i| Some(i as f64)).collect())
                    .unwrap(),
            ];
        for j in 0..k {
            columns.push(
                Column::numeric(
                    ColumnId(j as u64 + 2),
                    &format!("y{j}"),
                    (0..5)
                        .map(|i| Some(((i * i + j * j + i * j) as f64 + 1.) * scale))
                        .collect(),
                )
                .unwrap(),
            );
        }
        let factors = if design == 2 {
            vec![AnovaWithinFactor {
                name: "time".into(),
                levels: vec!["a".into(), "b".into(), "c".into()],
            }]
        } else {
            vec![
                AnovaWithinFactor {
                    name: "time".into(),
                    levels: vec!["a".into(), "b".into()],
                },
                AnovaWithinFactor {
                    name: "task".into(),
                    levels: vec!["x".into(), "y".into()],
                },
            ]
        };
        (
            Dataset::new(columns).unwrap(),
            AnovaDesign::Repeated {
                subject: ColumnId(1),
                factors,
                measurements: (0..k).map(|j| ColumnId(j as u64 + 2)).collect(),
            },
        )
    } else {
        let mut columns = vec![
            Column::numeric(
                ColumnId(1),
                "Y",
                (0..12)
                    .map(|i| Some(((i * i % 17) as f64 + 1.) * scale))
                    .collect(),
            )
            .unwrap(),
            Column::text(
                ColumnId(2),
                "group",
                (0..12)
                    .map(|i| Some(if i % 2 == 0 { "a" } else { "b" }.into()))
                    .collect(),
            )
            .unwrap(),
        ];
        let model = if design == 1 {
            columns.push(
                Column::text(
                    ColumnId(3),
                    "task",
                    (0..12)
                        .map(|i| Some(if i % 4 < 2 { "x" } else { "y" }.into()))
                        .collect(),
                )
                .unwrap(),
            );
            AnovaDesign::Factorial {
                outcome: ColumnId(1),
                factors: vec![ColumnId(2), ColumnId(3)],
            }
        } else {
            AnovaDesign::OneWay {
                outcome: ColumnId(1),
                factor: ColumnId(2),
            }
        };
        (Dataset::new(columns).unwrap(), model)
    };
    let r = AnovaRequest {
        design: model,
        confidence,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![AnovaPairRequest {
            factor: 0,
            first: Cell::Text(if absent { "missing" } else { "a" }.into()),
            second: Cell::Text("b".into()),
            scope: AnovaScope::Marginal,
        }],
        all_pairs: vec![AnovaFamilyRequest {
            factor: 0,
            scope: AnovaScope::Marginal,
        }],
    };
    assemble(&d, r)
}
pub(super) fn assemble(d: &Dataset, r: AnovaRequest) -> ComputedAnova {
    let c = JobControl::default();
    let s = cohort::select(d, &r, &c, 0).unwrap();
    let (outcome, means, families) = if matches!(r.design, AnovaDesign::Repeated { .. }) {
        let result = repeated::compute(d, &r, &s, &c, 0).unwrap();
        let families = contrasts::compute(
            d,
            &r,
            &s,
            &result.outcome,
            contrasts::Input::Repeated,
            &c,
            0,
        )
        .unwrap();
        (result.outcome, result.cell_means, families)
    } else {
        let result = independent::compute(d, &r, &s, &c, 0).unwrap();
        let families = contrasts::compute(
            d,
            &r,
            &s,
            &result.outcome,
            contrasts::Input::Independent(result.numerical.as_ref()),
            &c,
            0,
        )
        .unwrap();
        (result.outcome, result.cell_means, families)
    };
    let ids = validation::request(&r, |id| Ok(d.column(id)?.kind()), &c).unwrap();
    let result = ComputedAnova {
        revision: d.revision(),
        request: r,
        columns: support::capture_columns(d, &ids, &c).unwrap(),
        source_rows: d.row_count(),
        diagnostics: s.diagnostics.clone(),
        fitted_rows: s.rows.len(),
        frequency: s.frequency,
        incomplete_case_frequency: s.incomplete_case_frequency,
        observed_measurements: s.observed_measurements,
        grid: AnovaGrid {
            factors: s.factors.clone(),
            cells: s
                .cells
                .iter()
                .zip(means)
                .map(|(count, mean)| AnovaCell {
                    physical_rows: count.physical_rows,
                    frequency: count.frequency,
                    mean,
                })
                .collect(),
            complete: s.frequency > 0
                && !s.cells.is_empty()
                && s.cells.iter().all(|cell| cell.frequency > 0),
        },
        method: AnovaMethod {
            numerical_version: 1,
            basis_version: 1,
        },
        outcome,
        families,
    };
    validation::completed(&result, &c)
        .unwrap_or_else(|e| panic!("capture invalid: {e:?}; {result:?}"));
    result
}

pub(super) fn independent_rows(
    values: Vec<Option<f64>>,
    groups: Vec<Option<String>>,
    tasks: Option<Vec<Option<String>>>,
) -> ComputedAnova {
    let mut columns = vec![
        Column::numeric(ColumnId(1), "Y", values).unwrap(),
        Column::text(ColumnId(2), "group", groups).unwrap(),
    ];
    let design = if let Some(tasks) = tasks {
        columns.push(Column::text(ColumnId(3), "task", tasks).unwrap());
        AnovaDesign::Factorial {
            outcome: ColumnId(1),
            factors: vec![ColumnId(2), ColumnId(3)],
        }
    } else {
        AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        }
    };
    let r = AnovaRequest {
        design,
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    assemble(&Dataset::new(columns).unwrap(), r)
}
pub(super) fn repeated_rows(profiles: &[Vec<f64>], levels: &[usize]) -> ComputedAnova {
    let k = levels.iter().product::<usize>();
    let mut columns = vec![Column::numeric(
        ColumnId(1),
        "id",
        (0..profiles.len()).map(|i| Some(i as f64)).collect(),
    )
    .unwrap()];
    for j in 0..k {
        columns.push(
            Column::numeric(
                ColumnId(j as u64 + 2),
                &format!("y{j}"),
                profiles.iter().map(|v| Some(v[j])).collect(),
            )
            .unwrap(),
        );
    }
    let r = AnovaRequest {
        design: AnovaDesign::Repeated {
            subject: ColumnId(1),
            factors: levels
                .iter()
                .enumerate()
                .map(|(i, n)| AnovaWithinFactor {
                    name: format!("f{i}"),
                    levels: (0..*n).map(|j| j.to_string()).collect(),
                })
                .collect(),
            measurements: (0..k).map(|j| ColumnId(j as u64 + 2)).collect(),
        },
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
        planned_pairs: vec![],
        all_pairs: vec![],
    };
    assemble(&Dataset::new(columns).unwrap(), r)
}
