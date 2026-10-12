use super::*;
use tessera::{
    analysis::{CompletedAnalysis, Statistic},
    project::Project,
    survey::generate_with_budget,
};
fn fixture() -> Project {
    Project::new(
        "Synthetic survey",
        Dataset::new(vec![
            Column::text(
                ColumnId(1),
                "Choice",
                vec![
                    Some("Yes".into()),
                    Some("Yes".into()),
                    Some("No".into()),
                    None,
                ],
            )
            .unwrap(),
            Column::numeric(
                ColumnId(2),
                "Measure",
                vec![Some(1.), Some(2.), Some(3.), None],
            )
            .unwrap(),
        ])
        .unwrap(),
    )
    .unwrap()
}
fn request(p: &Project) -> SurveyRequest {
    SurveyRequest {
        revision: p.dataset().revision(),
        choices: vec![
            SurveyChoice {
                column: ColumnId(1),
                role: SurveyRole::Categories,
            },
            SurveyChoice {
                column: ColumnId(2),
                role: SurveyRole::NumericMeasure,
            },
        ],
    }
}
#[test]
fn batch_matches_manual_counts_and_summaries() {
    let p = fixture();
    let q = generate_with_budget(&p, &request(&p), &JobControl::default(), &[], 0).unwrap();
    assert_eq!(q.runs().len(), 3);
    let CompletedAnalysis::Frequencies(f) = &q.runs()[0].analysis else {
        panic!()
    };
    let t = &f.tables[0];
    assert_eq!((t.valid_n, t.missing_n), (3, 1));
    let yes = t.categories.iter().find(|c| c.frequency == 2).unwrap();
    assert_eq!(yes.percent.value, Some(50.));
    assert!((yes.valid_percent.value.unwrap() - 200. / 3.).abs() < 1e-10);
    let CompletedAnalysis::Describe(d) = &q.runs()[1].analysis else {
        panic!()
    };
    let r = &d.rows[0];
    assert_eq!((r.valid_n, r.missing_n), (3, 1));
    for (s, v) in [
        (Statistic::Mean, 2.),
        (Statistic::SampleStdDev, 1.),
        (Statistic::Minimum, 1.),
        (Statistic::Maximum, 3.),
    ] {
        assert_eq!(r.estimates[&s].value, Some(v));
    }
    let CompletedAnalysis::Histogram(h) = &q.runs()[2].analysis else {
        panic!()
    };
    assert_eq!(h.bins.iter().map(|b| b.frequency).sum::<u64>(), 3);
    for run in q.runs() {
        let recipe = match &run.analysis {
            CompletedAnalysis::Frequencies(r) => {
                tessera::analysis::AnalysisRequest::Frequencies(r.request.clone())
            }
            CompletedAnalysis::Describe(r) => {
                tessera::analysis::AnalysisRequest::Describe(r.request.clone())
            }
            CompletedAnalysis::Histogram(r) => {
                tessera::analysis::AnalysisRequest::Histogram(r.request.clone())
            }
            _ => panic!(),
        };
        let manual = tessera::analysis::analyze_with_budget(
            p.dataset(),
            &recipe,
            &JobControl::default(),
            &[],
            0,
        )
        .unwrap();
        assert_eq!(run.analysis, manual);
    }
}
#[test]
fn batch_preserves_dataset_and_prior_runs() {
    let p = fixture();
    let first = generate_with_budget(&p, &request(&p), &JobControl::default(), &[], 0).unwrap();
    let before = format!("{:?}", first.dataset());
    let q = generate_with_budget(&first, &request(&first), &JobControl::default(), &[], 0).unwrap();
    assert!(std::ptr::eq(q.dataset(), first.dataset()));
    assert_eq!(format!("{:?}", q.dataset()), before);
    assert_eq!(q.runs().len(), 6);
    for i in 0..3 {
        assert!(std::sync::Arc::ptr_eq(&q.runs()[i], &first.runs()[i]));
    }
    assert_eq!(q.runs()[3].id.0, 4);
}
#[test]
fn batch_is_atomic_on_failure() {
    let p = fixture();
    let before = format!("{p:?}");
    let c = JobControl::default();
    c.cancel();
    assert!(matches!(
        generate_with_budget(&p, &request(&p), &c, &[], 0),
        Err(tessera::error::AppError::Cancelled)
    ));
    assert!(matches!(
        generate_with_budget(
            &p,
            &request(&p),
            &JobControl::default(),
            &[],
            MAX_OWNED_BYTES - 1
        ),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(format!("{p:?}"), before);
}
#[test]
fn multiple_answers_require_an_explicit_role() {
    let p = Project::new(
        "Multiple",
        Dataset::new(vec![text(1, "Options", &["Red;Blue", "Red;Blue", "Blue"])]).unwrap(),
    )
    .unwrap();
    let mut r = SurveyRequest {
        revision: p.dataset().revision(),
        choices: vec![SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::Unresolved,
        }],
    };
    assert!(generate_with_budget(&p, &r, &JobControl::default(), &[], 0).is_err());
    r.choices[0].role = SurveyRole::Categories;
    let q = generate_with_budget(&p, &r, &JobControl::default(), &[], 0).unwrap();
    let CompletedAnalysis::Frequencies(f) = &q.runs()[0].analysis else {
        panic!()
    };
    assert_eq!(f.tables[0].categories.len(), 2);
    assert!(q
        .provenance()
        .last()
        .unwrap()
        .contains("Possible multiple answers"));
}
#[test]
fn missing_measures_keep_counts_without_histograms_and_reopen() {
    let p = Project::new(
        "Missing",
        Dataset::new(vec![Column::numeric(
            ColumnId(1),
            "Measure",
            vec![None, None],
        )
        .unwrap()])
        .unwrap(),
    )
    .unwrap();
    let r = SurveyRequest {
        revision: p.dataset().revision(),
        choices: vec![SurveyChoice {
            column: ColumnId(1),
            role: SurveyRole::NumericMeasure,
        }],
    };
    let q = generate_with_budget(&p, &r, &JobControl::default(), &[], 0).unwrap();
    assert_eq!(q.runs().len(), 1);
    let CompletedAnalysis::Describe(d) = &q.runs()[0].analysis else {
        panic!()
    };
    assert_eq!((d.rows[0].valid_n, d.rows[0].missing_n), (0, 2));
    assert!(d.rows[0]
        .estimates
        .values()
        .all(|e| e.value.is_none() && e.reason.is_some()));
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tessera-survey-{}.tessera", std::process::id()));
    tessera::files::save_project(&path, &q, None, &JobControl::default()).unwrap();
    let (reopened, _) = tessera::files::read_project(&path, &JobControl::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(q.runs(), reopened.runs());
    assert_eq!(q.provenance(), reopened.provenance());
    assert_eq!(
        format!("{:?}", q.dataset()),
        format!("{:?}", reopened.dataset())
    );
}
#[test]
fn retained_storage_and_request_are_charged_at_the_limit() {
    let p = fixture();
    let r = request(&p);
    let held = Dataset::new(vec![Column::text(
        ColumnId(9),
        "Retained",
        vec![Some("r".repeat(8192))],
    )
    .unwrap()])
    .unwrap();
    let mut low = MAX_OWNED_BYTES - 1_000_000;
    let mut high = MAX_OWNED_BYTES;
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if generate_with_budget(&p, &r, &JobControl::default(), &[], mid).is_ok() {
            low = mid
        } else {
            high = mid
        }
    }
    assert!(generate_with_budget(&p, &r, &JobControl::default(), &[], low).is_ok());
    assert!(matches!(
        generate_with_budget(&p, &r, &JobControl::default(), &[&held], low),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    let more = SurveyRequest {
        revision: r.revision,
        choices: {
            let mut v = Vec::with_capacity(4096);
            v.extend(r.choices);
            v
        },
    };
    assert!(matches!(
        generate_with_budget(&p, &more, &JobControl::default(), &[], low),
        Err(tessera::error::AppError::SizeLimit { .. })
    ));
    assert_eq!(p.runs().len(), 0);
}
