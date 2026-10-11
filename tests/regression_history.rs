use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tessera::{
    analysis::*,
    data::{Cell, Column, ColumnId, Dataset},
    jobs::{JobEvent, JobOperation, JobOutput, JobRunner, JobTask},
    operation::{JobControl, JobToken},
    project::Project,
    project_io::{decode, encode},
};

fn project() -> Arc<Project> {
    Arc::new(
        Project::new(
            "Regression integration",
            Dataset::new(vec![
                Column::numeric(
                    ColumnId(1),
                    "Outcome",
                    vec![Some(2.), Some(4.), Some(5.), Some(8.)],
                )
                .unwrap(),
                Column::numeric(
                    ColumnId(2),
                    "Predictor",
                    vec![Some(1.), Some(2.), Some(3.), Some(4.)],
                )
                .unwrap(),
            ])
            .unwrap(),
        )
        .unwrap(),
    )
}
fn request() -> AnalysisRequest {
    AnalysisRequest::Regression(RegressionRequest {
        outcome: ColumnId(1),
        predictors: vec![RegressionPredictor {
            column: ColumnId(2),
            role: PredictorRole::Numeric,
        }],
        confidence: ConfidenceLevel::P95,
        filter: RowFilter::default(),
        frequency_weight: None,
    })
}
#[test]
fn worker_retains_validated_regression_and_fresh_reopen_is_passive() {
    for unavailable in [false, true] {
        let project = project();
        let mut request = request();
        if unavailable {
            let AnalysisRequest::Regression(r) = &mut request else {
                panic!()
            };
            r.predictors[0].role = PredictorRole::Categorical {
                reference: Cell::Number(999.),
            };
        }
        let mut jobs = JobRunner::default();
        jobs.start(
            JobTask::new(JobOperation::Analyze {
                project: project.clone(),
                request,
            }),
            JobToken {
                document: 1,
                revision: project.dataset().revision(),
                generation: 1,
            },
        )
        .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let result = loop {
            if let Some(JobEvent::Completed {
                cancelled, result, ..
            }) = jobs
                .poll()
                .into_iter()
                .find(|e| matches!(e, JobEvent::Completed { .. }))
            {
                assert!(!cancelled);
                let JobOutput::Analyzed(result) = result.unwrap() else {
                    panic!("analysis output")
                };
                break result;
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        };
        assert!(
            matches!(&result, CompletedAnalysis::Regression(r) if matches!(r.outcome, RegressionOutcome::Unavailable(_)) == unavailable)
        );
        result.validate().unwrap();
        let saved = project.with_run(result).unwrap();
        let mut bytes = Vec::new();
        encode(&saved, &mut bytes, &JobControl::default()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["version"], 3);
        let reopened = decode(bytes.as_slice(), &JobControl::default()).unwrap();
        assert_eq!(saved.runs(), reopened.runs());
        assert_eq!(saved.next_run_id(), reopened.next_run_id());
    }
}
