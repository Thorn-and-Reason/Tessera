use super::*;
use crate::survey::{SurveyChoice, SurveyRequest, SurveyRole};
fn project() -> Arc<Project> {
    Arc::new(
        Project::new(
            "Survey",
            Dataset::new(vec![Column::numeric(
                ColumnId(1),
                "Value",
                vec![Some(1.), Some(2.)],
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap(),
    )
}
#[test]
fn survey_worker_cancellation_and_preflight_limits() {
    let project = project();
    for operation in [
        JobOperation::InspectSurvey {
            project: Arc::clone(&project),
        },
        JobOperation::SurveyReport {
            project: Arc::clone(&project),
            request: SurveyRequest {
                revision: project.dataset().revision(),
                choices: vec![SurveyChoice {
                    column: ColumnId(1),
                    role: SurveyRole::NumericMeasure,
                }],
            },
        },
    ] {
        let mut task = JobTask::new(operation);
        let control = JobControl::default();
        control.cancel();
        assert!(matches!(task.execute(&control), Err(AppError::Cancelled)));
        task.retained.extra_bytes = MAX_OWNED_BYTES;
        assert!(matches!(task.preflight(), Err(AppError::SizeLimit { .. })));
        assert!(project.runs().is_empty());
    }
}
#[test]
fn survey_worker_produces_distinct_outputs_and_shares_dataset() {
    let project = project();
    let control = JobControl::default();
    let task = JobTask::new(JobOperation::InspectSurvey {
        project: Arc::clone(&project),
    });
    let JobOutput::SurveyReview(review) = task.execute(&control).unwrap() else {
        panic!("survey review")
    };
    assert_eq!(review.revision, project.dataset().revision());
    let task = JobTask::new(JobOperation::SurveyReport {
        project: Arc::clone(&project),
        request: SurveyRequest {
            revision: project.dataset().revision(),
            choices: vec![SurveyChoice {
                column: ColumnId(1),
                role: SurveyRole::NumericMeasure,
            }],
        },
    });
    let JobOutput::SurveyReport(candidate) = task.execute(&control).unwrap() else {
        panic!("survey batch")
    };
    assert!(std::ptr::eq(candidate.dataset(), project.dataset()));
    assert_eq!(candidate.runs().len(), 2);
    assert!(project.runs().is_empty());
}
