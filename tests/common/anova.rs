use tessera::{
    analysis::*,
    data::{Cell, Column, ColumnId, Dataset},
    operation::JobControl,
    project::Project,
};
pub fn input(design: usize) -> (Dataset, AnovaRequest) {
    let n = 12;
    let mut columns = vec![
        Column::numeric(
            ColumnId(1),
            "Outcome <script>",
            (0..n)
                .map(|i| Some((i % 4) as f64 * 3. + (i / 4) as f64))
                .collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(2),
            "Factor & A",
            (0..n)
                .map(|i| Some(if i % 4 < 2 { " A " } else { "B" }.into()))
                .collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(3),
            "Factor B",
            (0..n)
                .map(|i| Some(if i % 2 == 0 { "X" } else { "Y" }.into()))
                .collect(),
        )
        .unwrap(),
        Column::text(
            ColumnId(4),
            "Subject ID",
            (0..n)
                .map(|i| Some(format!("private-person-{i}")))
                .collect(),
        )
        .unwrap(),
    ];
    for j in 0..4 {
        columns.push(
            Column::numeric(
                ColumnId(5 + j),
                &format!("Measurement {}", j + 1),
                (0..n)
                    .map(|i| Some(i as f64 + (j * j) as f64 + ((i * j as usize) % 3) as f64))
                    .collect(),
            )
            .unwrap(),
        );
    }
    let design = match design {
        0 => AnovaDesign::OneWay {
            outcome: ColumnId(1),
            factor: ColumnId(2),
        },
        1 => AnovaDesign::Factorial {
            outcome: ColumnId(1),
            factors: vec![ColumnId(2), ColumnId(3)],
        },
        _ => AnovaDesign::Repeated {
            subject: ColumnId(4),
            factors: vec![
                AnovaWithinFactor {
                    name: " Time ".into(),
                    levels: vec![" First ".into(), "Second".into()],
                },
                AnovaWithinFactor {
                    name: "Condition".into(),
                    levels: vec!["X".into(), "Y".into()],
                },
            ],
            measurements: (5..9).map(ColumnId).collect(),
        },
    };
    if matches!(design, AnovaDesign::Repeated { .. }) {
        let old = &columns[3];
        let metadata = tessera::data::ColumnMetadata {
            missing_rules: tessera::data::MissingRules::Text(vec!["private-missing-ID".into()]),
            value_labels: vec![tessera::data::ValueLabel {
                value: Cell::Text("private-person-0".into()),
                label: "private-ID-label".into(),
            }],
            ..old.metadata().clone()
        };
        columns[3] = old.with_metadata(metadata).unwrap();
    }
    let repeated = matches!(design, AnovaDesign::Repeated { .. });
    (
        Dataset::new(columns).unwrap(),
        AnovaRequest {
            design,
            confidence: ConfidenceLevel::P95,
            filter: if repeated {
                RowFilter {
                    predicates: vec![RowPredicate::Text {
                        column: ColumnId(4),
                        value: "private-filter-ID".into(),
                        equal: false,
                    }],
                }
            } else {
                RowFilter::default()
            },
            frequency_weight: None,
            planned_pairs: vec![AnovaPairRequest {
                factor: 0,
                first: Cell::Text(if repeated { " First " } else { " A " }.into()),
                second: Cell::Text(if repeated { "Second" } else { "B" }.into()),
                scope: AnovaScope::Marginal,
            }],
            all_pairs: vec![AnovaFamilyRequest {
                factor: 0,
                scope: AnovaScope::Marginal,
            }],
        },
    )
}
pub fn request_bytes(r: &AnovaRequest) -> usize {
    std::mem::size_of::<AnovaRequest>() + r.heap_bytes().unwrap()
}
pub fn captured(design: usize) -> Project {
    let (d, r) = input(design);
    let a = analyze_with_budget(
        &d,
        &AnalysisRequest::Anova(r.clone()),
        &JobControl::default(),
        &[],
        std::mem::size_of::<AnalysisRequest>() + r.heap_bytes().unwrap(),
    )
    .unwrap();
    Project::new("ANOVA history", d)
        .unwrap()
        .with_run(a)
        .unwrap()
}

pub struct Temp(pub std::path::PathBuf);
impl Temp {
    pub fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "anova-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
